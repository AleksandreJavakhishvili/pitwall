# Extensions

An extension adds to Pitwall. Its main part is usually a tile: a view of its own in Pitwall's layout, next to your agents' terminals, files and web pages. An author writes it in Rust, Pitwall runs it in a sandbox and draws it with its own UI code, so it looks and behaves like the rest of the app.

An extension can also give your agents tools of its own, and Pitwall registers them with every agent it starts through its MCP server, with no setup on your part. The first is opening the tile: any agent can open your tile next to itself with content in it ("draw me this diagram and show it on the right"), the way it already opens files and web pages in Pitwall with `show_files` and `show_web`. See [Agent tools](agent-tools.md).

**Preview.** The Tile SDK is being built. These pages describe extensions and tiles as they are designed for the SDK preview; names and details can change until version 1.0 of the tile world (WIT 1.0), when the Preview label comes off.

## Extensions and tiles

There is one kind of extension: one package, one manifest, one install, one permission screen. A tile is one thing an extension can contribute; an extension with no tile is still an extension.

| Term | Means |
|---|---|
| **Extension** | The package: its manifest (`extension.json`), id, version, source, permissions and everything it contributes |
| **Tile** | A view the extension draws in the layout, built from the SDK's elements and components |
| **Component** | The extension's own code: one WebAssembly component, run by Pitwall in a sandbox inside the app, with or without tiles |
| **Service** | An optional long-running server that comes with the extension |

## How a tile works

- **Written in Rust.** An author writes a tile against the `pitwall-extension` crate, whose UI code reads like GPUI (`div().flex().gap_2().child(…)`), and compiles it to a WebAssembly component (target `wasm32-wasip2`). The Rust target is all the toolchain it needs: `rustup target add wasm32-wasip2`.
- **The tile describes, Pitwall draws.** After each change the tile returns a description of its UI: a tree of boxes, text and Pitwall's own components. Pitwall draws that tree itself. The tile's code never touches a window.
- **It looks like Pitwall.** Styles name Pitwall's theme colours by meaning (text, surface, warning), not fixed colours, so a tile has Pitwall's fonts, focus ring, keyboard and text input, and follows Dark, Light, Flat, Glass and Reduce motion by itself.
- **Its own thread.** Each tile runs on a worker thread of its own, so a slow tile doesn't hold up the window.

## How tiles are kept safe

- **Nothing by default.** A tile starts with no files, no network and no programs: nothing but what you granted it.
- **Permissions with reasons.** An extension names each permission it needs, with the author's reason. You see them on the permission screen before it runs, can take any of them back at any time, and the change applies to the tile's next call.
- **Limits.** A tile has a memory limit (64 MiB unless its manifest asks for more, which the permission screen shows) and a time limit on each call; a tile that runs too long is stopped.

## The SDK preview

The first preview is for authors who want to build tiles early:

- `pitwall extension new` scaffolds a tile project, `pitwall extension dev` builds it on every save and reloads it in the running Pitwall, and `pitwall extension check` checks it.
- A tile can be shared before packages exist: Pitwall installs an unsigned folder (or one served over HTTPS) from Settings → Extensions or with `pitwall extension install`, shows the permission screen, and marks it "Unsigned, preview". Preview installs don't update.
- Tiles run on macOS, Linux and Windows.

Agent tools that open a tile with content come in phase 2 of the SDK, and the other kinds of tools (reading back what a tile shows, acting outside it) in phase 3. Signed packages, updates and a marketplace to browse come in later releases.

## In this section

- [Quick start](quick-start.md): from `pitwall extension new` to a tile running in Pitwall.
- [Tiles and UI components](ui.md): the elements a tile is built from.
- [Agent tools](agent-tools.md): tools a tile gives your agents.
- [Permissions](permissions.md): what each grant means.
- [Platform services](platform-services.md): what Pitwall gives every tile.
- [WIT reference 0.1](wit.md): the tile world's interfaces, generated from the WIT.
