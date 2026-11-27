# WIT reference 0.1

**Preview.** Every interface, type and function of the tile world `pitwall:tile@0.1.0`: the contract between a tile and Pitwall, written in WIT (the WebAssembly Interface Type language). It is generated from the WIT files' doc comments when the site is built, so it can't disagree with them.

## Reading it

- **You don't write WIT.** The `pitwall-extension` crate turns it into Rust: elements, the `Tile` trait, typed events and the `storage`, `settings`, `log`, `ui`, `agents`, `badges` and `asks` modules, and `tools` (over `agent-tools`). Read this page to see exactly what crosses between a tile and Pitwall, such as each limit and each error.
- **The world** says what a tile imports from Pitwall (`ui`, `log`, `tasks`, `storage`, `settings`, `agents`, `badges`, `asks`, `agent-tools`) and what it exports (`guest`). The SDK's `export!` writes the export for you.
- **Names.** WIT names use kebab case; the SDK's use Rust's (`request-render` is `ui::request_render`, `text-2` is `Token::Text2`).
- **What a tile gets besides.** WASI 0.2 for what Rust's standard library needs: clocks, random numbers, and stdout and stderr, which go to the tile's Log. There are no files, sockets, environment variables or arguments.
- **Versions.** A 0.x world may change between minor versions, and Pitwall keeps the last two 0.x worlds working. A tile names the world it was built for in its manifest (`"world": "pitwall:tile@0.1.0"`). WIT 1.0 ends the Preview.

The WIT files are in [`crates/pitwall-tile-host/wit/`](../../../crates/pitwall-tile-host/wit/world.wit), and a copy ships with the SDK.

<!-- wit-reference -->
