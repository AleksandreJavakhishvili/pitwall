#!/bin/sh
# Tile SDK phase 0 (docs/spec/tile-sdk/spike-results.md): the release
# `pitwall` binary's size without and with the tile runtime linked, on the
# OS this runs on. Four builds:
#   without     as shipped
#   with        pitwall-app's `tile-spike` feature (Wasmtime, Cranelift,
#               component model, WASI p2)
#   +cache      `tile-spike-cache`: also Wasmtime's own compiled-code cache
#   lean        `tile-spike` with opt-level "s" on Wasmtime's and
#               Cranelift's crates (scripts/tile-spike-lean.toml)
# Prints a Markdown table; also used by .github/workflows/tile-spike.yml.
#
#   scripts/tile-spike-size.sh [--target <triple>]
set -eu
cd "$(dirname "$0")/.."
target_args=""
dir=target/release
label="$(uname -s) $(uname -m)"
if [ "${1:-}" = --target ]; then
  target_args="--target $2"
  dir="target/$2/release"
  label="$2"
fi
exe=pitwall
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) exe=pitwall.exe ;; esac
out=target/tile-spike-size
mkdir -p "$out"

build() { # <name> [features] [extra cargo args]
  # shellcheck disable=SC2086
  cargo build --release -p pitwall-app $target_args ${2:+--features "$2"} ${3:-}
  cp "$dir/$exe" "$out/$1-$exe"
}
build without
build with tile-spike
build cache tile-spike-cache
build lean tile-spike "--config scripts/tile-spike-lean.toml"

size() { wc -c < "$out/$1-$exe" | tr -d ' '; }
echo "| OS | without | with tile-spike (growth) | + Wasmtime cache (growth) | lean: opt-level s (growth) |"
echo "|---|---|---|---|---|"
awk -v os="$label" -v a="$(size without)" -v b="$(size with)" -v c="$(size cache)" -v d="$(size lean)" 'BEGIN {
  printf "| %s | %.2f MB | %.2f MB (**+%.2f MB**) | %.2f MB (+%.2f MB) | %.2f MB (**+%.2f MB**) |\n",
    os, a/1e6, b/1e6, (b-a)/1e6, c/1e6, (c-a)/1e6, d/1e6, (d-a)/1e6 }'

# The linked runtime works in the release binary: compile, start, render,
# click, render (exit code only on Windows, where the release app has no
# console). Skipped for a cross-built target this machine may not run.
if [ -z "$target_args" ]; then
  PITWALL_TILE_SPIKE=crates/pitwall-tile-host/tests/fixtures/hello-tile.wasm "$out/with-$exe" \
    && echo "smoke run of the linked tile runtime: ok"
fi
