#!/bin/sh
# Install a built Pitwall.app into /Applications, signed with the local
# "Pitwall Local Signing" identity so macOS privacy grants (e.g. Desktop
# access) survive rebuilds: the designated requirement is the bundle id plus
# that certificate, not a per-build hash.
#
# Usage: scripts/install-local.sh [path/to/Pitwall.app]
# Default source: target/release/bundle/macos/Pitwall.app (pnpm tauri build --bundles app)
#
# Agents keep running: Pitwall is quit gracefully (like ⌘Q), their terminals
# live in pitwall-hold processes, and the new app re-attaches on launch.
set -eu

IDENTITY="${PITWALL_SIGN_IDENTITY:-Pitwall Local Signing}"
SRC="${1:-$(cd "$(dirname "$0")/.." && pwd)/target/release/bundle/macos/Pitwall.app}"
DEST=/Applications/Pitwall.app

[ -d "$SRC" ] || { echo "no app bundle at $SRC (build with: pnpm tauri build --bundles app)" >&2; exit 1; }
security find-identity -p codesigning | grep -q "\"$IDENTITY\"" || {
  echo "signing identity \"$IDENTITY\" not found in the keychain" >&2
  exit 1
}

# Quit the running app gracefully and wait for it to exit.
pid=$(pgrep -f "Pitwall.app/Contents/MacOS/pitwall\$" | head -1 || true)
if [ -n "$pid" ]; then
  osascript -e 'tell application id "dev.pitwall.app" to quit'
  i=0
  while kill -0 "$pid" 2>/dev/null; do
    i=$((i + 1))
    [ "$i" -gt 40 ] && { echo "Pitwall did not quit; not replacing it" >&2; exit 1; }
    sleep 0.25
  done
fi

rm -rf "$DEST"
ditto "$SRC" "$DEST"
# Sign every bundled helper (pitwall-hold, pitwall-cli, …) before the app itself,
# each with a stable identifier derived from its file name.
for bin in "$DEST"/Contents/MacOS/*; do
  name=$(basename "$bin")
  [ "$name" = pitwall ] && continue
  codesign --force --options runtime --timestamp=none --sign "$IDENTITY" --identifier "dev.pitwall.app.${name#pitwall-}" "$bin"
done
codesign --force --options runtime --timestamp=none --sign "$IDENTITY" --identifier dev.pitwall.app "$DEST"
codesign --verify --deep --strict "$DEST"
echo "installed and signed: $DEST"
open "$DEST"
