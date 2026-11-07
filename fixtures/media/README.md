# Fixture media

Small, invented media files that the generator cannot produce from integers.

| File | What it is | How it was made |
| --- | --- | --- |
| `fixture-32x32.heic` | 32×32 orange checkerboard, HEVC-in-HEIF | `convert -size 32x32 pattern:checkerboard -fill "#f28c28" -opaque black in.png && heif-enc -q 40 -o fixture-32x32.heic in.png` (libheif 1.17.6, x265 plugin) on Ubuntu 24.04 |

The Docker build decodes `fixture-32x32.heic` to prove ImageMagick can read
HEIC; seeing HEIC listed in `-list format` is not enough, because the listing
appears even without the libde265 decoder plugin.
