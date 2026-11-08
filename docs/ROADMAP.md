# Roadmap

Milestones from [SPEC.md](SPEC.md). Each one ends with something runnable.
Checks against the real backup use counts only. Tick items in the PR that
completes them.

## Next up

1. **2b: versions and multiple backups** (below).
2. **Milestone 3: Attachments**: copy files into `/data/blobs`, thumbnails, HEIC and voice-message conversion.

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

- [ ] Fixture: `message_summary_info` edit history and link-preview payloads
- [ ] Fixture: a second, later backup (new and changed messages, one dropped message, renamed contact)
- [ ] Changed messages keep every version; messages missing from a newer backup stay
- [ ] Tests: two backups imported in either order give the same archive

## 3. Attachments

- [ ] Content-addressed copy into `/data/blobs/ab/cd/<sha256>`
- [ ] Thumbnails at import; previews and conversions in a background queue
- [ ] HEIC to web formats; voice messages (CAF) and video conversion
- [ ] Missing-file and no-path tracking
- [ ] Check: 25,040 attachment records have files; 2,654 are flagged missing

## 4. Viewer

- [ ] Login: one Argon2-hashed password set on first run; session cookie
- [ ] Conversation list, recent and alphabetical, with search at the top
- [ ] Conversation view: bubbles, grouping, time separators, virtualized 10,000+ messages
- [ ] Reactions, replies, edits, unsent and recovered placeholders, system events
- [ ] Inline images, video and audio; full-screen viewer; media gallery by month
- [ ] Light and dark mode with manual toggle; phone layout
- [ ] Display timezone setting (default America/Los_Angeles)

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
