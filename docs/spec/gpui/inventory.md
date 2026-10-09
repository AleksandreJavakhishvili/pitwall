# GPUI port: feature-parity inventory

This is the parity checklist for porting Pitwall's UI from Tauri + React to
GPUI (see [README.md](README.md) in this folder for the port plan). It lists
everything the current app does, derived from the code (`src/`, `src-tauri/`)
and the specs in `docs/spec/`. Tick an item when the GPUI app does the same
thing. Paths are repo-relative; examples use made-up names. "⌘" items map to
Ctrl+Shift on Windows/Linux (see "Keyboard shortcuts"). Items marked
"(decide)" are spec-only, dead or test-only code: decide whether to port them.
`[in-house: <component>]` marks items that need a UI component Pitwall builds
itself (Zed's crates other than `gpui` are GPL-3.0); `[pitwall-term-view]`
marks terminal items. Section 36 lists each component once.

## 1. App shell and window layout

- [x] One window = CSS grid: top bar row, main row (sidebar · center · right panel), fixed-height status strip row (`src/styles/layout.css`, `src/App.tsx`)
- [x] Status strip height is fixed (28 px, `--statusbar-h`) so it never resizes terminals (`src/styles/layout.css`, `docs/spec/layout.md`)
- [x] Responsive breakpoints per window width: xl ≥ 2200, lg ≥ 1500, md ≥ 1100, sm ≥ 700, xs < 700 (`src/lib/useBreakpoint.ts`)
- [x] Sidebar mode by breakpoint: full (xl/lg/md, collapsible), icon rail (sm), hidden with overlay (xs) (`src/lib/useBreakpoint.ts`, `src/App.tsx`)
- [x] Right panel docked on xl/lg, slide-in drawer with scrim on md and below (`src/App.tsx`) [in-house: drawer/overlay]
- [x] Max panes per row per breakpoint (4/3/2/1/1) seeds Auto grid only, not a cap (`src/lib/useBreakpoint.ts`, `src/lib/tiling.ts`)
- [x] Drawers close automatically when the window grows past their breakpoint (`src/App.tsx`)
- [x] Overlay sidebar / drawer close on scrim click (`src/App.tsx`) [in-house: drawer/overlay]
- [x] Right panel hidden while Wall, Review or the file viewer is shown, or when no agent is focused (`src/App.tsx`)
- [x] Center area shows, in priority: empty state (no agents) → Wall → Review → file viewer → active space → "No spaces in this window" with "New space" button (`src/App.tsx`)
- [x] Empty state: "The pit wall is quiet" board, text, "New agent ⌘N" primary button, "⌘K opens the command palette" hint (`src/components/EmptyState.tsx`)
- [x] ~~`data-attention` on the app root when any agent is blocked (styling hook) (`src/App.tsx`)~~ n/a: DOM styling hook; GPUI styles from the blocked state directly
- [x] ~~Heavy screens load lazily: Review, file viewer, onboarding, Settings (`src/App.tsx`)~~ n/a: JS code-splitting; a native binary has nothing to load lazily
- [x] ~~Error boundaries: root ("Reload"), per screen (Wall, Review, Files: "Reload"/"Close"), per modal (Settings, onboarding), per pane (Close closes the pane) with collapsible stack details (`src/components/ErrorBoundary.tsx`, `src/components/LayoutView.tsx`)~~ n/a: React render-exception boundaries; Rust renders can't throw (release aborts on panic)
- [x] ~~Host info is loaded before first render (bounded 1.5 s; macOS defaults when the backend does not answer) (`src/main.tsx`, `src/lib/host.ts`)~~ n/a: host info is synchronous in-process; nothing to wait for
- [x] ~~All UI decisions by `HostInfo` capabilities, never by OS name (`src/lib/host.ts`, `crates/pitwall-core/src/host.rs`)~~ n/a: the native build knows its OS at compile time; cfg! branches follow the same HostInfo rules

## 2. Top bar

- [x] Whole top bar is a window drag region (`data-tauri-drag-region`) (`src/components/TopBar.tsx`) [in-house: title bar]
- [x] Sidebar toggle button, title "Show/Hide agents (⌘B)", pressed state (`src/components/TopBar.tsx`) [in-house: tooltip]
- [x] Wordmark "PITWALL" (hidden in compact xs/sm) with mark glyph (`src/components/TopBar.tsx`)
- [x] ~~"MOCK" chip in the browser mock (hidden with `?shots=1`) (`src/components/TopBar.tsx`)~~ n/a: browser-mock-only chip
- [x] Space tabs in the middle (section 3) (`src/components/SpaceTabs.tsx`) [in-house: tabs]
- [x] "Search ⌘K" button opens the command palette (label hidden when compact) (`src/components/TopBar.tsx`)
- [x] Live counts: "◐ N working" (when any agents), "⚑ N done", "▲ N needs you" (loud), words hidden when compact (`src/components/TopBar.tsx`) [in-house: tooltip]
- [x] Review toggle button (pressed while Review is on), title "Review: what agents changed (⌘R)" (`src/components/TopBar.tsx`) [in-house: tooltip]
- [x] Wall toggle button, title "Wall: every agent at once (⌘E)" (`src/components/TopBar.tsx`) [in-house: tooltip]
- [x] Settings gear button (`src/components/TopBar.tsx`)
- [x] Right panel toggle, title "Show/Hide details (⌘.)" (`src/components/TopBar.tsx`) [in-house: tooltip]
- [x] Shortcut hints in titles rendered for this desktop via `keys()` (`src/lib/host.ts`) [in-house: tooltip] [in-house: keybinding label]

## 3. Spaces and space tabs

- [x] Space kinds: "All" (every agent, cannot close/rename/move), project space, custom space (`src/state/workspace.ts`)
- [x] Tabs list only this window's spaces (`spacesOf`) (`src/state/workspace.ts`) [in-house: tabs]
- [x] Clicking a tab selects the space and leaves Wall (`src/App.tsx`) [in-house: tabs]
- [x] Tab shows ▲ when an agent it shows/holds is blocked, else ⚑ when one is done (`src/components/SpaceTabs.tsx`) [in-house: tabs]
- [x] Tab tooltip: project path / "Every agent" / "Custom space — drag agents here" (`src/components/SpaceTabs.tsx`) [in-house: tabs] [in-house: tooltip]
- [x] Double-click a non-All tab to rename inline; Enter or blur commits, Esc cancels; empty name keeps the old one (`src/components/SpaceTabs.tsx`, `src/state/workspace.ts`) [in-house: tabs] [in-house: text input]
- [x] Per-tab "Move to new window (⌘⇧N)" button (`src/components/SpaceTabs.tsx`) [in-house: tabs] [in-house: tooltip]
- [x] Per-tab "Close space" button (agents just stop being shown there) (`src/components/SpaceTabs.tsx`) [in-house: tabs]
- [x] "+" tab creates "Space N" (custom) in this window; also a drop target (`src/components/SpaceTabs.tsx`) [in-house: tabs] [in-house: drag and drop]
- [x] Tab drop highlight while an agent is dragged over it (`src/components/SpaceTabs.tsx`, `src/lib/dnd.ts`) [in-house: tabs] [in-house: drag and drop]
- [x] "Open as space" from a sidebar project creates or reuses a project space tiling that project's agents (preset by count) and focuses it (or its window) (`src/state/workspace.ts`, `src/App.tsx`)
- [x] Closing a space removes its window ownership (`src/state/workspace.ts`)
- [x] Moving "All" to a new window is refused with an info toast (`src/App.tsx`) [in-house: toast]
- [x] First run: the first agent is placed in the All space so the screen is not empty (main window only) (`src/App.tsx`)

## 4. Sidebar

- [x] Agents grouped by machine + project; creatable machines first, then other machines by label, then display name (`src/lib/groups.ts`) [in-house: list/virtual list] [in-house: scroll view]
- [x] Listed projects without agents also appear as groups (`src/components/onboarding/projects.ts`)
- [x] Machine heading above a machine's first group when agents run on 2+ machines (`src/lib/groups.ts`)
- [x] Agents within a group sorted blocked → done → working → idle → unknown → exited → stopped, then by creation time (`src/lib/status.ts`)
- [x] Project header: chevron collapse toggle (persisted in `ui.collapsed`), display name, "▲ N" blocked count, agent count (`src/components/Sidebar.tsx`) [in-house: list/virtual list]
- [x] Collapsed group shows a one-line summary ("1 needs you · 2 idle") (`src/lib/groups.ts`)
- [x] Project header "+" button (creatable machines only) opens menu: "New terminal here", "New agent…" (`src/components/Sidebar.tsx`) [in-house: popover/menu]
- [x] Right-click project header opens the same menu (`src/components/Sidebar.tsx`) [in-house: popover/menu]
- [x] Project header "Open as space" grid button (when it has agents) (`src/components/Sidebar.tsx`) [in-house: tooltip]
- [x] Project header "x" button "Remove from Pitwall (files are not touched)" when it has no agents (`src/components/Sidebar.tsx`) [in-house: tooltip]
- [x] Empty project body: "No agents yet" and "+ Agent" button (preselects that project) (`src/components/Sidebar.tsx`)
- [x] Agent row: status glyph, name, status word, location chip, kind name, "· in terminal" marker, +/− diffstat (`src/components/AgentRow.tsx`) [in-house: list/virtual list]
- [x] Agent row shows blocked detail line when blocked (`src/components/AgentRow.tsx`)
- [x] Agent row index hint ⌘1–⌘9 for the first nine agents (sidebar order across groups) (`src/components/AgentRow.tsx`)
- [x] Agent row tooltip: name, "— in <space>" or "— in other window", status detail (`src/components/Sidebar.tsx`, `src/components/AgentRow.tsx`) [in-house: tooltip]
- [x] Agent row selected highlight for the focused agent (`src/components/AgentRow.tsx`) [in-house: list/virtual list]
- [x] Click agent row → show agent (section 6 "Show agent") (`src/components/Sidebar.tsx`)
- [x] Agent row is a drag source (`src/components/AgentRow.tsx`) [in-house: drag and drop]
- [x] Agent row context menu: "Open terminal here" (creatable machines), "Remove agent…" (`src/components/Sidebar.tsx`) [in-house: popover/menu]
- [x] Agent row worktree chip "N worktrees" toggles its worktree list; expanding forces a worktree refresh (`src/components/AgentRow.tsx`, `src/components/Sidebar.tsx`)
- [x] Per-project "N other worktrees" chip listing worktrees no agent works in (`src/components/Sidebar.tsx`)
- [x] "Elsewhere" group: agents running in other terminal apps, collapsible, count (section 22) (`src/components/ElsewhereGroup.tsx`) [in-house: list/virtual list]
- [x] Footer: "New agent ⌘N" button and terminal icon button "New terminal here (⌘T) · ⌘⇧T to choose a folder" (`src/components/Sidebar.tsx`) [in-house: tooltip]
- [x] Icon rail mode: per-group column of initials + status dot; click shows agent; drag source; "+" new agent (`src/components/Sidebar.tsx`, `src/components/RailItem.tsx`) [in-house: drag and drop] [in-house: tooltip]
- [x] Rail initials: first letters of the first two `-`/`_` parts, else first two letters (`src/components/RailItem.tsx`)
- [x] Overlay sidebar mode over a scrim (xs/sm when toggled) (`src/App.tsx`) [in-house: drawer/overlay]
- [x] Popup menu component: positioned at click/anchor, kept on screen, ↑/↓ moves focus, Esc/outside click/window blur closes (`src/components/Menu.tsx`) [in-house: popover/menu]

## 5. Space view, tiling and panes

- [x] Space bar: space name, "N shown", "· N folded (too small at this density)" (`src/components/SpaceView.tsx`)
- [x] Space bar "Restore layout" button while a pane is maximised (`src/components/SpaceView.tsx`)
- [x] Space bar per-space density select: "Default (<global>)", Comfortable 80×20, Compact 60×12, Dense 40×8 (`src/components/SpaceView.tsx`, `src/layout/density.ts`) [in-house: select/dropdown]
- [x] Space bar preset buttons with mini layout icons, only presets that fit (`availablePresets`) (`src/components/SpaceView.tsx`, `src/layout/tree.ts`) [in-house: tooltip]
- [x] Presets: 1, 2, 3 ("1 big + 2"), 2×2, 3×2, 3×3, 4×3, 4×4, Auto grid (`src/layout/tree.ts`)
- [x] Applying a preset keeps shown agents first (focused first) then fills with members not shown elsewhere; rest become chips (`src/state/workspace.ts`)
- [x] Auto grid: tile all candidates evenly in rows sized to the area/density (`src/layout/tree.ts`, `src/lib/tiling.ts`)
- [x] Space bar "Move to new window" button (not on All) (`src/components/SpaceView.tsx`) [in-house: tooltip]
- [x] Layout is a tree of panes and row/col splits with fractional sizes (`src/layout/tree.ts`) [in-house: pane/split layout]
- [x] Drag split dividers to resize (min 8 % per side), live preview, persisted on release (`src/components/LayoutView.tsx`) [in-house: pane/split layout]
- [x] Render-time fit: panes below the density minimum fold away (focused pane kept) and become chips (`src/components/SpaceView.tsx`, `src/state/hidden.ts`) [in-house: pane/split layout]
- [x] Showing a folded agent swaps it into the focused pane (`src/App.tsx`)
- [x] Maximise one pane within the space (⌘⏎, double-click header, header button); only with 2+ panes (`src/state/workspace.ts`) [in-house: pane/split layout]
- [x] Pane focus on mouse down; focused pane styling (`src/components/PaneView.tsx`) [in-house: pane/split layout]
- [x] Pane header: status glyph, name, status word, blocked detail, machine chip (tooltip label + provider), project display (tooltip cwd), kind, branch with worktree tooltip, diffstat (`src/components/PaneHeader.tsx`) [in-house: tooltip]
- [x] Pane header "Re-apply rules & restart" button when rules are stale or failed (`src/components/rules/RulesStaleButton.tsx`)
- [x] Pane header actions: Stop process (when `caps.stop`), Remove agent, Maximise/Restore, Close pane (agent keeps running) (`src/components/PaneHeader.tsx`) [in-house: tooltip]
- [x] Pane header double-click maximises; header is a drag source (`src/components/PaneHeader.tsx`) [in-house: pane/split layout] [in-house: drag and drop]
- [x] Empty pane: "Empty pane", hint, up to 8 member chips to pick (drops into that pane) (`src/components/PaneView.tsx`)
- [x] Close button hidden on the only pane when it is empty (`src/components/LayoutView.tsx`)
- [x] Chip strip below the space: members not visible; click shows; drag source; "↗ <space>" / "↗ other window" when shown elsewhere (`src/components/ChipStrip.tsx`) [in-house: drag and drop] [in-house: tooltip]
- [x] Stopped/exited overlay on the pane: "<name> is stopped / has exited", reason text, Restart/Resume button (label from caps and `restartAs`), Remove button (`src/components/StoppedOverlay.tsx`, `src/lib/terminals.ts`)
- [x] Restart sends the terminal's current fitted size (`src/App.tsx`)
- [x] An agent is shown in at most one pane across all spaces/windows (`src/state/workspace.ts`)
- [x] `placeAgent`: split, or first empty pane, else focused pane; moving within a space swaps (`src/state/workspace.ts`)

## 6. Navigation and focus

- [x] Show agent: leave Wall/Review/viewer, close overlay sidebar; focus its pane in this window; or ask the owning window (via `focusRequest` in the blob + `focus_window`); or place it in the active space (`src/App.tsx`)
- [x] Focus moves keyboard focus into the agent's terminal on the next frame (`src/App.tsx`, `src/terminal/registry.ts`) [pitwall-term-view]
- [x] Cross-window focus request handled once per nonce by the owning window (`src/App.tsx`)
- [x] Jump to next blocked (⌘J) cycles through blocked agents in sidebar order (`src/App.tsx`)
- [x] Select agent by index ⌘1–⌘9 (`src/App.tsx`)
- [x] "Done" means unseen: selecting a done agent calls `mark_seen` at once; finishing while focused is seen after a 1.5 s glance if the window has focus (`src/App.tsx`)
- [x] Window focus marks the selected done agent seen (not in Wall) (`src/App.tsx`)

## 7. Drag and drop (agents)

- [ ] Pointer-driven DnD (not HTML5), 5 px threshold, floating ghost label, Esc cancels, window blur cancels (`src/lib/dnd.ts`) [in-house: drag and drop] BLOCKED: gpui 0.2.2 starts a drag after its private 2 px DRAG_THRESHOLD; 5 px is not settable
- [x] Drag sources: sidebar rows, rail items, pane headers, chips, Wall tiles (`src/lib/dnd.ts`) [in-house: drag and drop]
- [x] Drop targets: pane (left/right/top/bottom split zone or centre, edge 28 %), space area (auto-place), space tab, "+" tab (new space) (`src/lib/dnd.ts`, `src/layout/tree.ts`) [in-house: drag and drop]
- [x] Drop preview overlay on the pane: "Split <side>", "Swap", "Show here", "No room · adds as a chip" (`src/components/PaneView.tsx`) [in-house: drag and drop]
- [x] Centre drop swaps with the target's agent (displaced takes the old pane) or replaces it when the dragged agent was not shown (`src/state/workspace.ts`) [in-house: drag and drop]
- [x] Side drop moves (old pane closes, tree collapses, never duplicates) (`src/state/workspace.ts`) [in-house: drag and drop]
- [x] Drop on own pane is a no-op (`src/state/workspace.ts`) [in-house: drag and drop]
- [x] Drop that would go below the density minimum becomes a chip with an info toast (`src/state/workspace.ts`, `src/App.tsx`) [in-house: drag and drop] [in-house: toast]
- [x] Drop on a space tab auto-places (empty pane, else roomiest split) and switches to that space (`src/state/workspace.ts`, `src/App.tsx`) [in-house: drag and drop]
- [x] Drop on "+" creates a custom space containing the agent (`src/App.tsx`) [in-house: drag and drop]
- [x] A click following a drag is swallowed (`src/lib/dnd.ts`) [in-house: drag and drop]
- [x] Buttons inside a drag source keep their own clicks (`src/lib/dnd.ts`) [in-house: drag and drop]
- [x] Native file drop handler disabled on all windows (`src-tauri/tauri.conf.json`, `src-tauri/src/windows.rs`)

## 8. Terminals (interactive panes)

- [x] One long-lived terminal emulator per agent; mounting only re-parents it, so screen and scrollback survive switching (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] ~~Hidden terminals are parked off-screen and their output is batched (flush every 1 s or at 256 KiB; flushed when shown) (`src/terminal/registry.ts`) [pitwall-term-view]~~ n/a: xterm DOM workaround; hidden term views don't paint
- [x] Scrollback 5 000 lines (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] Font: JetBrains Mono Variable, line height 1.15, waits for font load before first fit (`src/terminal/registry.ts`, `src/terminal/screenStyle.ts`) [pitwall-term-view]
- [x] Unicode 11 widths (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] Cursor blink on interactive terminals (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] macOS Option acts as Meta (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] Dark and light terminal palettes (16 colours, cursor, selection), switched live with the theme (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] Output streamed via `attach_output` (replays buffered output, then raw PTY bytes); detach on dispose (`src/terminal/registry.ts`, `src/api.ts`) [pitwall-term-view]
- [x] Keystrokes sent via `write_input` (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] Fit to container on resize; sends `resize(cols, rows)` only when changed; retried on next fit if it failed (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] App shortcuts pass through the terminal (custom key handler) (`src/terminal/registry.ts`, `src/lib/shortcuts.ts`) [pitwall-term-view]
- [x] Copy/paste: macOS via the native Edit menu (⌘C/⌘V); Windows/Linux Ctrl+Shift+C copies selection, Ctrl+Shift+V pastes (`src/terminal/registry.ts`, `src/lib/host.ts`) [pitwall-term-view]
- [x] Per-tile font: auto-shrink from the base font down to a 10 px floor to keep the density minimum; user override via ⌘+/⌘−/⌘0 (`src/components/PaneView.tsx`, `src/layout/density.ts`) [pitwall-term-view]
- [x] Base font size global (Settings), default 13, range 9–22 (`src/layout/density.ts`) [pitwall-term-view]
- [x] Re-attach and reset the screen after a restart (not running → running) (`src/App.tsx`, `src/terminal/registry.ts`) [pitwall-term-view]
- [x] Dispose terminals of removed agents or agents now shown in another window (`src/App.tsx`) [pitwall-term-view]
- [x] New agents start at the size they will be shown at (empty pane, focused pane, or last fitted) (`src/terminal/registry.ts`, `src/terminal/spawnSize.ts`) [pitwall-term-view]
- [x] Onboarding hand-over size estimate per tile (`src/terminal/registry.ts`) [pitwall-term-view] (done in the settings redesign)
- [x] Scrollbar gutter accounted (14 px) in tile font fit (`src/components/PaneView.tsx`) [pitwall-term-view]
- [x] ~~Optional WebGL renderer path exists but is disabled (`MAX_WEBGL = 0`) (decide) (`src/terminal/registry.ts`) [pitwall-term-view]~~ n/a: xterm.js WebGL choice; GPUI renders on the GPU
- [x] ~~No terminal search, link detection or right-click menu exists today (xterm has no search/web-links addon loaded) (decide) (`package.json`, `src/terminal/registry.ts`) [pitwall-term-view]~~ n/a: xterm.js add-on decision; GPUI terminal has its own search and links
- [x] Plain terminals: kind `shell`, named after the folder ("home" for `~`, suffix -2, -3…) (`src/lib/terminals.ts`)
- [x] ⌘T folder: focused agent's cwd, else the project space's project, else `~` (`src/lib/terminals.ts`, `src/App.tsx`)
- [x] Agents started by hand inside a terminal are recognised (kind badge, "in terminal"), terminal restarts as that agent (`restartAs`) (`docs/spec/terminals.md`, `src/lib/terminals.ts`)

## 9. Wall

- [x] Wall mode per window (persisted in `ui.wall`), toggled by ⌘E, top bar, palette; any space tab or showing an agent leaves it (`src/App.tsx`, `src/components/Wall.tsx`)
- [x] Bar: "Wall", "every agent, live · view only · click a tile to take over", "Back esc" (`src/components/Wall.tsx`)
- [x] Esc leaves Wall (unless a dialog is open) (`src/components/Wall.tsx`)
- [x] Sections per project with machine headings, chevron collapse (persisted in `ui.wallCollapsed`), ▲ blocked count, summary when collapsed (`src/components/Wall.tsx`) [in-house: scroll view] [in-house: list/virtual list]
- [x] Tile: status glyph, name, status word, blocked detail, diffstat; body shows the screen (`src/components/Wall.tsx`)
- [x] Running tiles drawn from backend screen frames (`watch_screen`), not a terminal emulator; ≤ ~10 fps; only while the tile is within 200 px of view (`src/components/Wall.tsx`, `src/terminal/screenView.ts`) [pitwall-term-view]
- [x] Screen view reproduces xterm DOM rendering: cell metrics, letter-spacing per glyph, 256-colour cube, truecolour, bold-as-bright, dim, inverse, italic, underline styles, strikethrough, hidden, wide/cluster cells, outline cursor (`src/terminal/screenView.ts`, `src/terminal/screenStyle.ts`) [pitwall-term-view]
- [x] Cursor drawn only when the agent's own terminal would show it (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] Stopped tiles show the agent's own terminal's last screen if this window has it; else "stopped"/"not running" (`src/terminal/registry.ts`, `src/components/Wall.tsx`) [pitwall-term-view]
- [x] Scale to tile width (min scale 0.42, crop right), anchored bottom-left so bottom rows show (`src/components/Wall.tsx`) [pitwall-term-view]
- [x] Never resizes PTYs or sends input (`src/components/Wall.tsx`) [pitwall-term-view]
- [x] Click or Enter/Space on a tile shows that agent; tile is a drag source (`src/components/Wall.tsx`) [in-house: drag and drop]
- [x] Theme change restyles all screen views (`src/terminal/screenView.ts`) [pitwall-term-view]

## 10. Right panel (details for the focused agent)

- [x] Header: status glyph, name; close button in drawer mode (`src/components/RightPanel.tsx`)
- [x] Next up section: queue count, Auto-send toggle ("Send the next item when the agent is idle or done and you're not typing") (`src/components/NextUp.tsx`) [in-house: checkbox/toggle] [in-house: tooltip]
- [x] Queue list: numbered items, "Send now" (disabled when not running), "Remove from queue" (`src/components/NextUp.tsx`) [in-house: list/virtual list] [in-house: tooltip]
- [x] Compose textarea "queue a prompt…", ⌘↵ / Ctrl+↵ queues, "Queue" button; text sent verbatim; restored on failure (`src/components/NextUp.tsx`) [in-house: text input]
- [x] Unsent drafts kept per agent for the window session (`src/components/NextUp.tsx`)
- [x] Hint: "Sends when the agent is free" / "Auto-send off — send items manually" (`src/components/NextUp.tsx`)
- [x] Changes / Files tabs (Files only with `caps.explorer`), selected tab persisted (`pitwall.right.files`) (`src/components/RightPanel.tsx`) [in-house: tabs]
- [x] Changes: count, refresh control ("refreshing…"/"updated N s ago" + ↻), "Open in Review" button (with `caps.review`), diffstat (`src/components/Changes.tsx`, `src/components/Freshness.tsx`) [in-house: tooltip]
- [x] Where line: branch (or "detached HEAD") with "worktree" tag; folder path, click copies the absolute path ("Copied") (`src/components/Changes.tsx`) [in-house: tooltip]
- [x] Changes list: file icon, dir + base name, +/− or "bin", status letter (M/A/D/R/U); click opens the diff dialog (`src/components/Changes.tsx`, `src/components/FileIcon.tsx`) [in-house: list/virtual list] [in-house: scroll view]
- [x] Changes states: "Reading git…", "No changes since the agent started.", "Not a git repository…", error with Retry (`src/components/Changes.tsx`)
- [x] Changes refresh: forced on open, on totals change, every 5 s while visible, ↻ and ⌘⇧R (`src/components/Changes.tsx`, `src/lib/freshness.ts`)
- [x] Agent's worktrees sub-list (collapsed chip; expanding forces refresh; refresh control; error with Retry) (`src/components/Changes.tsx`) [in-house: list/virtual list]
- [x] Last sent: relative time (tooltip absolute), quoted text, or "Nothing sent through Pitwall yet…" (`src/components/LastSent.tsx`) [in-house: tooltip]
- [x] Files tab (section 14) (`src/components/explorer/FilesPanel.tsx`) [in-house: file tree]

## 11. Diff dialog (single file from Changes)

- [x] Sheet modal 1080 px: file icon, base + dir, status letter, diffstat or "bin", agent name, "esc to close", close button (`src/components/DiffView.tsx`) [in-house: modal/dialog]
- [x] Unified diff table with old/new line numbers, hunk and meta rows, +/− signs (`src/components/DiffView.tsx`, `src/lib/diff.ts`) [in-house: code/diff viewer] [in-house: scroll view]
- [x] States: "Loading diff…", "No textual changes.", "Couldn't load diff: …" (`src/components/DiffView.tsx`)

## 12. Review screen

- [x] Full main-area mode: ⌘R (toggle; opens on the focused agent when it can be reviewed), top bar, palette, Changes "Open in Review", viewer "Show diff", worktree "Review changes" (`src/App.tsx`)
- [x] Scope: active space's agents (project space → project, custom → members, All → focused agent's project, or everyone); plus the agent/worktree it was opened for (`src/lib/reviewScope.ts`)
- [x] Header: "Review", scope label, hint "what your agents changed · click a line number to comment" (`src/components/review/Review.tsx`)
- [x] "All projects" checkbox widens the list (session only) (`src/components/review/Review.tsx`, `src/App.tsx`) [in-house: checkbox/toggle]
- [x] Refresh control; opening Review forces a refresh of every agent's changes and worktrees; combined error with Retry (`src/components/review/Review.tsx`) [in-house: tooltip]
- [x] "Side by side" / "Inline" toggle, persisted (`pitwall.review.sideBySide`, default side by side) (`src/components/review/Review.tsx`) [in-house: segmented control]
- [x] "Back esc"; Esc leaves (unless a dialog, editor, field or menu has it) (`src/components/review/Review.tsx`)
- [x] Left list grouped by project → agent: collapse chevron, name button (status, name, branch), diffstat (`src/components/review/Review.tsx`) [in-house: list/virtual list] [in-house: scroll view]
- [x] Per-agent states: "this task only" note, error with Retry, "Reading git…", "No changes." (`src/components/review/Review.tsx`)
- [x] Compact-folder file tree (VS Code SCM style), collapsible folders (state per agent), comment count badge per file (`src/components/review/FileTree.tsx`) [in-house: file tree]
- [x] ↑/↓ moves focus through file rows; Enter opens (`src/components/review/Review.tsx`) [in-house: file tree]
- [x] Comments list per agent under its files: "file:line" + text, click jumps to the line, ✕ deletes (`src/components/review/Review.tsx`) [in-house: list/virtual list]
- [x] Agent's worktrees and per-project "Other worktrees" sections in the list (section 15) (`src/components/review/WorktreeSection.tsx`) [in-house: file tree]
- [x] Hint for agents outside a git repository ("N agents work outside a git repository") (`src/components/review/Review.tsx`)
- [x] Default selection: the agent it was opened for (its first file once read), else the first agent with changes (`src/components/review/Review.tsx`)
- [x] Selected file disappearing (discarded/merged) picks a neighbour (`src/components/review/Review.tsx`)
- [x] Picking an agent/worktree refreshes just that one (`src/components/review/Review.tsx`)
- [x] Per-agent task scope select: "Since the agent started · <name>" or "Task N · HH:MM · <prompt…>" (running, snapshot pending disabled) (`src/components/review/Review.tsx`) [in-house: select/dropdown]
- [x] Task prompt shown above the diff when a task is selected (`src/components/review/Review.tsx`)
- [x] File header: icon, base + dir, status letter, diffstat or "bin" (`src/components/review/Review.tsx`)
- [x] Footer actions for an agent: "Discard file", "Send comments (N)", "Commit" or "Commit & merge" (worktree agents); notice text after actions (`src/components/review/Review.tsx`)
- [x] Footer for a worktree: name, "locked" chip, "Open terminal", "Remove worktree…", "Commit" / "Commit & merge" by caps (`src/components/review/Review.tsx`) [in-house: tooltip]
- [x] Per-agent files refetched every 5 s while visible and on totals change (`src/components/review/Review.tsx`)
- [x] Empty states: "Nothing selected.", "Start an agent to review its changes.", "Pick a file on the left.", "Nothing to review here." (`src/components/review/Review.tsx`)
- [x] Draft comments kept in memory for the app session per agent (`src/components/review/comments.ts`)

## 13. Review diff editor

- [x] Side-by-side and inline diff (CodeMirror 6 merge view), styled after the Monaco diff editor (`src/components/review/diffView.ts`) [in-house: code/diff viewer]
- [x] Syntax highlighting with lazily loaded languages; JSON/TOML/diff kept plain; Markdown marker colouring (`src/components/review/editorSetup.ts`) [in-house: code/diff viewer]
- [x] Bracket pair colours, bracket matching, indentation detection + indent guides (`src/components/review/editorSetup.ts`) [in-house: code/diff viewer]
- [x] Read-only; typing shows a "Read-only…" message (`src/components/review/editorSetup.ts`) [in-house: code/diff viewer]
- [x] Gutter per editor: comment glyph, line number(s), +/− sign; hatched filler opposite inserted/deleted lines (`src/components/review/diffView.ts`) [in-house: code/diff viewer]
- [x] Collapsed unchanged regions ("N hidden lines"), click to expand (`src/components/review/diffView.ts`) [in-house: code/diff viewer]
- [x] Overlay scrollbars with cursor lane; click on track pages (`src/components/review/diffView.ts`) [in-house: code/diff viewer] [in-house: scroll view]
- [x] Diff overview ruler on the right (deletions left half, insertions right half) (`src/components/review/diffView.ts`) [in-house: code/diff viewer]
- [x] Click a line number puts the cursor on the line (⌘C copies it); any click in the margin starts a comment; hover shows "+" (`src/components/review/diffView.ts`) [in-house: code/diff viewer]
- [x] Commented lines: glyph + soft tint; hovering the glyph shows the comment text (`src/components/review/diffView.ts`) [in-house: code/diff viewer] [in-house: tooltip]
- [x] Comment composer: "Comment file:line", textarea "What should change here?", ⌘↵ adds, Esc cancels, "Collected, not sent." (`src/components/review/ReviewDiff.tsx`) [in-house: code/diff viewer] [in-house: text input]
- [x] Editor context menu: "Add review comment", separator, "Copy"; ↑/↓, Enter/Space, Esc (`src/components/review/ReviewDiff.tsx`) [in-house: code/diff viewer] [in-house: popover/menu]
- [x] Find widget (Monaco-like, top right): query, Match Case, Whole Word, Regex, "n of m", previous/next, Find in Selection, close (`src/components/review/findPanel.ts`) [in-house: code/diff viewer] [in-house: text input]
- [x] Find keys: Mod-F open, Enter / Shift+Enter next/prev, F3 / Shift+F3, Mod-G / Shift+Mod-G, Esc closes (`src/components/review/editorSetup.ts`, `src/components/review/findPanel.ts`) [in-house: code/diff viewer]
- [x] States: "Loading…", "Binary or very large file — not shown", "No content on either side", load error (`src/components/review/ReviewDiff.tsx`) [in-house: code/diff viewer]
- [x] Editor colours follow the scheme (dark/light) live (`src/components/review/ReviewDiff.tsx`, `src/components/review/review.css`) [in-house: code/diff viewer] [in-house: theme]
- [x] Reveal: scroll to a line on request (comment jump) (`src/components/review/ReviewDiff.tsx`) [in-house: code/diff viewer]

## 14. Code explorer (read-only)

- [x] Offered only with `AgentView.caps.explorer`; otherwise info toast "Focus an agent to browse its files" / "<name>'s files can't be read from here" (`src/App.tsx`) [in-house: toast]
- [x] Files tab: refresh control, "Open the file viewer" button, Where line, "Go to file ⌘P" and "Search ⌘⇧F" tool buttons, "Ignored" checkbox (`src/components/explorer/FilesPanel.tsx`) [in-house: tooltip] [in-house: checkbox/toggle]
- [x] "Show ignored files" persisted (`pitwall.explorer.showIgnored`); ignored entries dimmed (`src/components/explorer/useTree.ts`) [in-house: checkbox/toggle]
- [x] Lazy tree with VS Code file/folder icons, change letter on files, dot on folders with changes, symlink marker, "Reading…", per-folder error, "Only the first 5 000 entries are shown" (`src/components/explorer/ExplorerTree.tsx`) [in-house: file tree] [in-house: list/virtual list]
- [x] Tree keys: ↑/↓ move, → expand/enter, ← collapse/parent, click opens file or toggles folder (`src/components/explorer/ExplorerTree.tsx`) [in-house: file tree]
- [x] One tree model per agent shared by the Files tab and the viewer; refreshed on open, ↻, ⌘⇧R, totals change, every 10 s while visible (`src/components/explorer/useTree.ts`) [in-house: file tree]
- [x] Viewer: full main area; bar "Files", agent, "read-only · <cwd>", refresh, "Go to file ⌘P", "Back esc"; Esc leaves (`src/components/explorer/Viewer.tsx`)
- [x] Viewer side pane switch "Explorer" / "Search" (`src/components/explorer/Viewer.tsx`) [in-house: segmented control]
- [x] Viewer tabs of open files (icon, name, status colour, close button, middle-click closes, Enter/Space selects); kept per agent for the window session (`src/components/explorer/Viewer.tsx`) [in-house: tabs]
- [x] File header: icon, base + dir, status letter, size, "Copy path" (absolute), "Show diff" (opens Review on the file) (`src/components/explorer/Viewer.tsx`) [in-house: tooltip]
- [x] Read-only editor (same setup as Review: highlighting, find, gutter) with reveal of a line/selection (`src/components/explorer/FileEditor.tsx`) [in-house: code/diff viewer]
- [x] Binary file: "Binary file · size · not shown"; over 2 MiB: "Large file · size · not opened" + "Load anyway" (≤ 10 MiB) (`src/components/explorer/Viewer.tsx`, `src/explorerApi.ts`)
- [x] Quiet re-read on refresh/totals change; "Changed on disk since it was opened. Reload" banner (`src/components/explorer/Viewer.tsx`)
- [x] Empty viewer: "Pick a file on the left" + "Go to file ⌘P" (`src/components/explorer/Viewer.tsx`)
- [x] Quick open (⌘P): palette-style fuzzy picker over `list_all_files`, highlighted matches in base and dir, recent files (20 per agent) first on empty query, ↑/↓/Enter, "Only the first 100 000 files are listed", footer hint (`src/components/explorer/QuickOpen.tsx`, `src/lib/explorer.ts`) [in-house: command palette] [in-house: text input] [in-house: list/virtual list]
- [x] Search pane (⌘⇧F): query box with Aa / whole word / .* toggles, include/exclude globs, "Leave out node_modules and bower_components", hint ".gitignore applies; .git is never searched" (`src/components/explorer/SearchPane.tsx`) [in-house: text input] [in-house: checkbox/toggle]
- [x] Search as you type (300 ms debounce), Enter re-runs, newer search cancels older, leaving cancels (`src/components/explorer/SearchPane.tsx`) [in-house: text input]
- [x] Results grouped by file (collapsible, count), match lines with highlighted ranges and line number; click opens at line/columns (`src/components/explorer/SearchPane.tsx`) [in-house: list/virtual list] [in-house: scroll view]
- [x] Summary "N results in M files" (+ "stopped there, narrow the search" when truncated), "Searching…", error (`src/components/explorer/SearchPane.tsx`)
- [x] Search form and results kept per agent for the window session (`src/components/explorer/SearchPane.tsx`)
- [x] File icons: Material Icon Theme subset (137 SVGs) by file name, then longest extension, folder icons open/closed (`src/lib/fileIcons.ts`, `src/assets/file-icons/`) [in-house: file icons]

## 15. Worktrees

- [x] One worktree list per window (`list_worktrees`), shared by sidebar, Changes and Review; only when any agent has `caps.worktrees` (`src/lib/useWorktrees.ts`)
- [x] Polled every 30 s while visible, 1 s after agents' numbers settle; forced refresh on expand/↻/⌘⇧R (`src/lib/useWorktrees.ts`)
- [x] Worktree row: branch (or "<name> (detached)"), "locked" chip (reason tooltip), "gone" chip (prunable), diffstat read only while shown; tooltip path and "A process of this agent works here" (`src/components/WorktreeRows.tsx`) [in-house: list/virtual list] [in-house: tooltip]
- [x] Click a worktree row opens Review on it (`src/components/WorktreeRows.tsx`)
- [x] Worktree context menu: "Review changes", "Open terminal here" (caps.terminal), "Remove worktree…" (caps.remove) (`src/components/WorktreeRows.tsx`) [in-house: popover/menu]
- [x] Worktree changes against merge-base polled every 15 s (10 s in Review) while shown (`src/lib/useWorktrees.ts`)
- [x] Review worktree section: toggle, name/branch/locked, diffstat, refresh control, "Its folder is gone.", "No changes against <target>", file tree (`src/components/review/WorktreeSection.tsx`) [in-house: file tree]
- [x] Review main pane for a worktree: "Worktree <name> · <branch> · against <target>" header and diff (no comments) (`src/components/review/Review.tsx`) [in-house: code/diff viewer]
- [x] Commit / Commit & merge a worktree (same dialog as agents; conflicts become a notice) (`src/components/review/Review.tsx`) [in-house: modal/dialog]

## 16. Status strip, toasts, notifications in-app

- [x] Status strip calm: "No agents yet" or "◐ N working · Nothing needs you" and "⌘K commands" (`src/components/StatusStrip.tsx`)
- [x] Status strip amber: "▲ <name> needs you: <detail> · +N more", "Jump ⌘J" (`src/components/StatusStrip.tsx`, `src/lib/statusStrip.ts`)
- [x] Status strip mint: "⚑ <name> finished · +N more · ◐ N working", "Show" (`src/components/StatusStrip.tsx`)
- [x] Toasts: tones blocked ▲ / done ⚑ / error ✕ / info ●; title + detail; click jumps to agent or runs action; ✕ dismiss (`src/components/Toasts.tsx`) [in-house: toast]
- [x] Toasts: max 4, one per agent, auto-dismiss blocked 9 s, error 8 s, others 5 s (`src/lib/useToasts.ts`) [in-house: toast]
- [x] `attention` events become toasts ("<name> needs you" / "<name> is done"), except done for the focused agent while the window has focus (`src/App.tsx`) [in-house: toast]
- [x] Every failing backend call becomes a red "Couldn't <what>" toast (`src/App.tsx`) [in-house: toast]
- [x] One-time "macOS blocked a folder" toast pointing to Settings → Permissions on a privacy error (`src/App.tsx`, `src/lib/permissions.ts`) [in-house: toast]
- [x] Status glyphs: working pulse dot, blocked ▲, done ⚑, idle ●, unknown ?, exited/stopped ■; status words (`src/lib/status.ts`, `src/components/StatusGlyph.tsx`)

## 17. Command palette (⌘K)

- [x] Palette modal, top-anchored, input "Jump to an agent, or type  name: prompt  to queue it"; ↑/↓, Enter, hover selects, click runs (`src/components/CommandPalette.tsx`) [in-house: command palette] [in-house: text input] [in-house: list/virtual list]
- [x] Word-AND substring matching over each item's search text (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Inline queue syntax "name: text" or "queue [for] name: text" → single "Queue for <name>" item (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: go to each agent (status, project, "current"), "Remove <name>…" per agent (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Jump to next blocked ⌘J" (only when any blocked), "New agent ⌘N" (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Terminal at <typed path>" (when query looks like a path, listed first), "New terminal here ⌘T", "New terminal in folder… ⌘⇧T", "Terminal in <project>" per project (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Queue for <name>: …" per agent (prefills the input and keeps the palette open) (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Toggle Wall ⌘E", "Review changes ⌘R", "Go to file… ⌘P", "Search in files ⌘⇧F", "Browse files (read-only)" (file items only with `caps.explorer`) (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Move space to new window ⌘⇧N", "Tile layout: <preset>" (presets that fit) (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Density: <d> (default for all spaces)", "Density for this space: <d>", "Density for this space: use default" (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Toggle agents sidebar ⌘B", "Toggle details panel ⌘.", "Settings ⌘,", "Theme: System/Dark/Light" (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Items: "Quit Pitwall (agents keep running)", "Quit and stop all agents" (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] Empty query hides queue-for, preset, density, terminal-in and theme items (`src/components/CommandPalette.tsx`) [in-house: command palette]
- [x] "No matches. Try “name: your prompt” to queue." (`src/components/CommandPalette.tsx`) [in-house: command palette]

## 18. Dialogs

- [x] Modal base: backdrop click closes, Esc closes (capture), autofocus first `[data-autofocus]`/field, focus restored on close, title + ✕ header; variants dialog / palette / sheet (`src/components/Modal.tsx`) [in-house: modal/dialog]
- [x] New agent: "Runs on" select (only when 2+ machines can create), "Reading what <machine> has…", form error, partial-choices hint (`src/components/NewAgentDialog.tsx`) [in-house: modal/dialog] [in-house: select/dropdown]
- [x] New agent: Agent kind radio cards ("installed" / "not installed" / "any CLI"), disabled when not installed; first installed preselected (`src/components/NewAgentDialog.tsx`) [in-house: radio group] [in-house: tooltip]
- [x] New agent: custom command input for "any CLI" kinds (`src/components/NewAgentDialog.tsx`) [in-house: text input]
- [x] New agent: Project select (recent conversations then project list) + "Choose folder…" (native picker, typed path, "Browse…"); absolute path validation (`src/components/NewAgentDialog.tsx`) [in-house: select/dropdown] [in-house: text input]
- [x] New agent: Name (auto-suggested slug from folder, unique, rule `^[a-z][a-z0-9_-]{0,31}$` or the machine's rule) with inline errors (`src/components/NewAgentDialog.tsx`, `src/lib/status.ts`) [in-house: text input]
- [x] New agent: platform form fields (selects / text with rules, conditional `when`), summary "Also creates …" for non-folder machines (`src/components/CreateFormFields.tsx`, `src/lib/createForm.ts`) [in-house: select/dropdown] [in-house: text input]
- [x] New agent: "Separate worktree" checkbox (kinds with worktree support) (`src/components/NewAgentDialog.tsx`) [in-house: checkbox/toggle]
- [x] New agent: Rules field (section 21) (`src/components/rules/RulesField.tsx`) [in-house: select/dropdown]
- [x] New agent: submit "Start"/form's label ↵, "Starting…"/"Creating…", "Creating on <machine>… can take a few minutes", submit error; preselect project from caller (`src/components/NewAgentDialog.tsx`) [in-house: modal/dialog]
- [x] After create: patch agent, show it, report rules errors as a toast (`src/App.tsx`, `src/components/rules/RulesField.tsx`) [in-house: toast]
- [x] New terminal (⌘⇧T): Folder input (prefilled), "Browse…", up to 8 filtered recent/listed folders, "Open terminal ↵" (needs `/` or `~` path) (`src/components/TerminalDialog.tsx`) [in-house: modal/dialog] [in-house: text input] [in-house: list/virtual list]
- [x] Bring into Pitwall: explains resume in a new agent, title quote, "Close the original first (pid N)", "I closed it, resume here" (`src/components/BringInDialog.tsx`) [in-house: modal/dialog]
- [x] Remove agent: text differs for adopted sessions (keeps running) vs local; "Also remove worktree" checkbox with path, git note and branch kept; red warning; "Remove" / "Remove agent & worktree" (`src/components/RemoveDialog.tsx`) [in-house: modal/dialog] [in-house: checkbox/toggle]
- [x] Remove worktree: path, explanation (no --force, branch kept or detached warning), red warning; refusal text for locked / agent-owned; "Remove worktree" (`src/components/RemoveWorktreeDialog.tsx`) [in-house: modal/dialog]
- [x] Approval dialog (section 26) (`src/components/ApprovalDialog.tsx`) [in-house: modal/dialog]
- [x] Prompt dialog (Send comments / Merge conflict): editable textarea sent verbatim; "Send now" / "Add to Next up" / "Send now anyway" by agent state; ⌘↵ sends default (`src/components/review/ReviewDialogs.tsx`) [in-house: modal/dialog] [in-house: text input]
- [x] Discard file dialog: deleted (untracked) vs restored to base commit; task-scope note; "This can't be undone." (`src/components/review/ReviewDialogs.tsx`) [in-house: modal/dialog]
- [x] Commit / Commit & merge dialog: reads merge status; commit message (required when uncommitted); "Then merge <branch> into <target>" checkbox; refusals for dirty target, detached target/worktree; ahead count; main-checkout warning; result/err text (`src/components/review/ReviewDialogs.tsx`) [in-house: modal/dialog] [in-house: text input] [in-house: checkbox/toggle]
- [x] Merge conflict: prefilled "Rebase onto <branch> and resolve conflicts." prompt dialog (`src/components/review/ReviewDialogs.tsx`, `src/components/review/comments.ts`) [in-house: modal/dialog]
- [x] Rules: delete rule set uses a native confirm() (`src/components/rules/RulesSettings.tsx`) [in-house: modal/dialog]

## 19. Settings dialog (each field)

- [x] Opened by ⌘, (Ctrl+Shift+,), gear button, palette, native menu "Settings…", tray "Settings…", access-hint toast (`src/App.tsx`, `src-tauri/src/menu.rs`) [in-house: modal/dialog] [in-house: scroll view] (done in the settings redesign)
- [x] Projects & agents: "Scan again" button opens the rescan screen (`src/components/SettingsDialog.tsx`)
- [x] Exact Codex status (hooks): installed / not installed chip, hooks file path, "Install hooks…" → confirm box (backup, append, no-op outside Pitwall) → Install/Cancel (`src/components/SettingsDialog.tsx`)
- [x] Claude Code: "automatic" chip, "Your ~/.claude settings are never edited." (`src/components/SettingsDialog.tsx`)
- [x] "Show agents running elsewhere" checkbox (`ui.hideElsewhere` inverted) (`src/components/SettingsDialog.tsx`) [in-house: checkbox/toggle]
- [x] Permissions · Full Disk Access row (hidden where it does not apply): chip granted/not granted/unknown/checking, explanation, 3 steps, "Open Settings"; live polling while open (`src/components/PermissionsSettings.tsx`)
- [x] Command-line tool: installed chip, link path, "This build doesn't include the tool.", "Install command-line tool…" → choose dir (radio per candidate) → explanation (not-on-PATH note) → Install/Cancel (`src/components/CliSettings.tsx`) [in-house: radio group]
- [x] Appearance → Theme: System / Dark / Light segmented (`src/components/AppearanceSettings.tsx`) [in-house: segmented control] [in-house: theme]
- [x] Appearance → Look: Flat / Glass (label "Glass · native", "Glass · Mica" or "Glass lite"; tooltip per tier) (`src/components/AppearanceSettings.tsx`, `src/lib/look.ts`) [in-house: segmented control] [in-house: tooltip]
- [x] Appearance → Reduce motion toggle (`src/components/AppearanceSettings.tsx`) [in-house: checkbox/toggle]
- [x] Tiles → Density: Comfortable / Compact / Dense (global default; tooltip cell size) (`src/components/LayoutSettings.tsx`) [in-house: segmented control] [in-house: tooltip]
- [x] Tiles → Terminal font: − / value px / + (9–22), "Reset" when not 13; hint about per-tile ⌘+/⌘−/⌘0 (`src/components/LayoutSettings.tsx`)
- [x] Rules section (section 21) (`src/components/rules/RulesSettings.tsx`)

## 20. Onboarding (welcome screen and "Scan again")

- [x] Shown full-window on first launch in the main window when `get_onboarded` is false; "Scan again" from Settings (`src/App.tsx`, `src/components/onboarding/useProjects.ts`) [in-house: modal/dialog]
- [x] Step 1 Folder access (welcome only): skipped when FDA does not apply or is not "denied" (`src/components/onboarding/FolderAccess.tsx`, `src/lib/permissions.ts`)
- [x] Folder access screen: board "PIT 1/2 ACCESS/GO", explanation, 3 steps, live status (polls every 2 s and on focus), "Open Settings"/"Open Settings again", "Skip for now", "Continue →" when granted (`src/components/onboarding/FolderAccess.tsx`)
- [x] Scan screen: board with step counter and SCAN/BOX, title, lede; ✕ and Esc close only in rescan mode (`src/components/onboarding/Onboarding.tsx`) [in-house: scroll view]
- [x] Checklist with progress bar and 6 steps (Agents on your PATH, Projects, Conversations you can continue, Running now, Rule files, Codex hooks) driven by `scan-progress` (✓ ✕ – glyphs, summaries, "looking…") (`src/components/onboarding/Onboarding.tsx`)
- [x] Projects section: All/None, list (first 8, "Show all N"), checkbox (disabled when already added), path, chips (in Pitwall, sources Claude/Codex/VS Code/Cursor/Folder, no git, rulesync, CLAUDE.md, AGENTS.md), last used (`src/components/onboarding/Onboarding.tsx`) [in-house: checkbox/toggle] [in-house: list/virtual list]
- [x] Default project selection: recent agent projects (≤ 30 days, max 12) (`src/components/onboarding/projects.ts`)
- [x] Per ticked project "Start a new agent" toggle with kind select (when no conversation covers it) (`src/components/onboarding/Onboarding.tsx`) [in-house: checkbox/toggle] [in-house: select/dropdown]
- [x] "Add folder…" inline form: typed path, Add, Browse… (native picker opens automatically), Cancel, error (`src/components/onboarding/Onboarding.tsx`) [in-house: text input]
- [x] Conversations to continue: list (first 6 or ticked, "Show all"), None; kind chip, title, project · relative time, "in Pitwall" / "open elsewhere" chips, warning when ticked and open elsewhere (`src/components/onboarding/Onboarding.tsx`) [in-house: checkbox/toggle] [in-house: list/virtual list]
- [x] Default ticked conversations: latest per ticked project in the last 3 days (not in Pitwall, not elsewhere, kind installed) (`src/components/onboarding/projects.ts`)
- [x] Conversations started outside a project ("Started in ~"): All/None, "Show under <project>" select incl. "Add folder…"; choice remembered (`src/components/onboarding/Onboarding.tsx`, `src/components/onboarding/projects.ts`) [in-house: select/dropdown]
- [x] Running now (this machine): rows with kind, folder, title/session/pid, "Bring into Pitwall"/"can't bring over", "Show under" picker, close-the-original note (`src/components/onboarding/Onboarding.tsx`) [in-house: checkbox/toggle] [in-house: select/dropdown]
- [x] Running now (other places, e.g. agw VMs): machine groups with sessions (kind, name, workspace, user, status chip, "Add to Pitwall"); "could not be listed" note; tracking-only hint (`src/components/onboarding/Onboarding.tsx`) [in-house: checkbox/toggle]
- [x] Codex status: "hooks installed" or "Exact Codex status (install hooks)" checkbox with "Show changes" confirm text (`src/components/onboarding/Onboarding.tsx`) [in-house: checkbox/toggle]
- [x] Footer: errors, "Starting N agents… i/N", "No agents will start…", Skip/Cancel, primary "Start Pitwall · N agents →" / "Add N projects · start N agents" / "Done" (`src/components/onboarding/Onboarding.tsx`)
- [x] Finish: `complete_onboarding` (partial failure "Projects were saved…" becomes a toast), then start agents one by one (continue / create / adopt) at hand-over sizes; failures toast, rest continue (`src/components/onboarding/Onboarding.tsx`, `src/components/onboarding/projects.ts`) [in-house: toast] (done in the settings redesign)
- [x] Hand-over: new agents tiled in the All space (cap by breakpoint), first focused, Wall/Review off (`src/App.tsx`) (done in the settings redesign)
- [x] Skip on welcome still marks onboarded (`src/components/onboarding/Onboarding.tsx`)
- [x] "Elsewhere" polling waits until onboarding is done (avoids macOS folder prompts) (`src/App.tsx`)

## 21. Rules (via rulesync)

- [x] Settings → Rules: status chip "rulesync <ver>" / "npx rulesync" / "not installed" (`src/components/rules/RulesSettings.tsx`)
- [x] Install hint "npm install -g rulesync" and "Use npx -y rulesync when it isn't installed" checkbox (disabled without npx) (`src/components/rules/RulesSettings.tsx`) [in-house: checkbox/toggle]
- [x] "Manage rules…" / "Hide rules" expander (`src/components/rules/RulesSettings.tsx`)
- [x] Library: count, "Open folder" (reveals in file manager), "Refresh", rule list (label, source chip, root, local, description), empty hint with data dir path (`src/components/rules/RulesSettings.tsx`) [in-house: list/virtual list] [in-house: tooltip]
- [x] Import form: kind select (CLAUDE.md / AGENTS.md file, project's .rulesync, git repository), source input with kind-specific placeholder and hint, Import, log/error output (`src/components/rules/RulesSettings.tsx`) [in-house: select/dropdown] [in-house: text input]
- [x] Sources list: name, kind chip, origin, "Pull" (git), "Remove" (`src/components/rules/RulesSettings.tsx`) [in-house: list/virtual list] [in-house: tooltip]
- [x] Rule sets: list (name, rules), Edit, Delete (confirm), "New set" (disabled with an empty library) (`src/components/rules/RulesSettings.tsx`) [in-house: list/virtual list] [in-house: modal/dialog]
- [x] Set editor: name, checkbox per library rule, missing rules shown, Save/Cancel (`src/components/rules/RulesSettings.tsx`) [in-house: text input] [in-house: checkbox/toggle]
- [x] Project defaults: up to 40 projects with a set select (None / sets) (`src/components/rules/RulesSettings.tsx`) [in-house: select/dropdown]
- [x] New agent Rules field: "No extra rules"/"Project rules only" + sets (project default excluded), project default name, hints, unavailable error, "Write rule files into my main checkout" checkbox when no worktree (`src/components/rules/RulesField.tsx`) [in-house: select/dropdown] [in-house: checkbox/toggle]
- [x] Pane "Re-apply rules & restart" button: `apply_rules` then restart; error variant (`src/components/rules/RulesStaleButton.tsx`) [in-house: tooltip]
- [x] Shared per-agent rules poller (every 20 s, on window focus, after changes) (`src/rules/useAgentRules.ts`)

## 22. Elsewhere (agents in other terminal apps)

- [x] Sidebar group "Elsewhere" polled every 10 s while visible and enabled (`src/lib/useElsewhere.ts`)
- [x] Rows hide conversations a Pitwall agent already has (`src/lib/terminals.ts`)
- [x] Row: dot, title or folder or kind, kind, folder; tooltip with pid and cwd (`src/components/ElsewhereGroup.tsx`) [in-house: tooltip]
- [x] "Bring in" button (disabled when the conversation or folder is unknown) opens the Bring into Pitwall dialog (`src/components/ElsewhereGroup.tsx`) [in-house: tooltip] [in-house: modal/dialog]
- [x] Group hidden by the Settings toggle; collapse state shared with project collapse list (`src/components/Sidebar.tsx`)

## 23. Keyboard shortcuts

App chords are captured before the terminal (`src/lib/useShortcuts.ts`,
`src/lib/shortcuts.ts`, `src/lib/host.ts`). macOS uses ⌘ (shift variant ⌘⇧);
Windows/Linux use Ctrl+Shift (shift variant Ctrl+Shift+Alt), matched by physical
key code.

- [x] Command palette (toggle): ⌘K · Ctrl+Shift+K
- [x] New agent: ⌘N · Ctrl+Shift+N
- [x] Move active space to new window: ⌘⇧N · Ctrl+Shift+Alt+N
- [x] New terminal here: ⌘T · Ctrl+Shift+T
- [x] New terminal in folder…: ⌘⇧T · Ctrl+Shift+Alt+T
- [x] Jump to next blocked: ⌘J · Ctrl+Shift+J
- [x] Toggle sidebar: ⌘B · Ctrl+Shift+B
- [x] Toggle details panel: ⌘. · Ctrl+Shift+.
- [x] Toggle Wall: ⌘E · Ctrl+Shift+E
- [x] Toggle Review: ⌘R · Ctrl+Shift+R (also blocks webview reload)
- [x] Refresh source-control views: ⌘⇧R · Ctrl+Shift+Alt+R (`src/lib/freshness.ts`)
- [x] Quick open file: ⌘P · Ctrl+Shift+P
- [x] Search in files: ⌘⇧F · Ctrl+Shift+Alt+F
- [x] Maximise / restore focused pane: ⌘⏎ · Ctrl+Shift+Enter
- [x] Settings: ⌘, · Ctrl+Shift+, (Windows File menu accelerator Ctrl+Shift+Comma) (`src-tauri/src/menu.rs`)
- [x] Focused tile font +1: ⌘= or ⌘+ · Ctrl+Shift+= or Numpad + (base font when in Wall or nothing focused)
- [x] Focused tile font −1: ⌘− · Ctrl+Shift+− (incl. Numpad −)
- [x] Focused tile font reset: ⌘0 · Ctrl+Shift+0
- [x] Select agent 1–9: ⌘1–⌘9 · Ctrl+Shift+1–9 (incl. numpad)
- [x] Terminal copy / paste: ⌘C / ⌘V via Edit menu · Ctrl+Shift+C / Ctrl+Shift+V (`src/terminal/registry.ts`) [pitwall-term-view]
- [x] Form submit (Next up queue, prompt dialog, comment): ⌘↵ · Ctrl+↵ (either works everywhere) (`src/components/NextUp.tsx`, `src/components/review/ReviewDialogs.tsx`, `src/components/review/ReviewDiff.tsx`) [in-house: text input]
- [x] Esc: close modal/palette/menu; leave Wall, Review, viewer; cancel drag; cancel tab rename; close rescan; cancel comment; close find
- [x] Enter: run palette/quick-open item; submit dialogs; commit tab rename; open tree file; re-run search
- [x] Arrows: palette/quick-open/menu ↑/↓; explorer tree ↑/↓/←/→; Review file list ↑/↓; diff menu ↑/↓ [in-house: list/virtual list] [in-house: file tree]
- [x] Editor: Mod-F find, Enter/Shift+Enter next/prev in find, F3/Shift+F3, Mod-G/Shift+Mod-G, Esc closes find, default CodeMirror navigation keys (`src/components/review/editorSetup.ts`) [in-house: code/diff viewer]
- [x] Mouse: double-click tab renames; double-click pane header maximises; middle-click viewer tab closes; right-click project/agent/worktree opens menus [in-house: tabs] [in-house: pane/split layout] [in-house: popover/menu]
- [x] Shortcut labels everywhere rendered per desktop (`keys()`, compact "⌃⇧" form in rows) (`src/lib/host.ts`, `src/components/Kbd.tsx`) [in-house: keybinding label]
- [x] macOS native menu shortcuts: ⌘Q quit, ⌘W close window, ⌘H hide, ⌥⌘H hide others, ⌘M minimise, ⌃⌘F full screen, Edit ⌘Z/⇧⌘Z/⌘X/⌘C/⌘V/⌘A (Tauri default menu) (`src-tauri/src/menu.rs`)

## 24. Appearance

- [x] Theme: System (follow OS) / Dark / Light; stored in `ui.theme`, every window follows; native window theme set too (title bar) (`src/lib/theme.ts`) [in-house: theme]
- [x] Pre-paint: theme, look and motion read from localStorage mirrors before first render (no flash) (`index.html`)
- [x] Look: Flat (default) / Glass; tiers native (macOS vibrancy) / mica (Windows 11) / lite (painted gradient, no blur) (`src/lib/look.ts`, `src/styles/glass.css`) [in-house: theme]
- [x] Glass only on chrome (top bar, tabs, sidebar, right panel, strip, palette, dialogs, menus, toasts, chips); terminals, Wall tiles, code views stay solid (`src/styles/glass.css`, `docs/spec/layout.md`) [in-house: theme]
- [x] ~~Glass starts on the lite tier and switches when the window confirms its native material (`index.html`, `src/lib/look.ts`)~~ n/a: webview waited on an async command; GPUI picks the glass tier before painting
- [x] Reduce transparency (OS or media query) shows Flat and removes the material; Increase contrast makes glass nearly opaque (`data-contrast`) (`src/lib/look.ts`, `src-tauri/src/platform/glass.rs`) [in-house: theme]
- [x] Each window re-checks accessibility settings on focus (`src/lib/look.ts`)
- [x] Reduce motion toggle (`data-motion="reduce"`) plus OS prefers-reduced-motion; all animation paused while the page is hidden (`src/lib/look.ts`, `src/styles/motion.css`)
- [x] Motion: open fade + rise/scale for palette/dialogs/menus, toast slide, view cross-fade, press pop, blocked halo twice + glyph bounce, done flag pop, working dot loop, Glass ring + primary sheen; none on the Wall (`src/styles/motion.css`)
- [x] Design tokens (dark default + light, two identical light copies): surfaces, lines, text levels, focus, green/finish/amber/red + soft variants, flag, diff colours, shadow, backdrop (`src/styles/tokens.css`) [in-house: theme]
- [x] Layout tokens: radii 4/6/10, top bar 44 px, sidebar 268 px, right panel 320 px, status strip 28 px (`src/styles/tokens.css`, `src/styles/layout.css`) [in-house: theme]
- [x] Fonts bundled: Inter Variable (UI), Barlow Condensed 600/700 (labels), JetBrains Mono Variable (mono/terminal) (`src/main.tsx`, `src/styles/tokens.css`)
- [x] Icon set (inline SVG): sidebar, panel, search, gear, plus, x, send, stop, restart, refresh, trash, branch, folder, chevron, grid, wall, maximize, restore, window, more, review, terminal, file, expand, copy (`src/components/Icon.tsx`) [in-house: icons]
- [x] Status colours identical in both looks: amber needs you, mint done, green working (`src/styles/tokens.css`) [in-house: theme]
- [x] Review/explorer editor token colours after Monaco vs / vs-dark (`src/components/review/review.css`) [in-house: theme] [in-house: code/diff viewer]
- [x] Styles per area to mirror: base, layout, agents, panel, overlays, space, wall, terminals, glass, motion, onboarding, review, explorer, rules (`src/styles/`, `src/components/*/*.css`) [in-house: theme]

## 25. Multi-window

- [x] Main window label `main`; extra windows `pitwall-N` (next free number) showing one space each (`src-tauri/src/windows.rs`)
- [x] Move space to new window (⌘⇧N, tab button, space bar button, palette) opens a window with `?space=<id>`; the new window claims the space (`src/App.tsx`, `src-tauri/src/windows.rs`)
- [x] Every window has its own tabs, sidebar, Wall flag, toasts, strip; shares the UI blob (`src/App.tsx`)
- [x] UI blob changes broadcast to other windows (`ui-state-changed`, ignoring own writes) (`src/state/useUiState.ts`)
- [x] Closing a secondary window returns its spaces to main (event) and on startup main reclaims spaces of windows that no longer exist (after 1.5 s) (`src/App.tsx`, `src/state/workspace.ts`)
- [x] Secondary windows restored on launch with saved bounds; main bounds restored; bounds ignored when tiny or off all monitors (`src-tauri/src/windows.rs`)
- [x] Window default size 1400×900, min 640×480, title "Pitwall" (`src-tauri/tauri.conf.json`, `src-tauri/src/windows.rs`)
- [x] Settings from the native menu opens in the focused window (fallback main) (`src-tauri/src/menu.rs`)
- [x] Glass material requested per window (`set_window_glass`) (`src-tauri/src/commands/host.rs`)
- [x] Each window attaches its own terminal output; terminals of agents shown in another window are disposed (`src/App.tsx`)
- [x] ~~Spec: drag a tab out to make a window (not implemented) (decide) (`docs/spec/layout.md`)~~ n/a: never built in the React app (spec "decide"); post-migration idea

## 26. Approvals (requests from the `pitwall` CLI)

- [x] CLI socket served inside the app process; requests that change something wait for the user (`src-tauri/src/server.rs`)
- [x] Approval dialog in every window for the first pending request: "<requester> wants to <summary>.", details list, "Asked by …" line (`src/components/ApprovalDialog.tsx`) [in-house: modal/dialog]
- [x] "Allow … to do this again without asking (until Pitwall quits)" checkbox when rememberable (`src/components/ApprovalDialog.tsx`) [in-house: checkbox/toggle]
- [x] Countdown "Denied in m:ss" (120 s timeout) and "· N more waiting"; Deny / Allow; closing denies; error text (`src/components/ApprovalDialog.tsx`, `src-tauri/src/server.rs`)
- [x] New request bounces the Dock / flashes taskbar (critical user attention) (`src-tauri/src/server.rs`)
- [x] Only this app's own UI can answer (not socket clients) (`src-tauri/src/commands/cli.rs`)

## 27. OS integration

- [x] macOS app menu: Tauri default menu + "Settings… ⌘," after About + "Quit and Stop Agents" at the end of the app menu (`src-tauri/src/menu.rs`)
- [x] Windows File menu: "Settings… Ctrl+Shift+,", Close Window, Quit Pitwall, Quit and Stop Agents; no Edit menu (`src-tauri/src/menu.rs`)
- [x] Linux: no native menu (Settings and Quit live in the palette) (`src-tauri/src/menu.rs`, `crates/pitwall-core/src/host.rs`)
- [x] Windows tray icon: tooltip, left click shows main, menu Open Pitwall / Settings… / Quit Pitwall / Quit and Stop Agents (`src-tauri/src/platform/windows.rs`)
- [x] Badge: Dock badge count of blocked agents (macOS, Linux launchers); Windows taskbar overlay icon (red disc with count, "9+" past 99) + tray tooltip "Pitwall — N agents need you" (`src-tauri/src/platform/unix.rs`, `src-tauri/src/platform/windows.rs`, `src-tauri/src/platform/badge.rs`)
- [x] System notifications on attention ("Needs you — <detail>", "Done — <detail>", "Finished its turn"), suppressed when the main window is visible and focused, and in bench mode (`src-tauri/src/attention.rs`)
- [x] Close main: hides (macOS with Dock, Windows with tray; agents keep running); Linux quits (agents still keep running in holders); secondary windows close for real (`src-tauri/src/lib.rs`)
- [x] Dock "reopen" shows main (macOS) (`src-tauri/src/lib.rs`)
- [x] Quit leaves agents running in holders; "Quit and Stop Agents" stops all first; state saved and CLI socket removed on exit (`src-tauri/src/menu.rs`, `src-tauri/src/lib.rs`)
- [x] Native folder picker (dialog plugin) for New agent, New terminal, onboarding Add folder; typed-path fallback (`src/lib/pickFolder.ts`)
- [x] Clipboard: copy agent folder path, copy file path, terminal copy/paste, editor copy (`src/components/Changes.tsx`, `src/components/explorer/Viewer.tsx`, `src/terminal/registry.ts`)
- [x] Opener: rules library folder revealed in the file manager (`src-tauri/src/commands/rules.rs`)
- [x] macOS privacy: Full Disk Access status (read-only, never prompts) and opening System Settings panes (`fullDiskAccess`, `filesAndFolders`) (`src-tauri/src/commands/permissions.rs`)
- [x] Info.plist privacy usage strings for Desktop, Documents, Downloads, removable and network volumes (`src-tauri/Info.plist`)
- [x] Window material: macOS NSVisualEffectView (under-window-background, always active) behind a non-drawing webview; Windows 11 Mica behind transparent windows; `PITWALL_GLASS` override (`src-tauri/src/platform/glass.rs`)
- [x] Install `pitwall` CLI: symlink in `~/.local/bin` or `/usr/local/bin` (Windows: copy + marker in per-user `Pitwall\bin`); never replaces a foreign `pitwall` (`src-tauri/src/cli_install.rs`)
- [x] Ships helper binaries `pitwall-hold`, `pitwall-cli` (and `pitwall-hook` on Windows) next to the app; AppImage copies them to the data folder (`src-tauri/tauri.conf.json`, `src-tauri/tauri.windows.conf.json`, `src-tauri/src/holder.rs`, `src-tauri/src/platform/mod.rs`)
- [x] Login-shell PATH adopted at startup (cached in `login-path`) so Dock-launched apps find tools (`src-tauri/src/platform/mod.rs`)
- [x] ~~Linux: `WEBKIT_DISABLE_DMABUF_RENDERER=1` default (webview-specific; likely drop) (decide) (`src-tauri/src/platform/mod.rs`)~~ n/a: WebKitGTK-only workaround; no webview
- [x] Bundles: macOS app (category DeveloperTool), deb (depends curl), AppImage, Windows NSIS (per-user) and MSI with WebView2 bootstrapper (`src-tauri/tauri.conf.json`, `src-tauri/tauri.windows.conf.json`)
- [x] ~~Not present today: single-instance lock, deep links / URL scheme, autostart, global shortcuts, auto-update (decide)~~ n/a: React has none of these (single-instance lock exists; updates planned in packaging.md)

## 28. State and persistence

Data folder: `~/.pitwall` (macOS), XDG data dir `pitwall` (Linux), per-user app
data `Pitwall` (Windows); `$PITWALL_HOME` overrides.

- [x] `ui.json`: UI-owned blob written by `set_ui_state` (atomic), read by `get_ui_state`; UI debounces writes 150 ms; sanitized on load (`src-tauri/src/windows.rs`, `src/state/useUiState.ts`, `src/state/workspace.ts`)
- [x] `ui.json` fields: `v` (1), `spaces[]` (`id`, `name`, `kind`, `project?`, `members[]`, `layout` tree of `{type:"pane",id,agentId}` / `{type:"split",id,dir,children,sizes}`, `focusedPaneId`, `maximizedPaneId`, `density?`) (`src/state/workspace.ts`, `src/layout/tree.ts`)
- [x] `ui.json` fields: `windowOf` (spaceId → window label), `wall` (labels in Wall mode), `fontSize`, `density`, `tileFont` (agentId → px), `collapsed`, `wallCollapsed`, `focusRequest`, `hideElsewhere`, `theme`, `look`, `reduceMotion` (`src/state/workspace.ts`)
- [x] `ui.json` pruned of removed agents (panes, members, tile fonts) (`src/App.tsx`, `src/state/workspace.ts`)
- [x] `windows.json`: backend-owned `{windows:[{label, spaceId, bounds{x,y,width,height}}]}`, saved 500 ms after the last move/resize and on quit (`src-tauri/src/windows.rs`)
- [x] `state.json` (v2): agent records (id, locator, name, kind, kindName, customCommand, cwd, project, worktree, worktreePending, baseCommit, sessionId, hasConversation, queue, autoSend, lastSent, lastSentAt, createdAt, tasks, cols, rows, adopted, inner agent); v1 backup and `.corrupt` handling (`crates/pitwall-core/src/store.rs`, `crates/pitwall-core/src/model.rs`)
- [x] `projects.json`: `onboarded`, `projects[]` (path, display, isGit, addedAt), `conversationProjects` ("Show under" choices) (`crates/pitwall-core/src/onboarding/project_list.rs`)
- [x] `rules.json`: `allowNpx`, `sets`, `projectDefaults`, `sources`, per-agent rules state; library under `rules/rules/`, git sources under `rules-sources/` (`crates/pitwall-core/src/rules/mod.rs`)
- [x] Other files: `login-path`, `run/` (CLI socket, hook socket, holder dir, rules staging), `bin/` (hook script, AppImage helpers), `agents/` (user agent definitions), `bench-cmd`/`bench-ready` (bench only) (`crates/pitwall-core/src/paths.rs`, `src-tauri/src/bench.rs`)
- [x] ~~localStorage `pitwall.theme`, `pitwall.look`, `pitwall.motion`: pre-paint mirrors of the blob (`src/lib/theme.ts`, `src/lib/look.ts`, `index.html`)~~ n/a: ui.json is read before any window opens; no pre-paint mirrors needed
- [x] localStorage `pitwall.sidebarCollapsed`, `pitwall.rightOpen` (per window convenience) (`src/App.tsx`)
- [x] localStorage `pitwall.right.files` (Changes/Files tab), `pitwall.explorer.showIgnored`, `pitwall.review.sideBySide` (`src/components/RightPanel.tsx`, `src/components/explorer/useTree.ts`, `src/components/review/Review.tsx`)
- [x] localStorage `pitwall.accessHintShown` (one-time FDA hint) (`src/lib/permissions.ts`)
- [x] ~~localStorage `pitwall.mock.ui`, `pitwall.mock.projects` (browser mock only) (`src/mock.ts`, `src/components/onboarding/mockOnboarding.ts`)~~ n/a: browser-mock-only storage
- [x] In-memory session state (lost on restart): Next up drafts, review comments, viewer tabs, search panes, quick-open recents, Review "All projects", worktree list expansions (`src/components/NextUp.tsx`, `src/components/review/comments.ts`, `src/components/explorer/*`)

## 29. Events (backend → UI)

- [x] ~~`agents-changed`: `AgentView[]` (all windows) (`src-tauri/src/events.rs`)~~ n/a: Event replaced: bridge applies AgentsChanged to AgentStore; windows observe it
- [x] ~~`attention`: `{agentId, name, reason: "blocked"|"done", detail?}`; also drives notifications (`src-tauri/src/events.rs`, `src-tauri/src/attention.rs`)~~ n/a: Event replaced: StoreEvent::Attention drives notifications and toast
- [x] ~~`scan-progress`: `{step, status: running|done|skipped|error, summary?}` (`src-tauri/src/events.rs`)~~ n/a: Event replaced: StoreEvent::ScanProgress
- [x] ~~`projects-changed`: `Project[]` (`{path, display, isGit, addedAt}`) (`src-tauri/src/events.rs`)~~ n/a: Event replaced: StoreEvent::ProjectsChanged
- [x] ~~`BlockedCount` (internal, not a webview event): drives the badge (`src-tauri/src/events.rs`)~~ n/a: Handled in-process: blocked count sets the Dock badge directly
- [x] ~~`ui-state-changed`: `{state, sourceWindow}` (`src-tauri/src/windows.rs`)~~ n/a: One shared UiStore entity for all windows; nothing to broadcast
- [x] ~~`window-closed`: `{label}` (secondary windows only) (`src-tauri/src/windows.rs`)~~ n/a: Replaced by cx.on_window_closed reclaiming spaces
- [x] ~~`approvals-changed`: `ApprovalView[]` (`src-tauri/src/server.rs`)~~ n/a: Replaced by AppEvent::Approvals into AgentStore
- [x] ~~`open-settings`: no payload, sent to one window (`src-tauri/src/menu.rs`)~~ n/a: Replaced by the OpenSettings action
- [x] `bench`: command string to main; UI emits `bench-ready` (`src-tauri/src/bench.rs`, `src/lib/bench.ts`)
- [x] ~~Streaming channels: `attach_output` raw bytes (batched), `watch_screen` `ScreenFrame`s (`src-tauri/src/commands/agents.rs`)~~ n/a: Replaced by direct attach_output/watch_screen calls
- [x] In-UI window events: `pitwall:access-error` (privacy refusal seen) (`src/lib/permissions.ts`)

## 30. Commands the UI calls (generate_handler list, `src-tauri/src/lib.rs`)

Agents (`src-tauri/src/commands/agents.rs`, wrapper `src/api.ts`):
- [x] ~~`list_kinds` → `KindView[]` (installed agent kinds, caps)~~ n/a: direct call: engine list_kinds
- [x] ~~`recent_projects` → `RecentProject[]`~~ n/a: direct call: onboarding recent_projects
- [x] ~~`list_agents` → `AgentView[]`~~ n/a: the agent store reads engine views directly
- [x] ~~`create_agent(req)` → `AgentView` (kind, project, worktree, provider/machine, options, customCommand, ruleSetId, applyToMainCheckout, displayProject, cols/rows)~~ n/a: direct call: lifecycle::create
- [x] ~~`list_machines` → `ProviderMachines[]` ("Runs on")~~ n/a: direct call: engine machine_list
- [x] ~~`create_form(provider, machine)` → `CreateForm`~~ n/a: direct call: engine create_form
- [x] ~~`attach_output(agentId, onData)` → subscription id; `detach_output(agentId, subscriptionId)`~~ n/a: direct calls: input attach_output/detach_output
- [x] ~~`watch_screen(agentId, onFrame)` → watch id; `unwatch_screen(agentId, watchId)`~~ n/a: direct calls: input watch_screen/unwatch_screen
- [x] ~~`write_input(agentId, data)`, `resize(agentId, cols, rows)`~~ n/a: direct calls: input write_input_bytes/resize
- [x] ~~`send_prompt(agentId, text)`~~ n/a: direct call: input send_prompt
- [x] ~~`queue_add`, `queue_remove`, `queue_send_now`, `set_auto_send` → `AgentView`~~ n/a: direct calls: queue_add/remove/send_now/set_auto_send
- [x] ~~`mark_seen(agentId)`~~ n/a: direct call: engine mark_seen
- [x] ~~`get_changes(agentId)`, `refresh_changes(agentId)` → `FileChange[]`; `get_file_diff(agentId, path, untracked)` → unified diff~~ n/a: direct calls: changes refresh/file_versions
- [x] ~~`stop_agent`, `restart_agent(agentId, cols?, rows?)` → `AgentView`, `remove_agent(agentId, deleteWorktree)`~~ n/a: direct calls: lifecycle stop/restart/remove
- [x] `codex_hooks_status`, `install_codex_hooks` → `{installed, path}` (done in the settings redesign)

CLI and approvals (`src-tauri/src/commands/cli.rs`):
- [x] ~~`list_approvals`, `answer_approval(id, allow, remember)`, `cli_status` → `CliStatus`, `install_cli(dir)`~~ n/a: approvals answered directly; CLI install lives in Settings

Windows (`src-tauri/src/commands/windows.rs`):
- [x] ~~`get_ui_state`, `set_ui_state(state)`, `open_window(spaceId)` → label, `focus_window(label)`, `list_windows`~~ n/a: direct calls: ui_state and the windows module

Rules (`src-tauri/src/commands/rules.rs`, wrapper `src/rules/api.ts`):
- [x] `rules_status`, `set_rules_npx(enabled)`, `list_rule_library`, `reveal_rule_library`
- [x] `list_rule_sets`, `save_rule_set(set)`, `delete_rule_set(id)`
- [x] `import_rules({kind, source})`, `list_rule_sources`, `pull_rule_source(name)`, `remove_rule_source(name)`
- [x] `set_project_rules(projectPath, ruleSetId)`, `list_project_rules`, `get_project_rules(projectPath)`
- [x] `apply_rules(agentId, confirmMainCheckout?)`, `set_agent_rules(agentId, ruleSetId)` (no UI caller today) (decide), `agent_rules`

Review (`src-tauri/src/commands/review.rs`, wrapper `src/reviewTypes.ts`):
- [x] `list_tasks`, `get_task_changes(agentId, taskId)`, `get_file_versions(agentId, path, taskId)`
- [x] `discard_file`, `commit_agent(agentId, message)` → short id, `merge_agent` → `MergeResult`, `get_merge_status` → `MergeStatus`

Explorer (`src-tauri/src/commands/explorer.rs`, wrapper `src/explorerApi.ts`):
- [x] ~~`list_files(agentId, dir?, ignored?)`, `list_all_files`, `read_file(agentId, path, large?)`, `search_files(agentId, query)`, `cancel_search`~~ n/a: direct calls through explorer/source.rs

Worktrees (`src-tauri/src/commands/worktrees.rs`, wrapper `src/worktreesApi.ts`):
- [x] `list_worktrees`, `refresh_worktrees(projectId?)`, `get_worktree_changes`, `get_worktree_file_versions`, `get_worktree_merge_status`
- [x] `commit_worktree`, `merge_worktree`, `remove_worktree` (never --force)

Onboarding, terminals, permissions, host:
- [x] `scan_environment` → `ScanResult`, `get_onboarded`, `list_projects`, `add_project`, `remove_project`, `complete_onboarding(projects, installCodexHooks)` (`src-tauri/src/commands/onboarding.rs`) (done in the settings redesign)
- [x] ~~`continue_conversation(kind, sessionId, projectPath, name, displayProject?, cols?, rows?)`, `adopt_session(req)` (`src-tauri/src/commands/onboarding.rs`)~~ n/a: direct calls: continue_conversation, lifecycle::adopt
- [x] ~~`list_elsewhere` (`src-tauri/src/commands/terminals.rs`)~~ n/a: direct call: the onboarding Elsewhere scanner
- [x] `permissions_status`, `open_privacy_settings(kind)` (`src-tauri/src/commands/permissions.rs`) (done in the settings redesign)
- [x] ~~`host_info`, `set_window_glass(on)` → `GlassState`, `quit_app(stopAgents)` (`src-tauri/src/commands/host.rs`)~~ n/a: host info, glass and quit are direct calls and actions
- [x] ~~Plugin calls from the UI: dialog `open` (folder picker), window `setTheme` (`src/lib/pickFolder.ts`, `src/lib/theme.ts`, `src-tauri/capabilities/`)~~ n/a: folder picker via cx.prompt_for_paths; theme from the window appearance

## 31. Shared types (ts-rs generated, `src/gen/`) and hand-written shapes

- [x] ~~Agents: `AgentView`, `AgentCaps`, `Status`, `Source`, `QueueItem`, `MachineView` (`src/gen/`)~~ n/a: no ts-rs: the Rust types are used directly
- [x] ~~Creation: `CreateForm`, `CreateField`, `CreateChoice`, `FieldInput`, `FieldWhen`, `NameRule`, `ProviderMachines`, `MachineEntry`, `AgentCreate`, `FormRequest` (`src/gen/`)~~ n/a: no ts-rs: the Rust types are used directly
- [x] ~~Approvals: `ApprovalView`, `ApprovalAnswer`, `Requester`, `RequesterKind`, `Risk` (`src/gen/`)~~ n/a: no ts-rs: the Rust types are used directly
- [x] ~~Screen: `ScreenFrame`, `ScreenLine`, `ScreenRun` (text, fg, bg, attrs bit flags) (`src/gen/`, `src/terminal/screenStyle.ts`)~~ n/a: no ts-rs: the Rust types are used directly
- [x] ~~Explorer: `DirListing`, `FileEntry`, `EntryKind`, `FileIndex`, `FileView`, `ContentKind`, `FileStatus`, `SearchQuery`, `SearchResult`, `SearchMatch`, `MatchRange`, `SearchEngine` (`src/gen/`)~~ n/a: no ts-rs: the Rust types are used directly
- [x] ~~Worktrees: `ProjectWorktrees`, `WorktreeView`, `WorktreeCaps`, `WorktreeVia` (`src/gen/`)~~ n/a: no ts-rs: the Rust types are used directly
- [x] ~~Places/sessions: `ScannedPlace`, `ScannedMachine`, `ScannedSession`, `SessionAdd`, `SessionAdded` (`src/gen/`)~~ n/a: no ts-rs: the Rust types are used directly
- [x] ~~Hand-written: `KindView`, `RecentProject`, `CreateAgentRequest`, `FileChange`, `HooksStatus`, `AttentionEvent`, `Project`, scan types, `RunningElsewhere`, `CliStatus`, `PermissionsStatus`, `HostInfo`, `GlassState` (`src/types.ts`); `Task`, `FileVersions`, `MergeResult`, `MergeStatus` (`src/reviewTypes.ts`); rules shapes (`src/rules/api.ts`)~~ n/a: hand-written TS mirrors aren't needed in Rust

## 32. Polling and freshness model

- [x] Freshness controller: forced refresh on open, silent polls while visible, coalesced forced refreshes, errors kept next to data with Retry, "updated N s ago" label (`src/lib/freshness.ts`, `src/components/Freshness.tsx`)
- [x] Intervals: Changes 5 s, Review files 5 s, tree 10 s, worktree list 30 s, worktree files 15 s (10 s in Review), Elsewhere 10 s, agent rules 20 s, permissions 2 s (while waiting), approval clock 1 s (`src/`)
- [x] All polls pause while the window is hidden and catch up on visibility (`src/lib/freshness.ts`)
- [x] ~~`agents-changed` reuses unchanged agent objects to avoid re-render work (`src/lib/useAgents.ts`)~~ n/a: React.memo identity trick; core only emits AgentsChanged when dirty
- [x] Commands that return an `AgentView` are patched in at once (events may lag) (`src/lib/useAgents.ts`)

## 33. Perf and bench hooks

- [x] `PITWALL_BENCH=1`: accessory app (no Dock icon), windows off-screen (or `PITWALL_BENCH_VISIBLE=1` always-on-top), commands from `bench-cmd`, cold-start time to `bench-ready` (`src-tauri/src/bench.rs`)
- [x] Bench commands: `wall on|off`, `review on|off`, `visit-all` (show each agent then Auto grid), `palette`/`settings` on|off (`src/lib/bench.ts`, `src/App.tsx`)
- [ ] Perf budgets and measurements to re-baseline for GPUI (`docs/spec/perf.md`, `scripts/bench.sh`) (partly: bench hook ported, first GPUI numbers in perf.md "GPUI port: re-baseline"; a full run on a quiet machine is still to do)

## 34. Browser mock and website demo

- [x] ~~Mock backend used outside Tauri (`pnpm dev`): full contract with sample agents, ANSI screens, review, worktrees, explorer, onboarding, permissions, rules (`src/mock.ts`, `src/mockReview.ts`, `src/mockWorktrees.ts`, `src/mockExplorer.ts`, `src/components/onboarding/mockOnboarding.ts`, `src/components/onboarding/mockPermissions.ts`, `src/rules/mock.ts`)~~ n/a: browser mock is React web build only
- [x] ~~Mock URL flags: `?approval` (pending approval), `?fda=granted`, `?onboarded`, `?shots=1` (hide MOCK chip), `?demo=1` (`src/mock.ts`, `src/components/onboarding/*`, `src/components/TopBar.tsx`)~~ n/a: URL query flags only apply to the browser mock
- [x] ~~Website demo bridge: same-origin `postMessage` `{type:"pitwall-demo", view}` drives views needs-you / wall / next-up / review / terminals / rules by clicking UI; replies `pitwall-demo-ready` (`src/lib/demoBridge.ts`, `website/`)~~ n/a: website demo stays React (README)
- [x] ~~Mock terminal frames built from an xterm buffer (`src/terminal/xtermFrames.ts`)~~ n/a: xterm-buffer mock frames are browser-mock only
- [x] ~~Decide how the website demo works after the port (the GPUI app has no browser build) (decide)~~ n/a: decided: website keeps the React mock build

## 35. Spec-only, dead or unused code (decide)

- [x] Race Engineer preset button (top bar + ⌘K) described but not built (`docs/spec/engineer.md`) — built (`crates/pitwall-app/src/engineer`, `pitwall_core::engineer`)
- [x] ~~`set_agent_rules` command has no UI caller (`src-tauri/src/commands/rules.rs`, `src/rules/api.ts`)~~ n/a: no UI caller in either app
- [x] ~~JS deps `@tauri-apps/plugin-opener` and `@tauri-apps/plugin-notification` are unused by the UI (backend uses the plugins) (`package.json`)~~ n/a: JS package.json deps
- [x] ~~Unused helpers: `radioLine`, `swapPanes`, `useDraggedAgent`, `terminalSize`, `terminalFontsReady` (`src/lib/status.ts`, `src/layout/tree.ts`, `src/lib/dnd.ts`, `src/terminal/registry.ts`)~~ n/a: dead React helpers
- [x] ~~WebGL renderer path disabled (`MAX_WEBGL = 0`) (`src/terminal/registry.ts`)~~ n/a: no WebGL path; terminals painted natively
- [x] ~~Generated types used only by the CLI protocol: `AgentCreate`, `ApprovalAnswer`, `FormRequest`, `SessionAdd`, `SessionAdded` (`src/gen/`)~~ n/a: ts-rs CLI protocol types; Rust types in pitwall-proto

## 36. In-house components

Zed's crates other than `gpui` are GPL-3.0, so every UI capability Zed would
normally provide is built in Pitwall, clean-room, on GPUI's own APIs. No
feature is dropped for licensing reasons. Basic primitives (button, icon
button, label, chip, kbd) are needed almost everywhere and are not tagged per
item. Terminal items are tagged `[pitwall-term-view]` instead.

- [x] Basic primitives: button, icon button, label, chip, kbd (all sections)
- [x] `checkbox/toggle`: checkbox and switch (§10, §12, §14, §18, §19, §20, §21, §26)
- [x] `code/diff viewer`: read-only code view and side-by-side / inline diff with gutter, folding, find, highlighting (§11, §13, §14, §15, §23, §24)
- [x] `command palette`: palette-style fuzzy picker (⌘K, ⌘P) (§14, §17)
- [x] `drag and drop`: pointer drag with ghost, hit-testing and drop previews (§3, §4, §5, §7, §9)
- [x] `drawer/overlay`: slide-in drawer / overlay sidebar with scrim (§1, §4)
- [x] `file icons`: file/folder icon lookup (Material Icon Theme subset) (§14)
- [x] `file tree`: lazy, collapsible file tree with icons and status letters (§10, §12, §14, §15, §23)
- [x] `icons`: app icon set (§24)
- [x] `keybinding label`: shortcut label rendered per desktop (⌘K vs Ctrl+Shift+K) (§2, §23)
- [x] `list/virtual list`: long, keyboard-navigable lists (virtualised where large) (§4, §9, §10, §12, §14, §15, §17, §18, §20, §21, §23)
- [x] `modal/dialog`: modal dialogs, sheets, full-window overlays, confirm (§11, §15, §18, §19, §20, §21, §22, §26)
- [x] `pane/split layout`: split tree with resizable dividers, maximise, fold to chips (§5, §23)
- [x] `popover/menu`: popup and context menus (§4, §13, §15, §23)
- [x] `radio group`: radio group / radio cards (§18, §19)
- [x] `scroll view`: scroll containers with overlay scrollbars (§4, §9, §10, §11, §12, §13, §14, §19, §20)
- [x] `segmented control`: segmented button group (§12, §14, §19)
- [x] `select/dropdown`: select / dropdown (§5, §12, §18, §20, §21)
- [x] `tabs`: tab strip (space tabs, panel tabs, viewer tabs) (§2, §3, §10, §14, §23)
- [x] `text input`: single and multi-line text input with selection, IME, clipboard (§3, §10, §13, §14, §17, §18, §20, §21, §23)
- [x] `theme`: theme tokens, dark/light, Flat/Glass tiers (§13, §19, §24)
- [x] `title bar`: custom title bar / window drag region (§2)
- [x] `toast`: toast stack with auto-dismiss (§3, §7, §14, §16, §18, §20)
- [x] `tooltip`: hover tooltips (titles today) (§2, §3, §4, §5, §10, §12, §13, §14, §15, §18, §19, §21, §22)
- [x] `pitwall-term-view`: interactive terminal view and the Wall's view-only screen view (§6, §8, §9, §23)
