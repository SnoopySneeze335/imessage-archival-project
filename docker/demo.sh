#!/bin/sh
# `docker run … imessage-ap demo`: the demo archive, invented data only.
#
# On first start, generates the demo backup (fixtures/generator, with the
# "All message kinds" conversations and a 25,000-message conversation),
# imports it, copies its files, makes previews and conversions, and sets the
# password to "demo1234". Then serves it, marked as the demo on the sign-in page.
# Screenshots of it show only invented people and messages.
set -eu

data="${IMESSAGE_AP_DATA:-/data}"
# Every store stays under the demo's own data directory, whatever the
# container's environment says.
unset IMESSAGE_AP_DB_DIR IMESSAGE_AP_BLOBS_DIR IMESSAGE_AP_DERIVED_DIR IMESSAGE_AP_EXPORTS_DIR
backup="$data/demo-backup"

if [ -f "$data/archive.db" ] && [ ! -d "$backup" ]; then
  echo "demo: $data holds an archive that is not the demo; refusing to touch it." >&2
  echo "demo: give the demo its own empty folder." >&2
  exit 1
fi

if [ ! -f "$data/.demo-ready" ]; then
  echo "demo: generating the invented backup"
  make-fixture --variant demo --large --out "$backup" --force
  imessage-ap ingest --backup "$backup" --data "$data"
  imessage-ap attachments --backup "$backup" --data "$data"
  imessage-ap derive --data "$data" --min-free 1G
  printf 'demo1234\n' | imessage-ap set-password --stdin --data "$data"
  imessage-ap check-render --data "$data"
  touch "$data/.demo-ready"
fi

exec imessage-ap serve --data "$data" --demo
