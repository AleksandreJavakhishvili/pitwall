# Wall view


A window mode (toggle ⌘E, also in ⌘K and a top-bar button) that shows EVERY
agent's live terminal at once, grouped by project — the "pit wall" overview.
- Project sections with headers (amber ▲ when any agent inside is blocked;
  collapsible; collapsed shows a one-line summary like "2 idle").
- Inside a project: blocked → done → working → idle → others.
- Each tile: header (status glyph, name, status word / blocked detail, +/−),
  then a live terminal rendered at the PTY's CURRENT size (`cols`/`rows` from
  AgentView) and CSS-scaled to the tile width; if the tile is shorter than the
  scaled terminal, show the BOTTOM rows (crop the top), since prompts/questions
  live at the bottom. Keep a readable minimum scale; tile count per row adapts
  to window width (more on big/ultrawide screens).
- The Wall NEVER resizes PTYs and never sends input; it's view-only. Click a
  tile → leave Wall and focus that agent where it lives (or put it in the
  current space if it lives nowhere). Esc / ⌘E returns.
- Tiles are drawn without xterm.js (perf pass 2): the backend's screen copy
  of each agent (pitwall-detect `Screen`, alacritty_terminal) is sent as
  styled text — `watch_screen(agentId, onFrame: Channel<ScreenFrame>)` →
  watch id, `unwatch_screen(agentId, watchId)`. A `ScreenFrame` is
  `{cols, rows, cursor, full, lines: [row, [text, fg, bg, attrs][]][]}`: the
  whole screen first (and after a resize), then only the rows that changed,
  at most one frame per 100 ms per tile, only when something changed. The UI
  watches only tiles on (or within 200 px of) the screen. `src/terminal/
  screenView.ts` draws the rows exactly like xterm's DOM renderer (same
  spans, metrics, letter-spacing, theme palette + 256-colour cube, bold
  bright, dim, inverse, underline styles, the unfocused outline cursor —
  shown only when the agent's own terminal in this window would show it).
  The xterm instances of the panes stay parked meanwhile. A stopped agent's
  tile still shows its own terminal's last screen if this window has one.
- A window can stay in Wall mode permanently (e.g. on a second monitor); persist
  this per window in the UI state blob.

Backend addition: `AgentView` gains `cols: number` and `rows: number` (current
PTY size), and `agents-changed` fires when they change.
