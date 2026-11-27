# Permissions

**Preview.** What an extension's tile may do, and how you decide. This page is written for people installing extensions first, and for authors after that.

## What a tile can do without asking

A tile runs in a sandbox. It starts with no files, no network and no programs: it can't read your projects, reach the internet, start anything or see your agents' terminals. It can't read other extensions' data, settings or secrets.

What every tile has, with no permission, reaches only its own things:

- its own views in the layout, drawn by Pitwall;
- its own Log;
- its own timers, which pause while it is hidden;
- its own stored data, at most 10 MiB;
- its own settings and secrets, which you set in Settings;
- toasts, at most 3 a minute.

[Platform services](platform-services.md) describes each of these for authors.

## The first grants

In this preview (WIT 0.1) one permission has an effect:

| Permission | Gives | Risk |
|---|---|---|
| `memory` | More memory than the 64 MiB every tile gets, up to the size the extension names (at most 512 MiB) | Low: it can use more of your computer's memory |

`memory` is granted with the extension when you turn it on: the permission screen and the extension's card show it with its size ("256 MiB"), not as a switch. `pitwall extension check` shows the size an extension asks for.

More grants come with the services that need them, each with its own risk note on the permission screen: network access to named hosts and notifications in phase 2, then files and git in your projects, the clipboard, and reading your agents, each when a tile needs it ([what comes later](#what-comes-later)).

## The permission screen

An installed extension is off until you turn it on in Settings → Extensions. Before installing, Pitwall lists the tiles it adds and each permission it asks for, with the author's reason and, for `memory`, the size. When you turn it on, the permission screen shows the same permissions, to grant or refuse each, and for an extension that only adds tiles it says that its tiles run sealed off from your computer and get only what you grant there.

After that, the extension's card in Settings → Extensions lists each permission and whether it is granted. A permission with a switch can be taken back at any time; the tile hears it at once (`GrantsChanged`), and its next call that needs it is refused. In WIT 0.1 the only permission is `memory`, which has no switch. Turning the extension off stops its tiles at once.

## Unsigned preview tiles

Before signed packages exist, an author can share a tile as a folder, which you install from Settings → Extensions → Install from folder… or with `pitwall extension install <folder>` ([Quick start](quick-start.md#share-a-preview-tile)). Only extensions that add tiles and run no programs install this way.

Such an extension comes from an "unsigned preview folder", and its install review and permission screen carry the warning "Unsigned, preview: only install tiles from people you trust". Nobody has checked who made it or what it contains. It runs in the same sandbox with the same limits as any tile, and it can do only what you grant it. It is never updated: installing a newer folder replaces it. Installing from the address of a folder served over HTTPS is coming in this preview.

A tile loaded with `pitwall extension dev` is a development extension: it is listed as one in Settings → Extensions and runs from the author's folder, where it is.

## For authors

Every permission names your reason: one line the user reads before granting it. Ask only for what the tile uses.

In `extension.json`, a permission's value is its reason, or an object with the reason and the permission's settings. `memory` is an object with its size in MiB, more than 64 and at most 512: `"permissions": { "memory": { "reason": "To hold large diagrams", "mib": 128 } }`.

In the tile, `cx.is_granted("memory")` and `cx.granted()` say what the user granted. When it changes, the tile hears `Event::GrantsChanged`, and by default it renders again. A call that needs a permission the tile wasn't granted fails with `HostError::Denied`; the tile should carry on without it and say what is missing.

Test both cases with the harness: `Harness::build(…).grant("memory")` starts the tile with the grant.

## What comes later

| Phase | Grants |
|---|---|
| 2 | `net` (HTTPS to the hosts the manifest names), `notify` (notifications), `agents.read` and `agents.queue` (your agents and their Next up), `show` (open a file or a web page) |
| 2 | What agents may do with a tile: open it and put content in it, allowed by default, with a switch per tile ([Agent tools](agent-tools.md)) |
| 3 | Agents reading a tile (asks once), and tools that act outside the tile (asks each time) |
| On demand | `files.read`, `files.write`, `git.read`, `git.write`, `git.remote`, `clipboard.read`, each when a tile needs it |

A "Run tiles from others" switch in Settings → Extensions will stop every tile not made by Pitwall at once.
