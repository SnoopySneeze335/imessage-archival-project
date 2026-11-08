-- archive.db, schema version 1. See docs/SPEC.md, "Data model and storage",
-- and docs/decisions/0002-archive-identity.md.
--
-- Everything here can be rebuilt from the backups. Identity: messages, chats
-- and attachments by Apple GUID; handles by (address, service); sources by
-- (device_id, backup_date). The archive is append-only: imports add rows and
-- never delete them.
--
-- Times are UTC nanoseconds since 1970-01-01. NULL means "not set".

CREATE TABLE source (
    id INTEGER PRIMARY KEY,
    device_id TEXT NOT NULL,
    backup_date INTEGER NOT NULL,
    folder TEXT NOT NULL,
    device_name TEXT,
    product_type TEXT,
    ios_version TEXT,
    build_version TEXT,
    is_encrypted INTEGER NOT NULL,
    first_imported_at INTEGER NOT NULL,
    UNIQUE (device_id, backup_date)
);

CREATE TABLE import_run (
    id INTEGER PRIMARY KEY,
    source_id INTEGER REFERENCES source (id),
    backup_folder TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed')),
    ingester_version TEXT NOT NULL,
    imessage_database_version TEXT NOT NULL,
    -- Error message on failure. Never contains message content.
    error TEXT,
    -- JSON: rows seen, added and unchanged per table.
    counts TEXT,
    -- JSON: imessage-database's own diagnostics of the backup (counts only).
    diagnostics TEXT
);

CREATE TABLE import_issue (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES import_run (id),
    kind TEXT NOT NULL,
    guid TEXT,
    source_rowid INTEGER,
    -- Short, content-free detail such as a missing column name.
    detail TEXT
);
CREATE INDEX import_issue_run ON import_issue (run_id, kind);

CREATE TABLE handle (
    id INTEGER PRIMARY KEY,
    -- E.164 phone number, lowercase email, or the raw ID when neither applies
    -- (short codes, business IDs).
    address TEXT NOT NULL,
    service TEXT NOT NULL,
    UNIQUE (address, service)
);

-- One row per handle row in each backup, with its raw values.
CREATE TABLE source_handle (
    source_id INTEGER NOT NULL REFERENCES source (id),
    source_rowid INTEGER NOT NULL,
    handle_id INTEGER NOT NULL REFERENCES handle (id),
    raw_id TEXT NOT NULL,
    country TEXT,
    uncanonicalized_id TEXT,
    person_centric_id TEXT,
    PRIMARY KEY (source_id, source_rowid)
);

CREATE TABLE chat (
    id INTEGER PRIMARY KEY,
    guid TEXT NOT NULL UNIQUE,
    chat_identifier TEXT,
    service TEXT,
    style INTEGER,
    display_name TEXT,
    room_name TEXT,
    group_id TEXT
);
CREATE INDEX chat_identifier ON chat (chat_identifier);

-- One row per chat row in each backup, with that backup's display name.
CREATE TABLE source_chat (
    source_id INTEGER NOT NULL REFERENCES source (id),
    source_rowid INTEGER NOT NULL,
    chat_id INTEGER NOT NULL REFERENCES chat (id),
    display_name TEXT,
    PRIMARY KEY (source_id, source_rowid)
);

CREATE TABLE chat_participant (
    chat_id INTEGER NOT NULL REFERENCES chat (id),
    handle_id INTEGER NOT NULL REFERENCES handle (id),
    source_id INTEGER NOT NULL REFERENCES source (id),
    PRIMARY KEY (chat_id, handle_id, source_id)
);

CREATE TABLE message (
    id INTEGER PRIMARY KEY,
    guid TEXT NOT NULL UNIQUE,
    -- Primary chat. NULL for messages linked to no chat.
    chat_id INTEGER REFERENCES chat (id),
    -- NULL when sent by the owner or when the backup names no handle.
    sender_handle_id INTEGER REFERENCES handle (id),
    is_from_me INTEGER NOT NULL,
    service TEXT,
    sent_at INTEGER,
    delivered_at INTEGER,
    read_at INTEGER,
    edited_at INTEGER,
    retracted_at INTEGER,
    played_at INTEGER,
    -- Plain text, decoded from attributedBody when message.text is empty.
    text TEXT,
    subject TEXT,
    item_type INTEGER NOT NULL DEFAULT 0,
    group_action_type INTEGER NOT NULL DEFAULT 0,
    other_handle_id INTEGER REFERENCES handle (id),
    group_title TEXT,
    associated_message_guid TEXT,
    associated_message_type INTEGER,
    associated_message_emoji TEXT,
    reply_to_guid TEXT,
    reply_to_part TEXT,
    balloon_bundle_id TEXT,
    expressive_send_style_id TEXT,
    is_audio_message INTEGER NOT NULL DEFAULT 0,
    is_unsent INTEGER NOT NULL DEFAULT 0,
    is_recovered_deleted INTEGER NOT NULL DEFAULT 0,
    first_source_id INTEGER NOT NULL REFERENCES source (id)
);
CREATE INDEX message_chat_sent ON message (chat_id, sent_at);
CREATE INDEX message_reply_to ON message (reply_to_guid);

-- Every chat a message belongs to (usually one).
CREATE TABLE message_chat (
    message_id INTEGER NOT NULL REFERENCES message (id),
    chat_id INTEGER NOT NULL REFERENCES chat (id),
    PRIMARY KEY (message_id, chat_id)
);

-- The original blobs, so nothing is lost before later milestones parse them.
CREATE TABLE message_raw (
    message_id INTEGER PRIMARY KEY REFERENCES message (id),
    attributed_body BLOB,
    message_summary_info BLOB,
    payload_data BLOB
);

-- Every backup that held each message.
CREATE TABLE message_source (
    message_id INTEGER NOT NULL REFERENCES message (id),
    source_id INTEGER NOT NULL REFERENCES source (id),
    source_rowid INTEGER NOT NULL,
    PRIMARY KEY (message_id, source_id)
);

-- Reactions are messages too; this links each one to its target by GUID, so
-- the link survives the target arriving in a later backup.
CREATE TABLE reaction (
    message_id INTEGER PRIMARY KEY REFERENCES message (id),
    target_guid TEXT NOT NULL,
    target_part INTEGER NOT NULL,
    -- associated_message_type: 2000-2007 add, 3000-3007 remove, 1000 sticker.
    kind INTEGER NOT NULL,
    emoji TEXT,
    is_removal INTEGER NOT NULL
);
CREATE INDEX reaction_target ON reaction (target_guid);

CREATE TABLE attachment (
    id INTEGER PRIMARY KEY,
    guid TEXT NOT NULL UNIQUE,
    -- Path as sms.db stores it.
    filename TEXT,
    transfer_name TEXT,
    uti TEXT,
    mime_type TEXT,
    total_bytes INTEGER,
    is_sticker INTEGER NOT NULL DEFAULT 0,
    is_outgoing INTEGER NOT NULL DEFAULT 0,
    hide_attachment INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER,
    -- Filled in Milestone 3 when the file is copied into /data/blobs.
    sha256 TEXT,
    -- NULL when some imported backup has the file.
    missing TEXT CHECK (missing IN ('not_in_backup', 'no_path')),
    first_source_id INTEGER NOT NULL REFERENCES source (id)
);

CREATE TABLE message_attachment (
    message_id INTEGER NOT NULL REFERENCES message (id),
    attachment_id INTEGER NOT NULL REFERENCES attachment (id),
    PRIMARY KEY (message_id, attachment_id)
);

CREATE TABLE attachment_source (
    attachment_id INTEGER NOT NULL REFERENCES attachment (id),
    source_id INTEGER NOT NULL REFERENCES source (id),
    source_rowid INTEGER NOT NULL,
    -- Backup file ID (SHA-1 of "MediaDomain-<path>"); NULL when no path.
    backup_file_id TEXT,
    present INTEGER NOT NULL,
    PRIMARY KEY (attachment_id, source_id)
);
