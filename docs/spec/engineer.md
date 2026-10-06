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
   current space, cwd = the selected project.

## Example jobs
Set up agw (VMs, templates, plugins) by conversation; rearrange spaces
("all api agents on my second monitor"); create agents per project; set up
rules from an existing CLAUDE.md; diagnose "why is this agent unknown?";
batch queueing ("queue 'run tests' for every idle agent").
