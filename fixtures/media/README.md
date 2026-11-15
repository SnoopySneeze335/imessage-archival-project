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
