# Detection interface (Backend ↔ Detection)


```rust
// crate pitwall-detect (crates/pitwall-detect): `use pitwall_detect::{Screen, detect, Detected, Detection};`
// src/screen.rs — headless terminal per agent (fed from the terminal reader thread)
pub struct Screen { .. }
impl Screen {
    pub fn new(rows: u16, cols: u16) -> Self;
    pub fn feed(&mut self, bytes: &[u8]);
    pub fn resize(&mut self, rows: u16, cols: u16);
    /// Visible screen as plain text, one line per row, trailing spaces trimmed.
    pub fn text(&self) -> String;
    /// Last window title set via OSC 0/2, if any.
    pub fn title(&self) -> Option<String>;
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

Stubs exist so the backend compiles before detection lands.

