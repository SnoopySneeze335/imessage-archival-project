//! Imports one backup folder into archive.db (spec: "Ingestion and import").
//!
//! An import adds only what is new, keyed on Apple's GUIDs, and records an
//! `import_run` with counts and an `import_issue` for every problem found.
//! Running it twice changes no data: the second run records a run that added
//! nothing. Changed messages (edits between backups) are versioned in
//! Milestone 2b; until then the first stored copy is kept.
//!
//! Logs and issues carry counts, GUIDs and row numbers only, never message
//! text, names or addresses.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use imessage_database::{
    tables::{
        attachment::Attachment, capabilities::Capabilities, chat_handle::ChatToHandle,
        handle::Handle, messages::Message,
    },
    util::{platform::Platform, query_context::QueryContext},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

use crate::{address, archive, backup::Backup};

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
    let source_id = upsert_source(tx, backup)?;
    tx.execute(
        "UPDATE import_run SET source_id = ?2 WHERE id = ?1",
        params![run_id, source_id],
    )?;

    let handles = import_handles(tx, sms, source_id, &mut counts)?;
    let chats = import_chats(tx, sms, source_id, &handles, &mut counts, &mut issues)?;
    let messages = import_messages(
        tx,
        sms,
        source_id,
        &handles,
        &chats,
        &mut counts,
        &mut issues,
    )?;
    import_attachments(
        tx,
        sms,
        backup,
        source_id,
        &messages,
        &mut counts,
        &mut issues,
    )?;
    check_reaction_targets(tx, source_id, &mut issues)?;

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

fn upsert_source(tx: &Transaction, backup: &Backup) -> Result<i64> {
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
        "SELECT id FROM source WHERE device_id = ?1 AND backup_date = ?2",
        params![i.device_id, i.backup_date],
        |r| r.get(0),
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
    source_id: i64,
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
    let mut find = tx.prepare_cached("SELECT id FROM chat WHERE guid = ?1")?;
    let mut link = tx.prepare_cached(
        "INSERT OR IGNORE INTO source_chat (source_id, source_rowid, chat_id, display_name) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut map = ChatMap::new();
    let mut by_identifier: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut rows = read.query([])?;
    while let Some(r) = rows.next()? {
        let rowid: i64 = r.get(0)?;
        let guid: String = r.get(1)?;
        let identifier: Option<String> = r.get(2)?;
        let display_name: Option<String> = r.get::<_, Option<String>>(5)?.filter(|n| !n.is_empty());
        counts.chats_seen += 1;
        counts.chats_added += insert.execute(params![
            guid,
            identifier,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?,
            display_name,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?
        ])?;
        let id: i64 = find.query_row(params![guid], |r| r.get(0))?;
        link.execute(params![source_id, rowid, id, display_name])?;
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
            insert.execute(params![c, h, source_id])?;
        }
    }
    Ok(map)
}

/// Extra per-message fields the crate's `Message` does not carry.
struct Extras {
    attributed_body: Option<Vec<u8>>,
    message_summary_info: Option<Vec<u8>>,
    payload_data: Option<Vec<u8>>,
    date_retracted: i64,
    date_played: i64,
    is_audio_message: bool,
}

#[allow(clippy::too_many_arguments)]
fn import_messages(
    tx: &Transaction,
    sms: &Connection,
    source_id: i64,
    handles: &HandleMap,
    chats: &ChatMap,
    counts: &mut Counts,
    issues: &mut Vec<Issue>,
) -> Result<MessageMap> {
    // Chat membership, read directly so a message in several chats keeps all.
    let mut chat_join: HashMap<i64, Vec<i64>> = HashMap::new();
    {
        let mut stmt =
            sms.prepare("SELECT message_id, chat_id FROM chat_message_join ORDER BY chat_id")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            chat_join.entry(r.get(0)?).or_default().push(r.get(1)?);
        }
    }
    let mut recoverable: HashMap<i64, i64> = HashMap::new();
    if !columns(sms, "chat_recoverable_message_join")?.is_empty() {
        let mut stmt =
            sms.prepare("SELECT message_id, chat_id FROM chat_recoverable_message_join")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            recoverable.insert(r.get(0)?, r.get(1)?);
        }
    }

    let mc = columns(sms, "message")?;
    let extras_sql = format!(
        "SELECT {}, {}, {}, {}, {}, {} FROM message WHERE ROWID = ?1",
        col(&mc, "attributedBody"),
        col(&mc, "message_summary_info"),
        col(&mc, "payload_data"),
        col(&mc, "date_retracted"),
        col(&mc, "date_played"),
        col(&mc, "is_audio_message")
    );
    let mut extras_stmt = sms.prepare(&extras_sql)?;

    let mut insert = tx.prepare_cached(
        "INSERT INTO message (
            guid, chat_id, sender_handle_id, is_from_me, service, sent_at, delivered_at, read_at,
            edited_at, retracted_at, played_at, text, subject, item_type, group_action_type,
            other_handle_id, group_title, associated_message_guid, associated_message_type,
            associated_message_emoji, reply_to_guid, reply_to_part, balloon_bundle_id,
            expressive_send_style_id, is_audio_message, is_unsent, is_recovered_deleted, first_source_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                  ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)
        ON CONFLICT (guid) DO NOTHING",
    )?;
    let mut find = tx.prepare_cached("SELECT id FROM message WHERE guid = ?1")?;
    let mut raw = tx.prepare_cached(
        "INSERT OR IGNORE INTO message_raw (message_id, attributed_body, message_summary_info, payload_data) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut in_chat = tx.prepare_cached(
        "INSERT OR IGNORE INTO message_chat (message_id, chat_id) VALUES (?1, ?2)",
    )?;
    let mut held = tx.prepare_cached(
        "INSERT OR IGNORE INTO message_source (message_id, source_id, source_rowid) VALUES (?1, ?2, ?3)",
    )?;
    let mut react = tx.prepare_cached(
        "INSERT OR IGNORE INTO reaction (message_id, target_guid, target_part, kind, emoji, is_removal) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    let capabilities = Capabilities::determine(sms).context("reading the Messages schema")?;
    let context = QueryContext::default();
    let mut stmt =
        Message::stream_rows(sms, &capabilities, &context).context("querying messages")?;
    let mut map = MessageMap::new();
    for row in Message::rows(&mut stmt, [])? {
        let m = match row {
            Ok(m) => m,
            Err(err) => {
                issues.push(Issue {
                    kind: issue::DECODE_FAILED,
                    guid: None,
                    source_rowid: None,
                    detail: Some(err.to_string()),
                });
                continue;
            }
        };
        counts.messages_seen += 1;
        let rowid = i64::from(m.rowid);
        let extras = extras_stmt.query_row([rowid], |r| {
            Ok(Extras {
                attributed_body: r.get(0)?,
                message_summary_info: r.get(1)?,
                payload_data: r.get(2)?,
                date_retracted: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                date_played: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                is_audio_message: r.get::<_, Option<bool>>(5)?.unwrap_or(false),
            })
        })?;

        let member_of: Vec<i64> = chat_join
            .get(&rowid)
            .map(|v| v.iter().filter_map(|c| chats.get(c).copied()).collect())
            .unwrap_or_default();
        let recovered_in = recoverable.get(&rowid).and_then(|c| chats.get(c).copied());
        let chat_id = member_of.first().copied().or(recovered_in);
        if chat_id.is_none() {
            issues.push(Issue {
                kind: issue::ORPHAN_MESSAGE,
                guid: Some(m.guid.clone()),
                source_rowid: Some(rowid),
                detail: None,
            });
        }

        let sender = if m.is_from_me {
            None
        } else {
            match m.handle_id.map(i64::from).filter(|&h| h > 0) {
                Some(h) => match handles.get(&h) {
                    Some(&id) => Some(id),
                    None => {
                        issues.push(Issue {
                            kind: issue::UNKNOWN_HANDLE,
                            guid: Some(m.guid.clone()),
                            source_rowid: Some(rowid),
                            detail: Some(format!("handle row {h}")),
                        });
                        None
                    }
                },
                None => None,
            }
        };
        let other_handle = m
            .other_handle
            .map(i64::from)
            .filter(|&h| h > 0)
            .and_then(|h| handles.get(&h).copied());
        let is_unsent = extras.date_retracted != 0 || m.is_fully_unsent();

        let added = insert.execute(params![
            m.guid,
            chat_id,
            sender,
            m.is_from_me,
            m.service,
            apple_to_unix_ns(m.date),
            apple_to_unix_ns(m.date_delivered),
            apple_to_unix_ns(m.date_read),
            apple_to_unix_ns(m.date_edited),
            apple_to_unix_ns(extras.date_retracted),
            apple_to_unix_ns(extras.date_played),
            m.text,
            m.subject,
            m.item_type,
            m.group_action_type,
            other_handle,
            m.group_title,
            m.associated_message_guid,
            m.associated_message_type,
            m.associated_message_emoji,
            m.thread_originator_guid,
            m.thread_originator_part,
            m.balloon_bundle_id,
            m.expressive_send_style_id,
            extras.is_audio_message,
            is_unsent,
            recovered_in.is_some() && member_of.is_empty(),
            source_id,
        ])?;
        counts.messages_added += added;
        let id: i64 = find.query_row(params![m.guid], |r| r.get(0))?;
        map.insert(rowid, id);
        held.execute(params![id, source_id, rowid])?;
        for c in member_of.iter().copied().chain(recovered_in) {
            in_chat.execute(params![id, c])?;
        }
        if added == 1 {
            raw.execute(params![
                id,
                extras.attributed_body,
                extras.message_summary_info,
                extras.payload_data
            ])?;
        }
        if let (Some(kind), Some((part, target))) =
            (m.associated_message_type, m.clean_associated_guid())
            && is_reaction(kind)
        {
            counts.reactions_added += react.execute(params![
                id,
                target,
                part as i64,
                kind,
                m.associated_message_emoji,
                (3000..4000).contains(&kind)
            ])?;
        }
    }
    Ok(map)
}

/// Tapbacks and emoji reactions (2000-2999), their removals (3000-3999), and
/// stickers placed on a message (1000).
fn is_reaction(kind: i32) -> bool {
    kind == 1000 || (2000..4000).contains(&kind)
}

fn import_attachments(
    tx: &Transaction,
    sms: &Connection,
    backup: &Backup,
    source_id: i64,
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
    // A later backup that has the file clears `missing`; nothing sets it back.
    let mut insert = tx.prepare_cached(
        "INSERT INTO attachment (guid, filename, transfer_name, uti, mime_type, total_bytes, is_sticker, is_outgoing,
                                 hide_attachment, created_at, missing, first_source_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT (guid) DO UPDATE SET missing = NULL WHERE excluded.missing IS NULL AND attachment.missing IS NOT NULL",
    )?;
    let mut find = tx.prepare_cached("SELECT id FROM attachment WHERE guid = ?1")?;
    let mut exists = tx.prepare_cached("SELECT 1 FROM attachment WHERE guid = ?1")?;
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
        let is_new = exists
            .query_row(params![guid], |_| Ok(()))
            .optional()?
            .is_none();
        insert.execute(params![
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
            missing,
            source_id,
        ])?;
        counts.attachments_added += usize::from(is_new);
        let id: i64 = find.query_row(params![guid], |r| r.get(0))?;
        seen_in.execute(params![id, source_id, rowid, file_id, present])?;
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

/// Reactions from this backup whose target message is not in the archive.
fn check_reaction_targets(tx: &Transaction, source_id: i64, issues: &mut Vec<Issue>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT m.guid, s.source_rowid FROM reaction r
         JOIN message m ON m.id = r.message_id
         JOIN message_source s ON s.message_id = m.id AND s.source_id = ?1
         WHERE NOT EXISTS (SELECT 1 FROM message t WHERE t.guid = r.target_guid)
         ORDER BY s.source_rowid",
    )?;
    let mut rows = stmt.query([source_id])?;
    while let Some(r) = rows.next()? {
        issues.push(Issue {
            kind: issue::REACTION_TARGET_MISSING,
            guid: Some(r.get(0)?),
            source_rowid: Some(r.get(1)?),
            detail: None,
        });
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
