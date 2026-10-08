# Detection interface (Backend ↔ Detection)


```rust
// crate pitwall-detect (crates/pitwall-detect): `use pitwall_detect::{Screen, detect, Detected, Detection};`
// src/screen.rs — headless terminal per agent (fed from the terminal reader thread).
// Pitwall's own interface: the emulator behind it is private and swappable
// (alacritty_terminal since perf pass 2, vt100 before; libghostty-vt later?).
pub struct Screen { .. }
impl Screen {
    pub fn new(rows: u16, cols: u16) -> Self;
    pub fn feed(&mut self, bytes: &[u8]);
    pub fn resize(&mut self, rows: u16, cols: u16);
    pub fn size(&self) -> (u16, u16); // (rows, cols)
    /// Visible screen as plain text, one line per row, trailing spaces trimmed.
    pub fn text(&self) -> String;
    /// Last window title set via OSC 0/2, if any (exactly as sent: not trimmed).
    pub fn title(&self) -> Option<String>;
    /// Visible screen as rows of styled runs + the cursor when shown
    /// (parser-neutral `screen::{Snapshot, Run, Style, Color, attr}`); the
    /// engine turns it into `ScreenFrame`s for the Wall (wall.md).
    pub fn snapshot(&self) -> Snapshot;
}

// src/detect.rs (rules: crates/pitwall-detect/detect/<kind>.toml)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detected { Working, Blocked, Idle }
#[derive(Debug, Clone, Default)]
pub struct Detection { pub state: Option<Detected>, pub detail: Option<String> }
/// kind_id is "claude" | "codex" | "shell" | "custom" | user-defined.
/// `None` state = no rule matched (backend falls back to activity).
pub fn detect(kind_id: &str, screen_text: &str, title: Option<&str>) -> Detection;
```

Rules run on the ticker (every 400 ms) and only when the agent printed
something since the last run (`output_seq`), so at most 2.5× per second per
agent, never per chunk.

Emulator contract (what a replacement must keep): no scrollback; DEC 2026
synchronized updates applied as they arrive; tabs and blank cells read as
spaces; wide characters once; combining marks kept. `same_text_as_vt100`
(screen.rs) checks `text()` against vt100 frame by frame on TUI-like
output, and the rule fixtures (detect/tests*.rs) must pass unchanged.

