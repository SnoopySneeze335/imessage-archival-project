//! sms.db: the Messages database, built from the real iOS 26.1 schema
//! (`docs/reference/sms-schema.sql`, applied by [`crate::reference`]).

use rusqlite::{Connection, params};

use crate::scenario::{
    Message, OWNER_EMAIL, OWNER_PHONE, SERVICE_SMS, Scenario, URL_BALLOON, apple_ns, apple_s,
};

/// `message_summary_info` for edited and unsent messages; NULL otherwise.
fn summary_info(m: &Message) -> Option<Vec<u8>> {
    let unsent = m.date_retracted != 0;
    (unsent || !m.edit_history.is_empty()).then(|| {
        let history: Vec<(i64, &str)> = m
            .edit_history
            .iter()
            .map(|(at, t)| (*at, t.as_str()))
            .collect();
        crate::plists::summary_info(&history, unsent)
    })
}

pub fn write(conn: &Connection, s: &Scenario) -> rusqlite::Result<()> {
    crate::reference::create_tables(conn, crate::reference::SMS_SCHEMA)?;
    let tx = conn.unchecked_transaction()?;

    for h in &s.handles {
        tx.execute(
            "INSERT INTO handle (ROWID, id, country, service, uncanonicalized_id, person_centric_id) VALUES (?1, ?2, 'us', ?3, NULL, NULL)",
            params![h.rowid, h.id, h.service],
        )?;
    }

    for c in &s.chats {
        let account = if c.service_name == SERVICE_SMS {
            format!("P:{OWNER_PHONE}")
        } else {
            format!("E:{OWNER_EMAIL}")
        };
        let room = (c.style == crate::scenario::STYLE_GROUP).then(|| c.chat_identifier.clone());
        tx.execute(
            "INSERT INTO chat (ROWID, guid, style, state, account_id, chat_identifier, service_name, room_name, account_login, display_name, group_id, last_addressed_handle)
             VALUES (?1, ?2, ?3, 3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                c.rowid,
                c.guid,
                c.style,
                format!("{:08X}-ACC0-4000-8000-000000000001", c.rowid),
                c.chat_identifier,
                c.service_name,
                room,
                account,
                c.display_name.clone().unwrap_or_default(),
                format!("{:08X}-6000-4000-8000-000000000000", c.rowid),
                OWNER_PHONE,
            ],
        )?;
        for h in &c.participants {
            tx.execute(
                "INSERT INTO chat_handle_join (chat_id, handle_id) VALUES (?1, ?2)",
                params![c.rowid, h],
            )?;
        }
    }

    for a in &s.attachments {
        tx.execute(
            "INSERT INTO attachment (ROWID, guid, created_date, start_date, filename, uti, mime_type, transfer_state, is_outgoing, transfer_name, total_bytes, is_sticker, original_guid)
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, 5, ?7, ?8, ?9, ?10, ?2)",
            params![
                a.rowid,
                a.guid,
                apple_s(a.created),
                a.filename,
                a.uti,
                a.mime_type,
                a.is_outgoing,
                a.transfer_name,
                a.total_bytes,
                a.is_sticker,
            ],
        )?;
    }

    for m in &s.messages {
        let account = if m.service == SERVICE_SMS {
            format!("P:{OWNER_PHONE}")
        } else {
            format!("E:{OWNER_EMAIL}")
        };
        let destination = if m.service == SERVICE_SMS {
            OWNER_PHONE
        } else {
            OWNER_EMAIL
        };
        let opt = |t: i64| (t != 0).then(|| apple_ns(t));
        tx.execute(
            "INSERT INTO message (
                ROWID, guid, text, handle_id, service, account, account_guid, date, date_read, date_delivered,
                is_delivered, is_finished, is_from_me, is_read, is_sent, is_system_message, cache_has_attachments,
                is_audio_message, item_type, other_handle, group_title, group_action_type,
                associated_message_guid, associated_message_type, associated_message_range_location,
                associated_message_range_length, associated_message_emoji, destination_caller_id,
                thread_originator_guid, thread_originator_part, date_retracted, date_edited, part_count, sort_id,
                attributedBody, balloon_bundle_id, message_summary_info, payload_data
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21,
                      ?22, ?23, 0, ?24, ?25, ?26, ?27, ?28, ?29, ?30, 1, 0, ?31, ?32, ?33, ?34)",
            params![
                m.rowid,
                m.guid,
                if m.text_only_in_body { None } else { m.text.clone() },
                m.handle_id,
                m.service,
                account,
                "ACC00000-0000-4000-8000-000000000001",
                apple_ns(m.date),
                apple_ns(m.date_read),
                apple_ns(m.date_delivered),
                m.is_delivered,
                m.is_from_me,
                m.is_read,
                m.is_from_me,
                m.is_system_message,
                !m.attachments.is_empty(),
                m.is_audio_message,
                m.item_type,
                m.other_handle,
                m.group_title,
                m.group_action_type,
                m.associated_message_guid,
                m.associated_message_type,
                m.associated_message_range_length,
                m.associated_message_emoji,
                destination,
                m.thread_originator_guid,
                m.thread_originator_part,
                opt(m.date_retracted),
                opt(m.date_edited),
                m.text.as_deref().map(crate::typedstream::attributed_body),
                m.link_preview.as_ref().map(|_| URL_BALLOON),
                summary_info(m),
                m.link_preview
                    .as_ref()
                    .map(|l| crate::plists::link_preview(l.url, l.title, l.summary, l.site_name)),
            ],
        )?;
        if let Some(chat) = m.chat {
            tx.execute(
                "INSERT INTO chat_message_join (chat_id, message_id, message_date) VALUES (?1, ?2, ?3)",
                params![chat, m.rowid, apple_ns(m.date)],
            )?;
        }
        if let Some(chat) = m.recoverable_in {
            tx.execute(
                "INSERT INTO chat_recoverable_message_join (chat_id, message_id, delete_date) VALUES (?1, ?2, ?3)",
                params![chat, m.rowid, apple_ns(m.date + 3_600)],
            )?;
        }
        for a in &m.attachments {
            tx.execute(
                "INSERT INTO message_attachment_join (message_id, attachment_id) VALUES (?1, ?2)",
                params![m.rowid, a],
            )?;
        }
    }
    tx.commit()?;
    crate::reference::finish(conn, crate::reference::SMS_SCHEMA)
}
