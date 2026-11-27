# For coding agents

**Preview.** Many tiles will be built with a coding agent such as Claude Code or Codex. Two files let an agent build one from the docs alone, without Pitwall's source: the authoring skill, and `llms.txt`.

## The authoring skill

[`SKILL.md`](/sdk/pitwall-extension-authoring/SKILL.md) is a short guide for the agent working on a tile: the commands, the project's layout, the manifest, the `Tile` trait and elements, content from agents, the platform services, and testing with the harness. It ships with the SDK and is the same file on this site.

**In a project made with `pitwall extension new`, the agent already has it.** `new` writes the skill to `.claude/skills/pitwall-extension-authoring/` (where Claude Code finds skills) and `.agents/skills/pitwall-extension-authoring/` (where other agents look), and an `AGENTS.md` that points to it, which Codex and most other agents read first. Nothing to set up.

**In another project,** download the skill into one of those folders:

| Agent | Put `SKILL.md` in |
|---|---|
| Claude Code | `.claude/skills/pitwall-extension-authoring/` in the project, or `~/.claude/skills/pitwall-extension-authoring/` for all your projects |
| Codex and other agents | `.agents/skills/pitwall-extension-authoring/`, and a line in the project's `AGENTS.md` that says to read it before changing the tile |

The skill tells the agent to run `pitwall extension check .` and `cargo test` before it says a change is done.

## `llms.txt`

[`/sdk/llms.txt`](/sdk/llms.txt) is every Extensions page on this site as one plain Markdown file, with the examples and the [WIT reference](wit.md) filled in and every link absolute. It is built with the site, from the same sources as these pages, so it matches them and the WIT.

Point an agent at it when it needs more than the skill: the exact API, a limit, or what WIT 0.1 does and doesn't have. For example, ask it to "read Pitwall's extension docs at the llms.txt link on the For coding agents page before you start", with the link copied from above. An agent with a web fetch tool reads it directly; otherwise download it into the project and name the file.

Only the newest version is published for now. A copy per WIT version comes with the next world.

## Which to use

- **Building or changing a tile:** the skill, which `new` already put in the project.
- **A question the skill doesn't answer:** `llms.txt`.
- **Exactly what a tile and Pitwall exchange:** the [WIT reference](wit.md), which is in `llms.txt` too.

Agent tools that let your agents open and use tiles in Pitwall are a different thing: see [Agent tools](agent-tools.md).
