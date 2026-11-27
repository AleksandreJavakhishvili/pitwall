#!/usr/bin/env bash
# Copies the extensions Pitwall ships into DEST, for the app's bundle
# (scripts/package-app.sh; docs/spec/extensions.md, "Installing"): every
# folder of SRC with an extension.json. Settings lists them under
# Available.
#
# An extension of tiles (a folder with a tile.wasm) is copied as its
# package would hold it (docs/spec/tile-sdk/packaging.md §6.1): its
# manifest, component, README, licence, changelog, the JSON schemas at
# its top, its icon file, agents/SKILL.md, assets/, screenshots/ and
# media/. Never its sources (src/, Cargo.toml, Cargo.lock, build.rs,
# scripts/, tests/, target/). Any other extension (an adapter) is copied
# whole, without what is only for tests (folders named test or tests),
# made by a package manager (node_modules) or by a build (target). No
# .DS_Store files either way.
#
#   scripts/stage-extensions.sh extensions target/package/extensions
set -euo pipefail

die() { echo "stage-extensions: $*" >&2; exit 1; }
[ $# -eq 2 ] || die "usage: $0 SRC DEST"
src=$1 dest=$2
[ -d "$src" ] || die "$src isn't a folder"

# The top files a package holds (besides *.json schemas and the icon).
top_files="extension.json tile.wasm README.md CHANGELOG.md LICENSE LICENSE.md LICENSE.txt NOTICE icon.png icon.webp"
top_dirs="agents assets screenshots media"

rm -rf "$dest"
mkdir -p "$dest"
n=0
for d in "$src"/*/; do
  d=${d%/}
  [ -f "$d/extension.json" ] || continue
  id=$(basename "$d")
  if [ -f "$d/tile.wasm" ]; then
    mkdir -p "$dest/$id"
    for f in $top_files; do
      [ -f "$d/$f" ] && cp "$d/$f" "$dest/$id/$f"
    done
    for f in "$d"/*.json; do
      case $(basename "$f") in package.json|package-lock.json) continue ;; esac
      [ -f "$f" ] && cp "$f" "$dest/$id/"
    done
    for sub in $top_dirs; do
      [ -d "$d/$sub" ] && cp -R "$d/$sub" "$dest/$id/$sub"
    done
  else
    cp -R "$d" "$dest/$id"
  fi
  n=$((n + 1))
done
[ $n -gt 0 ] || die "no extensions in $src"
find "$dest" -depth \( -type d \( -name test -o -name tests -o -name node_modules -o -name target \) -o -name .DS_Store \) -exec rm -rf {} +
echo "stage-extensions: $n into $dest"
