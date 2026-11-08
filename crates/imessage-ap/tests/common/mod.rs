//! Helpers shared by the ingest tests.
#![allow(dead_code)]

use std::path::Path;

use imessage_ap::archive;
use rusqlite::{Connection, types::ValueRef};

pub fn archive_db(data: &Path) -> Connection {
    Connection::open(archive::path(data)).unwrap()
}

pub fn count(conn: &Connection, sql: &str) -> usize {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as usize
}

pub fn issues(conn: &Connection, run_id: i64, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT guid FROM import_issue WHERE run_id = ?1 AND kind = ?2 ORDER BY guid")
        .unwrap();
    stmt.query_map(rusqlite::params![run_id, kind], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A value as readable text (strings unquoted, blobs as their length).
fn show(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Text(t) => format!("{:?}", String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => format!("blob[{}:{:08x}]", b.len(), checksum(b)),
        other => format!("{other:?}"),
    }
}

/// FNV-1a, so equal-length blobs that differ still compare unequal.
fn checksum(b: &[u8]) -> u32 {
    b.iter().fold(0x811c_9dc5u32, |h, &x| {
        (h ^ u32::from(x)).wrapping_mul(0x0100_0193)
    })
}

fn rows(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap_or_else(|e| panic!("{e}: {sql}"));
    let n = stmt.column_count();
    let mut rows: Vec<String> = stmt
        .query_map([], |r| {
            Ok((0..n)
                .map(|i| show(r.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.sort();
    rows
}

/// Data tables (everything but the per-run history tables).
fn data_tables(conn: &Connection) -> Vec<String> {
    conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         AND name NOT IN ('import_run', 'import_issue') ORDER BY name",
    )
    .unwrap()
    .query_map([], |r| r.get(0))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Every data table's rows, exactly as stored, in a stable order.
pub fn dump(conn: &Connection) -> String {
    let mut out = String::new();
    for t in data_tables(conn) {
        let r = rows(conn, &format!("SELECT * FROM {t}"));
        out.push_str(&format!("## {t}\n{}\n", r.join("\n")));
    }
    out
}

/// Source key: device ID and backup date, never the row id.
fn src(col: &str) -> String {
    format!("(SELECT device_id || '@' || backup_date FROM source WHERE id = {col})")
}

fn handle(col: &str) -> String {
    format!("(SELECT address || '/' || service FROM handle WHERE id = {col})")
}

fn chat(col: &str) -> String {
    format!("(SELECT guid FROM chat WHERE id = {col})")
}

fn message(col: &str) -> String {
    format!("(SELECT guid FROM message WHERE id = {col})")
}

fn attachment(col: &str) -> String {
    format!("(SELECT guid FROM attachment WHERE id = {col})")
}

const VERSION: &str = "text, subject, edited_at, retracted_at, is_unsent, item_type,
    balloon_bundle_id, hex(attributed_body), hex(message_summary_info), hex(payload_data)";

/// Every data table's rows with internal row ids replaced by natural keys
/// (GUIDs, addresses, device and backup date), and import times left out.
/// Decision 0002 allows only row ids to differ between rebuilds, so two
/// archives with equal canonical dumps hold the same data.
pub fn canonical_dump(conn: &Connection) -> String {
    let queries: Vec<(&str, String)> = vec![
        (
            "source",
            "SELECT device_id, backup_date, device_name, product_type, ios_version, build_version, is_encrypted FROM source".into(),
        ),
        ("handle", "SELECT address, service FROM handle".into()),
        (
            "source_handle",
            format!(
                "SELECT {}, source_rowid, {}, raw_id, country, uncanonicalized_id, person_centric_id FROM source_handle",
                src("source_id"),
                handle("handle_id")
            ),
        ),
        (
            "chat",
            "SELECT guid, chat_identifier, service, style, display_name, room_name, group_id FROM chat".into(),
        ),
        (
            "source_chat",
            format!(
                "SELECT {}, source_rowid, {}, display_name FROM source_chat",
                src("source_id"),
                chat("chat_id")
            ),
        ),
        (
            "chat_participant",
            format!(
                "SELECT {}, {}, {} FROM chat_participant",
                chat("chat_id"),
                handle("handle_id"),
                src("source_id")
            ),
        ),
        (
            "message",
            format!(
                "SELECT guid, {}, {}, is_from_me, service, sent_at, delivered_at, read_at, edited_at,
                        retracted_at, played_at, text, subject, item_type, group_action_type, {},
                        group_title, associated_message_guid, associated_message_type,
                        associated_message_emoji, reply_to_guid, reply_to_part, balloon_bundle_id,
                        expressive_send_style_id, is_audio_message, is_unsent, is_recovered_deleted, {}
                 FROM message",
                chat("chat_id"),
                handle("sender_handle_id"),
                handle("other_handle_id"),
                src("first_source_id")
            ),
        ),
        (
            "message_chat",
            format!(
                "SELECT {}, {} FROM message_chat",
                message("message_id"),
                chat("chat_id")
            ),
        ),
        (
            "message_source",
            format!(
                "SELECT {}, {}, source_rowid FROM message_source",
                message("message_id"),
                src("source_id")
            ),
        ),
        (
            "message_version",
            format!(
                "SELECT {}, {VERSION} FROM message_version",
                message("message_id")
            ),
        ),
        (
            "message_version_source",
            format!(
                "SELECT {}, {VERSION}, {} FROM message_version_source vs
                 JOIN message_version v ON v.id = vs.version_id",
                message("v.message_id"),
                src("vs.source_id")
            ),
        ),
        (
            "message_edit",
            format!(
                "SELECT {}, part, edited_at, text FROM message_edit",
                message("message_id")
            ),
        ),
        (
            "reaction",
            format!(
                "SELECT {}, target_guid, target_part, kind, emoji, is_removal FROM reaction",
                message("message_id")
            ),
        ),
        (
            "attachment",
            format!(
                "SELECT guid, filename, transfer_name, uti, mime_type, total_bytes, is_sticker, is_outgoing,
                        hide_attachment, created_at, sha256, missing, {} FROM attachment",
                src("first_source_id")
            ),
        ),
        (
            "message_attachment",
            format!(
                "SELECT {}, {} FROM message_attachment",
                message("message_id"),
                attachment("attachment_id")
            ),
        ),
        (
            "attachment_source",
            format!(
                "SELECT {}, {}, source_rowid, backup_file_id, present FROM attachment_source",
                attachment("attachment_id"),
                src("source_id")
            ),
        ),
    ];
    let mut covered: Vec<String> = queries.iter().map(|(t, _)| (*t).to_owned()).collect();
    covered.sort();
    assert_eq!(
        covered,
        data_tables(conn),
        "canonical_dump must cover every data table"
    );
    let mut out = String::new();
    for (t, sql) in queries {
        out.push_str(&format!("## {t}\n{}\n", rows(conn, &sql).join("\n")));
    }
    out
}
