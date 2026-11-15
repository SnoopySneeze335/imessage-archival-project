# Reference schemas

Schemas, with no data, of the two databases the ingester reads, taken from
the real Jan 4, 2026 backup (iOS 26.1) with the sqlite3 shell's `.schema`.

| File | Database inside the backup |
| --- | --- |
| `sms-schema.sql` | Messages: `HomeDomain/Library/SMS/sms.db` (file `3d/3d0d7e5f…`) |
| `addressbook-schema.sql` | Contacts: `HomeDomain/Library/AddressBook/AddressBook.sqlitedb` (file `31/31bb7ba8…`) |

The fixture generator (`fixtures/generator`) builds its databases from these
files, and its build fails unless the result has exactly these tables,
indexes and triggers. See `fixtures/generator/src/reference.rs` for the few
statements that cannot be executed verbatim (SQLite-internal tables, FTS4
tables with Apple's tokenizer, triggers calling Apple-only functions) and
how each is reproduced.

When a newer iOS version changes the schema, replace these files with a fresh
`.schema` dump (never `.dump`, which includes data) and fix the generator
until its schema check passes again.

## Count-check baseline

`jan4-baseline.json` is the `imessage-ap stats --json` output for the real
Jan 4, 2026 backup. It holds counts only: no message content, names or
addresses. The device name is replaced with `<device name>`; `stats` prints the
real one, so leave it out when updating this file.

| | |
| --- | --- |
| Source backup | Jan 4, 2026 local backup (`backup_date` 1767575013 = 2026-01-04 17:03 PST) |
| iOS version | 26.1 |
| Reader | `imessage-database` 5.0.1, archive schema v3 (Milestone 3a) |
| Checked | 2026-11-08 (schema v1); 2026-11-15 after migrating to v2 and re-importing; 2026-11-15 fresh v3 import plus attachment files |

The v2 check migrated the v1 archive in place and re-imported the backup. The
re-import added no records and no versions (0 messages changed) and found the
same 5,381 issues, so every migrated message matched what the v2 ingester
reads. It added the 3,029 edit-history entries that schema v1 did not parse.
All other counts are unchanged from v1.

The v3 check started from an empty data directory: one `ingest`, then
`attachments` in four runs (`--limit 2000`, a run stopped with `docker stop`,
a run that resumed and finished, and a final run that read 0 files). Record
counts are unchanged from v2; `import_runs_succeeded` is 1 because the data
directory was fresh, and attachment runs are not counted there.

- 25,040 files copied, exactly the 25,040 attachments with a file. The other
  2,654 are the 2,652 missing files and 2 records with no path.
- `attachment_bytes_copied` is 49,086,254,711, exactly the crate's
  `attachment_bytes_on_disk`. `blob_bytes` is lower (43,430,863,802) because
  5,239 copies were duplicates of 19,801 distinct files.
- 16,736 thumbnails made and 28 failed. Every image and video blob gets one
  thumbnail row, and none are left pending. `derivatives_failed` is 28
  because in schema 3 only thumbnails can fail; the 7,047 pending derivatives
  (HEIC previews, voice and video conversions) wait for Milestone 3b.

After any change to the ingester, or an upgrade of `imessage-database`, run
the count check again and compare it with this file (see AGENTS.md). Every
difference must be explained in the PR. An expected change, such as a newer
crate decoding more, means replacing this file in that PR and updating the
table above.

The headline counts match the diagnostics SPEC.md first listed, which came from an
unknown legacy imessage-exporter build:

| Measure | Legacy build (old SPEC.md) | Baseline (5.0.1) |
| --- | --- | --- |
| Messages | 371,159 | 371,159 |
| Not linked to a chat | 33 | 33 (26 orphans + 7 recently deleted) |
| Chats | 1,002 | 1,002 |
| Attachment records | 27,694 | 27,694 |
| Files missing (incl. no path) | 2,654 | 2,654 |
| No path | 2 | 2 |
| Attachments with a file | (25,040 implied) | 25,040 |
| Attachment data referenced / on disk | 67.54 / 45.72 GiB | 67.53 / 45.71 GiB (72.52 / 49.09 GB) |

Three handle and chat counts differ. Both builds read the same data but group
it differently:

| Measure | Legacy build (old SPEC.md) | Baseline (5.0.1) |
| --- | --- | --- |
| Handle rows | 1,432 | 1,431 |
| Duplicated chats | 23 | 31 |
| Handles with more than one ID | 87 | 313 |

Duplicate handles removed are 334 in both. The differences, and the archive's
own unique-address and duplicate-chat counts, are explained in
[decision 0003](../decisions/0003-count-check-differences.md); SPEC.md now
shows the 5.0.1 figures. The legacy data sizes were binary gigabytes (GiB);
`imessage-ap stats` and SPEC.md use decimal GB.
