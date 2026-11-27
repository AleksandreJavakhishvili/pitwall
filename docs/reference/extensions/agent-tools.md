# Agent tools

**Preview.** Tile tools are in the preview: opening a tile and showing content in it (SDK phase 2), and reading a tile back and acting outside it with your approval (phase 3).

An extension gives your agents tools of its own, and Pitwall registers them with every agent it starts, through the MCP server it already gives them. You set up nothing in any agent. The first such tool opens the tile: an agent can open your tile next to its own terminal with content in it ("draft that as a note", "show me the diagram on the right"), the way it opens files and web pages in Pitwall with `show_files` and `show_web`.

## Every tile can be opened

Pitwall offers `<extension-id>__open` for every extension with tiles, with no code from you. It takes the tile's declared `"input"` schema as content; without one, it opens the tile as it was.

```json
{ "id": "main", "title": "Notes", "wasm": "tile.wasm", "world": "pitwall:tile@0.1.0", "input": "input.schema.json" }
```

The schema is what agents read to decide what to send, so give it a `description` and describe each property. The tile reads the content in `on_content` with `Content::parse` ([Tiles and UI components](ui.md#opening-a-tile-with-content)), the same way it reads content you open it with. A tile entry with `"agents": false` can't be opened by agents and has no tools.

An extension with several tiles gets one `__open` that takes `tile` (which one) and `content`.

## Show tools of your own

An extension may declare up to 16 tools, of four kinds:

| Kind | Does | Who decides | Example |
|---|---|---|---|
| `show` | Opens or focuses the tile, maybe at something | Allowed | `show_issue`: an issue number in, the list scrolled to it |
| `present` | Puts content into the tile, or changes content the same agent put there | Allowed: marked with the agent, and you can Undo it | `show_diagram`: Mermaid source in, a diagram on screen |
| `read` | Returns what the tile has now, including your edits | You: asked the first time, then "Always" for that agent if you choose | `current_diagram` |
| `act` | Has effects outside the tile: the network, files, other services | You, each call, unless you chose "Always" for that agent | `export_svg` |

Declare each in the tile's entry in `extension.json`, with a JSON schema for its input:

```json
"tools": [
  {
    "name": "show_message",
    "kind": "present",
    "description": "Show a short message under the agent's item id (the same item again replaces it).",
    "input": {
      "type": "object",
      "required": ["message"],
      "properties": {
        "item": { "type": "string", "description": "Your id for the message" },
        "message": { "type": "string", "maxLength": 200, "description": "The message" }
      }
    }
  }
]
```

A name is lowercase letters, digits and `_`, at most 32, and not `open`. With the extension's id it must fit in 50 characters (`notes__show_message`), which `pitwall extension check` tells you. The input is a JSON object; `where` and `of` are Pitwall's own, so you can't use them. The schema can use `type`, `properties`, `required`, `additionalProperties`, `enum`, `items`, `minLength`, `maxLength`, `minItems`, `maxItems`, `minimum`, `maximum` and `description`.

Then write each one as a method in a `#[tools]` block, named after the tool:

```rust
use pitwall_extension::prelude::*;
use pitwall_extension::tools;
use pitwall_extension::tools::{Call, Problem, Reply, ToolError};

#[derive(serde::Deserialize)]
struct Message { message: String }

#[tools]
impl Hello {
    #[tool(kind = "present")]
    fn show_message(&mut self, call: Call<Message>, cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let text = call.input().message.trim().to_string();
        if text.is_empty() {
            return Err(ToolError::problems("Nothing was shown: the message is empty.", [Problem::new("`message` has no text")]));
        }
        self.messages.insert(call.key(), text);   // `<agent-id>/<item>`: one per agent and item
        cx.notify();
        Ok(Reply::text("Shown"))
    }
}
```

`pitwall_extension::export!` finds the `#[tools]` block itself. The method gets the call's input read into your type, and `call.item()`, `call.caller()` (the agent's id, name and project), `call.view()` and `call.key()`. The kinds in code and in the manifest must match.

- **The reply** is what the agent reads: a short `Reply::text`, with `Reply::json` for a result and `.item(…)` when your tile picked the item id. Without a `#[tools]` method (the host's `__open`), Pitwall tells the agent where the tile opened.
- **Errors change nothing.** Return `ToolError::plain` or `ToolError::problems` with `Problem::at(line, column, …)` for each problem in the input (a Mermaid syntax error at line 3). The agent reads them and can fix its call. Show nothing new when you return an error.
- **Items.** The content's id is the agent's item id: a later call with the same item updates it in place, a new item adds one. Items are per agent, so two agents' item "a" are two items.
- **The user's edits are safe.** When the user edits something an agent showed, call `tools::user_edited(call.origin())` (or with the `call` of the content it came in). From then on Pitwall refuses that agent's calls for that item, before your tile sees them, and tells the agent to show its version under a new item.

`__open` content and a declared tool's call both arrive as view content: `on_view_opened` and then the content when the view wasn't open, or only the content when it was. `Content::from_agent()`, `call()`, `tool()` and `caller()` say which call, tool and agent it came from. For `__open`, `on_content` gets it; for a declared tool, your method does.

### An example: the C4 tile's `c4__show`

The [C4 diagrams tile](seed-tiles.md#c4-diagrams-ask-an-agent-to-draw-it) has one `present` tool. An agent sends Mermaid and the tile draws it; the same item again updates the drawing in place. Its `extension.json`; the description tells agents the convention:

```json
{{#include tiles/c4/extension.json}}
```

Its input, and the start of the method: the whole source is read first, and a problem sends every problem back with its line, changing nothing.

```rust
{{#include tiles/c4/src/tile.rs#c4-show-input}}

{{#include tiles/c4/src/tile.rs#c4-show}}
```

An agent's call, as Claude Code makes it (`mcp__pitwall__c4__show`):

```json
{ "item": "containers", "title": "Shop containers", "where": "right", "mermaid": "flowchart LR\n  buyer[\"Buyer [Person]\"]\n  subgraph shop[\"Shop [Software System]\"]\n    web[\"Web app [Container]\"]\n    api[\"Orders API [Container]\"]\n  end\n  buyer -->|uses| web\n  web -->|calls| api\n" }
```

The reply: `Showing “Shop containers” in the C4 tile (item “containers”): 3 boxes, 2 arrows, 1 boundary.` A later call with `"item": "containers"` answers `Updated “Shop containers” in place …` with what changed since the last show.

## Where it opens

Next to the calling agent's terminal, on the right by default, as `show_web` places a web page. Pitwall adds `where` (`right`, `left`, `above`, `below`, `replace` or `window`) and `of` (another agent's terminal) to every tile tool. A tile already in the layout is reused wherever you moved it; with `where` it moves there. A window of its own never comes in front of your work or takes the keyboard. When the agent's terminal isn't in any open window, the tile opens in a window of its own and the agent is told why.

Pitwall marks the tile with "from <agent>" while it shows what that agent put in it; pressing the mark goes to the agent's terminal.

## Limits

- 1 MiB of JSON input per call; larger is refused.
- 10 seconds per call, from Pitwall handing it over to the tile's answer.
- 10 calls in 10 seconds per agent and tile ("Busy" after that).
- A reply's text is cut at 4 KiB and its JSON dropped over 256 KiB.
- Pitwall checks the input against the tool's schema before the tile sees it, and answers a bad one itself, naming each problem (`` `message` is required ``).

## How agents get them

- **Through Pitwall's MCP server.** [`pitwall mcp`](../cli.md#mcp) already gives the agents Pitwall starts `show_files`, `show_web` and the rest. Tile tools join that list, and agents are told when it changes (`notifications/tools/list_changed`: an extension or a switch turned on or off, an extension installed or updated).
- **Every agent Pitwall starts with that server.** Pitwall adds it to each agent kind in the kind's own launch-time way, without editing your config files ([Showing files](../agents.md#showing-files)). So Claude Code, Codex and the others get tile tools with no setup.
- **Names.** A tool is named `<extension-id>__<tool>`, such as `notes__open` and `hello__show_message`. Claude Code shows it as `mcp__pitwall__notes__open`.
- **Hints and allow lists.** `show`, `present` and `read` tools carry MCP's read-only hint. An `act` tool doesn't, and carries `destructiveHint` and `idempotentHint` as its manifest says (`"destructive": false`, `"idempotent": true`; MCP's defaults, destructive and not idempotent, when it says nothing). Claude Code and Qwen Code get the `show` and `present` tools in the allow lists Pitwall passes at launch, beside `show_files`; `read` and `act` tools are never in them, and Pitwall asks you about those itself. A tool added after an agent started is offered to it but may ask once.
- **Their description.** Pitwall puts "<Extension name> (a Pitwall tile):" before each tool's description, and adds one line per extension to the server's instructions.
- **Only agents Pitwall started can put content in a tile** (`present`) or call `read` and `act` tools. `show` and `__open` also work from a shell an agent runs, as `show_files` does.

## What you control

- **A switch per tile,** "Agents can use <name>", on the extension's card in Settings → Extensions, with the tool names. Off removes its tools from every agent's list at once.
- **A switch for all tiles,** "Agents can use tiles" in Settings → Agents (`agents.tileTools`), beside "Agents can show you files". It lists the tools agents get now.
- **The permission screen** lists a tile's tools under "Adds to Pitwall" before you turn the extension on.
- **Activity.** Each call is in the extension's Activity on its card: when, which agent, which tool and item, the start of the input, and whether it was shown or why not.

## Testing them

The harness calls a tool as an agent would, through the same checks:

```rust
use pitwall_extension_test::{Called, Harness};

let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR")).open("main");
let r = tile.call("claude", Some("show_message"), Some("a"), json!({ "message": "hi" }));
assert_eq!(r, Called::Replied { text: "Shown".into(), json: None, item: None });
assert!(tile.call("claude", Some("show_message"), Some("b"), json!({})).text().contains("`message` is required"));
assert!(tile.call("claude", None, Some("a"), json!({ "message": "hi" })).is_ok());   // `<id>__open`
```

`None` as the tool is `__open`. The result is `Called::Shown`, `Replied`, `Failed` (your error), `Refused` (Pitwall's: the schema, an edited item, the rate, a denial) or `Waiting` (a `read` or `act` call answered later).

`read` and `act` calls go through the user's approval as in Pitwall. Set what the user answers, and see what they were asked:

```rust
use pitwall_extension_test::Approval;

tile.approve(Approval::Deny);
assert!(tile.call("claude", Some("current_messages"), None, json!({})).text().contains("denied"));
tile.approve(Approval::Always);                     // "Always for claude": no question next time
let r = tile.call("claude", Some("current_messages"), None, json!({}));
assert!(r.json().is_some());
tile.run_command("send_message", json!({ "text": "hi" }));   // the user, from a plain terminal
assert_eq!(tile.approvals_asked().len(), 3);
tile.undo(tile.last_call().unwrap());               // Undo beside the mark
```

`call_append` and `call_from_file` make a `present` call with `"mode": "append"` or a named file; `cancel_call` runs a call out of time.

## Reading and acting

A `read` tool returns data, an `act` tool changes something outside the tile. Declare them like the others, with the kind, and for `act` its MCP hints:

```json
{ "name": "current_messages", "kind": "read", "description": "The messages Hello shows now, with the user's edits, as JSON by item." },
{ "name": "send_message", "kind": "act", "description": "Send a message.", "destructive": false, "idempotent": false,
  "input": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } } }
```

```rust
#[tool(kind = "read")]
fn current_messages(&mut self, _call: Call<Empty>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
    let all: BTreeMap<_, _> = self.messages.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    Reply::json(format!("{} messages", all.len()), &all)
}
```

- **The user approves, in Pitwall.** The approval dialog names the agent, its project, the tool and its arguments, with **Deny**, **Always for <agent>** and **Allow once**. A `read` asks until the user chooses "Always" for that agent; an `act` asks each time unless they did. "Always" is kept per extension, agent and tool, listed on the extension's card and in Settings → Agents, each with **Revoke**. A tile that holds `agents.queue` or `agents.start` gets no "Always" for its `act` tools: those ask every time. Unanswered in 2 minutes counts as a denial; a denied call changes nothing.
- **The user's own calls.** From a plain Pitwall terminal (`pitwall extension run`), a `read` runs without asking; an `act` still asks, without "Always", so a script can't act behind their back. From outside Pitwall only `show` tools run.
- **Answer later.** Return `Reply::later()` and reply with `tools::reply(call_id, …)` when the work is done (a request to a service, say). Past the time limit (`read` 10 seconds, `act` 2 minutes after the approval) the agent hears it timed out and your tile gets `Event::ToolCancelled(call)`.
- **Without acting permissions.** During a `show`, `present` or `read` call the tile's acting permissions (`net`, its own server, `show`, `notify`, `agents.queue`, `agents.start`, `agents.focus`) are off; an `act` call has them all. So a tool's kind can't be a lie.
- **Output.** Up to 256 KiB of text or JSON; more is cut and says so. A `read` tool should take a range or a cursor.
- **Records.** Each call is in the extension's Activity: the agent, the tool, the input (all of it for `act`) and how it ended.

## Undo, appends and files

- **Undo.** Next to the "from <agent>" mark, **Undo** takes back the agent's last `present` call: your tile gets `Event::UndoPresent(call)`. Keep presented items in `Presented<T>` and it restores the value from before that call (or removes an item the call added) for you: `present(&call, value)`, `append(&call, |v| …)`, `edit_by_user(key, value)`, `undo(call)`.
- **Appends.** A `present` tool takes `"mode": "append"` with an `item`: what it gives adds to that item (`Content::is_append()`, `Call::is_append()`). Appends close together are drawn at most four times a second.
- **Large input from a file.** Instead of inline text, a `present` call can name `"from_file": "docs/flow.mmd"`, a file in the calling agent's project (relative, inside it, at most 16 MiB, UTF-8). Your tile reads it with `call.large_input()` (in parts of up to 1 MiB, joined for you). No `files.read` permission is needed: the agent chose a file of its own.

## Commands for the shell and the palette

A tool is also a command. `expose` says where it is offered: `agents` (their MCP tools), `cli` (`pitwall extension run`) and `palette` (the command palette); the default is `["agents", "cli"]`. `palette` needs a `title`, at most 60 characters, and an input of plain properties (strings, numbers, booleans, enums):

```json
{ "name": "send_message", "kind": "act", "expose": ["agents", "cli", "palette"], "title": "Send a message", … }
```

```sh
pitwall extension tools hello                 # what agents get
pitwall extension commands hello              # what `run` takes, with arguments
pitwall extension run hello send_message -- --text "Release 0.9 is out"
pitwall extension run hello current_messages --human
echo '{"text":"hi"}' | pitwall extension call hello send_message --input -
```

`run` prints the result's JSON (its text with `--human`); problems go to stderr. Exit status: 0 ok, 1 the tile answered with an error, 2 bad arguments, 3 refused or denied, 4 not available (Pitwall, the extension or the tile isn't there or on), 5 timed out. `--timeout <seconds>` may only shorten a tool's time. The palette lists "<Extension>: <title>" for each palette tool that needs no input, and runs it as yours, without asking.
