-- archive.db, schema version 5: the background derive queue (Milestone 3b).
-- See docs/decisions/0007-derive-queue.md.
--
-- `imessage-ap derive` makes previews and audio/video conversions from the
-- files in /data/blobs. `action` records what it did: 'original' (browsers
-- use the blob as it is, so no file was made and `path` stays NULL),
-- 'remux' (streams copied into a new container) or 'transcode'. Thumbnails
-- leave it NULL.
ALTER TABLE derivative ADD COLUMN action TEXT
    CHECK (action IN ('original', 'remux', 'transcode'));

-- One row per `derive` run, like import_run for imports.
CREATE TABLE derive_run (
    id INTEGER PRIMARY KEY,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed')),
    -- Derivative kinds the run worked on, comma-separated.
    kinds TEXT NOT NULL,
    version TEXT NOT NULL,
    error TEXT,
    -- What the run did, as JSON counts.
    counts TEXT
);
