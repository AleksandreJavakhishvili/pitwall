#!/bin/sh
# Checks every commit in a git range against the commit convention, using the
# same rules as the commit-msg hook (.githooks/commit-msg). CI runs it on the
# commits of a pull request.
#
#   scripts/check-commits.sh origin/main..HEAD
#   scripts/check-commits.sh --title "feat(ui): show the branch in the header"
set -u

root=$(cd "$(dirname "$0")/.." && pwd)
hook="$root/.githooks/commit-msg"
tmp=$(mktemp "${TMPDIR:-/tmp}/pitwall-commit.XXXXXX")
trap 'rm -f "$tmp"' EXIT

if [ "${1:-}" = "--title" ]; then
  printf '%s\n' "${2:-}" > "$tmp"
  if sh "$hook" --title "$tmp"; then echo "ok    title: ${2:-}"; exit 0; fi
  exit 1
fi

range=${1:-}
if [ -z "$range" ]; then
  echo "usage: $0 <revision-range> | --title <text>" >&2
  exit 2
fi

shas=$(git rev-list --reverse "$range") || exit 2
bad=0
count=0
for sha in $shas; do
  count=$((count + 1))
  git log -1 --format=%B "$sha" > "$tmp"
  subject=$(git log -1 --format=%s "$sha")
  short=$(git rev-parse --short "$sha")
  if sh "$hook" "$tmp" 2>"$tmp.err"; then
    echo "ok    $short $subject"
  else
    echo "FAIL  $short $subject"
    sed 's/^/      /' "$tmp.err"
    bad=$((bad + 1))
  fi
  rm -f "$tmp.err"
done

echo "$count commit(s) checked, $bad not following the convention (see CONTRIBUTING.md)."
[ "$bad" -eq 0 ]
