# GPUI port: packaging, signing, CI and tests

Up to v0.1.x `pnpm tauri build` made the macOS universal `.app`/`.dmg`,
the Linux AppImage and `.deb`, and the Windows NSIS `.exe` and `.msi`, with
three sidecars: `pitwall-hold` (terminal holder), `pitwall-cli` (linked as
`pitwall`) and, on Windows, `pitwall-hook` (the hook relay; Unix uses an sh
script). Since v0.2.0 (the switch-over) the same artifacts, under the same
release asset names, come from `cargo` plus a packager:
`scripts/package-app.sh`, run by `.github/workflows/release.yml`.

## Running during development

```sh
cargo build -p pitwall-app -p pitwall-hold -p pitwall-cli   # app (`pitwall`), holder and CLI side by side
PITWALL_HOME=/tmp/pw-preview cargo run -p pitwall-app       # its own data folder
target/debug/pitwall-cli --socket /tmp/pw-preview/run/pitwalld.sock agent new --kind shell --name demo
```

Keep `PITWALL_HOME` short on macOS: socket paths under it (holders,
`run/hold/<uuid>.sock`) must stay under the 104-byte Unix socket limit.

The app finds the holder as the Tauri app did: `$PITWALL_HOLD_BIN`,
next to its executable, one level up. That covers every bundle, since the
sidecars sit next to the app binary in all of them; only an AppImage needs
more, so `packaging::stable_sidecar` copies the holder out of the AppImage
mount into `<data folder>/bin/` first (ported from
`src-tauri/src/platform/mod.rs`). It has no `build.rs` building the
sidecars (a nested cargo build would run twice per workspace build):
`scripts/package-app.sh` builds them.

## Shaders

GPUI compiles its Metal shaders at build time with Xcode's `metal` tool,
which Xcode 26 ships as a separate download (`xcodebuild -downloadComponent
MetalToolchain`). `pitwall-app` turns on gpui's `runtime_shaders` by default
(feature `runtime-shaders`), so `cargo build`, tests and clippy need no Metal
toolchain; shaders compile when the first window opens (a few ms). Release
builds use `--no-default-features` (precompiled shaders) with the toolchain
installed on the CI runner: `release.yml` checks `xcrun -sdk macosx metal
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

Every bundle also carries the Race Engineer's skills (`skills/pitwall`,
`skills/race-engineer`; docs/spec/engineer.md): `Contents/Resources/skills`
on macOS, cargo-packager `resources` (target `skills/…`) on Linux and
Windows. `crates/pitwall-app/src/engineer` finds them at run time.

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

Channel (`PITWALL_CHANNEL`): **stable** (the default, and what
release.yml ships) is "Pitwall" with bundle id `dev.pitwall.app`, package
name `pitwall` and executable `pitwall`: the Tauri app's identity, so an
upgrade replaces it in /Applications and the Dock, the `.deb` upgrades the
`pitwall` package, and the Windows installers use the same product name and
install folder. macOS privacy grants follow the code signature's designated
requirement: they carry over with the same Developer ID (or the local
signing identity of `scripts/install-local.sh`), while ad-hoc signed builds
are tied to their own hash, as Tauri releases were. **preview** ("Pitwall
Preview", `dev.pitwall.app.preview`, `pitwall-preview`) is for dev bundles
that install next to the released app without sharing its TCC permissions,
notification settings or Dock identity; run them with their own
`PITWALL_HOME` (they share the default data folder otherwise). Both
channels' `.deb`s own `/usr/bin/pitwall*`, so only one installs at a time;
use the AppImage side by side.

| OS | Artifacts | Sidecars at | Notes |
|---|---|---|---|
| macOS | `Pitwall.app`, `.dmg`, universal (`lipo` of aarch64 + x86_64, as today) | `Contents/MacOS/` next to the app binary | `Info.plist` with the usage strings the Tauri app had (TCC); no URL schemes or document types (none today); `LSMinimumSystemVersion` 10.15.7 (GPUI's shaders); bundle id `dev.pitwall.app` (preview builds: `.preview`) |
| Linux | AppImage, `.deb` (Ubuntu 22.04 runner for the glibc floor) | `usr/bin/`; AppImage copies the holder out of the mount (`packaging::stable_sidecar`) | `.desktop` file, icons, `Depends:` curl (hook script), libxkbcommon0, libxkbcommon-x11-0, libxcb1, libx11-xcb1, libwayland-client0, libvulkan1, libfontconfig1, libfreetype6, libzstd1 instead of WebKitGTK (CI prints `ldd` of the app to check the list) |
| Windows | NSIS `.exe`, `.msi` | install dir next to `pitwall.exe` (incl. `pitwall-hook.exe`) | sets the AppUserModelID (notifications, taskbar overlay); no WebView2 bootstrapper any more |

The app binary is `pitwall` (the package is still `pitwall-app`:
`cargo run -p pitwall-app`).

## Signing

Unchanged in substance from the Tauri release workflow:

- macOS: ad-hoc without secrets; with them, Developer ID `codesign
  --options runtime --timestamp` on the sidecars first, then the app, with
  the hardened-runtime entitlements Tauri added (made explicit in an
  `entitlements.plist`); `notarytool submit --wait`, `stapler staple` for the
  app and the `.dmg`. Runtime shaders need no JIT entitlement (Metal compiles
  from source through the driver), but release builds precompile anyway.
- Windows: Authenticode with `signtool` on every `.exe` (app, sidecars,
  installer) when the certificate secret is present; unsigned otherwise, as
  now.
- Linux: none (checksums, as now).

## CI

- `ci.yml`: the macOS, Linux and Windows Rust jobs build and clippy the
  whole workspace, so `pitwall-app` is covered on all three. The Linux job
  installs GPUI's system libraries (X11/xcb, xkbcommon, Wayland,
  fontconfig/freetype, Vulkan loader, zstd, ALSA, GLib); WebKitGTK went
  with the Tauri jobs. The web job type-checks, builds and unit-tests the
  React web demo and builds the website. macOS needs no Metal toolchain (runtime shaders). Windows runs
  `cargo test -p pitwall-app` as an informational step until its first green
  run, then it becomes required.
- Workspace builds get heavier: gpui adds about 400 crates. On a warm cache
  `cargo clippy --workspace --all-targets` took 24 s and `cargo test
  --workspace` 75 s on an M-series Mac; a cold build of gpui is a few
  minutes. If this hurts, the GPUI crates can move to their own CI job with
  their own cache key; not needed yet.
- `release.yml` (since the switch; it absorbed phase 8's
  `release-gpui.yml`): macOS (universal), Linux and Windows (gated by the
  `RELEASE_WINDOWS` variable, shipped as a preview) jobs run
  `scripts/package-app.sh` with `PITWALL_CHANNEL=stable`, check the bundles
  (universal slices, signature, no runtime shaders, sidecars in the `.deb`
  and AppImage), rename them to the v0.1.x asset names
  (`Pitwall_<version>_universal.dmg`, `…_universal.app.tar.gz`,
  `…_amd64.AppImage`, `…_amd64.deb`, `…_x64-setup.exe`, `…_x64.msi`; the
  website's download page matches on these suffixes) and upload them with a
  `SHA256SUMS.txt`. A tag (or a manual run with a tag) adds the `publish`
  job, the only one with `contents: write`, which makes the GitHub release;
  pull requests that touch the packaging files and manual runs without a
  tag build only. One workflow means the bundles a pull request tests are
  exactly what a release ships. Signing uses the
  release.yml secrets when present (macOS: certificate imported into a
  temporary keychain, `APPLE_SIGNING_IDENTITY` and the notarization
  variables handed to the script; Windows: certificate imported, its
  thumbprint passed as `WINDOWS_CERTIFICATE_THUMBPRINT`; signtool signs the
  sidecars, cargo-packager the app and installers). Windows sets
  `GPUI_FXC_PATH` to the newest SDK's `fxc.exe`.
- Cross-platform check before the first push (round 2, local only): an
  Ubuntu 22.04 container with ci.yml's Linux packages (arm64; the runner is
  x86_64) ran `cargo clippy --workspace --all-targets -D warnings` and
  `cargo test --workspace` green, and `cargo xwin clippy --target
  x86_64-pc-windows-msvc --workspace --all-targets -D warnings` (with
  `PITWALL_SIDECAR_STUBS=1`) plus a debug link of pitwall-app, -hold, -cli
  and -hook for Windows passed (ring builds with clang-cl; no libgit2 in
  the tree). Found and fixed on the way: libc 0.2.190 removed Linux's
  `ENOATTR`, which xattr 0.2.3 (gpui → gpui_http_client → zed-async-tar)
  names, so gpui no longer compiled on Linux: the lockfile pins 0.2.189 and
  pitwall-app caps `libc < 0.2.190` until gpui drops xattr 0.2. Tests that
  commit through the app set a repo-local git identity (runners have none).
  The Windows tests were not run (no Windows host). Cross-checking Windows
  needs no `fxc.exe`: gpui compiles HLSL only in release builds on a
  Windows host.
- Running the app headless: gpui draws nothing under Xvfb with Mesa's
  lavapipe (black window; X11 software presentation), so the Linux smoke
  run uses Weston (13+, `xdg_wm_base` v2+; with a seat, so not the
  headless backend) nested in Xvfb, gpui on Wayland. Ubuntu 22.04's Weston
  9 is too old for gpui.
- The Linux packages name the launcher `pitwall.desktop` (after the
  binary); the window's app id and the badge's desktop id follow it
  (`platform::WINDOW_APP_ID`, `linux::DESKTOP_ID`). The channel's bundle id
  is baked in by `scripts/package-app.sh` (`PITWALL_BUNDLE_ID` →
  `platform::BUNDLE_ID`; `dev.pitwall.app` when unset), so Windows toasts
  name the AppUserModelID the installers set.

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

Pitwall has no updater and no version check (the Tauri app had none
either); users update by downloading a release, or with the Homebrew cask
(`packaging/homebrew/pitwall.rb`, `livecheck` on GitHub releases). The plan,
in two steps:

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

