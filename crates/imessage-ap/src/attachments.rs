//! `imessage-ap attachments`: copies a backup's attachment files into the
//! blob store and makes their thumbnails (Milestone 3a, decision 0005).
//!
//! Run it after `ingest` for the same backup. Every file is read from the
//! backup (never written), hashed, and stored once per distinct content.
//! Progress is committed in small batches, so an interrupted run loses at
//! most the file it was copying, and the next run picks up where it stopped.
//! A finished backup has nothing left to do: running again reads no files.
//!
//! Previews and audio/video conversions are queued here and made later by
//! the background queue (Milestone 3b). Logs and issues carry counts and
//! GUIDs only, never file names.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{
    archive,
    backup::Backup,
    blobs::{self, DERIVED_DIR},
    ingest::IMESSAGE_DATABASE_VERSION,
    media::{self, MediaKind},
};

/// Problem kinds this step records in `import_issue`.
pub mod issue {
    /// A file the backup should have could not be read; retried next run.
    pub const ATTACHMENT_READ_FAILED: &str = "attachment_read_failed";
}

/// Commit after this many files or this much time, whichever comes first.
const BATCH_FILES: usize = 200;
const BATCH_TIME: Duration = Duration::from_secs(5);
const PROGRESS_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Options {
    /// Thumbnails made at once (each tool call uses one thread).
    pub jobs: usize,
    /// Stop after reading this many files (for trying a run out).
    pub limit: Option<usize>,
    /// Set from outside (Ctrl-C, `docker stop`) to stop after the current
    /// file and thumbnails, with everything done so far kept.
    pub stop: Arc<AtomicBool>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            jobs: default_jobs(),
            limit: None,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Number of CPUs this process may use, which honours a container's
/// `--cpus` limit.
pub fn default_jobs() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// What one run did.
#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct Counts {
    /// Files of this backup not yet copied when the run started.
    pub files_pending_at_start: usize,
    pub files_read: usize,
    pub bytes_read: u64,
    /// Files stored as new blobs.
    pub blobs_added: usize,
    pub bytes_stored: u64,
    /// Files whose content was already stored, so no copy was kept.
    pub duplicates_skipped: usize,
    /// Partly finished copies of an interrupted run, removed at start.
    pub temporary_files_removed: usize,
    pub read_failures: usize,
    pub thumbnails_made: usize,
    pub thumbnails_failed: usize,
    /// Failed thumbnails set back to pending because a newer decoder may
    /// read them ([`media::input::DECODER`]); counted again as made or failed.
    pub thumbnails_retried: usize,
    /// Previews and conversions queued for the background queue.
    pub derivatives_queued: usize,
    /// Files of this backup still to copy when the run ended.
    pub files_pending_at_end: usize,
    /// Thumbnails still to make when the run ended.
    pub thumbnails_pending_at_end: usize,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub run_id: i64,
    pub source_id: i64,
    pub counts: Counts,
    /// Stopped by [`Options::stop`] before finishing.
    pub interrupted: bool,
    /// Why thumbnails were not made, when the tools are missing.
    pub thumbnails_skipped: Option<String>,
}

/// Copies the attachment files of `backup_dir`, which must already be
/// ingested into the archive in `data_dir`, and makes their thumbnails.
pub fn run(backup_dir: &Path, data_dir: &Path, opts: &Options) -> Result<Summary> {
    let backup = Backup::open(backup_dir)?;
    anyhow::ensure!(
        archive::path(data_dir).is_file(),
        "no archive at {}; run `imessage-ap ingest` for this backup first",
        archive::path(data_dir).display()
    );
    let _lock = blobs::lock(data_dir)?;
    let db = archive::open(data_dir)?;
    let source_id: i64 = db
        .query_row(
            "SELECT id FROM source WHERE device_id = ?1 AND backup_date = ?2",
            params![backup.info.device_id, backup.info.backup_date],
            |r| r.get(0),
        )
        .optional()?
        .context("this backup has not been imported yet; run `imessage-ap ingest` for it first")?;

    // A run that was killed outright never recorded how it ended.
    db.execute(
        "UPDATE import_run SET status = 'failed', finished_at = ?1, error = 'did not finish (stopped without warning)'
         WHERE kind = 'attachments' AND status = 'running'",
        [now_ns()],
    )?;
    db.execute(
        "INSERT INTO import_run (kind, source_id, backup_folder, started_at, status, ingester_version, imessage_database_version)
         VALUES ('attachments', ?1, ?2, ?3, 'running', ?4, ?5)",
        params![
            source_id,
            backup_dir.to_string_lossy(),
            now_ns(),
            crate::VERSION,
            IMESSAGE_DATABASE_VERSION
        ],
    )?;
    let run_id = db.last_insert_rowid();

    let mut counts = Counts::default();
    let mut thumbnails_skipped = None;
    let result = (|| -> Result<()> {
        counts.temporary_files_removed = blobs::reset_tmp(data_dir)?;
        copy_files(&db, &backup, data_dir, source_id, run_id, opts, &mut counts)?;
        if !opts.stop.load(Ordering::SeqCst) {
            if let Err(why) = media::check_thumbnail_tools() {
                tracing::warn!(reason = %why, "thumbnails stay pending");
                thumbnails_skipped = Some(why);
            } else {
                make_thumbnails(&db, data_dir, opts, &mut counts)?;
            }
        }
        counts.files_pending_at_end = pending_files(&db, source_id)?;
        counts.thumbnails_pending_at_end = count(
            &db,
            "SELECT COUNT(*) FROM derivative WHERE kind = 'thumb' AND status = 'pending'",
        )?;
        Ok(())
    })();
    let interrupted = opts.stop.load(Ordering::SeqCst);
    let (status, error) = match &result {
        Err(e) => ("failed", Some(format!("{e:#}"))),
        Ok(()) if interrupted => (
            "failed",
            Some("interrupted; run again to resume".to_owned()),
        ),
        Ok(()) => ("succeeded", None),
    };
    db.execute(
        "UPDATE import_run SET status = ?2, finished_at = ?3, error = ?4, counts = ?5 WHERE id = ?1",
        params![run_id, status, now_ns(), error, serde_json::to_string(&counts)?],
    )?;
    result?;
    tracing::info!(
        run_id,
        source_id,
        files_read = counts.files_read,
        blobs_added = counts.blobs_added,
        duplicates_skipped = counts.duplicates_skipped,
        thumbnails_made = counts.thumbnails_made,
        files_pending = counts.files_pending_at_end,
        interrupted,
        "attachment run finished"
    );
    Ok(Summary {
        run_id,
        source_id,
        counts,
        interrupted,
        thumbnails_skipped,
    })
}

fn count(db: &Connection, sql: &str) -> Result<usize> {
    Ok(db.query_row(sql, [], |r| r.get::<_, i64>(0))? as usize)
}

/// Files this backup holds that are not copied yet.
fn pending_files(db: &Connection, source_id: i64) -> Result<usize> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM attachment_source WHERE source_id = ?1 AND present = 1 AND sha256 IS NULL",
        [source_id],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

struct Work {
    attachment_id: i64,
    source_rowid: i64,
    backup_file_id: String,
    guid: String,
    kind: MediaKind,
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

    /// Counts one finished item and commits if the batch is full.
    fn done_one(&mut self, max: usize) -> Result<()> {
        self.files += 1;
        if self.files >= max || self.since.elapsed() >= BATCH_TIME {
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

fn copy_files(
    db: &Connection,
    backup: &Backup,
    data_dir: &Path,
    source_id: i64,
    run_id: i64,
    opts: &Options,
    counts: &mut Counts,
) -> Result<()> {
    let work: Vec<Work> = db
        .prepare(
            "SELECT s.attachment_id, s.source_rowid, s.backup_file_id, a.guid, a.mime_type, a.uti, a.transfer_name
             FROM attachment_source s JOIN attachment a ON a.id = s.attachment_id
             WHERE s.source_id = ?1 AND s.present = 1 AND s.sha256 IS NULL AND s.backup_file_id IS NOT NULL
             ORDER BY s.source_rowid",
        )?
        .query_map([source_id], |r| {
            let (mime, uti, name): (Option<String>, Option<String>, Option<String>) =
                (r.get(4)?, r.get(5)?, r.get(6)?);
            Ok(Work {
                attachment_id: r.get(0)?,
                source_rowid: r.get(1)?,
                backup_file_id: r.get(2)?,
                guid: r.get(3)?,
                kind: MediaKind::classify(mime.as_deref(), uti.as_deref(), name.as_deref()),
            })
        })?
        .collect::<Result<_, _>>()?;
    counts.files_pending_at_start = work.len();
    tracing::info!(source_id, files = work.len(), "copying attachment files");

    let mut add_blob = db.prepare_cached(
        "INSERT INTO blob (sha256, size, media_kind, first_stored_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (sha256) DO NOTHING",
    )?;
    let mut queue = db.prepare_cached(
        "INSERT INTO derivative (sha256, kind, status, recipe) VALUES (?1, ?2, 'pending', ?3)
         ON CONFLICT DO NOTHING",
    )?;
    let mut set_hash = db.prepare_cached(
        "UPDATE attachment_source SET sha256 = ?3 WHERE attachment_id = ?1 AND source_id = ?2",
    )?;
    // The attachment's hash is the newest backup's copy (decision 0004).
    let mut set_current = db.prepare_cached(
        "UPDATE attachment SET sha256 = (
             SELECT s.sha256 FROM attachment_source s JOIN source b ON b.id = s.source_id
             WHERE s.attachment_id = ?1 AND s.sha256 IS NOT NULL
             ORDER BY b.backup_date DESC LIMIT 1)
         WHERE id = ?1",
    )?;
    let mut add_issue = db.prepare_cached(
        "INSERT INTO import_issue (run_id, kind, guid, source_rowid, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;

    let started = Instant::now();
    let mut last_progress = Instant::now();
    let mut batch = Batch::new(db);
    for w in &work {
        if opts.stop.load(Ordering::SeqCst) || opts.limit.is_some_and(|l| counts.files_read >= l) {
            break;
        }
        let src = backup
            .root
            .join(&w.backup_file_id[..2])
            .join(&w.backup_file_id);
        counts.files_read += 1;
        let stored = blobs::store(&src, data_dir);
        batch.begin()?;
        match stored {
            Err(e) => {
                counts.read_failures += 1;
                tracing::warn!(guid = %w.guid, error_kind = ?e.kind(), "could not read attachment file");
                add_issue.execute(params![
                    run_id,
                    issue::ATTACHMENT_READ_FAILED,
                    w.guid,
                    w.source_rowid,
                    format!("{:?}", e.kind())
                ])?;
            }
            Ok(s) => {
                counts.bytes_read += s.size;
                let added = add_blob.execute(params![
                    s.sha256,
                    s.size as i64,
                    w.kind.as_str(),
                    now_ns()
                ])? == 1;
                if added {
                    // Also covers a blob an interrupted run renamed into
                    // place but did not get to record.
                    counts.blobs_added += 1;
                    counts.bytes_stored += s.size;
                    for (kind, recipe) in w.kind.derivatives() {
                        queue.execute(params![s.sha256, kind, recipe])?;
                        counts.derivatives_queued += usize::from(*kind != "thumb");
                    }
                } else {
                    counts.duplicates_skipped += 1;
                }
                set_hash.execute(params![w.attachment_id, source_id, s.sha256])?;
                set_current.execute([w.attachment_id])?;
            }
        }
        batch.done_one(BATCH_FILES)?;
        if last_progress.elapsed() >= PROGRESS_EVERY {
            last_progress = Instant::now();
            let secs = started.elapsed().as_secs_f64().max(0.001);
            tracing::info!(
                done = counts.files_read,
                of = work.len(),
                gb_read = format!("{:.2}", counts.bytes_read as f64 / 1e9),
                mb_per_s = format!("{:.1}", counts.bytes_read as f64 / 1e6 / secs),
                duplicates = counts.duplicates_skipped,
                "copying"
            );
        }
    }
    batch.commit()?;
    Ok(())
}

struct ThumbJob {
    sha256: String,
    kind: MediaKind,
}

struct ThumbResult {
    sha256: String,
    tmp: PathBuf,
    outcome: Result<u64, String>,
}

/// Sets failed derivatives of `kind` back to pending when a decoder older
/// than this build's tried them, so a decoder fix reaches files that failed
/// before it. Returns how many were set back. The background queue does the
/// same for its kinds.
pub fn retry_failed(db: &Connection, kind: &str) -> Result<usize> {
    Ok(db.execute(
        "UPDATE derivative SET status = 'pending', error = NULL, made_at = NULL
         WHERE kind = ?1 AND status = 'failed' AND decoder < ?2",
        params![kind, media::input::DECODER],
    )?)
}

/// Makes every pending thumbnail, `opts.jobs` at a time. This includes ones
/// an earlier, interrupted run queued but did not make, and failed ones an
/// older decoder tried.
fn make_thumbnails(
    db: &Connection,
    data_dir: &Path,
    opts: &Options,
    counts: &mut Counts,
) -> Result<()> {
    counts.thumbnails_retried = retry_failed(db, "thumb")?;
    if counts.thumbnails_retried > 0 {
        tracing::info!(
            thumbnails = counts.thumbnails_retried,
            decoder = media::input::DECODER,
            "retrying thumbnails an older decoder could not make"
        );
    }
    let jobs: VecDeque<ThumbJob> = db
        .prepare(
            "SELECT d.sha256, b.media_kind FROM derivative d JOIN blob b ON b.sha256 = d.sha256
             WHERE d.kind = 'thumb' AND d.status = 'pending' ORDER BY d.sha256",
        )?
        .query_map([], |r| {
            Ok(ThumbJob {
                sha256: r.get(0)?,
                kind: MediaKind::parse(&r.get::<_, String>(1)?),
            })
        })?
        .collect::<Result<_, _>>()?;
    if jobs.is_empty() {
        return Ok(());
    }
    let total = jobs.len();
    let workers = opts.jobs.max(1).min(total);
    tracing::info!(thumbnails = total, workers, "making thumbnails");
    let jobs = Mutex::new(jobs);
    let tmp_dir = blobs::tmp_dir(data_dir, DERIVED_DIR);
    let (tx, rx) = mpsc::channel::<ThumbResult>();

    let mut done = db.prepare_cached(
        "UPDATE derivative SET status = 'done', recipe = ?2, path = ?3, bytes = ?4, error = NULL, made_at = ?5, decoder = ?6
         WHERE sha256 = ?1 AND kind = 'thumb'",
    )?;
    let mut failed = db.prepare_cached(
        "UPDATE derivative SET status = 'failed', recipe = ?2, error = ?3, made_at = ?4, decoder = ?5
         WHERE sha256 = ?1 AND kind = 'thumb'",
    )?;

    std::thread::scope(|scope| -> Result<()> {
        for _ in 0..workers {
            let tx = tx.clone();
            let (jobs, tmp_dir, stop) = (&jobs, &tmp_dir, &opts.stop);
            scope.spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Some(job) = jobs.lock().expect("job queue").pop_front() else {
                        break;
                    };
                    let src = blobs::blob_path(data_dir, &job.sha256);
                    let tmp = blobs::tmp_path(tmp_dir, ".webp");
                    let outcome = media::thumbnail(&src, &tmp, job.kind, tmp_dir);
                    let result = ThumbResult {
                        sha256: job.sha256,
                        tmp,
                        outcome,
                    };
                    if tx.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut batch = Batch::new(db);
        let mut last_progress = Instant::now();
        for r in rx {
            // A tool killed by the same Ctrl-C did not fail on this file:
            // leave its thumbnail pending for the next run.
            if r.outcome.is_err() && opts.stop.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&r.tmp);
                continue;
            }
            batch.begin()?;
            match r.outcome {
                Ok(bytes) => {
                    let rel = blobs::derived_relative_path("thumb", &r.sha256, "webp");
                    let dest = data_dir.join(&rel);
                    std::fs::create_dir_all(dest.parent().expect("derived paths have a parent"))?;
                    std::fs::rename(&r.tmp, &dest)
                        .with_context(|| format!("moving a thumbnail into {}", dest.display()))?;
                    done.execute(params![
                        r.sha256,
                        media::THUMB_RECIPE,
                        rel,
                        bytes as i64,
                        now_ns(),
                        media::input::DECODER
                    ])?;
                    counts.thumbnails_made += 1;
                }
                Err(why) => {
                    let _ = std::fs::remove_file(&r.tmp);
                    // Store paths relative to the data directory.
                    let why = why.replace(&format!("{}/", data_dir.display()), "");
                    tracing::warn!(sha256 = %r.sha256, "thumbnail failed");
                    failed.execute(params![
                        r.sha256,
                        media::THUMB_RECIPE,
                        why,
                        now_ns(),
                        media::input::DECODER
                    ])?;
                    counts.thumbnails_failed += 1;
                }
            }
            batch.done_one(50)?;
            if last_progress.elapsed() >= PROGRESS_EVERY {
                last_progress = Instant::now();
                tracing::info!(
                    done = counts.thumbnails_made + counts.thumbnails_failed,
                    of = total,
                    failed = counts.thumbnails_failed,
                    "making thumbnails"
                );
            }
        }
        batch.commit()
    })?;
    Ok(())
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}
