# Race Engineer (optional assistant agent)

Pitwall stays AI-free. The Race Engineer is an ordinary agent pane (Claude or
Codex, on the user's own subscription) that Pitwall launches with a skill
teaching it the Pitwall CLI and agw. Pitwall must work fully without it.

## Building blocks
1. **Pitwall CLI + API** (`pitwall …` talking to the app/daemon socket), the same
   operations the UI uses: `agent list|create|stop|restart|remove`,
   `queue add|list`, `space create|move`, `project add|list`, `rules apply`,
   `review changes`, JSON output by default. Also useful for agents
   coordinating each other (spawn a reviewer, prompt it, wait for idle/blocked).
2. **Approval prompts in the Pitwall UI** for risky actions regardless of who
   asks (CLI, agent, or UI shortcut): remove agent, delete worktree, merge,
   discard, install hooks, agw VM/agent/workspace changes. The approval comes
   from Pitwall itself, so an agent can't approve on the user's behalf.
3. **Skill file** (`skills/pitwall/SKILL.md`) covering the CLI, plus pointers to
   agw's own agent-mode guides (`agw guide show <topic> --agent`).
4. **Preset**: "Race Engineer" button (top bar + ⌘K) opens it in a pane in the
   current space (see "The preset" below).

## Example jobs
Set up agw (VMs, templates, plugins) by conversation; rearrange spaces
("all api agents on my second monitor"); create agents per project; set up
rules from an existing CLAUDE.md; diagnose "why is this agent unknown?";
batch queueing ("queue 'run tests' for every idle agent").

## The preset (built)

**Opening it.** The headset button in the top bar's right group and "Race
Engineer" in ⌘K (`crates/pitwall-app/src/main_screen/engineer.rs`). There is
at most one: a running engineer is shown, a stopped one is started again
(resuming its conversation), and a new one is created when there is none,
in the current space. It is shown under the selected project (the focused
agent's, else the project space's, else home) and marked with an
`ENGINEER` chip in the sidebar row and pane header (`AgentView.engineer`,
`AgentRecord.engineer`). Nothing starts until the user opens it; Pitwall
works fully without it, and a build without its files just reports that.

**What it runs on.** Any agent Pitwall can start: `engineer.agent` (Settings
→ Agents → Race Engineer, `pitwall settings`, `ui.json` `engineerAgent`) is
`auto` (Claude Code if installed, else the first installed agent; the
default), a kind id from New agent (`claude`, `codex`, `gemini`, `opencode`,
`aider`, a kind from `~/.pitwall/agents/*.toml`, …) or `custom` with
`engineer.command` (an approval setting: it runs a command). A stopped
engineer of another kind is replaced by a new one the next time it opens.

**Knowledge, per launch, without touching any agent's global config**
(`pitwall_core::engineer`). Every launch (create, restart, resume):
- It works in its own folder, `<data>/engineer/` (`~/.pitwall/engineer`).
  Pitwall writes the persona (`skills/race-engineer/ENGINEER.md`, with its
  folder and the skill's path filled in) followed by the CLI skill
  (`skills/pitwall/SKILL.md`) there as `ENGINEER.md` and as the files agents
  read on their own: `AGENTS.md` (Codex, opencode, Amp, Cursor, Copilot),
  `CLAUDE.md` (Claude Code), `GEMINI.md` (Gemini CLI), `QWEN.md` (Qwen
  Code). The user's projects are elsewhere: it finds them with the CLI and
  uses absolute paths. Rule sets are never applied into this folder.
- Documented per-launch flags on top: Claude Code `--append-system-prompt
  "<pointer to CLAUDE.md>"` (its hooks still come with `--settings`), Codex
  `-c developer_instructions="<pointer to AGENTS.md>"` before its own
  arguments (also on `codex … resume`), Aider `--read <ENGINEER.md>`.
- Any other agent (a custom command, a kind of the user's own) gets "Read
  ENGINEER.md in this folder first" at the start of its first prompt.
- `pitwall` is on its PATH even without "Install command-line tool": the
  shipped `pitwall-cli` is linked as `<data>/bin/engineer/pitwall` (a copy on
  Windows) and that folder goes first on the pane's PATH. The bundle's own
  MacOS folder is not used: its `pitwall` is the app. `PITWALL_ENV`,
  `PITWALL_AGENT_ID` and `PITWALL_CLI_SOCKET` are set as for any agent.

Pitwall writes only inside its data folder (the engineer folder and the
link); `~/.claude`, `~/.codex` and the other agents' settings are never
edited.

**Shipped files.** `scripts/package-app.sh` bundles `skills/pitwall` and
`skills/race-engineer`: macOS `Contents/Resources/skills`, Linux packages
`/usr/lib/<package>/skills`, Windows `skills\` next to the app (cargo-packager
resources). The app looks there (and in `$PITWALL_SKILLS_DIR`, then the
repo's `skills/` in a workspace build), like the other sidecars
(`crates/pitwall-app/src/engineer`).

**First prompt.** A new engineer gets `engineer.greeting` queued (default:
"Introduce yourself in two lines and offer: set up agw, arrange spaces,
create agents per project, set up rules, tune settings."; empty = none). It
is sent like any Next up item, once the agent is idle.

**Web demo.** The top bar button, ⌘K row, chip and Settings → Agents section
are mirrored in the React demo with a mock engineer (`src/lib/engineer.ts`).
