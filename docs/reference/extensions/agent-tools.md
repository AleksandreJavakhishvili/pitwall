# Agent tools

**Preview. Coming in phase 2 and phase 3 of the SDK.** None of the tools on this page exists in the current preview yet. This page describes how they are designed, so you can build a tile now that is ready for them.

An extension gives your agents tools of its own, and Pitwall registers them with every agent it starts, through the MCP server it already gives them. You set up nothing in any agent. The first such tool opens the tile: an agent can open your tile next to its own terminal with content in it ("draw me this diagram and show it on the right"), the way it opens files and web pages in Pitwall today with `show_files` and `show_web`.

## Ready now: content a tile accepts

What phase 2's tools deliver to a tile already exists in WIT 0.1. A tile entry that names a JSON schema (`"input": "input.schema.json"`) can be opened with content matching it, and the tile reads it in `on_content` with `Content::parse` ([Tiles and UI components](ui.md#opening-a-tile-with-content)). The `list` and `form` templates do this, and their tests open the tile with content as an agent will.

The schema is what agents read to decide what to send, so give it a `description` and describe each property.

## Phase 2: opening a tile and showing content

- **Every tile can be opened by an agent, with no code from you.** Pitwall offers `<extension-id>__open` for every extension with tiles. It takes the tile's declared `"input"` as content; without one, it opens the tile as it was. A tile entry with `"agents": false` can't be opened by agents.
- **Show tools of your own.** An extension may declare up to 16 tools with typed input, of two kinds that only change what the tile shows:

| Kind | Does | Example |
|---|---|---|
| `show` | Opens or focuses the tile | The host-provided `open` |
| `present` | Puts content into the tile, or changes content the same agent put there | `show_diagram`: Mermaid source in, a diagram on screen |

- **They arrive as view content.** A call reaches the tile as `on_view_opened` and `on_content`, or only `on_content` when it goes to a view that is open, as in WIT 0.1 today. The content's id is the agent's item id: a later call with the same item updates it in place, a new item adds one. Phase 2 adds which call, tool and agent it came from.
- **Where it opens.** Next to the calling agent's terminal, on the right by default, as `show_web` places a web page. Pitwall adds `where` (`right`, `left`, `above`, `below`, `replace` or `window`) and `of` (another agent's terminal) to every show tool. A later call from the same agent goes to the view it opened before, wherever you moved it.
- **Your edits are safe.** An agent may replace only its own items, and not one you have edited since; it is told to show its version under a new item. Pitwall marks content an agent put in a tile with the agent's name.
- **Limits.** 1 MiB of JSON input, 10 s per call, 10 calls in 10 s per agent and tile. Pitwall checks the input against the tool's schema before the tile sees it, and answers a bad one itself.

## How agents get them

- **Through Pitwall's MCP server.** [`pitwall mcp`](../cli.md#mcp) already gives the agents Pitwall starts `show_files`, `show_web` and the rest. Tile tools join that list, and agents are told when it changes (an extension turned on or off, installed or updated).
- **Every agent Pitwall starts with that server.** Pitwall adds it to each agent kind in the kind's own launch-time way, without editing your config files ([Showing files](../agents.md#showing-files)). So Claude Code, Codex and the others get tile tools with no setup.
- **Names.** A tool is named `<extension-id>__<tool>`, such as `diagrams__show_diagram` and `diagrams__open`. Claude Code shows it as `mcp__pitwall__diagrams__show_diagram`.
- **Allowed without asking.** Show tools change only what a sandboxed tile shows, so they carry MCP's read-only hint and are added to the allow lists Pitwall passes at launch, beside `show_files`.
- **Their description.** Pitwall puts "<Extension name> (a Pitwall tile):" before each tool's description, and adds one line per tile to the server's instructions.

## Phase 2: what you control

- **A switch per tile,** "Agents can use <name>", on its card in Settings → Extensions. Off removes its tools from every agent's list.
- **A switch for all tiles,** "Tile tools", in Settings → Agents, beside "Show files" and "Show web pages".
- **The permission screen** lists a tile's show tools.
- **Activity.** Each call is in the tile's Activity: when, which agent and project, which tool, and whether it was allowed.

## Phase 3: reading and acting

| Kind | Does | Who decides |
|---|---|---|
| `read` | Returns what the tile has now, including your edits | You, once per tile |
| `act` | Has effects outside the tile: the network, files, other services | You, each call, in an approval dialog naming the agent, the tile, the tool and its arguments |

Phase 3 also brings Undo for content an agent put in a tile, large input from a file in the agent's project, adding to an item piece by piece, and the `pitwall extension tools` and `pitwall extension call` commands for agents that prefer the shell. A tool's kind is declared by its author and checked by Pitwall: during a `show`, `present` or `read` call the tile's acting permissions are switched off.
