#!/usr/bin/env bash
# Runs the demo archive locally with cargo: invented data only, under
# fixtures/out/ (gitignored). The image's equivalent is `docker run … demo`
# (docker/demo.sh).
#
#   scripts/demo.sh            # build, import if needed, serve on :8080
#   scripts/demo.sh --fresh    # start over
#   scripts/demo.sh --prepare  # only prepare the data (browser tests use it)
#   PORT=8081 scripts/demo.sh
#
# Sign in with the password "demo1234".
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

out=fixtures/out
backup="$out/demo-backup"
data="$out/demo-data"
port="${PORT:-8080}"

prepare_only=false
case "${1:-}" in
  --fresh) rm -rf "$backup" "$data" ;;
  --prepare) prepare_only=true ;;
esac

cargo build -q -p imessage-ap -p fixture-gen
bin=target/debug
if [ ! -f "$data/.demo-ready" ]; then
  rm -rf "$backup" "$data"
  "$bin/make-fixture" --variant demo --large --out "$backup"
  "$bin/imessage-ap" ingest --backup "$backup" --data "$data"
  "$bin/imessage-ap" attachments --backup "$backup" --data "$data"
  "$bin/imessage-ap" derive --data "$data" --min-free 1G
  printf 'demo1234\n' | "$bin/imessage-ap" set-password --stdin --data "$data"
  "$bin/imessage-ap" check-render --data "$data"
  touch "$data/.demo-ready"
fi

if $prepare_only; then
  exit 0
fi
if [ ! -f web/dist/index.html ]; then
  (cd web && npm ci && npm run build)
fi
exec env -u IMESSAGE_AP_DB_DIR -u IMESSAGE_AP_BLOBS_DIR -u IMESSAGE_AP_DERIVED_DIR -u IMESSAGE_AP_EXPORTS_DIR \
  "$bin/imessage-ap" serve --data "$data" --web web/dist --port "$port" --demo
