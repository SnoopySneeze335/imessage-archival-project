//! Messages, their versions, edit history and reactions
//! (decision 0004: versions and multiple backups).
//!
//! Identity is the GUID. Every distinct content version is kept, with the
//! backups that held it. The `message` row shows the version from the newest
//! backup (by backup date) that holds the message, so importing backups in
//! any order gives the same archive.

use std::collections::HashMap;

use anyhow::{Context, Result};
use imessage_database::{
    tables::{capabilities::Capabilities, messages::Message},
    util::query_context::QueryContext,
};
use rusqlite::{
    Connection, OptionalExtension, Transaction, params, params_from_iter, types::Value,
};

use super::{
    ChatMap, Counts, HandleMap, Issue, MessageMap, Source, apple_to_unix_ns, col, columns, issue,
};

/// Extra per-message fields the crate's `Message` does not carry.
struct Extras {
    attributed_body: Option<Vec<u8>>,
    message_summary_info: Option<Vec<u8>>,
    payload_data: Option<Vec<u8>>,
    date_retracted: i64,
    date_played: i64,
    is_audio_message: bool,
}

/// Columns of `message` besides `id`, `guid` and `first_source_id`, in the
/// order of `current` in [`import_messages`].
const CURRENT_COLUMNS: &str =
    "chat_id, sender_handle_id, is_from_me, service, sent_at, delivered_at,
    read_at, edited_at, retracted_at, played_at, text, subject, item_type, group_action_type,
    other_handle_id, group_title, associated_message_guid, associated_message_type,
    associated_message_emoji, reply_to_guid, reply_to_part, balloon_bundle_id,
    expressive_send_style_id, is_audio_message, is_unsent, is_recovered_deleted";

/// Content columns of `message_version` besides `message_id`, in the order of
/// `content` in [`import_messages`].
const VERSION_COLUMNS: [&str; 10] = [
    "text",
    "subject",
    "edited_at",
    "retracted_at",
    "is_unsent",
    "item_type",
    "balloon_bundle_id",
    "attributed_body",
    "message_summary_info",
    "payload_data",
];

fn placeholders(from: usize, n: usize) -> String {
    (from..from + n)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn import_messages(
    tx: &Transaction,
    sms: &Connection,
    source: Source,
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

    let mut insert = tx.prepare_cached(&format!(
        "INSERT INTO message (guid, {CURRENT_COLUMNS}, first_source_id)
         VALUES (?1, {}, ?28) ON CONFLICT (guid) DO NOTHING",
        placeholders(2, 26)
    ))?;
    let mut update = tx.prepare_cached(&format!(
        "UPDATE message SET ({CURRENT_COLUMNS}) = ({}) WHERE guid = ?1",
        placeholders(2, 26)
    ))?;
    let mut lower_first = tx.prepare_cached(
        "UPDATE message SET first_source_id = ?2
         WHERE id = ?1 AND (SELECT backup_date FROM source WHERE id = first_source_id) > ?3",
    )?;
    let mut find = tx.prepare_cached("SELECT id FROM message WHERE guid = ?1")?;
    let mut newest_holder = tx.prepare_cached(
        "SELECT MAX(s.backup_date) FROM message_source ms JOIN source s ON s.id = ms.source_id
         WHERE ms.message_id = ?1",
    )?;
    let same_content: Vec<String> = VERSION_COLUMNS
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{c} IS ?{}", i + 2))
        .collect();
    let mut find_version = tx.prepare_cached(&format!(
        "SELECT id FROM message_version WHERE message_id = ?1 AND {}",
        same_content.join(" AND ")
    ))?;
    let mut insert_version = tx.prepare_cached(&format!(
        "INSERT INTO message_version (message_id, {}) VALUES (?1, {})",
        VERSION_COLUMNS.join(", "),
        placeholders(2, VERSION_COLUMNS.len())
    ))?;
    let mut version_held = tx.prepare_cached(
        "INSERT OR IGNORE INTO message_version_source (version_id, source_id) VALUES (?1, ?2)",
    )?;
    let mut edit = tx.prepare_cached(
        "INSERT OR IGNORE INTO message_edit (message_id, part, edited_at, text) VALUES (?1, ?2, ?3, ?4)",
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
    let mut react_update = tx.prepare_cached(
        "UPDATE reaction SET (target_guid, target_part, kind, emoji, is_removal) = (?2, ?3, ?4, ?5, ?6) WHERE message_id = ?1",
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
        let edited_at = apple_to_unix_ns(m.date_edited);
        let retracted_at = apple_to_unix_ns(extras.date_retracted);

        // ?1 guid, then CURRENT_COLUMNS (?2..?27), then first_source_id (?28).
        let current: Vec<Value> = vec![
            m.guid.clone().into(),
            chat_id.into(),
            sender.into(),
            m.is_from_me.into(),
            m.service.clone().into(),
            apple_to_unix_ns(m.date).into(),
            apple_to_unix_ns(m.date_delivered).into(),
            apple_to_unix_ns(m.date_read).into(),
            edited_at.into(),
            retracted_at.into(),
            apple_to_unix_ns(extras.date_played).into(),
            m.text.clone().into(),
            m.subject.clone().into(),
            m.item_type.into(),
            m.group_action_type.into(),
            other_handle.into(),
            m.group_title.clone().into(),
            m.associated_message_guid.clone().into(),
            m.associated_message_type.into(),
            m.associated_message_emoji.clone().into(),
            m.thread_originator_guid.clone().into(),
            m.thread_originator_part.clone().into(),
            m.balloon_bundle_id.clone().into(),
            m.expressive_send_style_id.clone().into(),
            extras.is_audio_message.into(),
            is_unsent.into(),
            (recovered_in.is_some() && member_of.is_empty()).into(),
        ];
        let added = insert.execute(params_from_iter(
            current.iter().chain([&Value::from(source.id)]),
        ))? == 1;
        counts.messages_added += usize::from(added);
        let id: i64 = find.query_row(params![m.guid], |r| r.get(0))?;
        map.insert(rowid, id);
        held.execute(params![id, source.id, rowid])?;

        // Is this backup the newest that holds the message? Then its copy is
        // the current one. A re-import of the same backup rewrites the same
        // values, which changes nothing.
        let newest =
            added || newest_holder.query_row([id], |r| r.get::<_, i64>(0))? <= source.backup_date;
        if !added {
            if newest {
                update.execute(params_from_iter(current.iter()))?;
            }
            lower_first.execute(params![id, source.id, source.backup_date])?;
        }

        // ?1 message id, then VERSION_COLUMNS (?2..?11).
        let content: Vec<Value> = vec![
            id.into(),
            m.text.clone().into(),
            m.subject.clone().into(),
            edited_at.into(),
            retracted_at.into(),
            is_unsent.into(),
            m.item_type.into(),
            m.balloon_bundle_id.clone().into(),
            extras.attributed_body.into(),
            extras.message_summary_info.into(),
            extras.payload_data.into(),
        ];
        let existing = if added {
            None
        } else {
            find_version
                .query_row(params_from_iter(content.iter()), |r| r.get::<_, i64>(0))
                .optional()?
        };
        let version = match existing {
            Some(v) => v,
            None => {
                insert_version.execute(params_from_iter(content.iter()))?;
                counts.message_versions_added += 1;
                counts.messages_changed += usize::from(!added);
                tx.last_insert_rowid()
            }
        };
        version_held.execute(params![version, source.id])?;

        if let Some(parts) = &m.edited_parts {
            for (part, p) in parts.parts.iter().enumerate() {
                for e in &p.edit_history {
                    counts.message_edits_added +=
                        edit.execute(params![id, part as i64, apple_to_unix_ns(e.date), e.text])?;
                }
            }
        }

        for c in member_of.iter().copied().chain(recovered_in) {
            in_chat.execute(params![id, c])?;
        }
        if let (Some(kind), Some((part, target))) =
            (m.associated_message_type, m.clean_associated_guid())
            && is_reaction(kind)
        {
            let values = params![
                id,
                target,
                part as i64,
                kind,
                m.associated_message_emoji,
                (3000..4000).contains(&kind)
            ];
            let new = react.execute(values)?;
            counts.reactions_added += new;
            if new == 0 && newest {
                react_update.execute(values)?;
            }
        }
    }
    Ok(map)
}

/// Tapbacks and emoji reactions (2000-2999), their removals (3000-3999), and
/// stickers placed on a message (1000).
fn is_reaction(kind: i32) -> bool {
    kind == 1000 || (2000..4000).contains(&kind)
}

/// Reactions from this backup whose target message is not in the archive.
pub(super) fn check_reaction_targets(
    tx: &Transaction,
    source_id: i64,
    issues: &mut Vec<Issue>,
) -> Result<()> {
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
