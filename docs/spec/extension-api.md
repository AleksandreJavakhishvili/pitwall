Status: proposal. Phases 1 to 5 are built (see "Phased plan"); phase 6 is not.

# Extension API

What other people can build on Pitwall, how they ship it, and how it
updates without waiting for a Pitwall release. It grows the extensions
that exist today (docs/spec/extensions.md: one manifest, one adapter
process, one kind of view, the inbox) into a small public API.

Third-party tiles in Rust/WebAssembly, signed packages and the
marketplace: [tile-sdk/](tile-sdk/README.md). The owner decided on one extension
model ([tile-sdk/extension-model.md](tile-sdk/extension-model.md)): a wasm component becomes the way an extension
runs its own code, and the adapter process described here is removed
once each bundled extension has a component version, ported by Pitwall
or built by its authors on the public SDK.

*Removed (owner, 2026-11-26): the right Details panel, with its right-panel sections. Next up stays (the queue, Auto-send, `queue.add`, `agents.queue` with origin marks); its UI moves from the panel to a "N queued" chip on the agent's terminal tile, which opens a popover with the list, Send now / Remove and Auto-send. There is no compose box ("queue a prompt…") any more. Text below that places things in the right panel is kept as history.*

Marks used below: **[today]** is built and described in extensions.md or
profiles.md; **[proposed]** is new here.

## Summary (one page)

**Why.** Today an extension is installed only from the copy bundled with
the app, so it updates only when Pitwall releases. Agention's extension
pins Agention's plugin at `v0.4.0` in a setup action; when Agention ships
0.5.0, users wait for a Pitwall release to move. And the only thing an
extension can draw is an inbox. The owner wants updates to come from the
extension, as in VS Code, and an API others can build on.

**Principles (kept, with reasons in "Principles").**
- An extension is a declarative manifest plus a separate process (the
  adapter), in any language. No third-party code with access to
  Pitwall's process (reworded by the owner on 2026-11-26: sandboxed
  wasm components run in it without its rights; see
  [tile-sdk/README.md](tile-sdk/README.md#103-decisions), item 2).
- Pitwall draws everything natively from data the adapter sends, from a
  small set of components. A web tile (an http(s) page, no bridge) is the
  escape hatch, not the default.
- Everything an extension can read or ask Pitwall to do is a named
  permission, shown with the extension's own reason, granted by the user.
  Honest limit: without an OS sandbox this bounds what *Pitwall gives*
  the extension, not what its program can do on the machine.
- Nothing runs on install, on update or at start before the user turns it
  on. Installing and updating change files only.
- Durable layers: identity (id + source), authority (grants, approvals),
  records (what it did, what was installed from which commit),
  connections (the existing per-profile connections).

**Updates (phase 1).** A manifest names its `source`: a git repository,
a folder in it, and a tag pattern. Pitwall checks the tags, offers
"Update to 1.1.0", shows what changes on the machine (permissions,
adapter command, agent variables, setup steps), and installs the exact
commit the tag pointed to. Settings and secrets stay; the previous
version is kept for "Go back". A changed setup action is unticked and
asks to be run again, which is exactly the Agention pin case. Bundled
copies stay as offline seeds; the newest version wins, never a
downgrade.

**Contribution points, in build order.**
1. Settings field types (text, choice, switch) — cheap, unblocks others.
2. A view tile from native components: list, detail, markdown, status
   first; form and table next.
3. Command palette commands (no default keybindings).
4. Sidebar badges and a short row note on agents (generalizes the ask
   badge).
5. Notifications.
6. Right-panel sections for the focused agent.
7. Floating-control cards, only for blocking asks.
8. Web view (an http(s) page in a web tile) as the escape hatch.
9. Built since, beyond the phases: one local service per extension
   (a program Pitwall keeps running once the user confirms its commands;
   "10. A local service" below).
- Left out for now: default keybindings, status items, anything that
  reads terminal output or changes how agents behave, rules or skills
  contributions.

**What Pitwall gives an extension.** The daemon's own methods, filtered
by grants, over the adapter's stdio: read agents, projects, worktrees,
profiles; subscribe to their events; open a file or page, focus an
agent; queue a prompt into an agent's Next up and start an agent, both
behind approvals by default.

**Phases.** 1 update from a git source; 2 load from a folder (dev mode),
a log view, `pitwall extension check|test`; 3 protocol 2 (negotiation,
permissions, read API, more setting types); 4 view tile, commands,
badges, notifications; 5 actions on agents, right-panel sections,
floating cards, web view; 6 signed release feeds, opt-in OS sandbox, a
curated list.

**Decisions needed** (details and recommendations at the end):
1. First update channel: git tags or a release feed? *Git tags.*
2. Where Agention's extension releases from. *Pitwall's repo, tags
   `agention-v*`.*
3. Auto-update or update on click? *Check automatically, install on
   click; opt-in auto-install only when nothing sensitive changed.*
4. API transport: adapter stdio or the daemon socket? *Stdio.*
5. The socket lets any local process queue prompts today. *Tag
   extension callers and refuse them unless granted.*
6. May extensions send text to agents at all? *Yes, into Next up,
   marked, never auto-sent unless the user allows it per extension.*
7. Web view escape hatch? *Yes, later, no bridge.*
8. Keybindings? *Palette commands only.*
9. Runtimes? *Any executable; never run a package manager.*
10. Sandbox? *Disclose now; opt-in sandbox on macOS/Linux later.*
11. Registry? *Not until there are third-party extensions.*
12. View updates: whole-view replace or patches? *Replace.*
13. Integrity: same-source pinning now, signatures later? *Yes.*
14. Rebuild the inbox on generic components? *No, keep it.*

---

## Principles

### A separate process, not code in the app
*Changed (owner, 2026-11-26): see [tile-sdk/extension-model.md](tile-sdk/extension-model.md) (sandboxed
wasm components in the app; native code only in a managed service).
The principle now reads "no third-party code with access to Pitwall's
process".*
For. Pitwall is Rust and GPUI. Loading a dynamic library means one crash
takes the app down and the extension shares the app's memory, keychain
access and windows. The real alternative is WebAssembly, Zed's choice:
sandboxed and portable, but authors need a Rust/WIT toolchain, and the
main use (talk to the user's service, keep a connection open) needs
network and long-lived state the sandbox then has to grant anyway. A
process speaking JSON lines works in any language, crashes alone, and is
already built and tested [today]. The cost is that the process runs as
the user, with no sandbox; see "Security".

### Pitwall draws the UI
For, with one escape hatch. Native drawing keeps the look (Flat and
Glass, themes, density), the keyboard model (a tile like the others,
⌘⏎, Esc), accessibility and the inert Markdown renderer, and nothing
from an extension can run in Pitwall's windows. Raycast shows a small
component set (list, detail, form, grid, actions) covers most
extensions. Against: a chart, an editor or a service's rich page can't be
built from components. For those the service usually has a web UI
already, so the escape hatch is a web tile showing it, with the same
rules as any web tile (http(s), no script bridge, no reading back).

### Named permissions, granted by the user
For, honestly scoped. A permission bounds what Pitwall gives: which
methods answer, which events arrive, which surfaces draw. It doesn't
stop the adapter, a program running as the user, from reading files or
reaching the network itself. Settings says so in one line ("Pitwall
limits what it gives this extension; its program runs as you"). Each
permission carries the extension's own reason, shown beside it. The
grants are records (who granted what, when), so the dial can widen later
(ask each time → allowed for this project → allowed) without a new
model.

### Nothing runs before it is turned on
For, and extended to updates [proposed]. Install and update write files
only. An extension that is on is restarted with its new version after an
update the user clicked; with auto-install (opt-in) the card says
"Updated to 1.1.0 at 10:42" so new code never runs unseen.

### Fixed constraints
- **Durable layers, not scaffolding.** Identity (the extension's id, its
  source, the commit installed), authority (grants, approvals through the
  daemon's existing approval path), records (an activity log per
  extension, the install marker), connections (profiles.md). No
  contribution exists to make up for what agents can't do today.
- **No invented agent modes.** Extensions reach agents only through what
  agents natively have: environment variables at launch [today], typed
  input via Next up [proposed], MCP tools agents install themselves
  (setup actions [today]). Nothing marks a message as an "extension
  command" to the agent or asks it to answer in a special form.
- **Rules stay project and agent scoped.** No contribution adds rules,
  skills or instructions to agents.
- **No extension names in Pitwall's crates.** Every surface is generic
  and keyed by the extension's id. (One test in `catalog.rs` names the
  shipped extension today; see "What gets in the way".)

### How others do it, and where Pitwall differs

| | VS Code | Zed | Raycast | Pitwall (proposed) |
|---|---|---|---|---|
| Code runs | Node extension host, full user rights | WASM, capabilities by WIT interface | Node workers, full rights | separate process, any language |
| UI | contribution points + webviews | languages, themes, slash commands; little UI | React tree drawn natively (List, Detail, Form) | components drawn natively; web tile as escape hatch |
| Manifest | `package.json` `contributes`, `engines.vscode`, `activationEvents` | `extension.toml` | `package.json` commands, preferences | `extension.json` `contributes`, `pitwall`, `permissions` |
| Permissions | none (trust prompt per workspace) | WASM boundary | none (store review) | named grants with reasons; approvals for risky calls |
| Updates | marketplace, auto by default | registry repo pinning commits, auto | store monorepo, auto | from the extension's own source; check auto, install on click |
| Review | publisher verification, little review | PR to registry | every change reviewed by Raycast | none at first; curated list later |

Deliberate differences: no auto-install by default (an update is new code
running as the user, with access to agents that act in the user's
repositories); permissions that show; no store gatekeeping the first
year; the adapter's language is the author's.

---

## The manifest, format 2 [built, phase 3]

Format 1 [today] stays valid forever. Format 2 adds keys; a v1 Pitwall
already refuses `"v": 2` with "it needs a newer Pitwall", which is what
makes `pitwall` and `permissions` safe: no older Pitwall ignores them
silently.

```json
{
  "v": 2,
  "id": "ci-status",
  "name": "CI status",
  "version": "1.2.0",
  "pitwall": ">=0.10.0",
  "icon": "check",
  "description": "Your agents' CI runs, next to them",
  "docs": "https://github.com/example/pitwall-ci",
  "source": { "git": "https://github.com/example/pitwall-ci", "dir": ".", "tags": "v{version}" },
  "settings": [
    { "key": "server", "type": "url", "label": "CI server", "required": true },
    { "key": "token", "type": "secret", "label": "Token", "required": true },
    { "key": "branchesOnly", "type": "bool", "label": "Only agents' branches", "default": true }
  ],
  "adapter": { "command": ["node", "adapter.mjs"] },
  "permissions": {
    "agents.read": "To match CI runs to the agent whose branch they built",
    "agents.queue": "To offer “Send the failure to the agent”"
  },
  "contributes": {
    "views": [{ "id": "runs", "title": "CI runs", "icon": "check" }],
    "commands": [{ "id": "rerun", "title": "Re-run failed jobs", "context": "agent" }],
    "badges": true,
    "panels": [{ "id": "agent-ci", "title": "CI", "view": "runs-for-agent" }],
    "notifications": true
  }
}
```

- `pitwall`: the lowest Pitwall it needs, `>=X.Y.Z` only. Below it:
  "needs Pitwall 0.10 or later", and update checks skip such versions.
- `source`: where updates come from ("Distribution").
- `permissions`: name → the extension's reason (one line, ≤ 200). A name
  this Pitwall doesn't know is shown as "not available in this Pitwall"
  and never granted.
- `contributes`: what it adds. A v1 manifest with an `adapter` keeps its
  inbox [today]; in v2 the inbox is `"contributes": {"inbox": true}`.
- Unknown keys are ignored, as today.

---

## Contribution points

Each: the manifest shape, the protocol messages, what Pitwall draws.
Everything an extension draws carries its icon and name, so its text
can't pass for Pitwall's.

### 1. Inbox [today]
Threads, messages, asks with suggestions, replies. Unchanged. It stays a
dedicated surface because it does more than generic components can (the
ask card, number keys, optimistic sending with take-back). Two additions
that need no protocol change: blocking asks in the floating control (7),
and the inbox's ⌘I going to the most recently used inbox instead of the
first one on.

### 2. View tile [built, phase 4]
A tile of the layout (a peer of files, web and inbox tiles, placed and
moved as the inbox is), built from components. One per declared view.

Manifest: `"views": [{"id","title","icon"}]` (≤ 10).

Protocol (adapter → Pitwall), the whole view each time, as `threads`:
```json
{"type":"view","view":"runs","root":{
  "kind":"list","sections":[{"title":"Failing","items":[
    {"id":"r812","title":"api: tests failed","subtitle":"agent/fix-auth · 3 min ago",
     "tone":"bad","badges":[{"text":"2 jobs"}],
     "agent":{"agentId":"a1"},
     "actions":[{"id":"open","label":"Open log","primary":true},{"id":"rerun","label":"Re-run"},
                {"id":"send","label":"Send failure to agent","needs":"agents.queue"}]}]}],
  "detail":{"kind":"markdown","text":"Select a run"}}}
```
Pitwall → adapter: `{"type":"show","view":"runs"}` when it becomes
visible, `{"type":"hide","view":"runs"}`, `{"type":"select","view":"runs","item":"r812"}`
(the adapter answers with a `view` whose `detail` is that item), and
`{"type":"action","req":12,"view":"runs","item":"r812","action":"rerun"}`
answered by `result` [today's shape].

Components:
- `list`: sections of items (id, title, subtitle, tone `good|bad|warn|busy|none`,
  ≤ 3 badges, time, agent link, ≤ 5 actions); ↑↓/J K, Enter = primary
  action, a filter field. ≤ 500 items.
- `detail`: a title, metadata rows (label, value, optional link), a
  Markdown body (Pitwall's inert renderer), actions.
- `markdown`: text only.
- `status`: an empty, loading or error state: icon, words, ≤ 2 actions.
  ("Nothing failing", "Signed out — Open Settings".)
- `form` [later in phase 4]: fields of the setting types (text, choice,
  switch, number, multi-line), Submit sends
  `{"type":"submit","req","view","form","values":{…}}`. Secret fields are
  not allowed in forms: secrets are entered only in Settings.
- `table` [later in phase 4]: columns (≤ 8) and rows (≤ 500) of text,
  each row optionally with actions.

A view root is a `list` with an optional `detail` beside it (split,
narrow tiles stack them as the inbox does), or a single `detail`,
`markdown`, `form`, `table` or `status`. No nesting beyond that; no
layout primitives, colours or fonts. Links open in the browser on click,
http(s) only, as in the inbox. Actions Pitwall runs itself (open a page,
open a file, focus an agent) are `{"id":"…","open":{"url":"…"}}` /
`{"open":{"file":"…","agentId":"…"}}` / `{"focus":"a1"}`, checked against
grants.

### 3. Commands [built, phase 4]
Manifest: `"commands": [{"id","title","context":"none|agent"}]` (≤ 20).
They appear in the command palette as "<Extension>: <title>"; `agent`
ones only while an agent is focused, and their message names it (only
with `agents.read`):
`{"type":"command","req":5,"command":"rerun","agentId":"a1"}`.
No default keybindings: Pitwall's own shortcuts are dense (⌘ on macOS,
Ctrl+Shift elsewhere) and two extensions would collide. When Pitwall gets
a user keymap, a command can be bound there by the user.

### 4. Sidebar badges and row notes [built, phase 4]
Generalizes the ask badge [today: `agent_ask_badge`, from inbox asks].
```json
{"type":"badges","badges":[
  {"agent":{"agentId":"a1"},"count":2,"tone":"bad","tooltip":"2 failing runs","open":{"view":"runs","item":"r812"}},
  {"agent":{"from":{"agent":"api","session":"a7f3"}},"note":"PR #42 ✓"}]}
```
The whole set each time. An agent is named by Pitwall id (needs
`agents.read`) or by `matchAgents` names [today], which needs no
permission. One badge and one note (≤ 16 characters) per extension per
row; a row with badges from more than two extensions shows the first two
and "+N". A click opens what `open` says. The inbox's ask badge becomes
one producer of this.

### 5. Notifications [built, phase 4]
```json
{"type":"notify","id":"r812","title":"api: tests failed","body":"agent/fix-auth","open":{"view":"runs","item":"r812"}}
```
Through `platform::notify_attention`'s rules [today]: not while Pitwall
has the user's attention, never from a bench instance. At most 3 a
minute per extension (more are dropped and counted in its log); a
"Notifications" switch per extension in Settings; the same `id` again
replaces rather than repeats.

### 6. Right-panel sections [built, phase 5]
*Removed with the right Details panel (owner, 2026-11-26). Next up stays, as a chip and popover on the agent's terminal tile.*
Manifest: `"panels": [{"id","title","view"}]`. A folded section in the
right panel (with Next up, Changes, Last sent [today]) for the focused
agent. Needs `agents.read`. Pitwall sends `{"type":"show","view":"runs-for-agent","agentId":"a1"}`
when it is unfolded and the focus moves; the adapter answers with a
`view` (with `"agentId"`). Same components, height-limited, a "More…"
that opens the full view tile. Folded by default so the panel doesn't
fill with extensions.

### 7. Floating-control cards [phase 5: nothing added]
The floating control [today] shows what agents ask while Pitwall isn't in
front. Extensions get it only for what is the same thing: an ask that
holds an agent up. First from the inbox's own `blocking` asks (no
protocol change), later from a view item with `"blocking": true` and
suggestions. Its card: the question, up to 4 suggestions, Open. Not a
general notification surface.

### 8. Web view [built, phase 5]
`"webViews": [{"id":"dash","title":"Dashboard","url":"{setting:server}/dash"}]`,
or the adapter's `{"type":"webView","view":"dash","url":"http://127.0.0.1:43123/"}`
for a page it serves itself. A web tile [today: web.md] with its rules:
http(s) only, no script bridge, no reading back, the project's browser
account, never the keyboard on open. Right when the service has a rich UI
of its own; wrong as a way to skip components, which is why there is no
bridge to Pitwall: a page can't call the API.

### 9. Settings [today, extended in phase 3]
Today `url` and `secret`, per connection. Add `text` (≤ 500), `choice`
(`options`), `bool`, `number` (`min`, `max`), all per connection, all
with `default`. Plain values may go into `agentEnv` and action
arguments as `url` ones do; secrets still never.

### Left out
- **Default keybindings** (above).
- **Status items** in Pitwall's chrome: little need beside badges and
  views, and the chrome is narrow at laptop widths. Revisit later.
- **Terminal output or screen access.** Agents' screens carry secrets and
  prompts; no permission exposes them.
- **Changing agents**: no rules, skills, system prompts, hooks or
  "modes" from extensions. Setup actions [today] run the agents' own
  install commands after confirmation; that stays the way.
- **Themes and icons** shipped by extensions: icons stay Pitwall's set
  [today], unknown names draw the puzzle piece.

### 10. A local service [built, beyond the phases]
One long-running program per extension, such as the server its agents
talk to (docs/spec/extensions.md, "A service";
[docs/extensions/service.md](../extensions/service.md)). It came from the
first extension: its user had to start the server by hand, noticed only
by chance that an update needed a newer server, and had to find the first
sign-in command. Now that's three steps: turn it on, confirm once, sign
in.

Why it is shaped this way:
- **It is still a process, not code in the app.** It is started like an
  adapter (argv, no shell, its own process group) and adds no API
  surface: it can't call Pitwall, and Pitwall never talks to it beyond
  checking that its address accepts connections.
- **Consent is to exact commands, once.** The confirm card shows the
  commands as they will run, with the settings filled in. A digest of them
  is stored, as for setup actions, so an update can't change what runs
  without asking again. A service runs continuously, unlike a setup
  action that runs once, so the card, the permission screen and "What
  this extension does" each say so.
- **Little leaks in.** The service gets a minimal environment, not
  Pitwall's (no tokens from the user's shell), and a secret only when the
  manifest names it for a variable, which the confirm card shows. Its
  output is redacted like an adapter's.
- **Loopback only, and never someone else's.** `ready.listen` must be on
  this computer, so Pitwall never opens a port to the network for an
  extension. A taken port is told by binding it, never by connecting
  (connecting could reach the other program's API or wake it). Pitwall
  then uses what is there, and has no way to stop or signal a process it
  didn't start: it only ever signals the group it made. This is how a
  user who already runs the server stays undisturbed.
- **Links are secrets.** A first sign-in link grants the operator role,
  so it goes to the browser only: not to the Log, the activity log, the
  card or `ui.json`.
- **Data outside the installed folder.** `{data}` is per extension, mode
  700, not replaced by updates or Go back. Remove keeps it unless the
  user says otherwise, so a server's data never disappears by surprise.
- **Bounded restarts.** 1, 5 and 30 s, then it gives up with the last
  lines. A crashing program doesn't spin, and the user sees why.

What it doesn't change: it runs as the user, unsandboxed, like an adapter
("What a hostile extension can still do" holds as written); a program can
still `setsid` out of its group. It runs only while Pitwall runs. Not
built: starting at login, HTTPS, more than one service.

**Updates as one step.** The first extension's update moved its files,
its server and the plugin in its users' agents apart: running agents kept
the old plugin, the server was replaced by a build that couldn't read
the data, and the data didn't carry over. So an update is one ordered
step ([docs/extensions/updates.md](../extensions/updates.md)): stop the
service and back up its data (`service.backup`, a command the author
names, or a copy), install, update the server's program if needed, start
it and check it and the adapter, then the agents' setup. Before the setup,
a failure puts the files, the data and the previous service back by
itself. Why this shape:
- **The order is declared, not guessed.** `update.order` says which side
  must accept the other (`serviceFirst`: new servers accept old clients).
  `update.agents` says whether agents started before keep working.
- **Agents aren't restarted behind the user's back.** Pitwall knows what
  each agent started with (a digest of the setup and variables) and shows
  which use a previous one. Restarting is the user's setting: ask, when
  idle (only on the agent's own idle status, after 30 s, one at a time,
  resuming its conversation), or never.
- **Backups are the author's to make consistent.** A database needs its
  own backup command; Pitwall can put it back only when the manifest
  names the files it stands for, and never deletes what it replaces.
- **One confirmation.** The review lists every command the update will
  run, so pressing Update confirms them; nothing asks again halfway.

---

## What Pitwall gives an extension [built, phase 3]

### One method table
The daemon already has every method with its access level in one table
(`pitwall-daemon/src/methods.rs`, `METHODS`). The extension API is that
table, called over the adapter's stdio, with one more column: the
permission an extension needs. No second API to drift.

```json
→ {"type":"call","id":"c1","method":"agent.list"}
← {"type":"reply","id":"c1","result":[{"id":"a1","name":"fix-auth","status":"working", "…": "…"}]}
→ {"type":"call","id":"c2","method":"queue.add","params":{"agentId":"a1","text":"CI failed: …"}}
← {"type":"reply","id":"c2","error":{"code":"denied","message":"Not allowed: Send text to agents is off for CI status"}}
→ {"type":"call","id":"c3","method":"events.subscribe","params":{"topics":["agents","projects"]}}
← {"type":"event","event":"agents.changed","data":[…]}
```
`id` is the adapter's (≤ 64 characters), separate from Pitwall's numeric
`req`. Calls go to the in-process daemon with a caller of kind
`extension(<id>)`, so approvals name it ("CI status wants to…").

### Permissions

| Permission | Methods / events | Risk |
|---|---|---|
| `agents.read` | `agent.list`, `agent.status`, `events` `agents`, `attention`, `blocked` | Medium. Names, projects, full paths, branches, status details. A status detail can carry the prompt an agent is blocked on ("Allow Bash: …"), so it can leak command text. No terminal content. |
| `projects.read` | `project.list`, events `projects` | Low–medium: folder paths. |
| `worktrees.read` | the worktree list (read-only) | Low–medium. |
| `profiles.read` | profile and connection *names*, and which this extension's connection a project uses | Low. Never another extension's settings, never secrets, never tool logins. |
| `show` | `viewer.open`, `web.open` | Low: shows the user a file or page, as agents' `show_files` / `show_web` [today]; not Settings (`settings.open`, agents' `show_settings`, is never an extension's). Can put a misleading page in front of the user; the tile is labelled with the extension. |
| `agents.focus` | focus an agent's tile | Low; it can steal attention, so only in answer to the user's click on the extension's item, never unprompted. |
| `agents.queue` | `queue.add` | **High.** Text reaching an agent becomes action: the agent runs it with its own permissions in the user's repository (edit files, run commands, push). A hostile or compromised service could inject instructions. |
| `agents.start` | `agent.create` | **High.** Starts a process as the user in a project, with the project's profile; combined with `agents.queue` it is an autonomous action. |
| `notify` | notifications | Low; rate-limited. |

**Sending text to agents, honestly.** It is the most useful action (a
CI failure, a review comment, an answer from a teammate's system going to
the agent that can fix it) and the most dangerous. Proposed defaults:
- A queued item lands in the agent's Next up [today], marked with the
  extension's icon and name, and is **never auto-sent** by default even
  when the agent's auto-send is on. The user presses Send.
- Per extension, the user may turn on "Send to agents without asking"
  (stored as a grant record with date). Even then, items still show their
  origin in Next up and Last sent, and the activity log keeps them.
- At most one queued item per agent per 10 seconds per extension, ≤ 4000
  characters [today's reply limit].
- `agents.start` asks every time through the daemon's approval dialog
  [today: `Access::AskWhen`], naming the extension, project and kind.

### Events
`events.subscribe` [today: `agents`, `attention`, `blocked`] plus
`projects`, delivered as `{"type":"event",…}` lines with the existing
coalescing (≤ ~4 a second for the agent list; the newest 64 attention
events for a slow reader). A topic without its permission is left out of
the reply's `topics`, as unknown topics are today.

### Later: agents as callers
*For tiles, designed in [tile-sdk/agent-tools.md](tile-sdk/agent-tools.md).*
As agents improve, an extension's commands are as useful to agents as to
the user (re-run CI, look up a ticket). Pitwall's MCP server [today:
`show_files`, `show_web`] could list an extension's commands as tools,
with the same grants plus "agents may use this". Not in this plan; noted
so the command shape (id, title, context, result) stays fit for it.

---

## Distribution and updates [proposed, phase 1]

### Sources
A manifest's `source` says where its updates come from. Phase 1 knows
one kind:

```json
"source": { "git": "https://github.com/AleksandreJavakhishvili/pitwall", "dir": "extensions/agention", "tags": "agention-v{version}" }
```
- `git`: an `https://` URL (no `ssh:`, no `file:`, no credentials in the
  URL; a private repository works through the user's own git credential
  helper, `GIT_TERMINAL_PROMPT=0` so nothing waits on a prompt).
- `dir`: the folder in the repository holding `extension.json` (default
  the root).
- `tags`: a pattern with `{version}`; a tag that matches gives a version.
  Pre-releases (`1.2.0-beta`) are offered only to an installed
  pre-release.

**npm packages [built, after phase 5].** A second kind, for authors who
already publish to npm:
```json
"source": { "npm": "@me/ci-status", "registry": "https://registry.npmjs.org" }
```
Built as docs/spec/extensions.md "Updates from an npm package" says.
Differences from git: the version is the release (no tag pattern, no
folder: `extension.json` is at the package's root, and its `version`
must be the package's); the integrity anchor is the registry's sha512
`dist.integrity`, checked before unpacking, where git's is the commit;
"a moved tag" becomes "a version whose integrity changed"; deprecated
versions are passed over while another isn't; Pitwall makes the
requests itself (no npm, no credentials, so no private packages) and
unpacks the tarball itself, plain files under `package/` only. Like
git, nothing runs and no dependency is installed.

*Superseded by [tile-sdk/packaging.md](tile-sdk/packaging.md) (signed `.pwtile`
packages, keys in the index): the feed below is kept for the record.*
Phase 6 adds a feed for authors who build archives:
```json
"source": { "feed": "https://example.com/pitwall/ci-status.json", "key": "ed25519:4f1c…" }
```
```json
{"id":"ci-status","versions":[{"version":"1.3.0","pitwall":">=0.10.0",
  "url":"https://example.com/pitwall/ci-status-1.3.0.tar.gz",
  "sha256":"9b1e…","sig":"ed25519:…","notes":"Re-run single jobs"}]}
```

### Installing from a source
"Add from Git…" (built as "Add from Git or npm…") under Available in Settings → Extensions (and later
`pitwall extension install <url>`): a URL, optionally a tag. Pitwall
lists the tags (`git ls-remote --tags`, no clone), takes the newest
version, fetches that tag shallowly into a hidden folder beside
`<data>/extensions/`, reads the manifest, and shows before anything is
installed:
- name, version, the source URL and the commit, and "Not reviewed by
  Pitwall" (or "Comes with Pitwall" for a bundled one);
- its permissions with their reasons;
- "What this extension does on your computer" [today's disclosure]: the
  adapter command, the variables it sets on agents, each setup action's
  command lines.

Install moves it into place atomically [today's `install.rs` staging].
It stays off. Pitwall never runs a package manager or a build step: an
extension's source tag holds what runs (an adapter with dependencies
vendors or bundles them). Limits: 50 MiB and 5 000 files; symlinks
pointing outside the folder are refused.

### Checking and updating
- Checked at start and once a day, and by "Check for updates" on the
  card. A check is `git ls-remote`: cheap, nothing written.
- Offered: the newest version above the installed one whose `pitwall`
  this Pitwall meets. One that needs a newer Pitwall is named: "1.3.0
  needs Pitwall 0.11".
- **What the user confirms**: "Update to 1.1.0", unfolding into what
  changes on the machine compared to the installed version: permissions
  added or removed, the adapter command, agent variables, setup action
  steps, and the source if it moved. An update that adds a permission
  isn't granted it until the user agrees; until then the extension runs
  without it (calls are denied), never silently gains it.
- **Auto-install** (a switch per extension, off by default): installs by
  itself only when nothing in that list changed. Anything else waits for
  a click. The card says when it updated.
- Applying: as today's Update (stop if on, stage, swap, start again), the
  old folder kept as `.<id>.previous` for "Go back to 1.0.0" (one step
  back). A folder changed locally asks first, as today.

### Integrity
- **Pinned commit.** The install marker [today: `.pitwall-install.json`,
  version and digest] gains `source`, `tag` and `commit`. Pitwall fetches
  the commit `ls-remote` named and checks it after the fetch.
- **A moved tag** (the same version, a different commit than recorded
  elsewhere, e.g. in a previous check) is refused: "The 1.1.0 tag of …
  changed. Not installed."
- **Same source.** An update comes only from the `source` the user
  installed from. An update whose manifest names another source shows
  it and asks.
- **Signatures** (*superseded by [tile-sdk/packaging.md](tile-sdk/packaging.md#62-signing)*): phase 6, for feeds (ed25519, the key trusted on first
  install, rotation by a statement signed with the old key). Signed git
  tags could be checked too, but key distribution is the same problem
  and few authors sign tags; not before feeds.

### Bundled and self-updating together
- A bundled copy [today] is a seed: available offline, the first install
  without network. If its manifest has a `source`, updates come from
  there afterwards.
- The newest version wins whichever way it arrives; a bundled copy never
  replaces a newer installed one.
- The same `id` from a different source than the installed one is not an
  update: "Replace CI status from github.com/a with the one from
  github.com/b?", with its disclosure.

### Settings, secrets and grants across updates and rollback
- Settings and secrets stay [today], keyed by setting key. A key the new
  version drops is kept (hidden) so "Go back" finds it; it goes on
  Remove. A new required setting makes the card "Needs setup".
- Grants stay for permissions the new version still declares; one it
  drops is forgotten; one it adds is asked for (above).
- **Setup actions**: the record of a finished run [today: `actionsDone`]
  gains a digest of the action's steps. An update that changes the steps
  unticks the step: "Set up Agention in my agents changed in 1.1.0. Run
  it again."

### The case that motivated this
The Agention extension installs Agention's plugin pinned to `v0.4.0`
(its `setup` action), and its adapter speaks Agention's operator API of
that release. When Agention ships 0.5.0:
1. The extension's maintainer changes the pin in `extension.json` to
   `v0.5.0`, adjusts the adapter if the API moved, bumps `version` to
   1.1.0, and tags `agention-v1.1.0`. No Pitwall release.
2. Users see "Update to 1.1.0" with "Setup action steps changed: …#v0.5.0".
3. After the update the adapter restarts and the checklist's step 3 is
   unticked, asking to run the setup again, which moves the plugin.
4. A user whose server is still 0.4.0 gets the adapter's own `status`
   message ("Your Agention server is 0.4.0; this extension needs 0.5.0 —
   upgrade the server or Go back to 1.0.0"), and "Go back" is one click.

No protocol change is needed for step 4: the adapter already says what is
wrong in plain words [today].

### A curated list, later
*Superseded by [tile-sdk/marketplace.md](tile-sdk/marketplace.md) (the marketplace index).*
Not until there are extensions by other people. Then a list in a
repository (Zed's model): id, source and the pinned commit of each
reviewed version, read by "Browse extensions". The list only points;
installing still shows the disclosure and permissions.

---

## Versioning and compatibility

- **Manifest `v`** [today: 1, a higher one refused]. Format 2 above.
  Formats only add; old formats load forever (parsing is cheap).
- **`pitwall`**: the lowest Pitwall version, in format 2. Older formats
  can't say it, which is fine: format 2 is itself the "needs a newer
  Pitwall" signal for older Pitwalls.
- **Protocol negotiation.** Today Pitwall sends `"protocol":1` and stops
  an adapter that answers anything else (`hub.rs`). Proposed hello:
  ```json
  → {"type":"hello","protocol":1,"protocols":[1,2],"pitwall":"0.10.0",
     "settings":{…},"secrets":{…},
     "connection":{"id":"ext-ci-status-work","name":"Work"},
     "grants":["agents.read","notify"],
     "offers":["inbox","views","badges","commands","notify","call","events"]}
  ← {"type":"hello","protocol":2,"uses":["views","badges","call"]}
  ```
  `protocol` stays an integer 1 so a v1 adapter (which reads only that)
  answers 1 and works. A new adapter seeing only `protocol` (an old
  Pitwall) answers 1 and keeps to v1 messages. Pitwall accepts any
  version in its `protocols`.
- **Capabilities, not versions.** As the daemon's `welcome.caps`
  [today]: `offers` says what this Pitwall draws and serves; an adapter
  checks `offers`, not version numbers. `uses` lets Settings say what an
  extension actually adds.
- **Deprecation.** A protocol version stays supported for at least 6
  months and two minor releases after its successor ships. Settings marks
  an extension using a deprecated one ("uses an older protocol; it will
  stop working in Pitwall 0.14"). Messages and keys are only added
  within a version; unknown ones are ignored on both sides [today].

---

## Security model

### Trust levels
1. **Comes with Pitwall**: bundled, reviewed with Pitwall's code.
2. **From a source**: installed from a URL the user gave. "Not reviewed
   by Pitwall", the source and commit shown.
3. **Development**: loaded from a folder (dev mode), never updated,
   marked on every surface it draws.
(4. **Listed**: in the curated list, later.)

The level changes what is shown, not what is allowed: grants decide that.

### The permission screen
On turning an extension on: its permissions with reasons, each with a
switch (on by default except `agents.queue` and `agents.start`, which
start at "Ask each time"). Changeable later on the card. Each surface it
draws can say which permission made it ("Shown because CI status may
read your agents").

### Secrets [today, kept]
In the system's secure store, given to the adapter only in `hello`,
never to agents, never in argv or the environment, never in forms, never
shown. The log view redacts `hello`'s secrets. An extension never
receives another extension's settings or secrets, nor tool logins.

### Network
Not enforced; disclosed. The card says "Connects to the network itself"
for every extension with an adapter. Pitwall makes no requests on an
extension's behalf, so there is nothing to scope.

### Process sandboxing, per OS
- **Today**: none. The adapter runs as the user in its folder.
- **Realistic next** (phase 6, opt-in per extension, "Restricted"):
  - macOS: a Seatbelt profile via `sandbox-exec` (deprecated by Apple
    but still what Codex and Claude Code use): writes only in the
    extension's data folder and temp, reads its folder, the runtime and
    system libraries, network allowed.
  - Linux: Landlock (kernel 5.13+) for the same file rules, or
    bubblewrap when installed.
  - Windows: nothing realistic yet (AppContainer needs a packaged,
    capability-declared process). Restricted is unavailable there, and
    the card says so.
  The hard part is the runtime: `node` from PATH reads files all over
  (its install, `~/.npmrc`), so profiles need per-runtime allowances.
  Worth it for third-party extensions; not a blocker for phase 1.
- **Process group.** Start adapters in their own process group and stop
  the group (today: stdin closed, then the adapter killed after 2 s), so
  children it starts stop with it. A hostile one can still `setsid` away;
  only a sandbox stops that.

### Limits
Today: lines ≤ 1 MiB, ≤ 200 lines a second, strings and lists cut,
control and bidirectional characters removed. Proposed: a view ≤ 256 KiB
after cleaning and ≤ 500 items; ≤ 10 views, ≤ 20 commands; calls ≤ 20 a
second (more get `{"code":"busy"}`); notifications ≤ 3 a minute; one
queued item per agent per 10 s; badges one per agent per extension.

### What a hostile extension can still do
Honestly, without a sandbox, everything the user can:
- read and change the user's files and repositories, use their git and
  SSH credentials, reach the network;
- talk to `pitwalld.sock` as an "outside" caller: today
  `queue.add`, `web.open`, `viewer.open` and the read methods are
  `Access::Open`, so any local process can queue prompts to agents
  without asking (see decision 5);
- show misleading text in its own surfaces (mitigated by its name and
  icon on each), spam notifications (rate limit, mute);
- set the variables its manifest lists on agents (disclosed, shown in
  Settings);
- ask for keychain items it shouldn't have (on macOS the keychain asks
  the user, since the adapter isn't Pitwall's signed binary).

### How the user sees and stops it
- **Activity log** per extension (a record, kept for 30 days in
  `<data>/extensions/.log/<id>.jsonl`): every call that changed
  something (queued, started, opened), every denial, every update with
  its commit. Shown under the card.
- The status pill [today] and "Stopped: why".
- Turning it off stops the adapter's process group at once and removes
  everything it draws; its queued items not yet sent are removed from
  Next up.
- Remove [today]: off, settings and secrets forgotten, folder deleted.

---

## Author experience

### Starter template
`pitwall extension new <id>` [proposed] writes a folder:
```
<id>/
  extension.json        # v2, one view, no permissions
  adapter.mjs           # hello, status, one list view, an action
  pitwall-extension.mjs # the reference SDK, vendored (one file)
  test/pitwall.json     # what the checks expect
  test/fake-server.mjs  # a fake of the service
  README.md
```

### Dev mode
- "Load from folder…" in Settings → Extensions (and
  `pitwall extension dev <folder>`): the folder is used where it is, not
  copied, marked Development, never updated.
- The manifest is watched; a change reloads it and restarts the adapter.
  "Reload" on the card does the same by hand.
- **Log view** on the card: the last 500 protocol lines both ways
  (secrets in `hello` redacted), its stderr [today: the last few KiB],
  why it stopped, calls denied and why.

### Test harness
Today `test/pitwall.json` is read by Pitwall's own tests for the repo's
extensions only: expected action command lines, the variables shown
before and after settings, and an agreement run against a fake server.
Proposed: move the runner out of `pitwall-app`'s tests into
`pitwall_core::extensions::check` and expose it as
`pitwall extension check <folder>` (manifest, actions, variables;
no app needed) and `pitwall extension test <folder>` (the agreement run:
starts the fake, the adapter, plays `hello` with fixture settings, then
scripted exchanges from `test/pitwall.json`, e.g. "after `show runs` a
`view` with ≥ 1 item"). Third parties run the same checks in their CI.

### Any executable, a tiny Node SDK
The adapter is any program (`adapter.command` [today]). Node is the
reference, as the demo and Agention adapters are. `pitwall-extension.mjs`
(≈ 200 lines, no dependencies, vendored rather than installed):
```js
import { adapter } from "./pitwall-extension.mjs";
adapter({
  async hello({ settings, secrets, grants, offers }) { /* connect */ },
  views: { runs: { show: (ctx) => ctx.view(list(...)), action: async (item, id) => ... } },
  commands: { rerun: async ({ agentId }) => ... },
});
// ctx.call("agent.list"), ctx.on("agents.changed", fn), ctx.status("ready"), ctx.notify({...})
```
It does line framing, negotiation, request ids, exit on stdin close, and
nothing more. Others can write the same in any language from the
reference.

### Docs page outline (`docs/extensions/`)
1. What an extension is (and isn't); trust and permissions in a page.
2. Quick start: `new`, `dev`, see it in a tile.
3. Manifest reference (v1 and v2).
4. Protocol reference: hello, messages, calls, events, limits.
5. Components, with screenshots.
6. Permissions and what each risks.
7. Settings, secrets and connections.
8. Testing: `check`, `test`, fake servers.
9. Publishing: tags, `source`, what users see on update, going back.
10. Compatibility and deprecation.

---

## Phased plan

Each phase ships on its own.

### Phase 1 — Updates from a git source
- `pitwall-core/src/extensions/mod.rs`: parse `source` (`git`, `dir`,
  `tags`) in format 1 (an additive key; older Pitwalls ignore it, which
  is harmless).
- New `pitwall-core/src/extensions/source.rs`: `tags(url)` via
  `git ls-remote --tags`, `fetch(url, tag, commit, dir) -> staged folder`
  (shallow clone, check the commit, copy `dir`, limits, symlink check);
  reuse the git invocation of `rules/library.rs`.
- `install.rs`: `Offer` gains an origin (`Bundled` or `Git { url, tag,
  commit }`); the marker gains `source`, `tag`, `commit`; `update` keeps
  `.<id>.previous`; a `rollback`.
- `pitwall-app/src/extensions/catalog.rs`: checks at start, daily and on
  demand (off the UI thread); merges bundled and source offers, newest
  wins; "Add from Git…".
- `actions.rs` and the `actionsDone` records: a digest of each action's
  steps; a changed one is unticked.
- `settings/extensions.rs`: source and commit on the card, the update's
  "what changes" list, Go back, Add from Git.
- `extensions/agention/extension.json`: `source` pointing at this repo's
  `extensions/agention`, tags `agention-v{version}`; a release note in
  the extension's README on how to cut one.
- Risks: git missing (then only bundled copies; say so); private
  repositories and credential prompts; a user offline (checks fail
  quietly, the card says when it last checked).
- **Built later: npm sources** (docs/spec/extensions.md, "Updates from
  an npm package"). `mod.rs`: `Source` is `Git { git, dir, tags }` or
  `Npm { package, registry }`. New `pitwall-core/src/extensions/npm.rs`:
  `releases` (the registry's metadata over https through the host's `Http`, the app's ureq; no
  credentials), `newest`, `refusal` (no sha512, or an integrity that
  changed), `fetch` (the tarball, its integrity, then `unpack`, Pitwall's
  own tar reader). `install.rs`: `Origin::Npm { package, registry,
  version, integrity }` in the marker. `catalog.rs`: checks, offers and
  `prepare_from_npm`; tests route the registry to a loopback stand-in
  (`testing::NpmRegistry`, `catalog::NpmRedirect`). Settings: "Add from
  Git or npm…". Differences from git are under "Sources" above.

### Phase 2 — Dev mode and checks
- `mod.rs`/`discover`: a list of development folders in `ui.json`
  (`extensions.<id>.devPath`), loaded in place.
- `hub.rs`/`adapter.rs`: keep the last 500 protocol lines; process
  group on start, group kill on stop.
- `settings/extensions.rs`: Load from folder, Reload, Log.
- Move the `test/pitwall.json` runner from the app's tests to
  `pitwall_core::extensions::check`; `pitwall-cli`: `extension check`,
  `extension test`, `extension new` (offline, no socket needed).

### Phase 3 — Protocol 2: negotiation, permissions, reading [built]

Built as below, with these differences from the proposal:
- Only the reads are served: `agents.read`, `projects.read`,
  `worktrees.read` (a new `worktree.list` method) and `profiles.read`
  (only the extension's own `ext:<id>` connections). `show`,
  `agents.focus`, `agents.queue`, `agents.start` and `notify` are known
  names, shown as "not available in this Pitwall yet" and never granted;
  their methods are refused with `unsupported`.
- No `projects` event topic yet; events are the agents topics behind
  `agents.read`.
- Limits: 20 calls a second per extension (socket and stdio together,
  `busy`), 8 calls in flight per adapter, 10 events a second, params
  64 KiB, answers and events 512 KiB; 100 refusals in a row for limits
  stop the adapter. The activity log keeps 500 entries for 30 days.
- A revoke applies to the next call; a protocol 2 adapter gets a
  `grants` line and is not restarted.
- Author docs: `docs/extensions/manifest-2.md`, `protocol-2.md`,
  `permissions.md`, and `methods.md` generated from `METHODS`.

The plan as proposed:
- `protocol.rs`: `PROTOCOLS = [1, 2]`; `hello` with `protocols`,
  `grants`, `offers`, `connection`; `call`/`reply`/`event`; split the
  inbox messages from the general ones (today one `FromAdapter` enum
  holds both).
- `hub.rs`: accept any offered version (today `protocol != PROTOCOL`
  stops it); dispatch calls to the daemon with an extension caller.
- `pitwall-daemon/src/identity.rs`: `Caller` gains `extension:
  Option<String>`; `methods.rs`: an extension-permission column, checked
  in `dispatch`; `RequesterKind::Extension` for approvals.
- Also: a process whose parent chain reaches an adapter Pitwall started
  is that extension's caller on the socket (decision 5).
- Manifest format 2 (`pitwall`, `permissions`, `contributes`); grants in
  `ui.json` `extensions.<id>.grants` with dates; the permission screen;
  setting types text, choice, bool, number; the activity log.

### Phase 4 — Views, commands, badges, notifications [built]

Built as below, with these differences from the proposal:
- Components: `list` (with an optional `detail`, `markdown` or `status`
  beside it), `detail`, `markdown`, `status`. `form` and `table` are
  refused with a plain line in the tile ("a later Pitwall draws it").
  No item `agent` link, no actions Pitwall runs itself (`open`,
  `focus`), no `needs` on actions and no filter field in the list: those
  act on agents or the system, phase 5.
- Limits: a view line ≤ 256 KiB, ≤ 500 items, ≤ 50 sections, ≤ 3 badges
  and ≤ 5 actions an item, ≤ 30 fields, Markdown ≤ 40 000 characters,
  ≤ 500 badges a set, a note ≤ 16 characters. Over them a view is
  refused in its tile; text is cut.
- Views, commands, badges and notifications need a format 2 manifest
  (`contributes`), not protocol 2: a protocol 1 adapter may send them
  (the demo does). `hello`'s `offers` lists `views`, `commands`,
  `badges`, `notify`.
- A view is kept only while a tile shows it; `show` is sent again after
  a restart. With several connections, a list's sections come from each
  (named after it) and items' ids carry their connection, as threads do.
- Panes keep a view as `view` (`<extension>/<view>`) beside `inbox`, so
  older layouts load unchanged (an older Pitwall shows a view pane
  empty); windows are kept by that key, a view's in
  `extensions.<id>.viewWindows`. The inbox's window, place and drag code
  takes either (`inbox_window.rs`, by key).
- Commands go to the first running connection's adapter; the answer's
  `message` shows as a toast.
- Badges: at most four sets a second applied (the newest waits), each
  extension's shown beside the ask badge (not merged into it, no "+N"
  yet); gone when its adapter stops. A badge it may not draw is dropped
  and logged once.
- Notifications: the `notify` permission is served (on by default when
  asked for) and is the switch; at most 3 a minute (more are dropped and
  logged), the same `id` with the same words not again within 10
  minutes. A system notification can't report a click: it is shown with
  the extension's name and also as a toast in Pitwall, whose "Open"
  opens what it names.
- Each is in the activity log: actions and commands sent, notifications
  shown, refusals. Settings show what an extension adds (the permission
  screen and its card), the install and update review lists what it
  adds and drops, and `pitwall extension check|test` describe and test
  it (`views` in `test/pitwall.json`).
- `pitwall extension new --view` writes a starter with a list view, a
  command and badges; the shipped demo draws "Demo runs".

The plan as proposed:
- New `pitwall-core/src/extensions/view.rs`: the component model, its
  limits and cleaning (as `protocol.rs` cleans threads).
- New `pitwall-app/src/main_screen/view_tile.rs`; factor the placement,
  window and restore code out of `inbox_tile.rs` / `inbox_window.rs` so
  both tile kinds share it; `workspace.rs`: a pane kind keyed by
  `(extension, view)` beside `inbox` (old `ui.json` keeps loading).
- `commands.rs`: palette entries per command.
- `sidebar.rs`/`agent_ask_badge`: one badge producer per extension, the
  inbox's asks being one.
- Notifications through `platform::notify_attention`.

### Phase 5 — Acting on agents, panel sections, floating cards, web view [built]

Built as below (docs/spec/extensions.md, "Actions, right-panel sections
and web views"; docs/extensions/actions.md), with these differences from
the proposal:
- All four acting permissions start off on the permission screen,
  `show` and `agents.focus` included.
- `show`: an extension's `viewer.open`/`web.open` needs `agentId` (it
  has no agent of its own) and may not open a window of its own or
  replace the layout.
- `agents.focus`: a new method, `agent.focus` (UI and extensions only).
  It may be called at any time, not only after a click on the
  extension's item, but it acts only while Pitwall is the app in front
  and never brings it forward; otherwise it answers `focused: false`
  with why.
- `agents.queue`: besides one item per agent every 10 s and ≤ 4000
  characters, at most 3 of an extension's items wait per agent, and
  only Pitwall's own agents (not a peer's). The switch is "Send what
  <name> queues without asking", kept as
  `extensions.<id>.sendWithoutAsking` (a dated decision), shown only
  while `agents.queue` is granted; revoking the grant or turning the
  extension off removes its waiting items. Next up draws its items in
  the focus colour with "From <name>" and whether they wait for Send;
  the icon is the extension's, not per item.
- `agents.start`: never remembered (`Risk::High`); this computer only,
  in one of Pitwall's projects, without account or resume; one request
  waiting and 3 in 10 minutes per extension. Its `prompt` is shown in
  full and goes into the new agent's Next up as the extension's item
  (so it waits for Send), rather than being typed in.
- Panels: `"panels": [{"id","title","icon","more"}]` (`more` names a
  view; not `view`), at most 3. Sections are **open** by default and
  fold from their header (the fold is kept while Pitwall runs). A
  section is a view keyed `<panel>@<agentId>` in the hub. A list draws
  8 items with "and N more"; no pane beside it.
- Floating-control cards: nothing new. The control already shows an
  extension's blocking inbox asks; view items with `blocking` were not
  added and `cards` stays refused.
- Web views: their own browser store per extension (not the project's
  browser account), never opened where stores can't be separate; `http`
  only on this computer (localhost, 127.0.0.1, [::1]); opened from the
  palette only; not reloaded when the adapter sends a new address (the
  next open uses it).
- Activity entries gain `detail` and the `view` route (a button in a
  view); a view's `result`-less actions run by Pitwall are not sent to
  the adapter.
- The demo extension is unchanged: a `show` button needs an agent id,
  which it can't know without `agents.read`.

The plan as proposed:
- `queue.add` from an extension: Next up items gain an `origin`
  (extension id); auto-send skips them unless granted; Last sent shows
  the origin.
- `agent.create` from an extension through approvals.
- `panel.rs`: folded extension sections.
- Floating control: blocking inbox asks first.
- Web views through the existing web tile (`web::place`).

### Phase 6 — Feeds, signatures, sandbox, list
*Signatures and the list are superseded by [tile-sdk/](tile-sdk/README.md);
the opt-in OS sandbox for adapters stays here.*
Release feeds with sha256 and ed25519; opt-in Restricted mode on macOS
and Linux; a curated list once third-party extensions exist.

---

## What gets in the way in today's code

- **Exact protocol match.** `hub.rs` stops an adapter whose `hello`
  isn't exactly protocol 1. Fine for v1; negotiation (phase 3) must
  change it, and adapters must keep answering 1 to old Pitwalls.
- **The protocol is the inbox.** `FromAdapter`/`ToAdapter` mix the
  lifecycle (`hello`, `status`, `result`) with inbox messages; the hub's
  state per extension is threads. Generic views need the lifecycle split
  out.
- **The socket is open to local processes.** `queue.add`, `queue.send`,
  `web.open` and the read methods are `Access::Open`; an adapter (or
  anything it starts) connecting to `pitwalld.sock` counts as "outside"
  and can queue prompts today. Permissions mean little until extension
  callers are recognized there.
- **Installs come only from a bundle.** `install::Offer` is a bundled
  folder; the marker has no source or commit; Update deletes the old
  folder, so there is nothing to go back to.
- **Setup actions are ticked by id only.** A changed pin keeps its tick,
  so a user never learns they should run it again.
- **Inbox-specific layout plumbing.** The pane's `inbox` key,
  `inbox_window`, `extensions.<id>.window` and ⌘I ("the first
  extension's inbox that is on") are shaped for one view per extension.
- **An extension's name in a crate.** `pitwall-app/src/extensions/catalog.rs`
  has a test that asserts the repo ships `agention` first
  (`the_repo_ships_agention_then_the_demo`). It should check shipped
  folders generically.
- **Stopping is not a group kill.** An adapter's children can outlive it.
- **No CLI for extensions** [today, "Later"]; dev mode and checks want
  one, though `check`/`test`/`new` don't need the running app.

---

## Decisions for the owner

1. **First update channel.** Git tags, or a release feed with archives?
   *Recommend git tags*: authors already have a repository, Pitwall
   already shells out to git (rule sources), and the commit is the
   integrity anchor. Feeds in phase 6.
2. **Where Agention's extension releases from.** This repo with tags
   `agention-v*`, Agention's repository, or bundled only? *Recommend
   this repo with its own tags*: no coordination with Wayfarer Labs, the
   update path is proven on our own extension, and it can move later
   (the source-change prompt handles it).
3. **Auto-update.** *Recommend*: check automatically, install on click;
   a per-extension auto-install switch (off by default) that only
   applies when permissions, adapter command, variables, setup steps and
   source are unchanged.
4. **Transport for the API.** The adapter's stdio, or the daemon socket
   with an extension identity? *Recommend stdio*: Pitwall already knows
   which extension is on the other end, no new authentication, and the
   same method table behind it.
5. **The open socket.** Leave `queue.*` and `web.open` open to any local
   process, or tighten? *Recommend*: keep them open for the user's own
   terminal and CLI, but treat any process descended from an adapter as
   that extension (grants apply, as agents are recognized today). It
   doesn't stop a hostile adapter that detaches itself; say so.
6. **Text to agents.** Allow `agents.queue` at all? *Recommend yes*:
   into Next up, marked with the extension, never auto-sent unless the
   user allows it for that extension; `agents.start` asks every time.
7. **Web view escape hatch.** *Recommend yes, phase 5*, as a plain web
   tile with no bridge.
8. **Keybindings.** *Recommend palette commands only*; user keymaps
   later.
9. **Runtimes.** *Recommend any executable*, Node as the documented
   reference, and Pitwall never runs npm, pip or a build.
10. **Sandbox.** Ship third-party installs with disclosure only, or wait
    for a sandbox? *Recommend ship with disclosure*, opt-in Restricted
    mode on macOS and Linux in phase 6.
11. **Registry.** *Recommend none* until there are extensions by other
    people; then a curated list pinned to commits. (*Superseded by
    [tile-sdk/marketplace.md](tile-sdk/marketplace.md).*)
12. **View updates.** Whole-view replace or patches? *Recommend
    replace* (≤ 500 items, as threads today); add patches only if a
    real extension needs them.
13. **Integrity.** *Recommend* pinned commits and same-source updates
    now, signatures with feeds later.
14. **The inbox.** Rebuild it on generic components? *Recommend no*: it
    stays a dedicated contribution; generic views are for everything
    else.
