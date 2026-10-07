# More agents

One definition file per agent in `crates/pitwall-core/agents/` (+ detection rules in
`crates/pitwall-detect/detect/` where possible). Candidates: Gemini CLI (`gemini`),
opencode (`opencode`), Aider (`aider`), Amp (`amp`), Cursor CLI
(`cursor-agent`), GitHub Copilot CLI (`copilot`), Qwen Code (`qwen`).

- Fields: `command`, `new_args`, `resume_args` (only if the CLI supports
  resuming by id), `assign_session_id`, `hooks = "none"` unless the CLI has a
  per-launch hook mechanism that doesn't modify global config, optional
  `rulesync_target`.
- Detection: ported from Herdr's manifests (`src/detect/manifests/` in a
  local clone of Herdr; Apache-2.0, credit in NOTICE + file header), with
  tests built from Herdr's fixtures where available.
- Checking a CLI only needs `<cli> --help` / `--version`. Never start
  interactive sessions or answer dialogs while testing, and never accept an
  update prompt (that updates the CLI).
