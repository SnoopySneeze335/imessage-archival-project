# syntax=docker/dockerfile:1.7
# imessage-ap: one image with the Rust server/ingester, the built web app,
# and the media tools (ImageMagick with HEIC support, ffmpeg).

ARG RUST_VERSION=1.97.0
ARG NODE_VERSION=22

# --- Web app ---------------------------------------------------------------
FROM node:${NODE_VERSION}-trixie-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json web/.npmrc ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
RUN npm run build

# --- Rust binary -----------------------------------------------------------
# Bookworm's glibc (2.36) is older than the runtime's (2.39), so the binary runs there.
FROM rust:${RUST_VERSION}-slim-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY fixtures/generator/ fixtures/generator/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p imessage-ap \
 && install -D target/release/imessage-ap /out/imessage-ap

# --- Runtime ---------------------------------------------------------------
FROM ubuntu:24.04 AS runtime
ARG DEBIAN_FRONTEND=noninteractive
# libheif-plugin-libde265 is what decodes HEIC; libheif only suggests it.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      imagemagick libheif1 libheif-plugin-libde265 ffmpeg tini \
 && rm -rf /var/lib/apt/lists/*
# Fail the build unless ImageMagick can really decode HEIC (fixtures/media/README.md).
RUN --mount=type=bind,source=fixtures/media/fixture-32x32.heic,target=/tmp/check.heic \
    convert /tmp/check.heic png:- | identify - | grep -q ' 32x32 '

COPY --from=rust /out/imessage-ap /usr/local/bin/imessage-ap
COPY --from=web /src/web/dist /app/web
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

ENV IMESSAGE_AP_PORT=8080 \
    IMESSAGE_AP_DATA=/data \
    IMESSAGE_AP_WEB=/app/web \
    PUID=99 \
    PGID=100 \
    RUST_LOG=info

LABEL org.opencontainers.image.title="imessage-ap" \
      org.opencontainers.image.description="Self-hosted archive and viewer for iMessage history from local iPhone backups" \
      org.opencontainers.image.licenses="GPL-3.0-only" \
      org.opencontainers.image.source="https://github.com/SnoopySneeze335/imessage-archival-project"

VOLUME ["/data", "/backups"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
  CMD ["imessage-ap", "healthcheck"]
ENTRYPOINT ["tini", "--", "entrypoint.sh"]
CMD ["serve"]
