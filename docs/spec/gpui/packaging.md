# GPUI port: packaging, signing, CI and tests

Today `pnpm tauri build` (`.github/workflows/release.yml`) makes the macOS
universal `.app`/`.dmg`, the Linux AppImage and `.deb`, and the Windows NSIS
`.exe` and `.msi`, with three sidecars from `src-tauri/build.rs`:
`pitwall-hold` (terminal holder), `pitwall-cli` (linked as `pitwall`) and,
on Windows, `pitwall-hook` (the hook relay; Unix uses an sh script). Without
Tauri the same artifacts come from `cargo` plus a packager.

## Running during development

```sh
cargo build -p pitwall-app -p pitwall-hold -p pitwall-cli   # holder + CLI next to the app
PITWALL_HOME=/tmp/pw-preview cargo run -p pitwall-app       # its own data folder
target/debug/pitwall-cli --socket /tmp/pw-preview/run/pitwalld.sock agent new --kind shell --name demo
```

Keep `PITWALL_HOME` short on macOS: socket paths under it (holders,
`run/hold/<uuid>.sock`) must stay under the 104-byte Unix socket limit.

`pitwall-app` finds the holder like `src-tauri` does: `$PITWALL_HOLD_BIN`,
next to its executable, one level up. That covers every bundle, since the
sidecars sit next to the app binary in all of them; only an AppImage needs
more, so `packaging::stable_sidecar` copies the holder out of the AppImage
mount into `<data folder>/bin/` first (ported from
`src-tauri/src/platform/mod.rs`). It has no `build.rs` building the
sidecars (the Tauri crate's nested cargo build would run twice per
workspace build): `scripts/package-app.sh` builds them.

## Shaders

GPUI compiles its Metal shaders at build time with Xcode's `metal` tool,
which Xcode 26 ships as a separate download (`xcodebuild -downloadComponent
MetalToolchain`). `pitwall-app` turns on gpui's `runtime_shaders` by default
(feature `runtime-shaders`), so `cargo build`, tests and clippy need no Metal
toolchain; shaders compile when the first window opens (a few ms). Release
builds use `--no-default-features` (precompiled shaders) with the toolchain
installed on the CI runner: `release-gpui.yml` checks `xcrun -sdk macosx metal
-v` and runs `xcodebuild -downloadComponent MetalToolchain` when it fails
(Xcode 16 includes it, Xcode 26 doesn't), and afterwards checks the binary
holds no Metal source (`using namespace metal;` is only embedded with
runtime shaders). The `.metallib` is architecture-independent, so one
toolchain serves both halves of the universal build. Locally without the
toolchain, `scripts/package-app.sh --runtime-shaders` makes a test bundle. Windows compiles HLSL with `fxc.exe` from the
Windows SDK at build time (gpui's build script finds it; `GPUI_FXC_PATH`
overrides). Linux uses WGSL through blade/Vulkan, checked at build time.

## Bundles

`scripts/package-app.sh` builds and bundles on each OS (release profile,
`--locked`, sidecars in the same target dir, staged in `target/package/bin/`):

- **macOS: own steps, no packager.** The `.app` is a folder, a filled-in
  `crates/pitwall-app/packaging/macos/Info.plist`, `PkgInfo` and the icon;
  universal binaries are `lipo`s of the aarch64 and x86_64 builds; `codesign`
  signs the sidecars, then the bundle; the `.dmg` is `hdiutil create
  -format UDZO` of the app plus an `/Applications` link. Owning these steps
  keeps the inside-out signing order, the universal sidecars and
  notarization explicit (Tauri hid them), and the `.dmg` needs no Finder
  AppleScript (cargo-packager's and Tauri's `create-dmg` drive Finder, which
  is flaky headless and opens windows on a developer's desktop). Zed bundles
  macOS with its own script too.
- **Linux and Windows: [`cargo-packager`](https://github.com/crabnebula-dev/cargo-packager)
  0.11.8** (MIT OR Apache-2.0; from the Tauri bundler's authors, no Tauri
  runtime). The script writes its config (`target/package/packager.json`:
  per-OS sidecar list and channel, so no static
  `[package.metadata.packager]` section) and runs `cargo packager --formats
  appimage,deb` / `nsis,wix`. It installs the sidecars as extra `binaries`
  next to the main one, makes the `.desktop` file, and both Windows
  installers set the shortcut's AppUserModelID to the bundle id. Tools it
  downloads at packaging time, none linked into Pitwall: linuxdeploy and the
  AppImage runtime (MIT), NSIS (zlib/libpng licence), WiX 3.11 (MS-RL; it
  covers WiX itself, not the `.msi` it builds), the same ones Tauri's
  bundler uses today.

Channel (`PITWALL_CHANNEL`): until the switch-over, bundles are **preview**
builds, "Pitwall Preview" with bundle id `dev.pitwall.app.preview` and
package name `pitwall-preview`, so they install next to the Tauri app
without sharing its install folder (Windows), TCC permissions, notification
settings or Dock identity (macOS). `stable` gives "Pitwall" /
`dev.pitwall.app` / `pitwall`. The preview `.deb` still can't be installed
next to the Tauri `.deb`: both own `/usr/bin/pitwall-hold` and
`/usr/bin/pitwall-cli` (cargo-packager has no `Conflicts:` for debs); use
the AppImage side by side.

| OS | Artifacts | Sidecars at | Notes |
|---|---|---|---|
| macOS | `Pitwall.app`, `.dmg`, universal (`lipo` of aarch64 + x86_64, as today) | `Contents/MacOS/` next to the app binary | `Info.plist` with the usage strings of `src-tauri/Info.plist` (TCC); no URL schemes or document types (none today); `LSMinimumSystemVersion` 10.15.7 (GPUI's shaders); bundle id `dev.pitwall.app` (preview builds: `.preview`) |
| Linux | AppImage, `.deb` (Ubuntu 22.04 runner for the glibc floor) | `usr/bin/`; AppImage copies the holder out of the mount (`packaging::stable_sidecar`) | `.desktop` file, icons, `Depends:` curl (hook script), libxkbcommon0, libxkbcommon-x11-0, libxcb1, libx11-xcb1, libwayland-client0, libvulkan1, libfontconfig1, libfreetype6, libzstd1 instead of WebKitGTK (CI prints `ldd` of the app to check the list) |
| Windows | NSIS `.exe`, `.msi` | install dir next to `pitwall.exe` (incl. `pitwall-hook.exe`) | sets the AppUserModelID (notifications, taskbar overlay); no WebView2 bootstrapper any more |

The app binary is renamed `pitwall` at the switch (phase 9); until then it
is `pitwall-app` and is not shipped (bundles are workflow artifacts only).

## Signing

Unchanged in substance from release.yml:

- macOS: ad-hoc without secrets; with them, Developer ID `codesign
  --options runtime --timestamp` on the sidecars first, then the app, with
  the hardened-runtime entitlements Tauri adds today (made explicit in an
  `entitlements.plist`); `notarytool submit --wait`, `stapler staple` for the
  app and the `.dmg`. Runtime shaders need no JIT entitlement (Metal compiles
  from source through the driver), but release builds precompile anyway.
- Windows: Authenticode with `signtool` on every `.exe` (app, sidecars,
  installer) when the certificate secret is present; unsigned otherwise, as
  now.
- Linux: none (checksums, as now).

## CI

- `ci.yml` (done in phase 0): the macOS, Linux and Windows Rust jobs build
  and clippy the whole workspace, so `pitwall-app` is covered on all three.
  The Linux job installs GPUI's system libraries (X11/xcb, xkbcommon,
  Wayland, fontconfig/freetype, Vulkan loader, zstd, ALSA, GLib) next to
  WebKitGTK's. macOS needs no Metal toolchain (runtime shaders). Windows runs
  `cargo test -p pitwall-app` as an informational step until its first green
  run, then it becomes required.
- Workspace builds get heavier: gpui adds about 400 crates. On a warm cache
  `cargo clippy --workspace --all-targets` took 24 s and `cargo test
  --workspace` 75 s on an M-series Mac; a cold build of gpui is a few
  minutes. If this hurts, the GPUI crates can move to their own CI job with
  their own cache key; not needed yet.
- `release-gpui.yml` (phase 8): macOS (universal), Linux and Windows jobs
  run `scripts/package-app.sh` and upload the bundles plus a
  `SHA256SUMS.txt` as workflow artifacts. It runs on demand and on pushes to
  `gpui/**` that touch the packaging files. It **never publishes**: no
  release job, and its token is `contents: read`. Signing uses the
  release.yml secrets when present (macOS: certificate imported into a
  temporary keychain, `APPLE_SIGNING_IDENTITY` and the notarization
  variables handed to the script; Windows: certificate imported, its
  thumbprint passed as `WINDOWS_CERTIFICATE_THUMBPRINT`; signtool signs the
  sidecars, cargo-packager the app and installers). Windows sets
  `GPUI_FXC_PATH` to the newest SDK's `fxc.exe`. In phase 9 its jobs
  replace release.yml's build jobs and release.yml's `publish` job takes
  their artifacts unchanged.

## Tests

- Pure logic (grouping, sorting, data folder choice, window state, theme
  tokens vs tokens.css): plain unit tests (phase 0 has them).
- Entities and the bridge: `#[gpui::test]` with `TestAppContext` (gpui's
  `test-support` feature, a dev-dependency): send `AppEvent`s, `run_until_parked`,
  assert on the store (phase 0: `agents::tests::the_store_follows_engine_events`).
- Views: `VisualTestContext` (`cx.add_window_view`) to dispatch actions,
  simulate keystrokes and clicks and assert on view state; one test per
  in-house component and per screen flow from phase 1.
- Engine hosting: a real `Host` on a temp folder (phase 0:
  `host::tests::hosts_the_engine_in_a_temp_folder` checks the CLI socket
  opens and closes).
- End to end: the bench script (`scripts/bench.sh`, docs/spec/perf.md) gets
  a GPUI mode in phase 8: the same commands via a `PITWALL_BENCH` module that
  dispatches actions, plus memory/CPU numbers against the Tauri app's.
- Visual checks: screenshots of a temp-folder instance with made-up agents,
  never the user's data.

## Updates

The Tauri app has no updater and no version check (no `tauri-plugin-updater`);
users update by downloading a release, or with the Homebrew cask
(`packaging/homebrew/pitwall.rb`, `livecheck` on GitHub releases). The GPUI
app keeps that until the switch-over; the plan after it, in two steps:

1. **Version check** (small, no new trust): at most once a day, and from
   Settings → "Check now", `GET
   https://api.github.com/repos/<owner>/<repo>/releases/latest` (no auth,
   no identifiers sent), compare its `tag_name` with `CARGO_PKG_VERSION`
   (semver; pre-releases only when the running build is one), and show
   "Pitwall X.Y.Z is available" in the status strip with a link to the
   release page. A setting turns it off; `PITWALL_NO_UPDATE_CHECK=1` too
   (CI, bench). Homebrew installs (the app under the Caskroom path) point to
   `brew upgrade --cask pitwall` instead.
2. **In-app update** (after Developer ID signing exists): the
   [`cargo-packager-updater`](https://crates.io/crates/cargo-packager-updater)
   crate (MIT OR Apache-2.0, same authors) reads a `latest.json` manifest
   that release.yml's `publish` job uploads, downloads the `.app.tar.gz`
   (macOS), the AppImage (Linux) or the NSIS installer (Windows), checks its
   minisign signature (`cargo packager --private-key`, key in a repository
   secret, public key compiled into the app) and installs it. Agents survive
   it: they run in holders, so the update is "quit (agents keep running),
   replace, relaunch" and the new app re-attaches as after any restart.
   `.deb` installs are left to the package manager (step 1 only). Running
   holders keep their binary: on macOS the bundle is replaced as a whole
   and old holders keep the unlinked file, the AppImage copy is replaced by
   rename (`stable_sidecar`). Windows locks a running `pitwall-hold.exe`,
   so the installer (manual or updater) must rename it aside before
   writing the new one; open item, the Tauri NSIS installer has the same
   gap today.

