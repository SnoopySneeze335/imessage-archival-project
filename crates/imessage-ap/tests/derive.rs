//! The background derive queue (Milestone 3b, decision 0007) against the
//! fixture backup: each kind converts or is left as it is, runs resume after
//! a stop, a finished queue does no work, damaged files fail and are retried
//! after a decoder change, and outputs are owned by the user the run is.
//! Needs ImageMagick with HEIC support and ffmpeg, as the attachment tests.

mod common;

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

use common::{archive_db, canonical_dump, count, downgrade_to_v4};
use fixture_gen::{Variant, generate_variant, scenario};
use imessage_ap::{
    attachments, blobs,
    derive::{self, Options, Summary},
    ingest,
    media::convert::Kind,
    stats,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// A fixture backup, ingested and copied, with its backup folder deleted so
/// nothing can read it.
fn imported(dir: &TempDir, name: &str) -> PathBuf {
    let backup = dir.path().join(format!("{name}-backup"));
    let data = dir.path().join(name);
    generate_variant(&backup, Variant::V1).unwrap();
    ingest::run(&backup, &data).unwrap();
    attachments::run(&backup, &data, &attachments::Options::default()).unwrap();
    fs::remove_dir_all(&backup).unwrap();
    data
}

fn derive(data: &Path, opts: &Options) -> Summary {
    derive::run(data, opts).unwrap()
}

fn only(kind: Kind) -> Options {
    Options {
        kinds: vec![kind],
        ..Options::default()
    }
}

/// Derivative state of the v1 attachment called `name`:
/// (status, action, path).
fn state(data: &Path, name: &str, kind: &str) -> (String, Option<String>, Option<String>) {
    let content = scenario::build()
        .attachments
        .into_iter()
        .find(|a| a.transfer_name == name)
        .and_then(|a| a.content)
        .unwrap_or_else(|| panic!("no fixture file {name}"));
    let sha = blobs::hex(&Sha256::digest(&content));
    archive_db(data)
        .query_row(
            "SELECT status, action, path FROM derivative WHERE sha256 = ?1 AND kind = ?2",
            [&sha, kind],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or_else(|e| panic!("{name} {kind}: {e}"))
}

fn ffprobe(path: &Path, entries: &str) -> String {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-of",
            "default=nw=1",
            "-show_entries",
            entries,
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffprobe {}", path.display());
    String::from_utf8(out.stdout).unwrap()
}

/// Every file under `data/derived`, without the temporary folder.
fn derived(data: &Path) -> BTreeMap<String, Vec<u8>> {
    let root = data.join(blobs::DERIVED_DIR);
    let mut out = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            let rel = p.strip_prefix(&root).unwrap().display().to_string();
            if rel.starts_with(blobs::TMP_DIR) {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn kind_stats(data: &Path, kind: &str) -> stats::DerivativeStats {
    let mut a = stats::collect(data).unwrap().archive;
    a.derivatives.remove(kind).unwrap()
}

#[test]
fn each_kind_is_converted_or_left_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let data = imported(&dir, "data");
    let before = stats::collect(&data).unwrap().archive;
    assert_eq!(before.derivatives_pending, 12);

    let run = derive(&data, &Options::default());
    assert!(!run.interrupted && run.low_disk.is_none());
    assert_eq!(run.counts.files_started, 12);

    // Images: HEIC and a HEIF sequence (through the thumbnail fallback)
    // become WebP; a file labelled HEIC that holds a PNG is used as is.
    let s = kind_stats(&data, "preview");
    assert_eq!((s.made, s.original, s.failed, s.pending), (2, 1, 1, 0));
    for name in ["IMG_0006.HEIC", "Sticker.heics"] {
        let (status, action, path) = state(&data, name, "preview");
        assert_eq!(
            (status.as_str(), action.as_deref()),
            ("done", Some("transcode"))
        );
        let path = path.unwrap();
        assert!(path.starts_with("derived/preview/") && path.ends_with(".webp"));
        let webp = fs::read(data.join(&path)).unwrap();
        assert!(
            webp.starts_with(b"RIFF") && &webp[8..12] == b"WEBP",
            "{name}"
        );
    }
    let (status, action, path) = state(&data, "IMG_0016.HEIC", "preview");
    assert_eq!(
        (status.as_str(), action.as_deref(), path),
        ("done", Some("original"), None)
    );

    // Video: HEVC (SDR and HDR) to H.264; the chnl file remuxed; a browser
    // ready MP4 used as is.
    let s = kind_stats(&data, "video");
    assert_eq!((s.made, s.original, s.failed, s.pending), (3, 1, 1, 0));
    for (name, action) in [
        ("IMG_0010.MOV", "transcode"),
        ("IMG_0014.MOV", "transcode"),
        ("IMG_0011.MP4", "remux"),
    ] {
        let (status, got, path) = state(&data, name, "video");
        assert_eq!(
            (status.as_str(), got.as_deref()),
            ("done", Some(action)),
            "{name}"
        );
        let file = data.join(path.unwrap());
        let info = ffprobe(
            &file,
            "stream=codec_name,pix_fmt,color_transfer:format=format_name",
        );
        assert!(info.contains("codec_name=h264"), "{name}: {info}");
        assert!(info.contains("pix_fmt=yuv420p"), "{name}: {info}");
        assert!(info.contains("format_name=mov,mp4"), "{name}: {info}");
        if name == "IMG_0014.MOV" {
            // Tone-mapped from HLG to SDR.
            assert!(info.contains("color_transfer=bt709"), "{info}");
        }
        if name == "IMG_0011.MP4" {
            assert!(info.contains("codec_name=aac"), "{info}");
        }
    }
    let (status, action, path) = state(&data, "IMG_0013.MP4", "video");
    assert_eq!(
        (status.as_str(), action.as_deref(), path),
        ("done", Some("original"), None)
    );

    // Voice messages: PCM converted, AAC copied out of CAF.
    let s = kind_stats(&data, "audio");
    assert_eq!((s.made, s.original, s.failed, s.pending), (2, 0, 1, 0));
    for (name, action) in [
        ("Audio Message.caf", "transcode"),
        ("Audio Message 2.caf", "remux"),
    ] {
        let (status, got, path) = state(&data, name, "audio");
        assert_eq!(
            (status.as_str(), got.as_deref()),
            ("done", Some(action)),
            "{name}"
        );
        let path = path.unwrap();
        assert!(path.ends_with(".m4a"));
        let info = ffprobe(&data.join(path), "stream=codec_name:format=format_name");
        assert!(
            info.contains("codec_name=aac") && info.contains("mov,mp4"),
            "{info}"
        );
    }

    // Damaged files are failed with an error that names no data directory.
    let db = archive_db(&data);
    for name in ["IMG_0012.HEIC", "IMG_0015.MOV", "Audio Message 3.caf"] {
        let kind = match name {
            "IMG_0012.HEIC" => "preview",
            "IMG_0015.MOV" => "video",
            _ => "audio",
        };
        assert_eq!(state(&data, name, kind).0, "failed", "{name}");
    }
    let leaked: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM derivative WHERE error LIKE ?1",
            [format!("%{}%", data.display())],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);

    // Stats add up, and the byte total is what is on disk.
    let a = stats::collect(&data).unwrap().archive;
    assert_eq!(a.derivatives_pending, 0);
    assert_eq!(a.derivatives_failed, 3 + 3);
    let on_disk: u64 = derived(&data).values().map(|b| b.len() as u64).sum();
    assert_eq!(a.derivative_bytes, on_disk);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM derive_run WHERE status = 'succeeded'"
        ),
        1
    );
}

#[test]
fn a_finished_queue_does_no_work() {
    let dir = tempfile::tempdir().unwrap();
    let data = imported(&dir, "data");
    derive(&data, &Options::default());
    let files = derived(&data);
    let dump = canonical_dump(&archive_db(&data));

    let again = derive(&data, &Options::default());
    assert_eq!(again.counts.files_started, 0);
    for k in again.counts.kinds.values() {
        assert_eq!(
            (k.pending_at_start, k.retried, k.made, k.failed),
            (0, 0, 0, 0)
        );
    }
    assert!(derived(&data) == files);
    assert!(canonical_dump(&archive_db(&data)) == dump);
}

#[test]
fn kinds_run_one_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let data = imported(&dir, "data");
    let run = derive(&data, &only(Kind::Video));
    assert_eq!(run.counts.files_started, 5);
    assert_eq!(kind_stats(&data, "video").pending, 0);
    assert_eq!(kind_stats(&data, "preview").pending, 4);
    assert_eq!(kind_stats(&data, "audio").pending, 3);
    assert_eq!(derive(&data, &only(Kind::Audio)).counts.files_started, 3);
    assert_eq!(derive(&data, &only(Kind::Preview)).counts.files_started, 4);
    assert_eq!(
        stats::collect(&data).unwrap().archive.derivatives_pending,
        0
    );
}

#[test]
fn a_stopped_run_resumes_to_the_same_result() {
    let dir = tempfile::tempdir().unwrap();
    let whole = imported(&dir, "whole");
    derive(&whole, &Options::default());

    let data = imported(&dir, "resumed");
    // A stop request (Ctrl-C, docker stop) arriving while files convert:
    // the file in progress is finished and recorded, then the run ends.
    let stopped = derive(
        &data,
        &Options {
            jobs: 1,
            stop_after: Some(2),
            ..Options::default()
        },
    );
    assert!(stopped.interrupted);
    assert_eq!(stopped.counts.files_started, 2);
    assert_eq!(stopped.counts.total(|k| k.pending_at_end), 10);
    let status: String = archive_db(&data)
        .query_row("SELECT status FROM derive_run ORDER BY id DESC", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "failed");

    // A stop before anything starts, then --limit, then the rest.
    let none = derive(
        &data,
        &Options {
            stop: Arc::new(AtomicBool::new(true)),
            ..Options::default()
        },
    );
    assert!(none.interrupted);
    assert_eq!(none.counts.files_started, 0);
    let limited = derive(
        &data,
        &Options {
            limit: Some(3),
            ..Options::default()
        },
    );
    assert_eq!(limited.counts.files_started, 3);
    assert_eq!(limited.counts.total(|k| k.pending_at_end), 7);
    // A partly written file left by a killed run is removed.
    let tmp = blobs::tmp_dir(&data, blobs::DERIVED_DIR);
    fs::write(tmp.join("killed.mp4"), b"partial").unwrap();
    let rest = derive(&data, &Options::default());
    assert!(!rest.interrupted);
    assert_eq!(rest.counts.temporary_files_removed, 1);
    assert_eq!(rest.counts.files_started, 7);

    assert!(canonical_dump(&archive_db(&data)) == canonical_dump(&archive_db(&whole)));
    assert!(derived(&data) == derived(&whole));
}

#[test]
fn damaged_files_fail_and_are_retried_after_a_decoder_change() {
    let dir = tempfile::tempdir().unwrap();
    let data = imported(&dir, "data");
    let first = derive(&data, &Options::default());
    assert_eq!(first.counts.total(|k| k.failed), 3);
    assert_eq!(first.counts.total(|k| k.pending_at_end), 0);

    // As after a decoder fix (decision 0006): failures recorded by an older
    // decoder are tried again by the next ordinary run, once.
    let db = archive_db(&data);
    db.execute(
        "UPDATE derivative SET decoder = decoder - 1 WHERE status = 'failed'",
        [],
    )
    .unwrap();
    drop(db);
    let retry = derive(&data, &Options::default());
    assert_eq!(
        retry.counts.total(|k| k.retried),
        3,
        "previews, video and audio only"
    );
    for kind in ["preview", "video", "audio"] {
        assert_eq!(retry.counts.kinds[kind].retried, 1, "{kind}");
        assert_eq!(retry.counts.kinds[kind].failed, 1, "{kind}");
    }
    // Thumbnails belong to the attachment step and are not touched here.
    assert_eq!(kind_stats(&data, "thumb").failed, 3);
    let again = derive(&data, &Options::default());
    assert_eq!(again.counts.total(|k| k.retried), 0);
    assert_eq!(again.counts.files_started, 0);
}

#[test]
fn a_nearly_full_disk_stops_the_run_without_failing_files() {
    let dir = tempfile::tempdir().unwrap();
    let data = imported(&dir, "data");
    let run = derive(
        &data,
        &Options {
            min_free: u64::MAX / 4,
            ..Options::default()
        },
    );
    let low = run.low_disk.expect("stopped for disk space");
    assert!(low.needed > low.free);
    assert!(!run.interrupted);
    assert_eq!(run.counts.files_started, 0);
    let a = stats::collect(&data).unwrap().archive;
    assert_eq!((a.derivatives_pending, a.derivatives_failed), (12, 3));
    // With room again, the next run does everything.
    assert_eq!(derive(&data, &Options::default()).counts.files_started, 12);
}

#[test]
fn a_plan_converts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let data = imported(&dir, "data");
    let dump = canonical_dump(&archive_db(&data));
    let files = derived(&data);
    let report = derive::plan(&data, &Options::default()).unwrap();
    let files_of = |kind: &str, action: &str| report.rows[kind].get(action).map_or(0, |r| r.files);
    assert_eq!(
        files_of("preview", "transcode"),
        3,
        "the damaged HEIC is only found out by converting"
    );
    assert_eq!(files_of("preview", "original"), 1);
    assert_eq!(files_of("video", "transcode"), 2);
    assert_eq!(files_of("video", "remux"), 1);
    assert_eq!(files_of("video", "original"), 1);
    assert_eq!(files_of("video", "unreadable"), 1);
    assert_eq!(files_of("audio", "remux"), 1);
    assert_eq!(files_of("audio", "transcode"), 1);
    assert_eq!(files_of("audio", "unreadable"), 1);
    assert!(report.rows["video"]["transcode"].seconds > 1.0);
    assert!(canonical_dump(&archive_db(&data)) == dump);
    assert!(derived(&data) == files);
}

#[test]
fn a_schema_4_archive_migrates_and_then_derives() {
    let dir = tempfile::tempdir().unwrap();
    let want = imported(&dir, "fresh");
    derive(&want, &Options::default());
    let data = imported(&dir, "migrated");
    downgrade_to_v4(&archive_db(&data));
    derive(&data, &Options::default());
    assert!(canonical_dump(&archive_db(&data)) == canonical_dump(&archive_db(&want)));
}

/// Files are created by the user the run is: 99:100 in the image, where the
/// entrypoint drops to PUID:PGID. As root, this runs the real binary as
/// 99:100 the same way; otherwise it checks the current user.
#[test]
fn outputs_belong_to_the_user_the_run_is() {
    let dir = tempfile::tempdir().unwrap();
    let data = imported(&dir, "data");
    let probe = dir.path().join("whoami");
    fs::write(&probe, b"").unwrap();
    let me = fs::metadata(&probe).unwrap();
    let want = if me.uid() == 0 {
        chown_tree(dir.path(), 99, 100);
        let status = Command::new("setpriv")
            .args(["--reuid=99", "--regid=100", "--clear-groups", "--"])
            .arg(env!("CARGO_BIN_EXE_imessage-ap"))
            .args(["derive", "--data"])
            .arg(&data)
            .status()
            .expect("setpriv runs");
        assert!(status.success());
        (99, 100)
    } else {
        derive(&data, &Options::default());
        (me.uid(), me.gid())
    };
    let made = derived(&data);
    assert!(made.keys().any(|k| k.starts_with("video/")));
    for rel in made.keys() {
        let m = fs::metadata(data.join(blobs::DERIVED_DIR).join(rel)).unwrap();
        assert_eq!((m.uid(), m.gid()), want, "{rel}");
    }
    assert_eq!(
        stats::collect(&data).unwrap().archive.derivatives_pending,
        0
    );
}

fn chown_tree(root: &Path, uid: u32, gid: u32) {
    std::os::unix::fs::chown(root, Some(uid), Some(gid)).unwrap();
    if root.is_dir() {
        for entry in fs::read_dir(root).unwrap() {
            chown_tree(&entry.unwrap().path(), uid, gid);
        }
    }
}
