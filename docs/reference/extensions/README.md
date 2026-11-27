# Extensions

An extension adds to Pitwall. Its main part is usually a tile: a view of its own in Pitwall's layout, next to your agents' terminals, files and web pages. An author writes it in Rust, Pitwall runs it in a sandbox and draws it with its own UI code, so it looks and behaves like the rest of the app.

An extension can also give your agents tools of its own, and Pitwall registers them with every agent it starts through its MCP server, with no setup on your part. The first is opening the tile, in the preview now: any agent can open your tile next to itself with content in it ("draft that as a note on the right"), the way it already opens files and web pages in Pitwall with `show_files` and `show_web`, and your tile can declare show tools of its own. See [Agent tools](agent-tools.md).

**Preview.** The Tile SDK is being built. These pages describe what the SDK preview has today, and mark what is still coming; names and details can change until version 1.0 of the tile world (WIT 1.0), when the Preview label comes off.

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

- The `pitwall-extension` crate to write a tile with, and Pitwall's tile runtime with its sandbox and limits, on macOS, Linux and Windows.
- Tiles in Pitwall's layout, opened from the command palette, with their Log in Settings → Extensions.
- `pitwall extension new` scaffolds a tile project from a template, `pitwall extension dev` builds and checks it on every save and reloads it in the running Pitwall keeping its state, `pitwall extension check` checks it, and `pitwall extension test` runs its tests in Pitwall's own tile runtime.
- Tiles are shared as signed packages. `pitwall extension pack` makes a `.pwtile` file and `pitwall extension sign` signs it with your minisign key. Pitwall installs the file from Settings → Extensions → Install from file… or with `pitwall extension install <file>`. It checks the signature first, shows the key, and requires updates to be signed by the same key. The version before an update is kept for Go back ([Packaging](packaging.md), [Install](install.md)).
- For a quick try, an unsigned folder installs too (Install from folder…), with an "Unsigned, preview" warning. Folder installs don't update.
- An authoring skill for coding agents, and the docs as one `llms.txt` file ([For coding agents](for-agents.md)).
- Agents open tiles with content (`<extension-id>__open`) and use the show tools a tile declares, through Pitwall's MCP server, with a switch per tile ([Agent tools](agent-tools.md)).

The other kinds of tools (reading back what a tile shows, acting outside it) come in phase 3 of the SDK. A marketplace to browse comes in a later release.

## In this section

- [Quick start](quick-start.md): from `pitwall extension new` to a tile running in Pitwall, and sharing it.
- [Tiles and UI components](ui.md): the elements a tile is built from, styles, events and keys.
- Pitwall's components for tiles, by group: [Form controls](components-forms.md), [Feedback](components-feedback.md), [Layout](components-layout.md), [Data display](components-data.md) and [Charts](components-charts.md).
- [Canvas and drag](canvas.md): drawing diagrams with zoom, pan and clickable regions; drag and drop inside a tile.
- [Native building blocks](blocks.md): Pitwall's code view and diff view in a tile.
- [Agent tools](agent-tools.md): tools a tile gives your agents: opening it with content and show tools (phase 3: reading and acting).
- [Permissions](permissions.md): what each grant means.
- [Packaging](packaging.md): packing and signing a `.pwtile`, keys and updates.
- [Install](install.md): installing from a file, who signed it, updates, Go back and Remove.
- [Platform services](platform-services.md): what Pitwall gives every tile.
- [Servers in extensions](servers.md): a tile and a server the extension keeps running.
- [WIT reference 0.1](wit.md): the tile world's interfaces, generated from the WIT.
- [For coding agents](for-agents.md): the authoring skill and `llms.txt`.
