#!/usr/bin/env bash
# Copies the extensions Pitwall ships into DEST, for the app's bundle
# (scripts/package-app.sh; docs/spec/extensions.md, "Installing"): every
# folder of SRC with an extension.json, without what is only for tests
# (folders named test) or made by a package manager (node_modules), and
# without .DS_Store files. Settings lists them under Available.
#
#   scripts/stage-extensions.sh extensions target/package/extensions
set -euo pipefail

die() { echo "stage-extensions: $*" >&2; exit 1; }
[ $# -eq 2 ] || die "usage: $0 SRC DEST"
src=$1 dest=$2
[ -d "$src" ] || die "$src isn't a folder"

rm -rf "$dest"
mkdir -p "$dest"
n=0
for d in "$src"/*/; do
  d=${d%/}
  [ -f "$d/extension.json" ] || continue
  id=$(basename "$d")
  cp -R "$d" "$dest/$id"
  n=$((n + 1))
done
[ $n -gt 0 ] || die "no extensions in $src"
find "$dest" -depth \( -type d \( -name test -o -name node_modules \) -o -name .DS_Store \) -exec rm -rf {} +
echo "stage-extensions: $n into $dest"
