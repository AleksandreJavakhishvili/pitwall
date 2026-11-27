#!/usr/bin/env bash
# Copies the extensions Pitwall ships into DEST, for the app's bundle
# (scripts/package-app.sh; docs/spec/extensions.md, "Installing"): every
# folder of SRC with an extension.json, with only what it needs at run
# time, as a tile package holds it (crates/pitwall-core/src/extensions/
# package.rs): its extension.json, tile.wasm, README.md, CHANGELOG.md,
# licence and NOTICE, agents/SKILL.md, the assets/, icons/ and
# screenshots/ folders, and every file its manifest names (a tile's
# component and input schema, an adapter's script). Sources and build
# files (src/, Cargo.toml, Cargo.lock, scripts/), tests (test, tests),
# node_modules, target, hidden files and .DS_Store stay out. Settings
# lists them under Available.
#
#   scripts/stage-extensions.sh extensions target/package/extensions
set -euo pipefail

die() { echo "stage-extensions: $*" >&2; exit 1; }
[ $# -eq 2 ] || die "usage: $0 SRC DEST"
src=$1 dest=$2
[ -d "$src" ] || die "$src isn't a folder"

# What every extension may ship, when it has it.
always=(extension.json tile.wasm README.md CHANGELOG.md LICENSE LICENSE.md LICENSE.txt NOTICE agents/SKILL.md assets icons screenshots)

rm -rf "$dest"
mkdir -p "$dest"
n=0
for d in "$src"/*/; do
  d=${d%/}
  [ -f "$d/extension.json" ] || continue
  id=$(basename "$d")
  out="$dest/$id"
  mkdir -p "$out"
  # The manifest's strings that name a file or folder in it ("tile.wasm",
  # "adapter.mjs", "input.schema.json"): kept too.
  named=()
  while IFS= read -r s; do named+=("$s"); done < <(grep -o '"[^"\\]*"' "$d/extension.json" | sed -e 's/^"//' -e 's/"$//')
  for rel in "${always[@]}" ${named[@]+"${named[@]}"}; do
    rel=${rel#./}
    case "$rel" in '' | /* | .* | *..* | */.*) continue ;; esac
    [ -e "$d/$rel" ] || continue
    [ -e "$out/$rel" ] && continue
    mkdir -p "$(dirname "$out/$rel")"
    cp -R "$d/$rel" "$out/$rel"
  done
  n=$((n + 1))
done
[ $n -gt 0 ] || die "no extensions in $src"
find "$dest" -depth \( -type d \( -name test -o -name tests -o -name node_modules -o -name target \) -o -name '.*' \) -exec rm -rf {} +
echo "stage-extensions: $n into $dest"
