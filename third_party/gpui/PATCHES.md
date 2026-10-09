# GPUI 0.2.2, patched

The `gpui` 0.2.2 crate from crates.io (Apache-2.0, `LICENSE-APACHE`), used
through `[patch.crates-io]` in the workspace `Cargo.toml`. Examples and tests
are left out. Every change is marked `Pitwall patch` in the source:

- `src/platform/mac/metal_renderer.rs` — the 4× multisampled path texture
  is memoryless on Apple GPUs (it is only resolved within its pass): ~110 MB
  less per full-screen Retina window. Both path textures are made on the
  first frame that draws a path instead of with the window, and
  `release_offscreen` drops them and shrinks the layer's drawables.
- `src/platform/mac/window.rs` — a window that goes off screen (another
  Space, minimised, hidden, fully covered) releases that memory; when it
  shows again it is resized back and the current scene presented before
  its display link restarts. With the window hidden the GPU driver also lets
  go of its own working memory (measured: 342 → 102 MB for the whole app).

To move to a newer GPUI: re-apply these, or drop the patch once upstream
allocates lazily.
