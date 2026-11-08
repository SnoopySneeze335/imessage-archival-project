# 0004. Message versions and multiple backups

- Status: accepted
- Date: 2026-11-08
- Milestone: 2b, Ingester
- Supersedes: in [0002](0002-archive-identity.md), "the first stored copy of a
  record is kept" and "the one field a later import may change is
  `attachment.missing`". Its identity keys and read-only rules stand.

## Context

Later backups of the same phone hold the same messages, sometimes changed.
A message edited or unsent after one backup differs in the next, read and
delivered times fill in, groups get renamed, and a file missing from one
backup can be in another. Messages deleted from the phone are absent from
newer backups. The spec says the archive is append-only, keeps every
version of a changed message, and keeps messages a newer backup lacks.

Backups will not always be imported in date order. Older backups may be
imported later to fill missing files (spec, v2). Under 2a's rule ("the first
stored copy wins"), the result would depend on import order.

## Decision

**Current values come from the newest backup, not the latest import.** For
each message, chat and attachment, the stored row shows the copy from the
backup with the latest `backup_date` among those that hold it. An import of
an older backup never overwrites a newer one's values. Importing the same
backups in any order therefore gives the same archive, with only internal
row ids differing; a test checks this.

**Every content version of a message is kept.**

- `message_version` holds each distinct content: text, subject, edit and
  unsend times, unsent flag, item type, balloon bundle ID, and the original
  `attributedBody`, `message_summary_info` and `payload_data` blobs.
  `message_version_source` records which backups held each version.
- Versions are found by comparing every content column (`IS`, blobs
  byte-for-byte), so no hashing dependency is needed.
- Status fields (delivered, read, played times), chat links and the sender
  are not content: they follow the newest backup and create no version.
- The blobs moved from `message_raw`, which schema 2 drops, into
  `message_version`, so they are stored once.

**Edit history is parsed** from `message_summary_info` with
`imessage-database` into `message_edit`. Each row is keyed by message, body
part and edit time, so histories from several backups combine without
duplicates in any order.

**Other fields**

- `first_source_id` (message, attachment) means the earliest backup, by
  backup date, holding the record.
- `attachment.missing` is derived from every backup's `attachment_source`
  row. It is NULL if any backup has the file, `not_in_backup` if some backup
  has a path, and `no_path` otherwise.
- Handles, `message_chat`, `chat_participant` and the `source_*` tables are
  unions, as before.
- A message missing from a newer backup is untouched: it stays, held only by
  the backups that had it.

**Migration.** Schema 2 converts a schema 1 archive in place.

- Each message's stored copy becomes its first version, held by every backup
  in `message_source`.
- `message_edit` starts empty and fills on the next import of each backup;
  that import adds no versions.
- A test downgrades a fresh archive to schema 1, migrates it, re-imports, and
  checks the result against a fresh import.

## Consequences

- An import of an older backup can still add records: new versions, rows in
  `source_*`, a file that clears `missing`, an earlier `first_source_id`.
  It never replaces values from a newer backup.
- "Re-import changes nothing" still holds for any already-imported backup.
- Backups of two different phones with the same message GUID (rare) are
  treated as one message. The newer backup's copy is current and both
  versions are kept.
- Deleting, or changing the rules above, needs a new decision record.
