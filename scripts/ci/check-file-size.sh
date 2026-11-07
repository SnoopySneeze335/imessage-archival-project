#!/usr/bin/env bash
# Fails if any tracked file is larger than 5 MB.
# Real backups, databases and media must never enter the repository.
set -euo pipefail

limit=$((5 * 1024 * 1024))
cd "$(git rev-parse --show-toplevel)"

failed=0
while IFS= read -r -d '' path; do
  [ -f "$path" ] || continue
  size=$(wc -c <"$path")
  if [ "$size" -gt "$limit" ]; then
    printf 'too large: %s (%d bytes > %d)\n' "$path" "$size" "$limit"
    failed=1
  fi
done < <(git ls-files -z)

if [ "$failed" -ne 0 ]; then
  echo "File size guard failed: files over 5 MB are not allowed." >&2
  exit 1
fi
echo "File size guard passed."
