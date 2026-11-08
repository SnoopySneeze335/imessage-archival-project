//! Imports one backup folder into archive.db (spec: "Ingestion and import").
//!
//! An import adds only what is new, keyed on Apple's GUIDs, and records an
//! `import_run` with counts and an `import_issue` for every problem found.
//! Running it twice changes no data: the second run records a run that added
//! nothing. A message whose content changed between backups keeps every
//! version, and each record's current fields come from the newest backup that
//! holds it, so backups can be imported in any order (decision 0004).
//!
//! Logs and issues carry counts, GUIDs and row numbers only, never message
//! text, names or addresses.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use imessage_database::{
    tables::{
        attachment::Attachment, chat_handle::ChatToHandle, handle::Handle, messages::Message,
    },
    util::platform::Platform,
};
use rusqlite::{Connection, Transaction, params};
use serde::Serialize;

use crate::{address, archive, backup::Backup};

mod messages;

/// Version of the imessage-database crate this build reads backups with.
pub const IMESSAGE_DATABASE_VERSION: &str = "5.0.1";

/// Seconds between 1970-01-01 and Apple's epoch, 2001-01-01.
const APPLE_EPOCH_OFFSET: i64 = 978_307_200;

/// What one import found and added.
#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct Counts {
    pub handles_seen: usize,
    pub handles_added: usize,
    pub chats_seen: usize,
    pub chats_added: usize,
    pub messages_seen: usize,
    pub messages_added: usize,
    /// New content versions, including the first version of new messages.
    pub message_versions_added: usize,
    /// Messages already in the archive whose content differs in this backup.
    pub messages_changed: usize,
    pub message_edits_added: usize,
    pub reactions_added: usize,
    pub attachments_seen: usize,
    pub attachments_added: usize,
    pub attachment_files_present: usize,
    pub issues: usize,
}

/// Result of a successful import.
#[derive(Debug, Clone)]
pub struct Summary {
    pub run_id: i64,
    pub source_id: i64,
    pub counts: Counts,
    pub issues_by_kind: Vec<(String, usize)>,
}

/// Problem kinds recorded in `import_issue`.
pub mod issue {
    pub const ORPHAN_MESSAGE: &str = "orphan_message";
    pub const DUPLICATE_CHAT: &str = "duplicate_chat";
    pub const MISSING_ATTACHMENT_FILE: &str = "missing_attachment_file";
    pub const ATTACHMENT_NO_PATH: &str = "attachment_no_path";
    pub const REACTION_TARGET_MISSING: &str = "reaction_target_missing";
    pub const UNKNOWN_HANDLE: &str = "unknown_handle";
    pub const DECODE_FAILED: &str = "decode_failed";
    pub const ATTACHMENT_WITHOUT_MESSAGE: &str = "attachment_without_message";
}

/// The backup being imported.
#[derive(Debug, Clone, Copy)]
struct Source {
    id: i64,
    /// Unix seconds; decides which backup's copy of a record is current.
    backup_date: i64,
}

struct Issue {
    kind: &'static str,
    guid: Option<String>,
    source_rowid: Option<i64>,
    detail: Option<String>,
}

/// Imports `backup_dir` into the archive in `data_dir`.
pub fn run(backup_dir: &Path, data_dir: &Path) -> Result<Summary> {
    let backup = Backup::open(backup_dir)?;
    let (sms, sms_path) = backup.open_sms_db()?;
    let mut db = archive::open(data_dir)?;

    let run_id = start_run(&db, backup_dir)?;
    let result = (|| {
        let tx = db.transaction()?;
        let summary = import(&tx, &backup, &sms, &sms_path, run_id)?;
        tx.commit()?;
        Ok(summary)
    })();
    if let Err(err) = &result {
        // Data changes were rolled back; record why the run failed.
        let msg = format!("{err:#}");
        db.execute(
            "UPDATE import_run SET status = 'failed', finished_at = ?2, error = ?3 WHERE id = ?1",
            params![run_id, now_ns(), msg],
        )?;
    }
    result
}

fn start_run(db: &Connection, backup_dir: &Path) -> Result<i64> {
    db.execute(
        "INSERT INTO import_run (backup_folder, started_at, status, ingester_version, imessage_database_version)
         VALUES (?1, ?2, 'running', ?3, ?4)",
        params![
            backup_dir.to_string_lossy(),
            now_ns(),
            crate::VERSION,
            IMESSAGE_DATABASE_VERSION
        ],
    )?;
    Ok(db.last_insert_rowid())
}

fn import(
    tx: &Transaction,
    backup: &Backup,
    sms: &Connection,
    sms_path: &Path,
    run_id: i64,
) -> Result<Summary> {
    let mut counts = Counts::default();
    let mut issues: Vec<Issue> = Vec::new();
    let source = upsert_source(tx, backup)?;
    let source_id = source.id;
    tx.execute(
        "UPDATE import_run SET source_id = ?2 WHERE id = ?1",
        params![run_id, source_id],
    )?;

    let handles = import_handles(tx, sms, source_id, &mut counts)?;
    let chats = import_chats(tx, sms, source, &handles, &mut counts, &mut issues)?;
    let messages =
        messages::import_messages(tx, sms, source, &handles, &chats, &mut counts, &mut issues)?;
    import_attachments(tx, sms, backup, source, &messages, &mut counts, &mut issues)?;
    messages::check_reaction_targets(tx, source_id, &mut issues)?;

    let diagnostics = diagnostics(sms, sms_path, &backup.root)?;
    counts.issues = issues.len();
    let mut stmt = tx.prepare_cached(
        "INSERT INTO import_issue (run_id, kind, guid, source_rowid, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut by_kind: HashMap<&str, usize> = HashMap::new();
    for i in &issues {
        stmt.execute(params![run_id, i.kind, i.guid, i.source_rowid, i.detail])?;
        *by_kind.entry(i.kind).or_default() += 1;
    }
    drop(stmt);
    tx.execute(
        "UPDATE import_run SET status = 'succeeded', finished_at = ?2, counts = ?3, diagnostics = ?4 WHERE id = ?1",
        params![run_id, now_ns(), serde_json::to_string(&counts)?, diagnostics.to_string()],
    )?;

    let mut issues_by_kind: Vec<(String, usize)> = by_kind
        .into_iter()
        .map(|(k, n)| (k.to_owned(), n))
        .collect();
    issues_by_kind.sort();
    tracing::info!(
        run_id,
        source_id,
        messages_seen = counts.messages_seen,
        messages_added = counts.messages_added,
        messages_changed = counts.messages_changed,
        issues = counts.issues,
        "import finished"
    );
    Ok(Summary {
        run_id,
        source_id,
        counts,
        issues_by_kind,
    })
}

fn upsert_source(tx: &Transaction, backup: &Backup) -> Result<Source> {
    let i = &backup.info;
    tx.execute(
        "INSERT INTO source (device_id, backup_date, folder, device_name, product_type, ios_version, build_version, is_encrypted, first_imported_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (device_id, backup_date) DO NOTHING",
        params![
            i.device_id,
            i.backup_date,
            backup.root.to_string_lossy(),
            i.device_name,
            i.product_type,
            i.ios_version,
            i.build_version,
            i.is_encrypted,
            now_ns()
        ],
    )?;
    Ok(tx.query_row(
        "SELECT id, backup_date FROM source WHERE device_id = ?1 AND backup_date = ?2",
        params![i.device_id, i.backup_date],
        |r| {
            Ok(Source {
                id: r.get(0)?,
                backup_date: r.get(1)?,
            })
        },
    )?)
}

/// Column names of `table` in the backup's database.
fn columns(sms: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = sms.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    Ok(cols)
}

/// `name` if the column exists, else `NULL`, so older schemas still read.
fn col<'a>(cols: &HashSet<String>, name: &'a str) -> &'a str {
    if cols.contains(name) { name } else { "NULL" }
}

/// Backup handle ROWID → archive handle id.
type HandleMap = HashMap<i64, i64>;
/// Backup chat ROWID → archive chat id.
type ChatMap = HashMap<i64, i64>;
/// Backup message ROWID → archive message id.
type MessageMap = HashMap<i64, i64>;

fn import_handles(
    tx: &Transaction,
    sms: &Connection,
    source_id: i64,
    counts: &mut Counts,
) -> Result<HandleMap> {
    let c = columns(sms, "handle")?;
    let sql = format!(
        "SELECT ROWID, id, {}, {}, {}, {} FROM handle ORDER BY ROWID",
        col(&c, "service"),
        col(&c, "country"),
        col(&c, "uncanonicalized_id"),
        col(&c, "person_centric_id")
    );
    let mut read = sms.prepare(&sql)?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO handle (address, service) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
    )?;
    let mut find =
        tx.prepare_cached("SELECT id FROM handle WHERE address = ?1 AND service = ?2")?;
    let mut link = tx.prepare_cached(
        "INSERT OR IGNORE INTO source_handle (source_id, source_rowid, handle_id, raw_id, country, uncanonicalized_id, person_centric_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut map = HandleMap::new();
    let mut rows = read.query([])?;
    while let Some(r) = rows.next()? {
        let rowid: i64 = r.get(0)?;
        let raw: String = r.get(1)?;
        let service: String = r.get::<_, Option<String>>(2)?.unwrap_or_default();
        let addr = address::normalize(&raw);
        counts.handles_seen += 1;
        counts.handles_added += insert.execute(params![addr, service])?;
        let id: i64 = find.query_row(params![addr, service], |r| r.get(0))?;
        link.execute(params![
            source_id,
            rowid,
            id,
            raw,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?
        ])?;
        map.insert(rowid, id);
    }
    Ok(map)
}

fn import_chats(
    tx: &Transaction,
    sms: &Connection,
    source: Source,
    handles: &HandleMap,
    counts: &mut Counts,
    issues: &mut Vec<Issue>,
) -> Result<ChatMap> {
    let c = columns(sms, "chat")?;
    let sql = format!(
        "SELECT ROWID, guid, {}, {}, {}, {}, {}, {} FROM chat ORDER BY ROWID",
        col(&c, "chat_identifier"),
        col(&c, "service_name"),
        col(&c, "style"),
        col(&c, "display_name"),
        col(&c, "room_name"),
        col(&c, "group_id")
    );
    let mut read = sms.prepare(&sql)?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO chat (guid, chat_identifier, service, style, display_name, room_name, group_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (guid) DO NOTHING",
    )?;
    let mut update = tx.prepare_cached(
        "UPDATE chat SET (chat_identifier, service, style, display_name, room_name, group_id)
             = (?2, ?3, ?4, ?5, ?6, ?7) WHERE guid = ?1",
    )?;
    let mut newest_holder = tx.prepare_cached(
        "SELECT MAX(s.backup_date) FROM source_chat sc JOIN source s ON s.id = sc.source_id
         WHERE sc.chat_id = ?1",
    )?;
    let mut find = tx.prepare_cached("SELECT id FROM chat WHERE guid = ?1")?;
    let mut link = tx.prepare_cached(
        "INSERT OR IGNORE INTO source_chat (source_id, source_rowid, chat_id, display_name) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let source_id = source.id;
    let mut map = ChatMap::new();
    let mut by_identifier: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut rows = read.query([])?;
    while let Some(r) = rows.next()? {
        let rowid: i64 = r.get(0)?;
        let guid: String = r.get(1)?;
        let identifier: Option<String> = r.get(2)?;
        let display_name: Option<String> = r.get::<_, Option<String>>(5)?.filter(|n| !n.is_empty());
        counts.chats_seen += 1;
        let values = params![
            guid,
            identifier,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?,
            display_name,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?
        ];
        let added = insert.execute(values)?;
        counts.chats_added += added;
        let id: i64 = find.query_row(params![guid], |r| r.get(0))?;
        link.execute(params![source_id, rowid, id, display_name])?;
        // The newest backup's name and settings are current (decision 0004).
        if added == 0
            && newest_holder.query_row([id], |r| r.get::<_, i64>(0))? <= source.backup_date
        {
            update.execute(values)?;
        }
        map.insert(rowid, id);
        if let Some(ident) = identifier {
            by_identifier.entry(ident).or_default().push((rowid, guid));
        }
    }
    // Chats that share an identifier are proposed for merging (Milestone 6).
    let mut dups: Vec<(i64, String)> = by_identifier
        .into_values()
        .filter(|v| v.len() > 1)
        .flat_map(|mut v| {
            v.sort();
            v.into_iter().skip(1)
        })
        .collect();
    dups.sort();
    for (rowid, guid) in dups {
        issues.push(Issue {
            kind: issue::DUPLICATE_CHAT,
            guid: Some(guid),
            source_rowid: Some(rowid),
            detail: None,
        });
    }

    let mut read = sms.prepare("SELECT chat_id, handle_id FROM chat_handle_join")?;
    let mut insert = tx.prepare_cached(
        "INSERT OR IGNORE INTO chat_participant (chat_id, handle_id, source_id) VALUES (?1, ?2, ?3)",
    )?;
    let mut rows = read.query([])?;
    while let Some(r) = rows.next()? {
        let (chat, handle): (i64, i64) = (r.get(0)?, r.get(1)?);
        if let (Some(c), Some(h)) = (map.get(&chat), handles.get(&handle)) {
            insert.execute(params![c, h, source.id])?;
        }
    }
    Ok(map)
}

fn import_attachments(
    tx: &Transaction,
    sms: &Connection,
    backup: &Backup,
    source: Source,
    messages: &MessageMap,
    counts: &mut Counts,
    issues: &mut Vec<Issue>,
) -> Result<()> {
    let mut owners: HashMap<i64, Vec<i64>> = HashMap::new();
    {
        let mut stmt =
            sms.prepare("SELECT attachment_id, message_id FROM message_attachment_join")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            owners.entry(r.get(0)?).or_default().push(r.get(1)?);
        }
    }
    let c = columns(sms, "attachment")?;
    let sql = format!(
        "SELECT ROWID, guid, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM attachment ORDER BY ROWID",
        col(&c, "filename"),
        col(&c, "transfer_name"),
        col(&c, "uti"),
        col(&c, "mime_type"),
        col(&c, "total_bytes"),
        col(&c, "is_sticker"),
        col(&c, "is_outgoing"),
        col(&c, "hide_attachment"),
        col(&c, "created_date")
    );
    let mut read = sms.prepare(&sql)?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO attachment (guid, filename, transfer_name, uti, mime_type, total_bytes, is_sticker, is_outgoing,
                                 hide_attachment, created_at, missing, first_source_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT (guid) DO NOTHING",
    )?;
    let mut update = tx.prepare_cached(
        "UPDATE attachment SET (filename, transfer_name, uti, mime_type, total_bytes, is_sticker, is_outgoing,
                                hide_attachment, created_at) = (?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         WHERE guid = ?1",
    )?;
    let mut newest_holder = tx.prepare_cached(
        "SELECT MAX(s.backup_date) FROM attachment_source a JOIN source s ON s.id = a.source_id
         WHERE a.attachment_id = ?1",
    )?;
    let mut lower_first = tx.prepare_cached(
        "UPDATE attachment SET first_source_id = ?2
         WHERE id = ?1 AND (SELECT backup_date FROM source WHERE id = first_source_id) > ?3",
    )?;
    // The file is missing only if no imported backup has it (decision 0004).
    let mut set_missing = tx.prepare_cached(
        "UPDATE attachment SET missing = CASE
             WHEN EXISTS (SELECT 1 FROM attachment_source WHERE attachment_id = ?1 AND present = 1) THEN NULL
             WHEN EXISTS (SELECT 1 FROM attachment_source WHERE attachment_id = ?1 AND backup_file_id IS NOT NULL)
                 THEN 'not_in_backup'
             ELSE 'no_path' END
         WHERE id = ?1",
    )?;
    let mut find = tx.prepare_cached("SELECT id FROM attachment WHERE guid = ?1")?;
    let source_id = source.id;
    let mut seen_in = tx.prepare_cached(
        "INSERT OR IGNORE INTO attachment_source (attachment_id, source_id, source_rowid, backup_file_id, present) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut belongs = tx.prepare_cached(
        "INSERT OR IGNORE INTO message_attachment (message_id, attachment_id) VALUES (?1, ?2)",
    )?;
    let mut rows = read.query([])?;
    while let Some(r) = rows.next()? {
        let rowid: i64 = r.get(0)?;
        let guid: String = r.get(1)?;
        let filename: Option<String> = r.get(2)?;
        counts.attachments_seen += 1;
        let (file_id, present) = match &filename {
            Some(f) => {
                let (id, present) = backup.attachment_file(f);
                (Some(id), present)
            }
            None => (None, false),
        };
        let missing = match (&filename, present) {
            (None, _) => {
                issues.push(Issue {
                    kind: issue::ATTACHMENT_NO_PATH,
                    guid: Some(guid.clone()),
                    source_rowid: Some(rowid),
                    detail: None,
                });
                Some("no_path")
            }
            (Some(_), false) => {
                issues.push(Issue {
                    kind: issue::MISSING_ATTACHMENT_FILE,
                    guid: Some(guid.clone()),
                    source_rowid: Some(rowid),
                    detail: None,
                });
                Some("not_in_backup")
            }
            (Some(_), true) => None,
        };
        counts.attachment_files_present += usize::from(present);
        let metadata = params![
            guid,
            filename,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<bool>>(7)?.unwrap_or(false),
            r.get::<_, Option<bool>>(8)?.unwrap_or(false),
            r.get::<_, Option<i64>>(9)?.unwrap_or(0),
            // attachment.created_date is in seconds since 2001.
            r.get::<_, Option<i64>>(10)?
                .filter(|&s| s != 0)
                .map(|s| (s + APPLE_EPOCH_OFFSET) * 1_000_000_000),
        ];
        let mut values = metadata.to_vec();
        values.extend([&missing as &dyn rusqlite::ToSql, &source_id]);
        let is_new = insert.execute(values.as_slice())? == 1;
        counts.attachments_added += usize::from(is_new);
        let id: i64 = find.query_row(params![guid], |r| r.get(0))?;
        seen_in.execute(params![id, source_id, rowid, file_id, present])?;
        if !is_new {
            if newest_holder.query_row([id], |r| r.get::<_, i64>(0))? <= source.backup_date {
                update.execute(metadata)?;
            }
            lower_first.execute(params![id, source_id, source.backup_date])?;
            set_missing.execute([id])?;
        }
        let msg_ids: Vec<i64> = owners
            .get(&rowid)
            .map(|v| v.iter().filter_map(|m| messages.get(m).copied()).collect())
            .unwrap_or_default();
        if msg_ids.is_empty() {
            issues.push(Issue {
                kind: issue::ATTACHMENT_WITHOUT_MESSAGE,
                guid: Some(guid.clone()),
                source_rowid: Some(rowid),
                detail: None,
            });
        }
        for m in msg_ids {
            belongs.execute(params![m, id])?;
        }
    }
    Ok(())
}

/// imessage-database's own diagnostics of the backup, for comparison with
/// the counts in docs/SPEC.md ("Source data").
fn diagnostics(
    sms: &Connection,
    _sms_path: &Path,
    backup_root: &Path,
) -> Result<serde_json::Value> {
    let m = Message::run_diagnostic(sms)?;
    let h = Handle::run_diagnostic(sms)?;
    let c = ChatToHandle::run_diagnostic(sms)?;
    let a = Attachment::run_diagnostic(sms, backup_root, &Platform::iOS, None)?;
    Ok(serde_json::json!({
        "messages": m.total_messages,
        "messages_without_chat": m.messages_without_chat,
        "messages_in_multiple_chats": m.messages_in_multiple_chats,
        "recoverable_messages": m.recoverable_messages,
        "chats": c.total_chats,
        "duplicated_chats": c.total_duplicated,
        "chats_with_no_handles": c.chats_with_no_handles,
        "handles": h.total_handles,
        "merged_handles": h.merged_handles,
        "multi_handle_groups": h.multi_handle_groups,
        "attachments": a.total_attachments,
        "attachment_bytes_referenced": a.total_bytes_referenced,
        "attachment_bytes_on_disk": a.total_bytes_on_disk,
        "attachments_missing_file": a.missing_files,
        "attachments_no_path": a.no_path_provided,
    }))
}

/// Apple timestamp (nanoseconds since 2001, or seconds before iOS 11) →
/// Unix nanoseconds. 0 means unset.
pub fn apple_to_unix_ns(value: i64) -> Option<i64> {
    if value == 0 {
        return None;
    }
    let apple_ns = if value.abs() > 1_000_000_000_000 {
        value
    } else {
        value * 1_000_000_000
    };
    Some(apple_ns + APPLE_EPOCH_OFFSET * 1_000_000_000)
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
    fn apple_dates_convert_from_both_units() {
        let unix_2024 = 1_704_067_200i64; // 2024-01-01
        let apple_s = unix_2024 - APPLE_EPOCH_OFFSET;
        assert_eq!(
            apple_to_unix_ns(apple_s * 1_000_000_000),
            Some(unix_2024 * 1_000_000_000)
        );
        assert_eq!(apple_to_unix_ns(apple_s), Some(unix_2024 * 1_000_000_000));
        assert_eq!(apple_to_unix_ns(0), None);
    }
}
