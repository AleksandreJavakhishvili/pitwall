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
- In a window whose agents' xterm instances live elsewhere (other window), the
  Wall creates its own view-only xterm attached via attach_output (detach on exit).
- A window can stay in Wall mode permanently (e.g. on a second monitor); persist
  this per window in the UI state blob.

Backend addition: `AgentView` gains `cols: number` and `rows: number` (current
PTY size), and `agents-changed` fires when they change.
