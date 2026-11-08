//! Two backups of the same phone (Milestone 2b, decision 0004): versions,
//! messages missing from a newer backup, and import-order independence.

mod common;

use std::path::{Path, PathBuf};

use common::{archive_db, canonical_dump, count, dump};
use fixture_gen::{Expected, Variant, generate_variant, scenario};
use imessage_ap::{archive, ingest, stats};
use rusqlite::Connection;
use tempfile::TempDir;

struct Backups {
    dir: TempDir,
    v1: PathBuf,
    v2: PathBuf,
    expected_v2: Expected,
}

fn backups() -> Backups {
    let dir = tempfile::tempdir().unwrap();
    let (v1, v2) = (dir.path().join("backup-v1"), dir.path().join("backup-v2"));
    generate_variant(&v1, Variant::V1).unwrap();
    let expected_v2 = generate_variant(&v2, Variant::V2).unwrap();
    Backups {
        dir,
        v1,
        v2,
        expected_v2,
    }
}

impl Backups {
    /// Imports `order` into a fresh data directory named `name`.
    fn import(&self, name: &str, order: &[&Path]) -> (PathBuf, Vec<ingest::Counts>) {
        let data = self.dir.path().join(name);
        let counts = order
            .iter()
            .map(|b| ingest::run(b, &data).unwrap().counts)
            .collect();
        (data, counts)
    }
}

fn guid_of(text: &str) -> String {
    scenario::build()
        .messages
        .into_iter()
        .find(|m| m.text.as_deref() == Some(text))
        .unwrap()
        .guid
}

/// Content versions of a message, oldest backup first: (text, is_unsent, backups).
fn versions(db: &Connection, guid: &str) -> Vec<(Option<String>, bool, String)> {
    let mut stmt = db
        .prepare(
            "SELECT v.text, v.is_unsent,
                    (SELECT group_concat(s.backup_date) FROM message_version_source vs
                     JOIN source s ON s.id = vs.source_id WHERE vs.version_id = v.id ORDER BY s.backup_date)
             FROM message_version v JOIN message m ON m.id = v.message_id
             WHERE m.guid = ?1",
        )
        .unwrap();
    let mut out: Vec<_> = stmt
        .query_map([guid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    out.sort_by(|a: &(Option<String>, bool, String), b| a.2.cmp(&b.2));
    out
}

fn dates() -> (String, String) {
    (
        Variant::V1.backup_date().to_string(),
        Variant::V2.backup_date().to_string(),
    )
}

#[test]
fn two_backups_in_either_order_give_the_same_archive() {
    let b = backups();
    let (forward, fc) = b.import("forward", &[&b.v1, &b.v2]);
    let (backward, bc) = b.import("backward", &[&b.v2, &b.v1]);
    let (f, r) = (
        canonical_dump(&archive_db(&forward)),
        canonical_dump(&archive_db(&backward)),
    );
    if f != r {
        let diff: Vec<String> = f
            .lines()
            .zip(r.lines())
            .filter(|(a, b)| a != b)
            .take(5)
            .map(|(a, b)| format!("  v1,v2: {a}\n  v2,v1: {b}"))
            .collect();
        panic!("import order changed the archive:\n{}", diff.join("\n"));
    }

    // The edit and the unsend after backup 1 are the only content changes,
    // whichever backup came first.
    assert_eq!(fc[1].messages_changed, 2);
    assert_eq!(bc[1].messages_changed, 2);
    // Backup 2's new messages, then backup 1's dropped message.
    let s1 = scenario::build().messages.len();
    let s2 = b.expected_v2.messages;
    assert_eq!(fc[1].messages_added, s2 - (s1 - 1));
    assert_eq!(bc[1].messages_added, 1);

    let s = stats::collect(&forward).unwrap();
    assert_eq!(s.archive.messages as usize, s2 + 1);
    assert_eq!(s.archive.messages_with_versions, 2);
    assert_eq!(s.archive.message_versions as usize, s2 + 1 + 2);
    assert_eq!(s.sources.len(), 2);
}

#[test]
fn changed_messages_keep_every_version() {
    let b = backups();
    let (data, _) = b.import("both", &[&b.v1, &b.v2]);
    let db = archive_db(&data);
    let (d1, d2) = dates();

    // Edited after backup 1: both texts kept, the newer one current.
    let edited = guid_of("Meet at the north trailhead");
    assert_eq!(
        versions(&db, &edited),
        [
            (
                Some("Meet at the north trailhead".into()),
                false,
                d1.clone()
            ),
            (
                Some("Meet at the south trailhead".into()),
                false,
                d2.clone()
            ),
        ]
    );
    let (text, edited_at): (Option<String>, Option<i64>) = db
        .query_row(
            "SELECT text, edited_at FROM message WHERE guid = ?1",
            [&edited],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(text.as_deref(), Some("Meet at the south trailhead"));
    assert!(edited_at.is_some());
    let history: Vec<String> = db
        .prepare(
            "SELECT e.text FROM message_edit e JOIN message m ON m.id = e.message_id
             WHERE m.guid = ?1 ORDER BY e.edited_at",
        )
        .unwrap()
        .query_map([&edited], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        history,
        ["Meet at the north trailhead", "Meet at the south trailhead"]
    );

    // Unsent after backup 1: the text survives in the older version.
    let unsent = guid_of("Wrong photo, ignore that");
    assert_eq!(
        versions(&db, &unsent),
        [
            (Some("Wrong photo, ignore that".into()), false, d1.clone()),
            (None, true, d2.clone()),
        ]
    );
    let is_unsent: bool = db
        .query_row(
            "SELECT is_unsent FROM message WHERE guid = ?1",
            [&unsent],
            |r| r.get(0),
        )
        .unwrap();
    assert!(is_unsent);

    // Read after backup 1: a status change updates the message, no new version.
    let late = guid_of("Delivered but never read");
    assert_eq!(versions(&db, &late).len(), 1);
    assert_eq!(versions(&db, &late)[0].2, format!("{d1},{d2}"));
    let read_at: Option<i64> = db
        .query_row(
            "SELECT read_at FROM message WHERE guid = ?1",
            [&late],
            |r| r.get(0),
        )
        .unwrap();
    assert!(read_at.is_some());
}

#[test]
fn records_missing_from_a_newer_backup_stay() {
    let b = backups();
    let (data, _) = b.import("both", &[&b.v1, &b.v2]);
    let db = archive_db(&data);
    let dropped = scenario::dropped_in_v2(&scenario::build());
    let (held_by, first): (String, String) = db
        .query_row(
            "SELECT (SELECT group_concat(s.backup_date) FROM message_source ms JOIN source s ON s.id = ms.source_id
                     WHERE ms.message_id = m.id),
                    (SELECT backup_date FROM source WHERE id = m.first_source_id)
             FROM message m WHERE m.guid = ?1",
            [&dropped],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)?.to_string())),
        )
        .unwrap();
    let (d1, _) = dates();
    assert_eq!(held_by, d1);
    assert_eq!(first, d1);
}

#[test]
fn newest_backup_names_the_chat_and_fills_missing_files() {
    let b = backups();
    for (name, order) in [("forward", [&b.v1, &b.v2]), ("backward", [&b.v2, &b.v1])] {
        let (data, _) = b.import(name, &order.map(PathBuf::as_path));
        let db = archive_db(&data);
        let group = &scenario::build().chats[5].guid;
        let (name_now, names): (Option<String>, i64) = db
            .query_row(
                "SELECT c.display_name,
                        (SELECT COUNT(DISTINCT sc.display_name) FROM source_chat sc WHERE sc.chat_id = c.id)
                 FROM chat c WHERE c.guid = ?1",
                [group],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(name_now.as_deref(), Some("Summit Society"), "{name}");
        assert_eq!(names, 2, "each backup's name is kept ({name})");

        let missing: Option<String> = db
            .query_row(
                "SELECT missing FROM attachment WHERE transfer_name = 'IMG_0002.HEIC'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(missing, None, "backup 2 has the file ({name})");
    }
}

#[test]
fn reimporting_either_backup_changes_nothing() {
    let b = backups();
    let (data, _) = b.import("both", &[&b.v1, &b.v2]);
    let before = dump(&archive_db(&data));
    for backup in [&b.v2, &b.v1] {
        let c = ingest::run(backup, &data).unwrap().counts;
        assert_eq!(
            (
                c.messages_added,
                c.message_versions_added,
                c.messages_changed,
                c.message_edits_added,
                c.reactions_added,
                c.attachments_added
            ),
            (0, 0, 0, 0, 0, 0)
        );
    }
    assert!(
        before == dump(&archive_db(&data)),
        "a re-import changed the archive"
    );
}

/// Turns a current archive back into schema 1: one copy per message in
/// `message_raw`, no versions or edit history.
fn downgrade_to_v1(db: &Connection) {
    db.execute_batch(
        "CREATE TABLE message_raw (
             message_id INTEGER PRIMARY KEY REFERENCES message (id),
             attributed_body BLOB,
             message_summary_info BLOB,
             payload_data BLOB
         );
         INSERT INTO message_raw SELECT message_id, attributed_body, message_summary_info, payload_data
             FROM message_version;
         DROP TABLE message_edit;
         DROP TABLE message_version_source;
         DROP TABLE message_version;
         PRAGMA user_version = 1;",
    )
    .unwrap();
}

#[test]
fn schema_1_archive_migrates_without_reimport() {
    let b = backups();
    let (data, _) = b.import("single", &[&b.v1]);
    let want = canonical_dump(&archive_db(&data));
    downgrade_to_v1(&archive_db(&data));
    assert_eq!(archive::migrations().len(), 2);

    // Opening for writing migrates; nothing is lost except the edit history,
    // which the next import of the same backup fills without new versions.
    drop(archive::open(&data).unwrap());
    let db = archive_db(&data);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM message_version"),
        count(&db, "SELECT COUNT(*) FROM message")
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM message_edit"), 0);
    let c = ingest::run(&b.v1, &data).unwrap().counts;
    assert_eq!((c.message_versions_added, c.messages_changed), (0, 0));
    assert!(c.message_edits_added > 0);
    assert!(
        canonical_dump(&archive_db(&data)) == want,
        "migrated archive differs from a fresh import"
    );
}
