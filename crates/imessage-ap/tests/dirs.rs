//! Stores in separate folders (decision 0009): the same archive as the
//! default layout, with each kind of file where its setting says.

mod common;

use std::{fs, path::Path};

use common::{canonical_dump, count};
use fixture_gen::{Variant, generate_variant};
use imessage_ap::{archive, attachments, derive, dirs::Dirs, ingest, stats};
use rusqlite::Connection;

fn run_all(backup: &Path, dirs: &Dirs) {
    ingest::run(backup, dirs).unwrap();
    attachments::run(backup, dirs, &attachments::Options::default()).unwrap();
    derive::run(dirs, &derive::Options::default()).unwrap();
}

fn files_under(dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.ends_with(".tmp"))
        .map(|p| if p.is_dir() { files_under(&p) } else { 1 })
        .sum()
}

#[test]
fn every_store_can_live_elsewhere() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("backup");
    generate_variant(&backup, Variant::V1).unwrap();

    let default = Dirs::under(&tmp.path().join("plain"));
    run_all(&backup, &default);

    let data = tmp.path().join("data");
    let split = Dirs::new(
        &data,
        Some(&tmp.path().join("ssd/db")),
        Some(&tmp.path().join("array/blobs")),
        Some(&tmp.path().join("ssd/derived")),
        Some(&tmp.path().join("exports")),
    );
    run_all(&backup, &split);

    // Same records and the same derivative rows (paths are recorded
    // relative to the data directory, so they match too).
    let a = Connection::open(default.archive_db()).unwrap();
    let b = Connection::open(split.archive_db()).unwrap();
    assert_eq!(canonical_dump(&a), canonical_dump(&b));
    let sa = serde_json::to_value(stats::collect(&default).unwrap().archive).unwrap();
    let sb = serde_json::to_value(stats::collect(&split).unwrap().archive).unwrap();
    assert_eq!(sa, sb);

    // Nothing landed in the data directory itself.
    assert!(!data.join("archive.db").exists());
    assert_eq!(
        files_under(&data),
        0,
        "files written under the data directory"
    );
    assert_eq!(
        files_under(&split.blobs),
        count(&b, "SELECT COUNT(*) FROM blob")
    );
    let made = count(
        &b,
        "SELECT COUNT(*) FROM derivative WHERE status = 'done' AND path IS NOT NULL",
    );
    assert!(made > 0);
    assert_eq!(files_under(&split.derived), made);
    let mut stmt = b
        .prepare("SELECT path FROM derivative WHERE path IS NOT NULL")
        .unwrap();
    for p in stmt.query_map([], |r| r.get::<_, String>(0)).unwrap() {
        let p = p.unwrap();
        assert!(p.starts_with("derived/"), "{p}");
        assert!(split.derived_file(&p).is_file(), "{p}");
    }
    // The lock lives beside archive.db.
    assert!(split.db.join(".imessage-ap.lock").exists());
}

#[test]
fn an_archive_left_in_the_data_directory_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("backup");
    generate_variant(&backup, Variant::V1).unwrap();
    let data = tmp.path().join("data");
    ingest::run(&backup, &Dirs::under(&data)).unwrap();

    // Moved by copying: the old archive.db is still there.
    let db = tmp.path().join("db");
    fs::create_dir_all(&db).unwrap();
    fs::copy(data.join("archive.db"), db.join("archive.db")).unwrap();
    let moved = Dirs::new(&data, Some(&db), None, None, None);
    for err in [
        ingest::run(&backup, &moved).map(drop).unwrap_err(),
        stats::collect(&moved).map(drop).unwrap_err(),
        archive::open(&moved).map(drop).unwrap_err(),
    ] {
        assert!(err.to_string().contains("exists in both"), "{err:#}");
    }
    fs::remove_file(data.join("archive.db")).unwrap();
    for f in ["archive.db-wal", "archive.db-shm"] {
        let _ = fs::remove_file(data.join(f));
    }
    stats::collect(&moved).unwrap();
}
