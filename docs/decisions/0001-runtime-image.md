# 0001. Runtime image: Ubuntu 24.04 with a build-time HEIC decode check

- Status: accepted
- Date: 2026-11-07
- Milestone: 1, Foundation

## Context

The spec asks for one multi-stage Docker image (amd64) that contains the Rust
binary, the built web app, and the media tools: ImageMagick with libheif for
HEIC, and ffmpeg. Every photo from an iPhone is HEIC, so an image that cannot
decode HEIC is broken for this project even if everything else works.

The first draft used `debian:trixie-slim` as the runtime base. While building
it, two things came up:

1. The cloud sandbox that coding agents build in blocks Debian's package
   mirror (`deb.debian.org`), while the Ubuntu archive is reachable. A
   Debian-based runtime could only be built in GitHub Actions, never checked
   by the agent before pushing.
2. ImageMagick lists HEIC as readable (`HEIC r--` in `convert -list format`)
   even when no HEVC decoder is installed. Decoding then fails with
   "Unsupported codec". On Ubuntu, `libheif1` depends on the AV1 plugins
   only; the HEVC decoder, `libheif-plugin-libde265`, is merely suggested and
   is not installed by default.

## Decision

- The runtime stage is `ubuntu:24.04`. It provides ImageMagick 6.9.12, libheif
  1.17.6 and ffmpeg 6.1 from the Ubuntu archive.
- `libheif-plugin-libde265` is installed by name in the Dockerfile and in
  `scripts/cloud-setup.sh`.
- The Docker build decodes `fixtures/media/fixture-32x32.heic` (a 515-byte
  invented image, documented in `fixtures/media/README.md`) and fails unless
  the result is a 32×32 image. It is bind-mounted for that step, so it adds
  no layer. `scripts/cloud-setup.sh` runs the same check. A format listing is
  not accepted as proof of HEIC support.
- The Rust build stage is `rust:<version>-slim-bookworm`. Bookworm's glibc
  (2.36) is older than Ubuntu 24.04's (2.39), so the binary runs on the
  runtime image. The web build stage stays on the Debian-based
  `node:22-trixie-slim`; it needs npm only, not apt.

## Consequences

- Agents can build and smoke-test the exact production image in the sandbox
  before pushing, using a local copy of the Dockerfile that injects the
  sandbox proxy. Nothing proxy-related is committed.
- A missing or broken HEIC decoder fails the build, not the first import.
  This was confirmed: removing `libheif-plugin-libde265` makes the build fail
  at the decode step.
- The image is larger than a Debian slim one would be (about 770 MB, mostly
  ffmpeg and ImageMagick delegates). Acceptable for one container on a home
  server; revisit if size matters.
- Ubuntu's ImageMagick is 6.x, so commands use `convert`/`identify`, not the
  7.x `magick` entry point. Code that shells out to ImageMagick should do the
  same, or detect which is present.
- Changing the base image, dropping the libde265 plugin, or weakening the
  decode check needs a new decision record that supersedes this one.
