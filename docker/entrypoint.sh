#!/bin/sh
# Runs imessage-ap as PUID:PGID (Unraid defaults 99:100).
set -eu

data="${IMESSAGE_AP_DATA:-/data}"
mkdir -p "$data"

if [ "$(id -u)" = "0" ]; then
  # Only the top-level folder is chowned: re-owning tens of GB on every start
  # would be slow, and files the app writes are created as PUID:PGID anyway.
  owner="$(stat -c '%u:%g' "$data")"
  if [ "$owner" != "${PUID}:${PGID}" ]; then
    chown "${PUID}:${PGID}" "$data"
  fi
  exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups -- imessage-ap "$@"
fi

exec imessage-ap "$@"
