# 0006. Decoder workarounds and retrying failed derivatives

- Status: accepted
- Date: 2026-11-15
- Milestone: 3a follow-up, Attachments

## Context

The 3a real check on the Jan 4 backup made 16,736 thumbnails and failed 28.
The 28 were the only failed derivatives (previews and conversions wait for
3b). The files were not damaged: the server's copy of the backup is
byte-identical to the original, and the copy reported 0 read failures. A
probe on the server, which reported only file types and tool errors, found
two causes:

| Files | Type | Error | What reads them |
| --- | --- | --- | --- |
| 27 | `image/heic-sequence`, `.heics`, major brand `msf1` | ImageMagick: "no decode delegate for this image format" | `convert heic:<file>`, and ffmpeg's first frame |
| 1 | `video/mp4`, brand `mp42` | ffmpeg 6.1: "Unsupported 'chnl' box with version 1" | nothing in the image as is |

- Blobs have no file extension, so ImageMagick identifies them by their
  bytes. ImageMagick 6.9.12 recognises `heic` and `mif1` HEIF files but not
  `msf1` (an image sequence, such as an animated sticker). libheif, which it
  uses for HEIC, reads these files when the format is named.
- `chnl` (ISO/IEC 23001-8 channel layout) version 1 is newer than ffmpeg
  6.1.1, the version in Ubuntu 24.04. ffmpeg refuses to open the whole file
  when it sees one in an audio track, so even the video cannot be read.

Previews and conversions in 3b read the same files with the same tools and
would fail the same way.

## Decision

**One way into the tools.** Every tool call on a blob goes through
`media::input::Input::prepare`. It reads the file's `ftyp` box and:

- names HEIF files (major brand `heic`, `heix`, `heim`, `heis`, `hevc`,
  `hevx`, `hevm`, `hevs`, `mif1`, `mif2`, `msf1` or `miaf`) to ImageMagick as
  `heic:<file>`;
- in other MP4-family files, finds `chnl` boxes with a nonzero version or
  flags in the sample entries of sound tracks, by walking the box tree
  (`moov` up to 64 MB), and gives the tools a copy with those boxes renamed
  to `free`. Box sizes are unchanged, so nothing else moves, and every reader
  skips `free`. The copy is written to `derived/.tmp/`, deleted when the call
  ends, and cleaned up at the start of the next run if a crash leaves it. The
  blob is never written.

When ImageMagick still cannot read an image that is an MP4-family file (a
HEIF sequence with no still image, or a video labelled as a picture), the
thumbnail falls back to ffmpeg's first video frame.

3b's previews and conversions must use `Input` too. This is a rule for
3b, not optional.

**Retrying after a fix.** `derivative.decoder` (archive schema 4) records the
version of the input handling (`media::input::DECODER`) that last tried each
derivative. Existing rows get version 1. At the start of each step, failed
derivatives of that step's kinds tried by an older version go back to
`pending`, so:

- a fix reaches every file that failed before it, on the next ordinary run;
- files that already worked are not redone (that is what `recipe` is for);
- a file that still fails is recorded against the current version and is not
  retried again until the next decoder change.

`attachments` reports how many thumbnails it retried. 3b's `derive` does the
same for previews and conversions.

**Fixture.** The generator derives both cases from committed media (see
`fixtures/media/README.md`): the HEIC sample with major brand `msf1`, and a
small H.264 + AAC MP4 with a `chnl` version 1 box added. Without the
workarounds, both reproduce the errors from the real backup exactly.

## Consequences

- The 28 thumbnails are made by the next `attachments` run on the server, with
  no re-copy and no other thumbnails redone.
- A newer ffmpeg, from Ubuntu or a static build, would make the `chnl` patch
  unnecessary, but it would change decision 0001's runtime image. The patch
  is small, tested, and only applies to files that need it.
- A new workaround means bumping `DECODER` and adding a fixture case that
  reproduces the failure.
- Patching needs room for one temporary copy of the video in `derived/.tmp/`
  while it is read.
