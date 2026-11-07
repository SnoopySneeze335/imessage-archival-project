//! sms.db: the Messages database, in the shape recent iOS versions use.
//!
//! Only the tables and columns imessage-database reads are created, with
//! iOS's names, types and defaults.

use rusqlite::{Connection, params};

use crate::scenario::{OWNER_EMAIL, OWNER_PHONE, SERVICE_SMS, Scenario, apple_ns, apple_s};

const SCHEMA: &str = r"
CREATE TABLE handle (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE,
    id TEXT NOT NULL,
    country TEXT,
    service TEXT NOT NULL,
    uncanonicalized_id TEXT,
    person_centric_id TEXT,
    UNIQUE (id, service)
);
CREATE TABLE chat (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
    guid TEXT UNIQUE NOT NULL,
    style INTEGER,
    state INTEGER,
    account_id TEXT,
    properties BLOB,
    chat_identifier TEXT,
    service_name TEXT,
    room_name TEXT,
    account_login TEXT,
    is_archived INTEGER DEFAULT 0,
    last_addressed_handle TEXT,
    display_name TEXT,
    group_id TEXT,
    is_filtered INTEGER DEFAULT 0,
    successful_query INTEGER,
    engram_id TEXT,
    server_change_token TEXT,
    ck_sync_state INTEGER DEFAULT 0,
    original_group_id TEXT,
    last_read_message_timestamp INTEGER DEFAULT 0,
    cloudkit_record_id TEXT,
    last_addressed_sim_id TEXT,
    is_blackholed INTEGER DEFAULT 0,
    syndication_date INTEGER DEFAULT 0,
    syndication_type INTEGER DEFAULT 0,
    is_recovered INTEGER DEFAULT 0,
    is_deleting_incoming_messages INTEGER DEFAULT 0
);
CREATE TABLE message (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
    guid TEXT UNIQUE NOT NULL,
    text TEXT,
    replace INTEGER DEFAULT 0,
    service_center TEXT,
    handle_id INTEGER DEFAULT 0,
    subject TEXT,
    country TEXT,
    attributedBody BLOB,
    version INTEGER DEFAULT 0,
    type INTEGER DEFAULT 0,
    service TEXT,
    account TEXT,
    account_guid TEXT,
    error INTEGER DEFAULT 0,
    date INTEGER,
    date_read INTEGER,
    date_delivered INTEGER,
    is_delivered INTEGER DEFAULT 0,
    is_finished INTEGER DEFAULT 0,
    is_emote INTEGER DEFAULT 0,
    is_from_me INTEGER DEFAULT 0,
    is_empty INTEGER DEFAULT 0,
    is_delayed INTEGER DEFAULT 0,
    is_auto_reply INTEGER DEFAULT 0,
    is_prepared INTEGER DEFAULT 0,
    is_read INTEGER DEFAULT 0,
    is_system_message INTEGER DEFAULT 0,
    is_sent INTEGER DEFAULT 0,
    has_dd_results INTEGER DEFAULT 0,
    is_service_message INTEGER DEFAULT 0,
    is_forward INTEGER DEFAULT 0,
    was_downgraded INTEGER DEFAULT 0,
    is_archive INTEGER DEFAULT 0,
    cache_has_attachments INTEGER DEFAULT 0,
    cache_roomnames TEXT,
    was_data_detected INTEGER DEFAULT 0,
    was_deduplicated INTEGER DEFAULT 0,
    is_audio_message INTEGER DEFAULT 0,
    is_played INTEGER DEFAULT 0,
    date_played INTEGER,
    item_type INTEGER DEFAULT 0,
    other_handle INTEGER DEFAULT 0,
    group_title TEXT,
    group_action_type INTEGER DEFAULT 0,
    share_status INTEGER DEFAULT 0,
    share_direction INTEGER DEFAULT 0,
    is_expirable INTEGER DEFAULT 0,
    expire_state INTEGER DEFAULT 0,
    message_action_type INTEGER DEFAULT 0,
    message_source INTEGER DEFAULT 0,
    associated_message_guid TEXT,
    associated_message_type INTEGER DEFAULT 0,
    balloon_bundle_id TEXT,
    payload_data BLOB,
    expressive_send_style_id TEXT,
    associated_message_range_location INTEGER DEFAULT 0,
    associated_message_range_length INTEGER DEFAULT 0,
    time_expressive_send_played INTEGER,
    message_summary_info BLOB,
    ck_sync_state INTEGER DEFAULT 0,
    ck_record_id TEXT,
    ck_record_change_tag TEXT,
    destination_caller_id TEXT,
    is_corrupt INTEGER DEFAULT 0,
    reply_to_guid TEXT,
    sort_id INTEGER,
    is_spam INTEGER DEFAULT 0,
    has_unseen_mention INTEGER DEFAULT 0,
    thread_originator_guid TEXT,
    thread_originator_part TEXT,
    syndication_ranges TEXT,
    synced_syndication_ranges TEXT,
    was_delivered_quietly INTEGER DEFAULT 0,
    did_notify_recipient INTEGER DEFAULT 0,
    date_retracted INTEGER,
    date_edited INTEGER,
    was_detonated INTEGER DEFAULT 0,
    part_count INTEGER,
    is_stewie INTEGER DEFAULT 0,
    is_kt_verified INTEGER DEFAULT 0,
    is_sos INTEGER DEFAULT 0,
    is_critical INTEGER DEFAULT 0,
    bia_reference_id TEXT,
    fallback_hash TEXT,
    associated_message_emoji TEXT,
    is_pending_satellite_send INTEGER DEFAULT 0,
    needs_relay INTEGER DEFAULT 0,
    schedule_type INTEGER DEFAULT 0,
    schedule_state INTEGER DEFAULT 0,
    sent_or_received_off_grid INTEGER DEFAULT 0
);
CREATE TABLE attachment (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
    guid TEXT UNIQUE NOT NULL,
    created_date INTEGER DEFAULT 0,
    start_date INTEGER DEFAULT 0,
    filename TEXT,
    uti TEXT,
    mime_type TEXT,
    transfer_state INTEGER DEFAULT 0,
    is_outgoing INTEGER DEFAULT 0,
    user_info BLOB,
    transfer_name TEXT,
    total_bytes INTEGER DEFAULT 0,
    is_sticker INTEGER DEFAULT 0,
    sticker_user_info BLOB,
    attribution_info BLOB,
    hide_attachment INTEGER DEFAULT 0,
    ck_sync_state INTEGER DEFAULT 0,
    ck_server_change_token_blob BLOB,
    ck_record_id TEXT,
    original_guid TEXT UNIQUE NOT NULL,
    is_commsafety_sensitive INTEGER DEFAULT 0,
    emoji_image_content_identifier TEXT,
    emoji_image_short_description TEXT,
    preview_generation_state INTEGER DEFAULT 0
);
CREATE TABLE chat_handle_join (
    chat_id INTEGER REFERENCES chat (ROWID) ON DELETE CASCADE,
    handle_id INTEGER REFERENCES handle (ROWID) ON DELETE CASCADE,
    UNIQUE (chat_id, handle_id)
);
CREATE TABLE chat_message_join (
    chat_id INTEGER REFERENCES chat (ROWID) ON DELETE CASCADE,
    message_id INTEGER REFERENCES message (ROWID) ON DELETE CASCADE,
    message_date INTEGER DEFAULT 0,
    PRIMARY KEY (chat_id, message_id)
);
CREATE TABLE message_attachment_join (
    message_id INTEGER REFERENCES message (ROWID) ON DELETE CASCADE,
    attachment_id INTEGER REFERENCES attachment (ROWID) ON DELETE CASCADE,
    UNIQUE (message_id, attachment_id)
);
CREATE TABLE chat_recoverable_message_join (
    chat_id INTEGER REFERENCES chat (ROWID) ON DELETE CASCADE,
    message_id INTEGER REFERENCES message (ROWID) ON DELETE CASCADE,
    delete_date INTEGER,
    ck_sync_state INTEGER DEFAULT 0,
    PRIMARY KEY (chat_id, message_id)
);
CREATE TABLE deleted_messages (ROWID INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE, guid TEXT NOT NULL);
CREATE INDEX chat_message_join_idx_message_date_id_chat_id ON chat_message_join (chat_id, message_date, message_id);
CREATE INDEX message_idx_handle ON message (handle_id, date);
";

pub fn write(conn: &Connection, s: &Scenario) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
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
                thread_originator_guid, thread_originator_part, date_retracted, date_edited, part_count, sort_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21,
                      ?22, ?23, 0, ?24, ?25, ?26, ?27, ?28, ?29, ?30, 1, 0)",
            params![
                m.rowid,
                m.guid,
                m.text,
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
    tx.commit()
}
