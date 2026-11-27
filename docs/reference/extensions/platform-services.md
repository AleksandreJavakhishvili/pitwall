# Platform services

**Preview.** What Pitwall gives every tile, so authors don't build it themselves. In WIT 0.1 these are timers (`tasks`), a key-value store (`storage`), settings and secrets (`settings`), the Log (`log`) and requests about the tile's own views (`ui`), which need no permission because each reaches only the tile's own things; and the agents' questions group: your agents and their Next up (`agents`), badges on sidebar rows (`badges`) and questions in the floating control (`asks`) ([below](#agents-badges-and-questions)).

Pitwall adds a service only when a tile can't reasonably do without one. Phase 2 adds a SQLite database run by Pitwall, network access to named hosts, notifications and opening a file or a web page, each with the permission it needs ([what comes later](#what-comes-later)).

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

### Clocks

`std::time::Instant` and `SystemTime` work (WASI's clocks), so a tile can time a debounce or stamp what it stores; `std::thread::sleep` blocks the whole tile, so wait with a timer instead.

In tests time is virtual: it stands still until the test moves it, with `advance(duration)` (`wait` is the same), which fires each timer due on the way at its own time, and moves `Instant` and `SystemTime` with it. `wait_for_text(text, timeout)` moves it in small steps until the tile shows something. `Harness::wall_clock()` runs a test on the computer's clock instead.

## Storage

The tile's own key-value store, in `storage`. Use it for what the user must not lose: it is written to disk after each call into the tile that changed it, and kept across restarts and updates, outside the extension's folder.

| Call | Does |
|---|---|
| `storage::get(key)` | The bytes of `key`, or `None` |
| `storage::get_many(&keys)` | The bytes of each key, in order (`None` for a missing one), at most 1024 keys: one call, for reading many at once |
| `storage::entries(prefix)`, `storage::entries_json(prefix)` | Every key that starts with `prefix` with its value (bytes, or JSON with `serde`), in byte order: a `list` and `get_many` calls, so a tile reads all its records in a few calls |
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

In tests, `stored(key)` and `stored_json(key)` read what the tile stored, and `Harness::stored(key, value)` (or `stored_json`) starts the tile with values already stored. `restart()` starts it again with the same storage and no saved state; `reload()` keeps the saved state too.

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
| `ui::scroll_to(view, key_path)` | Scrolls the element at that key path into sight after the view's next render. The key path is the one `cx.focus` takes: keys from the root down, without the root's (`"list/rows/n4"`) |
| `ui::request_render(view)` | Renders the view again after this call; in a listener, `cx.notify()` is simpler |

The `form` template saves a note and says so in a toast:

```rust
{{#include crates/pitwall-cli/templates/tile/form/lib.rs.tmpl#form-save}}
```

In tests, `toasts()` lists the toasts and `ui_requests()` the other requests.

## Agents, badges and questions

The agents' questions group: what an inbox-style tile needs to show questions from a service or from agents, have them answered in the floating control, and send an answer to an agent. The same rules as extensions' adapters have: the same permissions, limits and Activity.

### Agents

With the `agents.read` permission, `agents::list()` gives every agent Pitwall shows, in the sidebar's order: its `id`, `name`, `kind` and `kind_name` ("Claude Code"), `project` (its folder), `branch`, `state` (`Working`, `Blocked`, `Idle`, `Done`, `Stopped`, `Unknown`), `waiting_on_you` (it is blocked on an approval or a question) and `queued` (items in its Next up). When that changes, the tile hears `Event::AgentsChanged` and reads them again; changes close together arrive as one.

With `agents.queue`, `agents::queue_add(agent_id, text)` puts text in an agent's Next up, marked "From <extension name>" on its "N queued" chip and in the popover it opens. It waits for the user's Send: Auto-send sends it only when the user lets the extension send without asking, in its card. At most 4 000 characters (`TooLarge`); at most 3 of the extension's items wait in one agent's Next up, and one is queued per agent every 10 seconds (`Busy`, with the reason in the Log).

```rust
let waiting: Vec<Agent> = agents::list()?.into_iter().filter(|a| a.waiting_on_you).collect();
agents::queue_add(&waiting[0].id, "Rebase on main and run the tests")?;
```

The agents' terminals are never readable, at any grant.

### Badges

With `"badges": true` in `contributes`, `badges::set(&[…])` puts a count or a few words on sidebar rows, beside Pitwall's own waiting badges: on an agent's row by its id (needs `agents.read`) or on a project's row by its folder (needs `projects.read`). The whole set each time; `&[]` clears them.

```rust
badges::set(&[Badge::count(Row::Agent(agent.id.clone()), 2).tone(Tone::Bad).tooltip("2 failing runs").open("main")])?;
```

A count is at most 9999, a note (`Badge::note`) at most 16 characters, a tooltip (shown after the extension's name) at most 120. `open` names one of the extension's tiles, opened by a click. At most 500 badges, and at most four sets a second are shown (the newest waits for its turn). A badge on a row the tile may not name is dropped and noted once in its Log. They go when the extension is turned off or its last tile closes.

### Questions

With `"asks": true` in `contributes`, `asks::set(&[…])` shows the tile's open questions in the floating control, after agents' own prompts, as an inbox's are: a card with who asks, the question, the suggestions, a field for the user's own words, Send, Decline and "Open <tile>". A blocking question comes first, marked "Waiting on you". The whole set each time; at most 50.

```rust
asks::set(&[Ask::new("deploy-42", "Deploy api to staging?")
    .title("Release 0.9")
    .from_agent(agent.id.clone())       // shown by its name; needs agents.read
    .suggest_with("Deploy", "Ship 0.9 to staging now")
    .suggest("Not now")
    .blocking()])?;
```

An answer arrives as `Event::AskAnswered(Answer { id, picks, text, declined })`, `picks` by place from 0. Reply with `asks::reply(id, None)` when it was taken, then send a set without the question; or with `asks::reply(id, Some("why"))` to refuse it: the card says "Not sent: why" and the user can answer again. Until the reply, the card says "Sending…".

A title is at most 200 characters, a question 2 000, a suggestion's label 80 and its description 200, and an ask has at most 8 suggestions; longer text is cut. Pitwall keeps the questions while the tile is hidden or let go to save memory, and an answer to a tile that was let go starts it again first.

### In tests

The harness has a fake Pitwall side: `Harness::agent(agent("a1", "web"))` (or `waiting_agent`) adds an agent, `set_agents(…)` changes them and sends `AgentsChanged`, `queued()` lists what the tile queued, `badges()` and `asks()` give its newest sets, `answer(id, &[0])`, `answer_text(id, "…")` and `decline(id)` answer, and `reply(id)` says what the tile replied.

```rust
let mut t = Harness::build(env!("CARGO_MANIFEST_DIR")).grant("agents.read").agent(waiting_agent("a2", "api")).open("main");
assert_eq!(t.asks()[0].from.as_deref(), Some("a2"));
t.answer("deploy", &[0]);
assert_eq!(t.reply("deploy"), Some(None));
```

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
| 2 | A toolbar item with a badge, which the user can hide |
| On demand | Files and git in your projects, file pickers, the clipboard, starting agents, and more, each when a tile needs it |
