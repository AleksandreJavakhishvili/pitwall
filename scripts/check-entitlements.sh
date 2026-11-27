#!/bin/sh
# The entitlements Pitwall must be signed with on macOS, checked in the
# entitlements file (no argument, or a .plist) or in what was signed (an
# .app or a program). Without them macOS kills Pitwall as soon as a tile
# starts ("Code Signature Invalid"): Wasmtime makes the memory it compiled
# a tile into executable, which the hardened runtime allows only with
# allow-unsigned-executable-memory (docs/spec/tile-sdk/runtime.md §2.6).
#
#   scripts/check-entitlements.sh                      # the file
#   scripts/check-entitlements.sh target/package/Pitwall.app
set -eu

KEYS="com.apple.security.cs.allow-unsigned-executable-memory"
PLIST="$(cd "$(dirname "$0")/.." && pwd)/crates/pitwall-app/packaging/macos/entitlements.plist"

target=${1:-$PLIST}
case "$target" in
  *.plist)
    what=$target
    [ -f "$target" ] || { echo "check-entitlements: no file $target" >&2; exit 1; }
    xml=$(cat "$target")
    ;;
  *)
    what="the signature of $target"
    xml=$(codesign -d --entitlements - --xml "$target" 2>/dev/null) || {
      echo "check-entitlements: $target isn't signed (codesign -d failed)" >&2
      exit 1
    }
    ;;
esac
flat=$(printf '%s' "$xml" | tr -d ' \t\r\n')
for k in $KEYS; do
  case "$flat" in
    *"<key>$k</key><true/>"*) ;;
    *)
      echo "check-entitlements: $what doesn't set $k to true." >&2
      echo "  Without it macOS kills Pitwall when a tile starts (Wasmtime's compiled code can't run under the hardened runtime)." >&2
      echo "  Keep it in crates/pitwall-app/packaging/macos/entitlements.plist and sign the app with --entitlements." >&2
      exit 1
      ;;
  esac
done
echo "check-entitlements: $what has $KEYS"
