# More agents

One definition file per agent in `src-tauri/agents/` (+ detection rules in
`crates/pitwall-detect/detect/` where possible). Candidates: Gemini CLI (`gemini`),
opencode (`opencode`), Aider (`aider`), Amp (`amp`), Cursor CLI
(`cursor-agent`), GitHub Copilot CLI (`copilot`), Qwen Code (`qwen`).

- Fields: `command`, `new_args`, `resume_args` (only if the CLI supports
  resuming by id), `assign_session_id`, `hooks = "none"` unless the CLI has a
  per-launch hook mechanism that doesn't modify global config, optional
  `rulesync_target`.
- Detection: port Herdr's manifests (Apache-2.0, credit in NOTICE + file
  header) for these agents from
  /private/tmp/claude-501/-Users-ako/f9e56143-b7ff-4854-8c11-e94c05e197cc/scratchpad/herdr/src/detect/manifests/
  with tests built from Herdr's fixtures where available.
- SAFETY: you may run `<cli> --help` / `--version` for installed CLIs only.
  Do NOT start interactive sessions, do NOT press Enter on any dialog, never
  accept update prompts (a previous run accidentally updated Codex this way).
