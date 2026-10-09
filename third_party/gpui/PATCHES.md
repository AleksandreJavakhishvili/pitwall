# GPUI 0.2.2, patched

The `gpui` 0.2.2 crate from crates.io (Apache-2.0, `LICENSE-APACHE`), used
through `[patch.crates-io]` in the workspace `Cargo.toml`. Examples and tests
are left out. Every change is marked `Pitwall patch` in the source:

- `src/platform/mac/metal_renderer.rs` — the 4× multisampled path texture
  is memoryless on Apple GPUs (it is only resolved within its pass): ~110 MB
  less per full-screen Retina window, at no cost to drawing. Both path
  textures are made on the first frame that draws a path instead of with
  the window.

Windows out of sight keep their GPU memory on purpose: releasing it made
coming back to a window (another Space) visibly slow to draw.

To move to a newer GPUI: re-apply these, or drop the patch once upstream
allocates lazily.
