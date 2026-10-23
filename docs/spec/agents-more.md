# More agents

One definition file per agent in `crates/pitwall-core/agents/` (+ detection rules in
`crates/pitwall-detect/detect/` where possible). Candidates: Gemini CLI (`gemini`),
opencode (`opencode`), Aider (`aider`), Amp (`amp`), Cursor CLI
(`cursor-agent`), GitHub Copilot CLI (`copilot`), Qwen Code (`qwen`).

- Fields: `command`, `new_args`, `resume_args` (only if the CLI supports
  resuming by id), `assign_session_id`, `hooks = "none"` unless the CLI has a
  per-launch hook mechanism that doesn't modify global config, optional
  `rulesync_target` (or `rules_fallback` when rulesync has no target for
  the CLI: Pitwall writes the rules file itself, docs/spec/rules.md).
- Detection: ported from Herdr's manifests (`src/detect/manifests/` in a
  local clone of Herdr; Apache-2.0, credit in NOTICE + file header), with
  tests built from Herdr's fixtures where available.
- Checking a CLI only needs `<cli> --help` / `--version`. Never start
  interactive sessions or answer dialogs while testing, and never accept an
  update prompt (that updates the CLI).

## Names from the agent's own session

An agent's name follows its conversation's title, as the agent itself
records it, until the user names it (`AgentRecord::name_source`: `auto` /
`manual`). Pitwall only reads the agent's files; it never types `/rename`
or anything else into the agent.

- Where titles come from (`onboarding::latest::session_title`, under the
  kind's folder for the agent's account): Claude Code, newest of each, the
  user's `/rename` title (`custom-title`), else its generated title
  (`ai-title`), else a `summary` entry; Codex, the thread's newest name in
  `session_index.jsonl`; else the first prompt (`scan::title_of`). One
  line, at most 80 characters. Other kinds keep their names; a kind whose
  files are readable gets a branch there.
- When: on the events that can change it (UserPromptSubmit, Stop, a new
  conversation id, a turn ending as seen on screen) and once when Pitwall
  loads the agent; never by polling. Only agents on this Mac; not terminals.
- `auto` for new agents unless a name was typed in New agent (the Race
  Engineer and platform sessions keep theirs). Continuing a conversation
  from onboarding follows its title. Records from before: `auto` while the
  name is still the kind's own, otherwise `manual`.
- Renaming (sidebar, `agent.rename`, `pitwall agent rename`) sets `manual`.
  Back to the title: sidebar "Use session title" (shown when named by the
  user and a title is known) or `pitwall agent rename <agent> --auto`
  (`agent.rename { auto: true }`); with no title yet the name stays until
  there is one.
