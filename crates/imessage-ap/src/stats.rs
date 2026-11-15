//! `imessage-ap stats`: counts only, never content.
//!
//! The output is safe to share: it is numbers and backup metadata (device
//! name, iOS version, dates). Two sections: what archive.db holds, and, for
//! each imported backup, imessage-database's own diagnostics of that backup,
//! which use the same definitions as the counts in docs/SPEC.md.

use std::{fmt::Write as _, path::Path};

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::archive;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArchiveStats {
    pub sources: u64,
    pub import_runs_succeeded: u64,
    pub import_runs_failed: u64,
    pub messages: u64,
    pub messages_in_chat: u64,
    pub messages_recovered_deleted: u64,
    pub orphan_messages: u64,
    pub reactions: u64,
    pub replies: u64,
    /// Content versions of all messages (one per message unless it changed
    /// between backups).
    pub message_versions: u64,
    /// Messages with more than one content version.
    pub messages_with_versions: u64,
    /// Entries in parsed edit histories.
    pub message_edits: u64,
    pub chats: u64,
    pub chats_sharing_identifier: u64,
    pub handles: u64,
    pub unique_addresses: u64,
    pub attachment_records: u64,
    pub attachments_with_file: u64,
    pub attachment_files_missing: u64,
    pub attachments_without_path: u64,
    /// Attachment files copied into /data/blobs (one per backup holding the
    /// file), and their size, duplicates included.
    pub attachment_files_copied: u64,
    pub attachment_bytes_copied: u64,
    /// Files a backup holds that are not copied yet.
    pub attachment_files_pending_copy: u64,
    /// Distinct files in /data/blobs and their total size.
    pub blobs: u64,
    pub blob_bytes: u64,
    /// Copied files whose content was already stored (copied minus blobs).
    pub duplicates_skipped: u64,
    pub thumbnails_made: u64,
    pub thumbnails_failed: u64,
    /// Thumbnails, previews and conversions waiting to be made.
    pub derivatives_pending: u64,
    pub derivatives_failed: u64,
}

#[derive(Debug, Serialize)]
pub struct SourceStats {
    pub id: i64,
    pub device_name: Option<String>,
    pub ios_version: Option<String>,
    /// Unix seconds.
    pub backup_date: i64,
    /// Rows in the backup's own handle and chat tables.
    pub handle_rows: u64,
    pub chat_rows: u64,
    pub messages_held: u64,
    /// imessage-database diagnostics from the latest successful import.
    pub diagnostics: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct Stats {
    pub archive_schema_version: usize,
    pub archive: ArchiveStats,
    pub sources: Vec<SourceStats>,
}

fn n(conn: &Connection, sql: &str) -> Result<u64> {
    Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
}

/// Reads counts from the archive in `data_dir`.
pub fn collect(data_dir: &Path) -> Result<Stats> {
    let conn = archive::open_read_only(data_dir)?;
    let copied = n(
        &conn,
        "SELECT COUNT(*) FROM attachment_source WHERE sha256 IS NOT NULL",
    )?;
    let blobs = n(&conn, "SELECT COUNT(*) FROM blob")?;
    let archive = ArchiveStats {
        sources: n(&conn, "SELECT COUNT(*) FROM source")?,
        import_runs_succeeded: n(
            &conn,
            "SELECT COUNT(*) FROM import_run WHERE kind = 'records' AND status = 'succeeded'",
        )?,
        import_runs_failed: n(
            &conn,
            "SELECT COUNT(*) FROM import_run WHERE kind = 'records' AND status = 'failed'",
        )?,
        messages: n(&conn, "SELECT COUNT(*) FROM message")?,
        messages_in_chat: n(
            &conn,
            "SELECT COUNT(*) FROM message WHERE chat_id IS NOT NULL AND is_recovered_deleted = 0",
        )?,
        messages_recovered_deleted: n(
            &conn,
            "SELECT COUNT(*) FROM message WHERE is_recovered_deleted = 1",
        )?,
        orphan_messages: n(&conn, "SELECT COUNT(*) FROM message WHERE chat_id IS NULL")?,
        reactions: n(&conn, "SELECT COUNT(*) FROM reaction")?,
        replies: n(
            &conn,
            "SELECT COUNT(*) FROM message WHERE reply_to_guid IS NOT NULL",
        )?,
        message_versions: n(&conn, "SELECT COUNT(*) FROM message_version")?,
        messages_with_versions: n(
            &conn,
            "SELECT COUNT(*) FROM (SELECT 1 FROM message_version GROUP BY message_id HAVING COUNT(*) > 1)",
        )?,
        message_edits: n(&conn, "SELECT COUNT(*) FROM message_edit")?,
        chats: n(&conn, "SELECT COUNT(*) FROM chat")?,
        chats_sharing_identifier: n(
            &conn,
            "SELECT COALESCE(SUM(c - 1), 0) FROM (SELECT COUNT(*) AS c FROM chat WHERE chat_identifier IS NOT NULL GROUP BY chat_identifier HAVING c > 1)",
        )?,
        handles: n(&conn, "SELECT COUNT(*) FROM handle")?,
        unique_addresses: n(&conn, "SELECT COUNT(DISTINCT address) FROM handle")?,
        attachment_records: n(&conn, "SELECT COUNT(*) FROM attachment")?,
        attachments_with_file: n(
            &conn,
            "SELECT COUNT(*) FROM attachment WHERE missing IS NULL",
        )?,
        attachment_files_missing: n(
            &conn,
            "SELECT COUNT(*) FROM attachment WHERE missing = 'not_in_backup'",
        )?,
        attachments_without_path: n(
            &conn,
            "SELECT COUNT(*) FROM attachment WHERE missing = 'no_path'",
        )?,
        attachment_files_copied: copied,
        attachment_bytes_copied: n(
            &conn,
            "SELECT COALESCE(SUM(b.size), 0) FROM attachment_source s JOIN blob b ON b.sha256 = s.sha256",
        )?,
        attachment_files_pending_copy: n(
            &conn,
            "SELECT COUNT(*) FROM attachment_source WHERE present = 1 AND sha256 IS NULL",
        )?,
        blobs,
        blob_bytes: n(&conn, "SELECT COALESCE(SUM(size), 0) FROM blob")?,
        duplicates_skipped: copied.saturating_sub(blobs),
        thumbnails_made: n(
            &conn,
            "SELECT COUNT(*) FROM derivative WHERE kind = 'thumb' AND status = 'done'",
        )?,
        thumbnails_failed: n(
            &conn,
            "SELECT COUNT(*) FROM derivative WHERE kind = 'thumb' AND status = 'failed'",
        )?,
        derivatives_pending: n(
            &conn,
            "SELECT COUNT(*) FROM derivative WHERE status = 'pending'",
        )?,
        derivatives_failed: n(
            &conn,
            "SELECT COUNT(*) FROM derivative WHERE status = 'failed'",
        )?,
    };

    let mut stmt = conn.prepare(
        "SELECT s.id, s.device_name, s.ios_version, s.backup_date,
                (SELECT COUNT(*) FROM source_handle WHERE source_id = s.id),
                (SELECT COUNT(*) FROM source_chat WHERE source_id = s.id),
                (SELECT COUNT(*) FROM message_source WHERE source_id = s.id),
                (SELECT diagnostics FROM import_run WHERE source_id = s.id AND kind = 'records' AND status = 'succeeded' ORDER BY id DESC LIMIT 1)
         FROM source s ORDER BY s.backup_date, s.id",
    )?;
    let sources = stmt
        .query_map([], |r| {
            Ok(SourceStats {
                id: r.get(0)?,
                device_name: r.get(1)?,
                ios_version: r.get(2)?,
                backup_date: r.get(3)?,
                handle_rows: r.get::<_, i64>(4)? as u64,
                chat_rows: r.get::<_, i64>(5)? as u64,
                messages_held: r.get::<_, i64>(6)? as u64,
                diagnostics: r
                    .get::<_, Option<String>>(7)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Stats {
        archive_schema_version: archive::schema_version(),
        archive,
        sources,
    })
}

fn thousands(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn gb(bytes: u64) -> String {
    format!("{:.2} GB", bytes as f64 / 1e9)
}

/// Human-readable report, laid out like the diagnostics table in SPEC.md.
pub fn render(s: &Stats) -> String {
    let a = &s.archive;
    let mut out = String::new();
    let _ = writeln!(out, "Archive (schema v{})", s.archive_schema_version);
    let rows: Vec<(&str, u64)> = vec![
        ("Sources (imported backups)", a.sources),
        ("Import runs succeeded", a.import_runs_succeeded),
        ("Import runs failed", a.import_runs_failed),
        ("Messages", a.messages),
        ("  in a chat", a.messages_in_chat),
        (
            "  recovered (recently deleted)",
            a.messages_recovered_deleted,
        ),
        ("  not linked to a chat", a.orphan_messages),
        ("  reactions", a.reactions),
        ("  replies", a.replies),
        ("  content versions", a.message_versions),
        ("  with more than one version", a.messages_with_versions),
        ("  edit-history entries", a.message_edits),
        ("Chats", a.chats),
        (
            "  sharing an identifier (extra rows)",
            a.chats_sharing_identifier,
        ),
        ("Handles (address + service)", a.handles),
        ("  unique addresses", a.unique_addresses),
        ("Attachment records", a.attachment_records),
        ("  with a file", a.attachments_with_file),
        ("  file not in any backup", a.attachment_files_missing),
        ("  no path", a.attachments_without_path),
    ];
    for (label, v) in rows {
        let _ = writeln!(out, "  {label:<42}{:>12}", thousands(v));
    }
    let _ = writeln!(out, "\nFiles (/data/blobs, /data/derived)");
    let files: Vec<(&str, String)> = vec![
        (
            "Attachment files copied",
            thousands(a.attachment_files_copied),
        ),
        ("  size, duplicates included", gb(a.attachment_bytes_copied)),
        (
            "  not copied yet",
            thousands(a.attachment_files_pending_copy),
        ),
        ("Unique blobs", thousands(a.blobs)),
        ("  total size", gb(a.blob_bytes)),
        ("  duplicates skipped", thousands(a.duplicates_skipped)),
        ("Thumbnails made", thousands(a.thumbnails_made)),
        ("  failed", thousands(a.thumbnails_failed)),
        (
            "Derivatives pending (all kinds)",
            thousands(a.derivatives_pending),
        ),
        ("  failed (all kinds)", thousands(a.derivatives_failed)),
    ];
    for (label, v) in files {
        let _ = writeln!(out, "  {label:<42}{v:>12}");
    }
    for src in &s.sources {
        let _ = writeln!(
            out,
            "\nBackup {} ({}, iOS {}, backup date {})",
            src.id,
            src.device_name.as_deref().unwrap_or("unknown device"),
            src.ios_version.as_deref().unwrap_or("?"),
            src.backup_date
        );
        for (label, v) in [
            ("Handle rows", src.handle_rows),
            ("Chat rows", src.chat_rows),
            ("Messages held", src.messages_held),
        ] {
            let _ = writeln!(out, "  {label:<42}{:>12}", thousands(v));
        }
        if let Some(d) = &src.diagnostics {
            let _ = writeln!(out, "  imessage-database diagnostics:");
            let get = |k: &str| d.get(k).and_then(serde_json::Value::as_u64);
            for (label, key) in [
                ("Messages", "messages"),
                (
                    "  not linked to a chat (incl. recoverable)",
                    "messages_without_chat",
                ),
                ("  in more than one chat", "messages_in_multiple_chats"),
                ("  recoverable", "recoverable_messages"),
                ("Chats", "chats"),
                ("  duplicated", "duplicated_chats"),
                ("  with no handles", "chats_with_no_handles"),
                ("Handles (rows)", "handles"),
                ("  duplicates removed", "merged_handles"),
                ("  people with more than one ID", "multi_handle_groups"),
                ("Attachment records", "attachments"),
                (
                    "  files missing (incl. no path)",
                    "attachments_missing_file",
                ),
                ("  no path", "attachments_no_path"),
            ] {
                let v = get(key).map_or_else(|| "n/a".to_owned(), thousands);
                let _ = writeln!(out, "    {label:<40}{v:>12}");
            }
            for (label, key) in [
                ("Attachment data referenced", "attachment_bytes_referenced"),
                ("Attachment data on disk", "attachment_bytes_on_disk"),
            ] {
                let v = get(key).map_or_else(|| "n/a".to_owned(), gb);
                let _ = writeln!(out, "    {label:<40}{v:>12}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::thousands;

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(371_159), "371,159");
        assert_eq!(thousands(1_002), "1,002");
    }
}
