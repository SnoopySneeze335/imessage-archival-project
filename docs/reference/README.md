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
| Reader | `imessage-database` 5.0.1, archive schema v5 (Milestone 3b, decision 0007) |
| Checked | 2026-11-08 (schema v1); 2026-11-15 after migrating to v2 and re-importing; 2026-11-15 fresh v3 import plus attachment files; 2026-11-16 v4 thumbnail retry; 2026-11-16 v5 derive queue |

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
- 16,764 thumbnails, one for every image and video blob, none pending. The
  v3 run failed 28 of them on undamaged files: 27 HEIF image sequences and
  1 MP4 with a `chnl` v1 box ([decision 0006](../decisions/0006-decoder-workarounds.md)).
  The first `attachments` run on schema v4 retried exactly those 28, read
  0 files and made all 28, so `thumbnails_failed` and `derivatives_failed`
  are now 0.

The v5 check migrated the v4 archive and ran `derive` with `--cpus=2`
following the real-check steps of the Milestone 3b PR: kind by kind, with a
`--limit` run, a run stopped with `docker stop`, and a final run. No backup
was mounted. Every record and file count is unchanged from v4. All 7,047 pending
derivatives were done, with 0 failed and 0 pending:

| Kind | Done | Made | Used as is | Size |
| --- | --- | --- | --- | --- |
| `preview` (images) | 5,671 | 5,665 | 6 | 1.16 GB |
| `video` | 1,266 | 581 | 685 | 4.91 GB |
| `audio` (voice) | 110 | 110 | 0 | 24 MB |
| `thumb` (from 3a) | 16,764 | 16,764 | 0 | 0.20 GB |

- `derivative_bytes` is 6,295,910,761 (6.30 GB), the sum of the four.
- 685 of the 1,266 videos were already H.264 with browser-ready audio, so
  they need no copy ([decision 0007](../decisions/0007-derive-queue.md)).
- The outputs came in under the plan's estimates: previews 1.16 GB against
  1.4–2.6 GB, and video 4.91 GB against 10–20 GB, mostly because over half
  the videos needed no conversion.

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

## Render check baseline

`imessage-ap check-render` runs every message through the view the web app
draws from ([decision 0010](../decisions/0010-render-model.md)) and prints
counts, with GUIDs for problems. The GUIDs stay with the user; only counts
are recorded here.

### 4a-1 (2026-11-17)

Run on the real archive after moving `archive.db` and `derived/` to the SSD
paths ([decision 0009](../decisions/0009-store-locations.md)):

- The move was checked first. `derived/` held 23,120 files and 6,295,910,761
  bytes before and after, which equals the baseline thumb 16,764 + preview
  5,665 + video 581 + audio 110 files. The `archive.db` copy was byte
  identical (`cmp`). The appdata share is `shareUseCache="only"` on pool
  `ssd_cache` with no secondary storage.
- `stats --json` against `jan4-baseline.json` (`.archive` without `sources`)
  showed no differences, and `derive --plan` showed 0 files.
- Over plain HTTP the viewer showed 371,159 messages, 1,002 conversations,
  27,694 attachments and 1 backup.

| Measure | Count | Note |
| --- | --- | --- |
| Messages | 371,159 | = items + reactions |
| Items drawn | 355,107 | |
| Reactions | 16,052 | |
| Render errors | 0 | |
| Items in several conversations | 0 | |
| `unknown`, `problem:unknown_item_type`, `problem:unknown_associated_type` | 0, 0, 0 | |
| `recovered` | 7 | as in the baseline |
| `problem:orphan` | 26 | as in the baseline |
| `problem:reaction_target_missing` | 69 | as in decision 0003 |
| Event kinds | 999 | |
| `media:other` | 179 | |
| Conversations listed | 883 | of 1,002 chats: see 3 below |
| `problem:empty` | 43 | see 1 |
| `file:missing`, `file:no_path` (items) | 38, 2 | see 2 |
| `audio_message`, `media:voice` (items) | 113, 110 | see 2 |
| `reaction:removed` | 0 | see 4 |
| `reply` (items) | 69,091 | baseline `replies` is 73,219: see 5 |
| `problem:reply_target_missing` | 306 | see 6 |

Six of these needed explaining. Each now has its own counts in
check-render's `details` section (and in `problem:empty:*`), so the next run
answers it directly. Every new count has a case in the demo archive and a
test that the parts add up to the totals.

1. **43 empty messages.** Messages with no text, no subject, no shown
   attachment and no balloon. The next run splits them by reason:
   `problem:empty:no_body` (no text and no attributedBody in any version),
   `body_undecoded` (a body the crate could not decode), `blank_text`, and
   `only_hidden_attachments`. Messages whose text holds an attachment
   placeholder (U+FFFC) but which have no attachment record are no longer
   empty. They are drawn as an "attachment not in the backup" placeholder,
   `file:no_record`, and counted in `attachments.unrecorded_placeholders`.
2. **66 missing files against 38 items.** `file:*` labels count items, and
   only visible attachments. `details.attachments` now counts files, split
   into on items and visible, on items and hidden, and on reactions, with
   `missing_on_messages` and `no_path_on_messages` matching `stats`. One item
   can hold several missing files (the demo has one with two).
   `details.audio_messages` splits the 113 voice messages into `voice`,
   `other_media` by kind, `hidden_only`, `no_record` (a voice message iOS
   deleted without keeping it), `no_attachment` and `unsent`.
3. **883 of 1,002 chats listed.** The list holds chats with at least one
   item. `details.conversations` counts the other 119 as `no_messages` or
   `reactions_only` (a reaction delivered into another chat row for the same
   person). They stay unlisted.
4. **No reaction removals took effect.** `details.reactions` counts reaction
   rows as adds and removals, and each removal as `removals_matched`,
   `removals_without_add` or `removals_other_kind`. If iOS deletes a tapback
   when it is taken back, removal rows come out as `removals_without_add`.
5. **4,128 fewer replies than `stats`.** `stats` counts every message with
   `reply_to_guid`. Reactions inside a thread carry one too, but they are
   drawn on their target, not as replies. `details.replies` gives
   `on_items + on_reactions + self_reference = with_reply_to`.
6. **306 replies whose original is missing.** The view looks the original up
   across the whole archive, so these originals are in no imported backup.
   The likely cause is that they were deleted on the phone before Jan 4, and
   Recently Deleted keeps messages for only 30 days. `details.replies` counts
   the distinct missing originals (`target_missing_distinct`), and how many of
   them a reaction also points at (`target_missing_also_reacted_to`). The demo
   has a case. Milestone 4b draws these replies with a "message not
   available" quote, not as an error.
