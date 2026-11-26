# Tile SDK: tools for agents

Part of the [Tile SDK and marketplace](README.md) design. Tools a tile offers to agents, the agent skill and commands. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 11. Tools for agents

### 11.1 Why and what
A tile is most useful to an agent as a place to put things for the
user: a diagram, a table of results, a chart, a draft. The user looks,
edits, and the agent continues from what the user did. Agents already
call tools through MCP, and Pitwall already gives the agents it starts
an MCP server [today: `pitwall mcp`, with `show_files`, `show_folder`,
`show_web`, `show_settings`, `show_device`]. So a tile offers tools,
and Pitwall lists them in that server. No new agent capability, no
special message format, nothing the agent must be taught beyond a tool
list. This follows the extension rule that nothing invents a mode or
capability agents don't natively have ([extension-api.md](../extension-api.md),
"Fixed constraints" [today]).

### 11.2 Four kinds
| Kind | What it does | Who decides | Example (Diagrams) |
|---|---|---|---|
| `show` | Opens or focuses the tile. Returns only what opened and where | Allowed by default | `open` |
| `present` | Puts content into the tile or changes content an agent put there: only what the tile shows changes | Allowed by default | `show_diagram` (Mermaid in, a diagram on screen) |
| `read` | Returns what the tile has now, including the user's edits | The user, once per tile | `current_diagram` |
| `act` | Has effects outside the tile: network, files, other services | The user, each call (approval dialog) | `export_svg` |

The kind is declared by the author and checked by Pitwall where it
can: a `show` or `present` call runs with the tile's `net`, `files.*`,
`git.*`, `clipboard` and `pitwall` acting calls switched off for its
duration (they return `denied`, and the Log says why), so a tool
declared `present` can't quietly act. A `read` call runs the same way.
Only `act` calls get the tile's full grants. The tile can still do
anything it may do on its own (timers, its own storage).

### 11.3 Declaring tools
In the manifest, under each tile, generated from the code by
`pitwall extension pack` ([4.1](authoring.md#41-shape)):
```json
"tools": [{ "name": "show_diagram", "kind": "present",
            "description": "…", "input": { "type": "object", … }, "output": { … } }]
```
- `name`: `[a-z0-9_]`, ≤ 32 characters, unique in the extension.
- `description`: ≤ 1 000 characters, what an agent reads to choose it.
  Pitwall puts "<Extension name> (a Pitwall tile):" before it.
- `input` / `output`: JSON Schema (draft 2020-12 subset: object,
  array, string, number, integer, boolean, enum, required,
  maxLength, description), ≤ 16 KiB each. Pitwall validates the input
  before the tile sees it, and answers a bad one itself with the
  schema's words.
- ≤ 16 tools per extension.
- Static, so Pitwall can list them, show them on the permission screen
  and in Browse, and offer them while the tile isn't running (a call
  starts it).

WIT, added to the world in [3.1](runtime.md#31-the-world):
```wit
interface agent-tools {
  enum tool-kind { show, present, read, act }
  record caller {
    agent-id: string,
    agent-name: string,
    project: option<string>,   // the agent's project folder, when it has one
  }
  record tool-call {
    id: u64,                   // also the origin id for presented content
    tool: string,
    kind: tool-kind,
    caller: caller,
    input: string,             // JSON, already validated against the schema
    view: option<view-id>,
  }
  record problem { line: option<u32>, column: option<u32>, message: string }
  record tool-ok { text: string, json: option<string>, item: option<string> }
  record tool-error { message: string, problems: list<problem> }
  variant tool-output { ok(tool-ok), error(tool-error) }

  /// Answer a call the tile's `tool` export didn't answer at once.
  reply: func(call: u64, out: tool-output);
  /// Large input in pages (11.7).
  input-part: func(call: u64, offset: u64, len: u32) -> result<string, string>;
}
```
The guest's `tool` export ([3.2](runtime.md#32-the-guests-exports)) answers at once or returns `none` and
calls `reply` later (an `act` that waits on the network). Calls to the
same tile are delivered one at a time on its worker, in order, like
events.

### 11.4 How agents reach them
- **MCP.** `pitwall mcp` [today] is a proxy: it asks the running
  Pitwall for its tools (`mcp.tools`) and sends every call there
  (`mcp.call`), and tells the agent `notifications/tools/list_changed`
  when the list changes. Tile tools join that list. Turning a tile on
  or off, installing, updating or revoking agent access changes the
  list, and agents re-read it the way they already do after a Pitwall
  update.
- **Names.** The canonical name is `tile.<extension-id>.<tool>`. In
  MCP it is spelled `<extension-id>__<tool>` with `.` in the id turned
  into `-` (for example `diagrams__show_diagram`), ≤ 64 characters,
  because some agents accept only letters, digits, `_` and `-` in tool
  names (*unverified across all agents Pitwall starts; phase 3 checks
  each*). Pitwall's own tools keep their names (`show_web`, …).
- **CLI.** `pitwall extension tools [<id>]` lists them;
  `pitwall extension call <id> <tool> --json '{…}'` (or `--json -` from
  stdin) calls one, through the same daemon method (`tile.call`) as
  MCP. For agents that prefer shell commands.
- **Who may call.** The daemon identifies the caller from the
  connecting process: it takes the socket's peer pid and walks its
  parents to the terminal holder Pitwall runs for an agent [today:
  `pitwall-daemon/src/identity.rs`]; nothing the caller says about
  itself counts. Tile tools answer only callers that resolve to an
  agent Pitwall started on this computer. Anyone else (the user's own
  shell outside Pitwall, an adapter, a peer's agent) gets `show` only;
  `present`, `read` and `act` are refused with "only agents Pitwall
  started can use this tool". Commands run from a plain terminal
  Pitwall runs for the user are the one exception ([11.12](#1112-commands-for-the-shell-and-the-palette)).
- **Instructions.** The server's instructions [today] gain one line per
  tile with tools ("Diagrams: show flowcharts to the user and read
  their edits"), within a fixed budget, so agents know what exists
  without a long prompt.

### 11.5 Permissions
Agent access is its own section of a tile's permission screen and card,
separate from the tile's own permissions:

> **What agents can do with Diagrams**
> - Open it · Put diagrams in it — *allowed*
> - Read the diagrams you see, with your edits — *ask once* [Allow]
> - Export a diagram as SVG into a project — *asks each time*
> - [switch] Agents can use Diagrams

- `show` and `present`: allowed by default. They only change what a
  sandboxed tile shows, and each change is marked and undoable ([11.6](#116-presented-content-ids-marks-undo)).
- `read`: off until the user allows it once for this tile (on the
  screen above, or from the approval dialog the first time an agent
  asks, with "Allow agents to read Diagrams"). It may return the user's
  own edits and anything the tile holds.
- `act`: asks every time through the daemon's approval path [today:
  `Access::AskWhen`, the dialog that already names a requester], naming
  the agent, its project, the tile, the tool and the exact arguments
  (pretty-printed, long values folded). Options: Allow once, Always
  allow for this project (a dated grant record, revocable on the card),
  Deny. Never remembered across projects.
- Per tile: "Agents can use <name>" off removes its tools from every
  agent's list. Globally: Settings → Agents lists "Tile tools" beside
  "Show files", "Show web pages" [today: `agents.showFiles`,
  `agents.showWeb`, …], off by default for none.
- An update that adds a tool, or changes a tool's kind to a stronger
  one, says so in its review; a new `read` or `act` tool starts not
  allowed.

### 11.6 Presented content: ids, marks, Undo
- Every `present` call carries an optional `item` id chosen by the
  agent (or returned by the tile when absent). The same id changes the
  same diagram; a new id adds one. Ids are per tile and per agent, so
  two agents can't overwrite each other's items by guessing ids; an
  agent may change another's item only through `read` then `present`
  under its own id.
- The node that shows presented content carries `origin` (the call id,
  [3.3](ui.md#33-the-ui-description)). The host draws a small mark on it, "from api"; clicking it focuses that agent's terminal. The mark is the
  host's, so a tile can't forge it.
- **Undo** beside the mark sends `undo-present(call)` to the tile; the
  SDK's `Presented<T>` restores the value before that call (or removes
  the item if the call created it). The user's own edits are
  `edit_by_user` and carry no mark. A tile that doesn't use
  `Presented<T>` must handle `undo-present` itself; `pitwall extension check`
  warns when a `present` tool exists and the tile never handles it.
- The tile reports user edits to agents only through `read`.

### 11.7 Size, time, streaming, errors
| | Limit |
|---|---|
| Input, inline | 1 MiB of JSON |
| Input, large | A `present` input may name a file instead of inline text: `{"from_file": "docs/flow.mmd"}`, resolved inside the calling agent's project, ≤ 16 MiB. The tile reads it in pages with `input-part`. No `files.read` grant is needed: the agent chose the file in its own project |
| Incremental | `present` with `"mode": "append"` and the same `item` adds to it; the tile redraws at most 4 times a second per item while appends arrive |
| Output | 256 KiB of text or JSON; more is cut and the result says `truncated`; a `read` tool should take a range or cursor in its input |
| Time | `show` and `present` 10 s, `read` 10 s, `act` 120 s (after the user's approval). Past it the agent gets a plain error and the tile `tool-cancelled` |
| Rate | 10 calls in 10 s per agent per tile; more get "busy, try again in a few seconds" |

Errors are words the agent can act on. A tile's `tool-error` becomes an
MCP result with `isError: true`, its message first, then each problem
as "line L, column C: message" ([4.2](authoring.md#42-a-diagram-tile-as-an-author-writes-it) shows one). `present` is all or
nothing: input with errors leaves the item as it was. Pitwall's own
refusals say why and what would change it ("Not allowed: reading
Diagrams is off. The user can allow it in Settings → Extensions →
Diagrams").

### 11.8 Where the tile opens
- Beside the calling agent's terminal, right of it by default, in the
  Pitwall window showing that agent, exactly as `show_web` places a web
  tile [today: `ViewerPlace`]. A `where` argument takes the same words
  (`right`, `left`, `above`, `below`, `replace`, `window`) and is added
  to every tile tool's schema by Pitwall.
- A later call goes to the same tile wherever the user moved it.
- `present` opens the tile when it isn't shown; `read` and `act` don't.
- Never takes the keyboard from a terminal and never brings Pitwall to
  the front [today's rule for show tools].

### 11.9 Built-in tiles, same model
Pitwall's own tiles describe themselves to agents in the same four
kinds, so built-in and third-party tiles look alike. Existing tool
names stay for compatibility.

| Tile | Tools |
|---|---|
| Files and editor | `show`: `show_files` (open a file at a line), `show_folder` [today] |
| Device | `show`: `show_device` [today] |
| Settings | `show`: `show_settings` [today] |
| Web | `show`: `show_web` [today], **and nothing else, on purpose** |
| Terminals | none |

The web tile stays show-only as a deliberate limit: agents never read
pages, click, type or run scripts in the user's web tiles, because
those carry the user's signed-in browser accounts. An agent that needs
the web uses its own tools. Terminals offer nothing either: agents'
screens carry secrets and prompts [today's rule]. A later `present`
for the editor (offer a draft as a diff the user accepts) fits the
model but is not in this plan.

### 11.10 Records
Each call is in the tile's Activity [today's log]: when, which agent and
project, tool, kind, a digest and the first 200 characters of the input
(full input for `act`), allowed or refused and why, ok or error,
duration. The agent's row offers "What it did in tiles" filtered from
the same records.

### 11.11 A skill for agents
Tools say *what* a tile can do; agents also do better when told *when*
to use them. So a package may bring one agent skill:
`agents/SKILL.md`, Markdown with the usual frontmatter (`name`,
`description`), for example:

```markdown
---
name: diagrams
description: Show flowcharts to the user in Pitwall's Diagrams tile and continue from their edits.
---
When the user asks for a diagram, a flow or an architecture sketch, call
`diagrams__show_diagram` with Mermaid flowchart text and an `item` id you
reuse for later changes. Before changing a diagram the user may have
edited, call `diagrams__current_diagram` and start from what it returns.
Only call `diagrams__export_svg` when the user asks for a file.
```

**How it reaches agents: the existing channel, not a new one.** Pitwall
already gives agents skills through rulesync ([rules.md](../rules.md)
[today]): a library of skills, assigned at two scopes only, project
and agent, generated into the agent's working folder and kept out of
git (`.git/info/exclude`), taking effect from the next session, with
"Re-apply & restart" when they change. A tile's skill joins that
library as a read-only entry of a new source kind, `ext:<id>`
(`ext:diagrams:skills/diagrams`), and is applied like any skill:
- **Scope: projects the user picks.** On install (and on the card), the
  screen asks "Give agents Diagrams' skill in: ☑ my-app ☐ api", defaulting
  to the project the user is in. It is a project-scope assignment kept
  in Pitwall (like the project default set's fallback), never written
  into the repository's tracked files and never account-wide or global
  [rules.md: no account-wide rules]. It applies only while the extension
  is on and its tools are offered to agents ([11.5](#115-permissions)); turning either off
  removes it at the next apply.
- **Removable per project**, from the card or from Settings → Rules &
  skills, where it shows as "from Diagrams (a tile)".
- **Updated with the tile.** An update that changes the skill lists it
  in the review, with the text's changes; the agents using it show the
  usual "rules changed" mark until restarted [today].
- **Agents without skills support** (rulesync has no skills target for
  Gemini CLI or Aider [today]) get the skill's one-line `description`
  only, through the MCP server instructions ([11.4](#114-how-agents-reach-them)).

**Limits.** Text only: one `SKILL.md`, ≤ 16 KiB, no other files (no
scripts, no references), no per-tool sections that run anything
(`pitwall extension check` refuses them). Its `name` is forced to the
extension's id so it can't shadow another skill.

**Prompt injection, honestly.** A skill is instructions to the user's
agents, which act with their own permissions in the user's
repositories. What Pitwall does:
- shows the whole text before install and on every update that changes
  it, and on the card;
- marks it as the tile's: Pitwall adds a first line, "This skill comes
  from the Diagrams tile installed in Pitwall; it describes that tile's
  tools.";
- never lets it grant anything: a skill is text; the tools' kinds and
  approvals ([11.5](#115-permissions)) apply whatever it says, and it can't name a tool of
  another extension in a way that changes that tool's grants;
- Verified review ([7.3](marketplace.md#73-verified)) reads it: it may only describe the tile's own
  tools and when to use them, not tell agents to run commands, skip
  confirmations or fetch other instructions.
What Pitwall can't do: stop an agent from following persuasive text in
a skill the user accepted, any more than in a README it reads.

### 11.12 Commands for the shell and the palette
"Scripts" in the sense of named things a user or an agent can run from
a shell: `pitwall extension run <tile> <command> [args]`. They are **calls
into the tile's wasm**, the same machinery as tools, not native
programs.

**A package can't ship native executables to run.** No shell scripts,
binaries or interpreters' files are ever executed from a `.pwtile`. The
only native-code route is the managed service ([13.2](services.md#132-a-a-tile-plus-a-managed-service)), at its higher
trust level ("Runs a program on your computer").

**Declaring.** A tool becomes a command by saying where it is exposed:
```json
{ "name": "export_svg", "kind": "act", "expose": ["agents", "cli", "palette"],
  "title": "Export diagram as SVG", "input": { … } }
```
- `expose` defaults to `["agents", "cli"]`. `palette` needs a `title`
  and an input Pitwall can ask for in a small form (strings, numbers,
  booleans, enums; nothing nested). Palette entries read
  "Diagrams: Export diagram as SVG", with no keys [today's rule].
- Same kinds, schemas, limits and grants as tools ([11.2](#112-four-kinds)–[11.7](#117-size-time-streaming-errors)): a
  command is a tool with another caller.

**Running.**
```
pitwall extension run diagrams export_svg --item auth-flow --path docs/auth.svg
pitwall extension run diagrams current_diagram --json '{"item":"auth-flow"}' > flow.json
echo '{"mermaid":"flowchart LR; A-->B"}' | pitwall extension run diagrams show_diagram --json -
pitwall extension commands [diagrams]        # what each tile offers, with their arguments
```
- Arguments: `--<property> <value>` for each top-level property of the
  input schema, or `--json <text|->` for the whole input.
- Output: the result's `json` on stdout with `--json`, otherwise its
  text; problems on stderr, one per line, as agents see them ([11.7](#117-size-time-streaming-errors)).
- Exit codes: 0 ok; 1 the tile answered with an error; 2 refused
  (permission, approval denied, agent access off); 3 not available
  (Pitwall not running, tile off or not installed); 4 timed out.
- Time limits as tools: 10 s for `show`, `present` and `read`, 120 s for
  `act` after approval; `--timeout` may only shorten them.

**Who the caller is.** The daemon identifies the process as for tools
([11.4](#114-how-agents-reach-them)):
- from an agent's terminal: that agent, with exactly the agent rules
  ([11.5](#115-permissions));
- from a plain terminal Pitwall runs for the user (not an agent): the
  user. `show`, `present` and `read` run; `act` still goes through the
  approval dialog, so a script can't act behind the user's back
  (whether plain terminals resolve apart from agents through their
  holders today is *to confirm*);
- from anywhere else (a shell outside Pitwall, cron): `show` only, as
  for tools.

**Minimum first.** Both the skill and commands are cheap: the skill is
a text file through the existing rules path, and commands reuse the
tool machinery and the CLI. They are part of v0.x, built with agent
tools in phase 3.

### 11.13 Decisions
1. **Should `present` really be allowed by default?** It can fill a
   tile with misleading content. Decided (owner, 2026-11-26): yes: it only changes a
   sandboxed tile, is marked with the agent and undoable, and asking
   each time would kill the main use.
2. **Agents of peers** (another computer's Pitwall). Decided (owner, 2026-11-26): no in
   this plan, matching read-only peers.
3. **One tool per tile vs. many.** Long tool lists cost agents context.
   Decided (owner, 2026-11-26): ≤ 16 per extension, listing only tools of tiles that
   are on; revisit with a "tools of tiles shown in this space" filter
   if lists grow.
4. **Should Pitwall's own MCP tools be renamed to `files__show` style?**
   Decided (owner, 2026-11-26): no: existing agents and docs use the current names.
5. **Agents calling `act` without the user present.** Decided (owner, 2026-11-26): no
   "always allow" across projects, and none for `act` tools whose
   tile holds `git.remote` or `agents.queue` grants (asks every time).
6. **Skill scope default.** Decided (owner, 2026-11-26): offered on install for the
   project the user is in, unticked for the others; never all projects
   silently.
7. **May a tile's skill mention other extensions' tools?** Decided (owner, 2026-11-26):
   no (refused by `check`): cross-tile advice is how a hostile skill
   would steer agents to a more powerful tool.
8. **Commands from cron or other programs outside Pitwall** (`show`
   only today). Decided (owner, 2026-11-26): keep that until a seed tile needs
   unattended runs; then a per-command "may run from outside Pitwall"
   grant that still can't cover `act`.
