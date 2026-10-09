//! Live terminals in panes: `pitwall-term-view` plugged into the main
//! screen's terminal slot (`main_screen::set_terminal_slot`), fed by the
//! hosted engine.
//!
//! The stream goes through the engine, as the Tauri app's
//! `attach_output` / `write_input` / `resize` commands do, rather than a
//! second connection to the agent's holder: the engine already holds the
//! holder connection (local agents and agw sessions alike), replays what it
//! buffered, and tracks input (tasks, "has a conversation"). No JSON, no
//! base64, no IPC: bytes go from the engine's reader straight into the
//! terminal's parser.
//!
//! One terminal and one view per agent, kept while the agent exists (so
//! its scrollback survives switching spaces); a new one when a stopped agent
//! runs again. The view takes keyboard focus when its pane becomes the
//! focused one; ⌘F finds, ⌘C / ⌘V copy and paste (Ctrl+Shift+C / V
//! elsewhere); ⌘+ / ⌘− / ⌘0 are the main screen's (per tile, `ui.tileFont`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use gpui::{
    div, prelude::*, AnyElement, App, Entity, Focusable, Global, KeyBinding, Subscription, Window,
};

use pitwall_core::engine::input;
use pitwall_core::Shared;
use pitwall_term_view::{
    Copy, Feed, Paste, ScrollLineDown, ScrollLineUp, ScrollPageDown, ScrollPageUp, ScrollToBottom,
    ScrollToTop, SearchNext, SearchPrevious, SelectAll, TermFont, TermSize, TermStream, TermTheme,
    Terminal, TerminalConfig, TerminalView, ToggleSearch, ViewMode, ViewSettings, KEY_CONTEXT,
};

use crate::agents::AgentStore;
use crate::main_screen::{EngineHandle, TerminalArgs};
use crate::theme::Mode;

/// An agent's output and input through the engine.
pub struct EngineStream {
    engine: Shared,
    agent: String,
    sub: Mutex<Option<u64>>,
    /// The engine is replaying buffered output (synchronously, inside
    /// `attach`): what the parser answers now (cursor reports, colour
    /// queries of a program that ran earlier) must not reach the agent as
    /// typed input.
    replaying: AtomicBool,
}

impl EngineStream {
    pub fn new(engine: Shared, agent: impl Into<String>) -> EngineStream {
        EngineStream {
            engine,
            agent: agent.into(),
            sub: Mutex::new(None),
            replaying: AtomicBool::new(false),
        }
    }
}

impl TermStream for EngineStream {
    fn attach(&self, feed: Feed) {
        let to = feed.clone();
        // The engine replays what it buffered, then streams live output.
        let sink = Box::new(move |bytes: &[u8]| {
            to.push(bytes);
            true
        });
        self.replaying.store(true, Ordering::SeqCst);
        let attached = input::attach_output(&self.engine, &self.agent, sink);
        self.replaying.store(false, Ordering::SeqCst);
        match attached {
            Ok(id) => *self.sub.lock().unwrap_or_else(|e| e.into_inner()) = Some(id),
            Err(_) => feed.close(),
        }
    }

    fn write(&self, bytes: &[u8]) {
        if self.replaying.load(Ordering::SeqCst) {
            return;
        }
        let _ = input::write_input_bytes(&self.engine, &self.agent, bytes);
    }

    fn resize(&self, size: TermSize) {
        let _ = input::resize(&self.engine, &self.agent, size.cols, size.rows);
    }
}

impl Drop for EngineStream {
    fn drop(&mut self) {
        if let Some(id) = self.sub.lock().unwrap_or_else(|e| e.into_inner()).take() {
            input::detach_output(&self.engine, &self.agent, id);
        }
    }
}

/// The pane terminal's colours (`registry.ts` themes).
pub fn term_theme(mode: Mode) -> TermTheme {
    match mode {
        Mode::Dark => TermTheme::pitwall_dark(),
        Mode::Light => TermTheme::pitwall_light(),
    }
}

struct Entry {
    terminal: Terminal,
    view: Entity<TerminalView>,
    /// Attached to a running agent (else a blank, input-less terminal).
    live: bool,
    mode: Mode,
    /// The window the view was made in (its focus and activation hooks).
    window: gpui::WindowId,
    /// xterm's `isCursorInitialized`: the cursor shows once the terminal
    /// was focused (or typed into, which needs focus) or the program
    /// switched screens. The Wall's tiles follow it.
    cursor_init: bool,
}

impl Entry {
    fn note_cursor(&mut self) -> bool {
        use pitwall_term_view::alacritty_terminal::term::TermMode;
        if !self.cursor_init && self.terminal.mode().contains(TermMode::ALT_SCREEN) {
            self.cursor_init = true;
        }
        self.cursor_init
    }
}

/// The grid an agent's terminal shows now, if this app has it (`sizeInPane`).
pub fn size_of(agent_id: &str, cx: &App) -> Option<(u16, u16)> {
    let e = cx.try_global::<Terminals>()?.entries.get(agent_id)?;
    let s = e.terminal.size();
    (s.cols >= 2 && s.rows >= 1).then_some((s.cols, s.rows))
}

/// Whether the agent's own terminal would draw its cursor (`cursorShown`
/// in registry.ts): Wall tiles draw the cursor only then.
pub fn cursor_shown(agent_id: &str, cx: &mut App) -> bool {
    if !cx.has_global::<Terminals>() {
        return false;
    }
    cx.global_mut::<Terminals>()
        .entries
        .get_mut(agent_id)
        .is_some_and(Entry::note_cursor)
}

/// A stopped agent's own terminal, view-only, for its Wall tile in
/// `window` (`mountStoppedWallView`): its last screen at the terminal's
/// size, scaled to the tile like the screen tiles. `None` when this window
/// has no terminal for it.
pub fn stopped_tile_view(
    agent_id: &str,
    font_size: f32,
    min_scale: f32,
    padding: [f32; 4],
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<TerminalView>> {
    let here = window.window_handle().window_id();
    let terminal = cx
        .try_global::<Terminals>()?
        .entries
        .get(agent_id)
        .filter(|e| e.window == here)?
        .terminal
        .clone();
    let mut s = settings(font_size);
    s.scrollbar = false;
    s.padding = padding;
    Some(cx.new(|cx| TerminalView::new(terminal, ViewMode::Tile { min_scale }, s, window, cx)))
}

/// Every agent's terminal.
#[derive(Default)]
pub struct Terminals {
    entries: HashMap<String, Entry>,
    _subs: Vec<Subscription>,
}

impl Global for Terminals {}

/// The key bindings inside a terminal (context `Terminal`). ⌘K, ⌘+/⌘−/⌘0
/// and the other app shortcuts are not bound here: they stay the app's.
pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    let mut b = vec![
        KeyBinding::new("shift-pageup", ScrollPageUp, c),
        KeyBinding::new("shift-pagedown", ScrollPageDown, c),
    ];
    if cfg!(target_os = "macos") {
        b.extend([
            KeyBinding::new("cmd-c", Copy, c),
            KeyBinding::new("cmd-v", Paste, c),
            KeyBinding::new("cmd-a", SelectAll, c),
            KeyBinding::new("cmd-f", ToggleSearch, c),
            KeyBinding::new("cmd-g", SearchNext, c),
            KeyBinding::new("cmd-shift-g", SearchPrevious, c),
            KeyBinding::new("cmd-up", ScrollLineUp, c),
            KeyBinding::new("cmd-down", ScrollLineDown, c),
            KeyBinding::new("cmd-home", ScrollToTop, c),
            KeyBinding::new("cmd-end", ScrollToBottom, c),
        ]);
    } else {
        // Ctrl+C and Ctrl+V belong to the program (lib/host.ts).
        b.extend([
            KeyBinding::new("ctrl-shift-c", Copy, c),
            KeyBinding::new("ctrl-shift-v", Paste, c),
            KeyBinding::new("ctrl-shift-f", ToggleSearch, c),
            KeyBinding::new("shift-home", ScrollToTop, c),
            KeyBinding::new("shift-end", ScrollToBottom, c),
        ]);
    }
    b
}

/// Plug the terminal view into the main screen (call once, after the
/// engine and the agent store exist).
pub fn init(store: &Entity<AgentStore>, cx: &mut App) {
    cx.bind_keys(bindings());
    // Agents removed from Pitwall drop their terminals.
    let sub = cx.observe(store, |store, cx| {
        let ids: Vec<String> = store.read(cx).agents.iter().map(|a| a.id.clone()).collect();
        if let Some(t) = cx.try_global::<Terminals>() {
            if t.entries.keys().any(|k| !ids.contains(k)) {
                cx.global_mut::<Terminals>()
                    .entries
                    .retain(|k, _| ids.contains(k));
            }
        }
    });
    cx.set_global(Terminals {
        _subs: vec![sub],
        ..Default::default()
    });
    crate::main_screen::set_terminal_slot(cx, element);
}

fn settings(font_size: f32) -> ViewSettings {
    ViewSettings {
        font: TermFont {
            size: font_size,
            ..TermFont::default()
        },
        ..ViewSettings::default()
    }
}

/// The pane body for an agent.
fn element(args: &TerminalArgs, window: &mut Window, cx: &mut App) -> AnyElement {
    let a = args.agent;
    let mode = crate::theme::theme(cx).mode;
    let font = args.font_size as f32;
    let fresh = cx
        .try_global::<Terminals>()
        .and_then(|t| t.entries.get(&a.id))
        .is_none_or(|e| a.running && !e.live);
    if fresh {
        let engine = cx.try_global::<EngineHandle>().map(|e| e.0.clone());
        let size = TermSize::new(a.cols.max(20), a.rows.max(5));
        let config = TerminalConfig {
            theme: term_theme(mode),
            ..TerminalConfig::default()
        };
        let live = a.running && engine.is_some();
        let terminal = match engine.filter(|_| live) {
            Some(engine) => Terminal::new(EngineStream::new(engine, a.id.clone()), size, config),
            None => Terminal::new(pitwall_term_view::NullStream, size, config),
        };
        let view = cx.new(|cx| {
            TerminalView::new(
                terminal.clone(),
                ViewMode::Interactive,
                settings(font),
                window,
                cx,
            )
        });
        cx.default_global::<Terminals>().entries.insert(
            a.id.clone(),
            Entry {
                terminal,
                view,
                live,
                mode,
                window: window.window_handle().window_id(),
                cursor_init: false,
            },
        );
    }
    // Its space moved to another window: a new view there on the same
    // terminal (the scrollback stays).
    let here = window.window_handle().window_id();
    let moved = cx
        .try_global::<Terminals>()
        .and_then(|t| t.entries.get(&a.id))
        .filter(|e| e.window != here)
        .map(|e| e.terminal.clone());
    if let Some(terminal) = moved {
        let view = cx.new(|cx| {
            TerminalView::new(terminal, ViewMode::Interactive, settings(font), window, cx)
        });
        if let Some(e) = cx.global_mut::<Terminals>().entries.get_mut(&a.id) {
            e.view = view;
            e.window = here;
        }
    }
    let t = cx.global_mut::<Terminals>();
    let Some(entry) = t.entries.get_mut(&a.id) else {
        return div().into_any_element();
    };
    if !a.running {
        // Stopped or exited: keep the last screen; a new run attaches anew.
        entry.live = false;
    }
    if entry.mode != mode {
        entry.mode = mode;
        entry.terminal.set_theme(term_theme(mode));
    }
    let view = entry.view.clone();
    if view.read(cx).settings().font.size != font {
        view.update(cx, |v, cx| {
            let mut s = v.settings().clone();
            s.font.size = font;
            v.set_settings(s, cx);
        });
    }
    // Focus follows the focused pane, unless something else (a dialog, the
    // sidebar, a menu, a field) has the keys.
    let handle = view.read(cx).focus_handle(cx);
    if args.focused && args.screen_focused && !handle.is_focused(window) {
        window.focus(&handle);
    }
    if handle.is_focused(window) || (args.focused && args.screen_focused) {
        if let Some(e) = cx.global_mut::<Terminals>().entries.get_mut(&a.id) {
            e.cursor_init = true;
        }
    }
    // Cached: a frame drawn for something else (a working dot's pulse)
    // replays the terminal's last paint instead of laying it out again.
    let style = gpui::StyleRefinement::default().flex_1().min_w_0().min_h_0();
    div()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .flex()
        .child(gpui::AnyView::from(view).cached(style))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_shortcuts_stay_the_apps() {
        let keys: Vec<String> = bindings()
            .iter()
            .map(|b| {
                b.keystrokes()
                    .iter()
                    .map(|k| k.unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        for owned in [
            "cmd-k", "cmd-=", "cmd--", "cmd-0", "cmd-j", "ctrl-c", "ctrl-v",
        ] {
            assert!(
                !keys.contains(&owned.to_string()),
                "{owned} must reach the app"
            );
        }
    }

    #[gpui::test]
    fn no_terminal_no_cursor(cx: &mut gpui::TestAppContext) {
        // A Wall tile of an agent this window never showed: xterm would
        // never have drawn its cursor.
        cx.update(|cx| {
            assert!(!cursor_shown("never-shown", cx));
            cx.set_global(Terminals::default());
            assert!(!cursor_shown("never-shown", cx));
        });
    }

    #[test]
    fn themes_follow_the_mode() {
        assert_eq!(term_theme(Mode::Dark), TermTheme::pitwall_dark());
        assert_eq!(term_theme(Mode::Light), TermTheme::pitwall_light());
    }
}
