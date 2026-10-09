//! `TerminalView`: a GPUI view that shows a [`Terminal`] and turns keyboard,
//! IME, mouse, scroll, focus and clipboard input into bytes for its stream.
//!
//! Two modes: [`ViewMode::Interactive`] (a pane: sizes the terminal to fit,
//! takes input) and [`ViewMode::Tile`] (the Wall: read-only, never resizes
//! the terminal, scales it to the tile width and shows the bottom rows,
//! repaints at most 10 times a second).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{Term, TermMode};
use gpui::{
    actions, div, prelude::*, px, App, Bounds, ClipboardItem, Context, CursorStyle, EventEmitter, FocusHandle,
    Focusable, InputHandler, KeyBinding, KeyDownEvent, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, Task, UTF16Selection, Window,
};

use crate::element::{GridLayout, RenderCache, RenderStats, TermElement, TermFont, SCROLLBAR_W};
use crate::keys::{self, KeyModes, KeyPress, Mods};
use crate::mouse::{self, Button, MouseAction, MouseModes, MouseMods};
use crate::paths;
use crate::terminal::{TermEvent, Terminal, UiRequest};

actions!(
    terminal,
    [
        /// Copy the selection.
        Copy,
        /// Paste the clipboard (bracketed when the program asked for it).
        Paste,
        SelectAll,
        /// Clear the scrollback (and the screen, by sending Ctrl+L).
        Clear,
        ScrollLineUp,
        ScrollLineDown,
        ScrollPageUp,
        ScrollPageDown,
        ScrollToTop,
        ScrollToBottom,
        /// Open or close the find bar.
        ToggleSearch,
        SearchNext,
        SearchPrevious,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
    ]
);

/// The key context terminal bindings use.
pub const KEY_CONTEXT: &str = "Terminal";

/// macOS-style bindings for the actions (the host app can use its own).
pub fn default_key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("cmd-c", Copy, c),
        KeyBinding::new("cmd-v", Paste, c),
        KeyBinding::new("cmd-a", SelectAll, c),
        KeyBinding::new("cmd-k", Clear, c),
        KeyBinding::new("shift-pageup", ScrollPageUp, c),
        KeyBinding::new("shift-pagedown", ScrollPageDown, c),
        KeyBinding::new("cmd-up", ScrollLineUp, c),
        KeyBinding::new("cmd-down", ScrollLineDown, c),
        KeyBinding::new("cmd-home", ScrollToTop, c),
        KeyBinding::new("cmd-end", ScrollToBottom, c),
        KeyBinding::new("cmd-f", ToggleSearch, c),
        KeyBinding::new("cmd-g", SearchNext, c),
        KeyBinding::new("cmd-shift-g", SearchPrevious, c),
        KeyBinding::new("cmd-=", IncreaseFontSize, c),
        KeyBinding::new("cmd--", DecreaseFontSize, c),
        KeyBinding::new("cmd-0", ResetFontSize, c),
        // Linux / Windows style, where Ctrl+C belongs to the terminal.
        KeyBinding::new("ctrl-shift-c", Copy, c),
        KeyBinding::new("ctrl-shift-v", Paste, c),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewMode {
    Interactive,
    /// Read-only Wall tile; `min_scale` is the smallest scale it shrinks to.
    Tile {
        min_scale: f32,
    },
}

#[derive(Clone, Debug)]
pub struct ViewSettings {
    pub font: TermFont,
    /// Option / Alt sends ESC + key (like xterm.js `macOptionIsMeta`, Pitwall's default).
    pub option_as_meta: bool,
    pub cursor_blink: bool,
    pub copy_on_select: bool,
    pub scrollbar: bool,
    /// Lines per wheel notch (line-based wheels).
    pub wheel_lines: f32,
    /// Space around the grid (top, right, bottom, left); the default is
    /// Pitwall's pane padding (`.pane-body` in space.css).
    pub padding: [f32; 4],
}

impl Default for ViewSettings {
    fn default() -> Self {
        ViewSettings {
            font: TermFont::default(),
            option_as_meta: true,
            cursor_blink: true,
            copy_on_select: false,
            scrollbar: true,
            wheel_lines: 1.0,
            padding: [6.0, 4.0, 4.0, 10.0],
        }
    }
}

const BLINK: Duration = Duration::from_millis(600);
/// Stop blinking after this long without input (xterm.js does the same).
const BLINK_IDLE: Duration = Duration::from_secs(300);
/// Wall tiles repaint at most this often.
const TILE_FRAME: Duration = Duration::from_millis(100);

/// Repaints every Wall tile whose terminal changed, all in one update every
/// [`TILE_FRAME`], so N busy tiles cost one frame per tick (not N), and idle
/// tiles cost nothing. One clock per app.
#[derive(Default)]
struct TileClock {
    tiles: Vec<(gpui::WeakEntity<TerminalView>, Terminal, u64)>,
    running: bool,
    task: Option<Task<()>>,
}

impl gpui::Global for TileClock {}

impl TileClock {
    fn register(view: gpui::WeakEntity<TerminalView>, terminal: Terminal, cx: &mut App) {
        let clock = cx.default_global::<TileClock>();
        clock.tiles.push((view, terminal, u64::MAX));
        if clock.running {
            return;
        }
        clock.running = true;
        let task = cx.spawn(async move |cx| loop {
            cx.background_executor().timer(TILE_FRAME).await;
            let alive = cx
                .update(|cx| {
                    let mut changed = Vec::new();
                    let clock = cx.global_mut::<TileClock>();
                    clock.tiles.retain_mut(|(view, terminal, seen)| {
                        if view.upgrade().is_none() {
                            return false;
                        }
                        // Apply a synchronized update that outlived its deadline.
                        if terminal.sync_deadline().is_some_and(|d| d <= Instant::now()) {
                            drop(terminal.lock());
                        }
                        let epoch = terminal.epoch();
                        if epoch != *seen {
                            *seen = epoch;
                            changed.push(view.clone());
                        }
                        true
                    });
                    let empty = clock.tiles.is_empty();
                    if empty {
                        clock.running = false; // the next tile starts a new clock
                    }
                    for view in changed {
                        let _ = view.update(cx, |_, cx| cx.notify());
                    }
                    !empty
                })
                .unwrap_or(false);
            if !alive {
                break;
            }
        });
        cx.global_mut::<TileClock>().task = Some(task);
    }
}

/// Where file references in the text resolve: relative paths under `cwd`,
/// `~/…` under `home`. Without it, only URLs are links.
#[derive(Clone, Debug, PartialEq)]
pub struct FileLinks {
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
}

/// ⌘-click (Ctrl-click off macOS) on a file reference: the file (absolute,
/// links resolved; it existed when hovered) and where in it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenFile {
    pub path: PathBuf,
    /// 1-based.
    pub line: Option<u32>,
    /// 1-based.
    pub column: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
enum Link {
    Url(String),
    File(OpenFile),
}

/// A line's file references once checked on disk; read again after this.
const FILE_CHECK_TTL: Duration = Duration::from_secs(5);
/// Lines kept checked (then the cache starts over).
const FILE_CACHE_LINES: usize = 256;

type CheckedLine = Vec<(Range<usize>, Option<OpenFile>)>;

/// File references found per line of text (keyed by the line's text), and
/// the lines being checked now.
#[derive(Default)]
struct FileCache {
    lines: HashMap<String, (Instant, CheckedLine)>,
    checking: HashSet<String>,
}

/// What a press of the left button is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Select,
    Scrollbar {
        grab: f32,
    },
    /// Reported to the program (mouse mode).
    Report(Button),
}

struct SearchState {
    query: String,
    regex: Option<Rc<RefCell<RegexSearch>>>,
    current: Option<Match>,
    error: bool,
}

pub struct TerminalView {
    terminal: Terminal,
    focus: FocusHandle,
    mode: ViewMode,
    settings: ViewSettings,
    font_size: f32,
    cache: Rc<RefCell<RenderCache>>,
    layout: Rc<RefCell<GridLayout>>,
    drag: Drag,
    scroll_px: f32,
    last_report_cell: Option<(usize, usize)>,
    preedit: Option<String>,
    blink_on: bool,
    /// Has keyboard focus (in the active window): the cursor blinks.
    focused: bool,
    last_input: Instant,
    blink_task: Option<Task<()>>,
    link: Option<(Match, Link)>,
    file_links: Option<FileLinks>,
    files: FileCache,
    /// Where the pointer was when links were last looked for.
    hover_pos: Option<Point<Pixels>>,
    /// A ⌘-click on a file reference still being checked: open it then.
    click_pending: Option<Point<Pixels>>,
    cmd_held: bool,
    hovered: bool,
    search: Option<SearchState>,
    title: Option<String>,
    exited: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<TermEvent> for TerminalView {}
impl EventEmitter<OpenFile> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    pub fn new(
        terminal: Terminal,
        mode: ViewMode,
        settings: ViewSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let mut subs = Vec::new();
        if mode == ViewMode::Interactive {
            subs.push(cx.on_focus_in(&focus, window, |this, window, cx| this.focus_changed(true, window, cx)));
            subs.push(cx.on_focus_out(&focus, window, |this, _, window, cx| this.focus_changed(false, window, cx)));
            subs.push(cx.observe_window_activation(window, |this, window, cx| {
                if this.focus.contains_focused(window, cx) {
                    let active = window.is_window_active();
                    this.focus_changed(active, window, cx);
                }
            }));
        }
        let mut tasks = Vec::new();
        if let ViewMode::Tile { .. } = mode {
            TileClock::register(cx.weak_entity(), terminal.clone(), cx);
        } else {
            let mut changes = terminal.changes();
            let watched = terminal.clone();
            tasks.push(cx.spawn_in(window, async move |this, cx| {
                while changes.next().await {
                    let ok = this
                        .update_in(cx, |this, window, cx| {
                            this.handle_requests(cx);
                            this.blink_on = true;
                            // Echo of typing at once; other output on the
                            // window's shared frame grid (frames.rs).
                            if this.last_input.elapsed() < crate::frames::ECHO_WINDOW {
                                cx.notify();
                            } else {
                                crate::frames::request(cx.entity_id(), window);
                            }
                        })
                        .is_ok();
                    if !ok {
                        break;
                    }
                    // A synchronized update that never ends is applied at its deadline.
                    if let Some(deadline) = watched.sync_deadline() {
                        let wait = deadline.saturating_duration_since(Instant::now());
                        cx.background_executor().timer(wait + Duration::from_millis(1)).await;
                        watched.touch();
                    }
                }
            }));
        }
        let font_size = settings.font.size;
        let mut view = TerminalView {
            terminal,
            focus,
            mode,
            settings,
            font_size,
            cache: Rc::default(),
            layout: Rc::default(),
            drag: Drag::None,
            scroll_px: 0.0,
            last_report_cell: None,
            preedit: None,
            blink_on: true,
            focused: false,
            last_input: Instant::now(),
            blink_task: None,
            link: None,
            file_links: None,
            files: FileCache::default(),
            hover_pos: None,
            click_pending: None,
            cmd_held: false,
            hovered: false,
            search: None,
            title: None,
            exited: false,
            _tasks: tasks,
            _subscriptions: subs,
        };
        view.restart_blink(cx);
        view
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn has_exited(&self) -> bool {
        self.exited
    }

    /// Window-space bounds of the visible cell at (`col`, `row`) as of the
    /// last frame (for popups, tests and tools).
    pub fn cell_bounds(&self, col: usize, row: usize) -> Bounds<Pixels> {
        let l = *self.layout.borrow();
        Bounds::new(
            gpui::point(l.origin.x + px(col as f32 * l.cell_w), l.origin.y + px(row as f32 * l.cell_h)),
            gpui::size(px(l.cell_w), px(l.cell_h)),
        )
    }

    /// Text being composed by the input method, if any.
    pub fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }

    pub fn render_stats(&self) -> RenderStats {
        self.cache.borrow().stats
    }

    pub fn settings(&self) -> &ViewSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: ViewSettings, cx: &mut Context<Self>) {
        self.font_size = settings.font.size;
        self.settings = settings;
        cx.notify();
    }

    /// Make file references links, resolved as `links` says (`None`: only
    /// URLs).
    pub fn set_file_links(&mut self, links: Option<FileLinks>) {
        if self.file_links != links {
            self.file_links = links;
            self.files = FileCache::default();
        }
    }

    pub fn file_links(&self) -> Option<&FileLinks> {
        self.file_links.as_ref()
    }

    pub(crate) fn focus_ref(&self) -> &FocusHandle {
        &self.focus
    }

    fn interactive(&self) -> bool {
        self.mode == ViewMode::Interactive
    }

    // ── events from the terminal ────────────────────────────────────────────

    fn handle_requests(&mut self, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        for r in self.terminal.take_requests() {
            match r {
                UiRequest::Event(TermEvent::ClipboardStore(text)) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    cx.emit(TermEvent::ClipboardStore(text));
                }
                UiRequest::Event(ev) => {
                    match &ev {
                        TermEvent::Title(t) => self.title = t.clone(),
                        TermEvent::Exited => self.exited = true,
                        _ => {}
                    }
                    cx.emit(ev);
                }
                UiRequest::ClipboardLoad(format) => {
                    let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                    self.terminal.write(format(&text).as_bytes());
                }
            }
        }
    }

    fn focus_changed(&mut self, focused: bool, _window: &mut Window, cx: &mut Context<Self>) {
        self.focused = focused;
        let mode = self.terminal.mode();
        {
            let mut st = self.terminal.lock();
            st.term.is_focused = focused;
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            self.terminal.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
        self.blink_on = true;
        self.restart_blink(cx);
        cx.notify();
    }

    // ── cursor blink ────────────────────────────────────────────────────────

    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.blink_on = true;
        // Only a focused terminal blinks (an unfocused one draws a still,
        // hollow cursor), so only it needs the clock.
        if !self.interactive() || !self.settings.cursor_blink || !self.focused {
            self.blink_task = None;
            return;
        }
        self.blink_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(BLINK).await;
            let go_on = this
                .update(cx, |this, cx| {
                    if this.last_input.elapsed() > BLINK_IDLE || !this.focused {
                        this.blink_on = true;
                        cx.notify();
                        return false;
                    }
                    this.blink_on = !this.blink_on;
                    // A hidden cursor (TUIs hide it while they draw) has
                    // nothing to blink: no frame for it.
                    if this.terminal.mode().contains(TermMode::SHOW_CURSOR) {
                        cx.notify();
                    }
                    true
                })
                .unwrap_or(false);
            if !go_on {
                break;
            }
        }));
    }

    fn input_happened(&mut self, cx: &mut Context<Self>) {
        self.last_input = Instant::now();
        if self.blink_task.is_none() || !self.blink_on {
            self.restart_blink(cx);
        }
        self.blink_on = true;
    }

    /// Send bytes typed by the user: back to the live screen, cursor shown.
    fn send_input(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if self.exited {
            return;
        }
        self.scroll_to_bottom_internal();
        self.terminal.write(bytes);
        self.input_happened(cx);
        cx.notify();
    }

    fn scroll_to_bottom_internal(&mut self) {
        let mut st = self.terminal.lock();
        if st.term.grid().display_offset() != 0 {
            st.term.scroll_display(Scroll::Bottom);
        }
    }

    // ── keyboard ────────────────────────────────────────────────────────────

    fn on_key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        let ks = &ev.keystroke;
        if self.search.is_some() && self.search_key(ks.key.as_str(), ks.modifiers.shift, ks.modifiers.platform, cx) {
            cx.stop_propagation();
            return;
        }
        if self.preedit.is_some() {
            return; // composing: the input method has the key
        }
        let mode = self.terminal.mode();
        let press = KeyPress {
            key: ks.key.as_str(),
            text: ks.key_char.as_deref(),
            mods: Mods {
                shift: ks.modifiers.shift,
                alt: ks.modifiers.alt,
                ctrl: ks.modifiers.control,
                cmd: ks.modifiers.platform,
            },
        };
        let modes =
            KeyModes { app_cursor: mode.contains(TermMode::APP_CURSOR), option_as_meta: self.settings.option_as_meta };
        if let Some(bytes) = keys::encode_key(press, modes) {
            {
                let mut st = self.terminal.lock();
                st.term.selection = None;
            }
            self.send_input(&bytes, cx);
            cx.stop_propagation();
        }
    }

    fn on_modifiers_changed(&mut self, ev: &ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        let held = ev.modifiers.secondary();
        if held != self.cmd_held {
            self.cmd_held = held;
            if !held && self.link.take().is_some() {
                cx.notify();
            } else if held {
                let pos = window.mouse_position();
                self.update_link(pos, cx);
            }
        }
    }

    // ── IME / text input ────────────────────────────────────────────────────

    fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.preedit = None;
        if text.is_empty() {
            return;
        }
        if let Some(s) = self.search.as_mut() {
            s.query.push_str(text);
            self.search_changed(cx);
            return;
        }
        {
            let mut st = self.terminal.lock();
            st.term.selection = None;
        }
        self.send_input(text.as_bytes(), cx);
    }

    // ── clipboard ───────────────────────────────────────────────────────────

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.terminal.lock().term.selection_to_string();
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        if let Some(s) = self.search.as_mut() {
            s.query.push_str(text.lines().next().unwrap_or(""));
            self.search_changed(cx);
            return;
        }
        let bracketed = self.terminal.mode().contains(TermMode::BRACKETED_PASTE);
        self.send_input(&keys::paste_bytes(&text, bracketed), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        let mut st = self.terminal.lock();
        let term = &mut st.term;
        let start = GridPoint::new(term.topmost_line(), Column(0));
        let end = GridPoint::new(term.bottommost_line(), term.last_column());
        let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
        sel.update(end, Side::Right);
        term.selection = Some(sel);
        drop(st);
        cx.notify();
    }

    fn clear(&mut self, _: &Clear, _window: &mut Window, cx: &mut Context<Self>) {
        {
            let mut st = self.terminal.lock();
            st.term.grid_mut().clear_history();
            st.term.selection = None;
        }
        // Let the program redraw its screen (shells clear on Ctrl+L).
        self.send_input(b"\x0c", cx);
        self.terminal.touch();
    }

    // ── scrolling ───────────────────────────────────────────────────────────

    fn scroll_by(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.terminal.scroll(scroll);
        self.link = None;
        cx.notify();
    }

    fn scroll_line_up(&mut self, _: &ScrollLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(Scroll::Delta(1), cx);
    }
    fn scroll_line_down(&mut self, _: &ScrollLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(Scroll::Delta(-1), cx);
    }
    fn scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(Scroll::PageUp, cx);
    }
    fn scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(Scroll::PageDown, cx);
    }
    fn scroll_to_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(Scroll::Top, cx);
    }
    fn scroll_to_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(Scroll::Bottom, cx);
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        let layout = *self.layout.borrow();
        let lines = match ev.delta {
            ScrollDelta::Pixels(p) => {
                self.scroll_px += f32::from(p.y);
                let n = (self.scroll_px / layout.cell_h.max(1.0)).trunc();
                self.scroll_px -= n * layout.cell_h;
                n as i32
            }
            ScrollDelta::Lines(l) => (l.y * self.settings.wheel_lines).round() as i32,
        };
        if lines == 0 {
            return;
        }
        let mode = self.terminal.mode();
        let mods = ev.modifiers;
        if mouse_modes(mode).any() && !mods.shift {
            let (col, row, _) = layout.cell_at(ev.position);
            let action = if lines > 0 { MouseAction::WheelUp } else { MouseAction::WheelDown };
            let m = MouseMods { shift: false, alt: mods.alt, ctrl: mods.control };
            let mut out = Vec::new();
            for _ in 0..lines.unsigned_abs().min(10) {
                if let Some(b) = mouse::encode_mouse(action, col, row, m, mouse_modes(mode)) {
                    out.extend(b);
                }
            }
            self.terminal.write(&out);
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !mods.shift {
            // Pagers and editors on the alternate screen get arrow keys.
            let key = if lines > 0 { "up" } else { "down" };
            let modes = KeyModes { app_cursor: mode.contains(TermMode::APP_CURSOR), option_as_meta: false };
            let one = keys::encode_key(KeyPress { key, text: None, mods: Mods::default() }, modes).unwrap_or_default();
            self.terminal.write(&one.repeat(lines.unsigned_abs().min(10) as usize));
        } else {
            self.scroll_by(Scroll::Delta(lines), cx);
        }
        cx.stop_propagation();
    }

    // ── mouse ───────────────────────────────────────────────────────────────

    fn on_mouse_down(&mut self, button: MouseButton, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        window.focus(&self.focus);
        let layout = *self.layout.borrow();
        // Scrollbar.
        if button == MouseButton::Left && self.settings.scrollbar {
            if let Some((track, thumb)) = layout.scrollbar() {
                if track.contains(&ev.position) {
                    let grab = if thumb.contains(&ev.position) {
                        f32::from(ev.position.y - thumb.top())
                    } else {
                        f32::from(thumb.size.height) / 2.0
                    };
                    self.drag = Drag::Scrollbar { grab };
                    self.drag_scrollbar(ev.position, cx);
                    return;
                }
            }
        }
        // ⌘-click (Ctrl-click off macOS) opens a link.
        if button == MouseButton::Left && ev.modifiers.secondary() {
            let checking = self.update_link(ev.position, cx);
            match self.link.as_ref().map(|(_, l)| l.clone()) {
                Some(Link::Url(uri)) => return cx.open_url(&uri),
                Some(Link::File(f)) => return cx.emit(f),
                None if checking => {
                    self.click_pending = Some(ev.position);
                    return;
                }
                None => {}
            }
        }
        let mode = self.terminal.mode();
        let (col, row, right_half) = layout.cell_at(ev.position);
        let btn = match button {
            MouseButton::Left => Some(Button::Left),
            MouseButton::Middle => Some(Button::Middle),
            MouseButton::Right => Some(Button::Right),
            _ => None,
        };
        // Programs that track the mouse get it, unless Shift is held (xterm).
        if mouse_modes(mode).any() && !ev.modifiers.shift {
            if let Some(b) = btn {
                let m = MouseMods { shift: false, alt: ev.modifiers.alt, ctrl: ev.modifiers.control };
                if let Some(bytes) = mouse::encode_mouse(MouseAction::Press(b), col, row, m, mouse_modes(mode)) {
                    self.terminal.write(&bytes);
                }
                self.drag = Drag::Report(b);
                self.last_report_cell = Some((col, row));
            }
            return;
        }
        if button != MouseButton::Left {
            return;
        }
        let ty = match ev.click_count {
            2 => SelectionType::Semantic,
            n if n >= 3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        let point = layout.grid_point(col, row);
        let side = if right_half { Side::Right } else { Side::Left };
        {
            let mut st = self.terminal.lock();
            if ev.modifiers.shift && ty == SelectionType::Simple {
                if let Some(sel) = st.term.selection.as_mut() {
                    sel.update(point, side);
                } else {
                    st.term.selection = Some(Selection::new(ty, point, side));
                }
            } else {
                st.term.selection = Some(Selection::new(ty, point, side));
            }
        }
        self.drag = Drag::Select;
        cx.notify();
    }

    fn drag_scrollbar(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Drag::Scrollbar { grab } = self.drag else { return };
        let layout = *self.layout.borrow();
        let Some((track, thumb)) = layout.scrollbar() else { return };
        let room = f32::from(track.size.height - thumb.size.height).max(1.0);
        let y = (f32::from(pos.y - track.top()) - grab).clamp(0.0, room);
        let target = ((1.0 - y / room) * layout.history as f32).round() as i32;
        let delta = target - layout.display_offset as i32;
        if delta != 0 {
            self.scroll_by(Scroll::Delta(delta), cx);
        }
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        let layout = *self.layout.borrow();
        let hovered = layout.bounds.contains(&ev.position);
        if hovered != self.hovered {
            self.hovered = hovered;
            cx.notify();
        }
        match self.drag {
            Drag::Scrollbar { .. } => return self.drag_scrollbar(ev.position, cx),
            Drag::Select if ev.pressed_button == Some(MouseButton::Left) => {
                let (col, row, right_half) = layout.cell_at(ev.position);
                // Dragging past the top or bottom scrolls.
                let y = f32::from(ev.position.y - layout.origin.y);
                if y < 0.0 {
                    self.terminal.scroll(Scroll::Delta(1));
                } else if y > layout.rows as f32 * layout.cell_h {
                    self.terminal.scroll(Scroll::Delta(-1));
                }
                let layout = *self.layout.borrow();
                let point = layout.grid_point(col, row);
                let mut st = self.terminal.lock();
                if let Some(sel) = st.term.selection.as_mut() {
                    sel.update(point, if right_half { Side::Right } else { Side::Left });
                }
                drop(st);
                cx.notify();
                return;
            }
            _ => {}
        }
        let mode = self.terminal.mode();
        let mm = mouse_modes(mode);
        if mm.any() && !ev.modifiers.shift && hovered {
            let (col, row, _) = layout.cell_at(ev.position);
            if self.last_report_cell != Some((col, row)) {
                self.last_report_cell = Some((col, row));
                let held = match self.drag {
                    Drag::Report(b) => Some(b),
                    _ => None,
                };
                let m = MouseMods { shift: false, alt: ev.modifiers.alt, ctrl: ev.modifiers.control };
                if let Some(bytes) = mouse::encode_mouse(MouseAction::Motion(held), col, row, m, mm) {
                    self.terminal.write(&bytes);
                }
            }
        }
        self.cmd_held = ev.modifiers.secondary();
        if self.cmd_held {
            self.update_link(ev.position, cx);
        } else if self.link.take().is_some() {
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, button: MouseButton, ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::Report(b) => {
                let layout = *self.layout.borrow();
                let (col, row, _) = layout.cell_at(ev.position);
                let mode = self.terminal.mode();
                let m = MouseMods { shift: false, alt: ev.modifiers.alt, ctrl: ev.modifiers.control };
                if let Some(bytes) = mouse::encode_mouse(MouseAction::Release(b), col, row, m, mouse_modes(mode)) {
                    self.terminal.write(&bytes);
                }
            }
            Drag::Select if button == MouseButton::Left => {
                let text = {
                    let mut st = self.terminal.lock();
                    if st.term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                        st.term.selection = None;
                    }
                    st.term.selection_to_string()
                };
                if self.settings.copy_on_select {
                    if let Some(text) = text.filter(|t| !t.is_empty()) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                cx.notify();
            }
            _ => {}
        }
    }

    // ── links ───────────────────────────────────────────────────────────────

    /// Look for a link under `pos` (on hover with ⌘ held and on ⌘-click
    /// only). `true`: a file reference there is still being checked.
    fn update_link(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        self.hover_pos = Some(pos);
        let layout = *self.layout.borrow();
        let mut checking = false;
        let new = if layout.bounds.contains(&pos) && layout.cols > 0 {
            let (col, row, _) = layout.cell_at(pos);
            let point = layout.grid_point(col, row);
            let terminal = self.terminal.clone();
            let st = terminal.lock();
            match link_at(st.term(), point) {
                Some((m, uri)) => Some((m, Link::Url(uri))),
                None => match self.file_link_at(st.term(), point, cx) {
                    FileLookup::Found(m, f) => Some((m, Link::File(f))),
                    FileLookup::Checking => {
                        checking = true;
                        None
                    }
                    FileLookup::None => None,
                },
            }
        } else {
            None
        };
        if new != self.link {
            self.link = new;
            cx.notify();
        }
        checking
    }

    /// The file reference under `point`, from the cache; a line not checked
    /// yet is checked on the background executor (the hover then looks
    /// again).
    fn file_link_at<T>(&mut self, term: &Term<T>, point: GridPoint, cx: &mut Context<Self>) -> FileLookup {
        let Some(cfg) = self.file_links.clone() else { return FileLookup::None };
        let (chars, points) = line_cells(term, point);
        let Some(idx) = points.iter().rposition(|p| *p <= point) else { return FileLookup::None };
        let refs = paths::find_paths(&chars);
        if !refs.iter().any(|r| r.range.contains(&idx)) {
            return FileLookup::None;
        }
        let text: String = chars.iter().collect();
        if let Some((at, found)) = self.files.lines.get(&text) {
            if at.elapsed() < FILE_CHECK_TTL {
                return found
                    .iter()
                    .find(|(r, _)| r.contains(&idx))
                    .and_then(|(r, f)| {
                        let f = f.clone()?;
                        let mut end = points[r.end - 1];
                        if term.grid()[end].flags.contains(Flags::WIDE_CHAR) {
                            end.column += 1;
                        }
                        Some(FileLookup::Found(points[r.start]..=end, f))
                    })
                    .unwrap_or(FileLookup::None);
            }
        }
        if self.files.checking.insert(text.clone()) {
            let check = cx.background_executor().spawn(async move {
                refs.into_iter()
                    .map(|r| {
                        let file = paths::resolve(&r.path, &cfg.cwd, cfg.home.as_deref())
                            .and_then(|p| paths::existing_file(&p))
                            .map(|path| OpenFile { path, line: r.line, column: r.column });
                        (r.range, file)
                    })
                    .collect::<CheckedLine>()
            });
            cx.spawn(async move |this, cx| {
                let found = check.await;
                let _ = this.update(cx, |this, cx| this.line_checked(text, found, cx));
            })
            .detach();
        }
        FileLookup::Checking
    }

    fn line_checked(&mut self, text: String, found: CheckedLine, cx: &mut Context<Self>) {
        self.files.checking.remove(&text);
        if self.files.lines.len() >= FILE_CACHE_LINES {
            self.files.lines.clear();
        }
        self.files.lines.insert(text, (Instant::now(), found));
        if let Some(pos) = self.click_pending.take() {
            self.update_link(pos, cx);
            if let Some((_, Link::File(f))) = &self.link {
                cx.emit(f.clone());
            }
        } else if self.cmd_held {
            if let Some(pos) = self.hover_pos {
                self.update_link(pos, cx);
            }
        }
    }

    pub(crate) fn mouse_cursor_style(&self) -> CursorStyle {
        if self.link.is_some() {
            CursorStyle::PointingHand
        } else if matches!(self.drag, Drag::Scrollbar { .. }) {
            CursorStyle::Arrow
        } else {
            CursorStyle::IBeam
        }
    }

    // ── search ──────────────────────────────────────────────────────────────

    fn toggle_search(&mut self, _: &ToggleSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.take().is_none() {
            self.search = Some(SearchState { query: String::new(), regex: None, current: None, error: false });
            window.focus(&self.focus);
        }
        cx.notify();
    }

    /// Keys while the find bar is open. `true` = handled.
    fn search_key(&mut self, key: &str, shift: bool, cmd: bool, cx: &mut Context<Self>) -> bool {
        if cmd {
            return false;
        }
        match key {
            "escape" => {
                self.search = None;
                cx.notify();
            }
            "enter" => self.search_step(if shift { Direction::Right } else { Direction::Left }, cx),
            "backspace" => {
                if let Some(s) = self.search.as_mut() {
                    s.query.pop();
                }
                self.search_changed(cx);
            }
            _ => return false,
        }
        true
    }

    fn search_changed(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.search.as_mut() else { return };
        s.current = None;
        s.error = false;
        s.regex = if s.query.is_empty() {
            None
        } else {
            match RegexSearch::new(&regex_escape(&s.query)) {
                Ok(r) => Some(Rc::new(RefCell::new(r))),
                Err(_) => {
                    s.error = true;
                    None
                }
            }
        };
        self.search_step(Direction::Left, cx);
    }

    fn search_next(&mut self, _: &SearchNext, _: &mut Window, cx: &mut Context<Self>) {
        self.search_step(Direction::Left, cx);
    }

    fn search_previous(&mut self, _: &SearchPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.search_step(Direction::Right, cx);
    }

    /// Find the next match upwards (`Left`, older output) or downwards.
    fn search_step(&mut self, dir: Direction, cx: &mut Context<Self>) {
        let Some(s) = self.search.as_mut() else { return };
        let Some(regex) = s.regex.clone() else {
            cx.notify();
            return;
        };
        let mut st = self.terminal.lock();
        let term = &mut st.term;
        let origin = match (&s.current, dir) {
            (Some(m), Direction::Left) => m.start().sub(term, Boundary::None, 1),
            (Some(m), Direction::Right) => m.end().add(term, Boundary::None, 1),
            (None, _) => GridPoint::new(term.bottommost_line(), term.last_column()),
        };
        let found = term.search_next(&mut regex.borrow_mut(), origin, dir, Side::Left, None);
        if let Some(m) = &found {
            term.scroll_to_point(*m.start());
        }
        s.current = found.or(s.current.clone());
        drop(st);
        self.terminal.touch();
        cx.notify();
    }

    fn change_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.font_size = if delta == 0.0 { self.settings.font.size } else { (self.font_size + delta).clamp(6.0, 48.0) };
        cx.notify();
    }
}

/// Mouse reporting modes from the terminal's mode flags.
fn mouse_modes(mode: TermMode) -> MouseModes {
    MouseModes {
        click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
        drag: mode.contains(TermMode::MOUSE_DRAG),
        motion: mode.contains(TermMode::MOUSE_MOTION),
        sgr: mode.contains(TermMode::SGR_MOUSE),
        utf8: mode.contains(TermMode::UTF8_MOUSE),
    }
}

/// The search box takes literal text.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// URLs the terminal makes clickable (⌘-click).
const URL_PATTERN: &str =
    r#"(https?://|file://|ftp://|mailto:)[^\s<>"'`{}|\\^\x00-\x1f]*[^\s<>"'`{}|\\^\x00-\x1f.,;:!?)\]]"#;

/// The link under `point`: an OSC 8 hyperlink, else a URL in the text.
pub(crate) fn link_at<T>(term: &Term<T>, point: GridPoint) -> Option<(Match, String)> {
    let grid = term.grid();
    if point.line < term.topmost_line() || point.line > term.bottommost_line() {
        return None;
    }
    if let Some(h) = grid[point].hyperlink() {
        // Extend over neighbouring cells with the same link.
        let same = |p: GridPoint| grid[p].hyperlink().is_some_and(|o| o == h);
        let mut start = point;
        while start.column.0 > 0 && same(GridPoint::new(start.line, start.column - 1)) {
            start.column -= 1;
        }
        let mut end = point;
        while end.column < term.last_column() && same(GridPoint::new(end.line, end.column + 1)) {
            end.column += 1;
        }
        return Some((start..=end, h.uri().to_string()));
    }
    let mut regex = RegexSearch::new(URL_PATTERN).ok()?;
    let start = term.line_search_left(point);
    let end = term.line_search_right(point);
    let m = RegexIter::new(start, end, Direction::Right, term, &mut regex).find(|m| m.contains(&point))?;
    let text = term.bounds_to_string(*m.start(), *m.end());
    Some((m, text.replace('\n', "")))
}

enum FileLookup {
    Found(Match, OpenFile),
    Checking,
    None,
}

/// Longest line (in cells) searched for file references.
const MAX_LINE_CELLS: usize = 8192;

/// The text of the (wrapped) line through `point`, one `char` per entry,
/// with the cell each comes from (wide-char spacers skipped).
pub(crate) fn line_cells<T>(term: &Term<T>, point: GridPoint) -> (Vec<char>, Vec<GridPoint>) {
    let (mut chars, mut points) = (Vec::new(), Vec::new());
    if point.line < term.topmost_line() || point.line > term.bottommost_line() {
        return (chars, points);
    }
    let grid = term.grid();
    let (start, end) = (term.line_search_left(point), term.line_search_right(point));
    let last = term.last_column();
    let mut p = start;
    for _ in 0..MAX_LINE_CELLS {
        let cell = &grid[p];
        if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            chars.push(cell.c);
            points.push(p);
            for &z in cell.zerowidth().unwrap_or(&[]) {
                chars.push(z);
                points.push(p);
            }
        }
        if p >= end {
            break;
        }
        p = if p.column < last { GridPoint::new(p.line, p.column + 1) } else { GridPoint::new(p.line + 1, Column(0)) };
    }
    (chars, points)
}

/// The view's text input handler (keyboard text, IME composition).
pub(crate) struct TermInputHandler {
    pub view: gpui::Entity<TerminalView>,
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl InputHandler for TermInputHandler {
    fn selected_text_range(&mut self, _ignore_disabled: bool, _: &mut Window, cx: &mut App) -> Option<UTF16Selection> {
        let n = self.view.read(cx).preedit.as_deref().map(utf16_len).unwrap_or(0);
        Some(UTF16Selection { range: n..n, reversed: false })
    }

    fn marked_text_range(&mut self, _: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.view.read(cx).preedit.as_deref().map(|p| 0..utf16_len(p))
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        let p = self.view.read(cx).preedit.clone().unwrap_or_default();
        let units: Vec<u16> = p.encode_utf16().collect();
        let r = range.start.min(units.len())..range.end.min(units.len());
        *adjusted = Some(r.clone());
        Some(String::from_utf16_lossy(&units[r]))
    }

    fn replace_text_in_range(&mut self, _range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut App) {
        self.view.update(cx, |v, cx| v.commit_text(text, cx));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |v, cx| {
            v.preedit = if new_text.is_empty() { None } else { Some(new_text.to_string()) };
            cx.notify();
        });
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut App) {
        self.view.update(cx, |v, cx| {
            v.preedit = None;
            cx.notify();
        });
    }

    fn bounds_for_range(&mut self, range: Range<usize>, _: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        let v = self.view.read(cx);
        let layout = *v.layout.borrow();
        let st = v.terminal.lock();
        let c = st.term().grid().cursor.point;
        drop(st);
        let row = (c.line.0 + layout.display_offset as i32).max(0) as f32;
        let x = layout.origin.x + px((c.column.0 + range.start) as f32 * layout.cell_w);
        let y = layout.origin.y + px(row * layout.cell_h);
        Some(Bounds::new(gpui::point(x, y), gpui::size(px(layout.cell_w), px(layout.cell_h))))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut App) -> Option<usize> {
        None
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        // Held keys repeat, as in every terminal (no accent menu).
        false
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window) && window.is_window_active();
        let theme = self.terminal.theme();
        let mut font = self.settings.font.clone();
        font.size = self.font_size;
        let search = self.search.as_ref();
        let params = crate::element::FrameParams {
            mode: self.mode,
            font,
            focused,
            cursor_blink_on: self.blink_on || !self.settings.cursor_blink,
            preedit: self.preedit.clone(),
            link: self.link.as_ref().map(|(m, _)| (*m.start(), *m.end())),
            search_regex: search.and_then(|s| s.regex.clone()),
            search_current: search.and_then(|s| s.current.as_ref().map(|m| (*m.start(), *m.end()))),
            reserve_scrollbar: self.settings.scrollbar && self.interactive(),
            show_scrollbar: self.settings.scrollbar
                && self.interactive()
                && (self.hovered || self.drag != Drag::None || { self.layout.borrow().display_offset > 0 }),
            padding: if self.interactive() { self.settings.padding } else { [0.0; 4] },
        };
        let element = TermElement {
            view: self.interactive().then(|| cx.entity()),
            terminal: self.terminal.clone(),
            cache: self.cache.clone(),
            layout: self.layout.clone(),
            params,
        };
        let mut root = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(crate::element::hsla_rgb(theme.background))
            .child(element);
        if self.interactive() {
            root = root
                .key_context(KEY_CONTEXT)
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::on_key_down))
                .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
                .on_scroll_wheel(cx.listener(Self::on_scroll))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|v, e, w, cx| v.on_mouse_down(MouseButton::Left, e, w, cx)),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(|v, e, w, cx| v.on_mouse_down(MouseButton::Middle, e, w, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|v, e, w, cx| v.on_mouse_down(MouseButton::Right, e, w, cx)),
                )
                .on_mouse_move(cx.listener(Self::on_mouse_move))
                .on_mouse_up(MouseButton::Left, cx.listener(|v, e, w, cx| v.on_mouse_up(MouseButton::Left, e, w, cx)))
                .on_mouse_up(
                    MouseButton::Middle,
                    cx.listener(|v, e, w, cx| v.on_mouse_up(MouseButton::Middle, e, w, cx)),
                )
                .on_mouse_up(MouseButton::Right, cx.listener(|v, e, w, cx| v.on_mouse_up(MouseButton::Right, e, w, cx)))
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|v, e, w, cx| v.on_mouse_up(MouseButton::Left, e, w, cx)),
                )
                .on_action(cx.listener(Self::copy))
                .on_action(cx.listener(Self::paste))
                .on_action(cx.listener(Self::select_all))
                .on_action(cx.listener(Self::clear))
                .on_action(cx.listener(Self::scroll_line_up))
                .on_action(cx.listener(Self::scroll_line_down))
                .on_action(cx.listener(Self::scroll_page_up))
                .on_action(cx.listener(Self::scroll_page_down))
                .on_action(cx.listener(Self::scroll_to_top))
                .on_action(cx.listener(Self::scroll_to_bottom))
                .on_action(cx.listener(Self::toggle_search))
                .on_action(cx.listener(Self::search_next))
                .on_action(cx.listener(Self::search_previous))
                .on_action(cx.listener(|v, _: &IncreaseFontSize, _, cx| v.change_font(1.0, cx)))
                .on_action(cx.listener(|v, _: &DecreaseFontSize, _, cx| v.change_font(-1.0, cx)))
                .on_action(cx.listener(|v, _: &ResetFontSize, _, cx| v.change_font(0.0, cx)));
        }
        if let Some(s) = search {
            let fg = crate::element::hsla_rgb(theme.foreground);
            let mut bg = crate::element::hsla_rgb(theme.selection_background);
            bg.a = 0.95;
            let status = if s.error {
                "invalid".to_string()
            } else if s.query.is_empty() {
                "type to find".to_string()
            } else if s.current.is_some() {
                "↑ ⏎  ↓ ⇧⏎  esc".to_string()
            } else {
                "no matches".to_string()
            };
            root = root.child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(SCROLLBAR_W + 6.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .bg(bg)
                    .text_color(fg)
                    .text_size(px(12.0))
                    .flex()
                    .gap(px(10.0))
                    .child(div().min_w(px(120.0)).child(format!("Find: {}▏", s.query)))
                    .child(div().opacity(0.7).child(status)),
            );
        }
        if let Some((m, link)) = &self.link {
            root = root.child(self.link_tooltip(m, link, &theme));
        }
        root
    }
}

impl TerminalView {
    /// The hovered link's target and how to open it, next to the link.
    fn link_tooltip(&self, m: &Match, link: &Link, theme: &crate::theme::TermTheme) -> gpui::Div {
        let layout = *self.layout.borrow();
        let fg = crate::element::hsla_rgb(theme.foreground);
        let bg = crate::element::hsla_rgb(theme.background);
        let mut line = fg;
        line.a = 0.18;
        let target = match link {
            Link::Url(u) => u.clone(),
            Link::File(f) => file_label(f, self.file_links.as_ref().and_then(|l| l.home.as_deref())),
        };
        let hint = if cfg!(target_os = "macos") { "⌘-click to open" } else { "Ctrl-click to open" };
        let start = m.start();
        let row = (start.line.0 + layout.display_offset as i32 - layout.first_row as i32).max(0) as f32;
        let x = f32::from(layout.origin.x - layout.bounds.origin.x) + start.column.0 as f32 * layout.cell_w;
        let top = f32::from(layout.origin.y - layout.bounds.origin.y) + row * layout.cell_h;
        let height = f32::from(layout.bounds.size.height);
        let width = f32::from(layout.bounds.size.width);
        let mut tip = div()
            .absolute()
            .max_w(px((width - 16.0).max(80.0)))
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .bg(bg)
            .border_1()
            .border_color(line)
            .shadow_md()
            .text_color(fg)
            .text_size(px(12.0))
            .flex()
            .gap(px(10.0))
            .child(div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(target))
            .child(div().flex_none().opacity(0.6).child(hint));
        // Below the link, or above it near the bottom; from its left edge,
        // or from the right on the right half.
        tip = if top + layout.cell_h * 2.0 + 28.0 > height {
            tip.bottom(px(height - top + 4.0))
        } else {
            tip.top(px(top + layout.cell_h + 4.0))
        };
        if x > width / 2.0 {
            tip.right(px(8.0))
        } else {
            tip.left(px(x.max(4.0)))
        }
    }
}

/// `~/proj/src/a.rs:12:3`.
fn file_label(f: &OpenFile, home: Option<&std::path::Path>) -> String {
    let shown = match home.and_then(|h| f.path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => f.path.display().to_string(),
    };
    match (f.line, f.column) {
        (Some(l), Some(c)) => format!("{shown}:{l}:{c}"),
        (Some(l), None) => format!("{shown}:{l}"),
        _ => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{NullStream, TermSize, TerminalConfig};
    use alacritty_terminal::index::Line;

    #[test]
    fn finds_urls_and_osc8_links() {
        let t = Terminal::new(NullStream, TermSize::new(60, 3), TerminalConfig::default());
        t.feed().push(b"see https://example.com/a_b?c=1. and more\r\n");
        t.feed().push(b"\x1b]8;;https://example.org/x\x1b\\click\x1b]8;;\x1b\\ here");
        let st = t.lock();
        let term = st.term();
        let (m, url) = link_at(term, GridPoint::new(Line(0), Column(10))).unwrap();
        assert_eq!(url, "https://example.com/a_b?c=1");
        assert_eq!(m.start().column.0, 4);
        assert!(link_at(term, GridPoint::new(Line(0), Column(1))).is_none());
        let (m, url) = link_at(term, GridPoint::new(Line(1), Column(2))).unwrap();
        assert_eq!(url, "https://example.org/x");
        assert_eq!((m.start().column.0, m.end().column.0), (0, 4));
    }

    #[gpui::test]
    fn ime_composition_sends_only_the_commit(cx: &mut gpui::TestAppContext) {
        use std::sync::{Arc, Mutex};
        #[derive(Clone, Default)]
        struct Rec(Arc<Mutex<Vec<u8>>>);
        impl crate::terminal::TermStream for Rec {
            fn attach(&self, _: crate::terminal::Feed) {}
            fn write(&self, b: &[u8]) {
                self.0.lock().unwrap().extend_from_slice(b);
            }
            fn resize(&self, _: TermSize) {}
        }
        let rec = Rec::default();
        let t = Terminal::new(rec.clone(), TermSize::new(40, 5), TerminalConfig::default());
        let (view, cx) =
            cx.add_window_view(|w, cx| TerminalView::new(t, ViewMode::Interactive, ViewSettings::default(), w, cx));
        cx.run_until_parked();
        rec.0.lock().unwrap().clear();
        cx.update(|window, cx| {
            let mut h = TermInputHandler { view: view.clone() };
            h.replace_and_mark_text_in_range(None, "に", None, window, cx);
            assert_eq!(h.marked_text_range(window, cx), Some(0..1));
            h.replace_and_mark_text_in_range(None, "にほ", None, window, cx);
            assert!(h.bounds_for_range(0..1, window, cx).is_some());
        });
        assert_eq!(view.read_with(cx, |v, _| v.preedit().map(str::to_string)), Some("にほ".into()));
        assert!(rec.0.lock().unwrap().is_empty(), "nothing is sent while composing");
        cx.update(|window, cx| {
            let mut h = TermInputHandler { view: view.clone() };
            h.replace_text_in_range(None, "日本", window, cx);
            assert_eq!(h.marked_text_range(window, cx), None);
        });
        assert_eq!(String::from_utf8(rec.0.lock().unwrap().clone()).unwrap(), "日本");
    }

    #[gpui::test]
    fn cmd_click_on_a_file_reference_opens_it(cx: &mut gpui::TestAppContext) {
        use gpui::Modifiers;
        let dir = std::env::temp_dir().join(format!("pw-filelink-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/cart")).unwrap();
        std::fs::write(dir.join("src/cart/cart.ts"), "export {}\n").unwrap();
        let t = Terminal::new(NullStream, TermSize::new(60, 4), TerminalConfig::default());
        t.feed().push(b"see src/cart/cart.ts:42:7 and src/nope.ts:1\r\n");
        let (view, cx) =
            cx.add_window_view(|w, cx| TerminalView::new(t, ViewMode::Interactive, ViewSettings::default(), w, cx));
        view.update(cx, |v, _| v.set_file_links(Some(FileLinks { cwd: dir.clone(), home: None })));
        let opened: Rc<RefCell<Vec<OpenFile>>> = Rc::default();
        let sink = opened.clone();
        cx.update(|_, cx| {
            cx.subscribe(&view, move |_, e: &OpenFile, _| sink.borrow_mut().push(e.clone())).detach();
        });
        cx.run_until_parked();
        let layout = view.read_with(cx, |v, _| *v.layout.borrow());
        assert!(layout.cols > 0, "the view was drawn");
        let at = |col: usize| {
            gpui::point(
                layout.origin.x + px((col as f32 + 0.5) * layout.cell_w),
                layout.origin.y + px(0.5 * layout.cell_h),
            )
        };
        let cmd = Modifiers::secondary_key();
        // Hover with ⌘ held: checked off the main thread, then underlined.
        cx.simulate_mouse_move(at(8), None, cmd);
        cx.run_until_parked();
        let link = view.read_with(cx, |v, _| v.link.clone());
        let (m, l) = link.expect("a link under the pointer");
        assert_eq!((m.start().column.0, m.end().column.0), (4, 24));
        assert!(matches!(l, Link::File(_)));
        cx.simulate_click(at(8), cmd);
        cx.run_until_parked();
        // A reference to a file that isn't there is no link.
        cx.simulate_mouse_move(at(32), None, cmd);
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.link.is_none()));
        cx.simulate_click(at(32), cmd);
        cx.run_until_parked();
        let opened = opened.borrow().clone();
        assert_eq!(opened.len(), 1, "{opened:?}");
        assert_eq!(opened[0].path, std::fs::canonicalize(dir.join("src/cart/cart.ts")).unwrap());
        assert_eq!((opened[0].line, opened[0].column), (Some(42), Some(7)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[gpui::test]
    fn cmd_click_before_the_check_finishes_still_opens(cx: &mut gpui::TestAppContext) {
        use gpui::Modifiers;
        let dir = std::env::temp_dir().join(format!("pw-filelink2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ჩემი.md"), "# x\n").unwrap();
        let t = Terminal::new(NullStream, TermSize::new(60, 4), TerminalConfig::default());
        t.feed().push("ok \"ჩემი.md\":3\r\n".as_bytes());
        let (view, cx) =
            cx.add_window_view(|w, cx| TerminalView::new(t, ViewMode::Interactive, ViewSettings::default(), w, cx));
        view.update(cx, |v, _| v.set_file_links(Some(FileLinks { cwd: dir.clone(), home: None })));
        let opened: Rc<RefCell<Vec<OpenFile>>> = Rc::default();
        let sink = opened.clone();
        cx.update(|_, cx| {
            cx.subscribe(&view, move |_, e: &OpenFile, _| sink.borrow_mut().push(e.clone())).detach();
        });
        cx.run_until_parked();
        let layout = view.read_with(cx, |v, _| *v.layout.borrow());
        let pos = gpui::point(layout.origin.x + px(5.5 * layout.cell_w), layout.origin.y + px(0.5 * layout.cell_h));
        cx.simulate_click(pos, Modifiers::secondary_key());
        cx.run_until_parked();
        let opened = opened.borrow().clone();
        assert_eq!(opened.len(), 1);
        assert!(opened[0].path.ends_with("ჩემი.md"));
        assert_eq!(opened[0].line, Some(3));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_labels_shorten_home() {
        let f = OpenFile { path: PathBuf::from("/home/demo/proj/a.rs"), line: Some(3), column: Some(2) };
        assert_eq!(file_label(&f, Some(std::path::Path::new("/home/demo"))), "~/proj/a.rs:3:2");
        let f = OpenFile { line: None, column: None, ..f };
        assert_eq!(file_label(&f, None), "/home/demo/proj/a.rs");
    }

    #[test]
    fn search_escapes_literals() {
        assert_eq!(regex_escape("a.b*"), "a\\.b\\*");
    }
}
