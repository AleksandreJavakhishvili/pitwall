# Agent tools

**Preview.** Coming in the SDK: tools that open a tile with content in phase 2, the other kinds in phase 3. This page will describe the tools an extension gives your agents, which Pitwall registers with every agent it starts through its MCP server, with no setup on your part.

## What this page will cover

- How an agent opens a tile next to itself with content in it ("draw me this diagram and show it on the right"), like Pitwall's own `show_files` and `show_web`.
- The kinds of tools: opening the tile, putting content in it, reading back what it shows (with your edits), and acting outside it.
- What each kind may do without asking, and which ones ask you first.
- For authors: how to declare a tool in the SDK and name its kind.
- How agents find the tools: Pitwall's MCP server and the `pitwall extension` commands.

Until then, the [overview](README.md) describes what an extension can add.
