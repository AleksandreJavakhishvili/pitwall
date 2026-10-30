# Plain terminals anywhere + auto-recognised agents

Need: run a terminal at a specific path easily, and have an agent started
there by hand show up too.

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

## 3. Restart a terminal as the agent that ran in it
Need: a shell that became `claude` should restart as `claude`.
- While an agent runs in a terminal, the terminal's record remembers it
  (`AgentRecord.innerAgent = { kind, kindName, sessionId?, pid?, leftAt? }`,
  optional in state.json v2): kind and conversation id as detected above
  (hook, args, newest transcript). It is kept when the agent exits.
- Restart/Resume of such a terminal launches the login shell, runs the
  agent's resume command in it (`claude --resume <id>`, hooks wired; a fresh
  start when no conversation is known), then the terminal's own shell
  (`…; exec $SHELL`): the conversation continues, and exiting the agent leaves
  the user at the shell as before. Only on providers that run command lines
  (`caps.customCommand`); the record's kind stays `shell`.
- `AgentView.restartAs` names that agent ("Resume Claude Code");
  `caps.resume` is true when its kind can resume there and its conversation
  is known.
- **When it is forgotten:** when another agent is started in the terminal
  (replaced right away), or once the user has gone on using the shell
  without it: Enter pressed in the shell after the agent exited **and** 3
  minutes passed since it exited, while the terminal runs. Exiting the agent
  and leaving the pane at the prompt, the shell exiting, or Pitwall
  restarting keeps it.

## 4. Open preview
Need: an agent started a dev server; open it without copying the address.
- When an agent's terminal prints a local dev-server address, its pane
  header shows a "Preview :<port>" chip (before the header actions); a click
  opens it in the pane, beside the terminal (below the chip: the in-app
  preview); a second click closes it. Several addresses: the most recent
  wins, and the chevron on the chip lists them all (most recent first, up to
  4: each opens in the preview) and "Open in the browser".
- Matched: `http(s)://` + `localhost` | `127.0.0.1` | `0.0.0.0` | `[::1]` +
  a port, with an optional path; a bare `localhost:<port>` only standing on
  its own (not `user@localhost:5432`, `redis://localhost:6379`). `0.0.0.0`
  opens as `localhost`. No port, another host or scheme: no chip.
- Read from the output the engine already receives (`TermHost`, core
  `term/preview.rs`), incrementally per chunk: escape sequences dropped on
  the way (colours vanish, so Vite's bold port joins up; cursor moves and
  OSC titles don't), each finished line matched once, the unfinished one
  only for an address already followed by something. Never a scrollback
  rescan.
- `AgentView.previewUrls` (most recent first), read by the ticker; empty
  once the agent stops or exits, and reset by a restart.
- `AgentView.previewSeen`: how many times the terminal printed an address
  (each once per line). A dev server that restarts prints its address
  again with the list unchanged; the preview reloads on the count.

### In-app preview (GPUI)
- The pane splits: terminal, a draggable divider (15 %–85 %), the page.
  The toolbar's layout button moves the page right → left → below the
  terminal, which takes the rest; remembered for every pane (`ui.json`
  `previewLeft`, `previewBelow`).
- Toolbar (30 px): back, forward, reload, the address (read-only,
  selectable; the title as its tooltip; a dot while loading), phone width
  (the page 390 px wide, centred), right/left/below, Web Inspector, Open in
  the browser, close.
- The system's own engine in a native view over the window (`src/webview`:
  WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux), made on the
  first click. Where none can be made (no engine, Wayland), the chip opens
  the browser instead and a toast says why, once.
- Tied to the terminal (`src/preview` `Follow`): a new address (the server
  came back on another port) is navigated to; the same one printed again
  reloads; the agent stopping shows "Server stopped" in the page's place
  (no engine error page), and its address printed again brings the page
  back. A failed load shows "Can't reach :<port>" and retries on the next
  printing or Reload. Removing the agent closes its preview.
- Only local addresses (`webview::allowed`: localhost, loopback, private
  LAN, tailnet, over http(s)) load in it; a link or new window elsewhere
  opens in the browser.
- Each project keeps its own cookies and storage across restarts (a data
  folder per project under `<data>/webview/`; macOS keys WebKit's own
  store by it, macOS 14+, and shares the default store before that).
- Cost: hidden while its pane isn't drawn (another space, route or a
  folded tile; the page paused), and after 10 minutes hidden the view is
  dropped, its address kept: shown again, it is made again on that page.
  No polling: after each frame the window draws, the view follows its
  element's bounds (moved only when they change).
- Overlays: the view sits above GPUI's drawing. While anything drawn after
  it (menu, popover, palette, dialog, toast, tooltip) overlaps it, it is
  swapped for a snapshot of itself drawn in its place, and comes back when
  the overlay goes (a GPUI patch reports the frame's hitboxes in paint
  order: `third_party/gpui/PATCHES.md`).
- Keyboard: a click into the page gives it the keys; Pitwall's shortcuts
  still work from there (dispatched from the preview; the page keeps copy,
  paste, cut, select all, undo, redo). Esc (after the page has it too), a
  click anywhere outside the page, or focus moving elsewhere gives the
  keys back to the terminal.

## Order
Backend parts land on top of migration Step 2 (core crate), UI after the
drag & drop fix lands.
