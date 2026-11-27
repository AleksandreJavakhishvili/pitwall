#!/bin/sh
# macOS: tiles run under the hardened runtime, as the signed app runs them
# (CI's macOS job; docs/spec/tile-sdk/runtime.md §2.6). A tile update once
# made the installed Pitwall crash on every launch: Wasmtime makes the
# memory it compiled a tile into executable, and the hardened runtime
# killed the app for it ("Code Signature Invalid").
#
# 1. The app's entitlements file sets the key that allows it.
# 2. A small program that starts tiles as the app does (pitwall-tile-host's
#    tile_smoke example) is signed ad hoc with --options runtime and those
#    entitlements, then starts the hello fixture and each shipped extension
#    with a tile.wasm, compiled and then from the compile cache.
# 3. The control: the same program signed without the entitlements is
#    killed, so step 2 tests what it says (a note, not a failure, if a
#    later macOS or Wasmtime no longer needs the key).
#
#   scripts/check-hardened-tiles.sh
set -eu
cd "$(dirname "$0")/.."

[ "$(uname -s)" = Darwin ] || { echo "check-hardened-tiles: macOS only; skipped"; exit 0; }

ENTITLEMENTS=crates/pitwall-app/packaging/macos/entitlements.plist
sh scripts/check-entitlements.sh "$ENTITLEMENTS"

cargo build -p pitwall-tile-host --example tile_smoke
built="${CARGO_TARGET_DIR:-target}/debug/examples/tile_smoke"
[ -x "$built" ] || { echo "check-hardened-tiles: no $built" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$built" "$work/signed"
cp "$built" "$work/control"
codesign --force --sign - --options runtime --entitlements "$ENTITLEMENTS" "$work/signed"
sh scripts/check-entitlements.sh "$work/signed"

tiles="crates/pitwall-tile-host/tests/fixtures/hello"
for d in extensions/*/; do
  [ -f "$d/tile.wasm" ] && tiles="$tiles ${d%/}"
done
for t in $tiles; do
  echo "check-hardened-tiles: $t"
  "$work/signed" "$t" || {
    echo "check-hardened-tiles: $t didn't run under the hardened runtime with the app's entitlements (exit $?)." >&2
    echo "  The installed app would crash as this tile starts. See docs/spec/tile-sdk/runtime.md §2.6." >&2
    exit 1
  }
done

codesign --force --sign - --options runtime "$work/control"
if "$work/control" >/dev/null 2>&1; then
  echo "check-hardened-tiles: note: the control ran without the entitlements; this macOS or Wasmtime may no longer need allow-unsigned-executable-memory."
else
  echo "check-hardened-tiles: control: without the entitlements macOS stops it, as expected."
fi
echo "check-hardened-tiles: ok"
