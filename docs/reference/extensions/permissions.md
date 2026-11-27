# Permissions

**Preview.** What an extension's tile may do, and how you decide. This page is written for people installing extensions first, and for authors after that.

## What a tile can do without asking

A tile runs in a sandbox. It starts with no files, no network and no programs: it can't read your projects, reach the internet, start anything or see your agents' terminals. It can't read other extensions' data, settings or secrets.

What every tile has, with no permission, reaches only its own things:

- its own views in the layout, drawn by Pitwall;
- its own Log;
- its own timers, which pause while it is hidden;
- its own stored data, at most 10 MiB, and its own database, at most 100 MiB;
- its own settings and secrets, which you set in Settings;
- toasts, at most 3 a minute;
- one button in the top bar, if it declares one, which you can hide;
- the extension's own server, if it has one.

[Platform services](platform-services.md) describes each of these for authors.

## The grants

| Permission | Gives | Starts | Risk |
|---|---|---|---|
| `memory` | More memory than the 64 MiB every tile gets, up to the size the extension names (at most 512 MiB) | granted | Low: it can use more of your computer's memory |
| `sql` | A larger database than the 100 MiB every tile gets, up to the size the extension names (at most 2048 MiB) | granted | Low: it can use more of your disk |
| `net` | Connections to the servers the extension names, listed on the permission screen | on | Medium: what it reads or sends goes to those servers |
| `notify` | Notifications, at most 3 a minute | on | Low |
| `show` | Opening a file in Pitwall's editor, or a web page in a web tile | off | Low: it shows you something; it gets nothing back |
| `agents.read` | Your agents' names, kinds, projects, branches and states, whether one waits on you, and how many items wait in its Next up; badges on agents' rows; naming the agent a question is from | on | Medium: a state can say what an agent waits on. Never what is on their screens |
| `agents.queue` | Putting text in an agent's Next up, marked with the extension's name. Nothing it puts there is sent until you press Send, unless you let it send without asking | off | High: text an agent gets becomes what it does, with its own permissions in your repositories |
| `projects.read` | Your projects' folders; badges on projects' rows | on | Low |

Badges on sidebar rows and questions in the floating control need no permission: the extension declares them (`"badges": true`, `"asks": true`), and the permission screen lists them under "Adds to Pitwall" ("badges and short notes on agents' and projects' rows in the sidebar", "questions in the floating control, answered there or in its tile").

`memory` and `sql` are granted with the extension when you turn it on: the permission screen and the extension's card show them with their size ("256 MiB"), not as a switch. `pitwall extension check` shows the sizes an extension asks for.

### net

The permission screen lists each server the extension may reach, as its manifest writes it (`api.example.com`, `*.example.org` for its subdomains). Only these, over HTTPS; plain `http` only to your own computer, when listed. Pitwall checks each address before connecting and on every redirect, and sends no cookies or passwords of its own. An extension with its own server reaches that server without `net`; the permission screen says so under "Adds to Pitwall".

### notify

A notification is the system's when you aren't looking at Pitwall, and a toast in Pitwall's window. At most 3 a minute, and the same words only once in 10 minutes.

More grants come with the services that need them, each with its own risk note on the permission screen: files and git in your projects and the clipboard, each when a tile needs it ([what comes later](#what-comes-later)).

## The permission screen

An installed extension is off until you turn it on in Settings → Extensions. Before installing, Pitwall lists the tiles it adds and each permission it asks for, with the author's reason and, for `memory`, the size. When you turn it on, the permission screen shows the same permissions, to grant or refuse each, and for an extension that only adds tiles it says that its tiles run sealed off from your computer and get only what you grant there.

After that, the extension's card in Settings → Extensions lists each permission and whether it is granted. A permission with a switch can be taken back at any time; the tile hears it at once (`GrantsChanged`), and its next call that needs it is refused. `memory` and `sql` have no switch; a larger `sql` size takes effect when the tile starts again. Turning the extension off stops its tiles at once, and its badges and questions go. Every call a tile makes to your agents is in the extension's Activity, marked "tile".

## Agents opening a tile

Agents need no permission from the extension to open its tiles: every tile can be opened by the agents Pitwall starts, with content in it, and an extension can declare tools that show things in its tile ([Agent tools](agent-tools.md)). These tools only change what the sandboxed tile shows. The agent gets nothing of the tile back, and Pitwall refuses to let it replace anything you edited there, so they are on by default and allowed in agents without asking.

You decide which tiles agents may use:

- **Per tile:** "Agents can use <name>" on the extension's card in Settings → Extensions, with the names of its tools. Off, its tools are gone from every agent's list at once.
- **All tiles:** "Agents can use tiles" in Settings → Agents (`agents.tileTools`).
- **Before turning it on:** the permission screen lists a tile's tools under "Adds to Pitwall", and an author can keep a tile from agents with `"agents": false`.

Each call is in the extension's Activity on its card: which agent, which tool and item, and whether it was shown.

## Signed packages and updates

A tile extension usually comes as a signed `.pwtile` file ([Install](install.md)). Installing it trusts its signing key for that extension, and every update must be signed by the same key. An update grants nothing new by itself: new permissions, more memory, a larger database and new `net` servers each wait on the card until you allow them. When new servers are added, you are asked about `net` again.

## Unsigned preview tiles

An author can also share a tile as an unsigned package or as a folder. A folder is installed from Settings → Extensions → Install from folder…, or with `pitwall extension install <folder>` which you install from Settings → Extensions → Install from folder… or with `pitwall extension install <folder>` ([Quick start](quick-start.md#share-a-preview-tile)). Only extensions that add tiles and run no programs install this way.

Such an extension comes from an "unsigned preview folder", and its install review and permission screen carry the warning "Unsigned, preview: only install tiles from people you trust". Nobody has checked who made it or what it contains. It runs in the same sandbox with the same limits as any tile, and it can do only what you grant it. A folder is never updated: installing a newer folder replaces it. An unsigned package shows "Unsigned: nobody vouches for this file" and installs only with Install unsigned. Installing from the address of a folder served over HTTPS is coming in this preview.

A tile loaded with `pitwall extension dev` is a development extension: it is listed as one in Settings → Extensions and runs from the author's folder, where it is.

## For authors

Every permission names your reason: one line the user reads before granting it. Ask only for what the tile uses.

In `extension.json`, a permission's value is its reason, or an object with the reason and the permission's settings:

- `memory`: its size in MiB, more than 64 and at most 512: `"memory": { "reason": "To hold large diagrams", "mib": 128 }`.
- `sql`: its size in MiB, more than 100 and at most 2048: `"sql": { "reason": "To keep a year of builds", "mib": 500 }`.
- `net`: the hosts, at most 32: `"net": { "reason": "To read your builds", "hosts": ["ci.example.com"] }`.
- `notify` and `show`: the reason alone.

In the tile, `cx.is_granted("memory")` and `cx.granted()` say what the user granted. When it changes, the tile hears `Event::GrantsChanged`, and by default it renders again. A call that needs a permission the tile wasn't granted fails with `HostError::Denied`; the tile should carry on without it and say what is missing.

Test both cases with the harness: `Harness::build(…).grant("memory")` starts the tile with the grant.

## What comes later

| Phase | Grants |
|---|---|
| 3 | Agents reading a tile (asks once), and tools that act outside the tile (asks each time) |
| On demand | `files.read`, `files.write`, `git.read`, `git.write`, `git.remote`, `clipboard.read`, each when a tile needs it |

A "Run tiles from others" switch in Settings → Extensions will stop every tile not made by Pitwall at once.
