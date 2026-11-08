# AGENTS.md

Rules for every coding agent working in this repo (Claude Code, Codex and
others). Full detail lives in [docs/SPEC.md](docs/SPEC.md); progress is
tracked in [docs/ROADMAP.md](docs/ROADMAP.md). If this file and the spec
disagree, the spec wins. Point it out in the PR.

## Summary

imessage-ap is a self-hosted, open-source archive for iMessage history. It
imports local iPhone backups, keeps every message and attachment permanently,
and shows them in a fast, iMessage-like web app. It runs as one Docker
container, for one user, on a home server reachable over Tailscale. No data
ever leaves that server.

**Milestone in progress: 2, Ingester** (2a done; 2b next; see the roadmap). Work only on the
current milestone unless told otherwise.

## Stack

| Layer | Choice |
| --- | --- |
| Backup reading | `imessage-database` crate (`crabapple` for encrypted backups in Milestone 7) |
| Server, ingester, CLI | Rust 2024, axum, rusqlite (`crates/imessage-ap`) |
| Database | SQLite in WAL mode, FTS5 for search |
| Media | ImageMagick with libheif (`libheif-plugin-libde265`), ffmpeg |
| Frontend | React, TypeScript (strict), Vite, TanStack Virtual (`web/`) |
| Charts | Apache ECharts |
| Packaging | Multi-stage Dockerfile, linux/amd64, `ghcr.io/<owner>/imessage-ap` |

## Layout

```
crates/imessage-ap/   server + CLI (serve, ingest, stats, healthcheck)
fixtures/generator/   make-fixture: builds the fake iPhone backup
fixtures/media/       tiny invented media the generator embeds
web/                  React app, built to web/dist
scripts/ci/           repository guards (file size, PII scan)
scripts/cloud-setup.sh  toolchain setup for cloud sessions
docker/               container entrypoint
docs/                 SPEC.md, ROADMAP.md
docs/decisions/       decision records (read before changing what they cover)
docs/reference/       real iOS schemas (no data); the fixture must match them
```

## Commands

```bash
# Rust (toolchain pinned in rust-toolchain.toml)
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all

# Fake backup for tests and manual runs -> fixtures/out/backup-v1 (gitignored)
cargo run -p fixture-gen -- --out fixtures/out/backup-v1 --force

# Server against the built web app
cargo run -p imessage-ap -- serve --port 8080 --data ./data --web web/dist

# Import the fixture and print counts
cargo run -p imessage-ap -- ingest --backup fixtures/out/backup-v1 --data ./data
cargo run -p imessage-ap -- stats --data ./data

# Web
cd web && npm ci && npm run typecheck && npm run build   # npm run dev proxies /api to :8080

# Guards (CI runs these; run them before pushing)
scripts/ci/check-file-size.sh
scripts/ci/check-pii.sh

# Image
docker build -t imessage-ap .
```

A change is done when fmt, clippy, the tests, the web build and both guards
pass.

## Privacy rules (non-negotiable)

- **No real data, ever.** No real messages, contacts, phone numbers, emails,
  attachments, backups or databases in the repo, its history, tests, logs,
  issues or PRs. Tests use only the generated fixture backup.
- Fixtures use invented people, fictional `555-0100`..`555-0199` numbers and
  `example.com`/`.org`/`.net` emails, and live only under `fixtures/`.
  CI fails on phone or email patterns anywhere else, and on any file over 5 MB.
- When the real backup misbehaves, the user shares **counts, error messages
  and GUIDs only**. Never ask for message text, names or numbers.
- **No outbound network requests at runtime**: no CDNs, web fonts, analytics,
  telemetry, update checks or link-preview fetching. Every response carries
  `Content-Security-Policy: default-src 'self'`; never loosen it to allow
  another origin. Bundle every asset.
- Logs never contain message text, names or addresses: log GUIDs and counts.
- Backup passwords are used for one import run only and never written to disk.
- Never write anything into a backup folder; `/backups` is mounted read-only.

## Non-goals for v1 (refuse to add these)

- Sending, replying to or editing messages.
- Live sync with a phone, Mac or iCloud; data arrives only by importing backups.
- Multiple users, sharing or public access.
- Any cloud service or outbound request at runtime, including link previews
  and telemetry.
- LLMs of any kind.
- Importing CopyTrans PDFs or other exports.
- iCloud backups; only local Apple Devices / iTunes backups.

## Data rules

- Message identity is Apple's GUID; attachment file identity is its sha256.
- The archive is append-only. A message missing from a newer backup stays,
  and a changed message keeps every version. Imports are idempotent.
- `archive.db`, blobs and derived files are rebuildable from backups.
  `user.db` (people, links, merges, settings) is not: treat changes to its
  schema with extra care and provide migrations.
- Times are stored in UTC and displayed in the user's timezone setting.

## Decisions

Lasting technical decisions are recorded in `docs/decisions/NNNN-title.md`
(context, decision, consequences). Read the relevant ones before changing
what they cover; to reverse one, add a new record that supersedes it.

- [0001](docs/decisions/0001-runtime-image.md): Ubuntu 24.04 runtime image,
  `libheif-plugin-libde265`, and the build-time HEIC decode check.
- [0002](docs/decisions/0002-archive-identity.md): archive.db identity keys,
  append-only imports, and strictly read-only backup access.

## Dependencies

- Pin exact versions: `=x.y.z` in `[workspace.dependencies]` and exact versions
  in `web/package.json`. Commit `Cargo.lock` and `package-lock.json`.
- Add crates to the workspace table, not to a member crate directly.
- Upgrades are deliberate, in their own PR. After upgrading the iMessage
  crates, run the fixture tests and ask the user for a count check against
  the real backup.

## Branches and pull requests

- Never commit to `main`. Work on a feature branch (cloud agents use the
  `claude/...` or `codex/...` branch they were given), then open a PR to `main`.
- One milestone or one coherent change per PR. Keep PRs reviewable.
- The PR description lists what changed, how it was verified, and any
  deviation from the spec or roadmap.
- CI (`CI` and `Docker image` workflows) must be green before merge. Never
  skip, disable or weaken a test or guard to get green.
- Tick roadmap items in `docs/ROADMAP.md` in the PR that completes them, and
  update "Milestone in progress" above when a milestone closes.
- Commit messages: imperative subject under 72 characters, body explaining why.
