# GPUI 0.2.2, patched

The `gpui` 0.2.2 crate from crates.io (Apache-2.0, `LICENSE-APACHE`), used
through `[patch.crates-io]` in the workspace `Cargo.toml`. Examples and tests
are left out. Every change is marked `Pitwall patch` in the source:

- `src/platform/mac/metal_renderer.rs` — the 4× multisampled path texture
  is memoryless on Apple GPUs (it is only resolved within its pass): ~110 MB
  less per full-screen Retina window, at no cost to drawing. Both path
  textures are made on the first frame that draws a path instead of with
  the window.

- `src/platform/mac/platform.rs` — the folder/file picker (`NSOpenPanel`)
  is held until it answers. Autoreleased, it was let go of before it was
  shown in the app (its connection to the system's picker service cancelled
  half a second in): "Choose folder…" opened nothing.

- `src/window.rs` — `Window::on_frame_drawn` (a listener run after every
  frame the window draws), `Window::rendered_hitboxes` (the drawn frame's
  hitboxes in paint order) and `Window::rendered_tooltip_bounds`. The
  agent pane's web preview is a native view over GPUI's drawing: after each
  frame it follows its element's bounds, hides when its element wasn't
  drawn, and swaps itself for a snapshot while anything drawn later
  (a menu, a dialog, the palette, a tooltip) covers it
  (`crates/pitwall-app/src/preview`).

Windows out of sight keep their GPU memory on purpose: releasing it made
coming back to a window (another Space) visibly slow to draw.

To move to a newer GPUI: re-apply these, or drop the patch once upstream
allocates lazily.
