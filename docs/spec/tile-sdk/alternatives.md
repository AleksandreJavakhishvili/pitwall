# Tile SDK: alternatives

Part of the [Tile SDK and marketplace](README.md) design. Alternatives considered. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 12. Alternatives considered

| Alternative | For | Against | Verdict |
|---|---|---|---|
| **Native dynamic libraries** (Rust `cdylib` loaded with `libloading`) | Full GPUI, fastest | No stable Rust ABI across compiler versions, so every tile must be rebuilt per Pitwall release; a crash or a hang takes the app down; full access to memory, keychain and windows; per-OS binaries; code signing issues on macOS | **Native plugin API: rejected** (as extension-api.md [today]); see [16.1](native-blocks.md#161-the-principle-no-native-plugin-api) for the full reasons and Zed's same choice |
| **A separate process drawing its own window**, embedded in the layout | Any toolkit, crash isolation | Can't look native (fonts, glass, focus ring differ); embedding foreign windows is OS-specific and fragile; no shared keyboard model | Rejected |
| **A separate process sending an element tree** (the adapter protocol, extended) | Any language, exists today | No sandbox (runs as the user); needs a runtime installed (Node); per-OS binaries for compiled languages; a pipe round trip per event | Rejected: adapters are removed when components ship ([section 15](extension-model.md#15-one-extension-model)). Wasm is enforced, portable and one file; a native server stays possible as a managed service |
| **Web tiles only** | Exists today; any web stack | Doesn't look or feel native; no Pitwall keyboard model; no bridge by policy, so no access to agents or files | Stays the escape hatch [today] |
| **Lua or JavaScript scripting in process** | Easy for authors; hot reload | An engine to embed and secure (JS engines are large); no types; authors asked for Rust; sandboxing is per-engine and weaker than wasm's linear memory; slower for heavy work (parsing, diffing) | Rejected |
| **Wasm, but with a different runtime** (Wasmer, WAMR, wasmi) | Smaller (WAMR, wasmi) | Wasmtime is the Bytecode Alliance's reference for the Component Model and WASI 0.2/0.3 (S15; that others are less complete is *unverified*); Zed's extensions and its wasm grammars run on Wasmtime (S18), so GPUI-adjacent prior art exists | Wasmtime |
| **Expose GPUI itself to the guest** (proxy every GPUI call) | Authors use real GPUI | Thousands of calls per frame across the boundary; GPUI's API changes often and isn't versioned; the guest could then do anything GPUI can in the window | Rejected; the SDK mimics GPUI's builder style instead |
