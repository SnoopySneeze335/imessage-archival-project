# Roadmap

Milestones from [SPEC.md](SPEC.md). Each one ends with something runnable.
Checks against the real backup use counts only. Tick items in the PR that
completes them.

## Next up

1. **Milestone 4: Viewer**: 4a-1 in review; then 4a-2, 4b, 4c, 4d.
2. **Milestone 5: Search**.

## 1. Foundation

- [x] Repository, GPL-3.0 license, README
- [x] AGENTS.md (shared agent rules) and CLAUDE.md
- [x] `.gitignore` for data, backups, databases, PDFs, exports and local config
- [x] CI guard: fail on any file over 5 MB
- [x] CI guard: fail on phone-number or email patterns outside `fixtures/`
- [x] Cargo workspace with `imessage-ap` (serve, ingest stub, healthcheck)
- [x] Vite + React + TypeScript skeleton served by the Rust server, strict CSP
- [x] Multi-stage Dockerfile (ImageMagick with HEIC decoding, ffmpeg), PUID/PGID
- [x] GitHub Action that builds, smoke-tests and pushes the image to GHCR
- [x] Fixture generator: fake backup with invented people and 555-01xx numbers
- [x] `scripts/cloud-setup.sh` for cloud coding sessions

## 2. Ingester

Split into two PRs.

**2a: fixture parity and core ingest**

- [x] Fixture generator builds sms.db and AddressBook from the real iOS 26.1 schemas (`docs/reference/`), verified exactly
- [x] Fixture: `attributedBody` typedstream text, including messages with text only there
- [x] Add `imessage-database`, pinned; confirm it reads the fixture backup
- [x] archive.db schema: source, handle, chat, message, reaction, attachment, import_run, import_issue
- [x] `imessage-ap ingest --backup <dir> --data <dir>`: strictly read-only, upserts by GUID
- [x] Import runs and issues recorded; running twice changes nothing; a failed run rolls back
- [x] `imessage-ap stats --data <dir>`: counts only, beside imessage-database's own diagnostics
- [x] Fixture tests: first import, re-import, reactions, orphans, missing files, read-only backup, encrypted refused
- [x] Check: Jan 4 backup yields 371,159 messages, 1,002 raw chats, 27,694 attachment records: matched on 2026-11-08; baseline in [`docs/reference/jan4-baseline.json`](reference/jan4-baseline.json) ([notes](reference/README.md#count-check-baseline))

**2b: versions and multiple backups**

- [x] Fixture: `message_summary_info` edit history and link-preview payloads
- [x] Fixture: a second, later backup (new and changed messages, one dropped message, renamed contact)
- [x] Changed messages keep every version; messages missing from a newer backup stay ([decision 0004](decisions/0004-versions-and-multiple-backups.md))
- [x] Tests: two backups imported in either order give the same archive

## 3. Attachments

Split into two PRs ([decision 0005](decisions/0005-blobs-and-derivatives.md)).

**3a: blobs and thumbnails**

- [x] Content-addressed copy into `/data/blobs/ab/cd/<sha256>`: `imessage-ap attachments`, read-only, resumable, crash-safe
- [x] Thumbnails at import; previews and conversions queued for the background queue
- [x] Missing-file and no-path tracking; unreadable files reported and retried
- [x] Stats: unique blobs, bytes, duplicates skipped, thumbnails made, derivatives pending
- [x] Fixture: a duplicate file, a damaged JPEG and an HEVC video
- [x] Check: 25,040 attachment records have files and are copied; 2,654 are flagged missing
- [x] Check: the 28 thumbnails that failed (27 HEIF sequences, 1 MP4 with `chnl` v1) are made on the next run ([decision 0006](decisions/0006-decoder-workarounds.md))

**3b: background queue** ([decision 0007](decisions/0007-derive-queue.md))

- [x] `imessage-ap derive`: the queue, resumable, CPU-limited; reads files through `media::input` and retries failures after a decoder fix ([decision 0006](decisions/0006-decoder-workarounds.md))
- [x] HEIC to web formats; voice messages (CAF) and video conversion: only what browsers cannot use, remuxed when only the container is wrong
- [x] `--kind`, `--limit`, `--min-free` and `--plan`; stats per derivative kind and total derivative bytes
- [x] Fixture: an AAC voice message in CAF, an HDR (HLG) HEVC video, a browser-ready MP4, a PNG labelled HEIC, and a damaged file of each converted kind
- [x] Check: the 7,047 pending derivatives (5,671 image, 1,266 video, 110 voice) are done or failed with 0 pending, a run stopped with `docker stop` resumes, and a final run does 0 work: all 7,047 done, 0 failed, 6.30 GB of derived files, record counts unchanged (2026-11-16; [baseline](reference/jan4-baseline.json), [notes](reference/README.md#count-check-baseline))

## 4. Viewer

Split into five PRs, each ending with something to open and use. Reviews use
the demo archive of invented data; real-data checks are counts only
(`check-render`, `stats`).

**4a-1: sign-in, store locations, render model, demo**

- [x] Every store its own setting: `IMESSAGE_AP_DB_DIR`, `_BLOBS_DIR`, `_DERIVED_DIR`, `_EXPORTS_DIR`; two archives refused ([decision 0009](decisions/0009-store-locations.md))
- [x] user.db with migrations: password, sessions, settings
- [x] Login: one Argon2-hashed password set on first run with a one-time setup code from the log; session cookie that follows the scheme; plain HTTP and HTTPS behind a proxy both work ([decision 0008](decisions/0008-access-and-sign-in.md))
- [x] `set-password`, rate-limited sign-in, Origin check against the request's own Host, request logs without query strings
- [x] View module and `check-render` v1: every message to one kind, with reactions, attachments and flags; counts and GUIDs only ([decision 0010](decisions/0010-render-model.md))
- [x] Demo archive: "All message kinds" conversations (1:1 and group) with a labelled example of every kind, a 25,000-message conversation, `docker run … demo`, `scripts/demo.sh`; a test fails on any kind without an example
- [x] Playwright in CI (Chromium, WebKit, iPhone): first-run setup, sign-in, no request to any other host
- [ ] Check: the databases and derived files moved to the SSD paths, `stats --json` matches the baseline, `derive --plan` shows 0 pending
- [ ] Check: `check-render` on the real archive: 0 render errors; counts per kind and problem sent back

**4a-2: conversation list and view**

- [ ] Conversation list, recent and alphabetical, with search at the top (filters by name or number)
- [ ] Conversation view: bubbles, grouping, time separators, virtualized 10,000+ messages
- [ ] Light and dark mode with manual toggle; phone layout

**4b: every message kind**

- [ ] Reactions, replies, edits, unsent and recovered placeholders, system events
- [ ] Link previews, app balloons and effects from stored data only
- [ ] `view::PENDING` 4b entries built, with demo examples

**4c: attachments**

- [ ] Inline images, video and audio (HTTP range requests); full-screen viewer; media gallery by month
- [ ] Missing-file placeholders; `view::PENDING` empty

**4d: names and settings**

- [ ] Names from the backup's AddressBook, snapshotted at ingest
- [ ] Display timezone setting (default America/Los_Angeles); theme; change password
- [ ] Browser-side render check over every message

## 5. Search

- [ ] Text mode (FTS5 unicode61), exact mode (FTS5 trigram)
- [ ] Similar mode (fts5vocab + Levenshtein expansion)
- [ ] Filters: person, chat, date range, direction, attachment presence and type
- [ ] Attachment filenames and link-preview titles searchable
- [ ] Jump to a result in context, highlighted
- [ ] Search behind one internal interface (Tantivy-replaceable)
- [ ] Check: p95 under 200 ms over the full archive on snoopyserver

## 6. Contacts

- [ ] People and handles in user.db; E.164 and lowercase-email normalization
- [ ] AddressBook snapshots per backup; user names beat snapshots; name history
- [ ] Unknown-number triage sorted by message count
- [ ] Businesses and short codes
- [ ] Merge and split people; date-ranged handle links
- [ ] vCard import
- [ ] Chat merging (the 31 duplicated chats proposed; definition in [decision 0003](decisions/0003-count-check-differences.md))

## 7. Import UI

- [ ] Backup discovery under `/backups` with device, iOS version, date, encryption state
- [ ] Encrypted backups: password prompt, used for one run only
- [ ] Background imports with live progress, resumable after restart
- [ ] Review screen: missing files, unknown handles, orphan messages, merges, name conflicts, decode failures
- [ ] Reprocess all

## 8. Statistics

- [ ] Stat module framework: query, cached result table, chart definition
- [ ] Top conversations, messages per month, weekday/hour heatmap
- [ ] Sent vs received, streaks, median response time
- [ ] Emoji and reaction leaderboards, first and last messages
- [ ] Attachment counts and storage, records

## 9. Release

- [ ] JSON export: per-conversation files, people, handles, manifest, versioned schema
- [ ] User-data export written after every contacts or merge change
- [ ] Rebuild from backups plus user data reproduces the archive
- [ ] PWA install
- [ ] README: setup on Unraid, Tailscale, backups; not affiliated with Apple
- [ ] Published image; license decision (GPL-3.0 or AGPL-3.0)

## v1 is done when

- [x] Importing the Jan 4 backup a second time changes nothing: run 2 added 0 records (2026-11-08)
- [ ] Raw message, chat and attachment counts match the diagnostics
- [ ] Every message renders without errors, and missing files show placeholders
- [ ] Search p95 is under 200 ms on snoopyserver
- [ ] The browser's network panel shows zero requests to any host but the app
- [ ] A rebuild from backups plus user data reproduces the same archive
- [ ] No real data exists anywhere in the repository history

## v2

- [ ] Per-conversation stats pages
- [ ] Static HTML export
- [ ] Missing-file recovery from older backups
- [ ] GPU (NVENC) transcoding
- [ ] Unraid Community Apps template
- [ ] More stats modules
