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
Jan 4, 2026 backup. It holds counts only: no message content, names (apart
from the device name) or addresses.

| | |
| --- | --- |
| Source backup | Jan 4, 2026 local backup (`backup_date` 1767575013 = 2026-01-04 17:03 PST) |
| iOS version | 26.1 |
| Reader | `imessage-database` 5.0.1, archive schema v1 (Milestone 2a) |
| Checked | 2026-11-08 |

After any change to the ingester, or an upgrade of `imessage-database`, run
the count check again and compare it with this file (see AGENTS.md). Every
difference must be explained in the PR. An expected change, such as a newer
crate decoding more, means replacing this file in that PR and updating the
table above.

The headline counts match the diagnostics in SPEC.md. They came from an
unknown legacy imessage-exporter build:

| Measure | SPEC.md (legacy build) | Baseline (5.0.1) |
| --- | --- | --- |
| Messages | 371,159 | 371,159 |
| Not linked to a chat | 33 | 33 (26 orphans + 7 recently deleted) |
| Chats | 1,002 | 1,002 |
| Attachment records | 27,694 | 27,694 |
| Files missing (incl. no path) | 2,654 | 2,654 |
| No path | 2 | 2 |
| Attachments with a file | (25,040 implied) | 25,040 |
| Attachment data referenced / on disk | 67.54 / 45.72 GB | 67.53 / 45.71 GiB (72.52 / 49.09 GB) |

Three handle and chat counts differ. Both builds read the same data but group
it differently:

| Measure | SPEC.md (legacy build) | Baseline (5.0.1) |
| --- | --- | --- |
| Handle rows | 1,432 | 1,431 |
| Duplicated chats | 23 | 31 |
| Handles with more than one ID | 87 | 313 |

Duplicate handles removed are 334 in both. None of the three differences
blocks Milestone 2. Chat and handle merging is built and checked in
Milestone 6 (Contacts), so these numbers are rechecked there. The data sizes
in SPEC.md are binary gigabytes (GiB); `imessage-ap stats` prints decimal GB.
