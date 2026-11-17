# 0009. Where each store lives

- Status: accepted
- Date: 2026-11-16
- Milestone: 4a-1, Viewer

## Context

On snoopyserver, `/data` is an Unraid user share on the HDD array. Every
read goes through Unraid's FUSE layer (shfs), and the mover moves files from
the SSD cache to the array. Until now archive.db, user.db, blobs and derived
files all lived there.

- SQLite in WAL mode keeps a memory-mapped `-shm` file and uses file locks.
  shfs is a known weak spot for both, which is why apps with SQLite
  databases are usually told to keep them off `/mnt/user`.
- The mover can move a database to the array while nothing has it open.
  Reads then go to a disk that may be spun down (seconds for the first
  request) and is slow at random reads. The viewer would feel it, and
  Milestone 5's search target (p95 under 200 ms) would fail on a cold cache.
- Thumbnails are read many at a time while scrolling through photos, with
  the same problem on a spun-down disk.
- The original files (about 43 GB) are large, written once, read in
  sequence and rarely; the array suits them.

## Decision

**Every store is its own setting**, defaulting to a folder under the data
directory, so a plain `--data /data` works as before:

| Store | Default | Setting |
| --- | --- | --- |
| archive.db, user.db, the run lock | `<data>` | `--db-dir`, `IMESSAGE_AP_DB_DIR` |
| original files | `<data>/blobs` | `--blobs-dir`, `IMESSAGE_AP_BLOBS_DIR` |
| thumbnails, previews, conversions | `<data>/derived` | `--derived-dir`, `IMESSAGE_AP_DERIVED_DIR` |
| exports (Milestone 9) | `<data>/exports` | `--exports-dir`, `IMESSAGE_AP_EXPORTS_DIR` |

Every command (`serve`, `ingest`, `attachments`, `derive`, `stats`,
`migrate`, `check-render`, `set-password`) honours them.

- `derivative.path` stays relative to the data directory
  (`derived/thumb/ab/cd/<sha256>.webp`). The app resolves it under the
  derived directory wherever that is, so moving `derived/` changes nothing in
  archive.db and the count-check baseline is unaffected.
- Temporary files live inside each store (`<blobs>/.tmp`, `<derived>/.tmp`),
  so a rename into place stays on one file system.
- The lock that keeps two file runs apart moves from the data directory to
  the database directory, beside archive.db.
- **Two archives are refused.** When the database directory is not the data
  directory and the data directory still holds an `archive.db` or `user.db`
  (left by a copy instead of a move), every command stops with an error
  naming both. Using either would silently ignore the other.
- The container's entrypoint creates each configured folder and gives its
  top level to PUID:PGID, as it does for `/data`.

**Recommended layout on snoopyserver**: databases and derived files on an
SSD pool path that bypasses FUSE, originals on the array.

| Store | Host path | Container path |
| --- | --- | --- |
| data, blobs | `/mnt/user/main/vault/sh-data/imessage-archive-project/data` | `/data` |
| archive.db, user.db | `/mnt/ssd_cache/appdata/imessage-ap` | `/db` |
| derived | `/mnt/ssd_cache/appdata/imessage-ap/derived` | `/db/derived` (`IMESSAGE_AP_DERIVED_DIR`) |

The `ssd_cache` share has no secondary storage, so the mover never touches
it. The derived files (about 6.3 GB) and databases fit on the SSD easily.

## Consequences

- Viewer and search reads of the databases and thumbnails come from the SSD
  through no FUSE layer. Opening an original (a full photo download, a
  video that needed no conversion) may still wait for an array disk to spin
  up.
- user.db sits in appdata, where Unraid's appdata backup already looks. It
  is the one file that cannot be rebuilt.
- Moving an existing archive is a one-time manual step, documented in the
  Milestone 4a-1 PR: stop the container, move the files, add the settings.
- Changing a store's layout, or recording paths relative to anything but the
  data directory, needs a new decision record.
