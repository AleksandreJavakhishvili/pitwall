# Pitwall's patches to GPUI

Every change Pitwall makes to the vendored crates (README.md) is marked
`Pitwall patch` in the source (`grep -rn "Pitwall patch" third_party/zed`):

- `gpui_apple/src/metal_renderer.rs` — the path textures (window-sized,
  one 4× multisampled) are made on the first frame that draws a path, not
  with the window and again on every resize: a window that draws no path
  has none. `update_drawable_size` only drops textures of the wrong size;
  `render_frame` makes them when the scene has paths.

- `gpui/src/window.rs` — `Window::on_frame_drawn` (a listener run after
  every frame the window draws, until it returns `false`),
  `Window::rendered_hitboxes` (the drawn frame's hitboxes in paint order)
  and `Window::rendered_tooltip_bounds`. The agent pane's web preview is a
  native view over GPUI's drawing: after each frame it follows its
  element's bounds, hides when its element wasn't drawn, and swaps itself
  for a snapshot while anything drawn later (a menu, a dialog, the palette,
  a tooltip) covers it (`crates/pitwall-app/src/main_screen/preview.rs`,
  `peer_term.rs`). GPUI has no equivalent.

- `gpui/src/view.rs` — `ViewElement::keep_inner_caches`: a cached view
  that renders again leaves the cached views inside it that weren't
  notified to replay their last frame, as they do inside an uncached view
  (GPUI renders them all, as a window refresh does). The main screen is
  a cached view while an overlay is open over it (Settings, the palette);
  a terminal's frame behind it then renders the screen without
  re-rendering its sidebar, top bar and other terminals
  (`crates/pitwall-app/src/ui/mod.rs`, docs/spec/perf.md).

## Now upstream (dropped)

Patches Pitwall made to GPUI 0.2.2 (crates.io) that Zed's GPUI has since
made itself, so they are no longer applied:

- The 4× multisampled path texture is memoryless on Apple GPUs
  (`gpui_apple/src/metal_renderer.rs`, `update_path_intermediate_textures`).
  Making the path textures lazily was the other half of that patch and is
  still applied (above).

- The Metal layer is opaque when the window is
  (`WindowBackgroundAppearance::Opaque`): `MetalRenderer::update_transparency`
  and the layer's creation in `gpui_apple/src/metal_renderer.rs`.

- The folder/file picker (`NSOpenPanel`) is held until it answers
  (`gpui_macos/src/platform.rs`, `prompt_for_paths`): the panel is a
  `Retained` that the completion handler keeps.

- Windows: the DirectComposition target is created behind the window's
  child windows, `CreateTargetForHwnd(hwnd, false)`
  (`gpui_windows/src/directx_renderer.rs`), so the in-app preview's
  WebView2 child window shows.

- Linux X11: a window's raw window handle is its XID as an
  `XcbWindowHandle` (`gpui_linux/src/linux/x11/window.rs`), no longer
  `unimplemented!()`; the in-app preview reparents its WebKitGTK window
  into it.

Also in this GPUI and needed by Pitwall: macOS frame pacing with
`CAMetalDisplayLink` (Zed PR #65472, "Fix animations running at 30 fps on
M5 Macs": `gpui_apple/src/display_link.rs`, `gpui_macos/src/window.rs`).

To move to a newer GPUI: re-vendor (README.md) and re-apply the patches
above, dropping any that upstream has made.
