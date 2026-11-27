# Notes

A Pitwall tile: Rust built to a WebAssembly component (`wasm32-wasip2`) that
Pitwall runs in a sandbox and draws in its own theme. It is a seed tile
(docs/spec/2026-11-27-notes-tile/): it uses only the public
`pitwall-extension` SDK, so whatever it lacks is an SDK gap, recorded in that
spec, never worked around with a private hook.

Before changing it, read the authoring skill in this repository:
`sdk/pitwall-extension/skills/pitwall-extension-authoring/SKILL.md` (what
`pitwall extension new` copies into a new project's `.agents/skills/` and
`.claude/skills/`; not copied here, so it can't drift from the SDK).

- Test it: `cargo test` (the tests build `tile.wasm` and run it in Pitwall's
  tile runtime)
- Check it: `cargo run -p pitwall-cli -- extension check tiles/notes` from the
  repository's root
- Its dependencies: `sh scripts/check-deps.sh`
