//! `imessage-ap derive`: the background queue that makes previews and
//! audio/video conversions (Milestone 3b, decision 0007).
//!
//! It reads only archive.db and `/data/blobs`, so the backup need not be
//! mounted, and writes only to `/data/derived` and archive.db. The work list
//! is the `pending` rows of `derivative` that the attachment step queued.
//! Progress is committed in small batches, so a stopped run loses at most
//! the files it was converting, and a run with nothing pending does nothing.
//!
//! What each kind becomes is decided in [`media::convert`]. Logs carry counts
//! and hashes only, never file names.

use std::{
    collections::{BTreeMap, VecDeque},
    ffi::CString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::{
    archive,
    attachments::retry_failed,
    blobs,
    dirs::Dirs,
    media::{
        self,
        convert::{self, Action, Kind},
    },
};

const BATCH_FILES: usize = 50;
const BATCH_TIME: Duration = Duration::from_secs(5);
const PROGRESS_EVERY: Duration = Duration::from_secs(30);

/// Default for `--min-free`: 5 GiB.
pub const DEFAULT_MIN_FREE: u64 = 5 << 30;

#[derive(Debug, Clone)]
pub struct Options {
    /// Files converted at once (each tool call uses one thread).
    pub jobs: usize,
    /// Kinds to work on, in this order.
    pub kinds: Vec<Kind>,
    /// Stop after starting this many files.
    pub limit: Option<usize>,
    /// Free space to keep on the file system of `derived/`. A file is only
    /// started while this much plus twice its size is free.
    pub min_free: u64,
    /// Set from outside (Ctrl-C, `docker stop`) to stop once the files in
    /// progress are done, keeping everything finished so far.
    pub stop: Arc<AtomicBool>,
    /// Tests only: request a stop while the file with this number (counting
    /// from 1) is being converted.
    #[doc(hidden)]
    pub stop_after: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            jobs: crate::attachments::default_jobs(),
            kinds: Kind::ALL.to_vec(),
            limit: None,
            min_free: DEFAULT_MIN_FREE,
            stop: Arc::new(AtomicBool::new(false)),
            stop_after: None,
        }
    }
}

/// What one run did for one kind.
#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct KindCounts {
    pub pending_at_start: usize,
    /// Failed ones an older decoder tried, set back to pending first.
    pub retried: usize,
    /// Files written (remuxed or converted).
    pub made: usize,
    /// Files browsers can use as they are: nothing written.
    pub original: usize,
    pub failed: usize,
    pub bytes_written: u64,
    pub pending_at_end: usize,
}

/// What one run did.
#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct Counts {
    /// Partly written files of an interrupted run, removed at start.
    pub temporary_files_removed: usize,
    /// Files handed to the tools.
    pub files_started: usize,
    pub kinds: BTreeMap<String, KindCounts>,
}

impl Counts {
    fn kind(&mut self, kind: Kind) -> &mut KindCounts {
        self.kinds.entry(kind.as_str().to_owned()).or_default()
    }

    pub fn total(&self, f: impl Fn(&KindCounts) -> usize) -> usize {
        self.kinds.values().map(f).sum()
    }
}

/// Why a run stopped for lack of disk space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowDisk {
    pub free: u64,
    pub needed: u64,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub run_id: i64,
    pub counts: Counts,
    /// Stopped by [`Options::stop`] before finishing.
    pub interrupted: bool,
    /// Stopped because the disk holding `derived/` is nearly full.
    pub low_disk: Option<LowDisk>,
}

/// Makes every pending derivative of `opts.kinds`.
pub fn run(dirs: &Dirs, opts: &Options) -> Result<Summary> {
    anyhow::ensure!(
        archive::path(dirs).is_file(),
        "no archive at {}; run `imessage-ap ingest` and `imessage-ap attachments` first",
        archive::path(dirs).display()
    );
    media::check_conversion_tools().map_err(|why| anyhow::anyhow!(why))?;
    let _lock = blobs::lock(&dirs.db)?;
    let db = archive::open(dirs)?;

    // A run that was killed outright never recorded how it ended.
    db.execute(
        "UPDATE derive_run SET status = 'failed', finished_at = ?1, error = 'did not finish (stopped without warning)'
         WHERE status = 'running'",
        [now_ns()],
    )?;
    let kinds_text = opts
        .kinds
        .iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join(",");
    db.execute(
        "INSERT INTO derive_run (started_at, status, kinds, version) VALUES (?1, 'running', ?2, ?3)",
        params![now_ns(), kinds_text, crate::VERSION],
    )?;
    let run_id = db.last_insert_rowid();

    let mut counts = Counts::default();
    let low_disk = Mutex::new(None);
    let result = (|| -> Result<()> {
        counts.temporary_files_removed = blobs::reset_tmp(dirs)?;
        for &kind in &opts.kinds {
            counts.kind(kind).retried = retry_failed(&db, kind.as_str())?;
        }
        make_all(&db, dirs, opts, &mut counts, &low_disk)?;
        for &kind in &opts.kinds {
            counts.kind(kind).pending_at_end = pending(&db, kind)?;
        }
        Ok(())
    })();
    let low_disk = low_disk.into_inner().expect("low-disk flag");
    // A stop that came after the last file had started left nothing undone.
    let interrupted = opts.stop.load(Ordering::SeqCst)
        && low_disk.is_none()
        && (result.is_err() || counts.total(|k| k.pending_at_end) > 0);
    let (status, error) = match (&result, &low_disk) {
        (Err(e), _) => ("failed", Some(format!("{e:#}"))),
        (Ok(()), Some(l)) => (
            "failed",
            Some(format!(
                "stopped: {} free on the disk of derived/, {} needed",
                gib(l.free),
                gib(l.needed)
            )),
        ),
        (Ok(()), None) if interrupted => (
            "failed",
            Some("interrupted; run again to resume".to_owned()),
        ),
        (Ok(()), None) => ("succeeded", None),
    };
    db.execute(
        "UPDATE derive_run SET status = ?2, finished_at = ?3, error = ?4, counts = ?5 WHERE id = ?1",
        params![run_id, status, now_ns(), error, serde_json::to_string(&counts)?],
    )?;
    result?;
    tracing::info!(
        run_id,
        started = counts.files_started,
        made = counts.total(|k| k.made),
        original = counts.total(|k| k.original),
        failed = counts.total(|k| k.failed),
        pending = counts.total(|k| k.pending_at_end),
        interrupted,
        low_disk = low_disk.is_some(),
        "derive run finished"
    );
    Ok(Summary {
        run_id,
        counts,
        interrupted,
        low_disk,
    })
}

fn pending(db: &Connection, kind: Kind) -> Result<usize> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM derivative WHERE kind = ?1 AND status = 'pending'",
        [kind.as_str()],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

struct Job {
    sha256: String,
    kind: Kind,
    size: u64,
}

/// Pending derivatives of `kinds`, kind by kind in that order.
fn work_list(db: &Connection, kinds: &[Kind], failed_too: bool) -> Result<Vec<Job>> {
    let mut stmt = db.prepare(
        "SELECT d.sha256, b.size FROM derivative d JOIN blob b ON b.sha256 = d.sha256
         WHERE d.kind = ?1 AND (d.status = 'pending' OR (?2 AND d.status = 'failed' AND d.decoder < ?3))
         ORDER BY d.sha256",
    )?;
    let mut out = Vec::new();
    for &kind in kinds {
        let rows = stmt.query_map(
            params![kind.as_str(), failed_too, media::input::DECODER],
            |r| {
                Ok(Job {
                    sha256: r.get(0)?,
                    kind,
                    size: r.get::<_, i64>(1)? as u64,
                })
            },
        )?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

struct Outcome {
    sha256: String,
    kind: Kind,
    size: u64,
    tmp: PathBuf,
    result: Result<convert::Made, String>,
}

fn make_all(
    db: &Connection,
    dirs: &Dirs,
    opts: &Options,
    counts: &mut Counts,
    low_disk: &Mutex<Option<LowDisk>>,
) -> Result<()> {
    let mut jobs = work_list(db, &opts.kinds, false)?;
    for j in &jobs {
        counts.kind(j.kind).pending_at_start += 1;
    }
    if let Some(limit) = opts.limit {
        jobs.truncate(limit);
    }
    if jobs.is_empty() {
        return Ok(());
    }
    let total = jobs.len();
    let total_bytes: u64 = jobs.iter().map(|j| j.size).sum();
    let workers = opts.jobs.max(1).min(total);
    tracing::info!(files = total, workers, kinds = ?opts.kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(), "converting");
    let jobs = Mutex::new(VecDeque::from(jobs));
    let tmp_dir = blobs::tmp_dir(&dirs.derived);
    let (tx, rx) = mpsc::channel::<Outcome>();
    let started = Mutex::new(0usize);

    let mut done = db.prepare_cached(
        "UPDATE derivative SET status = 'done', recipe = ?3, action = ?4, path = ?5, bytes = ?6, error = NULL, made_at = ?7, decoder = ?8
         WHERE sha256 = ?1 AND kind = ?2",
    )?;
    let mut failed = db.prepare_cached(
        "UPDATE derivative SET status = 'failed', recipe = ?3, action = NULL, path = NULL, bytes = NULL, error = ?4, made_at = ?5, decoder = ?6
         WHERE sha256 = ?1 AND kind = ?2",
    )?;
    let stop_disk = |free: u64, needed: u64| {
        low_disk
            .lock()
            .expect("low-disk flag")
            .get_or_insert(LowDisk { free, needed });
        opts.stop.store(true, Ordering::SeqCst);
    };

    std::thread::scope(|scope| -> Result<()> {
        for _ in 0..workers {
            let tx = tx.clone();
            let (jobs, tmp_dir, started, stop_disk) = (&jobs, &tmp_dir, &started, &stop_disk);
            scope.spawn(move || {
                while !opts.stop.load(Ordering::SeqCst) {
                    let Some(job) = jobs.lock().expect("job queue").pop_front() else {
                        break;
                    };
                    let needed = opts.min_free.saturating_add(job.size.saturating_mul(2));
                    match free_space(tmp_dir) {
                        Ok(free) if free < needed => {
                            stop_disk(free, needed);
                            break;
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "could not read free disk space"),
                    }
                    let n = {
                        let mut n = started.lock().expect("counter");
                        *n += 1;
                        *n
                    };
                    let src = blobs::blob_path(dirs, &job.sha256);
                    let tmp = blobs::tmp_path(tmp_dir, &format!(".{}", job.kind.ext()));
                    let result = convert::make(job.kind, &src, &tmp, tmp_dir);
                    // Tests: a stop request that arrives while this file
                    // is being converted.
                    if opts.stop_after.is_some_and(|after| n >= after) {
                        opts.stop.store(true, Ordering::SeqCst);
                    }
                    let outcome = Outcome {
                        sha256: job.sha256,
                        kind: job.kind,
                        size: job.size,
                        tmp,
                        result,
                    };
                    if tx.send(outcome).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut batch = Batch::new(db);
        let mut last_progress = Instant::now();
        let (since, mut results, mut bytes_done) = (Instant::now(), 0usize, 0u64);
        for o in rx {
            results += 1;
            bytes_done += o.size;
            match o.result {
                Err(why) if convert::is_disk_full(&why) => {
                    // Not the file's fault: keep it pending and stop.
                    let _ = std::fs::remove_file(&o.tmp);
                    let free = free_space(&tmp_dir).unwrap_or(0);
                    tracing::warn!(sha256 = %o.sha256, "the disk is full; stopping");
                    stop_disk(free, opts.min_free);
                }
                // A tool killed by the same Ctrl-C did not fail on this
                // file: leave it pending for the next run.
                Err(_) if opts.stop.load(Ordering::SeqCst) => {
                    let _ = std::fs::remove_file(&o.tmp);
                }
                Err(why) => {
                    let _ = std::fs::remove_file(&o.tmp);
                    let why = dirs.shorten(&why);
                    tracing::warn!(sha256 = %o.sha256, kind = o.kind.as_str(), "conversion failed");
                    batch.begin()?;
                    failed.execute(params![
                        o.sha256,
                        o.kind.as_str(),
                        o.kind.recipe(),
                        why,
                        now_ns(),
                        media::input::DECODER
                    ])?;
                    counts.kind(o.kind).failed += 1;
                    batch.done_one()?;
                }
                Ok(made) => {
                    let path = match made.bytes {
                        Some(_) => {
                            let rel = blobs::derived_relative_path(
                                o.kind.as_str(),
                                &o.sha256,
                                o.kind.ext(),
                            );
                            let dest = dirs.derived_file(&rel);
                            std::fs::create_dir_all(
                                dest.parent().expect("derived paths have a parent"),
                            )?;
                            std::fs::rename(&o.tmp, &dest).with_context(|| {
                                format!("moving a file into {}", dest.display())
                            })?;
                            Some(rel)
                        }
                        None => None,
                    };
                    batch.begin()?;
                    done.execute(params![
                        o.sha256,
                        o.kind.as_str(),
                        o.kind.recipe(),
                        made.action.as_str(),
                        path,
                        made.bytes.map(|b| b as i64),
                        now_ns(),
                        media::input::DECODER
                    ])?;
                    let k = counts.kind(o.kind);
                    if made.action == Action::Original {
                        k.original += 1;
                    } else {
                        k.made += 1;
                        k.bytes_written += made.bytes.unwrap_or(0);
                    }
                    batch.done_one()?;
                }
            }
            if last_progress.elapsed() >= PROGRESS_EVERY {
                last_progress = Instant::now();
                let elapsed = since.elapsed().as_secs_f64();
                let left = total_bytes.saturating_sub(bytes_done) as f64;
                let eta_min = if bytes_done > 0 {
                    elapsed * left / bytes_done as f64 / 60.0
                } else {
                    0.0
                };
                tracing::info!(
                    done = results,
                    of = total,
                    failed = counts.total(|k| k.failed),
                    eta_minutes = format!("{eta_min:.0}"),
                    "converting"
                );
            }
        }
        batch.commit()
    })?;
    counts.files_started = started.into_inner().expect("counter");
    Ok(())
}

/// Groups writes into transactions of a few seconds each.
struct Batch<'a> {
    db: &'a Connection,
    open: bool,
    files: usize,
    since: Instant,
}

impl<'a> Batch<'a> {
    fn new(db: &'a Connection) -> Self {
        Self {
            db,
            open: false,
            files: 0,
            since: Instant::now(),
        }
    }

    fn begin(&mut self) -> Result<()> {
        if !self.open {
            self.db.execute_batch("BEGIN IMMEDIATE")?;
            self.open = true;
            self.files = 0;
            self.since = Instant::now();
        }
        Ok(())
    }

    fn done_one(&mut self) -> Result<()> {
        self.files += 1;
        if self.files >= BATCH_FILES || self.since.elapsed() >= BATCH_TIME {
            self.commit()?;
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if self.open {
            self.db.execute_batch("COMMIT")?;
            self.open = false;
        }
        Ok(())
    }
}

impl Drop for Batch<'_> {
    fn drop(&mut self) {
        if self.open {
            let _ = self.db.execute_batch("ROLLBACK");
        }
    }
}

/// Bytes available to this process on the file system holding `path`.
pub fn free_space(path: &Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    // SAFETY: `c` is a valid NUL-terminated path and `stat` is a properly
    // sized, writable struct that statvfs fills in.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut stat) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Parses a size such as `20G`, `512M` or `1073741824`. Units are binary
/// (as `df -h` shows them): K, M, G and T, with an optional `i` and `B`.
pub fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let split = t
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let value: f64 = num
        .parse()
        .map_err(|_| format!("{s:?} is not a size such as 20G"))?;
    let unit = unit.trim().to_ascii_uppercase();
    let unit = unit.trim_end_matches('B').trim_end_matches('I');
    let mult: u64 = match unit {
        "" => 1,
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        "T" => 1 << 40,
        _ => return Err(format!("{s:?} has an unknown unit; use K, M, G or T")),
    };
    Ok((value * mult as f64) as u64)
}

fn gib(b: u64) -> String {
    format!("{:.1} GiB", b as f64 / f64::from(1u32 << 30))
}

/// What `derive --plan` found for one kind and action.
#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct PlanRow {
    pub files: usize,
    pub input_bytes: u64,
    /// Total length of the videos or voice messages, in seconds.
    pub seconds: f64,
    /// Projected output size and single-thread run time: low and high.
    pub output_bytes: (u64, u64),
    pub cpu_seconds: (f64, f64),
}

/// What a run would do, found without converting anything.
#[derive(Debug, Default, Clone, Serialize)]
pub struct PlanReport {
    pub jobs: usize,
    /// By kind, then action (or `unreadable` when probing failed).
    pub rows: BTreeMap<String, BTreeMap<String, PlanRow>>,
}

/// Probes every file a run of `opts.kinds` would work on (including failed
/// ones a decoder fix will retry) and projects output size and run time.
/// Writes nothing to archive.db or the stores. The projections are rough:
/// see decision 0007 for the rates behind them.
pub fn plan(dirs: &Dirs, opts: &Options) -> Result<PlanReport> {
    media::check_conversion_tools().map_err(|why| anyhow::anyhow!(why))?;
    let _lock = blobs::lock(&dirs.db)?;
    let db = archive::open_read_only(dirs).context("run `imessage-ap migrate` first")?;
    let mut jobs = work_list(&db, &opts.kinds, true)?;
    if let Some(limit) = opts.limit {
        jobs.truncate(limit);
    }
    let work_dir = blobs::tmp_dir(&dirs.derived);
    std::fs::create_dir_all(&work_dir)?;
    let mut report = PlanReport {
        jobs: opts.jobs.max(1),
        rows: BTreeMap::new(),
    };
    let mut last_progress = Instant::now();
    for (i, job) in jobs.iter().enumerate() {
        if opts.stop.load(Ordering::SeqCst) {
            break;
        }
        let src = blobs::blob_path(dirs, &job.sha256);
        let probed = convert::plan(job.kind, &src, &work_dir);
        let action = probed
            .as_ref()
            .map_or("unreadable", |p| p.action.as_str())
            .to_owned();
        let row = report
            .rows
            .entry(job.kind.as_str().to_owned())
            .or_default()
            .entry(action)
            .or_default();
        row.files += 1;
        row.input_bytes += job.size;
        if let Ok(p) = &probed {
            row.seconds += p.duration;
            let (bytes, cpu) = projection(job.kind, p, job.size);
            row.output_bytes.0 += bytes.0;
            row.output_bytes.1 += bytes.1;
            row.cpu_seconds.0 += cpu.0;
            row.cpu_seconds.1 += cpu.1;
        }
        if last_progress.elapsed() >= PROGRESS_EVERY {
            last_progress = Instant::now();
            tracing::info!(done = i + 1, of = jobs.len(), "probing");
        }
    }
    Ok(report)
}

/// Rough output size (bytes) and single-thread CPU time (seconds), low and
/// high, for one file. Rates come from the measurements in decision 0007,
/// scaled to an i5-9400.
fn projection(kind: Kind, p: &convert::Plan, size: u64) -> ((u64, u64), (f64, f64)) {
    let d = p.duration;
    match (kind, p.action) {
        (_, Action::Original) => ((0, 0), (0.1, 0.2)),
        (Kind::Preview, _) => ((250_000, 450_000), (0.9, 1.6)),
        (_, Action::Remux) => ((size, size + size / 50), (0.2, 0.5 + d / 200.0)),
        (Kind::Audio, Action::Transcode) => {
            let b = (d * 64_000.0 / 8.0) as u64;
            ((b, b + b / 5), (d / 100.0, d / 40.0))
        }
        (_, Action::Transcode) => {
            // Bit rate of H.264 at CRF 23 and real-time factor per thread,
            // by source size (sources over 1080p are scaled down).
            let (mbps, speed) = match p.pixels {
                0..=500_000 => ((1.0, 3.0), (0.4, 0.9)),
                500_001..=2_200_000 => ((3.5, 8.0), (1.2, 2.4)),
                _ => ((4.0, 9.0), (2.0, 4.0)),
            };
            let bytes = |m: f64| (d * m * 1e6 / 8.0) as u64;
            (
                (bytes(mbps.0), bytes(mbps.1)),
                (d * speed.0, d * speed.1 * 1.3),
            )
        }
    }
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_in_binary_units() {
        assert_eq!(parse_size("20G").unwrap(), 20 << 30);
        assert_eq!(parse_size("20GiB").unwrap(), 20 << 30);
        assert_eq!(parse_size("512m").unwrap(), 512 << 20);
        assert_eq!(
            parse_size("1.5T").unwrap(),
            (1.5 * (1u64 << 40) as f64) as u64
        );
        assert_eq!(parse_size("0").unwrap(), 0);
        assert!(parse_size("lots").is_err());
        assert!(parse_size("5X").is_err());
    }

    #[test]
    fn free_space_is_read() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).unwrap() > 0);
        assert!(free_space(&dir.path().join("missing")).is_err());
    }
}
