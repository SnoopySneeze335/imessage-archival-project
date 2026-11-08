# 0002. archive.db identity, append-only imports, read-only backups

- Status: accepted
- Date: 2026-11-08
- Milestone: 2a, Ingester

## Context

The spec makes the archive append-only and imports repeatable: a message
missing from a newer backup stays, and importing the same backup twice
changes nothing. The Jan 4, 2026 backup is the only copy of the history
before that date, so reading it must never change it. Later milestones
(merging people and chats, the viewer, search) build on these rows, so their
keys must stay stable across imports and rebuilds.

## Decision

**Identity**

| Record | Key |
| --- | --- |
| source (one per backup) | device ID + backup date from the backup's plists |
| message, chat, attachment | Apple GUID |
| handle | normalized address (E.164 with US default, lowercase email, else raw) + service |
| reaction | the reaction's own message; the target is linked by GUID, not by row id |

Backup row numbers (`ROWID`) are never used as keys; they are recorded in
`source_handle`, `source_chat`, `message_source` and `attachment_source`,
which also record which backups held each record.

**Append-only**

- An import inserts with `ON CONFLICT DO NOTHING`, so the first stored copy
  of a record is kept. Milestone 2b adds versions for messages that changed.
- The one field a later import may change is `attachment.missing`, and only
  from missing to present, when another backup has the file.
- Each import is one transaction. If it fails, nothing is kept except the
  `import_run` row marked `failed` with its error message.
- `import_run` and `import_issue` grow with every run; they are the history
  of imports, not data. "Re-import changes nothing" means every other table
  is identical, which a test checks.

**Read-only backups**

- Databases in a backup are opened with `SQLITE_OPEN_READ_ONLY` and the URI
  options `mode=ro&immutable=1`, so SQLite creates no journal, WAL or
  shared-memory files beside them.
- Attachment files are only checked for existence in Milestone 2; copying
  (Milestone 3) reads them.
- A test hashes every file in the fixture backup before and after an import,
  with write permission removed.

**Reading**

- Messages are read through `imessage-database`, which decodes
  `attributedBody` text and edit history. Handles, chats, joins and
  attachments are read with plain SQL, because the crate's structs omit
  fields the archive keeps (chat GUID and style, handle service, attachment
  dates). Columns missing in older schemas read as NULL.
- The original `attributedBody`, `message_summary_info` and `payload_data`
  blobs are stored in `message_raw`, so nothing is lost before later
  milestones parse them.
- Timestamps are stored as UTC nanoseconds since 1970.

## Consequences

- Rebuilding from the same backups gives the same records with the same
  keys; only internal row ids may differ.
- A backup re-imported from another folder path is the same source (the
  first folder is kept).
- Encrypted backups are refused until Milestone 7 adds `crabapple`.
- Changing a key, or letting an import modify or delete stored rows beyond
  what is described here, needs a new decision record.
