# Servers in extensions

**Preview.** Some extensions need a program that keeps running: a server that holds a team's board, indexes a codebase, or answers your agents. An extension declares it as its `service`, Pitwall keeps it running while the extension is on, and the extension's tile shows what it holds.

The server is any program, in any language, installed on your computer by one of the extension's actions. The tile stays in its sandbox: it reaches only its own server, at the address the manifest names, and nothing else on your computer.

## The manifest

The `service` block sits beside `contributes.tiles`:

```json
{
  "v": 2,
  "id": "team-board",
  "name": "Team board",
  "version": "1.0.0",
  "settings": [
    { "key": "listen", "type": "text", "label": "Listens on", "default": "127.0.0.1:8790" }
  ],
  "actions": [
    { "id": "server", "label": "Install the board server",
      "steps": [{ "run": ["cargo", "install", "team-board-server", "--version", "1.0.0", "--locked"] }] }
  ],
  "service": {
    "label": "The board server",
    "run": ["team-board-server", "--data", "{data}", "--listen", "{setting:listen}"],
    "install": "server",
    "version": { "run": ["team-board-server", "--version"], "match": "(\\d+\\.\\d+\\.\\d+)", "requires": ">=1.0.0" },
    "ready": { "listen": "{setting:listen}" }
  },
  "contributes": {
    "tiles": [{ "id": "board", "title": "Team board", "icon": "list", "wasm": "tile.wasm", "world": "pitwall:tile@0.1.0" }]
  }
}
```

- `run` is the command, with `{data}` (the extension's private folder, outside its installed folder) and `{setting:…}` filled in.
- `install` names the action that installs the program; the card offers it when the program isn't found.
- `version` checks the installed version before each start.
- `ready.listen` is the address the server listens on. It must be on this computer (`127.0.0.1`, `localhost` or `[::1]`, with a port).

You confirm its exact command once, on the extension's card. Pitwall starts it when the extension is on, starts it again after 1, 5 and 30 seconds if it stops, and stops it when the extension is turned off, updated or removed. Something already listening on its address is used, and nothing is started.

## The tile reaches its server

The tile may connect to its server's `ready.listen` address over `http`, without the `net` permission: the server is part of the same extension and you confirmed it. Nothing else on your computer is reachable. When the address comes from a setting, the tile follows the setting's value. The permission screen says "Its tiles talk to its own server at 127.0.0.1:8790" under "Adds to Pitwall".

```rust
match service::url("/api/items") {
    Some(url) => {
        cx.fetch(net::Request::get(url), |tile, answer, cx| {
            tile.items = answer.and_then(|r| r.json()).unwrap_or_default();
            cx.notify();
        })?;
    }
    None => tile.status = "The board server isn't set up".into(),
}
```

`service::info()` says how the server stands: `Up`, `Starting`, `Off` (its command isn't confirmed yet, or the extension is off), `NeedsInstall`, `Stopped` (you stopped it) or `Failed`, with its address, its version and why it isn't up. The tile hears `Event::ServiceChanged(info)` when that changes. When the server isn't up, show why with a button that calls `service::open_card()`: the card is where you install, confirm and start it. `service::restart()` starts it again.

## The tile and the server live apart

Hiding or closing the tile doesn't stop the server; turning the extension off stops both. Work that must go on while the tile is hidden belongs in the server.

Your agents can reach the server through the tile's tools ([Agent tools](agent-tools.md)). They run as you and can also reach a port on your computer themselves; Pitwall can't stop that. A server that wants a key from its clients declares it as a secret setting and an `env` variable; the tile reads the same secret with `settings::secret`.

## Testing

`Harness::own_server("127.0.0.1:8790")` says the server is up there, `respond(url, status, body)` answers its requests, and `set_service(info)` changes its state while the tile runs:

```rust
let mut t = Harness::load(".").own_server("127.0.0.1:8790").respond("http://127.0.0.1:8790/api/items", 200, "[]").open("board");
t.set_service(ServiceInfo { state: ServiceState::Failed, address: None, version: None, why: Some("exited with 1".into()) });
```

## Later

A server written with `pitwall-extension` and run by Pitwall in the same sandbox as tiles, with no program to install, comes after WIT 1.0. A tile written against this page works with it unchanged.
