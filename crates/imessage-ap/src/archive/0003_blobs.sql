-- archive.db, schema version 3: attachment files (Milestone 3a).
-- See docs/decisions/0005-blobs-and-derivatives.md.
--
-- `imessage-ap attachments` copies each attachment file of a backup into
-- /data/blobs/ab/cd/<sha256>, once per distinct content, and makes thumbnails.
-- Like the rest of archive.db, everything here can be rebuilt from backups.

-- Hash of the file this backup holds for the attachment; NULL until copied.
-- attachment.sha256 follows the newest backup that has a copied file.
ALTER TABLE attachment_source ADD COLUMN sha256 TEXT;
CREATE INDEX attachment_source_sha256 ON attachment_source (sha256);

-- What a run did: 'records' (ingest) or 'attachments' (file copy).
ALTER TABLE import_run ADD COLUMN kind TEXT NOT NULL DEFAULT 'records'
    CHECK (kind IN ('records', 'attachments'));

-- One row per file in /data/blobs.
CREATE TABLE blob (
    sha256 TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    -- How the file is treated, from the attachment that first brought it:
    -- web_image, image (needs a preview), video, voice, audio or other.
    media_kind TEXT NOT NULL,
    first_stored_at INTEGER NOT NULL
);

-- Files made from a blob: a thumbnail at import; previews, audio and video
-- conversions later, from the background queue. `recipe` is the version of
-- the settings that made it, so a changed recipe can be redone.
CREATE TABLE derivative (
    sha256 TEXT NOT NULL REFERENCES blob (sha256),
    kind TEXT NOT NULL CHECK (kind IN ('thumb', 'preview', 'audio', 'video')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'done', 'failed')),
    recipe INTEGER NOT NULL,
    -- Relative to the data directory; set when done.
    path TEXT,
    bytes INTEGER,
    -- Shortened tool output when failed. Never message content.
    error TEXT,
    made_at INTEGER,
    PRIMARY KEY (sha256, kind)
);
CREATE INDEX derivative_status ON derivative (status, kind);
