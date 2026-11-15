# 0005. Blob store, resumable copies and the derivative queue

- Status: accepted
- Date: 2026-11-15
- Milestone: 3a, Attachments

## Context

The Jan 4, 2026 backup holds 25,040 attachment records with files, about
49 GB. The spec stores each original once by content hash in
`/data/blobs/ab/cd/<sha256>`, makes thumbnails at import, and leaves
previews and audio/video conversion to a background queue. On snoopyserver,
`/data` is a folder on the Unraid array (a FUSE user share). A copy that
size takes long enough to be interrupted (a reboot, `docker stop`, Ctrl-C),
and starting over each time is not acceptable. The backup must still never
be written to (decision 0002).

## Decision

**A separate step.** `imessage-ap ingest` imports records only, in one
transaction, as before. `imessage-ap attachments --backup <dir>` then copies
that backup's files and makes thumbnails. It refuses a backup that has not
been ingested. Milestone 7's background import runs both in order.

**Content addressing.** A blob's name is the SHA-256 of its bytes. Each
backup's copy is recorded in `attachment_source.sha256`. `attachment.sha256`
is the hash from the newest backup with a copied file, which keeps decision
0004's import-order independence. `blob` has one row per stored file. Its
`media_kind` comes from the first attachment that brought it, using MIME
type, then UTI, then file extension.

**Copying, crash-safe and resumable.**

1. The work list is this backup's `attachment_source` rows with
   `present = 1` and no `sha256`. A finished backup has an empty list, so
   running again reads no files.
2. Each file is opened with `File::open` (read-only) and streamed into
   `blobs/.tmp/` while it is hashed. The temporary file is `fsync`ed, then
   renamed to its final path, unless a blob of that hash and size is already
   there (a duplicate). A rename within one file system is atomic, so a blob
   under its final name is always complete.
3. Rows are committed every 200 files or 5 seconds. A run stops between
   files when asked (first Ctrl-C or SIGTERM), or after `--limit N` files.

After a hard kill:

| Left behind | Next run |
| --- | --- |
| Partial file in `blobs/.tmp/` | Deleted at start |
| Renamed blob with no `blob` row | Re-read, found in place, recorded as a new blob (not a duplicate) |
| `import_run` still `running` | Marked `failed` ("did not finish") |
| Files of an uncommitted batch | Still in the work list; copied again |

A file that cannot be read becomes an `attachment_read_failed` issue and
stays in the work list for the next run. A `.imessage-ap.lock` file in the
data directory keeps two file runs apart. Where the file system cannot lock
files, the run warns and continues.

**Runs.** `import_run.kind` is `records` or `attachments`. Existing counts
(`import_runs_succeeded`, the per-backup diagnostics) only consider
`records`, so the count-check baseline is unaffected.

**Derivatives.** `derivative` has one row per (blob, kind), with a status
of `pending`, `done` or `failed` and a `recipe` number for its settings:

| Media kind | Made at import | Queued for 3b |
| --- | --- | --- |
| `web_image` (JPEG, PNG, GIF, WebP) | thumbnail | none |
| `image` (HEIC, TIFF, ...) | thumbnail | preview |
| `video` | thumbnail | video |
| `voice` (CAF, AMR) | none | audio |
| `audio`, `other` | none | none |

- Thumbnails are WebP, at most 400 px on the long side, so stickers keep
  their transparency. Images use `convert … [0] -auto-orient -thumbnail
  -strip` with `-limit thread 1` and memory limits. Videos use the first
  frame from ffmpeg with `-threads 1`. Each call has a 120 s timeout.
- `--jobs` tools run at once. It defaults to `available_parallelism()`,
  which follows a container's `--cpus` quota.
- Outputs go to `derived/.tmp/` and are renamed to
  `derived/<kind>/ab/cd/<sha256>.<ext>`.
- A failed tool records a shortened error, with paths relative to the data
  directory, and never fails the run. A tool killed by the same Ctrl-C is
  left `pending`, not `failed`.
- Thumbnails are per blob, so a duplicate file is never processed twice.

**Stats** add counts only: files copied and their bytes, files pending
copy, unique blobs and their bytes, duplicates skipped (files copied minus
blobs), thumbnails made and failed, and derivatives pending and failed.

## Consequences

- A copy can be stopped and resumed any number of times. The result is
  identical to an uninterrupted run, which a test checks along with
  adopting orphans and cleaning temporary files.
- Re-running a finished backup costs one query, not 49 GB of reads.
- `attachment_bytes_copied` for one backup should equal imessage-database's
  `attachment_bytes_on_disk` (49,086,254,711 for Jan 4). It is part of the
  real-data check.
- The store trusts the hash in a blob's name and does not re-verify blobs
  already in place (only their size). A later `verify` command can re-hash
  them if bit rot is a concern.
- Deleting `blobs/` without deleting `archive.db` leaves rows pointing at
  missing files. Rebuild deletes both (spec, "Export, backup and rebuild").
- Changing thumbnail settings means bumping `THUMB_RECIPE`. Redoing older
  recipes is a later command.
- Changing the blob path layout or hash, or letting this step write
  anywhere but `blobs/`, `derived/` and archive.db, needs a new decision
  record.
