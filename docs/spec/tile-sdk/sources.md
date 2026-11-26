# Tile SDK: sources

Part of the [Tile SDK and marketplace](README.md) design. Sources (S1…) cited across the Tile SDK spec, and the claims not verified.

## Sources

Checked on 2026-11-26. Each was read on the page or in the project's
own repository unless marked.

- **S1** Wasmtime releases and release policy: v51.0.0 on 2026-11-20;
  patch releases 50.0.3, 49.0.7, 48.0.10, 36.0.23 on 2026-11-19 for a
  WASI filesystem sandbox escape on Windows; monthly majors, LTS every
  12th version for 24 months, others 2 months, security fixes
  backported; precompiled files tied to the exact version.
  https://github.com/bytecodealliance/wasmtime/releases,
  https://docs.wasmtime.dev/stability-release.html
- **S2** Wasmtime licence (Apache-2.0 WITH LLVM-exception).
  https://github.com/bytecodealliance/wasmtime/blob/main/LICENSE
- **S3** `StoreLimitsBuilder` (defaults, `trap_on_grow_failure`) and
  `Store::limiter`.
  https://docs.rs/wasmtime/latest/wasmtime/struct.StoreLimitsBuilder.html,
  https://docs.rs/wasmtime/latest/wasmtime/struct.Store.html
- **S4** Fuel and epoch interruption (`Config::consume_fuel`,
  `Config::epoch_interruption`, `Store::set_epoch_deadline`, the
  "2-3x" note, 64-bit atomics).
  https://docs.rs/wasmtime/latest/wasmtime/struct.Config.html,
  https://docs.rs/wasmtime/latest/wasmtime/struct.Store.html
- **S5** `Component::serialize` / `deserialize_file`, why `deserialize`
  is `unsafe`, `Engine::precompile_component` and
  `precompile_compatibility_hash`, `Config::cache` (off by default).
  https://docs.rs/wasmtime/latest/wasmtime/component/struct.Component.html,
  https://docs.rs/wasmtime/latest/wasmtime/struct.Module.html,
  https://docs.rs/wasmtime/latest/wasmtime/struct.Engine.html,
  https://docs.wasmtime.dev/cli-cache.html
- **S6** WIT: package versions, `@since`, `@unstable`, `@deprecated`;
  `bindgen!` matching semver-compatible imports.
  https://github.com/WebAssembly/component-model/blob/main/design/mvp/WIT.md,
  https://component-model.bytecodealliance.org/design/wit.html,
  https://docs.rs/wasmtime/latest/wasmtime/component/macro.bindgen.html
- **S7** `WasiCtxBuilder` defaults (no env, args or preopens; stdin
  closed, stdout/stderr discarded; name lookup denied; addresses denied,
  with an inconsistent note on `allow_tcp`), `socket_addr_check`;
  `WasiHttpHooks::send_request` for filtering outgoing HTTP.
  https://docs.rs/wasmtime-wasi/latest/wasmtime_wasi/struct.WasiCtxBuilder.html,
  https://docs.rs/wasmtime-wasi-http/latest/wasmtime_wasi_http/trait.WasiHttpHooks.html
- **S8** Zed's extension index: `extensions.toml` with git submodules,
  PR rules, CI packaging and upload, the removal workflow, review times.
  https://zed.dev/docs/extensions/publishing/publishing-guide,
  https://github.com/zed-industries/extensions/blob/main/src/package-extensions.js,
  https://github.com/zed-industries/extensions/blob/main/.github/workflows/remove-extension.yml,
  https://zed.dev/docs/extensions/publishing/faq
- **S9** Minisign: Ed25519 over a BLAKE2b-512 prehash, key id, trusted
  comment, file format. https://jedisct1.github.io/minisign/
- **S10** Sigstore: keyless signing with Fulcio certificates and the
  Rekor log (as npm provenance uses it); `sigstore-rs` still
  "experimental" (v0.15.1, 2026-11-22).
  https://github.blog/security/supply-chain-security/introducing-npm-package-provenance/,
  https://docs.npmjs.com/generating-provenance-statements,
  https://github.com/sigstore/sigstore-rs
- **S11** Obsidian: `community-plugins.json` and
  `community-plugins-removed.json` (id, name, reason), now a bot mirror;
  submission moved to community.obsidian.md.
  https://github.com/obsidianmd/obsidian-releases,
  https://docs.obsidian.md/Plugins/Releasing/Submit+your+plugin
- **S12** crates.io sparse index layout, per-version `cksum` and
  `yanked`; a yank doesn't delete.
  https://doc.rust-lang.org/cargo/reference/registry-index.html,
  https://doc.rust-lang.org/cargo/commands/cargo-yank.html
- **S13** Raycast: a React tree serialised to JSON, diffed with JSON
  Patch, sent over stdio as JSON-RPC and built into native views; every
  store extension reviewed by the Raycast team, open source required.
  https://www.raycast.com/blog/how-raycast-api-extensions-work,
  https://developers.raycast.com/basics/publish-an-extension,
  https://developers.raycast.com/basics/prepare-an-extension-for-store
  (How Raycast's 2026 rewrite renders extensions is *unverified*.)
- **S14** VS Code: marketplace signatures verified at install, a
  publisher trust prompt, malicious extensions removed and uninstalled
  from clients through a control manifest's `malicious` list.
  https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security,
  https://github.com/microsoft/vscode/blob/main/src/vs/platform/extensionManagement/common/extensionManagement.ts
- **S15** WASI 0.3 declared stable (June 2026), Wasmtime 46+ ships it;
  `wasmtime-wasi-http`'s p3 module still "experimental".
  https://bytecodealliance.org/articles/WASI-0.3,
  https://docs.rs/wasmtime-wasi-http/latest/wasmtime_wasi_http/
- **S16** Rust targets `wasm32-wasip2` (tier 2; Rust 1.82 per
  cargo-component's README) and `wasm32-wasip3` (tier 2 from Rust
  1.100); cargo-component calls itself experimental and says plain
  cargo is enough for WASI-only use.
  https://doc.rust-lang.org/nightly/rustc/platform-support/wasm32-wasip2.html,
  https://doc.rust-lang.org/nightly/rustc/platform-support/wasm32-wasip3.html,
  https://github.com/bytecodealliance/cargo-component
- **S17** Wasmtime minimal builds: sizes of `libwasmtime.so`
  (19M / [2.1](runtime.md#21-where-it-fits)M / 1.2M / 698K), features that can be turned off, Winch
  and Pulley. https://docs.wasmtime.dev/examples-minimal.html
- **S18** Zed's extension system: `zed_extension_api` (0.7.0 on
  crates.io, 0.8.0 unpublished in the repo, Apache-2.0), versioned WIT
  folders, kinds of extensions, `wasm32-wasip2`, grammars through
  tree-sitter's `wasm` feature on Wasmtime, the capabilities setting,
  the theme schema, the licence requirement; requests for UI
  extensions are open discussions.
  https://zed.dev/blog/zed-decoded-extensions,
  https://github.com/zed-industries/zed/tree/main/crates/extension_api/wit,
  https://zed.dev/docs/extensions/developing-extensions,
  https://github.com/zed-industries/zed/blob/main/crates/language/Cargo.toml,
  https://github.com/zed-industries/zed/blob/main/docs/src/extensions/capabilities.md,
  https://zed.dev/docs/extensions/publishing/license-requirements,
  https://github.com/zed-industries/zed/discussions/6679
- **S19** Dioxus `WriteMutations`.
  https://docs.rs/dioxus_core/latest/dioxus_core/trait.WriteMutations.html
- **S20** Zellij plugins render text; Figma's plugin sandbox and iframe
  UI. https://zellij.dev/documentation/plugin-lifecycle,
  https://developers.figma.com/docs/plugins/how-plugins-run/
- **S21** Serving incoming HTTP to a component with
  `wasmtime-wasi-http` p2: `WasiHttpView`, `add_to_linker_async` for
  the `wasi:http/proxy` world, `ProxyPre` for quick per-request
  instantiation, a fresh `Store` per request, `call_handle`; the hyper
  example is HTTP/1.1 only; the `handler` module dispatches to
  `incoming-handler@0.2` or `handler@0.3`; `wasmtime serve` needs
  wasi-http and the component model (checked 2026-11-26).
  https://docs.rs/wasmtime-wasi-http/latest/wasmtime_wasi_http/p2/index.html,
  https://docs.wasmtime.dev/api/src/wasmtime_wasi_http/handler.rs.html,
  https://docs.rs/crate/wasmtime-cli/43.0.1/source/src/commands/serve.rs
- **S22** SQLite and WASI: rusqlite's wasm feature targets
  `wasm32-unknown-unknown`; third-party component builds
  (tegmentum/sqlite-wasm) and `sqlite-wasm-wasi`, whose docs mention
  missing `flock` support in Wasmtime. Read through search summaries,
  not tried. https://lib.rs/crates/rusqlite/features,
  https://awesome.ecosyste.ms/projects/github.com%2Ftegmentum%2Fsqlite-wasm,
  https://docs.rs/crate/sqlite-wasm-wasi/0.6.0/source/README.md
- **S23** OAuth 2.0 for native apps: the system browser, a loopback
  redirect and PKCE. RFC 8252, https://www.rfc-editor.org/rfc/rfc8252
- **S24** The `llms.txt` proposal: a Markdown file at a site's root
  giving language models a concise guide to its docs.
  https://llmstxt.org
- **S25** Zed on grammars as shared libraries vs. WebAssembly: "We
  would never be able to prevent these kinds of crashes if we loaded
  extensions directly as shared libraries"; wasm is "cross-platform,
  and it's designed for running untrusted code safely" (Max Brunsfeld,
  2024-04-18). https://zed.dev/blog/language-extensions-part-1

### Not verified
- That WIT has no recursive types ([3](runtime.md#3-the-wit-interface)).
- That Wasmtime refuses to re-enter a component instance ([2.4](runtime.md#24-threading-with-gpui)).
- Compile times, frame encoding cost and binary growth for Pitwall's
  case ([2.6](runtime.md#26-compile-and-cache), [2.7](runtime.md#27-performance-budget), [10.3](README.md#103-decisions)): phase 0 measures them.
- Which characters each agent accepts in MCP tool names ([11.4](agent-tools.md#114-how-agents-reach-them)).
- Whether Rust 1.100 (with `wasm32-wasip3`) has shipped.
- How Wasmtime handles per-OS traps in detail ([2.2](runtime.md#22-crates-new)).
- Whether a Zed theme maps well onto Pitwall's tokens ([10.3](README.md#103-decisions), item 12).
- Whether Wasmtime's `wasi:sockets` TCP listen path suits a long-lived
  service ([13.3](services.md#133-b-a-headless-wasm-service)).
- Whether SQLite compiled for `wasm32-wasip2` works reliably, and the
  `flock` gap ([13.3](services.md#133-b-a-headless-wasm-service)).
- What each OS reports about a loopback peer process ([13.3](services.md#133-b-a-headless-wasm-service)).
- What macOS's hardened runtime requires of a loaded plugin library
  ([16.1](native-blocks.md#161-the-principle-no-native-plugin-api)).
- Whether walking a large project through WASI is too slow for the
  files tree port, which is why search is a host service ([16.3](native-blocks.md#163-which-built-in-tiles-move-to-the-sdk)).
