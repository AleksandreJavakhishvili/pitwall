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
//! terminal's parser, and that parser is the agent's only one: the
//! engine's status rules and the Wall read this terminal's grid
//! (`input::share_screen`) instead of a headless screen of their own.
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
    Copy, Feed, FileLinks, OpenFile, Paste, ScrollLineDown, ScrollLineUp, ScrollPageDown,
    ScrollPageUp, ScrollToBottom, ScrollToTop, SearchNext, SearchPrevious, SelectAll, TermFont,
    TermSize, TermStream, TermTheme, Terminal, TerminalConfig, TerminalView, ToggleSearch,
    ViewMode, ViewSettings, KEY_CONTEXT,
};
use pitwall_proto::AgentView;

use crate::agents::AgentStore;
use crate::main_screen::{EngineHandle, TerminalArgs};
use crate::theme::Mode;

/// An agent's output and input through the engine.
pub struct EngineStream {
    engine: Shared,
    agent: String,
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
            replaying: AtomicBool::new(false),
        }
    }
}

impl TermStream for EngineStream {
    fn attach(&self, feed: Feed) {
        // The engine replays what it buffered, then pushes live output; it
        // reads the agent's screen from this terminal from now on (and
        // goes back to its own once the terminal is dropped).
        self.replaying.store(true, Ordering::SeqCst);
        let shared =
            input::share_screen(&self.engine, &self.agent, Box::new(feed.screen_source()));
        self.replaying.store(false, Ordering::SeqCst);
        if shared.is_err() {
            feed.close();
        }
    }

    fn write(&self, bytes: &[u8]) {
        if self.replaying.load(Ordering::SeqCst) {
            return;
        }
        let _ = input::write_input_bytes(&self.engine, &self.agent, bytes);
    }

    fn resize(&self, size: TermSize) {
        if size.cols >= 2 && size.rows >= 1 {
            *LAST_FITTED.lock().unwrap_or_else(|e| e.into_inner()) = Some((size.cols, size.rows));
        }
        let _ = input::resize(&self.engine, &self.agent, size.cols, size.rows);
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
    /// ⌘-click on a file reference opens the explorer (this view's).
    _open_file: Subscription,
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
/// The size a terminal was last fitted to (`lastFitted` in registry.ts).
static LAST_FITTED: Mutex<Option<(u16, u16)>> = Mutex::new(None);

/// The last fitted terminal size: a new agent's size when nothing better
/// is known.
pub fn last_fitted() -> Option<(u16, u16)> {
    *LAST_FITTED.lock().unwrap_or_else(|e| e.into_inner())
}

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
    // Terminals no view looks at (folded, in a space not shown) keep their
    // scrollback compact; busy ones do it as output comes, this is for the
    // quiet ones.
    cx.spawn(async move |cx: &mut gpui::AsyncApp| loop {
        cx.background_executor()
            .timer(pitwall_term_view::terminal::FREEZE_AFTER)
            .await;
        let alive = cx.update(|cx| {
            if let Some(t) = cx.try_global::<Terminals>() {
                t.entries.values().for_each(|e| e.terminal.freeze_if_unseen());
            }
        });
        if alive.is_err() {
            break;
        }
    })
    .detach();
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
        let open_file = file_links(&view, a, window, cx);
        cx.default_global::<Terminals>().entries.insert(
            a.id.clone(),
            Entry {
                terminal,
                view,
                live,
                mode,
                window: window.window_handle().window_id(),
                _open_file: open_file,
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
        let open_file = file_links(&view, a, window, cx);
        if let Some(e) = cx.global_mut::<Terminals>().entries.get_mut(&a.id) {
            e.view = view;
            e.window = here;
            e._open_file = open_file;
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
    // The agent moved to another folder (a worktree made later).
    let stale = match view.read(cx).file_links() {
        Some(l) => l.cwd.as_os_str() != std::ffi::OsStr::new(&a.cwd),
        None => links_for(a).is_some(),
    };
    if stale {
        let links = links_for(a);
        view.update(cx, |v, _| v.set_file_links(links));
    }
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
        // Edit ▸ Copy / Paste / Select All (the menu's actions) reach the
        // focused terminal as its own (its keys are bound in its context).
        .on_action(|_: &crate::menu::Copy, window, cx| window.dispatch_action(Box::new(Copy), cx))
        .on_action(|_: &crate::menu::Paste, window, cx| window.dispatch_action(Box::new(Paste), cx))
        .on_action(|_: &crate::menu::SelectAll, window, cx| {
            window.dispatch_action(Box::new(SelectAll), cx)
        })
        .child(gpui::AnyView::from(view).cached(style))
        .into_any_element()
}

/// Where an agent's file references resolve: its folder. Only for agents
/// on this machine (the terminal checks files on this disk).
fn links_for(a: &AgentView) -> Option<FileLinks> {
    if a.machine.provider != "local" || a.cwd.is_empty() {
        return None;
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from);
    Some(FileLinks {
        cwd: a.cwd.clone().into(),
        home,
    })
}

/// The view's file references made links: ⌘-click opens the window's
/// explorer on the file (`explorer::file_link`).
fn file_links(
    view: &Entity<TerminalView>,
    a: &AgentView,
    window: &mut Window,
    cx: &mut App,
) -> Subscription {
    let links = links_for(a);
    view.update(cx, |v, _| v.set_file_links(links));
    let agent_id = a.id.clone();
    window.subscribe(view, cx, move |_, e: &OpenFile, window, cx| {
        let link = crate::explorer::file_link::OpenFileLink {
            agent_id: agent_id.clone(),
            path: e.path.clone(),
            line: e.line,
            column: e.column,
        };
        window.dispatch_action(Box::new(link), cx);
    })
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

    /// ⌘V is bound twice (the terminal's Paste, Edit ▸ Paste of the app):
    /// a paste reaches the program once.
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn cmd_v_pastes_once(cx: &mut gpui::TestAppContext) {
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Rec(Arc<Mutex<Vec<u8>>>);
        impl TermStream for Rec {
            fn attach(&self, _feed: Feed) {}
            fn write(&self, bytes: &[u8]) {
                self.0.lock().unwrap().extend_from_slice(bytes);
            }
            fn resize(&self, _size: TermSize) {}
        }
        struct Pane(Entity<TerminalView>);
        impl Render for Pane {
            fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .on_action(|_: &crate::menu::Paste, window, cx| window.dispatch_action(Box::new(Paste), cx))
                    .child(self.0.clone())
            }
        }

        cx.update(|cx| {
            cx.bind_keys(crate::menu::bindings());
            cx.bind_keys(bindings());
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("one\ntwo".into()));
        });
        let rec = Rec::default();
        let terminal = Terminal::new(rec.clone(), TermSize::new(80, 24), TerminalConfig::default());
        let (pane, cx) = cx.add_window_view(|window, cx| {
            Pane(cx.new(|cx| TerminalView::new(terminal, ViewMode::Interactive, settings(13.), window, cx)))
        });
        cx.update(|window, cx| window.focus(&pane.read(cx).0.focus_handle(cx)));
        cx.simulate_keystrokes("cmd-v");
        assert_eq!(&*rec.0.lock().unwrap(), b"one\rtwo");
    }

    /// The pane's terminal is the agent's only parser: it gets the
    /// engine's buffered and live output, and the Wall's frames and the
    /// status rules read its grid.
    #[test]
    fn the_pane_terminal_is_the_engines_screen() {
        use pitwall_core::model::CreateAgentRequest;
        use pitwall_core::testing::Harness;
        use std::time::{Duration, Instant};
        let wait = |f: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !f() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            f()
        };
        let h = Harness::new(vec![]);
        let req = CreateAgentRequest {
            kind: "shell".into(),
            project_path: h.dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let id = pitwall_core::engine::lifecycle::create(&h.engine, req).unwrap().id;
        let ctl = h.provider.ctl(&id).unwrap();
        // The Wall's frames: from the engine's own screen first.
        let frames = std::sync::Arc::new(Mutex::new(Vec::<pitwall_proto::ScreenFrame>::new()));
        let f = frames.clone();
        let sink = Box::new(move |frame: &pitwall_proto::ScreenFrame| {
            f.lock().unwrap().push(frame.clone());
            true
        });
        input::watch_screen(&h.engine, &id, sink, Duration::from_millis(5)).unwrap();
        let has = |text: &str| {
            frames.lock().unwrap().iter().any(|fr| fr.lines.iter().any(|l| l.1.iter().any(|r| r.0.contains(text))))
        };
        ctl.output(b"earlier output\r\n");
        assert!(wait(&|| has("earlier")));
        assert!(!input::screen_shared(&h.engine, &id));

        let stream = EngineStream::new(h.engine.clone(), id.clone());
        let terminal = Terminal::new(stream, TermSize::new(40, 5), TerminalConfig::default());
        assert!(terminal.screen_text().contains("earlier output"), "replayed");
        assert!(input::screen_shared(&h.engine, &id));
        ctl.output(b"\x1b[1mlive\x1b[0m output");
        assert!(wait(&|| terminal.screen_text().contains("live output")));
        // Now from this terminal.
        assert!(wait(&|| has("live")));

        // Dropped (the agent left Pitwall's panes): the engine parses again.
        drop(terminal);
        ctl.output(b"\r\nlater");
        assert!(wait(&|| !input::screen_shared(&h.engine, &id)));
        assert!(wait(&|| has("later")), "the Wall keeps getting frames");
    }

    #[test]
    fn themes_follow_the_mode() {
        assert_eq!(term_theme(Mode::Dark), TermTheme::pitwall_dark());
        assert_eq!(term_theme(Mode::Light), TermTheme::pitwall_light());
    }
}
