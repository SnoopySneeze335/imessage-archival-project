# 0003. Count-check differences on the Jan 4 backup

- Status: accepted
- Date: 2026-11-08
- Milestone: 2, Ingester (before 2b)

## Context

The count check on the real Jan 4, 2026 backup passed: 371,159 messages,
1,002 chats, 1,431 handle rows and 27,694 attachment records. Of the
attachment records, 25,040 have a file, 2,652 are not in the backup and 2
have no path. All of these match imessage-database 5.0.1's own diagnostics
(`docs/reference/jan4-baseline.json`). A second import added 0 records.

Three things still needed explaining before 2b: two archive counts that
differ from the crate's, and two issue kinds with large counts. The user ran
the read-only, counts-only queries below on the backup's `sms.db` and on
archive.db.

| Query | Result |
| --- | --- |
| a. Attachments with no surviving message (plain SQL on sms.db) | 2,632 |
| b. `message_attachment_join` rows pointing at a missing message | 0 |
| c. Reactions whose target GUID is not in sms.db; of those, in `deleted_messages` or `sync_deleted_messages` | 69; 0 |
| d. Distinct raw `handle.id`; distinct non-empty `person_centric_id` | 1,104; 87 |
| e. Archive attachments with no message, by file state | 2,586 not in backup, 46 present |
| f. Import runs 1 and 2, records added | all; 0 |

## Decision

**1. Unique addresses: 1,104 (archive) vs 1,097 (crate) are different measures. Not a bug.**

- The archive counts distinct normalized addresses (E.164 or lowercase
  email). 1,104 equals the number of distinct raw IDs (d), so normalization
  merges nothing more than identical IDs. All 1,431 − 1,104 = 327 collapsed
  rows are one address seen on two services (iMessage and SMS).
- The crate groups handle rows with union-find. It joins rows with the same
  raw ID and rows that share a `person_centric_id`, iOS's link between one
  person's phone numbers and emails. That joins 7 more groups: 1,104 − 7 =
  1,097, which is 1,431 − 334 merged. The crate's number approximates
  people, not addresses.
- The legacy build's 1,432 rows and 1,098 unique each counted iOS's
  synthetic handle row 0, which 5.0.1 leaves out.
- The legacy "87 with more than one ID" equals the number of distinct
  `person_centric_id` values (d). The crate's 313 counts groups of more
  than one row instead, so it includes the iMessage/SMS pairs.
- The handle key stays the normalized address plus service (decision 0002).
  Milestone 6 proposes person merges from `person_centric_id`, which
  `source_handle` already stores.

**2. Duplicated chats: 0 (archive) vs 31 (crate) are different measures. Not a bug, but Milestone 6 uses the crate's.**

- The archive's `chats_sharing_identifier` and the `duplicate_chat` issue
  find only chat rows with the same `chat_identifier`. The Jan 4 backup
  has none.
- The crate's chat dedupe merges in three passes: links in `chat_lookup`,
  then the same set of participants (with handles grouped as in 1), then
  display-name rules. That finds 31. The legacy build had neither
  `chat_lookup` nor handle grouping and found 23.
- Milestone 6 merge proposals use the crate's definition and are checked
  against 31. Until then the identifier-based stat and issue stay as a
  cheap check on the fixture.

**3. `attachment_without_message` (2,632) and `reaction_target_missing` (69) are iOS leftovers. Not a join bug.**

- Plain SQL on sms.db gives exactly the ingester's counts (a, c), so
  neither comes from the ingester's joins.
- Attachments: `message_attachment_join` has `ON DELETE CASCADE`, so
  deleting a message removes its join rows (b = 0). The attachment row
  itself stays. 2,586 of these 2,632 rows have no file (e), which is most
  of the 2,652 files "not in backup". Only 66 missing files belong to
  messages that still exist. The other 46 leftovers still have their files,
  which Milestone 3 copies like any other.
- Reactions: iOS's trigger meant to delete reactions with their target,
  `delete_associated_messages_after_delete_on_message`, can never match.
  It requires `OLD.associated_message_guid IS NULL` and then compares
  `guid` with that NULL. So reactions outlive their deleted targets.
- None of the 69 targets is in iOS's deleted-message logs (c). That proves
  nothing either way, because iOS clears those logs after syncing.
- Both stay recorded as issues. The viewer drops reactions that have no
  target, and a later backup that holds the target links it, because
  reactions point at their target by GUID.

## Consequences

- No code changes. The ingester's counts stand as the baseline.
- SPEC.md's diagnostics now use 5.0.1's figures. Its old data sizes,
  67.54 and 45.72, were binary gigabytes (GiB); the crate reports 72.52 GB
  referenced and 49.09 GB on disk.
- Milestone 6 is checked against 31 duplicated chats and 313
  multi-row handle groups, not 23 and 87.
- Milestone 3 expects 66 missing files on messages that still exist (plus
  the 2 with no path), and 2,586 on ownerless records the viewer never
  shows.
- The fixture gains an ownerless attachment and a reaction without a
  target in Milestone 2b, so both issue kinds are tested.
