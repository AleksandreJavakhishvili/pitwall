# Tile SDK: authoring

Part of the [Tile SDK and marketplace](README.md) design. The SDK crate, its examples, the CLI, dev reload, the test harness and docs for authors and their agents. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 4. The SDK crate (`pitwall-extension`)

### 4.1 Shape
- `Tile` trait: `fn new(cx: &mut Cx<Self>) -> Self`,
  `fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement`,
  optional `save`/`restore` (serde), `on_theme`, `on_visible`.
- Builders that read like GPUI: `div()`, `h_flex()`, `v_flex()`,
  `.flex()`, `.gap_2()`, `.p_3()`, `.size_full()`, `.child()`,
  `.children()`, `.text_color(Token::Text2)`, `.bg(Token::Surface)`,
  `.when(cond, |d| …)`, `.id("key")`. Same spacing scale names as GPUI
  (`gap_1`…`gap_8`) so GPUI code ports by search and replace.
- Components as functions returning builders: `button("Re-run")
  .primary()`, `icon_button(Icon::Refresh, "Refresh")`, `input("q")
  .placeholder("Filter")`, `toggle(..)`, `select(..)`, `tabs(..)`,
  `virtual_list("runs", count, row_h, |i, cx| …)`, `table(..)`,
  `tree(..)`, `code(text, "rust")`, `markdown(text)`, `canvas(..)`,
  `status(..)`, `chip(..)`, `split(..)`.
- Listeners like GPUI's: `.on_click(cx.listener(|this, ev, cx| …))`.
- Async: `cx.spawn(async move |this, cx| { let r = net::get(url).await?; this.update(cx, |t, cx| …) })`.
  `net`, `files`, `git`, `pitwall` are async modules.
- `pitwall_extension::export!(MyTile);` generates the WIT glue
  (`wit-bindgen`).
- Typed helpers for the method table (`pitwall::agents()`,
  `pitwall::queue(agent, text)`), generated from `METHODS` like
  `methods.md` [today].
- Agent tools ([section 11](agent-tools.md#11-tools-for-agents)): `#[pitwall_extension::tools] impl MyTile { … }`
  with one `#[tool(kind = …)]` method per tool. Input and output types
  derive `serde` and `schemars::JsonSchema`; doc comments become the
  descriptions agents read. The macro writes the tool list (names,
  kinds, descriptions, schemas) into a custom section of the wasm, and
  `pitwall extension pack` copies it into `extension.json`, so the Rust
  code is the one source and `pitwall extension check` refuses a manifest
  that disagrees with it. `Presented<T>` keeps what each agent put in
  the tile by item id, with the value before, for Undo.

### 4.2 A diagram tile, as an author writes it

The main worked example, because it shows the main reason agents call
tiles: putting content in front of the user and continuing from what
the user did with it.

`extension.json` (the `tools` block is written by `pitwall extension pack`
from the code below; shown here for reading):
```json
{
  "v": 2,
  "id": "diagrams",
  "name": "Diagrams",
  "version": "0.1.0",
  "pitwall": ">=0.14.0",
  "icon": "shapes",
  "description": "Flowcharts you and your agents draw together",
  "license": "Apache-2.0",
  "categories": ["docs"],
  "permissions": {
    "files.write": "To export a diagram as SVG into a project, when you or an agent asks"
  },
  "contributes": {
    "tiles": [{
      "id": "board", "title": "Diagrams", "icon": "shapes", "wasm": "tile.wasm",
      "world": "pitwall:tile@0.1.0",
      "tools": [
        { "name": "show_diagram", "kind": "present",
          "description": "Show a flowchart to the user, written in Mermaid flowchart syntax. Pass the same item to change a diagram you showed before instead of adding a new one. Errors say which line is wrong.",
          "input": { "type": "object", "required": ["mermaid"], "properties": {
            "item": { "type": "string", "description": "Your id for this diagram, e.g. \"auth-flow\"" },
            "title": { "type": "string" },
            "mermaid": { "type": "string", "maxLength": 200000 } } } },
        { "name": "current_diagram", "kind": "read",
          "description": "Get a diagram as it is now, including the user's edits, as Mermaid.",
          "input": { "type": "object", "properties": { "item": { "type": "string" } } } },
        { "name": "export_svg", "kind": "act",
          "description": "Write a diagram as an SVG file into your project.",
          "input": { "type": "object", "required": ["item", "path"], "properties": {
            "item": { "type": "string" },
            "path": { "type": "string", "description": "Relative to your project, e.g. docs/auth.svg" } } } }
      ]
    }]
  }
}
```

`src/lib.rs` (the diagram model, parser and layout are the author's own
modules, left out):
```rust
use pitwall_extension::prelude::*;
use pitwall_extension::tools::{Call, Reply, ToolError, Problem};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod flow; // Mermaid flowchart subset: parse, layout, draw, to_mermaid, to_svg

#[derive(Default)]
struct Diagrams {
    items: Presented<flow::Diagram>,   // by item id, with origin and the value before
    shown: Option<String>,
    drag: Option<flow::Drag>,
}

impl Tile for Diagrams {
    fn new(cx: &mut Cx<Self>) -> Self {
        cx.restore().unwrap_or_default()
    }

    fn render(&mut self, _view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        let Some(id) = self.shown.clone() else {
            return status::empty("No diagram yet", "Ask an agent to draw one, or press New.")
                .action("New", cx.listener(|t, _, cx| t.new_blank(cx)))
                .into_any();
        };
        let d = self.items.get(&id).unwrap();
        v_flex().size_full()
            .child(tabs("diagrams", self.items.ids().map(|i| (i.clone(), self.items.get(i).unwrap().title())))
                .selected(&id)
                .on_select(cx.listener(|t, key: &str, _| t.shown = Some(key.into()))))
            .child(canvas(format!("d:{id}"))
                .size_full()
                .origin(self.items.origin(&id))          // host draws "from <agent>" + Undo
                .on_draw({ let d = d.clone(); move |at, ops| d.draw(at, ops) })
                .on_drag(cx.listener(|t, ev: &CanvasDrag, cx| t.move_node(ev, cx))))
            .into_any()
    }
}

/// Input of show_diagram.
#[derive(Deserialize, JsonSchema)]
struct ShowDiagram {
    /// Your id for this diagram; the same id changes the same diagram.
    item: Option<String>,
    title: Option<String>,
    /// Mermaid flowchart text (`flowchart LR`, nodes, `-->` edges).
    mermaid: String,
}

#[derive(Deserialize, JsonSchema)]
struct Which { item: Option<String> }

#[derive(Serialize, JsonSchema)]
struct Current { item: String, mermaid: String, edited_by_user: bool }

#[derive(Deserialize, JsonSchema)]
struct Export { item: String, path: String }

// No code for opening: Pitwall offers `diagrams__open` for every tile
// (agent-tools 11.3) and places it next to the calling agent.
#[pitwall_extension::tools]
impl Diagrams {
    /// Show a flowchart to the user, written in Mermaid flowchart syntax.
    #[tool(kind = "present")]
    fn show_diagram(&mut self, call: Call<ShowDiagram>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let input = call.input();
        // All or nothing: a diagram with errors changes nothing.
        let diagram = flow::parse(&input.mermaid).map_err(|errs| {
            ToolError::problems("The diagram has errors; nothing was changed.",
                errs.into_iter().map(|e| Problem::at(e.line, e.column, e.message)))
        })?;
        let item = input.item.clone().unwrap_or_else(|| self.items.new_id("diagram"));
        self.items.put(&item, diagram.titled(input.title.as_deref()), call.origin());
        self.shown = Some(item.clone());
        Ok(Reply::text(format!("Showing “{item}” ({} boxes).", self.items.get(&item).unwrap().len()))
            .item(item))
    }

    /// Get a diagram as it is now, including the user's edits, as Mermaid.
    #[tool(kind = "read")]
    fn current_diagram(&mut self, call: Call<Which>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let item = call.input().item.clone().or(self.shown.clone())
            .ok_or_else(|| ToolError::plain("There is no diagram yet."))?;
        let d = self.items.get(&item).ok_or_else(|| ToolError::plain(format!("No diagram “{item}”.")))?;
        Reply::json(&Current { item: item.clone(), mermaid: d.to_mermaid(), edited_by_user: self.items.user_edited(&item) })
    }

    /// Write a diagram as an SVG file into your project.
    #[tool(kind = "act")]
    async fn export_svg(&mut self, call: Call<Export>, cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let Export { item, path } = call.input().clone();
        let svg = self.items.get(&item).ok_or_else(|| ToolError::plain(format!("No diagram “{item}”.")))?.to_svg();
        // The caller's own project, resolved and checked by Pitwall; needs files.write on it.
        files::write(call.caller().project()?, &path, svg.as_bytes()).await?;
        Ok(Reply::text(format!("Wrote {path}.")))
    }
}

impl Diagrams {
    fn move_node(&mut self, ev: &CanvasDrag, cx: &mut Cx<Self>) {
        if let Some(id) = &self.shown {
            self.items.edit_by_user(id, |d| d.move_node(ev.start, ev.delta));
            cx.notify();
        }
    }
    fn new_blank(&mut self, cx: &mut Cx<Self>) {
        let id = self.items.new_id("diagram");
        self.items.put(&id, flow::Diagram::default(), None);
        self.shown = Some(id);
        cx.notify();
    }
}

pitwall_extension::export!(Diagrams);
```

What an agent sees when the call goes wrong (an MCP result with
`isError: true`):
```
Diagrams: the diagram has errors; nothing was changed.
- line 4, column 9: expected "-->" or "---" after "Auth"
- line 7: the node "Cache" is used but never closed with "]"
```

### 4.3 A CI status tile

`extension.json`:
```json
{
  "v": 2,
  "id": "ci-status",
  "name": "CI status",
  "version": "1.0.0",
  "pitwall": ">=0.14.0",
  "icon": "check",
  "description": "GitHub Actions runs for your agents' branches",
  "license": "MIT",
  "categories": ["ci"],
  "settings": [
    { "key": "repo", "type": "text", "label": "Repository", "placeholder": "owner/name", "required": true },
    { "key": "token", "type": "secret", "label": "GitHub token", "required": true }
  ],
  "permissions": {
    "net": { "hosts": ["api.github.com"], "reason": "To read your workflow runs" },
    "agents.read": "To show which agent pushed each branch",
    "agents.queue": "To offer “Send the failure to the agent”"
  },
  "contributes": {
    "tiles": [{ "id": "runs", "title": "CI runs", "icon": "check", "wasm": "tile.wasm" }]
  }
}
```

`src/lib.rs`:
```rust
use pitwall_extension::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
struct Run { id: u64, name: String, head_branch: String, conclusion: Option<String>, html_url: String }

#[derive(Default)]
struct Ci { runs: Load<Vec<Run>>, selected: Option<u64>, filter: String }

impl Tile for Ci {
    fn new(cx: &mut Cx<Self>) -> Self {
        cx.every(Duration::from_secs(60), |this, cx| this.refresh(cx));
        let mut ci = Ci::default();
        ci.refresh(cx);
        ci
    }

    fn render(&mut self, _view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex().gap_2().p_2().border_b_1().border_color(Token::Line)
                    .child(input("filter").placeholder("Filter by branch")
                        .value(&self.filter)
                        .on_change(cx.listener(|this, v: &str, _| this.filter = v.into())))
                    .child(icon_button(Icon::Refresh, "Refresh")
                        .on_click(cx.listener(|this, _, cx| this.refresh(cx)))),
            )
            .child(match &self.runs {
                Load::Pending => status::loading("Loading runs").into_any(),
                Load::Failed(e) => status::error("Couldn't load runs", e)
                    .action("Try again", cx.listener(|this, _, cx| this.refresh(cx)))
                    .into_any(),
                Load::Ready(runs) => {
                    let shown: Vec<Run> = runs.iter()
                        .filter(|r| r.head_branch.contains(&self.filter)).cloned().collect();
                    virtual_list("runs", shown.len(), RowHeight::Standard, move |i, cx| {
                        let r = &shown[i];
                        list_row(r.id.to_string())
                            .tone(match r.conclusion.as_deref() {
                                Some("success") => Tone::Good,
                                Some("failure") => Tone::Bad,
                                None => Tone::Busy,
                                _ => Tone::None,
                            })
                            .title(&r.name)
                            .subtitle(&r.head_branch)
                            .action(button("Open").on_click(open_url(&r.html_url)))
                    })
                    .on_select(cx.listener(|this, key: &str, _| this.selected = key.parse().ok()))
                    .into_any()
                }
            })
    }
}

impl Ci {
    fn refresh(&mut self, cx: &mut Cx<Self>) {
        let repo = settings::get("repo").unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let token = settings::secret("token").await?;
            let res = net::get(format!("https://api.github.com/repos/{repo}/actions/runs?per_page=50"))
                .bearer(&token)
                .header("Accept", "application/vnd.github+json")
                .send().await;
            #[derive(Deserialize)] struct Page { workflow_runs: Vec<Run> }
            this.update(cx, |t, _| t.runs = match res.and_then(|r| r.json::<Page>()) {
                Ok(p) => Load::Ready(p.workflow_runs),
                Err(e) => Load::Failed(e.to_string()),
            });
            Ok(())
        });
    }
}

pitwall_extension::export!(Ci);
```

Its tools: the agent that pushed a branch can see the run, read it and
re-run it.
```rust
#[derive(Deserialize, JsonSchema)] struct Branch { branch: String }
#[derive(Deserialize, JsonSchema)] struct RunId { run: u64 }

#[pitwall_extension::tools]
impl Ci {
    /// Open the CI runs tile next to your terminal, filtered to a branch.
    #[tool(kind = "show")]
    fn show_runs(&mut self, call: Call<Branch>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        self.filter = call.input().branch.clone();
        Ok(Reply::text(format!("Showing runs of {}.", self.filter)))
    }

    /// The latest runs of a branch: name, conclusion, URL.
    #[tool(kind = "read")]
    fn latest_runs(&mut self, call: Call<Branch>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let Load::Ready(runs) = &self.runs else { return Err(ToolError::plain("Runs are still loading; try again in a moment.")) };
        let mine: Vec<_> = runs.iter().filter(|r| r.head_branch == call.input().branch).take(10).collect();
        Reply::json(&mine)
    }

    /// Re-run the failed jobs of a run on GitHub.
    #[tool(kind = "act")]
    async fn rerun_failed(&mut self, call: Call<RunId>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let repo = settings::get("repo").unwrap_or_default();
        let token = settings::secret("token").await?;
        net::post(format!("https://api.github.com/repos/{repo}/actions/runs/{}/rerun-failed-jobs", call.input().run))
            .bearer(&token).send().await?.ok()?;
        Ok(Reply::text("Re-running the failed jobs."))
    }
}
```

`tests/runs.rs`, run by `cargo test` on the author's machine (native,
loads the built `tile.wasm`):
```rust
use pitwall_tile_test::{Harness, Fake};

#[test]
fn shows_a_failed_run() {
    let mut h = Harness::load("ci-status")             // builds and loads tile.wasm
        .setting("repo", "octo/app").secret("token", "t")
        .grant("net")
        .fake_http("https://api.github.com/repos/octo/app/actions/runs?per_page=50",
                   Fake::json(include_str!("runs.json")))
        .open("runs");
    h.settle();                                        // timers and tasks run
    let row = h.find_text("api: tests").row();
    assert_eq!(row.tone(), "bad");
    h.type_into("filter", "nope").settle();
    assert!(h.find_text("api: tests").is_none());
    insta::assert_snapshot!(h.text_snapshot());        // an outline of the tree
    let out = h.call_tool("latest_runs", json!({"branch": "agent/fix-auth"}))
        .as_agent("api", "~/code/my-app").settle();
    assert_eq!(out.json()[0]["conclusion"], "failure");
}
```

---

## 9. Author experience

### 9.1 Commands (`pitwall extension …`)
| Command | Does |
|---|---|
| `new <id> [--template list\|ci\|canvas\|form\|blank]` | A Cargo project: `Cargo.toml` (`crate-type = ["cdylib"]`, `pitwall-extension`), `extension.json`, `src/lib.rs`, `tests/`, `README.md`, `.github/workflows/release.yml` (build, test, pack, sign with a repository secret, attach to the release) |
| `dev [folder]` | Builds with `cargo build --release --target wasm32-wasip2` on every save, loads the folder into the running Pitwall as a Development extension (`devPath` [today]), reloads the tile after each build keeping its state (`save-state` → new instance → `init`), and streams its Log to the terminal |
| `check [folder\|file]` | Manifest, limits, imports, world, reserved keys, permissions and reasons, package rules; offline |
| `test [folder]` | `cargo test` with the harness, plus a smoke run (init, render, no traps) |
| `install <dir or url>` | Installs an unsigned folder (or one served over HTTPS) as a developer preview, with the permission screen and an "Unsigned, preview" warning, so authors on the SDK preview can share tiles before packages exist (phase 1, [6.3](packaging.md#63-install)) |
| `pack [folder]` | A deterministic `.pwtile` (sorted entries, fixed times) and its SHA-256 |
| `sign <file>` | A `.minisig` with the author's key (`pitwall extension key new` makes one; or use `minisign`) |
| `publish` | Prints the index entry and opens the pull request (via `gh` when installed, else a link) |

The Rust target `wasm32-wasip2` is a tier 2 target in the Rust
toolchain (S16), so authors need `rustup target add wasm32-wasip2`
and nothing else; no `cargo-component` is required (S16).

### 9.2 Dev reload
- `pitwall extension dev` talks to the running app over the daemon socket
  (a new method `tile.reload`, allowed only to the user's own CLI).
- Reload keeps the tile open and its state (through `save-state`); a
  state that fails to deserialize starts fresh with a note in the Log.
- A build error shows in the tile itself ("Build failed — see the
  terminal") instead of the old version, so it's never unclear which
  version runs.
- Development tiles are marked on every surface [today's trust level 3]
  and never updated.

### 9.3 Test harness
- `pitwall-extension-test` loads the built component in the same host
  runtime Pitwall uses, with a fake host: fake settings and secrets,
  scripted `net` answers (unmatched requests fail the test), a fake
  agents list and method table, a temp folder as a project, a virtual
  clock for timers.
- Queries on the semantic tree: by text, key, role; `click`,
  `type_into`, `press`, `scroll_to`, `drop`; `settle()` runs tasks and
  timers until idle.
- Snapshots: `text_snapshot()` (an indented outline with roles, text
  and tones), stable across Pitwall versions within a world.
- Pixel snapshots are Pitwall's job, not authors': Pitwall's own tests
  render the seed tiles headless with GPUI's test platform to catch
  renderer regressions.

### 9.4 Docs
A new section in `docs/extensions/` and a "Build a tile" page on the
website: quick start (new, dev, see it), the element and component
reference with screenshots in both themes and looks, host APIs and
permissions, keyboard rules, testing, packaging and signing, publishing
and Verified, compatibility. API docs on docs.rs for `pitwall-extension`.
The pages, their paths and the phase that publishes each are in 9.6.

### 9.5 Docs for authors' agents
Many authors will build an extension with a coding agent (Claude Code,
Codex, …). The docs are written so such an agent can build one from the
docs alone, without Pitwall's source:
- **An authoring skill**, `pitwall-extension-authoring/SKILL.md`,
  shipped with the SDK: how to scaffold (`pitwall extension new`), the
  manifest, the permission kinds and how to justify each, agent tools
  (that every tile can be opened by agents with no code; how to declare
  the tile's `"input"` and show tools, `show` and `present`, with typed
  input and read them from `view-opened` and `view-content`; that Pitwall registers them with every
  agent through its MCP server; from phase 3 `read` and `act`), the agent skill and
  commands, the platform services and what is held back ([14](platform-services.md#14-platform-services-what-pitwall-gives-every-tile)), testing
  with the harness, `check`, packing and signing, publishing. Short, with
  links into the full docs.
- **`pitwall extension new` puts the skill into the new project**, where the
  author's agents find it (`.claude/skills/…`, `.agents/skills/…`, as
  rulesync names those folders [today, rules.md]), so the agent working
  in the project has it from the first prompt.
- **One file, `llms.txt`**, published on the website beside the docs:
  a single reference generated from the WIT (every interface, record,
  function, permission and limit) and the author docs, including how to
  declare agent tools (the manifest's `open` and `tools` blocks, the
  `#[tools]` attributes, the view content events), following the
  proposed `llms.txt` convention for docs meant for language models
  (S24). It is regenerated in CI on each release, so it never disagrees
  with the WIT; a test fails if a WIT item has no doc comment.
- Versioned like the WIT: `llms.txt` for the newest, and one per
  published WIT version (`/sdk/0.2/llms.txt`), so an agent working on an
  extension for an older world reads the matching reference.
- Small: built with the SDK docs in phase 1, then kept current by the
  generator and the doc-comment test.

### 9.6 Website docs
**The rule.** A phase is done only when the public website docs match
what it shipped. Each phase's pages are listed in the "Website docs"
column of the phases table ([10.2](README.md#102-order-of-work)) and
published or updated in the same pull requests as the features.

**Where the pages go, by the site's conventions** [today,
[website/README.md](../../../website/README.md)]:
- The site is plain HTML pages bundled by Vite. Docs pages live at
  `website/docs/<name>/index.html` and are served at `/docs/<name>/`.
- Reference pages are written in Markdown in `docs/reference/<name>.md`.
  A page holds a `<!-- markdown:<name> -->` marker that the build
  replaces with the rendered Markdown (`SOURCES` and `ON_SITE` in
  `website/scripts/pages.mjs`, rendered by `website/scripts/markdown.mjs`).
- The docs sidebar is `DOCS_NAV` in `pages.mjs`, in three groups:
  Start, Guide, Reference. The docs overview (`website/docs/index.html`)
  has one card per page.

Proposed: a fourth sidebar group, **Extensions**, with pages at
`/docs/extensions/<page>/`. Their Markdown lives in
`docs/reference/extensions/<page>.md` and is rendered the same way, so
the pages read well on GitHub too. There is a card for the group on the
docs overview.

`markdown.mjs` handles a subset on purpose: headings, one-level lists,
fenced code, tables, inline code, links, bold, emphasis and `<kbd>`.
It has no images. The screenshots promised in [9.4](#94-docs) need images added
to it, or the switch to `marked` that the file itself suggests; decide
in phase 1.

**The pages.**

| Page | Path | For | Published in | Grows in |
|---|---|---|---|---|
| Extensions overview. Its first lines: extensions bring their own agent tools, and Pitwall registers them with every agent it starts, so agents can open your tile and send it content with no setup. Then the one extension model, what an extension can contain, how it is kept safe | `/docs/extensions/` | users and authors | 1 (Preview; agents opening tiles from 2) | 7 (Preview removed, deprecation policy), 8 (adapters on their way out) |
| Quick start: `pitwall extension new` → `dev` → `pack` → `publish` | `/docs/extensions/quick-start/` | authors | 1 (`new`, `dev`, share a preview tile with `install`) | 4 (`pack`), 5 (`publish`) |
| Tiles and UI components | `/docs/extensions/ui/` | authors; a part for users on hiding and ordering toolbar items | 1 (core elements) | 2 (all components, toolbar items) |
| Native building blocks | `/docs/extensions/native-blocks/` | authors | 2 (code view, diff view) | 6a and on demand (each block) |
| Agent tools, skill and commands | `/docs/extensions/agent-tools/` | authors; a part for users on what agents may do with a tile | 2 (show-only part: the host-provided open, declaring show tools, placement, how agents get them) | 3 (`read`, `act`, approvals, Undo, skill, commands), on demand |
| Permissions: what each grant means | `/docs/extensions/permissions/` | users and authors (written for users first) | 1 (first grants) | every phase that adds a grant |
| Platform services | `/docs/extensions/platform-services/` | authors | 1 (`tasks`, `storage`, `settings`) | 2 (the minimum), on demand (one row per service) |
| Servers in extensions | `/docs/extensions/servers/` | authors | 2 (tile plus managed service) | after 7 (headless wasm services) |
| Packaging and signing | `/docs/extensions/packaging/` | authors; a part for users on install, update and Go back | 4 | 5 |
| Publishing to the marketplace, and the Verified badge | `/docs/extensions/publishing/` | authors; a part for users on what Verified means and how to report | 5 | 7 |
| WIT reference | `/docs/extensions/wit/`, one page per published version (`/docs/extensions/wit/0.2/`) | authors | 1 (WIT 0.1, generated) | each WIT release; 1.0 in 7 |
| For coding agents: `llms.txt` and the authoring skill download | `/docs/extensions/for-agents/`; the files at `/sdk/llms.txt` and `/sdk/<version>/llms.txt` ([9.5](#95-docs-for-authors-agents)) | authors and their agents | 1 | each WIT release |
| Migrating from adapters | `/docs/extensions/migrating/` | authors of today's extensions | 8 | 9 |

Pages that exist today and change [today]:
- **CLI reference** (`docs/reference/cli.md` → `/docs/cli/`): gains
  `pitwall extension …` in the phase that ships each command (1, 3, 4, 5).
  Today it doesn't document `pitwall extension …`.
- **Settings reference** (`docs/reference/settings.md` →
  `/docs/settings/`): its Extensions section describes adapters ("runs
  an adapter per connection"). It gains install and update in phase 4
  and Browse in 5, and its adapter wording is replaced in the release
  that removes adapters (step 9).
- **How it works** (`/docs/how-it-works/`, hand-written HTML): gains a
  short part on extensions for users in phase 4, linking to the
  Permissions page.

**How the pages stay true.**
- **Generated from the source.** The WIT reference and `llms.txt` are
  generated in CI from the WIT doc comments ([9.5](#95-docs-for-authors-agents)), and a test fails
  when a WIT item has no doc comment.
- **Examples are compiled.** A page doesn't hold example code of its
  own. It names a file and a marked region in a seed tile's source
  (for example `// docs:start add-shape` … `// docs:end`), and the
  docs build pulls the region in. CI builds the seed tiles for
  `wasm32-wasip2` and fails when a named region is missing, so an
  example can't drift from code that compiles. This needs an include
  step in `pages.mjs` [new]. Until the seed tiles exist, examples come
  from the template that `pitwall extension new` writes, which is built
  in CI the same way.
- **Links are checked.** Today CI builds the website
  (`.github/workflows/ci.yml`, "Website build", and
  `.github/workflows/pages.yml`) but nothing checks its links [today;
  checked in both workflows and `website/package.json`]. Plan: add a
  small link check over `website/dist` to that job in phase 1. Every
  root-absolute link must name a built page, and every `#anchor` must
  name an id on it.
- **Preview until 1.0.** The site has no extension pages today [today:
  `website/docs/` holds the overview, Quick start, How it works and
  the CLI, Settings, Agent definitions and Detection references]. The
  adapter author docs are in the repository (`docs/extensions/`), not
  on the site. So nothing on the site needs a Preview mark now. The
  new pages carry a "Preview" label until WIT 1.0 (phase 7). Pages and
  wording about adapters (the repository's `docs/extensions/`, the
  Settings reference) get a "being replaced" note in step 8 ([15.7](extension-model.md#157-effect-on-the-plan)). They are
  replaced, with a link to Migrating from adapters, in the release
  that removes adapters (step 9).

**Release checklist.** This reuses CONTRIBUTING.md's rules [today]:
- "Update the docs (`README.md`, `docs/spec/`) when you change what
  users or the specs describe." Phase 1 extends this line to name
  `docs/reference/` and the website's docs pages, so a feature and its
  pages land in the same pull request.
- `CHANGELOG.md` entries are generated from the commits by git-cliff
  and never edited by hand; only the "Highlights" block under
  `## Unreleased` is written by hand. So the changelog comes from the
  feature's own Conventional Commit, and the release pull request adds
  the highlight.
- `ROADMAP.md` is already updated "in the same pull request" when an
  item moves; the extension items follow that.
- A release that ships a phase isn't cut while that phase's
  "Website docs" cell is unfinished.
