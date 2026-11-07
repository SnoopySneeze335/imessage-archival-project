# imessage-ap — Project Spec

Nov 6, 2026 · @mason

## Summary

imessage-ap is a self-hosted, open-source archive for iMessage history. It imports local iPhone backups, keeps every message and attachment permanently, and shows them in a fast, iMessage-like web app. The first archive is the Jan 4, 2026 backup (371,159 messages, 1,002 chats, 27,694 attachments); later backups add to it.

v1 commits to:

- Lossless import of the Jan 4 backup: every message, reaction, reply, edit and attachment it holds.
- Repeatable imports of future backups, deduplicated by Apple's message ID; nothing is deleted because a newer backup lacks it.
- An iMessage-like viewer with search under 200 ms across the whole archive.
- Contact management that turns 1,098 raw phone numbers and emails into named people.
- A statistics dashboard, plus a JSON export so the archive outlives the app.
- One Docker container on Unraid, single user, reachable over Tailscale, with no data leaving the server.

## Source data

The source of truth is local iPhone backups, read directly. The CopyTrans PDF exports are not used.

| Item | Detail |
| --- | --- |
| Primary backup | `F:\iPhone Backups\1.4.26\00008110-0014110E0104801E`, made Jan 4, 2026, believed unencrypted |
| Why it matters | Messages before Jan 4, 2026 were deleted from the phone; this backup is the only complete copy |
| Future backups | New dated local backups of the current iPhone; messages are no longer wiped from the phone |
| Older backups | Still on the PC; not needed for history, but may hold some of the missing attachment files |
| Existing exports | imessage-exporter HTML export (`imessage-extractor-full`) and CopyTrans PDFs; kept as a fallback, never imported |

Diagnostics for the Jan 4 backup, from the imessage-exporter legacy build:

| Measure | Value |
| --- | --- |
| Messages | 371,159 (33 not linked to a chat) |
| Chats | 1,002 (23 duplicated) |
| Handles (phone numbers and emails) | 1,432 rows; 1,098 unique after removing 334 duplicates; 87 with more than one ID |
| Handles with a name in the backup's address book | 279 of 1,098 (25%) |
| Attachment records | 27,694 |
| Attachment data | 67.54 GB referenced, 45.72 GB present on disk |
| Missing attachment files | 2,654 (10%): 2,652 not found, 2 with no path |
| Messages database size | 435 MB |

What these numbers decide:

- Search volume is small: about 371K messages fits comfortably in SQLite with full-text indexes.
- /data needs about 46 GB for original attachments plus derivatives; plan for 60 GB.
- Three in four handles have no name, so unknown-number triage is a core feature.
- Duplicate handles and chats must merge into one conversation per person or group.
- Missing files are recorded and shown as placeholders; importing another backup that has them fills the gap.

## Non-goals

v1 deliberately excludes these, and coding sessions should refuse to add them:

- Sending, replying to, or editing messages.
- Live sync with a phone, Mac or iCloud; data arrives only by importing backups.
- Multiple users, sharing, or public access.
- Any cloud service or outbound network request at runtime, including link-preview fetching and telemetry.
- LLMs of any kind.
- Importing CopyTrans PDFs or other exports, while every conversation exists in a backup.
- Writing anything into a backup folder.
- iCloud backups; only local backups made with the Apple Devices app or iTunes.

## Architecture and stack

One Rust service handles importing, storage and the API; a TypeScript single-page app is the UI; both ship in one Docker image.

| Layer | Choice | Why |
| --- | --- | --- |
| Backup reading | `imessage-database` and `crabapple` crates by ReagentX | Already parse every iMessage feature and encrypted backups, and track iOS database changes |
| Ingester | Rust module plus CLI (`imessage-ap ingest <backup>`) | Same crates, same binary; runs without the web UI |
| Server | Rust with axum; SQLite via rusqlite | One language for server and ingester; one static binary |
| Database | SQLite in WAL mode, FTS5 for search | \~371K messages from a 435 MB source; no database server needed |
| Media tools | ImageMagick with libheif, ffmpeg | HEIC, voice-message audio and video conversion; the same tools imessage-exporter detected |
| Frontend | React, TypeScript, Vite, TanStack Virtual | Most familiar stack for coding assistants; virtualized lists for 10,000+ message threads |
| Charts | Apache ECharts | Heatmaps, timelines and bars for the dashboard |
| Packaging | Multi-stage Docker build, amd64 | Unraid runs on x86; arm64 can come later |

Crate versions are pinned. The diagnostics came from an unknown legacy build, so the ingester milestone first confirms the current crates read the Jan 4 backup with the same counts.

&#91;embedded content: architecture · 3 processes, 5 stores in /data\]

The ingester and media worker only add to /data; user.db is the one store the backups cannot regenerate, so it is the file to back up.

## Data model and storage

The app keeps two kinds of data apart: derived data that can be rebuilt from backups, and user-authored data that exists only in the app.

| Path | Contents | Rebuildable |
| --- | --- | --- |
| `/backups` (read-only mount) | iPhone backup folders | No: these are the originals, backed up outside the app |
| `/data/blobs/ab/cd/<sha256>` | Original attachment files, each stored once by content hash | Yes, from backups |
| `/data/derived/` | Thumbnails, web previews, converted audio and video | Yes, from blobs |
| `/data/archive.db` | Messages, chats, handles, attachments, search indexes, cached stats | Yes, from backups |
| `/data/user.db` | People, handle links, name overrides, chat merges, settings, password hash | No: must be backed up |
| `/data/exports/` | JSON exports | Yes |

Core records in archive.db:

- **source**: one per imported backup, with folder, device name, iOS version, backup date, and the ingester and crate versions that read it.
- **handle**: a raw address (phone number in E.164 or lowercase email) plus its service (iMessage, SMS, RCS).
- **chat**: a conversation from the backup, with its GUID, group-name history and participants over time.
- **message**: keyed by Apple's message GUID. Holds chat, sender handle or "me", service, sent, delivered, read and edited times in UTC, text, formatting runs, reply-to GUID, item type, edit history, unsent and recovered-deleted flags, and every source that held it.
- **reaction**: stored as a message and linked to its target GUID and part; classic tapbacks and emoji reactions alike.
- **attachment**: GUID, message GUID, original filename, MIME type, size, sha256, missing flag, source.
- **import\_run** and **import\_issue**: what each import added and every problem it found.

Rules:

- Message identity is the GUID; attachment file identity is the sha256.
- The archive is append-only: a message missing from a newer backup stays, and a changed message keeps every version.
- Every message records which backups held it.

## Ingestion and import

An import reads one backup folder from /backups, adds only what is new, and reports problems; running it twice changes nothing.

1. **Discover**: the Import page lists backup folders under /backups with device name, iOS version, date and encryption state, read from the backup's own plist files.
2. **Unlock**: an encrypted backup asks for its password in the UI; the password is used for that run only and never stored.
3. **Read**: open the Messages database and the AddressBook database inside the backup.
4. **Upsert**: add messages, chats, handles and attachments by GUID; record changed messages as new versions.
5. **Copy files**: hash each attachment file and copy it into /data/blobs unless that hash already exists.
6. **Derive**: make thumbnails right away; previews and audio/video conversions run in a background queue.
7. **Index**: update search indexes and recompute stats for the chats that changed.
8. **Report**: save the run's counts and issues, then open the review screen.

Imports run as background jobs with live progress and resume after a restart. The CLI runs the same steps for scripting and testing.

The review screen flags problems without blocking the import:

- Missing attachment files, grouped by chat (2,654 expected for the Jan 4 backup).
- Unknown handles, sorted by message count, linked to contact triage.
- Messages not linked to a chat (33 expected).
- Proposed merges for duplicated chats (23 expected).
- Name conflicts between address-book snapshots.
- Anything the parser could not decode, with its GUID and source.

A "Reprocess all" action re-runs every recorded import with the current ingester, for when a newer version parses more.

## Contacts and identity

People and handles are separate: one person owns many handles, and a handle can belong to different people over time.

- Handles are normalized: phone numbers to E.164 with US as the default region, emails to lowercase. iMessage and SMS rows for the same number merge into one address.
- Each backup's AddressBook gives a dated snapshot of names. A person shows the newest snapshot's name unless the user set one.
- A name set by the user always beats a snapshot. Name history stays viewable on the person's page.
- Raw numbers are always kept and shown in details and on hover.
- A handle-to-person link can carry a date range, for numbers that were reassigned.

Contact management screens:

- **Unknown numbers**: unresolved handles sorted by message count, with their chats and a few message previews, assignable to a new or existing person in one step.
- **Businesses and short codes**: mark a handle as one so it leaves the unknown list.
- **People**: merge and split people; resolve conflicts when two snapshots disagree.
- **vCard import**: .vcf files as an extra source of names.
- **Chat merging**: the 23 duplicated chats are proposed for merging; the user can merge or split any conversation.

## Search

Search covers every message and answers within 200 ms at the 95th percentile on snoopyserver (i5-9400).

| Mode | Matches | Index |
| --- | --- | --- |
| Text | Words in any order and word prefixes, ignoring case and accents | FTS5, unicode61 tokenizer |
| Exact | The exact character sequence, punctuation included | FTS5, trigram tokenizer |
| Similar | Words within 1–2 edits of the query, so a typo in the query or in the original message still matches | Query words expanded against the index vocabulary (fts5vocab plus a Levenshtein automaton), then searched as Text |

- Filters: person, chat, date range, sent or received, has attachment, attachment type.
- Attachment filenames and stored link-preview titles are searchable too.
- Results show the message, sender, chat and date with the match highlighted, newest first by default.
- Clicking a result opens the chat at that message, loads the messages around it, and highlights it.
- Search sits behind one internal interface, so Tantivy can replace the SQLite indexes if Similar misses the speed or quality target.

## UI

The web app follows the Messages app's layout and conventions, built with its own code, an open-source icon set (e.g., Lucide) and the system font stack; SF Pro is not bundled.

Layout:

- Desktop: a conversation sidebar on the left and the open conversation on the right, as in Messages on macOS.
- Phone: a conversation list that slides into a conversation, as in Messages on iOS; installable to the home screen as a PWA.
- The sidebar sorts by most recent message or alphabetically, with search at its top.
- Light and dark mode follow the system, with a manual toggle.
- Times display in America/Los\_Angeles (a setting); everything is stored in UTC.

Conversation view:

- Bubbles are blue for your iMessages, green for your SMS, gray for others. Consecutive messages from one sender group together, with a tail on the last.
- Group chats show the sender's name above each run and an avatar beside its last bubble.
- A time separator appears after any gap of an hour or more; hovering or swiping a bubble shows its exact time.
- Reactions sit as badges on a bubble's corner; tapping one lists who reacted.
- Replies show the quoted message and open the thread.
- Edited messages show "Edited" with earlier versions viewable. Unsent and recovered deleted messages show as labeled placeholders.
- System events (group renames, people added or removed) appear as centered gray lines.
- Long conversations load in windows and scroll smoothly through 10,000+ messages.

Attachments:

- Images show inline and open in a full-screen viewer; videos and voice messages play inline.
- Stickers, link previews and other rich messages render from stored data only; nothing is fetched from the internet.
- A missing file shows a placeholder with its filename and type.
- Each conversation has a media gallery grouped by month.

Other screens: contacts, import and review, statistics, and settings (display timezone, theme, password). The README states the project is not affiliated with Apple.

## Statistics dashboard

v1 ships one archive-wide dashboard, refreshed after each import; more stats are added later as separate modules.

v1 stats:

- Top conversations by message count, all time and per year.
- Messages per month across the whole archive, sent and received.
- Activity heatmap by weekday and hour, in the display timezone.
- Sent vs. received ratio, overall and per person.
- Longest daily streaks per conversation.
- Median response time, yours and theirs, per person.
- Emoji leaderboard, and reactions given and received by type.
- First and last message with each person.
- Attachment counts and storage by conversation and type.
- Records: busiest day, longest message, most active group chat.

Each stat is a module: a query, a cached result table, and a chart definition. Adding a stat touches no existing one. Per-conversation stats pages come in v2.

## Export, backup and rebuild

The archive can always be rebuilt from the backups plus the user-authored data, and exported to open formats.

- **JSON export (v1)**: one file per conversation plus people, handles and a manifest, in a documented, versioned schema; attachments referenced by their sha256 path.
- **User-data export (v1)**: user.db as JSON, written automatically after every change to contacts or merges, and on demand.
- **Static HTML export (v2)**: a self-contained copy of the archive that browses offline. Until then, the existing imessage-exporter HTML export is the fallback.
- **Rebuild**: delete archive.db, blobs and derived files, re-import every recorded source, then re-apply user data. The result must match the previous state.

Protecting the sources:

- Copy the Jan 4 backup to snoopyserver and include it in the existing offsite backup job.
- Never run a new phone backup into the Jan 4 folder: Apple updates the folder with the same device ID in place, which would replace the pre-Jan 4 history.
- Back up /data/user.db and /data/blobs; derived files and archive.db can be regenerated.
- Keep the older backups until v2 can check them for the 2,654 missing attachment files.

## Deployment, access and privacy

The app runs as one container on snoopyserver, for one user, reachable only over the LAN and Tailscale.

| Setting | Value |
| --- | --- |
| Image | `ghcr.io/<user>/imessage-ap`, built by GitHub Actions from the public repo |
| Volumes | `/backups` read-only, `/data` read-write |
| Port | 8080, configurable |
| File ownership | PUID/PGID, default 99/100 for Unraid |
| Auth | One password set on first run, hashed with Argon2; session cookie |
| Remote access | Tailscale (`tailscale serve` or a sidecar); never exposed publicly |

Privacy rules:

- No outbound network requests at runtime: no CDNs, web fonts, analytics, update checks or link fetching. A strict Content-Security-Policy (`default-src 'self'`) enforces it.
- All processing (HEIC conversion, transcoding, indexing) runs inside the container.
- Logs never contain message text.
- Backup passwords are never written to disk.

Later: an Unraid Community Apps template, and optional GPU (NVENC) video transcoding on snoopyserver's GTX 1650 Super.

## Repository, license and development rules

The code is public on GitHub under GPL-3.0; real message data never touches the repo or the coding assistant.

- **License**: GPL-3.0, which the imessage-exporter crates require. Anyone can download and fork; anyone who distributes a modified version must provide its source under the same license. AGPL-3.0 would also cover someone who hosts a modified version for others without distributing it. It is compatible with the crates; decide before the first release.
- **.gitignore** covers `/data`, `/backups`, `*.db`, `*.pdf`, exports and local config.
- **Fixtures only**: tests run on a generated fake backup (Manifest, Info.plist, Messages and AddressBook databases, small attachments) with invented people and fictional 555-01xx numbers.
- **CI guards**: fail on any file over 5 MB, and on phone-number or email patterns outside the fixtures folder.
- **Coding assistant**: Claude Code works against fixtures. When the real backup misbehaves, share counts, error messages and GUIDs, never message text.
- **CLAUDE.md** in the repo restates these rules, the non-goals and the milestone in progress.
- **Dependencies**: crate versions are pinned; upgrades are deliberate, followed by fixture tests and a count check against the real backup.

## Milestones

Each milestone ends with something runnable. Checks against the real backup use counts only.

1. **Foundation**: repo, license, CI guards, Docker skeleton, fixture-backup generator, CLAUDE.md.
2. **Ingester**: the CLI imports a backup into archive.db, repeatably, with an import report. Check: the Jan 4 backup yields 371,159 messages, 1,002 raw chats and 27,694 attachment records.
3. **Attachments**: content-addressed copying, thumbnails, HEIC and voice-message conversion, missing-file tracking. Check: 25,040 attachment records have files; 2,654 are flagged missing.
4. **Viewer**: login, conversation list (recent and alphabetical), conversation view with bubbles, reactions, replies, edits, attachments and dark mode.
5. **Search**: three modes, filters, jump to context. Check: p95 under 200 ms over the full archive.
6. **Contacts**: people and handles, address-book snapshots, unknown-number triage, vCard import, chat merging.
7. **Import UI**: backup discovery, background imports with progress, review screen.
8. **Statistics**: the v1 dashboard.
9. **Release**: JSON and user-data exports, PWA install, README, published image.

v2: per-conversation stats, static HTML export, missing-file recovery from older backups, GPU transcoding, Unraid template, more stats.

v1 is done when:

- [ ] Importing the Jan 4 backup a second time changes nothing.
- [ ] Raw message, chat and attachment counts match the diagnostics above.
- [ ] Every message renders without errors, and missing files show placeholders.
- [ ] Search p95 is under 200 ms on snoopyserver.
- [ ] The browser's network panel shows zero requests to any host but the app.
- [ ] A rebuild from backups plus user data reproduces the same archive.
- [ ] No real data exists anywhere in the repository history.

## Risks and open items

The largest risk is losing the Jan 4 backup before it has a second copy; everything else has a fallback.

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Only one copy of the pre-2026 history exists | Permanent loss | Copy to snoopyserver and offsite before development starts |
| Diagnostics came from an unknown legacy build | Current crates might read the backup differently | Milestone 2 compares counts with the current release first |
| Future iOS versions change the Messages database | New backups fail to import or lose fields | Pinned crates; each import records its versions; Reprocess all after upgrades |
| 2,654 attachment files (10%) are missing | Gaps in photos and media | Placeholders now; check older backups in v2 |
| 75% of handles have no name | Long manual triage | Triage sorted by message count, so the most active numbers get named first |
| Encryption is assumed off | Import fails if the assumption is wrong | Encrypted backups supported with a password prompt |

Open items:

- Confirm the Jan 4 backup's encryption state (the Import page will show it).
- Choose GPL-3.0 or AGPL-3.0 before the first public release.
- Decide whether older backups get imported in v1 to fill missing files.

Sources: [imessage-exporter](https://github.com/ReagentX/imessage-exporter) · [crabapple](https://docs.rs/crabapple) · [imessage-exporter releases](https://github.com/ReagentX/imessage-exporter/releases)
