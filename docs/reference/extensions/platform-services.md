# Platform services

**Preview.** What Pitwall gives every tile, so authors don't build it themselves. Without a permission, each reaching only the tile's own things: timers (`tasks`), a key-value store (`storage`), settings and secrets (`settings`), the Log (`log`), requests about the tile's own views (`ui`), a SQLite database (`sql`), a toolbar button (`toolbar`) and the extension's own server (`service`). With a permission: the network, to the hosts the manifest names (`net`), notifications (`notify`), opening a file or a web page (`show`), the files of the project a view is open in (`files`), and the agents' questions group: your agents and their Next up (`agents`), badges on sidebar rows (`badges`) and questions in the floating control (`asks`) ([below](#agents-badges-and-questions)).

Pitwall adds a service only when a tile can't reasonably do without one. Each is checked by Pitwall, not by the SDK: a call without its permission fails with `Denied` and a line in the Log.

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
| `ui::put_image(name, &bytes)` | Hands Pitwall an image (PNG, JPEG, GIF, WebP or SVG, at most 8 MiB) for `image_data(name)` ([Images](ui.md#images)). Kept until put again or dropped; at most 64 MiB for all of a tile's. `Invalid` when the bytes aren't one of those formats, `TooLarge` over a limit |
| `ui::drop_image(name)` | Forgets an image put under `name` |

The `form` template saves a note and says so in a toast:

```rust
{{#include crates/pitwall-cli/templates/tile/form/lib.rs.tmpl#form-save}}
```

In tests, `toasts()` lists the toasts and `ui_requests()` the other requests.

## Network

`net` fetches from the hosts the manifest names, with the `net` permission ([Permissions](permissions.md#net)). Declare them with the reason:

```json
"permissions": {
  "net": { "reason": "To read your builds from the CI server", "hosts": ["ci.example.com", "*.example.org"] }
}
```

A host is a name (`api.example.com`), a name with a port (`api.example.com:8443`), or `*.example.org` for its subdomains (not `example.org` itself). At most 32. Only `https` reaches them; `http` only to this computer (`localhost`, `127.0.0.1`, `[::1]`), and only when the list names it with its port.

A request is made on the context, since its answer calls back into the tile:

```rust
let req = net::Request::get("https://ci.example.com/api/builds?limit=20").header("Accept", "application/json");
cx.fetch(req, |tile, answer, cx| {
    match answer.and_then(|r| r.json::<Vec<Build>>()) {
        Ok(builds) => tile.builds = builds,
        Err(e) => tile.error = Some(e.to_string()),
    }
    cx.notify();
})?;
```

`cx.fetch` returns a `RequestId` at once, or an error when the request is refused before anything is sent (`Denied` names the host). The answer comes later: `Ok(Response)` for any status (`ok()` is 200 to 299), or `Err(net::Error)`: `TimedOut`, `Failed` (no connection), `TooLarge`, `Denied` (a redirect to a host not listed). `cx.cancel_fetch(id)` drops one.

| Limit | |
|---|---|
| At once | 8 requests |
| Request body | 8 MiB |
| Answer | 32 MiB |
| Time | 30 s, at most 120 s with `.timeout(…)` |
| Redirects | 5, each checked against the hosts again |

Pitwall checks the address before looking up the name, and again on every redirect. It sends no cookies and no credentials of its own, and uses no proxy. There are no raw sockets.

In tests, `Harness::respond(url, status, body)` answers a request (the whole address, query included), `respond_error(url, error)` fails it, and nothing leaves the test; others get a 404. `requests()` lists what the tile sent. The tile still needs the grant (`grant("net")`) and the host in its manifest, as in Pitwall.

## Database

`sql` is a SQLite database run by Pitwall, one per extension, kept outside its folder across restarts and updates. No permission: 100 MiB. A larger one, at most 2048 MiB, is asked for with the `sql` permission and granted with the extension, shown with its size: `"sql": { "reason": "To keep a year of builds", "mib": 500 }`.

Declare the schema as migrations, in order, and run them when the tile starts. Pitwall keeps the schema's version and runs each step not yet run in a transaction of its own:

```rust
const MIGRATIONS: &[&str] = &[
    "CREATE TABLE builds (id INTEGER PRIMARY KEY, name TEXT NOT NULL, state TEXT NOT NULL)",
    "ALTER TABLE builds ADD COLUMN url TEXT",
];

fn new(cx: &mut Cx<Self>) -> Self {
    if let Err(e) = sql::migrate(MIGRATIONS) {
        log::error(format!("the database couldn't be set up: {e}"));
    }
    // …
}
```

| Call | Does |
|---|---|
| `sql::execute(statement, params)` | Runs one statement; the rows it changed |
| `sql::query(statement, params)` | Runs one query; its `Rows` (`row.get::<i64>("n")`, or by index) |
| `sql::transaction(\|\| …)` | Runs the closure in a transaction: committed when it returns `Ok`, rolled back on `Err` |
| `sql::migrate(steps)` | Runs the steps not run yet; the schema's version |
| `sql::usage()`, `sql::quota()` | The bytes it takes and may take |

Parameters are `()`, a tuple (`("a note", 3)`) or a `Vec<sql::Value>`; `?` marks each place. A query returns at most 10 000 rows and 8 MiB; page with `LIMIT`. A write past the quota fails with `TooLarge`. A transaction lasts one call into the tile: one left open when the call returns is rolled back, with a line in the Log.

The database can't reach anything else: no `ATTACH`, no extensions, no `PRAGMA` that changes how the file is kept (`table_info`, `foreign_keys` and other read-only ones work). A statement still running at the call's deadline is stopped (`Busy`).

In tests the database is in the test's temporary folder; `database()` gives its file.

## Notifications

`notify::send(title, body)` shows a notification, with the `notify` permission. It is the system's notification when you aren't looking at Pitwall (as for an agent that needs you) and a toast in Pitwall's window, with Open. At most 3 a minute; the same words within 10 minutes are shown once. A title is at most 80 characters, the text 200.

In tests, `notifications()` lists what it sent.

## Opening a file or a page

With the `show` permission:

| Call | Does |
|---|---|
| `show::file(path, line)` | Opens the file (a whole path) in Pitwall's editor, at the line when given |
| `show::view(path, line)` | Shows the file in Pitwall's file viewer, scrolled to the line, as an agent's `show_files` does: with its git repository's tree (else its folder's) and git status, in the extension's own viewer window, which opens behind the front window without taking the keyboard. Later calls add tabs to it |
| `show::web(url)` | Opens the page in a new web tile, in the extension's own browser store. `https` only, `http` only to this computer |

None gives the tile anything back. In tests, `ui_requests()` lists them (`OpenFile`, `OpenWeb`, `ViewFile`).

## Files in the project

A view is open in a project: its space's, the one of the agent it was opened next to, or the one whose sidebar menu opened it. `files` reaches that project's folder, and the extension's own files folder. A tile never names a folder of its own.

| Call | Does |
|---|---|
| `files::project(view)` | The view's project, `Project { name, folder }`, or `None` (no project, or neither `files.read` nor `projects.read`) |
| `files::read(&root, path)`, `read_text` | A file's bytes, or its text |
| `files::stat(&root, path)` | An `Entry`: `path`, `kind` (`File`, `Folder`), `size`, `modified` (ms) |
| `files::list(&root, folder, deep)` | The entries in a folder (`""` for the root), every level down with `deep`, sorted by path |
| `files::watch(&root, folder)`, `unwatch(id)` | Follow a folder and everything under it: `Event::FilesChanged { watch, paths }`, changes close together in one event (a quarter second quiet, at most a second late) |
| `files::write(&root, path, bytes)`, `write_text` | Write a file whole (made with its folders when missing, replaced at once, never half written) |
| `files::create(&root, path, bytes)` | Write a new file; `Exists` when one is there |
| `files::make_folder(&root, path)`, `rename(&root, from, to)` | A folder with its parents; move a file or folder (`Exists` when the target is there) |
| `files::delete(&root, path)` | To the Trash, in a project; for good, in its own folder |

`Root::project(view)` is the view's project; `Root::Data` the extension's own files folder, kept across updates, at most 100 MiB, needing no permission. Reading a project (`project`, `read`, `stat`, `list`, `watch`) needs `files.read`; changing it needs `files.write`, which starts off. During an agent's `show`, `present` or `read` call, writes are refused.

Paths are relative, with `/` between parts. Pitwall refuses (`NotAllowed`, with why) an absolute path, `..`, a link that leads out of the root, anything in `.git` (never listed, read or changed), and a change to Pitwall's own `.pitwall.json` or `.pitwall/`. `list` and `watch` leave out what the project's `.gitignore` files and `.git/info/exclude` ignore; `read` and `stat` still reach those files by name. A file read or written is at most 8 MiB (`TooLarge`), a list at most 10 000 entries, and a tile has at most 8 watches.

```rust
let root = files::Root::project(view);
for e in files::list(&root, "docs", true)?.iter().filter(|e| e.path.ends_with(".md")) {
    let text = files::read_text(&root, &e.path)?;
    // …
}
let watch = files::watch(&root, "docs")?;
```

When the view moves to another project, or out of every project, the tile hears `Event::ProjectChanged(view)`: its watches there have ended; read the new project. Every change to a project, and every refusal, is a line in the extension's Activity.

In tests, `Harness::project("shop")` starts the tile's view in a project folder of the test's, `project_file(path, text)` puts files in it, and `.grant("files.read")` grants it. Then `write_project_file`, `remove_project_file` and `rename_project_file` change it (the watches hear it at once), `project_file(path)` reads it back, `trashed()` lists what went to the Trash, `watches()` counts the watches, `move_to_project(view, Some("api"))` moves a view, and `activity()` lists its Activity lines.

## Toolbar button

An extension may put one button in Pitwall's top bar, in a group of its own after the counts. Declare it in `contributes.toolbar`; the user can hide it, and an update never shows it again ([Tiles and UI](ui.md#toolbar-button)).

| Call | Does |
|---|---|
| `toolbar::set_badge(badge, tone)` | Its badge: `Badge::None`, `Dot` or `Count(n)`; `Tone::Normal` or `Attention`. At most 4 changes a second |
| `toolbar::set_project_badge(project, badge, tone)` | A project's badge, for a button with `"scope": "project"` (needs `projects.read`) |
| `toolbar::set_tooltip(text)` | Its tooltip, at most 80 characters |
| `toolbar::set_shown(shown)` | Hides or shows the button for a while (at most one change in 5 s); the user's Hide wins |
| `toolbar::close_popover()` | Closes its popover |

The badge shows only while the tile runs. With `"badge": true`, Pitwall keeps the tile running with no view open, so the badge stays current. A press runs its action; the tile hears `Event::ToolbarClicked { project }`. Its popover is a view of the tile, `toolbar::POPOVER_VIEW`: render it in `render` when `view` is that id.

In tests, `toolbar()` gives the badge, tooltip and whether the popover is open; `click_toolbar(project)` presses the button and `click_in_popover(key)` clicks inside the popover.

## Its own server

An extension with a `service` (a program Pitwall keeps running, such as the server its agents talk to) reaches that server from its tile without `net` ([Servers in extensions](servers.md)):

| Call | Does |
|---|---|
| `service::info()` | Its state (`Up`, `Starting`, `Off`, `NeedsInstall`, `Stopped`, `Failed`), its address, its version, and why when it isn't up |
| `service::url(path)` | The address with `path`, for `cx.fetch`; `None` when it doesn't listen |
| `service::restart()` | Starts it again, as Restart on the card does |
| `service::open_card()` | Opens the extension's card, where the user installs and starts it |

The tile hears `Event::ServiceChanged(info)` when its state changes.

In tests, `Harness::own_server("127.0.0.1:8790")` says it is up there, and `set_service(info)` changes its state.

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

- **Programs, and files outside its project.** `std::fs` fails; use `files`, `storage` or `sql`.
- **Raw sockets**, and the network beyond the hosts the manifest names.
- **Agents' terminals**, at any grant. They carry secrets and prompts.
- **Other extensions' data, settings or secrets.**
- **Background work while hidden**, except a tile kept running for its toolbar badge. Work that must go on belongs in the extension's server.

## What comes later

| Phase | Services |
|---|---|
| On demand | Git in your projects, file pickers, the clipboard, starting agents, and more, each when a tile needs it |
