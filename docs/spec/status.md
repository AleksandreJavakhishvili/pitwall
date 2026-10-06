# Status model


`working | blocked | idle | done | unknown | exited | stopped`

- `done` = agent finished a turn and the user hasn't looked yet; `mark_seen` turns it into `idle`.
- `exited` = process ended; `stopped` = loaded from disk after restart, not running.

**Status sources, in priority order:**
1. `hooks` — if the agent has sent a hook event in this run, hooks are authoritative.
2. `screen` — headless terminal screen + per-agent TOML rules (always running).
3. `activity` — no rules match / unknown agent: output in last ~1.2s ⇒ `working`, else `unknown`.

Hooks are optional and must never be required for the app to work.

