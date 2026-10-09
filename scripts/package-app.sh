#!/usr/bin/env bash
# Packages Pitwall (crates/pitwall-app, the GPUI app; binary `pitwall`):
# builds the app and its sidecars (pitwall-hold, pitwall-cli, and
# pitwall-hook on Windows) in release mode and bundles them, with the
# Race Engineer's skills (skills/pitwall, skills/race-engineer;
# docs/spec/engineer.md) as resources (docs/spec/gpui/packaging.md).
# .github/workflows/release.yml runs it.
#
#   scripts/package-app.sh                       # this OS's default formats
#   scripts/package-app.sh --universal           # macOS: arm64 + x86_64 (CI)
#   scripts/package-app.sh --runtime-shaders     # macOS without the Metal toolchain
#   scripts/package-app.sh --formats app         # only some formats
#
#   macOS    app,dmg        own steps below: .app, codesign, hdiutil
#   Linux    appimage,deb   cargo-packager
#   Windows  nsis,wix       cargo-packager (run from Git Bash)
#
# Output: target/package/ (or --out DIR): the bundles plus bin/ with the
# staged binaries. Nothing is installed anywhere.
#
# Channel (PITWALL_CHANNEL): "stable" (default) builds "Pitwall", bundle id
# dev.pitwall.app, package name pitwall: the released app (the same identity
# the Tauri app had up to v0.1.x, so macOS privacy grants, the Dock entry
# and the install folder carry over). "preview" builds "Pitwall Preview" /
# dev.pitwall.app.preview / pitwall-preview, a dev bundle that installs next
# to the released one without sharing its permissions or notification
# settings (run it with PITWALL_HOME set to its own folder).
#
# The .dmg is named like the release asset: Pitwall_<version>_<arch>.dmg
# ("Pitwall_Preview_…" for the preview channel).
#
# macOS signing (all optional; ad-hoc without them):
#   APPLE_SIGNING_IDENTITY   "Developer ID Application: Name (TEAMID)" (in a keychain)
#   APPLE_ID, APPLE_PASSWORD (app-specific), APPLE_TEAM_ID   notarize + staple
# Release builds precompile GPUI's Metal shaders: they need Xcode's Metal
# toolchain (`xcodebuild -downloadComponent MetalToolchain` on Xcode 26).
#
# Windows signing (optional; unsigned without it):
#   WINDOWS_CERTIFICATE_THUMBPRINT   a code-signing certificate in CurrentUser\My;
#   signtool signs the sidecars here, cargo-packager the app and installers.
#
# Needs: cargo (rust-toolchain.toml), and on Linux/Windows `cargo packager`
# (cargo install cargo-packager --locked --version 0.11.8).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

die() { echo "package-app: $*" >&2; exit 1; }

universal=false
runtime_shaders=false
formats=""
out="${CARGO_TARGET_DIR:-$root/target}/package"
while [ $# -gt 0 ]; do
  case "$1" in
    --universal) universal=true ;;
    --runtime-shaders) runtime_shaders=true ;;
    --formats) formats=${2:?}; shift ;;
    --out) out=${2:?}; shift ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
  shift
done

case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) die "unsupported OS: $(uname -s)" ;;
esac
if [ -z "$formats" ]; then
  case $os in macos) formats=app,dmg ;; linux) formats=appimage,deb ;; windows) formats=nsis,wix ;; esac
fi
$universal && [ $os != macos ] && die "--universal is macOS only"

case "${PITWALL_CHANNEL:-stable}" in
  preview) product="Pitwall Preview"; bundle_id=dev.pitwall.app.preview; pkg_name=pitwall-preview ;;
  stable) product="Pitwall"; bundle_id=dev.pitwall.app; pkg_name=pitwall ;;
  *) die "PITWALL_CHANNEL must be preview or stable" ;;
esac
# The first `version = ` of [package] (scripts/release.sh bumps it).
version=$(awk '/^\[/ { pkg = ($0 == "[package]") } pkg && /^version = / { gsub(/"/, "", $3); print $3; exit }' crates/pitwall-app/Cargo.toml)
[ -n "$version" ] || die "no version in crates/pitwall-app/Cargo.toml"
icons="$root/crates/pitwall-app/packaging/icons"
exe=""
[ $os = windows ] && exe=.exe
sidecars="pitwall-hold pitwall-cli"
[ $os = windows ] && sidecars="$sidecars pitwall-hook"
# The Race Engineer's files: found at run time by crates/pitwall-app/src/engineer
# (macOS: Contents/Resources/skills; Linux: /usr/lib/<package>/skills;
# Windows: skills/ next to the app).
skill_dirs="pitwall race-engineer"
for d in $skill_dirs; do [ -f "skills/$d/SKILL.md" ] || [ -f "skills/$d/ENGINEER.md" ] || die "skills/$d is missing"; done

host=$(rustc -vV | sed -n 's/^host: //p')
if $universal; then targets="aarch64-apple-darwin x86_64-apple-darwin"; arch_label=universal
else targets=$host; arch_label=${host%%-*}; fi

# --- build -------------------------------------------------------------------
app_features="--no-default-features"
if $runtime_shaders; then
  app_features=""
  [ $os = macos ] && echo "package-app: runtime shaders (dev only; release bundles precompile them)" >&2
elif [ $os = macos ] && ! xcrun -sdk macosx metal -v >/dev/null 2>&1; then
  die "Xcode's Metal toolchain is missing: run 'xcodebuild -downloadComponent MetalToolchain', or pass --runtime-shaders for a local test bundle"
fi

target_dir="${CARGO_TARGET_DIR:-$root/target}"
# The channel's identity, baked into the app (platform::BUNDLE_ID): the
# Windows AppUserModelID its toasts name must be the installers'.
export PITWALL_BUNDLE_ID=$bundle_id
for t in $targets; do
  sidecar_args=""
  for s in $sidecars; do sidecar_args="$sidecar_args -p $s --bin $s"; done
  # Two invocations: --no-default-features must only reach pitwall-app.
  # shellcheck disable=SC2086
  cargo build --release --locked --target "$t" $sidecar_args
  # shellcheck disable=SC2086
  cargo build --release --locked --target "$t" -p pitwall-app --bin pitwall $app_features
done

# --- stage the binaries --------------------------------------------------------
bin="$out/bin"
rm -rf "$bin"
mkdir -p "$bin"
for b in pitwall $sidecars; do
  if $universal; then
    lipo -create -output "$bin/$b" "$target_dir/aarch64-apple-darwin/release/$b" "$target_dir/x86_64-apple-darwin/release/$b"
    lipo -info "$bin/$b"
  else
    cp "$target_dir/$targets/release/$b$exe" "$bin/$b$exe"
  fi
done

# --- macOS: .app and .dmg ------------------------------------------------------
macos_sign() { # <path> — Developer ID (hardened runtime) or ad-hoc
  local identity=${APPLE_SIGNING_IDENTITY:--}
  if [ "$identity" = - ]; then
    codesign --force --sign - "$1"
  else
    codesign --force --sign "$identity" --options runtime --timestamp \
      --entitlements crates/pitwall-app/packaging/macos/entitlements.plist "$1"
  fi
}

macos_notarize() { # <path to .app or .dmg>; staples it
  local subject=$1 upload=$1
  if [ "${APPLE_SIGNING_IDENTITY:--}" = - ] || [ -z "${APPLE_ID:-}" ] || [ -z "${APPLE_PASSWORD:-}" ] || [ -z "${APPLE_TEAM_ID:-}" ]; then
    return 0
  fi
  if [ -d "$subject" ]; then
    upload="$out/notarize.zip"
    ditto -c -k --keepParent "$subject" "$upload"
  fi
  xcrun notarytool submit "$upload" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
  xcrun stapler staple "$subject"
  [ "$upload" = "$subject" ] || rm -f "$upload"
}

if [ $os = macos ]; then
  app="$out/$product.app"
  case ",$formats," in *,app,*|*,dmg,*) ;; *) die "macOS formats: app, dmg" ;; esac
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  for b in pitwall $sidecars; do cp "$bin/$b" "$app/Contents/MacOS/$b"; done
  sed -e "s|@PRODUCT_NAME@|$product|g" -e "s|@BUNDLE_ID@|$bundle_id|g" \
      -e "s|@VERSION@|$version|g" -e "s|@EXECUTABLE@|pitwall|g" \
      crates/pitwall-app/packaging/macos/Info.plist > "$app/Contents/Info.plist"
  plutil -lint "$app/Contents/Info.plist" >/dev/null
  printf 'APPL????' > "$app/Contents/PkgInfo"
  cp "$icons/icon.icns" "$app/Contents/Resources/icon.icns"
  mkdir -p "$app/Contents/Resources/skills"
  for d in $skill_dirs; do cp -R "skills/$d" "$app/Contents/Resources/skills/$d"; done
  # Inside out: the sidecars first, then the bundle (which seals the main binary).
  for s in $sidecars; do macos_sign "$app/Contents/MacOS/$s"; done
  macos_sign "$app"
  codesign --verify --deep --strict --verbose=2 "$app"
  macos_notarize "$app"
  echo "package-app: $app"

  case ",$formats," in *,dmg,*)
    dmg="$out/${product// /_}_${version}_${arch_label}.dmg"
    stage="$out/dmg-root"
    rm -rf "$stage" "$dmg"
    mkdir -p "$stage"
    ditto "$app" "$stage/$product.app"
    ln -s /Applications "$stage/Applications"
    # A plain compressed image: no Finder scripting, so it runs headless.
    hdiutil create -quiet -volname "$product" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg"
    rm -rf "$stage"
    if [ "${APPLE_SIGNING_IDENTITY:--}" != - ]; then
      codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$dmg"
      macos_notarize "$dmg"
    fi
    echo "package-app: $dmg"
  ;; esac
  exit 0
fi

# --- Linux and Windows: cargo-packager ------------------------------------------
cargo packager --version >/dev/null 2>&1 || die "cargo-packager is missing: cargo install cargo-packager --locked --version 0.11.8"

native() { # a path cargo-packager (a native Windows program) can read
  if [ $os = windows ]; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

windows_cfg=""
if [ $os = windows ] && [ -n "${WINDOWS_CERTIFICATE_THUMBPRINT:-}" ]; then
  timestamp_url=http://timestamp.digicert.com
  signtool=$(ls -d "/c/Program Files (x86)/Windows Kits/10/bin/"10.*/x64/signtool.exe 2>/dev/null | sort -V | tail -1)
  [ -n "$signtool" ] || die "signtool.exe not found (Windows SDK)"
  for s in $sidecars; do
    "$signtool" sign //sha1 "$WINDOWS_CERTIFICATE_THUMBPRINT" //fd sha256 //tr "$timestamp_url" //td sha256 "$(native "$bin/$s.exe")"
  done
  windows_cfg="\"windows\": { \"certificateThumbprint\": \"$WINDOWS_CERTIFICATE_THUMBPRINT\", \"digestAlgorithm\": \"sha256\", \"timestampUrl\": \"$timestamp_url\" },"
fi

resources=""
for d in $skill_dirs; do
  resources="$resources${resources:+, }{ \"src\": \"$(native "$root/skills/$d")\", \"target\": \"skills/$d\" }"
done
binaries='{ "path": "pitwall", "main": true }'
for s in $sidecars; do binaries="$binaries, { \"path\": \"$s\" }"; done
cfg="$out/packager.json"
# GPUI's runtime libraries instead of WebKitGTK; curl as the Tauri .deb had
# (the hook script posts with it).
# "name" set: cargo-packager 0.11.8 otherwise looks for a Cargo.toml by
# changing into the config *file* path, which fails.
cat > "$cfg" <<EOF
{
  "name": "pitwall",
  "productName": "$product",
  "version": "$version",
  "identifier": "$bundle_id",
  "description": "Hosts terminal coding agents and shows who needs you",
  "homepage": "https://aleksandrejavakhishvili.github.io/pitwall/",
  "publisher": "Pitwall",
  "category": "DeveloperTool",
  "licenseFile": "$(native "$root/LICENSE")",
  "outDir": "$(native "$out")",
  "binariesDir": "$(native "$bin")",
  "binaries": [ $binaries ],
  "resources": [ $resources ],
  "icons": [
    "$(native "$icons/32x32.png")",
    "$(native "$icons/128x128.png")",
    "$(native "$icons/128x128@2x.png")",
    "$(native "$icons/icon.ico")"
  ],
  "deb": {
    "packageName": "$pkg_name",
    "section": "devel",
    "depends": ["curl", "libxkbcommon0", "libxkbcommon-x11-0", "libxcb1", "libx11-xcb1", "libwayland-client0", "libvulkan1", "libfontconfig1", "libfreetype6", "libzstd1"]
  },
  $windows_cfg
  "nsis": { "installMode": "currentUser", "displayLanguageSelector": false }
}
EOF
cargo packager --config "$(native "$cfg")" --formats "$formats" --out-dir "$(native "$out")"
echo "package-app: $(ls "$out")"
