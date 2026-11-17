#!/bin/sh
# Runs imessage-ap as PUID:PGID (Unraid defaults 99:100).
# `demo` runs the demo archive of invented data instead (docker/demo.sh).
set -eu

data="${IMESSAGE_AP_DATA:-/data}"

if [ "${1:-}" = "demo" ]; then
  shift
  cmd=/usr/local/bin/demo.sh
  dirs="$data"
else
  cmd=imessage-ap
  # Each store can be its own mount (decision 0009).
  dirs="$data ${IMESSAGE_AP_DB_DIR:-} ${IMESSAGE_AP_BLOBS_DIR:-} ${IMESSAGE_AP_DERIVED_DIR:-} ${IMESSAGE_AP_EXPORTS_DIR:-}"
fi

for dir in $dirs; do
  mkdir -p "$dir"
done

if [ "$(id -u)" = "0" ]; then
  # Only the top-level folders are chowned: re-owning tens of GB on every
  # start would be slow, and files the app writes are created as PUID:PGID
  # anyway.
  for dir in $dirs; do
    owner="$(stat -c '%u:%g' "$dir")"
    if [ "$owner" != "${PUID}:${PGID}" ]; then
      chown "${PUID}:${PGID}" "$dir"
    fi
  done
  exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups -- "$cmd" "$@"
fi

exec "$cmd" "$@"
