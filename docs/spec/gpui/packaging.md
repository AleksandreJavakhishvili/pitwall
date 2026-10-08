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
next to its executable, one level up. It has no `build.rs` building the
sidecars yet (the Tauri crate's nested cargo build would run twice per
workspace build); phase 8 moves that sidecar build into one place shared by
both apps, or into the packaging script.

## Shaders

GPUI compiles its Metal shaders at build time with Xcode's `metal` tool,
which Xcode 26 ships as a separate download (`xcodebuild -downloadComponent
MetalToolchain`). `pitwall-app` turns on gpui's `runtime_shaders` by default
(feature `runtime-shaders`), so `cargo build`, tests and clippy need no Metal
toolchain; shaders compile when the first window opens (a few ms). Release
builds use `--no-default-features` (precompiled shaders) with the toolchain
installed on the CI runner. Windows compiles HLSL with `fxc.exe` from the
Windows SDK at build time (gpui's build script finds it; `GPUI_FXC_PATH`
overrides). Linux uses WGSL through blade/Vulkan, checked at build time.

## Bundles

Use [`cargo-packager`](https://github.com/crabnebula-dev/cargo-packager)
(MIT OR Apache-2.0; from the Tauri bundler's authors, no Tauri runtime): one
`[package.metadata.packager]` section in `crates/pitwall-app/Cargo.toml`
makes `.app` + `.dmg`, AppImage + `.deb`, NSIS + WiX `.msi`, with
`binaries` for the sidecars and the existing icons from `src-tauri/icons/`.
If it falls short on one format, that format gets a small script
(`scripts/bundle-<os>.sh`) instead; Zed bundles with its own scripts too.

| OS | Artifacts | Sidecars at | Notes |
|---|---|---|---|
| macOS | `Pitwall.app`, `.dmg`, universal (`lipo` of aarch64 + x86_64, as today) | `Contents/MacOS/` next to the app binary | `Info.plist` from `src-tauri/Info.plist` (usage strings for TCC); bundle id `dev.pitwall.app` (preview builds: `.preview`) |
| Linux | AppImage, `.deb` (Ubuntu 22.04 runner for the glibc floor) | `usr/bin/`; AppImage copies them out of the mount (`stable_sidecar` logic moves from `src-tauri/src/platform/mod.rs` to core or the app) | `.desktop` file, icons, `Depends:` on libxkbcommon-x11-0, libwayland-client0, libvulkan1, libfontconfig1 instead of WebKitGTK |
| Windows | NSIS `.exe`, `.msi` | install dir next to `pitwall.exe` (incl. `pitwall-hook.exe`) | sets the AppUserModelID (notifications, taskbar overlay); no WebView2 bootstrapper any more |

The app binary is renamed `pitwall` at the switch (phase 9); until then it
is `pitwall-app` and is not shipped.

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
- release.yml: a parallel `pitwall-app` matrix (unsigned artifacts uploaded
  to the run, not published) from phase 8; it replaces the Tauri jobs in
  phase 9.

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
