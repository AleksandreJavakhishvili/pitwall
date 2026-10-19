#!/bin/sh
# Install a built Pitwall.app into /Applications, signed with the local
# "Pitwall Local Signing" identity so macOS privacy grants (e.g. Desktop
# access) survive rebuilds: the designated requirement is the bundle id plus
# that certificate, not a per-build hash.
#
# Usage: scripts/install-local.sh [path/to/Pitwall.app]
# Default source: target/package/Pitwall.app, as scripts/package-app.sh builds
# it (stable channel; `scripts/package-app.sh --formats app`, or add
# `--runtime-shaders` without Xcode's Metal toolchain).
#
# Agents keep running: Pitwall is quit gracefully (like ⌘Q), their terminals
# live in pitwall-hold processes, and the new app re-attaches on launch.
set -eu

IDENTITY="${PITWALL_SIGN_IDENTITY:-Pitwall Local Signing}"
SRC="${1:-${CARGO_TARGET_DIR:-$(cd "$(dirname "$0")/.." && pwd)/target}/package/Pitwall.app}"
DEST=/Applications/Pitwall.app

[ -d "$SRC" ] || { echo "no app bundle at $SRC (build with: scripts/package-app.sh --formats app)" >&2; exit 1; }
# Only the stable channel replaces /Applications/Pitwall.app (a local build
# says dev.pitwall.app.unsigned until it's signed below; package-app.sh).
id=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$SRC/Contents/Info.plist")
case "$id" in
  dev.pitwall.app | dev.pitwall.app.unsigned) ;;
  *) echo "$SRC is $id, not dev.pitwall.app (build with PITWALL_CHANNEL=stable)" >&2; exit 1 ;;
esac
security find-identity -p codesigning | grep -q "\"$IDENTITY\"" || {
  echo "signing identity \"$IDENTITY\" not found in the keychain" >&2
  exit 1
}

# Quit the running app gracefully and wait for it to exit.
pid=$(pgrep -f "^$DEST/Contents/MacOS/pitwall\$" | head -1 || true)
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
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier dev.pitwall.app" "$DEST/Contents/Info.plist"
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
# Terminals still running a holder from inside an older bundle lose macOS
# file access (Desktop, Full Disk Access); newer ones run a copy outside it.
old=$(pgrep -f "^$DEST/Contents/MacOS/pitwall-hold " | wc -l | tr -d ' ')
if [ "$old" -gt 0 ]; then
  echo "note: $old terminal(s) were started by an older Pitwall from inside the app; if one says \"Operation not permitted\", restart it."
fi
open "$DEST"
