#!/usr/bin/env bash
# Fails if a tracked file outside fixtures/ contains something that looks
# like a phone number or an email address.
#
# Matches are reported as path:line only. The matched text is never printed,
# so a real number or address does not end up in CI logs.
#
# Exceptions go in scripts/ci/pii-allowlist.txt: one fixed string per line
# (blank lines and lines starting with # are ignored). A line matching the
# pattern is allowed when, with every allowlisted string removed, it no longer
# matches.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
allowlist="scripts/ci/pii-allowlist.txt"

# North American numbers with separators, shaped (NNN) NNN-NNNN, NNN-NNN-NNNN,
# NNN.NNN.NNNN or NNN NNN NNNN, optionally with a leading +1 or 1.
nanp='(^|[^0-9A-Za-z])(\+?1[ .-]?)?(\([2-9][0-9]{2}\)[ .-]?|[2-9][0-9]{2}[ .-])[0-9]{3}[ .-][0-9]{4}([^0-9]|$)'
# International: + then 10 to 15 digits, optionally separated by space, dot or dash.
e164='(^|[^0-9A-Za-z+])\+[1-9]([ .-]?[0-9]){9,14}([^0-9]|$)'
email='[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}'
pattern="$nanp|$e164|$email"

allowed=()
if [ -f "$allowlist" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in '' | '#'*) continue ;; esac
    allowed+=("$line")
  done <"$allowlist"
fi

failed=0
# -I skips binary files. Hashes in lockfiles have no separators or leading +,
# so they do not match.
while IFS= read -r hit; do
  path=${hit%%:*}
  rest=${hit#*:}
  lineno=${rest%%:*}
  text=${rest#*:}
  for a in "${allowed[@]+"${allowed[@]}"}"; do
    text=${text//"$a"/}
  done
  if printf '%s\n' "$text" | grep -qE -- "$pattern"; then
    printf 'possible phone number or email: %s:%s\n' "$path" "$lineno"
    failed=1
  fi
done < <(git grep -nIE -e "$pattern" -- . ':(exclude)fixtures/' ':(exclude)scripts/ci/pii-allowlist.txt' || true)

if [ "$failed" -ne 0 ]; then
  cat >&2 <<'MSG'
PII guard failed. Phone numbers and email addresses belong only in fixtures/,
using the fictional 555-0100..555-0199 range and example.com/.org/.net.
If a match is a false positive, add the exact string to
scripts/ci/pii-allowlist.txt.
MSG
  exit 1
fi
echo "PII guard passed."
