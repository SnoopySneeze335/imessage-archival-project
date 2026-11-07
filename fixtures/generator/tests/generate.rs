use std::{collections::BTreeMap, fs, path::Path};

use fixture_gen::{ADDRESS_BOOK_DB, EXPECTED_FILE, SMS_DB, file_id, generate};
use rusqlite::Connection;

fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn stored(root: &Path, (domain, rel): (&str, &str)) -> std::path::PathBuf {
    let id = file_id(domain, rel);
    root.join(&id[..2]).join(id)
}

fn count(conn: &Connection, sql: &str) -> usize {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as usize
}

#[test]
fn output_is_byte_for_byte_deterministic() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    generate(a.path()).unwrap();
    generate(b.path()).unwrap();
    let (ta, tb) = (read_tree(a.path()), read_tree(b.path()));
    assert_eq!(ta.keys().collect::<Vec<_>>(), tb.keys().collect::<Vec<_>>());
    for (name, bytes) in &ta {
        assert!(bytes == &tb[name], "{name} differs between runs");
    }
}

#[test]
fn file_ids_match_real_ios_backups() {
    // Well-known IDs of these files in every real iOS backup.
    assert_eq!(
        file_id(SMS_DB.0, SMS_DB.1),
        "3d0d7e5fb2ce288813306e4d4636395e047a3d28"
    );
    assert_eq!(
        file_id(ADDRESS_BOOK_DB.0, ADDRESS_BOOK_DB.1),
        "31bb7ba8914766d4ba40d6dfb6113c8b614be442"
    );
}

#[test]
fn refuses_non_empty_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("keep.txt"), "x").unwrap();
    assert!(generate(dir.path()).is_err());
}

#[test]
fn databases_match_expected_counts() {
    let dir = tempfile::tempdir().unwrap();
    let e = generate(dir.path()).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join(EXPECTED_FILE)).unwrap()).unwrap();
    assert_eq!(json["messages"], e.messages);

    let sms = Connection::open(stored(dir.path(), SMS_DB)).unwrap();
    assert_eq!(count(&sms, "SELECT COUNT(*) FROM message"), e.messages);
    assert_eq!(count(&sms, "SELECT COUNT(*) FROM chat"), e.chats);
    assert_eq!(count(&sms, "SELECT COUNT(*) FROM handle"), e.handles);
    assert_eq!(
        count(&sms, "SELECT COUNT(*) FROM attachment"),
        e.attachments
    );
    assert_eq!(
        count(
            &sms,
            "SELECT COUNT(DISTINCT message_id) FROM chat_message_join"
        ),
        e.messages_in_chats
    );
    assert_eq!(
        count(&sms, "SELECT COUNT(*) FROM chat_recoverable_message_join"),
        e.recoverable_deleted
    );
    assert_eq!(
        count(
            &sms,
            "SELECT COUNT(*) FROM message WHERE ROWID NOT IN (SELECT message_id FROM chat_message_join)
             AND ROWID NOT IN (SELECT message_id FROM chat_recoverable_message_join)"
        ),
        e.messages_without_chat
    );
    assert_eq!(
        count(
            &sms,
            "SELECT COUNT(*) FROM message WHERE associated_message_type != 0"
        ),
        e.reactions
    );
    assert_eq!(
        count(
            &sms,
            "SELECT COUNT(*) FROM attachment WHERE filename IS NULL"
        ),
        e.attachments_without_path
    );
    assert!(e.messages_without_chat >= 1 && e.duplicated_chats >= 1 && e.duplicate_handles >= 1);
    assert!(e.attachments_missing_file >= 1 && e.unsent >= 1 && e.edited >= 1);

    let ab = Connection::open(stored(dir.path(), ADDRESS_BOOK_DB)).unwrap();
    assert_eq!(
        count(&ab, "SELECT COUNT(*) FROM ABPerson"),
        e.addressbook_people
    );
    assert!(
        e.addresses_with_name < e.unique_addresses,
        "some handles must be unknown"
    );
}

#[test]
fn manifest_lists_every_stored_file_and_attachments_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let e = generate(dir.path()).unwrap();
    let manifest = Connection::open(dir.path().join("Manifest.db")).unwrap();
    let mut stmt = manifest
        .prepare("SELECT fileID, domain, relativePath FROM Files WHERE flags = 1")
        .unwrap();
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (id, domain, rel) in &rows {
        assert_eq!(id, &file_id(domain, rel));
        assert!(dir.path().join(&id[..2]).join(id).is_file(), "{rel}");
    }

    let sms = Connection::open(stored(dir.path(), SMS_DB)).unwrap();
    let mut stmt = sms
        .prepare("SELECT filename FROM attachment WHERE filename IS NOT NULL")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let present = names
        .iter()
        .filter(|f| stored(dir.path(), ("MediaDomain", f.trim_start_matches("~/"))).is_file())
        .count();
    assert_eq!(present, e.attachments_present);
    assert_eq!(names.len() - present, e.attachments_missing_file);
}

#[test]
fn plists_describe_an_unencrypted_device_backup() {
    let dir = tempfile::tempdir().unwrap();
    generate(dir.path()).unwrap();
    let manifest = plist::Value::from_file(dir.path().join("Manifest.plist")).unwrap();
    let manifest = manifest.as_dictionary().unwrap();
    assert_eq!(manifest["IsEncrypted"].as_boolean(), Some(false));
    let info = plist::Value::from_file(dir.path().join("Info.plist")).unwrap();
    assert_eq!(
        info.as_dictionary().unwrap()["Device Name"].as_string(),
        Some("Fixture iPhone")
    );
    assert!(dir.path().join("Status.plist").is_file());
}
