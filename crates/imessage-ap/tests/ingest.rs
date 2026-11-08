//! Ingest tests against the generated fixture backup.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use fixture_gen::{SMS_DB, file_id, generate, scenario};
use imessage_ap::{archive, ingest, stats};
use rusqlite::Connection;
use tempfile::TempDir;

struct Env {
    _dir: TempDir,
    backup: PathBuf,
    data: PathBuf,
    expected: fixture_gen::Expected,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("backup");
    let data = dir.path().join("data");
    let expected = generate(&backup).unwrap();
    Env {
        _dir: dir,
        backup,
        data,
        expected,
    }
}

fn archive_db(data: &Path) -> Connection {
    Connection::open(archive::path(data)).unwrap()
}

fn count(conn: &Connection, sql: &str) -> usize {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as usize
}

fn issues(conn: &Connection, run_id: i64, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT guid FROM import_issue WHERE run_id = ?1 AND kind = ?2 ORDER BY guid")
        .unwrap();
    stmt.query_map(rusqlite::params![run_id, kind], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Every data table's rows, in a stable order. Excludes the per-run tables.
fn dump(conn: &Connection) -> String {
    let mut tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT IN ('import_run', 'import_issue') ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables.retain(|t| !t.starts_with("sqlite_"));
    let mut out = String::new();
    for t in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {t}")).unwrap();
        let n = stmt.column_count();
        let mut rows: Vec<String> = stmt
            .query_map([], |r| {
                Ok((0..n)
                    .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.sort();
        out.push_str(&format!("## {t}\n{}\n", rows.join("\n")));
    }
    out
}

/// Path, size and content hash of every file under `root`.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).unwrap() {
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

#[test]
fn first_import_matches_fixture_expectations() {
    let e = env();
    let summary = ingest::run(&e.backup, &e.data).unwrap();
    let x = &e.expected;
    let c = &summary.counts;
    assert_eq!(c.messages_seen, x.messages);
    assert_eq!(c.messages_added, x.messages);
    assert_eq!(c.chats_added, x.chats);
    assert_eq!(c.handles_added, x.handles);
    assert_eq!(c.attachments_added, x.attachments);
    assert_eq!(c.reactions_added, x.reactions);

    let s = stats::collect(&e.data).unwrap();
    let a = &s.archive;
    assert_eq!(a.messages as usize, x.messages);
    assert_eq!(a.orphan_messages as usize, x.messages_without_chat);
    assert_eq!(a.messages_recovered_deleted as usize, x.recoverable_deleted);
    assert_eq!(a.messages_in_chat as usize, x.messages_in_chats);
    assert_eq!(a.reactions as usize, x.reactions);
    assert_eq!(a.replies as usize, x.replies);
    assert_eq!(a.chats as usize, x.chats);
    assert_eq!(a.chats_sharing_identifier as usize, x.duplicated_chats);
    assert_eq!(a.handles as usize, x.handles);
    assert_eq!(a.unique_addresses as usize, x.unique_addresses);
    assert_eq!(a.attachment_records as usize, x.attachments);
    assert_eq!(a.attachments_with_file as usize, x.attachments_present);
    assert_eq!(
        a.attachment_files_missing as usize,
        x.attachments_missing_file
    );
    assert_eq!(
        a.attachments_without_path as usize,
        x.attachments_without_path
    );
    assert_eq!(s.sources.len(), 1);
    assert_eq!(s.sources[0].handle_rows as usize, x.handles);
    assert_eq!(s.sources[0].ios_version.as_deref(), Some("26.1"));
    let d = s.sources[0].diagnostics.as_ref().unwrap();
    assert_eq!(d["messages"], x.messages);
    assert_eq!(d["chats"], x.chats);

    // Text survives, including messages whose text is only in attributedBody.
    let db = archive_db(&e.data);
    let mut stmt = db.prepare("SELECT guid, text FROM message").unwrap();
    let stored: HashMap<String, Option<String>> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for m in scenario::build().messages {
        assert_eq!(stored[&m.guid], m.text, "text of {}", m.guid);
    }
    assert!(
        count(
            &db,
            "SELECT COUNT(*) FROM message_raw WHERE attributed_body IS NOT NULL"
        ) > 0
    );
}

#[test]
fn reimport_changes_nothing() {
    let e = env();
    ingest::run(&e.backup, &e.data).unwrap();
    let before = dump(&archive_db(&e.data));
    let second = ingest::run(&e.backup, &e.data).unwrap();
    let after = dump(&archive_db(&e.data));
    assert!(before == after, "a second import changed the archive");

    let c = &second.counts;
    assert_eq!(
        (
            c.messages_added,
            c.chats_added,
            c.handles_added,
            c.attachments_added,
            c.reactions_added
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(c.messages_seen, e.expected.messages);
    let db = archive_db(&e.data);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM import_run WHERE status = 'succeeded'"
        ),
        2
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM source"), 1);
}

#[test]
fn reactions_link_to_their_targets() {
    let e = env();
    let run = ingest::run(&e.backup, &e.data).unwrap().run_id;
    let db = archive_db(&e.data);
    let reactions: Vec<_> = scenario::build()
        .messages
        .into_iter()
        .filter(|m| m.associated_message_type != 0)
        .collect();
    assert!(!reactions.is_empty());
    for m in &reactions {
        let expected_target = m
            .associated_message_guid
            .as_deref()
            .unwrap()
            .trim_start_matches("p:0/");
        let (target, removal, emoji, target_exists): (String, bool, Option<String>, bool) = db
            .query_row(
                "SELECT r.target_guid, r.is_removal, r.emoji,
                        EXISTS (SELECT 1 FROM message t WHERE t.guid = r.target_guid)
                 FROM reaction r JOIN message m ON m.id = r.message_id WHERE m.guid = ?1",
                [&m.guid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(target, expected_target, "{}", m.guid);
        assert!(target_exists, "target of {} is in the archive", m.guid);
        assert_eq!(removal, m.associated_message_type >= 3000, "{}", m.guid);
        assert_eq!(emoji, m.associated_message_emoji, "{}", m.guid);
    }
    assert!(
        reactions.iter().any(|m| m.associated_message_type >= 3000),
        "fixture has a removal"
    );
    assert!(
        reactions
            .iter()
            .any(|m| m.associated_message_emoji.is_some()),
        "fixture has an emoji reaction"
    );
    assert!(issues(&db, run, ingest::issue::REACTION_TARGET_MISSING).is_empty());
}

#[test]
fn orphan_messages_are_kept_and_reported() {
    let e = env();
    let run = ingest::run(&e.backup, &e.data).unwrap().run_id;
    let db = archive_db(&e.data);
    let s = scenario::build();
    let orphans: Vec<String> = s
        .messages
        .iter()
        .filter(|m| m.chat.is_none() && m.recoverable_in.is_none())
        .map(|m| m.guid.clone())
        .collect();
    assert!(!orphans.is_empty());
    assert_eq!(issues(&db, run, ingest::issue::ORPHAN_MESSAGE), orphans);
    for g in &orphans {
        let chat: Option<i64> = db
            .query_row("SELECT chat_id FROM message WHERE guid = ?1", [g], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(chat, None);
    }
    // A recently deleted message is not an orphan: it keeps its chat.
    let recovered = s
        .messages
        .iter()
        .find(|m| m.recoverable_in.is_some())
        .unwrap();
    let (chat_guid, flag): (String, bool) = db
        .query_row(
            "SELECT c.guid, m.is_recovered_deleted FROM message m JOIN chat c ON c.id = m.chat_id WHERE m.guid = ?1",
            [&recovered.guid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(flag);
    assert_eq!(
        chat_guid,
        s.chats[(recovered.recoverable_in.unwrap() - 1) as usize].guid
    );
}

#[test]
fn missing_attachment_files_are_flagged() {
    let e = env();
    let run = ingest::run(&e.backup, &e.data).unwrap().run_id;
    let db = archive_db(&e.data);
    let s = scenario::build();
    let mut missing_file = Vec::new();
    let mut no_path = Vec::new();
    for a in &s.attachments {
        let (missing, present, owner): (Option<String>, bool, String) = db
            .query_row(
                "SELECT a.missing, s.present, m.guid FROM attachment a
                 JOIN attachment_source s ON s.attachment_id = a.id
                 JOIN message_attachment ma ON ma.attachment_id = a.id
                 JOIN message m ON m.id = ma.message_id
                 WHERE a.guid = ?1",
                [&a.guid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let owner_expected = s
            .messages
            .iter()
            .find(|m| m.attachments.contains(&a.rowid))
            .unwrap();
        assert_eq!(owner, owner_expected.guid);
        match (&a.filename, &a.content) {
            (None, _) => {
                assert_eq!(missing.as_deref(), Some("no_path"));
                no_path.push(a.guid.clone());
            }
            (Some(_), None) => {
                assert_eq!(missing.as_deref(), Some("not_in_backup"));
                assert!(!present);
                missing_file.push(a.guid.clone());
            }
            (Some(_), Some(_)) => {
                assert_eq!(missing, None, "{}", a.guid);
                assert!(present);
            }
        }
    }
    assert!(!missing_file.is_empty() && !no_path.is_empty());
    missing_file.sort();
    no_path.sort();
    assert_eq!(
        issues(&db, run, ingest::issue::MISSING_ATTACHMENT_FILE),
        missing_file
    );
    assert_eq!(issues(&db, run, ingest::issue::ATTACHMENT_NO_PATH), no_path);
}

#[test]
fn backup_folder_is_never_written() {
    let e = env();
    let before = tree(&e.backup);
    // Remove write permission everywhere (effective when not running as root).
    set_read_only(&e.backup, true);
    let result = ingest::run(&e.backup, &e.data);
    set_read_only(&e.backup, false);
    result.unwrap();
    assert!(tree(&e.backup) == before, "the backup folder changed");
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
fn encrypted_backups_are_refused_before_anything_is_written() {
    let e = env();
    let path = e.backup.join("Manifest.plist");
    let mut manifest = plist::Value::from_file(&path)
        .unwrap()
        .into_dictionary()
        .unwrap();
    manifest.insert("IsEncrypted".into(), true.into());
    plist::Value::Dictionary(manifest)
        .to_file_binary(&path)
        .unwrap();
    let err = ingest::run(&e.backup, &e.data).unwrap_err();
    assert!(format!("{err:#}").contains("encrypted"), "{err:#}");
    assert!(!archive::path(&e.data).exists());
}

#[test]
fn failed_import_is_recorded_and_rolled_back() {
    let e = env();
    let id = file_id(SMS_DB.0, SMS_DB.1);
    let sms = e.backup.join(&id[..2]).join(&id);
    fs::write(&sms, b"not a database").unwrap();
    assert!(ingest::run(&e.backup, &e.data).is_err());
    let db = archive_db(&e.data);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM import_run WHERE status = 'failed' AND error IS NOT NULL"
        ),
        1
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM message"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM source"), 0);
}

#[test]
fn stats_never_print_message_content() {
    let e = env();
    ingest::run(&e.backup, &e.data).unwrap();
    let s = stats::collect(&e.data).unwrap();
    let text = stats::render(&s);
    let json = serde_json::to_string(&s).unwrap();
    let sc = scenario::build();
    for m in sc
        .messages
        .iter()
        .filter_map(|m| m.text.as_deref())
        .filter(|t| t.len() > 3)
    {
        assert!(
            !text.contains(m) && !json.contains(m),
            "stats leaked message text"
        );
    }
    for h in &sc.handles {
        assert!(
            !text.contains(h.id) && !json.contains(h.id),
            "stats leaked an address"
        );
    }
    assert!(text.contains("Messages"));
}
