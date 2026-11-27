# Platform services

**Preview.** What Pitwall gives every tile, so authors don't build it themselves. In WIT 0.1 these are timers (`tasks`), a key-value store (`storage`), settings and secrets (`settings`), the Log (`log`) and requests about the tile's own views (`ui`). None of them needs a permission: each reaches only the tile's own things.

Pitwall adds a service only when a tile can't reasonably do without one. Phase 2 adds a SQLite database run by Pitwall, network access to named hosts, notifications, opening a file or a web page, and your agents and their Next up, each with the permission it needs ([what comes later](#what-comes-later)).

## Errors

The calls that can fail return `Result<_, HostError>`:

| Error | Means |
|---|---|
| `Denied` | The tile isn't granted the permission this needs |
| `Busy` | Too many calls or things at once (over 200 calls a second, or 32 timers); try again later |
| `TooLarge` | Over a size limit or a quota |
| `Invalid(why)` | An argument isn't valid; the text says which and why |
| `Failed(why)` | Pitwall couldn't do it, such as a disk error |
| `Decode(why)` | Stored data couldn't be read as the type asked for |

Each has a plain message (`Display`), fit for the Log.

## Tasks: timers

Timers are on the tile's context, since they call back into the tile:

| Call | Does |
|---|---|
| `cx.after(delay, f)` | Calls `f` once, after `delay` (a `Duration`) |
| `cx.every(period, f)` | Calls `f` every `period` until cancelled |
| `cx.cancel(timer)` | Stops a timer; one already finished is ignored |

`f` gets the tile and the context, like a listener. Both return `Option<Timer>`: `None` (and a Log line) when it can't be made. A delay or period is at least 100 ms, and a tile has at most 32 timers at once. Call `cx.notify()` in the closure when what the tile shows changed.

Timers pause while the tile is hidden and go on when it is shown again. A repeating timer that was due while hidden fires once, not once per missed period. There is no background work while hidden: a tile polls only while you can see it.

In tests, `wait(duration)` lets time pass and `wait_for_text(text, timeout)` waits until the tile shows something.

## Storage

The tile's own key-value store, in `storage`. Use it for what the user must not lose: it is written to disk after each call into the tile that changed it, and kept across restarts and updates, outside the extension's folder.

| Call | Does |
|---|---|
| `storage::get(key)` | The bytes of `key`, or `None` |
| `storage::set(key, value)` | Sets `key` to bytes |
| `storage::delete(key)` | Removes `key` (a missing key is no error) |
| `storage::list(prefix)` | The keys that start with `prefix` (all for `""`), in byte order |
| `storage::usage()` | The bytes the store takes now |
| `storage::get_json(key)`, `storage::set_json(key, &value)` | The same, as JSON with `serde` |
| `storage::get_string(key)` | The value as text |

The store holds at most 10 MiB in all; a write past that fails with `TooLarge`. Keys are 1 to 256 bytes of UTF-8 without control characters.

The `blank` template reads its count when it starts:

```rust
{{#include crates/pitwall-cli/templates/tile/blank/lib.rs.tmpl#blank-new}}
```

The `list` template keeps its whole list as JSON and notes a failed write in the Log:

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-save}}
```

For state that only has to survive a reload (what is being typed, a scroll position), use `save_state` instead ([Quick start](quick-start.md#develop-with-pitwall-extension-dev)).

In tests, `stored(key)` and `stored_json(key)` read what the tile stored.

## Settings

An extension declares its settings in its manifest's `settings` list. Pitwall draws the form on the extension's card in Settings → Extensions, keeps the values, and tells the tile when they change.

Each setting has a `key`, a `type`, a `label`, and optionally a `default`, a `placeholder` and a `hint` (one line under it). The types:

| Type | Read with | Drawn as |
|---|---|---|
| `text` | `settings::text(key)` | A text field |
| `url` | `settings::text(key)` | A text field for a URL |
| `number` | `settings::number(key)` | A number field (with `min` and `max`) |
| `bool` | `settings::flag(key)` | A switch |
| `choice` | `settings::text(key)`, which gives the option's value | A list of `options`, each with a `value` and a `label` |
| `secret` | `settings::secret(key)` | A password field, kept in the system's secure store |

`settings::get(key)` returns the value as a `Value` (`Text`, `Number` or `Flag`): what the user set, else the manifest's default, else `None`. A secret is never returned by `get`.

`settings::secret(key)` reads a secret from the system's secure store, and returns `None` when the user hasn't set it. Keep a secret in memory only: never store it, log it or show it.

`settings::open()` opens the extension's card in Settings, for a "Set up" button when a setting the tile needs is missing.

When the user changes a setting, the tile hears `Event::SettingsChanged` and, by default, renders again. The `form` template signs each note with its "Signed as" setting, read while rendering:

```rust
{{#include crates/pitwall-cli/templates/tile/form/lib.rs.tmpl#form-setting}}
```

In tests, `Harness::setting(key, value)` and `secret(key, value)` start the tile with settings, and `set_setting(key, value)` changes one while it runs:

```rust
{{#include crates/pitwall-cli/templates/tile/form/test.rs.tmpl#form-test}}
```

## Log

Each tile has a Log: open it from the extension's card in Settings → Extensions, or print it with `pitwall extension log <id>`. It also notes when the tile started, stopped or was reloaded. Pitwall keeps the newest 500 lines.

| Call | For |
|---|---|
| `log::debug(…)` | Detail for whoever is debugging the tile |
| `log::info(…)` | Something worth knowing happened |
| `log::warn(…)` | Something went wrong, and the tile carried on |
| `log::error(…)` | Something failed |

`println!` and `eprintln!` reach the same Log, as info and warnings. A line is at most 4 KiB; longer lines are cut. Pitwall adds its own notes, such as a slow call or a dropped toast.

In tests, `log_text()` gives the whole Log and `logged(level, text)` checks for a line.

## UI

Requests about the tile's own views, in `ui`. They act only inside the tile's own area of the window.

| Call | Does |
|---|---|
| `ui::toast(text, tone)` | Shows a short message, labelled with the tile's name. `Tone::Info`, `Good`, `Warn` or `Bad`. At most 200 characters and 3 toasts a minute; more are dropped and noted in the Log |
| `ui::open_view(tile)` | Opens one of the extension's tiles (an id from `contributes.tiles`), or focuses its view when one is open |
| `ui::scroll_to(view, key_path)` | Scrolls the element at that key path into sight after the view's next render |
| `ui::request_render(view)` | Renders the view again after this call; in a listener, `cx.notify()` is simpler |

The `form` template saves a note and says so in a toast:

```rust
{{#include crates/pitwall-cli/templates/tile/form/lib.rs.tmpl#form-save}}
```

In tests, `toasts()` lists the toasts and `ui_requests()` the other requests.

## Not provided

- **Files, the network and programs.** WIT 0.1 has none. `std::fs` and sockets fail; keep data in `storage`. Network access and files come as permissions ([Permissions](permissions.md#what-comes-later)).
- **Agents' terminals**, at any grant. They carry secrets and prompts.
- **Other extensions' data, settings or secrets.**
- **Background work while hidden.** That is a service's job, which comes in phase 2.

## What comes later

| Phase | Services |
|---|---|
| 2 | `sql`: a SQLite database run by Pitwall, with migrations, 100 MiB |
| 2 | `net`: HTTPS to the hosts the manifest names, with the `net` permission |
| 2 | Notifications, and opening a file at a line, the editor or a web page |
| 2 | Your agents and their status (`agents.read`), badges on sidebar rows, questions in the floating control, and adding to an agent's Next up (`agents.queue`) |
| 2 | A toolbar item with a badge, which the user can hide |
| On demand | Files and git in your projects, file pickers, the clipboard, starting agents, and more, each when a tile needs it |
