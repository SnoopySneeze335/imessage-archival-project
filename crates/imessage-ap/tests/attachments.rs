//! The attachment step (Milestone 3a, decision 0005) against the fixture
//! backups: content-addressed copies, resuming, read-only backups,
//! thumbnails and the derivative queue. Needs ImageMagick with HEIC support
//! and ffmpeg (`scripts/cloud-setup.sh` installs them; so does CI).

mod common;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use common::{archive_db, canonical_dump, count, downgrade_to_v2, downgrade_to_v3, issues};
use fixture_gen::{Expected, Variant, file_id, generate_variant, scenario};
use imessage_ap::{
    archive,
    attachments::{self, Options, Summary},
    blobs, ingest, stats,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    v1: PathBuf,
    v2: PathBuf,
    expected: Expected,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let (v1, v2) = (dir.path().join("backup-v1"), dir.path().join("backup-v2"));
    let expected = generate_variant(&v1, Variant::V1).unwrap();
    generate_variant(&v2, Variant::V2).unwrap();
    Env {
        dir,
        v1,
        v2,
        expected,
    }
}

impl Env {
    fn data(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Ingests then copies each backup in `order` into the data dir `name`.
    fn import(&self, name: &str, order: &[&Path]) -> PathBuf {
        let data = self.data(name);
        for b in order {
            ingest::run(b, &data).unwrap();
            copy(b, &data, &Options::default());
        }
        data
    }
}

fn copy(backup: &Path, data: &Path, opts: &Options) -> Summary {
    attachments::run(backup, data, opts).unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    blobs::hex(&Sha256::digest(bytes))
}

/// Every file under `root`, by relative path, with its content.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().display().to_string(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    out
}

/// The blob and derived stores, without temporary folders.
fn stores(data: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for store in [blobs::BLOBS_DIR, blobs::DERIVED_DIR] {
        for (k, v) in tree(&data.join(store)) {
            if !k.starts_with(blobs::TMP_DIR) {
                out.insert(format!("{store}/{k}"), v);
            }
        }
    }
    out
}

/// Present file contents of the v1 scenario, by transfer name.
fn v1_files() -> BTreeMap<String, Vec<u8>> {
    scenario::build()
        .attachments
        .into_iter()
        .filter(|a| a.filename.is_some())
        .filter_map(|a| a.content.map(|c| (a.transfer_name, c)))
        .collect()
}

#[test]
fn every_file_is_stored_once_by_content() {
    let e = env();
    let data = e.import("data", &[&e.v1]);
    let x = &e.expected;
    let a = stats::collect(&data).unwrap().archive;
    assert_eq!(a.attachment_files_copied as usize, x.attachments_present);
    assert_eq!(a.attachment_bytes_copied, x.attachment_bytes_present);
    assert_eq!(a.attachment_files_pending_copy, 0);
    assert_eq!(a.blobs as usize, x.attachment_files_unique);
    assert_eq!(a.blob_bytes, x.attachment_bytes_unique);
    assert_eq!(
        a.duplicates_skipped as usize,
        x.attachments_present - x.attachment_files_unique
    );
    assert!(
        a.duplicates_skipped >= 1,
        "the fixture has a duplicate file"
    );

    // Each blob is at blobs/ab/cd/<sha256> and holds exactly that content.
    let stored = tree(&data.join(blobs::BLOBS_DIR));
    let real: Vec<_> = stored
        .iter()
        .filter(|(k, _)| !k.starts_with(blobs::TMP_DIR))
        .collect();
    assert_eq!(real.len(), x.attachment_files_unique);
    for (path, bytes) in real {
        let sha = sha256(bytes);
        assert_eq!(path, &format!("{}/{}/{sha}", &sha[..2], &sha[2..4]));
    }

    // Each attachment with a file points at its content's blob.
    let db = archive_db(&data);
    for (name, content) in v1_files() {
        let got: Option<String> = db
            .query_row(
                "SELECT sha256 FROM attachment WHERE transfer_name = ?1",
                [&name],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(got.as_deref(), Some(sha256(&content).as_str()), "{name}");
    }
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM attachment WHERE missing IS NULL AND sha256 IS NULL"
        ),
        0
    );
    // Files missing from the backup are still flagged, and have no blob.
    assert_eq!(
        a.attachment_files_missing as usize,
        x.attachments_missing_file
    );
    assert_eq!(
        a.attachments_without_path as usize,
        x.attachments_without_path
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM attachment WHERE missing IS NOT NULL AND sha256 IS NOT NULL"
        ),
        0
    );
}

#[test]
fn thumbnails_are_made_at_import_and_the_rest_is_queued() {
    let e = env();
    let data = e.import("data", &[&e.v1]);
    let db = archive_db(&data);
    let files = v1_files();
    let state = |name: &str, kind: &str| -> Option<String> {
        db.query_row(
            "SELECT status FROM derivative WHERE sha256 = ?1 AND kind = ?2",
            rusqlite::params![sha256(&files[name]), kind],
            |r| r.get(0),
        )
        .ok()
    };
    for name in [
        "IMG_0001.PNG",
        "IMG_0006.HEIC",
        "IMG_0010.MOV",
        "sticker.png",
        // Read through the decoder workarounds (decision 0006).
        "Sticker.heics",
        "IMG_0011.MP4",
    ] {
        assert_eq!(state(name, "thumb").as_deref(), Some("done"), "{name}");
    }
    // A damaged file fails its thumbnail without failing the run.
    assert_eq!(state("IMG_0008.JPG", "thumb").as_deref(), Some("failed"));
    let error: String = db
        .query_row(
            "SELECT error FROM derivative WHERE status = 'failed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(error.starts_with("convert:"), "{error}");
    assert!(!error.contains("IMG_0008") && !error.contains(&*data.to_string_lossy()));
    // Voice messages get no thumbnail; previews and conversions are queued.
    assert_eq!(state("Audio Message.caf", "thumb"), None);
    assert_eq!(
        state("Audio Message.caf", "audio").as_deref(),
        Some("pending")
    );
    assert_eq!(
        state("IMG_0006.HEIC", "preview").as_deref(),
        Some("pending")
    );
    assert_eq!(state("IMG_0010.MOV", "video").as_deref(), Some("pending"));
    assert_eq!(state("IMG_0011.MP4", "video").as_deref(), Some("pending"));
    assert_eq!(
        state("Sticker.heics", "preview").as_deref(),
        Some("pending")
    );
    assert_eq!(state("IMG_0001.PNG", "preview"), None);

    let a = stats::collect(&data).unwrap().archive;
    // Failed: the damaged JPEG, HEIC and MOV.
    assert_eq!((a.thumbnails_made, a.thumbnails_failed), (11, 3));
    assert_eq!((a.derivatives_pending, a.derivatives_failed), (12, 3));

    // Thumbnails are WebP files at derived/thumb/ab/cd/<sha256>.webp and are
    // never larger than the original's size.
    let mut stmt = db
        .prepare("SELECT sha256, path, bytes FROM derivative WHERE status = 'done'")
        .unwrap();
    let done: Vec<(String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(done.len(), 11);
    for (sha, path, bytes) in done {
        assert_eq!(
            path,
            format!("derived/thumb/{}/{}/{sha}.webp", &sha[..2], &sha[2..4])
        );
        let webp = fs::read(data.join(&path)).unwrap();
        assert_eq!(webp.len() as i64, bytes);
        assert!(webp.starts_with(b"RIFF") && &webp[8..12] == b"WEBP");
    }
}

#[test]
fn running_again_reads_nothing_and_changes_nothing() {
    let e = env();
    let data = e.import("data", &[&e.v1]);
    let (db_before, files_before) = (canonical_dump(&archive_db(&data)), stores(&data));
    let again = copy(&e.v1, &data, &Options::default());
    assert_eq!(again.counts.files_pending_at_start, 0);
    assert_eq!(again.counts.files_read, 0);
    assert_eq!(
        again.counts.thumbnails_made + again.counts.thumbnails_failed,
        0
    );
    assert!(canonical_dump(&archive_db(&data)) == db_before);
    assert!(stores(&data) == files_before);
}

#[test]
fn an_interrupted_copy_resumes_to_the_same_result() {
    let e = env();
    let whole = e.import("whole", &[&e.v1]);

    let data = e.data("resumed");
    ingest::run(&e.v1, &data).unwrap();
    let limited = Options {
        limit: Some(3),
        ..Options::default()
    };
    let first = copy(&e.v1, &data, &limited);
    assert_eq!(first.counts.files_read, 3);
    assert_eq!(
        first.counts.files_pending_at_end,
        e.expected.attachments_present - 3
    );
    // A stop request (Ctrl-C, docker stop) before any file is read.
    let stopped = copy(
        &e.v1,
        &data,
        &Options {
            stop: Arc::new(AtomicBool::new(true)),
            ..Options::default()
        },
    );
    assert!(stopped.interrupted);
    assert_eq!(stopped.counts.files_read, 0);
    let rest = copy(&e.v1, &data, &Options::default());
    assert!(!rest.interrupted);
    assert_eq!(
        rest.counts.files_read,
        e.expected.attachments_present - 3,
        "the resumed run read files the first one had already copied"
    );
    assert_eq!(rest.counts.files_pending_at_end, 0);

    assert!(canonical_dump(&archive_db(&data)) == canonical_dump(&archive_db(&whole)));
    assert!(stores(&data) == stores(&whole));
    let db = archive_db(&data);
    let runs: Vec<(String, Option<String>)> = db
        .prepare("SELECT status, error FROM import_run WHERE kind = 'attachments' ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(runs[0].0, "succeeded");
    assert_eq!(runs[1].0, "failed");
    assert!(runs[1].1.as_deref().unwrap().contains("interrupted"));
    assert_eq!(runs[2].0, "succeeded");
}

#[test]
fn leftovers_of_a_killed_run_are_cleaned_up_or_adopted() {
    let e = env();
    let whole = e.import("whole", &[&e.v1]);
    let data = e.data("killed");
    ingest::run(&e.v1, &data).unwrap();

    // What a run killed outright leaves: a half-written temporary file, a
    // blob renamed into place but never recorded, and a run still "running".
    let tmp = blobs::tmp_dir(&data, blobs::BLOBS_DIR);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("123-0"), b"half a file").unwrap();
    let png = &v1_files()["IMG_0001.PNG"];
    let orphan = blobs::blob_path(&data, &sha256(png));
    fs::create_dir_all(orphan.parent().unwrap()).unwrap();
    fs::write(&orphan, png).unwrap();
    archive_db(&data)
        .execute(
            "INSERT INTO import_run (kind, backup_folder, started_at, status, ingester_version, imessage_database_version)
             VALUES ('attachments', 'x', 0, 'running', 'x', 'x')",
            [],
        )
        .unwrap();

    let run = copy(&e.v1, &data, &Options::default());
    assert_eq!(run.counts.temporary_files_removed, 1);
    // The adopted blob is a new blob, not a duplicate.
    assert_eq!(run.counts.blobs_added, e.expected.attachment_files_unique);
    assert!(canonical_dump(&archive_db(&data)) == canonical_dump(&archive_db(&whole)));
    assert!(stores(&data) == stores(&whole));
    assert_eq!(
        count(
            &archive_db(&data),
            "SELECT COUNT(*) FROM import_run WHERE status = 'running'"
        ),
        0
    );
}

#[test]
fn backup_folder_is_never_written() {
    let e = env();
    let before = tree(&e.v1);
    set_read_only(&e.v1, true);
    let data = e.data("data");
    let result =
        ingest::run(&e.v1, &data).map(|_| attachments::run(&e.v1, &data, &Options::default()));
    set_read_only(&e.v1, false);
    let summary = result.unwrap().unwrap();
    assert_eq!(summary.counts.read_failures, 0);
    assert!(tree(&e.v1) == before, "the backup folder changed");
}

fn set_read_only(root: &Path, read_only: bool) {
    let mut stack = vec![root.to_path_buf()];
    let mut paths = vec![];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p.clone());
            }
            paths.push(p);
        }
        paths.push(dir);
    }
    for p in paths {
        let mut perm = fs::metadata(&p).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(read_only);
        fs::set_permissions(&p, perm).unwrap();
    }
}

#[test]
fn an_unreadable_file_is_reported_and_retried() {
    let e = env();
    let data = e.data("data");
    ingest::run(&e.v1, &data).unwrap();
    // The file disappears between ingest and copy.
    let sc = scenario::build();
    let heic = sc
        .attachments
        .iter()
        .find(|a| a.transfer_name == "IMG_0006.HEIC")
        .unwrap();
    let rel = heic.filename.as_ref().unwrap().trim_start_matches("~/");
    let id = file_id("MediaDomain", rel);
    let path = e.v1.join(&id[..2]).join(&id);
    let saved = e.dir.path().join("saved");
    fs::rename(&path, &saved).unwrap();

    let run = copy(&e.v1, &data, &Options::default());
    assert_eq!(run.counts.read_failures, 1);
    assert_eq!(run.counts.files_pending_at_end, 1);
    assert_eq!(
        issues(
            &archive_db(&data),
            run.run_id,
            attachments::issue::ATTACHMENT_READ_FAILED
        ),
        vec![heic.guid.clone()]
    );

    fs::rename(&saved, &path).unwrap();
    let retry = copy(&e.v1, &data, &Options::default());
    assert_eq!(
        (retry.counts.files_read, retry.counts.read_failures),
        (1, 0)
    );
    assert_eq!(retry.counts.files_pending_at_end, 0);
}

#[test]
fn files_need_their_backup_ingested_first() {
    let e = env();
    let data = e.data("data");
    let err = attachments::run(&e.v1, &data, &Options::default()).unwrap_err();
    assert!(format!("{err:#}").contains("ingest"), "{err:#}");
    assert!(!archive::path(&data).exists());

    ingest::run(&e.v1, &data).unwrap();
    let err = attachments::run(&e.v2, &data, &Options::default()).unwrap_err();
    assert!(format!("{err:#}").contains("not been imported"), "{err:#}");
}

#[test]
fn a_later_backup_fills_a_missing_file_in_either_order() {
    let e = env();
    let forward = e.import("forward", &[&e.v1, &e.v2]);
    let backward = e.import("backward", &[&e.v2, &e.v1]);
    assert!(canonical_dump(&archive_db(&forward)) == canonical_dump(&archive_db(&backward)));
    assert!(stores(&forward) == stores(&backward));

    // IMG_0002.HEIC is missing from backup 1 and present in backup 2.
    let db = archive_db(&forward);
    let (missing, sha): (Option<String>, Option<String>) = db
        .query_row(
            "SELECT missing, sha256 FROM attachment WHERE transfer_name = 'IMG_0002.HEIC'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(missing, None);
    let sha = sha.unwrap();
    assert!(blobs::blob_path(&forward, &sha).is_file());
    // Files both backups hold are stored once.
    let a = stats::collect(&forward).unwrap().archive;
    assert_eq!(a.attachment_files_pending_copy, 0);
    assert_eq!(a.blobs as usize, e.expected.attachment_files_unique);
    assert_eq!(a.duplicates_skipped, a.attachment_files_copied - a.blobs);
}

#[test]
fn file_runs_leave_the_import_counts_alone() {
    let e = env();
    let data = e.data("data");
    ingest::run(&e.v1, &data).unwrap();
    let before = stats::collect(&data).unwrap();
    copy(&e.v1, &data, &Options::default());
    let after = stats::collect(&data).unwrap();
    assert_eq!(after.archive.import_runs_succeeded, 1);
    assert_eq!(after.archive.import_runs_failed, 0);
    assert_eq!(after.archive.messages, before.archive.messages);
    assert_eq!(after.sources[0].diagnostics, before.sources[0].diagnostics);
    assert!(after.sources[0].diagnostics.is_some());
}

#[test]
fn stats_and_issues_never_name_files() {
    let e = env();
    let data = e.import("data", &[&e.v1]);
    let s = stats::collect(&data).unwrap();
    let text = stats::render(&s) + &serde_json::to_string(&s).unwrap();
    assert!(text.contains("Unique blobs"));
    let db = archive_db(&data);
    let details: String = db
        .query_row(
            "SELECT COALESCE(group_concat(COALESCE(i.detail, '') || COALESCE(r.error, ''), ' '), '')
             FROM import_run r LEFT JOIN import_issue i ON i.run_id = r.id",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    for a in scenario::build().attachments {
        assert!(!text.contains(&a.transfer_name), "stats named a file");
        assert!(!details.contains(&a.transfer_name), "a run named a file");
    }
}

#[test]
fn schema_2_archive_migrates_and_then_copies() {
    let e = env();
    let want = e.import("fresh", &[&e.v1]);
    let data = e.data("migrated");
    ingest::run(&e.v1, &data).unwrap();
    downgrade_to_v2(&archive_db(&data));
    assert_eq!(archive::migrations().len(), 5);
    copy(&e.v1, &data, &Options::default());
    assert!(canonical_dump(&archive_db(&data)) == canonical_dump(&archive_db(&want)));
}

/// The 3a real check: thumbnails an older decoder failed on are retried once
/// by the next run after a fix, and only those (decision 0006).
#[test]
fn thumbnails_failed_by_an_older_decoder_are_retried_once() {
    let e = env();
    let want = e.import("fresh", &[&e.v1]);
    let data = e.import("old", &[&e.v1]);
    let files = v1_files();
    let fixed = [&files["Sticker.heics"], &files["IMG_0011.MP4"]].map(|b| sha256(b));

    // What a 3a archive holds: those two failed, and no decoder column.
    let db = archive_db(&data);
    for sha in &fixed {
        let rel = blobs::derived_relative_path("thumb", sha, "webp");
        fs::remove_file(data.join(&rel)).unwrap();
        db.execute(
            "UPDATE derivative SET status = 'failed', path = NULL, bytes = NULL, error = 'old'
             WHERE sha256 = ?1 AND kind = 'thumb'",
            [sha],
        )
        .unwrap();
    }
    downgrade_to_v3(&db);
    drop(db);

    let run = copy(&e.v1, &data, &Options::default());
    assert_eq!(run.counts.files_read, 0);
    // Both fixed files and the three damaged ones are tried again; the
    // damaged ones still fail, now recorded against this decoder.
    assert_eq!(run.counts.thumbnails_retried, 5);
    assert_eq!(
        (run.counts.thumbnails_made, run.counts.thumbnails_failed),
        (2, 3)
    );
    let a = stats::collect(&data).unwrap().archive;
    assert_eq!((a.thumbnails_made, a.thumbnails_failed), (11, 3));
    assert!(stores(&data) == stores(&want));

    let again = copy(&e.v1, &data, &Options::default());
    assert_eq!(again.counts.thumbnails_retried, 0);
    assert_eq!(
        again.counts.thumbnails_made + again.counts.thumbnails_failed,
        0
    );
}
