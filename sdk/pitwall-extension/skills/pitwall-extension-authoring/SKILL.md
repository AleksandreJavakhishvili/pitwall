---
name: pitwall-extension-authoring
description: "How to build, test and check a Pitwall tile (a Rust WebAssembly component made with the pitwall-extension SDK): the project layout, extension.json, the Tile trait and elements, content and show tools for agents, storage and settings, the test harness, and the pitwall extension commands. Use it whenever you change this extension."
---

# Building a Pitwall tile

This project is a **Pitwall extension with a tile**: Rust, built to a
WebAssembly component for `wasm32-wasip2`, that Pitwall runs in a sandbox and
draws in its own theme. The tile describes what to show as a tree of
elements; Pitwall lays it out and sends back what the user did. It never
draws pixels and can't reach files, the network or other programs (WIT
`pitwall:tile@0.1.0`, a preview: it may change before 1.0).

## Commands

| Command | Does |
|---|---|
| `rustup target add wasm32-wasip2` | Once per machine: the only toolchain addition needed (no cargo-component) |
| `pitwall extension dev` | Build (`cargo build --release --target wasm32-wasip2`), copy the component to `tile.wasm`, check it, load it into the running Pitwall; again on every save. `--once` does one round |
| `pitwall extension check .` | Offline: the manifest, the component's world and imports, limits, input schema, permission reasons. Exit status 1 when something fails |
| `cargo test` | The tests in `tests/`: they build `tile.wasm` and run it in Pitwall's own tile runtime |
| `pitwall extension new <id> --template blank\|list\|form` | A new project like this one |
| `pitwall extension pack` | Offline: `<id>-<version>.pwtile` from the built project (manifest, `tile.wasm`, README, input schemas, assets; never sources). The same files always give the same bytes |
| `pitwall extension sign <file>` | Offline: `<file>.minisig` with the user's minisign key. Only the user signs: never create, read or commit a secret key |

Run `pitwall extension check .` and `cargo test` before you say a change is
done.

## Layout

- `extension.json`: the manifest (below).
- `src/lib.rs`: the tile. `crate-type = ["cdylib"]` in `Cargo.toml`.
- `tests/*.rs`: tests with `pitwall-extension-test`.
- `input.schema.json` (if the tile takes content): JSON schema of what the
  tile can be opened with.
- `tile.wasm`: built; not committed.

## The manifest (`extension.json`)

```json
{
  "v": 2,
  "id": "my-tile",
  "name": "My tile",
  "version": "0.1.0",
  "pitwall": ">=0.9.0",
  "icon": "grid",
  "description": "One line",
  "settings": [ { "key": "repo", "type": "text", "label": "Repository", "default": "" } ],
  "permissions": { "memory": { "reason": "To hold large diagrams", "mib": 128 } },
  "contributes": {
    "tiles": [ { "id": "main", "title": "My tile", "icon": "grid", "wasm": "tile.wasm",
                 "world": "pitwall:tile@0.1.0", "input": "input.schema.json" } ]
  }
}
```

- `id`: lowercase letters, digits and hyphens; the folder is named after it.
- Every tile entry names the same `wasm` (one component per extension); each
  tile id is a view the component renders.
- **Permissions**: every permission needs your reason, one line the user
  reads before granting it. Ask only for what the tile uses:
  - `agents.read` (list your agents), `agents.queue` (put text in an
    agent's Next up), `projects.read` (badges on project rows), `notify`,
    `show`: the reason alone;
  - `net`: `{ "reason": "…", "hosts": ["api.example.com", "*.example.org"] }`
    (https only; `http` only to this computer, when listed with its port);
  - `memory` (more than 64 MiB, at most 512) and `sql` (more than 100 MiB,
    at most 2048): `{ "reason": "…", "mib": 128 }`, granted with the
    extension.
- `"badges": true` and `"asks": true` in `contributes` let the tile put
  badges on sidebar rows and questions in the floating control.
- **Toolbar button** (at most one): `"contributes": { "toolbar": { "icon":
  "server", "tooltip": "Builds", "badge": true, "action": { "kind":
  "popover", "tile": "main", "size": { "w": 320, "h": 400 } } } }`; `kind` is
  also `tile`, `command` or `menu`. The user can hide it; an update never
  shows it again.
- Setting types: `text`, `url`, `number`, `bool`, `choice`, `secret` (kept
  in the system's secure store, read with `settings::secret`).

## The tile

```rust
use pitwall_extension::prelude::*;

#[derive(Default)]
struct App { clicks: u32 }

impl Tile for App {
    fn new(cx: &mut Cx<Self>) -> Self { App::default() }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        v_flex().gap_2().p_3()
            .child(text(format!("Clicked {} times", self.clicks)).text_color(Token::Text2))
            .child(button("Click me").primary().on_click(cx.listener(|this, _, cx| {
                this.clicks += 1;
                cx.notify();                       // render again
            })))
    }
}

pitwall_extension::export!(App);
```

- **Elements** read like GPUI: `div()`, `h_flex()`, `v_flex()`, `text(..)`
  (`.child(text(..))` for inline runs), `button(..)` (`.primary()`,
  `.danger()`, `.small()`, `.link()`, `.icon(Icon::Plus)`,
  `.disabled(b)`), `icon_button(Icon::X, "Remove")`, `icon(..)`,
  `input("key")` (`.placeholder`, `.value`, `.multiline()`, `.rows(n)`,
  `.on_change`, `.on_submit`). Scroll with `.overflow_y_scroll()` on a
  `div`.
- **Styles** by GPUI's names: `flex`, `flex_col`, `flex_1`, `gap_1`…`gap_8`,
  `p_2`, `px_3`, `size_full`, `w(px(120.0))`, `items_center`,
  `justify_between`, `border_b_1`, `rounded_md`, `text_sm`, `font_mono`,
  `font_semibold`, `truncate`. **Colours are theme tokens only**:
  `.text_color(Token::Text2)`, `.bg(Token::Surface2)`,
  `.border_color(Token::Line)`, `Token::Good`/`Warn`/`Bad`. No raw colours.
- **Keys**: `.id("key")` on nodes that move (list rows: the item's id).
  Pitwall keeps scroll, focus and typed text by key path; text fields always
  have a key (`input("key")`).
- **Listeners**: `cx.listener(|this, event, cx| …)`, made while rendering and
  handed to the element at once. `on_click` takes `Click`; `on_change` takes
  `&str` (annotate it: `|this, v: &str, cx|`) or `InputChange`; `on_submit`
  takes `Submit`; `on_blur` (`Blur`) fires when a field loses the keyboard,
  the time to save it. Call `cx.notify()` when what shows changed.
- **Buttons**: look `.primary()`, `.danger()`, `.link()` and size `.small()`
  combine (`.small().danger()`). `icon_button(Icon::Trash, "Delete")` is
  square. `Icon` lists every icon Pitwall has; `Icon::Named` with another
  name draws a dot.
- **Hover**: `.group("row")` on a row and `.visible_on_group_hover("row")` on
  its Delete shows the Delete only under the pointer (Pitwall draws it).
- **Keys**: `cx.bind_key("n", ..)` while rendering. Pitwall's own keys
  (Esc, Tab, `cmd-` with most letters and digits, `cmd-enter`, …: the list is
  in ui.md) are dropped with a Log line, so bind `cmd-s` or plain keys. A
  key without ⌘/Ctrl/Alt doesn't fire while a text field has focus.
  `cx.focus("list/head/search")`: the key path without the root's.
- **Text fields**: Pitwall owns what the user types. Keep your copy from
  `on_change` and pass it back with `.value(..)`: the SDK sends it only
  when it differs, so setting your copy to `""` clears the field.
- **Canvas** (diagrams, charts): `canvas().id("d").flex_1().transform(self.viewport)`
  with `.draw(Shape::rect(..).fill(Token::Surface2).stroke(Token::Line, 1.).radius(8.))`,
  `Shape::line`/`Shape::path()` (`.dash([5., 4.])`) and
  `Label::new(text, x, y).size(14.).center().middle().max_width(w)` (px sizes,
  so text zooms). Clickable boxes are `.region(Region::new(key, x, y, w, h, label).on_click(..))`.
  Keep a `Viewport` in the state and feed it from `.on_pointer` and `.on_wheel`
  (`if t.viewport.on_pointer(p) { cx.notify() }`); `fit`, `zoom_at` for buttons.
  No text-measuring call: measure with an Inter width table.
- **Drag and drop** inside a tile: `.draggable(kind, data, label)` on the source,
  `.on_drop([kind], handler)` (`Dropped { kind, data, at }`) on the target.
- **Code and diff views** (read-only, Pitwall's own): `code_view(text).language("rust")`
  with `.mark(range, Mark::Added)`, `.reveal(line)`, `.on_line_click`, `.on_select`;
  `diff_view(old, new)` or `diff_view_unified(patch)`, `.inline()`. Give them a size.
- **Components**, drawn by Pitwall (prefer them to hand-made ones):
  - Long lists: `virtual_list("rows", count, cx.rows(|this, i, cx| row))`,
    each row keyed by its item (`.id(item.id)`, no `/` in a key), small
    (boxes, text, icons, buttons, images, a context menu), one
    `.row_height`. `.on_select(cx.listener(|this, e: &Picked, cx| …))`
    (`e.activate` for a double click or Enter), `.selected([key])`,
    `.revision(n)` when the data has a version. Give its parent a height.
  - `table("t", vec![column("name", "Name").sortable(), column("size",
    "Size").width(90.0).align_end()], count, rows)`: a row's root holds one
    child per column; `.sort(col, dir)`, `.on_sort`, `.on_column_resize`.
  - `tree("t", visible_count, cx.rows(|this, i, _| tree_row(el).depth(d)
    .branch(open)))`: flatten the open branches; `.on_toggle` (`Toggled`).
  - `tabs("k", [tab("a", "A")])`, `select("k", [choice("s", "Small")])`
    with `.selected` and `.on_select` (`&str` key).
  - `menu_button("Actions", vec![menu_item("copy", "Copy").icon(Icon::Copy),
    separator(), menu_item("del", "Delete").danger()])` and
    `.context_menu(entries, listener)` on any element (rows too).
  - `markdown(text)`: inert; `.on_link` (`LinkClick`), `.on_toggle` (a
    task's line and state). `image_asset("assets/x.svg")`,
    `image_data(name)` after `ui::put_image(name, &bytes)`; no network.
- **More components** (Pitwall's own controls, same look; "kit parity").
  Controls are controlled: give the value, hear the change in `on_change`,
  store it, `cx.notify()`. Every one has `.disabled(true)`; name ones
  without text with `.label("…")`.
  - Form: `checkbox("Only failures").checked(on).mixed(some)`,
    `switch("Notify me").checked(on)` (`on_change`: `&bool`);
    `radio_group()` / `segmented()` with `.option(key, label)`,
    `.option_disabled()`, `.selected(key)`, `.horizontal()` (`&str`);
    `slider(v, 0.0..=100.0).step(5.0).show_value()` (`&f64`);
    `number_field(Some(2.)).min(0.).max(10.).step(1.).decimals(0)
    .placeholder("None").readonly(b)` (`&Option<f64>`, `None` when emptied);
    `search_field("q")` (an `input` with a magnifier and a clear button);
    `form_row("Branch").required().help("…").error(msg_option).inline()
    .child(control)`.
  - Feedback: `progress(Some(0.6))` or `progress(None)` (`.tone(Tone::Good)`),
    `spinner()`, `skeleton(SkeletonShape::Lines(3) | Block | Circle)`,
    `empty_state("No runs yet").icon(..).text(..).action("Refresh", listener)`,
    `banner(Tone::Bad, text).title(..).action(..).on_dismiss(listener)`,
    `badge("3").tone(Tone::Bad)`, `chip("web").icon(..).selected(b)
    .on_click(..).on_remove(listener)`, `kbd("cmd-k")`, `.tooltip("…")` on
    anything.
  - Layout: `h_split()` / `v_split()` with `.first(a).second(b)
    .initial(0.4).min_sizes(120., 160.).on_resize(&f64)` (the user's size is
    kept per view); `disclosure("Env", open).detail("3").on_toggle(&bool)
    .child(..)`; `card().title(..).detail(..).on_click(..)`; `divider()`,
    `vertical_divider()`; `breadcrumbs().crumb(key, label).on_pick(&str)`;
    `toolbar().child(..)`; `popover().placement(Placement::Below).width(240.)
    .on_dismiss(..)` as a child of the box holding its button, rendered
    while open; `confirm_sheet("Delete 3 notes?", "Delete").text(..).danger()
    .on_confirm(..).on_cancel(..)`, rendered while asking (tiles can't open
    system dialogs).
  - Data: `key_values().entry("Commit", sha).mono().copyable()`,
    `avatar("Ada Lovelace").small()/.large()`, `relative_time(system_time)` /
    `relative_time_ms(ms)` (Pitwall keeps it current: no timer),
    `copyable(text).mono()` (Pitwall copies on the user's click),
    `link("Open in CI").url("https://…")` (opens a web tile with the `show`
    permission; else its `on_click` hears it).
  - Charts (data only, theme colours, tooltips): `sparkline(values).color(Token::Good)
    .filled()`, `bar_chart(labels).series(name, values)
    .series_colored(name, values, Token::Bad).stacked().unit("runs")`,
    `line_chart(labels).series(..).unit("ms").min(0.).max(..)`,
    `status_timeline().segment(key, label, Some(Tone::Good))
    .segment_weighted(key, label, tone, w).on_pick(&str)`. Give each a
    height and a `.label("…")`.
- **Render quickly** (under 8 ms; the deadline is 250 ms): compute in
  listeners and timers, read state in `render`. `handle` calls have 1 s,
  `new` 2 s. A tile over a deadline is stopped and the user sees why.
- **Limits**: 20 000 nodes and about 2 MiB a frame, 64 MiB of memory, 32
  timers (≥ 100 ms), 10 MiB of storage, 3 toasts a minute.

## Content from agents and the user

A tile entry with `"input": "input.schema.json"` can be opened with content
matching that schema (an agent showing something to the user). Pitwall checks
it against the schema, then:

```rust
#[derive(serde::Deserialize)]
struct Shown { title: Option<String>, items: Vec<Item> }

fn on_content(&mut self, view: &ViewInfo, cx: &mut Cx<Self>) {
    if let Some(c) = &view.content {
        match c.parse::<Shown>() {
            Ok(shown) => { /* replace what the view shows */ }
            Err(e) => log::warn(e.to_string()),
        }
    }
}
```

The same content id again updates the same view (`on_content` again). Keep
the schema small and describe each field: agents read it.

## Tools for agents

Every agent Pitwall starts gets `<extension-id>__open` for your tiles
(content: the `"input"` schema; `"agents": false` on a tile entry keeps it
from agents). Declare show tools of your own in the tile entry, kind `show`
(opens or focuses, maybe at something) or `present` (puts content in):

```json
"tools": [{ "name": "show_message", "kind": "present",
            "description": "Show a short message (the same item replaces it).",
            "input": { "type": "object", "required": ["message"],
                       "properties": { "item": { "type": "string" }, "message": { "type": "string" } } } }]
```

and write each as a method of a `#[tools]` block of the same name and kind
(`export!` finds it):

```rust
use pitwall_extension::tools;
use pitwall_extension::tools::{Call, Problem, Reply, ToolError};

#[derive(serde::Deserialize)]
struct Message { message: String }

#[tools]
impl MyTile {
    #[tool(kind = "present")]
    fn show_message(&mut self, call: Call<Message>, cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        if call.input().message.trim().is_empty() {
            return Err(ToolError::problems("Nothing was shown.", [Problem::new("`message` has no text")]));
        }
        self.shown.insert(call.key(), call.input().message.clone()); // per agent and item
        cx.notify();
        Ok(Reply::text("Shown"))
    }
}
```

- Names: lowercase, digits, `_`, at most 32, not `open`; `<id>__<name>`
  at most 50. `where` and `of` are Pitwall's (placement), not yours.
- An error shows nothing new; give each input problem with
  `Problem::at(line, column, msg)`.
- When the user edits what an agent showed, call
  `tools::user_edited(call.origin())` (or `content.call()` for `__open`
  content): Pitwall then refuses that agent's replacing that item.
- Limits: 1 MiB input, 10 s per call, 10 calls in 10 s; the reply's text
  at most 4 KiB.
- Test with `tile.call("claude", Some("show_message"), Some("a"), json!({..}))`
  (`None`: `__open`), which returns `Called::Shown`, `Replied`, `Failed` or
  `Refused`. The harness refuses more than 10 calls in 10 s as Pitwall
  does: advance its clock between calls, or start from stored data
  (`Harness::stored_json`).
- A worked `present` tool: the C4 tile's `c4__show` in Pitwall's
  repository (`tiles/c4/src/tile.rs`, `extension.json`): input read all
  or nothing, items per agent, an update in place, problems with lines.

## Services (no permission needed)

- `storage`: the tile's own key-value store on disk (`get`, `set`,
  `delete`, `list`, `get_json`, `set_json`; `entries_json(prefix)` reads
  every record under a prefix in a couple of calls). What the user must not
  lose goes here.
- Clocks: `std::time::Instant` and `SystemTime` work. Don't
  `thread::sleep`; use a timer.
- `settings`: `text(key)`, `number`, `flag`, `secret(key)`, `open()`.
  `Tile::on_event` gets `Event::SettingsChanged` (by default it renders
  again).
- `ui`: `toast(text, Tone::Good)`, `open_view(tile)`, `scroll_to(view,
  key_path)`, `put_image(name, &bytes)`, `drop_image(name)`.
- `log`: `debug`, `info`, `warn`, `error` (and `println!`), shown in the
  tile's Log and by `pitwall extension dev`.
- Timers: `cx.after(Duration::from_secs(5), |this, cx| …)`,
  `cx.every(..)`, `cx.cancel(timer)`. They pause while the tile is hidden.
- State across a reload: `fn save_state(&self) -> Option<SavedState>` with
  `SavedState::json(&…)`, read back in `new` with `cx.restore::<T>()`.
- `sql`: the extension's SQLite database (100 MiB). Run
  `sql::migrate(&[..])` in `new`; `sql::execute(stmt, (a, b))`,
  `sql::query(stmt, ())` → `Rows` (`row.get::<i64>("n")`),
  `sql::transaction(|| …)` (one call long). No `ATTACH`, no writing
  `PRAGMA`s.
- `toolbar`: `set_badge(Badge::Count(n), Tone::Attention)`, `set_tooltip`,
  `set_shown`, `close_popover()`; the popover is the view
  `toolbar::POPOVER_VIEW`; a press is `Event::ToolbarClicked { project }`.
- `service` (an extension with a `service` block): `service::info()`,
  `service::url("/api/x")` for `cx.fetch` (no `net` needed), `restart()`,
  `open_card()`; `Event::ServiceChanged(info)`.

## Agents, badges and questions (granted)

- `agents::list()` (`agents.read`): id, name, kind, project, branch,
  state, `waiting_on_you`, `queued`. `Event::AgentsChanged` says to read
  again.
- `agents::queue_add(&id, text)` (`agents.queue`): into the agent's Next
  up, marked "From <name>", waiting for the user's Send. ≤ 4 000
  characters; 3 waiting per agent; one per agent every 10 s (`Busy`).
- `badges::set(&[Badge::count(Row::Agent(id), 2).tone(Tone::Warn)])`
  (`"badges": true`): the whole set each time, `&[]` clears.
- `asks::set(&[Ask::new("id", "Question?").suggest("Yes").blocking()])`
  (`"asks": true`): shown in the floating control. The answer arrives as
  `Event::AskAnswered(Answer { id, picks, text, declined })`; reply with
  `asks::reply(&id, None)` (taken) or `Some("why")` (refused), then set
  the questions without it.
- Harness: `.grant("agents.read")`, `.agent(agent("a1", "web"))`,
  `set_agents`, `queued()`, `badges()`, `asks()`, `answer(id, &[0])`,
  `answer_text`, `decline`, `reply(id)`.

## Network, notifications and opening things (granted)

- `net`: `cx.fetch(net::Request::get(url).header(..), |this, answer, cx|
  …)?` (answer: `Result<Response, net::Error>`; `r.json::<T>()`,
  `r.text()`); 8 at once, 30 s by default. Only the manifest's hosts.
- `notify::send(title, body)`: 3 a minute, repeats within 10 minutes
  dropped.
- `show::file(path, Some(line))`, `show::web(url)`.

Without the grant a call fails with `HostError::Denied` (`net::Error::
Denied`); carry on and say what is missing. There is no file or program
access, and never agents' terminals; don't try (no `std::fs`, no raw
sockets): they fail.

## Tests

```rust
use pitwall_extension_test::Harness;

#[test]
fn counts() {
    let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR"))   // builds tile.wasm
        .setting("repo", "octo/app")
        .open("main");                                          // or .open_with("main", json!({..}))
    tile.click_text("Click me");                                 // also click_key("list/row-1"), click_label("Remove")
    assert!(tile.find_text("Clicked 1 times").is_some(), "{}", tile.text_snapshot());
    tile.type_into("filter", "abc").submit("filter");
    assert_eq!(tile.stored_json::<u32>("clicks"), Some(1));
}
```

Each action waits until the tile handled it and rendered. Time is virtual:
`advance(duration)` moves it (timers, `Instant` and `SystemTime`), so tests
never sleep; `wait_for_text(text, timeout)` moves it until the text shows.
`text_snapshot()` prints the tree (with each field's text and focus);
`field("key")` reads a field; `focus_key`, `blur()`, `focus(false)` move the
keyboard; `press("n")` a bound key; `resize(view, w, h)`, `light()`;
`log_text()`, `toasts()`, `stored(key)`, `Harness::stored(key, value)`;
`reload()` (save state, restart), `restart()` (no saved state),
`set_setting`, `set_content`.
Canvases and blocks: `canvas(key)`, `click_region(label)`, `hover_region`,
`wheel`, `pinch`, `drag_pointer`, `drag(from, to)`, `code_view(key)`,
`click_line(key, Side::New, n)`, `select_code`.
Components: `rows(list, start, count)` (each
`ListRow` with its key, text, depth, expanded), `find_row`, `select_row`,
`activate_row`, `expand_row`/`collapse_row`, `click_in_row`,
`choose_row_menu_item`, `choose_menu_item(key_path, label)`,
`select_option`, `select_tab`, `sort_by`, `resize_column`, `click_link`,
`toggle_task` (each takes the component's key path, or its own key).
More components: `toggle` (checkbox, switch, disclosure), `choose(key, option)`
(radio group, segmented control, breadcrumbs, status timeline), `set_slider`,
`type_number(key, "12")` (returns whether it sent), `step_number(key, up)`,
`resize_split(key, 0.7)`, `dismiss` (banner, chip, popover, confirm sheet),
`key_on(key, "space")` (a key as Pitwall handles it on that component),
`copy(key)` then `clipboard()`, `open_link(key)` then `opened_pages()`
(with `.grant("show")`), `clear_search`. The snapshot shows each one's
state (`checkbox "Only failures" on`, `slider 75 in 0..100`).

Services in tests: `Harness::grant("net")` (or `grant`/`revoke` while it
runs), `respond(url, status, body)` and `respond_error` (nothing leaves the
test), `requests()`, `notifications()`, `ui_requests()` (files and pages
opened), `toolbar()`, `click_toolbar(None)`, `click_in_popover("close")`,
`Harness::own_server("127.0.0.1:8790")`, `set_service(info)`,
`database()` (the SQLite file, in the test's temporary folder).

## Docs

The design and reference: `docs/spec/tile-sdk/` in Pitwall's repository
(`authoring.md`, `runtime.md`, `ui.md`, its component catalogue in 3.10)
and the WIT files in the SDK's `wit/` folder, every item documented. The
website's Extensions pages have a page per component group (Form controls,
Feedback, Layout, Data display, Charts) with screenshots and examples.
