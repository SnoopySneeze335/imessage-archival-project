-- archive.db, schema version 2: message versions (Milestone 2b).
-- See docs/decisions/0004-versions-and-multiple-backups.md.
--
-- A message's content can differ between backups (edited or unsent after an
-- earlier backup). Every distinct content version is kept, with the backups
-- that held it. The `message` row holds the current version: the one from the
-- newest backup (by backup date) that holds the message, so the result does
-- not depend on the order backups are imported in.

CREATE TABLE message_version (
    id INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES message (id),
    text TEXT,
    subject TEXT,
    edited_at INTEGER,
    retracted_at INTEGER,
    is_unsent INTEGER NOT NULL,
    item_type INTEGER NOT NULL,
    balloon_bundle_id TEXT,
    -- The original blobs, so nothing is lost before later milestones parse them.
    attributed_body BLOB,
    message_summary_info BLOB,
    payload_data BLOB
);
CREATE INDEX message_version_message ON message_version (message_id);

-- Every backup that held each version.
CREATE TABLE message_version_source (
    version_id INTEGER NOT NULL REFERENCES message_version (id),
    source_id INTEGER NOT NULL REFERENCES source (id),
    PRIMARY KEY (version_id, source_id)
);

-- Edit history parsed from message_summary_info: every version of each body
-- part, keyed by its edit time, so histories from several backups combine.
-- The original text of a part is its earliest entry.
CREATE TABLE message_edit (
    message_id INTEGER NOT NULL REFERENCES message (id),
    part INTEGER NOT NULL,
    edited_at INTEGER NOT NULL,
    text TEXT NOT NULL,
    PRIMARY KEY (message_id, part, edited_at)
);

-- Schema 1 kept one copy per message: it becomes that message's first version,
-- held by every backup that held the message. message_edit fills on the next
-- import of each backup.
INSERT INTO message_version (
    message_id, text, subject, edited_at, retracted_at, is_unsent, item_type, balloon_bundle_id,
    attributed_body, message_summary_info, payload_data
)
SELECT m.id, m.text, m.subject, m.edited_at, m.retracted_at, m.is_unsent, m.item_type,
       m.balloon_bundle_id, r.attributed_body, r.message_summary_info, r.payload_data
FROM message m LEFT JOIN message_raw r ON r.message_id = m.id
ORDER BY m.id;

INSERT INTO message_version_source (version_id, source_id)
SELECT v.id, s.source_id FROM message_version v JOIN message_source s ON s.message_id = v.message_id;

DROP TABLE message_raw;
