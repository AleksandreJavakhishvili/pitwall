# GPUI, vendored from Zed

GPUI and the crates it needs, from Zed's repository
(https://github.com/zed-industries/zed) at commit
`58e29b637c8670770817245e05e08bd9bd19b408`. GPUI has not been published to
crates.io since 0.2.2, so Pitwall vendors it; the workspace `Cargo.toml`
points `gpui` here through `[patch.crates-io]`. Pitwall's own changes are
listed in PATCHES.md.

Crates: `gpui`, `gpui_macros`, `gpui_platform`, `gpui_macos`, `gpui_apple`,
`gpui_ios`, `gpui_windows`, `gpui_linux`, `gpui_wgpu`, `gpui_web`,
`gpui_shared_string`, `gpui_util`, `collections`, `refineable`,
`sum_tree`, `scheduler`, `http_client`, `bench_metrics`, `ztracing`,
`ztracing_macro` and `zlog`. All Apache-2.0 (each crate's
`LICENSE-APACHE`).

How they were made: `cargo vendor` of Zed's crates as a git dependency at
that commit, so the manifests are cargo-normalized (workspace values filled
in). Then:

- the `gpui` examples were removed, and some dev-dependencies that would
  pull in more of Zed (`util_macros`, `reqwest_client`, and `zlog` for the
  `sum_tree` tests);
- `zlog` was added with a hand-written manifest (from Zed's
  `crates/zlog/Cargo.toml`, workspace values filled in), since `ztracing`
  needs it.

## Updating

1. Vendor the same crates at a new Zed commit (`cargo vendor` from a git
   dependency on Zed at that commit) and replace these folders with them.
2. Redo the removals and the `zlog` manifest above.
3. Re-apply PATCHES.md (`grep -rn "Pitwall patch"` in the old copy shows
   each change), dropping any that upstream has made, and update this
   README's commit.
