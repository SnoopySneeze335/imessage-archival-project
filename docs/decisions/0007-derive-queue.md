# 0007. The derive queue: previews and audio/video conversions

- Status: accepted
- Date: 2026-11-16
- Milestone: 3b, Attachments

## Context

The attachment step (decision 0005) copies every file into `/data/blobs`,
makes thumbnails, and queues the rest as `pending` derivatives: on the Jan 4
backup, 5,671 image previews, 1,266 videos and 110 voice messages. The spec
asks that images show inline and that videos and voice messages play inline
in the browser. The originals stay in the blob store whatever happens here.

Things that shaped the decision:

- **HEIC** shows only in Safari. TIFF and DNG are the same. Every image
  queued for a preview needs one, unless its bytes are really a JPEG, PNG,
  GIF, WebP, BMP or AVIF (a file labelled HEIC can hold a JPEG).
- **HEVC**, the codec iPhones record in, plays in Safari, and in Chrome and
  Edge only where the machine has hardware HEVC decoding. Firefox on Linux
  cannot play it. Most of the 1,266 videos are expected to be HEVC, many of
  them HDR (HLG, the default on iPhone 12 and later).
- **CAF**, the container of voice messages, plays only in Safari. The audio
  inside may be AAC, which every browser plays in MP4.
- `/data` sits on an Unraid share whose SSD cache also holds `docker.img` and
  appdata, so a run must never fill it.
- snoopyserver is an i5-9400 and the container runs with `--cpus=2`.

## Decision

**`imessage-ap derive --data <dir> [--kind image|video|voice] [--jobs N]
[--limit N] [--min-free SIZE] [--plan]`** works through the pending rows of
`derivative`. It reads only archive.db and `/data/blobs`, never a backup, and
writes only to `derived/` and archive.db. It takes the data directory's lock,
so it never runs beside `attachments`. Kinds run in the order voice, image,
video, or one at a time with `--kind`.

**Convert only what browsers cannot use; remux when only the container is
wrong.** "Browsers" means current Safari, Chrome, Edge and Firefox on any
system, without relying on hardware decoding. The user chose this over
leaving HEVC as it is, which would play only in Safari and in Chrome or Edge
with hardware HEVC. The rules are in `media::convert`:

| Kind | Used as is (`original`) | `remux` | `transcode` |
| --- | --- | --- | --- |
| `preview` | JPEG, PNG, GIF, WebP, BMP or AVIF bytes | | WebP q82, ≤ 2048 px, auto-oriented, metadata stripped but the ICC profile kept (Display P3 colours) |
| `video` | MP4/QuickTime with H.264 (8-bit 4:2:0, Baseline to High) and AAC, MP3 or no audio | same video with other audio (copied video, AAC 128 kbit/s), or a file that needed the `chnl` patch | H.264 High, CRF 23, `veryfast`, ≤ 1920 px on the long side, AAC; HLG and PQ tone-mapped to BT.709 (`zscale` and `tonemap=hable`) |
| `audio` | MP4 or MP3 with AAC or MP3 | AAC in CAF, copied into M4A | AAC 64 kbit/s M4A (Opus, AMR, PCM, iLBC, ...) |

- Outputs are `derived/preview/ab/cd/<sha256>.webp`,
  `derived/video/…/<sha256>.mp4` and `derived/audio/…/<sha256>.m4a`, with
  `+faststart` so playback starts before the whole file arrives.
- Opus is converted rather than copied: ffmpeg 6.1 cannot copy Opus from CAF
  into MP4, and older iOS Safari cannot play it in WebM.
- Animated HEIF stickers get a still preview of their first frame. Animation
  can come later with a recipe bump.
- `derivative.action` (archive schema 5) records which of the three was
  done. A `done` row with `action = 'original'` has no `path`: the viewer
  serves the blob itself. Thumbnails leave `action` NULL.

**Same inputs and fallbacks as thumbnails.** Every file reaches a tool
through `media::input::Input` (decision 0006). Previews use the thumbnail
pipeline with larger settings, including the fallback to ffmpeg's first
video frame for MP4-family files ImageMagick cannot read. At the start of a
run, failed rows of the chosen kinds that an older decoder tried go back to
`pending` (the same `retry_failed` as thumbnails), so a decoder fix reaches
them once.

**Resumable, like the attachment step.**

- Each output is written to `derived/.tmp/` and renamed into place.
  Leftovers in `.tmp` are deleted at the start of the next run.
- Rows are committed every 50 files or 5 s.
- The first Ctrl-C or SIGTERM lets the files in progress finish and be
  recorded, then the run exits cleanly. The second exits at once. Under
  Docker, tini passes the signal only to `imessage-ap`, so the tools finish
  their files. A tool killed by a terminal's Ctrl-C leaves its file `pending`.
  A long 4K video can take several minutes on one thread, so stop the
  container with a long timeout (`docker stop -t 600`).
- `derive_run` records each run (`running`, `succeeded`, `failed`). A run
  killed outright is marked failed by the next one. `import_run` is left
  alone, so import counts are unaffected.
- With nothing pending, a run is a few queries and starts no tools.

**One thread per tool call; `--jobs` sets the CPU use.** `-limit thread 1`
for ImageMagick, and `-threads 1` and `-filter_threads 1` for ffmpeg and
x264. `--jobs` defaults to `available_parallelism()`, which follows
`--cpus`. Images and voice messages time out after 120 s. Videos time out
after 120 s plus 8 s per second of video.

**Disk space.** A file is started only while the disk holding `derived/`
has `--min-free` (default 5 GiB) plus twice the file's size free. Below
that, the run stops cleanly and exits with an error saying how much is free.
The file stays `pending`. A tool error saying the disk is full is treated
the same way and is never recorded as a file failure.

**Failures.** Any other tool error or timeout marks the row `failed` with a
short error (paths relative to the data directory), and the run continues.

**`--plan`** probes the files a run would work on with ffprobe and prints,
per kind and action, the files, input size, total length, and projected
output size and run time. It needs a current schema (`migrate` first) and
changes nothing in archive.db or the stores. The only files it writes are
the temporary patched copies every read through `Input` may need, in
`derived/.tmp/`.

**Stats** add, for each of `thumb`, `preview`, `video` and `audio`, the
counts `done` (`made` plus `original`), `failed` and `pending`, plus `bytes`,
and a `derivative_bytes` total.

**Tools.** These are the Ubuntu 24.04 packages decision 0001 already
installs: ffmpeg and ffprobe 6.1.1 (libx264, the native AAC encoder, zimg
for `zscale`, libwebp), ImageMagick 6.9.12-98, and libheif 1.17.6 with
libde265. No newer ffmpeg is needed: its one known problem, the `chnl` box,
is handled by decision 0006. The Dockerfile does not change.

## Measurements behind the estimates

These were measured on the agent sandbox (Xeon at 2.1 GHz) with invented
footage. Real camera footage encodes slower than test patterns, and an
i5-9400 is about 1.3 to 1.5 times faster per core.

| Work | Threads | Result |
| --- | --- | --- |
| 1080p HEVC to H.264, test pattern | 2 | 47 fps |
| 4K HEVC to 1080p H.264, test pattern | 2 | 27 fps |
| 60 s 1080p HDR HEVC to H.264, tone-mapped | 1 | 97 s (1.6 × real time) |
| 12 MP image to a 2048 px WebP | 1 | about 1.5 s |
| Remux of 10 s of 1080p video | 1 | 0.08 s |

`--plan` uses rates derived from these. Its output on the real archive
replaces the estimates below. These were made before the real run, for
`--cpus=2`, from the blob counts alone:

| Kind | Files | Output on disk | Run time |
| --- | --- | --- | --- |
| voice | 110 | 10–35 MB | under 1 min |
| image | 5,671 | 1.4–2.6 GB | 50–70 min |
| video | 1,266 | 10–20 GB | 8–14 h (6–20 h possible) |

## Consequences

- Every attachment with a file can be shown or played in any current
  browser, from `derived/` or from the blob itself. Milestone 4's viewer
  serves `derivative.path` when it is set, and the blob otherwise.
- Video conversion is the one long step. It is CPU-bound and runs
  separately with `--kind video`. GPU (NVENC) transcoding in v2 would add a
  recipe, not replace this one.
- HDR videos are seen tone-mapped to SDR. The originals keep the HDR.
- `derived/` grows by about the video conversions' size. All of it is
  rebuildable from the blobs, so it is not part of the backup set.
- Changing a recipe's settings means bumping its `*_RECIPE` constant.
  Redoing older recipes is a later command, as for thumbnails.
- Serving HEVC as is (for example to Safari only), or loosening "any
  browser", needs a new decision record.
