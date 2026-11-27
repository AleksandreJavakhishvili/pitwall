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

A tile that isn't granted `memory` still runs, with 64 MiB. `pitwall extension check` shows the size an extension asks for.

More grants come with the services that need them, each with its own risk note on the permission screen: network access to named hosts and notifications in phase 2, then files and git in your projects, the clipboard, and reading your agents, each when a tile needs it ([what comes later](#what-comes-later)).

## The permission screen

**Coming in this preview.** Before an extension's tile runs for the first time, Pitwall shows its permission screen: the extension's name, each permission it asks for with the author's reason for it, the memory it asks for, and what it adds (its tiles). You turn it on from there.

After that, the extension's card in Settings → Extensions lists each permission with whether it is granted and when, and a switch. Taking a grant back reaches the tile at once (as `GrantsChanged`), and its next call that needs it is refused. Turning the extension off stops its tile at once.

## Unsigned preview tiles

**Coming in this preview.** Before signed packages exist, an author can share a tile as a folder, which you install from Settings → Extensions → Install from folder… or with `pitwall extension install <folder>` ([Quick start](quick-start.md#share-a-preview-tile)).

Such a tile is marked "Unsigned, preview" on the permission screen and wherever it appears, with a warning: "Only install tiles from people you trust". Nobody has checked who made it or what it contains. It runs in the same sandbox with the same limits as any tile, and it can do only what you grant it. It is never updated: installing a newer folder replaces it.

A tile loaded with `pitwall extension dev` is a development extension: it runs from the author's folder and is marked as such.

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
