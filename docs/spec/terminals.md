# Plain terminals anywhere + auto-recognised agents (user feedback)

User: "I should be able to easily run a terminal at a specific path, and if I
start an agent manually it should also appear."

## 1. Open a terminal anywhere (one action)
- ⌘T: new shell terminal in the focused agent's working folder (else the
  selected project, else ~). ⌘⇧T: choose folder (native picker + recent
  projects + typed path).
- Sidebar: project header "+" menu → "New terminal here" / "New agent…";
  right-click a project or agent → "Open terminal here".
- ⌘K: "terminal in <project>", "terminal at <path>".
- A terminal is just a pane like any agent (kind `shell`): tiles, Wall,
  survives restarts via pitwall-hold. Default name = folder name (+ suffix).

## 2. Agents started by hand are recognised
- **Inside a Pitwall terminal**: when the user types `claude`, `codex`,
  `gemini`… in a shell pane, Pitwall notices the foreground process and the
  pane becomes that agent (kind badge, status detection rules, Next up,
  resume info) — and goes back to "shell" when it exits. Detection: the
  shell's terminal foreground process group (`ps -o tpgid= -p <shell pid>`,
  then that group's command; behind the platform layer per architecture §9
  #7), matched against agent definitions (`command` + optional
  `process_names` field). Polled cheaply (e.g. 1 s while output flows,
  backoff when idle). No holder protocol change needed.
  Session id: from hook `SessionStart` when hooks are on, else from args
  (`--resume/--session-id`) or the newest transcript for that cwd (same
  helpers as scan.rs).
- **In another terminal app (outside Pitwall)**: the sidebar shows a live
  "Elsewhere" group (same detection as the scan's "Running now", refreshed
  every ~10 s, cheap `ps`), each with "Bring into Pitwall" (resume here; warn
  to close the original first so two copies don't run). Toggle in Settings
  to hide the group.
- Never kill or adopt someone else's process; outside agents stay read-only.

## Order
Backend parts land on top of migration Step 2 (core crate), UI after the
drag & drop fix lands.
