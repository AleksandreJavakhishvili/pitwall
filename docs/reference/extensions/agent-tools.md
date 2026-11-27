# Agent tools

**Preview.** Opening a tile and showing content in it is in the preview (SDK phase 2). Tools that read a tile back or act outside it come in phase 3.

An extension gives your agents tools of its own, and Pitwall registers them with every agent it starts, through the MCP server it already gives them. You set up nothing in any agent. The first such tool opens the tile: an agent can open your tile next to its own terminal with content in it ("draft that as a note", "show me the diagram on the right"), the way it opens files and web pages in Pitwall with `show_files` and `show_web`.

## Every tile can be opened

Pitwall offers `<extension-id>__open` for every extension with tiles, with no code from you. It takes the tile's declared `"input"` schema as content; without one, it opens the tile as it was.

```json
{ "id": "main", "title": "Notes", "wasm": "tile.wasm", "world": "pitwall:tile@0.1.0", "input": "input.schema.json" }
```

The schema is what agents read to decide what to send, so give it a `description` and describe each property. The tile reads the content in `on_content` with `Content::parse` ([Tiles and UI components](ui.md#opening-a-tile-with-content)), the same way it reads content you open it with. A tile entry with `"agents": false` can't be opened by agents and has no tools.

An extension with several tiles gets one `__open` that takes `tile` (which one) and `content`.

## Show tools of your own

An extension may declare up to 16 tools, of two kinds that only change what the tile shows:

| Kind | Does | Example |
|---|---|---|
| `show` | Opens or focuses the tile, maybe at something | `show_issue`: an issue number in, the list scrolled to it |
| `present` | Puts content into the tile, or changes content the same agent put there | `show_diagram`: Mermaid source in, a diagram on screen |

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
- **Allowed without asking.** Tile tools change only what a sandboxed tile shows, so they carry MCP's read-only hint. Claude Code and Qwen Code get them in the allow lists Pitwall passes at launch, beside `show_files`; a tool added after an agent started is offered to it but may ask once.
- **Their description.** Pitwall puts "<Extension name> (a Pitwall tile):" before each tool's description, and adds one line per extension to the server's instructions.
- **Only agents Pitwall started can put content in a tile** (`present`). `show` and `__open` also work from a shell an agent runs, as `show_files` does.

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

`None` as the tool is `__open`. The result is `Called::Shown`, `Replied`, `Failed` (your error) or `Refused` (Pitwall's: the schema, an edited item, the rate).

## Phase 3: reading and acting

| Kind | Does | Who decides |
|---|---|---|
| `read` | Returns what the tile has now, including your edits | You, once per tile |
| `act` | Has effects outside the tile: the network, files, other services | You, each call, in an approval dialog naming the agent, the tile, the tool and its arguments |

Phase 3 also brings Undo for content an agent put in a tile, large input from a file in the agent's project, adding to an item piece by piece, a tool's schema written from its Rust type, and the `pitwall extension tools` and `pitwall extension call` commands for agents that prefer the shell. A tool's kind is declared by its author and checked by Pitwall: during a `show`, `present` or `read` call the tile's acting permissions are switched off.
