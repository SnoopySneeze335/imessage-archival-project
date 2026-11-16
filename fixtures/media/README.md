# Fixture media

Small, invented media files that the generator cannot produce from integers.

| File | What it is | How it was made |
| --- | --- | --- |
| `fixture-32x32.heic` | 32×32 orange checkerboard, HEVC-in-HEIF | `convert -size 32x32 pattern:checkerboard -fill "#f28c28" -opaque black in.png && heif-enc -q 40 -o fixture-32x32.heic in.png` (libheif 1.17.6, x265 plugin) on Ubuntu 24.04 |

The Docker build decodes `fixture-32x32.heic` to prove ImageMagick can read
HEIC; seeing HEIC listed in `-list format` is not enough, because the listing
appears even without the libde265 decoder plugin.

| File | What it is | How it was made |
| --- | --- | --- |
| `fixture-1s-hevc.mov` | 1 s, 64×48, 10 fps of ffmpeg's built-in `testsrc` pattern, HEVC (`hvc1`) in QuickTime, no audio | `ffmpeg -f lavfi -i testsrc=size=64x48:rate=10 -t 1 -c:v libx265 -preset medium -crf 35 -pix_fmt yuv420p -tag:v hvc1 -an -map_metadata -1 -fflags +bitexact -flags:v +bitexact fixture-1s-hevc.mov` (ffmpeg 6.1.1, x265 3.5) on Ubuntu 24.04 |

The generator puts the video in the fixture backup as an iPhone-style `.MOV`
attachment, so thumbnails (Milestone 3a) and H.264 conversion (3b) are tested
on HEVC, the codec iPhones record in.

| File | What it is | How it was made |
| --- | --- | --- |
| `fixture-1s-aac.mp4` | 1 s, 64×48, 10 fps of `testsrc`, H.264, with 1 s of a 440 Hz mono AAC tone | `ffmpeg -f lavfi -i testsrc=size=64x48:rate=10 -f lavfi -i sine=frequency=440:sample_rate=8000 -t 1 -c:v libx264 -preset veryslow -crf 40 -threads 1 -pix_fmt yuv420p -c:a aac -b:a 16k -ac 1 -brand mp42 -map_metadata -1 -fflags +bitexact -flags:v +bitexact -flags:a +bitexact fixture-1s-aac.mp4` (ffmpeg 6.1.1, x264) on Ubuntu 24.04 |

Two attachments are derived from these files by the generator
(`fixtures/generator/src/media.rs`) rather than committed, so they stay
reproducible from the files above:

- **`fixture-32x32.heic` with its major brand changed to `msf1`**, as iOS
  writes HEIF image sequences (`.heics`, such as animated stickers).
  ImageMagick 6 does not recognise that brand from the bytes and fails with
  "no decode delegate"; the app names the format explicitly.
- **`fixture-1s-aac.mp4` with a version-1 `chnl` (channel layout) box added
  to its audio sample entry**, as some iPhone videos have. ffmpeg 6.1 refuses
  to open such a file at all ("Unsupported 'chnl' box with version 1"); the
  app reads a copy with the box renamed to `free`.

Both reproduce the exact errors seen on the real backup (decision 0006).

| File | What it is | How it was made |
| --- | --- | --- |
| `fixture-1s-hlg.mov` | 1 s, 64×48, 10 fps of `testsrc`, 10-bit HEVC tagged HDR (BT.2020, HLG), no audio, like an iPhone HDR video | `ffmpeg -f lavfi -i testsrc=size=64x48:rate=10 -t 1 -c:v libx265 -preset medium -crf 35 -pix_fmt yuv420p10le -x265-params colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc -color_primaries bt2020 -color_trc arib-std-b67 -colorspace bt2020nc -tag:v hvc1 -an -map_metadata -1 -fflags +bitexact -flags:v +bitexact fixture-1s-hlg.mov` (ffmpeg 6.1.1, x265 3.5) on Ubuntu 24.04 |
| `fixture-1s-aac.adts` | 1 s of a 330 Hz mono AAC-LC tone at 8 kHz, as raw ADTS frames | `ffmpeg -f lavfi -i sine=frequency=330:sample_rate=8000 -t 1 -c:a aac -b:a 16k -ac 1 -map_metadata -1 -fflags +bitexact -flags:a +bitexact -f adts fixture-1s-aac.adts` (ffmpeg 6.1.1) on Ubuntu 24.04 |

The background queue (Milestone 3b, decision 0007) is tested on these and on
more attachments the generator derives:

- **`fixture-1s-aac.adts` rewrapped into a CAF** (`media::caf_aac`), as iOS
  stores AAC voice messages. ffmpeg cannot write AAC into CAF, so the
  generator writes the `desc`, `kuki` (ES descriptor), `pakt` and `data`
  chunks itself. The queue copies the AAC into an M4A without re-encoding.
- **`fixture-1s-hlg.mov`** as an iPhone HDR video, converted to SDR H.264.
- **`fixture-1s-aac.mp4` as it is**, which browsers play, so no file is made.
- **A PNG labelled HEIC**, which browsers show, so no preview is made.
- **Damaged files, one per converted kind**: the HEIC cut to 200 bytes, the
  HEVC `.mov` cut to 600 bytes (before its `moov` box), and a PCM CAF cut to
  40 bytes. Each must be recorded as failed without stopping the run.
