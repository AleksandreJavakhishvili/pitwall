# The starter

```sh
pitwall extension new <folder> [--name "My inbox"]
```

Writes a working extension into a new folder (or an empty one). Its id is
the folder's name, so name the folder like an id: lower-case letters,
digits and `-`, starting with a letter (`my-inbox`). `--name` is what
Pitwall shows; without it, the name comes from the folder's (`my-inbox`
becomes "My inbox"). It works offline and doesn't need the app.

It writes:

| File | What it is |
| --- | --- |
| `extension.json` | The manifest: id, name, version 0.1.0, the inbox icon, and the adapter's command, `node adapter.mjs`. No settings. |
| `adapter.mjs` | An adapter for [protocol 1](protocol.md) in plain Node (18 or later, no dependencies). It shows one thread, "Hello from …", with one message. Opening it sends the thread, a reply is added to it and answered ok, and "mark read" works. It prints a line to stderr when it starts, and exits when its stdin closes. |
| `README.md` | A short readme to fill in. |
| `test/pitwall.json` | A [`run`](checks.md#run) check: the adapter answers hello, says ready, lists 1 thread, opens `welcome` and takes a reply. |

As written, it passes `pitwall extension check` and `pitwall extension
test`. Pitwall's own tests make sure of that.

## `--protocol 2`

```sh
pitwall extension new <folder> --protocol 2
```

Writes an extension that reads Pitwall's data instead
([protocol 2](protocol-2.md)):

| File | What it is |
| --- | --- |
| `extension.json` | [Format 2](manifest-2.md): `pitwall` set to this version, the `agents.read` [permission](permissions.md) with its reason, and `contributes.inbox`. |
| `adapter.mjs` | Answers hello with protocol 2 when Pitwall offers it, calls `agent.list`, subscribes to `agents` and shows the agents in one thread, "Your agents", kept current from `agents.changed` events. Without the grant it says so in the thread rather than failing. |
| `test/pitwall.json` | A [`calls`](checks.md#calls) check: a fake Pitwall answers `agent.list` and sends one event; the thread shows the agents before and after. |

Turning it on shows the permission screen with your reason. Its calls
show under **Activity** in its card.

## `pitwall extension new <folder> --view`

Writes an extension that draws a [view tile](views.md) instead of an
inbox:

| File | What it is |
| --- | --- |
| `extension.json` | [Format 2](manifest-2.md): the `agents.read` permission with its reason, and `contributes` with one view (`items`), one command (`refresh`) and `badges`. |
| `adapter.mjs` | Answers hello with protocol 2. While the view is shown it sends a list of two items with the selected one's detail beside it; "Mark done" on an item is answered and moves it to a "Done" section; the palette command "<name>: Refresh" is answered; and it puts a short note on each agent's row (read with `agent.list`). |
| `test/pitwall.json` | A [`views`](checks.md#views) check of all of that against a fake Pitwall. |

In Pitwall, turn it on and pick **Open <name>** in the command palette
(⌘K): the view opens as a tile beside the focused one.

## From here

1. Load it in Pitwall ([dev mode](dev-mode.md)) and turn it on. Open the
   inbox (⌘I) to see its thread.
2. Replace the `threads` map in `adapter.mjs` with your service's data:
   fetch it after `hello`, and send the whole list (`threads`) whenever
   it changes. Send `status` with `connecting`, `ready`, `signedOut` or
   `error` as things go, with a message that says what to do.
3. If it needs an address or a key, add [`settings`](manifest.md#settings)
   to the manifest. They reach the adapter only in `hello`
   (`msg.settings`, `msg.secrets`).
4. Answer what your service can do (`answer`, `archive`). Answer
   everything else with `{"type":"result","ok":false,"message":"…"}`, as
   the starter does.
5. Keep `test/pitwall.json` in step, and run `pitwall extension test`
   (in CI too: it exits with status 1 when a check fails).
