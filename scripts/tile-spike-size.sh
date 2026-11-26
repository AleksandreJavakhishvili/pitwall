#!/bin/sh
# Tile SDK phase 0 (docs/spec/tile-sdk/spike-results.md): the release
# `pitwall` binary's size without and with the tile runtime linked
# (pitwall-app's `tile-spike` feature), on the OS this runs on. Prints a
# Markdown table; also used by .github/workflows/tile-spike.yml.
#
#   scripts/tile-spike-size.sh [--target <triple>]
set -eu
cd "$(dirname "$0")/.."
target_args=""
dir=target/release
if [ "${1:-}" = --target ]; then
  target_args="--target $2"
  dir="target/$2/release"
fi
exe=pitwall
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) exe=pitwall.exe ;; esac
out=target/tile-spike-size
mkdir -p "$out"

# shellcheck disable=SC2086
cargo build --release -p pitwall-app $target_args
cp "$dir/$exe" "$out/without-$exe"
# shellcheck disable=SC2086
cargo build --release -p pitwall-app --features tile-spike $target_args
cp "$dir/$exe" "$out/with-$exe"

size() { wc -c < "$1" | tr -d ' '; }
a=$(size "$out/without-$exe")
b=$(size "$out/with-$exe")
echo "| OS | without | with tile-spike | growth |"
echo "|---|---|---|---|"
awk -v os="$(uname -s) $(uname -m)" -v a="$a" -v b="$b" 'BEGIN {
  printf "| %s | %.2f MB | %.2f MB | **%.2f MB** (%d bytes) |\n", os, a/1e6, b/1e6, (b-a)/1e6, b-a }'

# The linked runtime works in the release binary: compile, start, render,
# click, render (exit code only on Windows, where the release app has no
# console).
PITWALL_TILE_SPIKE=crates/pitwall-tile-host/tests/fixtures/hello-tile.wasm "$out/with-$exe" \
  && echo "smoke run of the linked tile runtime: ok"
