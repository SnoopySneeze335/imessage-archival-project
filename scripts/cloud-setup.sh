#!/usr/bin/env bash
# Sets up a Claude Code cloud environment (Ubuntu 24.04) for this repo:
# Rust (version from rust-toolchain.toml), Node 22, ImageMagick with HEIC
# decoding, ffmpeg and sqlite3. Safe to run repeatedly; anything already
# present is skipped. Run as root from anywhere.
#
# Downloads come only from the Ubuntu archive, static.rust-lang.org and
# nodejs.org, each checked against its published SHA-256.
set -euo pipefail

NODE_MAJOR=22
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rust_version="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$repo_root/rust-toolchain.toml")"

log() { printf '\n==> %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

# --- System packages ---------------------------------------------------------
packages=(
  build-essential pkg-config curl ca-certificates xz-utils
  sqlite3 ffmpeg imagemagick
  libheif1 libheif-plugin-libde265 # libde265 is the HEIC decoder; libheif only suggests it
)
missing=()
for p in "${packages[@]}"; do
  dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed' || missing+=("$p")
done
if [ "${#missing[@]}" -gt 0 ]; then
  log "Installing: ${missing[*]}"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends "${missing[@]}"
else
  log "System packages already installed"
fi

# --- Rust ----------------------------------------------------------------------
export PATH="$HOME/.cargo/bin:$PATH"
if ! have rustup; then
  log "Installing rustup"
  base=https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu
  tmp="$(mktemp -d)"
  curl -fsSLo "$tmp/rustup-init" "$base/rustup-init"
  expected="$(curl -fsSL "$base/rustup-init.sha256" | awk '{print $1}')"
  echo "$expected  $tmp/rustup-init" | sha256sum -c --quiet
  chmod +x "$tmp/rustup-init"
  "$tmp/rustup-init" -y --no-modify-path --profile minimal --default-toolchain none
  rm -rf "$tmp"
fi
if ! rustup toolchain list | grep -q "^${rust_version}"; then
  log "Installing Rust $rust_version"
  rustup toolchain install "$rust_version" --profile minimal --component rustfmt,clippy
else
  log "Rust $rust_version already installed"
fi

# --- Node ----------------------------------------------------------------------
node_major() { node --version 2>/dev/null | sed -E 's/^v([0-9]+).*/\1/'; }
if [ "$(node_major || echo 0)" != "$NODE_MAJOR" ]; then
  log "Installing Node $NODE_MAJOR"
  version="$(curl -fsSL https://nodejs.org/dist/index.json \
    | grep -oE "\"version\":\"v${NODE_MAJOR}\.[0-9]+\.[0-9]+\"" | head -1 | cut -d'"' -f4)"
  file="node-${version}-linux-x64.tar.xz"
  tmp="$(mktemp -d)"
  curl -fsSLo "$tmp/$file" "https://nodejs.org/dist/${version}/${file}"
  (cd "$tmp" && curl -fsSL "https://nodejs.org/dist/${version}/SHASUMS256.txt" | grep " ${file}\$" | sha256sum -c --quiet)
  mkdir -p /opt/node
  tar -xJf "$tmp/$file" -C /opt/node --strip-components=1
  ln -sf /opt/node/bin/node /opt/node/bin/npm /opt/node/bin/npx /usr/local/bin/
  rm -rf "$tmp"
else
  log "Node $(node --version) already installed"
fi

# --- Checks ---------------------------------------------------------------------
log "Checking HEIC decoding"
heic="$repo_root/fixtures/media/fixture-32x32.heic"
if convert "$heic" png:- | identify - | grep -q ' 32x32 '; then
  echo "ImageMagick decodes HEIC"
else
  echo "ImageMagick cannot decode HEIC (is libheif-plugin-libde265 installed?)" >&2
  exit 1
fi

log "Versions"
(cd "$repo_root" && rustc --version && cargo --version)
echo "node $(node --version), npm $(npm --version)"
convert -version | head -1
ffmpeg -hide_banner -version | head -1
echo "sqlite3 $(sqlite3 --version | cut -d' ' -f1)"
