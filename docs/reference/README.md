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
