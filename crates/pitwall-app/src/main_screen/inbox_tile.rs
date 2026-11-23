//! An extension's inbox in a tile of the space's layout (docs/spec/
//! extensions.md "The inbox tile"), a peer of the files and web tiles:
//! one per extension, opened with ⌘I / Ctrl+Shift+I or "Open <name> inbox"
//! in the command palette, placed as a web page is, and back after a
//! restart (`ui.json`: the pane's `inbox`, the thread shown in
//! `extensions.<id>.selected`).
//!
//! The threads on the left, the thread shown on the right; in a narrow
//! tile the list, and the thread over it with a way back. What an agent
//! asks is the point: an open ask is a card with its suggestions as
//! buttons (a key each), a blocking one in the attention colour. An answer
//! shows as sent at once and goes back, with why, if the extension says it
//! failed. Keys work while the tile itself has the keyboard (never while
//! typing in its fields or while a menu of its bar is open, never in a
//! terminal): ↑↓ or J K, Enter, 1–9, E, R, Esc.
//!
//! An extension with two or more connections (docs/spec/profiles.md)
//! lists all of their threads by default, each row naming its
//! connection; a menu beside "Needs you / All" lists one connection's
//! instead (each named with the profiles that use it), kept in
//! `extensions.<id>.connection`. A connection that can't show its threads
//! gets a line of its own over the list. With one connection, none of
//! this shows.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    anchored, canvas, deferred, div, prelude::*, px, AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent, MouseButton, Pixels, Point,
    ScrollHandle, SharedString, Subscription, Window,
};
use pitwall_core::extensions::protocol::{Ask, AuthorKind, ConnState, Message, Thread, ThreadDetail};
use pitwall_core::extensions::Manifest;
use pitwall_proto::AgentView;

use super::files_tile::TileDrag;
use super::strip::Toast;
use super::tree::Content;
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::explorer::dock::FilesPlace;
use crate::extensions::hub::{self, Hub, Request, Run};
use crate::kit::{self, Ellipsis, InputEvent, TextInput};
use crate::markdown_view::MarkdownView;
use crate::theme::{theme, Theme, RADIUS};

/// Narrower than this, the list and the thread take turns.
pub const NARROW: f32 = 760.;
/// The thread's text is at most this wide.
const READING: f32 = 720.;

/// Which threads the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Those with an open ask.
    #[default]
    NeedsYou,
    All,
}

/// The threads to list: those `filter` keeps and that aren't being
/// archived, blocking ones first, then those that ask, then the latest.
pub fn ordered(threads: &[Thread], filter: Filter, hidden: &HashSet<String>) -> Vec<Thread> {
    let mut out: Vec<Thread> = threads
        .iter()
        .filter(|t| !hidden.contains(&t.id))
        .filter(|t| filter == Filter::All || t.open_asks > 0)
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        (!a.blocking, a.open_asks == 0)
            .cmp(&(!b.blocking, b.open_asks == 0))
            .then_with(|| b.last_activity.cmp(&a.last_activity))
    });
    out
}

/// `iso` said shortly ("now", "5m", "3h", "2d", "Mar 4") and exactly (for
/// its tooltip), from `now`.
pub fn when(iso: &str, now: chrono::DateTime<chrono::Utc>) -> Option<(String, String)> {
    let t = chrono::DateTime::parse_from_rfc3339(iso).ok()?.with_timezone(&chrono::Utc);
    let s = (now - t).num_seconds().max(0);
    let local = t.with_timezone(&chrono::Local);
    let short = match s {
        0..60 => "now".to_string(),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        86400..604800 => format!("{}d", s / 86400),
        _ => local.format("%b %-d").to_string(),
    };
    Some((short, local.format("%A %-d %B %Y, %H:%M").to_string()))
}

/// An answer sent and not yet confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sent {
    req: u64,
    labels: Vec<String>,
    text: Option<String>,
    declined: bool,
}

/// The "Move…" / "Dock…" menu: where it opened, the terminals it may
/// take the place of.
type PlaceMenu = (Point<Pixels>, Vec<(String, String)>);

pub struct InboxView {
    ext: String,
    focus: FocusHandle,
    filter: Filter,
    selected: Option<String>,
    /// The thread opened in the hub (its view kept current).
    opened: Option<String>,
    /// Narrow: the thread is over the list.
    pushed: bool,
    width: Rc<Cell<f32>>,
    reply: Entity<TextInput>,
    /// "Or write your own answer…" of each open ask: (thread, ask).
    others: HashMap<(String, u32), Entity<TextInput>>,
    /// The picks of a multiple-choice ask so far.
    picks: HashMap<(String, u32), Vec<u32>>,
    sent: HashMap<(String, u32), Sent>,
    /// A reply on its way: thread, request, text.
    replying: Option<(String, u64, String)>,
    /// Threads being archived (out of the list): their request.
    archiving: HashMap<String, u64>,
    /// Threads marked read while open.
    marked: HashSet<String>,
    /// Messages whose details are shown.
    expanded: HashSet<String>,
    md: HashMap<String, Entity<MarkdownView>>,
    /// The thread's position to go to when it arrives.
    scroll_pending: bool,
    list_scroll: ScrollHandle,
    thread_scroll: ScrollHandle,
    /// The Pitwall agents (whose threads are whose).
    pub agents: Vec<AgentView>,
    /// Drawn by a window of its own (else a tile of a layout).
    in_window: bool,
    /// Its window kept above other apps.
    pinned: bool,
    /// The "Move…" / "Dock…" menu, open.
    menu: Option<PlaceMenu>,
    /// The connections menu, open.
    conn_menu: bool,
    /// The keys go back to the tile itself at the next frame (a field
    /// let them go: Esc).
    refocus: bool,
    /// An ask just answered: if its field had the keys, they go back to
    /// the tile (the field is gone).
    answered: Option<(String, u32)>,
    _subs: Vec<Subscription>,
}

impl Focusable for InboxView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl InboxView {
    /// Extension `ext`'s inbox. Not tied to a window: one view is drawn
    /// wherever the inbox is ([`view`]).
    pub fn new(ext: &str, cx: &mut Context<Self>) -> Self {
        let reply = cx.new(|cx| TextInput::new(cx, "", "Reply…  ⌘↵ sends").grow(6).max_len(pitwall_core::extensions::protocol::MAX_REPLY));
        let h = hub::hub(cx);
        let mut subs = vec![
            cx.observe(&h, |this, _, cx| {
                this.settle(cx);
                cx.notify();
            }),
            cx.subscribe(&reply, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Submit => this.send_reply(cx),
                InputEvent::Cancel => this.keys_back(cx),
                _ => {}
            }),
        ];
        subs.push(cx.on_release(|this: &mut InboxView, cx: &mut App| {
            if let (Some(t), Some(h)) = (this.opened.take(), hub::try_hub(cx)) {
                let ext = this.ext.clone();
                h.update(cx, |h, _| h.close(&ext, &t));
            }
        }));
        let selected = crate::ui_state::get(cx).extension(ext).selected;
        let mut v = InboxView {
            ext: ext.to_string(),
            focus: cx.focus_handle(),
            filter: Filter::default(),
            selected: None,
            opened: None,
            pushed: false,
            width: Rc::new(Cell::new(-1.)),
            reply,
            others: HashMap::new(),
            picks: HashMap::new(),
            sent: HashMap::new(),
            replying: None,
            archiving: HashMap::new(),
            marked: HashSet::new(),
            expanded: HashSet::new(),
            md: HashMap::new(),
            scroll_pending: false,
            list_scroll: ScrollHandle::new(),
            thread_scroll: ScrollHandle::new(),
            agents: Vec::new(),
            in_window: false,
            pinned: false,
            menu: None,
            conn_menu: false,
            refocus: false,
            answered: None,
            _subs: subs,
        };
        if let Some(s) = selected {
            v.show(&s, false, cx);
        }
        v
    }

    pub fn ext(&self) -> &str {
        &self.ext
    }

    /// Who draws it now (a window of its own, or a tile) and the Pitwall
    /// agents (whose threads are whose).
    pub fn held(&mut self, in_window: bool, agents: Vec<AgentView>, cx: &mut Context<Self>) {
        if self.in_window != in_window {
            self.in_window = in_window;
            self.menu = None;
            cx.notify();
        }
        if self.agents != agents {
            self.agents = agents;
        }
    }

    /// The keys back to the tile itself (a field let them go).
    fn keys_back(&mut self, cx: &mut Context<Self>) {
        self.refocus = true;
        cx.notify();
    }

    /// One of its fields has the keyboard: typing goes there, and the
    /// tile's single keys don't apply.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.reply.read(cx).is_focused(window) || self.others.values().any(|i| i.read(cx).is_focused(window))
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn narrow(&self) -> bool {
        (0. ..NARROW).contains(&self.width.get())
    }

    /// The thread is over the list (narrow tiles).
    pub fn pushed(&self) -> bool {
        self.pushed
    }

    fn manifest(&self, cx: &App) -> Option<Manifest> {
        crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == self.ext).cloned()
    }

    /// Show thread `id` (`push`: over the list in a narrow tile), going to
    /// its first open ask; remembered for the next start.
    pub fn show(&mut self, id: &str, push: bool, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_string());
            self.scroll_pending = true;
            let ext = self.ext.clone();
            let h = hub::hub(cx);
            if let Some(old) = self.opened.replace(id.to_string()) {
                h.update(cx, |h, _| h.close(&ext, &old));
            }
            h.update(cx, |h, cx| h.open(&ext, id, cx));
            let keep = id.to_string();
            crate::ui_state::update(cx, |ui| *ui = std::mem::take(ui).with_extension(&ext, |e| e.selected = Some(keep)));
        }
        self.pushed |= push;
        cx.notify();
    }

    /// Back to the list (narrow tiles).
    pub fn back(&mut self, cx: &mut Context<Self>) {
        self.pushed = false;
        cx.notify();
    }

    pub fn set_filter(&mut self, f: Filter, cx: &mut Context<Self>) {
        self.filter = f;
        cx.notify();
    }

    /// The connection whose threads are listed (`None`: all): the one
    /// kept in `ui.json`, while the extension has it and another.
    pub fn connection(&self, cx: &App) -> Option<String> {
        let want = crate::ui_state::get(cx).extension(&self.ext).connection?;
        let st = hub::try_hub(cx)?.read(cx).get(&self.ext)?;
        (st.connections.len() > 1 && st.connections.iter().any(|v| v.id == want)).then_some(want)
    }

    /// List connection `id`'s threads only (`None`: all of them), kept for
    /// the next start.
    pub fn set_connection(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.conn_menu = false;
        let ext = self.ext.clone();
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(&ext, |e| e.connection = id));
        cx.notify();
    }

    /// The connections menu is open (the tile's single keys don't apply).
    pub fn conn_menu_open(&self) -> bool {
        self.conn_menu
    }

    /// The threads listed now.
    pub fn listed(&self, cx: &App) -> Vec<Thread> {
        let Some(h) = hub::try_hub(cx) else { return vec![] };
        let hidden: HashSet<String> = self.archiving.keys().cloned().collect();
        let conn = self.connection(cx);
        h.read(cx)
            .get(&self.ext)
            .map(|s| {
                let threads: Vec<Thread> = s.threads_of(conn.as_deref()).cloned().collect();
                ordered(&threads, self.filter, &hidden)
            })
            .unwrap_or_default()
    }

    fn detail(&self, cx: &App) -> Option<ThreadDetail> {
        let id = self.selected.as_ref()?;
        hub::try_hub(cx)?.read(cx).get(&self.ext)?.details.get(id).cloned()
    }

    /// The hub changed: answers confirmed or refused, archives done, the
    /// thread shown marked read, and the view moved to its open ask.
    fn settle(&mut self, cx: &mut Context<Self>) {
        let Some(h) = hub::try_hub(cx) else { return };
        let h = h.read(cx);
        let Some(st) = h.get(&self.ext) else { return };
        let details = &st.details;
        self.sent.retain(|(thread, ask), s| match st.result(s.req) {
            Some(false) => false,
            // Kept until the thread says the ask is answered.
            Some(true) => details.get(thread).is_none_or(|d| d.messages.iter().flat_map(|m| &m.asks).any(|a| a.number == *ask && a.open)),
            None => st.busy.contains(thread),
        });
        let mut restore = None;
        if let Some((thread, req, text)) = self.replying.clone() {
            match st.result(req) {
                Some(false) => {
                    restore = Some(text);
                    self.replying = None;
                }
                Some(true) => self.replying = None,
                None if !st.busy.contains(&thread) => self.replying = None,
                None => {}
            }
        }
        let threads = &st.threads;
        self.archiving.retain(|id, req| match st.result(*req) {
            Some(false) => false,
            _ => threads.iter().any(|t| t.id == *id),
        });
        let mut mark = None;
        if let Some(d) = self.selected.as_ref().and_then(|id| details.get(id)) {
            if d.unread > 0 && !self.marked.contains(&d.id) {
                mark = Some(d.id.clone());
            }
            if self.scroll_pending {
                self.scroll_pending = false;
                match d.messages.iter().position(|m| m.asks.iter().any(|a| a.open)) {
                    Some(i) => self.thread_scroll.scroll_to_item(i),
                    None => self.thread_scroll.scroll_to_bottom(),
                }
            }
        }
        if let Some(text) = restore {
            self.reply.update(cx, |r, cx| {
                if r.text().is_empty() {
                    r.set_text(text, cx);
                }
            });
        }
        self.fill(cx);
        if let Some(id) = mark {
            self.marked.insert(id.clone());
            let ext = self.ext.clone();
            let h = hub::hub(cx);
            let _ = h.update(cx, |h, cx| h.request(&ext, &id, Request::MarkRead, cx));
        }
    }

    /// A wide tile always shows a thread: the first, when none is (once
    /// its width is known).
    fn fill(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_none() && self.width.get() >= NARROW {
            if let Some(first) = self.listed(cx).first().map(|t| t.id.clone()) {
                self.show(&first, false, cx);
            }
        }
    }

    fn request(&self, thread: &str, r: Request, cx: &mut Context<Self>) -> Result<u64, String> {
        let ext = self.ext.clone();
        let h = hub::hub(cx);
        h.update(cx, |h, cx| h.request(&ext, thread, r, cx))
    }

    /// A note on `thread` (why something didn't go), shown in it.
    fn note(&self, thread: &str, why: String, cx: &mut Context<Self>) {
        let (ext, thread) = (self.ext.clone(), thread.to_string());
        hub::hub(cx).update(cx, |h, cx| h.set_note(&ext, &thread, why, cx));
    }

    /// Answer `ask` of `thread`: shown as answered at once, back with why
    /// if it fails. The request, when it went.
    pub fn answer(&mut self, thread: &str, ask: &Ask, picks: Vec<u32>, text: Option<String>, declined: bool, cx: &mut Context<Self>) -> Option<u64> {
        let labels: Vec<String> = picks.iter().filter_map(|i| ask.suggestions.get(*i as usize)).map(|s| s.label.clone()).collect();
        let sent = match self.request(thread, Request::Answer { ask: ask.number, picks, text: text.clone(), declined }, cx) {
            Ok(req) => {
                let key = (thread.to_string(), ask.number);
                self.sent.insert(key.clone(), Sent { req, labels, text: text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()), declined });
                self.picks.remove(&key);
                if let Some(i) = self.others.get(&key) {
                    i.update(cx, |i, cx| i.set_text("", cx));
                }
                self.answered = Some(key);
                Some(req)
            }
            Err(why) => {
                self.note(thread, why, cx);
                None
            }
        };
        cx.notify();
        sent
    }

    fn send_reply(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.selected.clone() else { return };
        let text = self.reply.read(cx).text().to_string();
        match self.request(&thread, Request::Message { text: text.clone() }, cx) {
            Ok(req) => {
                self.replying = Some((thread, req, text.trim().to_string()));
                self.reply.update(cx, |r, cx| r.set_text("", cx));
            }
            Err(why) => self.note(&thread, why, cx),
        }
        cx.notify();
    }

    /// Archive `thread`: out of the list at once (back if it fails), the
    /// next one shown.
    pub fn archive(&mut self, thread: &str, cx: &mut Context<Self>) {
        let list = self.listed(cx);
        let at = list.iter().position(|t| t.id == thread);
        match self.request(thread, Request::Archive, cx) {
            Ok(req) => {
                self.archiving.insert(thread.to_string(), req);
                if self.selected.as_deref() == Some(thread) {
                    let next = at.and_then(|i| list.get(i + 1).or(i.checked_sub(1).and_then(|j| list.get(j))));
                    match next {
                        Some(n) => {
                            let id = n.id.clone();
                            self.show(&id, false, cx);
                        }
                        None => self.pushed = false,
                    }
                }
            }
            Err(why) => self.note(thread, why, cx),
        }
        cx.notify();
    }

    /// The first open ask of the thread shown that isn't answered yet.
    fn first_open(&self, cx: &App) -> Option<(String, Ask)> {
        let d = self.detail(cx)?;
        d.messages.iter().flat_map(|m| &m.asks).find(|a| a.open && !self.sent.contains_key(&(d.id.clone(), a.number))).map(|a| (d.id.clone(), a.clone()))
    }

    /// Pick suggestion `i` of `ask`: sent at once, or toggled in a
    /// multiple-choice one.
    pub fn pick(&mut self, thread: &str, ask: &Ask, i: u32, cx: &mut Context<Self>) {
        if i as usize >= ask.suggestions.len() {
            return;
        }
        if ask.multiple {
            let p = self.picks.entry((thread.to_string(), ask.number)).or_default();
            match p.iter().position(|x| *x == i) {
                Some(at) => {
                    p.remove(at);
                }
                None => {
                    p.push(i);
                    p.sort();
                }
            }
            cx.notify();
        } else {
            self.answer(thread, ask, vec![i], None, false, cx);
        }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let list = self.listed(cx);
        if list.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| list.iter().position(|t| t.id == *s));
        let next = match at {
            Some(i) => (i as isize + by).clamp(0, list.len() as isize - 1) as usize,
            None => 0,
        };
        let id = list[next].id.clone();
        self.show(&id, false, cx);
        self.list_scroll.scroll_to_item(next);
    }

    /// A key while the tile has the keyboard (itself, or its text being
    /// selected), never while one of its fields does: typed there.
    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) || self.typing(window, cx) {
            return;
        }
        let m = &e.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.function {
            return;
        }
        // A menu of its bar is open: its keys are the menu's (Esc closes
        // it), never the tile's.
        if self.conn_menu {
            if e.keystroke.key == "escape" {
                self.conn_menu = false;
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        let key = e.keystroke.key.as_str();
        let handled = match key {
            "up" | "k" => {
                self.step(-1, cx);
                true
            }
            "down" | "j" => {
                self.step(1, cx);
                true
            }
            "enter" => {
                match self.selected.clone() {
                    Some(s) => self.show(&s, true, cx),
                    None => self.step(0, cx),
                }
                true
            }
            "escape" if self.pushed => {
                self.back(cx);
                true
            }
            "e" => {
                if let Some(s) = self.selected.clone() {
                    self.archive(&s, cx);
                }
                true
            }
            "r" if self.detail(cx).is_some_and(|d| d.can_reply) => {
                self.pushed |= self.selected.is_some();
                self.reply.update(cx, |r, cx| r.focus(window, cx));
                true
            }
            k if k.len() == 1 && ("1"..="9").contains(&k) => {
                if let Some((thread, ask)) = self.first_open(cx) {
                    let i = k.parse::<u32>().unwrap_or(1) - 1;
                    self.pick(&thread, &ask, i, cx);
                }
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn markdown(&mut self, key: String, text: &str, cx: &mut Context<Self>) -> Entity<MarkdownView> {
        let v = self.md.entry(key.clone()).or_insert_with(|| cx.new(|cx| MarkdownView::new(cx).inline())).clone();
        let text: Arc<str> = Arc::from(text);
        v.update(cx, |v, cx| v.set(&key, text, None, None, cx));
        v
    }

    /// Bench: a field by name, `reply` or `other` (the first open ask's
    /// own answer, of the thread shown).
    pub(super) fn debug_field(&self, which: &str) -> Option<Entity<TextInput>> {
        match which {
            "reply" => Some(self.reply.clone()),
            _ => {
                let th = self.selected.as_ref()?;
                self.others.iter().filter(|((t, _), _)| t == th).min_by_key(|((_, a), _)| *a).map(|(_, i)| i.clone())
            }
        }
    }

    fn other_input(&mut self, thread: &str, ask: &Ask, cx: &mut Context<Self>) -> Entity<TextInput> {
        let key = (thread.to_string(), ask.number);
        if let Some(i) = self.others.get(&key) {
            return i.clone();
        }
        let input = cx.new(|cx| TextInput::new(cx, "", "Or write your own answer…").max_len(pitwall_core::extensions::protocol::MAX_REPLY));
        let (t, a) = (thread.to_string(), ask.clone());
        let sub = cx.subscribe(&input, move |this, input, e: &InputEvent, cx| match e {
            InputEvent::Submit => {
                let text = input.read(cx).text().to_string();
                let picks = this.picks.get(&(t.clone(), a.number)).cloned().unwrap_or_default();
                this.answer(&t, &a, picks, Some(text), false, cx);
            }
            InputEvent::Cancel => this.keys_back(cx),
            _ => {}
        });
        self._subs.push(sub);
        self.others.insert(key, input.clone());
        input
    }
}

// ── drawing ──────────────────────────────────────────────────────────────

/// A grey bar of a loading row.
/// Why connection `v` shows nothing, in a line naming it.
fn offline_note(v: &hub::ConnView) -> String {
    match (&v.run, &v.conn) {
        (Run::Stopped(why), _) => format!("{} stopped: {why}", v.name),
        (_, Some((ConnState::SignedOut, m))) if !m.is_empty() => format!("{} is signed out: {m}", v.name),
        (_, Some((ConnState::SignedOut, _))) => format!("{} is signed out. Check its key in Settings → Extensions.", v.name),
        (_, Some((_, m))) if !m.is_empty() => format!("{} is offline: {m}", v.name),
        _ => format!("{} is offline. It will keep trying.", v.name),
    }
}

fn bar(w: f32, t: &Theme) -> gpui::Div {
    div().h(px(9.)).w(px(w)).rounded(px(4.)).bg(t.text.opacity(0.08))
}

/// A calm message filling the tile, with what to do.
pub(super) fn calm(icon: &str, title: &str, body: String, t: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .px(px(24.))
        .text_center()
        .child(div().size(px(40.)).rounded_full().bg(t.surface_2).flex().items_center().justify_center().child(kit::icon(icon, 20., t.text_3)))
        .child(div().mt(px(4.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title.to_string()))
        .child(div().max_w(px(360.)).text_size(px(13.)).line_height(px(19.)).text_color(t.text_3).child(body))
}

impl InboxView {
    fn header(&self, m: &Manifest, st: Option<&hub::ExtState>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let conn = self.connection(cx);
        let counts = st.map(|s| s.counts_of(conn.as_deref())).unwrap_or_default();
        let chooser = st.filter(|s| s.run == Run::Running && s.connections.len() > 1).map(|s| self.conn_chooser(s, conn.as_deref(), t, cx));
        let running = st.is_some_and(|s| s.run == Run::Running);
        let filter = self.filter;
        let weak = cx.entity().downgrade();
        let pick: kit::OnPick = Rc::new(move |i, _, cx| {
            let _ = weak.update(cx, |v, cx| v.set_filter(if i == 0 { Filter::NeedsYou } else { Filter::All }, cx));
        });
        let needs = if counts.open_asks > 0 { format!("Needs you · {}", counts.open_asks) } else { "Needs you".to_string() };
        let in_window = self.in_window;
        // Its tile maximised in its space (the bar's Maximise / Restore).
        let maximized = {
            let ui = crate::ui_state::get(cx);
            ui.tile_of(&Content::Inbox(self.ext.clone())).is_some_and(|(space, pane)| ui.space(&space).is_some_and(|s| s.maximized_pane_id.as_deref() == Some(pane.as_str())))
        };
        let btn = |name: &str, icon: &str, tip: String, on: Option<bool>| {
            let sel = format!("inbox-{name}");
            kit::icon_btn_state(sel.clone(), icon, tip, true, 13., on, t)
                .debug_selector(move || sel.clone())
                // A press on a control is never a drag of the tile.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let name: SharedString = m.name.clone().into();
        let ext = self.ext.clone();
        div()
            .id(SharedString::from(format!("inbox-bar-{}", self.ext)))
            .debug_selector(|| "inbox-bar".into())
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(4.))
            .pr(px(8.))
            .border_b_1()
            .border_color(if t.is_glass() { t.g.line } else { t.line })
            // A tile: dragged in its window's layout by the bar, as a web
            // tile (GPUI's drag); a window: its grip drags it into a layout.
            .when(!in_window, |d| d.on_drag(TileDrag { what: Content::Inbox(ext.clone()), name: name.clone() }, |d, grab, w, cx| d.ghost(grab, w, cx)))
            .child(
                div()
                    .id("inbox-grip")
                    .debug_selector(|| "inbox-grip".into())
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(kit::tooltip(if in_window { "Drag into a Pitwall window to place it in the layout" } else { "Drag to move the inbox" }))
                    .when(in_window, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(|v, _, window, cx| {
                            cx.stop_propagation();
                            let ext = v.ext.clone();
                            window.defer(cx, move |window, cx| super::inbox_window::drag_start(&ext, window, cx));
                        }))
                    })
                    .child(kit::icon("grip", 13., t.text_4)),
            )
            .child(kit::icon(crate::extensions::icon_of(m), 15., t.text_2))
            .child(div().flex_1().min_w_0().ellipsis().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(kit::one_line(m.name.clone())))
            .children(chooser)
            .when(running, |d| {
                d.child(
                    div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(kit::seg("inbox-filter", vec![kit::SegItem::new(needs, filter == Filter::NeedsYou), kit::SegItem::new("All", filter == Filter::All)], t, pick)),
                )
            })
            .child(
                btn(if in_window { "dock" } else { "move" }, "grid", (if in_window { "Place it in a Pitwall window's layout" } else { "Move it in the layout, or to a window" }).into(), Some(self.menu.is_some()))
                    .on_click(cx.listener(|v, e: &gpui::ClickEvent, _, cx| {
                        v.menu = match v.menu {
                            Some(_) => None,
                            None => Some((e.position(), super::inbox_window::replaceable(&v.ext, cx))),
                        };
                        cx.notify();
                    })),
            )
            .map(|d| {
                if in_window {
                    d.when(crate::platform::can_keep_on_top(), |d| {
                        let pinned = self.pinned;
                        d.child(
                            btn("pin", "pin", (if pinned { "Kept on top of other windows: click to let it go behind them" } else { "Keep on top of other windows" }).into(), Some(pinned)).on_click(cx.listener(|v, _, window, cx| {
                                v.pinned = !v.pinned;
                                crate::platform::keep_on_top(window, v.pinned);
                                cx.notify();
                            })),
                        )
                    })
                } else {
                    d.child(
                        btn("maximize", if maximized { "restore" } else { "maximize" }, kit::keys(if maximized { "Restore (⌘⏎)" } else { "Maximise (⌘⏎)" }, false), None).on_click(cx.listener(|v, _, window, cx| {
                            // Its tile is the space's focused one (the press
                            // made it so): ⌘⏎'s own action.
                            window.focus(&v.focus, cx);
                            window.dispatch_action(Box::new(super::ToggleMaximize), cx);
                        })),
                    )
                    .child(btn("undock", "window", "Open in a window of its own".into(), None).on_click(cx.listener(|v, _, _, cx| {
                        let ext = v.ext.clone();
                        cx.defer(move |cx| {
                            if let Err(e) = super::inbox_window::pop_out(&ext, None, true, cx) {
                                eprintln!("pitwall: could not open the inbox's window: {e}");
                            }
                        });
                    })))
                    .child(btn("close", "x", "Close the inbox".into(), None).on_click(cx.listener(|v, _, _, cx| {
                        let ext = v.ext.clone();
                        cx.defer(move |cx| close(&ext, cx));
                    })))
                }
            })
            .into_any_element()
    }

    /// Which connections' threads are listed: a small button naming them
    /// ("All connections", or one), a dot when one of them is offline, and
    /// under it, while open, the menu: All connections, then each with the
    /// profiles that use it.
    fn conn_chooser(&self, st: &hub::ExtState, conn: Option<&str>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let open = self.conn_menu;
        let label = conn.and_then(|c| st.connections.iter().find(|v| v.id == c)).map(|v| v.name.clone()).unwrap_or_else(|| "All connections".into());
        let off = st.connections.iter().filter(|v| conn.is_none_or(|c| v.id == c)).any(|v| v.offline());
        let button = div()
            .id("inbox-conn")
            .debug_selector(|| "inbox-conn".into())
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(RADIUS)
            .border_1()
            .border_color(if open { t.text_3 } else { t.line_strong })
            .cursor_pointer()
            .hover(|d| d.bg(t.surface_2))
            .tooltip(kit::tooltip("Which connections' threads are listed"))
            .when(off, |d| d.child(div().flex_none().size(px(6.)).rounded_full().bg(t.amber)))
            .child(div().max_w(px(150.)).ellipsis().text_size(px(12.)).text_color(t.text).child(kit::one_line(label)))
            .child(kit::icon("chevron-down", 10., t.text_3))
            // A press on a control is never a drag of the tile.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|v, _, _, cx| {
                v.conn_menu = !v.conn_menu;
                cx.notify();
            }));
        let menu = open.then(|| {
            let list = crate::accounts::list(cx).cloned().unwrap_or_default();
            let me = cx.entity().downgrade();
            let mut items: Vec<(Option<String>, String, Option<SharedString>)> = vec![(None, "All connections".into(), None)];
            for v in &st.connections {
                let mut hint: Vec<String> = list.profiles_of(&v.id);
                if v.offline() {
                    hint.push("offline".into());
                }
                items.push((Some(v.id.clone()), v.name.clone(), (!hint.is_empty()).then(|| hint.join(" · ").into())));
            }
            let rows: Vec<_> = items
                .into_iter()
                .enumerate()
                .map(|(i, (id, name, hint))| {
                    let me = me.clone();
                    let on = id.as_deref() == conn;
                    kit::menu::menu_item(("inbox-conn-item", i), if on { "check" } else { "" }, name, hint, false, t)
                        .debug_selector(move || format!("inbox-conn-item-{i}"))
                        .on_click(move |_, _, cx| {
                            let id = id.clone();
                            let _ = me.update(cx, |v, cx| v.set_connection(id, cx));
                        })
                })
                .collect();
            let close = cx.entity().downgrade();
            deferred(
                anchored().snap_to_window().child(
                    kit::menu::menu_panel(t)
                        .id("inbox-conn-menu")
                        .debug_selector(|| "inbox-conn-menu".into())
                        .mt(px(28.))
                        .children(rows)
                        .on_mouse_down_out(move |_, _, cx| {
                            let _ = close.update(cx, |v, cx| {
                                v.conn_menu = false;
                                cx.notify();
                            });
                        }),
                ),
            )
            .with_priority(2)
        });
        div().flex_none().child(button).children(menu).into_any_element()
    }

    /// The "Move…" / "Dock…" menu's places ([`crate::explorer::dock::menu`]).
    fn place_menu(&self, at: Point<Pixels>, replace: Vec<(String, String)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let ext = self.ext.clone();
        let pick = move |p: Option<(FilesPlace, Option<String>)>, cx: &mut App| {
            let _ = me.update(cx, |v, cx| {
                v.menu = None;
                cx.notify();
            });
            if let Some((place, of)) = p {
                let ext = ext.clone();
                cx.defer(move |cx| {
                    if let Err(e) = super::inbox_window::place(&ext, place, of.as_deref(), cx) {
                        eprintln!("pitwall: could not move the inbox: {e}");
                    }
                });
            }
        };
        crate::explorer::dock::menu(at, !self.in_window, None, replace, "inbox", t, pick)
    }

    /// The tile when there are no threads to show: why, and what to do.
    fn state(&self, m: &Manifest, st: Option<&hub::ExtState>, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ext = self.ext.clone();
        let settings = |id: &'static str, label: &'static str, t: &Theme| {
            kit::text_button(id, label, kit::BtnKind::Primary, false, t).debug_selector(move || id.into()).on_click(|_, window, cx| {
                to_extensions_page(cx);
                window.dispatch_action(Box::new(crate::menu::OpenSettings), cx)
            })
        };
        let run = st.map(|s| s.run.clone()).unwrap_or_default();
        let el = match run {
            Run::Off if !crate::ui_state::get(cx).extension_on(&m.id) => calm("extension", &format!("{} is off", m.name), "Turn it on in Settings → Extensions.".into(), t).child(settings("inbox-settings", "Open Settings", t)),
            Run::Off => calm("alert", "It has no adapter", "This extension has nothing to show here.".into(), t),
            Run::Setup(why) => calm("lock", &format!("Connect {}", m.name), why, t).child(settings("inbox-connect", "Open Settings", t)),
            Run::Stopped(why) => {
                // What it printed goes in a box of its own, its last lines.
                let (head, said) = match why.split_once(" It said: ") {
                    Some((h, s)) => (h.to_string(), Some(s.to_string())),
                    None => (why, None),
                };
                calm("alert", &format!("{} stopped", m.name), format!("{head} Restart it, or check its settings in Settings → Extensions."), t)
                    .when_some(said, |d, said| d.child(output_tail(&said, t)))
                    .child(
                kit::text_button("inbox-restart", "Restart", kit::BtnKind::Primary, false, t).debug_selector(|| "inbox-restart".into()).on_click(cx.listener(move |_, _, _, cx| {
                    let ext = ext.clone();
                    hub::hub(cx).update(cx, |h, cx| h.restart(&ext, cx));
                })),
                    )
            }
            Run::Starting => return Some(self.skeleton(t)),
            Run::Running => {
                let st = st?;
                // Several connections: the list says which is offline.
                let one = st.connections.len() < 2;
                match &st.conn {
                    Some((ConnState::SignedOut, msg)) if st.threads.is_empty() && one => {
                        calm("lock", "Signed out", if msg.is_empty() { "Check the API key in Settings → Extensions.".into() } else { msg.clone() }, t).child(settings("inbox-signed-out", "Open Settings", t))
                    }
                    Some((ConnState::Error, msg)) if st.threads.is_empty() && one => calm("alert", &format!("Can't reach {}", m.name), if msg.is_empty() { "It will keep trying.".into() } else { msg.clone() }, t).child(
                        kit::text_button("inbox-retry", "Restart", kit::BtnKind::Ghost, false, t).on_click(cx.listener(move |_, _, _, cx| {
                            let ext = ext.clone();
                            hub::hub(cx).update(cx, |h, cx| h.restart(&ext, cx));
                        })),
                    ),
                    _ if !st.listed() => return Some(self.skeleton(t)),
                    _ if st.threads.is_empty() && one => calm("check", "Nothing needs you", "Questions your agents ask will show here.".into(), t),
                    _ => return None,
                }
            }
        };
        Some(el.debug_selector(|| "inbox-state".into()).into_any_element())
    }

    fn skeleton(&self, t: &Theme) -> AnyElement {
        div()
            .debug_selector(|| "inbox-loading".into())
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(8.))
            .children((0..5).map(|i| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(12.))
                    .child(bar([180., 140., 200., 120., 160.][i], t))
                    .child(bar([110., 90., 130., 80., 100.][i], t).opacity(0.6))
            }))
            .into_any_element()
    }

    /// A thread's row; `conn`: its connection's name, when the list mixes
    /// several.
    #[allow(clippy::too_many_arguments)]
    fn row(&self, th: &Thread, conn: Option<String>, m: &Manifest, cursor: bool, now: chrono::DateTime<chrono::Utc>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.as_deref() == Some(th.id.as_str());
        let unread = th.unread > 0;
        let who = th.from.as_ref().map(|f| crate::extensions::agent_of(m, &self.agents, &th.id, f, cx).map_or_else(|| f.agent.clone(), |a| a.name.clone()));
        let time = when(&th.last_activity, now);
        let id = th.id.clone();
        let title = if th.title.is_empty() { "(untitled)".to_string() } else { th.title.clone() };
        div()
            .id(SharedString::from(format!("inbox-row-{}", th.id)))
            .debug_selector({
                let id = th.id.clone();
                move || format!("inbox-row-{id}")
            })
            .relative()
            .flex()
            .gap(px(8.))
            .pl(px(18.))
            .pr(px(10.))
            .py(px(9.))
            .mx(px(6.))
            .rounded(RADIUS)
            .cursor_pointer()
            .map(|d| if selected { d.bg(t.surface_3) } else { d.hover(|s| s.bg(t.surface_2)) })
            .when(cursor && selected, |d| d.border_1().border_color(t.text.opacity(0.22)))
            .tooltip(kit::tooltip(title.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus, cx);
                this.show(&id, true, cx);
            }))
            // Unread: a dot in the margin.
            .when(unread, |d| d.child(div().absolute().left(px(7.)).top(px(15.)).size(px(6.)).rounded_full().bg(if th.blocking { t.amber } else { t.text })))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .ellipsis()
                                    .text_size(px(13.))
                                    .font_weight(if unread { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                    .text_color(if unread { t.text } else { t.text_2 })
                                    .child(kit::one_line(title)),
                            )
                            .when_some(time.clone(), |d, (short, _)| d.child(div().flex_none().text_size(px(11.)).text_color(t.text_3).child(short))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().flex_1().min_w_0().ellipsis().text_size(px(12.)).text_color(t.text_3).child(kit::one_line(who.unwrap_or_default())))
                            .when_some(conn, |d, c| {
                                let id = th.id.clone();
                                d.child(
                                    div()
                                        .debug_selector(move || format!("inbox-row-conn-{id}"))
                                        .flex_none()
                                        .max_w(px(110.))
                                        .ellipsis()
                                        .px(px(5.))
                                        .rounded(px(4.))
                                        .bg(t.surface_2)
                                        .text_size(px(11.))
                                        .text_color(t.text_3)
                                        .child(kit::one_line(c)),
                                )
                            })
                            .when(th.blocking, |d| d.child(div().flex_none().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(t.amber).child("waiting")))
                            .when(th.open_asks > 0, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .min_w(px(18.))
                                        .h(px(18.))
                                        .px(px(5.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(if th.blocking { t.amber } else { t.surface_3 })
                                        .text_color(if th.blocking { kit::on_primary(t) } else { t.text })
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(th.open_asks.to_string()),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn list(&self, m: &Manifest, st: &hub::ExtState, focused: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let list = self.listed(cx);
        let now = chrono::Utc::now();
        let several = st.connections.len() > 1;
        let scope = self.connection(cx);
        // Several connections: a line for each one listed that is offline
        // (the others' threads stay); one: what its adapter said.
        let notes: Vec<(String, String)> = if several {
            st.connections.iter().filter(|v| v.offline() && scope.as_ref().is_none_or(|c| *c == v.id)).map(|v| (v.id.clone(), offline_note(v))).collect()
        } else {
            vec![]
        };
        let banner = match &st.conn {
            _ if several => None,
            Some((ConnState::Error | ConnState::SignedOut, msg)) if !msg.is_empty() => Some(msg.clone()),
            Some((ConnState::Connecting, _)) => Some("Reconnecting…".to_string()),
            _ => None,
        };
        // Rows name their connection while the list mixes several.
        let label = several && scope.is_none();
        let body: AnyElement = if list.is_empty() {
            let all = st.threads_of(scope.as_deref()).count();
            calm("check", "Nothing needs you", if all > 0 { format!("{all} other thread{} in All.", if all == 1 { "" } else { "s" }) } else { "Questions your agents ask will show here.".into() }, t)
                .debug_selector(|| "inbox-empty".into())
                .when(all > 0, |d| d.child(kit::text_button("inbox-show-all", "Show all", kit::BtnKind::Ghost, false, t).on_click(cx.listener(|this, _, _, cx| this.set_filter(Filter::All, cx)))))
                .into_any_element()
        } else {
            div()
                .id("inbox-list")
                .track_scroll(&self.list_scroll)
                .overflow_y_scroll()
                .flex_1()
                .min_h_0()
                .py(px(6.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .children(list.iter().map(|th| self.row(th, st.label(&th.id).filter(|_| label).map(String::from), m, focused, now, t, cx)))
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when_some(banner, |d, b| d.child(div().px(px(14.)).py(px(6.)).text_size(px(12.)).text_color(t.text_2).bg(t.surface_2).child(b)))
            .children(notes.into_iter().map(|(id, text)| {
                div()
                    .debug_selector(move || format!("inbox-offline-{id}"))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(14.))
                    .py(px(6.))
                    .bg(t.surface_2)
                    .child(div().flex_none().size(px(6.)).rounded_full().bg(t.amber))
                    .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(t.text_2).child(text))
            }))
            .child(body)
            .into_any_element()
    }

    /// The thread shown, or a hint to pick one.
    fn thread(&mut self, m: &Manifest, st: &hub::ExtState, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(id) = self.selected.clone() else {
            return calm("inbox", "Pick a thread", "Or press ↓ to go through them.".into(), t).into_any_element();
        };
        let Some(d) = st.details.get(&id).cloned() else {
            let gone = st.listed() && !st.threads.iter().any(|x| x.id == id);
            if gone {
                return calm("check", "This thread is gone", "It was archived or deleted.".into(), t).into_any_element();
            }
            return div().flex_1().p(px(24.)).flex().flex_col().gap(px(10.)).child(bar(260., t)).child(bar(200., t)).child(bar(320., t)).into_any_element();
        };
        let note = st.notes.get(&id).cloned();
        let busy = st.busy.contains(&id);
        let narrow = self.narrow();
        let who = d.from.as_ref().and_then(|f| crate::extensions::agent_of(m, &self.agents, &d.id, f, cx).cloned());
        let from_name = d.from.as_ref().map(|f| who.as_ref().map_or_else(|| f.agent.clone(), |a| a.name.clone()));
        let now = chrono::Utc::now();
        let archive_id = id.clone();
        let head = div()
            .flex_none()
            .flex()
            .items_start()
            .gap(px(8.))
            .px(px(20.))
            .pt(px(14.))
            .pb(px(12.))
            .border_b_1()
            .border_color(if t.is_glass() { t.g.line } else { t.line })
            .when(narrow, |d| d.child(kit::icon_btn("inbox-back", "back", "Back to the list (Esc)", true, t).on_click(cx.listener(|this, _, _, cx| this.back(cx)))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(if d.title.is_empty() { "(untitled)".to_string() } else { d.title.clone() }))
                    .when_some(from_name.clone(), |el, n| el.child(div().flex().child(self.name_chip(&n, who.as_ref(), t)))),
            )
            .child(kit::text_button("inbox-archive", "Archive", kit::BtnKind::GhostSm, false, t).tooltip(kit::tooltip("Archive (E)")).on_click(cx.listener(move |this, _, _, cx| this.archive(&archive_id, cx))));
        let mut items: Vec<AnyElement> = Vec::new();
        if d.earlier {
            items.push(div().text_size(px(12.)).text_color(t.text_3).child("Earlier messages aren't shown here.").into_any_element());
        }
        for msg in d.messages.iter().filter(|msg| !said_already(&d, msg)) {
            items.push(self.message(&d, msg, m, who.as_ref(), busy, now, t, cx));
        }
        if let Some((thread, _, text)) = self.replying.clone().filter(|(th, _, _)| *th == id) {
            let _ = thread;
            items.push(self.bubble_you(&text, Some("Sending…"), t));
        }
        let body = div()
            .id("inbox-thread")
            .track_scroll(&self.thread_scroll)
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .child(div().w_full().max_w(px(READING + 40.)).px(px(20.)).pt(px(16.)).pb(px(24.)).flex().flex_col().gap(px(20.)).children(items));
        let reply = d.can_reply.then(|| {
            div()
                .flex_none()
                .w_full()
                .flex()
                .border_t_1()
                .border_color(if t.is_glass() { t.g.line } else { t.line })
                .child(
                    div()
                        .w_full()
                        .max_w(px(READING + 40.))
                        .px(px(20.))
                        .py(px(12.))
                        .flex()
                        .items_end()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().child(self.reply.clone()))
                        .child(kit::text_button("inbox-send", "Send", kit::BtnKind::Primary, busy, t).tooltip(kit::tooltip("Send (⌘↵)")).on_click(cx.listener(|this, _, _, cx| this.send_reply(cx)))),
                )
        });
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(head)
            .when_some(note, |el, n| {
                el.child(
                    div()
                        .debug_selector(|| "inbox-note".into())
                        .flex_none()
                        .mx(px(16.))
                        .mt(px(10.))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(RADIUS)
                        .bg(t.red_soft)
                        .text_size(px(12.))
                        .text_color(t.text)
                        .child(format!("Not sent: {n}")),
                )
            })
            .child(body)
            .children(reply)
            .into_any_element()
    }

    /// An agent's name in its Pitwall look: its kind's logo and its name.
    fn name_chip(&self, name: &str, agent: Option<&AgentView>, t: &Theme) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(20.))
            .px(px(7.))
            .rounded_full()
            // A tint of the text, so it shows on light and dark alike.
            .bg(t.text.opacity(0.07))
            .max_w(px(260.))
            .when_some(agent, |d, a| d.child(kit::agent_logo(&a.kind, &a.kind_name, 12., t)))
            .child(div().min_w_0().ellipsis().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text_2).child(kit::one_line(name.to_string())))
            .into_any_element()
    }

    fn bubble_you(&self, text: &str, status: Option<&str>, t: &Theme) -> AnyElement {
        div()
            .flex()
            .justify_end()
            .child(
                div()
                    .max_w(relative_w(0.85))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .bg(t.surface_3)
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(t.text)
                    .child(text.to_string())
                    .when_some(status, |d, s| d.child(div().mt(px(2.)).text_size(px(11.)).text_color(t.text_3).child(s.to_string()))),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn message(&mut self, d: &ThreadDetail, msg: &Message, m: &Manifest, who: Option<&AgentView>, busy: bool, now: chrono::DateTime<chrono::Utc>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let time = when(&msg.time, now);
        let you = msg.author.kind == AuthorKind::You;
        let author = match msg.author.kind {
            AuthorKind::Agent => self.name_chip(&who.map_or_else(|| msg.author.name.clone(), |a| a.name.clone()), who, t),
            AuthorKind::Person => div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text_2).child(msg.author.name.clone()).into_any_element(),
            AuthorKind::You => div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text_2).child("You").into_any_element(),
        };
        let stamp = time.map(|(short, exact)| {
            div()
                .id(SharedString::from(format!("inbox-time-{}", msg.id)))
                .text_size(px(11.))
                .text_color(t.text_3)
                .tooltip(kit::tooltip(exact))
                .child(short)
        });
        let text = (!msg.text.is_empty()).then(|| self.markdown(format!("{}:{}:t", d.id, msg.id), &msg.text, cx));
        let open_detail = self.expanded.contains(&format!("{}:{}", d.id, msg.id));
        let detail = (open_detail && !msg.detail.is_empty()).then(|| self.markdown(format!("{}:{}:d", d.id, msg.id), &msg.detail, cx));
        let mid = format!("{}:{}", d.id, msg.id);
        let toggle = (!msg.detail.is_empty()).then(|| {
            div()
                .id(SharedString::from(format!("inbox-details-{}", msg.id)))
                .debug_selector({ let id = msg.id.clone(); move || format!("inbox-details-{id}") })
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text_3)
                .cursor_pointer()
                .hover(|s| s.text_color(t.text))
                .child(kit::chevron_at(open_detail, 10., t.text_3))
                .child(if open_detail { "Hide details" } else { "Show details" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&mid) {
                        this.expanded.insert(mid.clone());
                    }
                    cx.notify();
                }))
        });
        let asks: Vec<AnyElement> = msg.asks.iter().map(|a| self.ask(&d.id, a, busy, t, cx)).collect();
        let shown = |n: u32| asked_before(d, msg, n);
        let carried: Vec<AnyElement> = msg
            .answers
            .iter()
            .filter(|a| !shown(a.ask))
            .map(|a| {
                let said = if a.declined { "Declined".to_string() } else { [a.labels.join(", "), a.text.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — ") };
                quiet("check", &format!("{} → {said}", a.question), false, t)
            })
            .chain(msg.withdrawals.iter().filter(|w| !shown(w.ask)).map(|w| quiet("x", &format!("Question {} withdrawn{}", w.ask, if w.reason.is_empty() { String::new() } else { format!(": {}", w.reason) }), true, t)))
            .collect();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_w_0()
            .children(text)
            .children(toggle)
            .children(detail.map(|v| div().pl(px(10.)).border_l_2().border_color(t.line_strong).child(v)))
            .children(carried);
        let _ = m;
        div()
            .debug_selector({
                let id = msg.id.clone();
                move || format!("inbox-msg-{id}")
            })
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().flex().items_center().gap(px(8.)).when(you, |d| d.justify_end()).child(author).children(stamp))
            .child(if you { div().flex().justify_end().child(div().max_w(relative_w(0.85)).px(px(12.)).py(px(8.)).rounded(px(10.)).bg(t.surface_3).child(body)) } else { div().child(body) })
            .children(asks)
            .into_any_element()
    }

    fn ask(&mut self, thread: &str, a: &Ask, busy: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = (thread.to_string(), a.number);
        if let Some(s) = self.sent.get(&key) {
            let said = if s.declined { "Declined".to_string() } else { s.labels.iter().cloned().chain(s.text.clone()).collect::<Vec<_>>().join(" — ") };
            return quiet("check", &format!("{} → {said}", a.question), false, t);
        }
        if let Some(why) = &a.withdrawn {
            return quiet("x", &format!("{} — withdrawn{}", a.question, if why.is_empty() { String::new() } else { format!(": {why}") }), true, t);
        }
        if let Some(ans) = &a.answer {
            let said = if ans.declined {
                "Declined".to_string()
            } else {
                ans.picks.iter().filter_map(|i| a.suggestions.get(*i as usize)).map(|s| s.label.clone()).chain((!ans.text.is_empty()).then(|| ans.text.clone())).collect::<Vec<_>>().join(" — ")
            };
            return quiet(if ans.declined { "x" } else { "check" }, &format!("{} → {said}", a.question), ans.declined, t);
        }
        if !a.open {
            return quiet("x", &format!("{} — closed", a.question), true, t);
        }
        let picked = self.picks.get(&key).cloned().unwrap_or_default();
        let other = self.other_input(thread, a, cx);
        let described = a.suggestions.iter().any(|s| !s.description.is_empty());
        let accent = if a.blocking { t.amber } else { t.line_strong };
        let options = a.suggestions.iter().enumerate().map(|(i, s)| {
            let on = picked.contains(&(i as u32));
            let (th, ask) = (thread.to_string(), a.clone());
            let num = (i < 9).then(|| kit::kbd(&(i + 1).to_string(), t));
            let base = div()
                .id(SharedString::from(format!("inbox-pick-{thread}-{}-{i}", a.number)))
                .debug_selector({
                    let n = a.number;
                    move || format!("inbox-pick-{n}-{i}")
                })
                .cursor_pointer()
                .border_1()
                .border_color(if on { t.text } else { t.line_strong })
                .bg(if on { t.surface_3 } else { t.surface })
                .hover(|s| s.bg(t.surface_2).border_color(t.text_3))
                .when(busy, |d| d.opacity(0.5))
                .on_click(cx.listener(move |this, _, _, cx| this.pick(&th, &ask, i as u32, cx)));
            if described {
                base.w_full()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(8.))
                    .children(num)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(s.label.clone()))
                            .when(!s.description.is_empty(), |d| d.child(div().text_size(px(12.)).line_height(px(17.)).text_color(t.text_3).child(s.description.clone()))),
                    )
                    .when(on, |d| d.child(kit::icon("check", 14., t.text)))
                    .into_any_element()
            } else {
                base.flex()
                    .items_center()
                    .gap(px(7.))
                    .h(px(30.))
                    .px(px(12.))
                    .rounded_full()
                    .children(num)
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(s.label.clone()))
                    .when(on, |d| d.child(kit::icon("check", 13., t.text)))
                    .into_any_element()
            }
        });
        let (th_send, ask_send, other_send) = (thread.to_string(), a.clone(), other.clone());
        let (th_dec, ask_dec) = (thread.to_string(), a.clone());
        let send_label = if a.multiple && !picked.is_empty() { format!("Send {} picked", picked.len()) } else { "Send".to_string() };
        div()
            .debug_selector({
                let n = a.number;
                move || format!("inbox-ask-{n}")
            })
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(12.))
            .border_1()
            .border_color(accent)
            .bg(if a.blocking { t.amber_soft } else { t.raised })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(a.blocking, |d| {
                        d.child(div().flex().items_center().gap(px(5.)).child(div().size(px(7.)).rounded_full().bg(t.amber)).child(kit::tracked("WAITING ON YOU", 11., 0.06).font_weight(FontWeight::SEMIBOLD).text_color(t.amber)))
                    })
                    .when(!a.blocking, |d| d.child(kit::tracked("QUESTION", 11., 0.06).font_weight(FontWeight::SEMIBOLD).text_color(t.text_3)))
                    .when(a.multiple, |d| d.child(div().text_size(px(11.)).text_color(t.text_3).child("· pick any"))),
            )
            .child(div().text_size(px(16.)).line_height(px(23.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(a.question.clone()))
            .when(!a.suggestions.is_empty(), |d| {
                d.child(if described { div().flex().flex_col().gap(px(6.)).children(options).into_any_element() } else { div().flex().flex_wrap().gap(px(8.)).children(options).into_any_element() })
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w(px(200.)).child(other))
                    .child(kit::text_button(SharedString::from(format!("inbox-answer-{thread}-{}", a.number)), send_label, kit::BtnKind::Primary, busy, t).on_click(cx.listener(move |this, _, _, cx| {
                        let text = other_send.read(cx).text().to_string();
                        let picks = this.picks.get(&(th_send.clone(), ask_send.number)).cloned().unwrap_or_default();
                        this.answer(&th_send, &ask_send, picks, Some(text), false, cx);
                    })))
                    .child(
                        kit::text_button(SharedString::from(format!("inbox-decline-{thread}-{}", a.number)), "Decline", kit::BtnKind::Link, busy, t)
                            .tooltip(kit::tooltip("Tell the agent you won't answer this"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.answer(&th_dec, &ask_dec, vec![], None, true, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

/// Whether ask `n` is in a message of `d` before `msg` (where its answer
/// or withdrawal already shows).
fn asked_before(d: &ThreadDetail, msg: &Message, n: u32) -> bool {
    d.messages.iter().take_while(|m| m.id != msg.id).flat_map(|m| &m.asks).any(|a| a.number == n)
}

/// A message that only repeats what the thread shows: answers to, or
/// withdrawals of, asks drawn above (each ask shows how it went).
pub fn said_already(d: &ThreadDetail, msg: &Message) -> bool {
    msg.text.is_empty()
        && msg.detail.is_empty()
        && msg.asks.is_empty()
        && (!msg.answers.is_empty() || !msg.withdrawals.is_empty())
        && msg.answers.iter().all(|a| asked_before(d, msg, a.ask))
        && msg.withdrawals.iter().all(|w| asked_before(d, msg, w.ask))
}

/// The last lines an adapter printed, as it printed them.
pub(super) fn output_tail(said: &str, t: &Theme) -> AnyElement {
    let lines: Vec<&str> = said.lines().collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    div()
        .mt(px(4.))
        .w_full()
        .max_w(px(560.))
        .px(px(12.))
        .py(px(10.))
        .rounded(RADIUS)
        .bg(t.surface_2)
        .border_1()
        .border_color(t.line)
        .text_left()
        .font_family(kit::MONO_FONT)
        .text_size(px(11.5))
        .line_height(px(17.))
        .text_color(t.text_2)
        .overflow_hidden()
        .child(tail)
        .into_any_element()
}

fn relative_w(f: f32) -> gpui::Length {
    gpui::relative(f).into()
}

/// One quiet line: an ask answered (or one withdrawn or declined: dimmed).
fn quiet(icon: &str, text: &str, dim: bool, t: &Theme) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(7.))
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(if dim { t.text_4 } else { t.text_3 })
        .child(div().mt(px(2.)).child(kit::icon(icon, 12., if dim { t.text_4 } else { t.green })))
        .child(div().flex_1().min_w_0().child(text.to_string()))
        .into_any_element()
}

impl Render for InboxView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let width = self.width.clone();
        let me = cx.entity().downgrade();
        let measure = canvas(
            move |b, _, cx| {
                let w = f32::from(b.size.width);
                if (width.get() - w).abs() > 0.5 {
                    width.set(w);
                    // Drawn again at its width (a view is redrawn only
                    // when it says so).
                    let me = me.clone();
                    cx.defer(move |cx| {
                        let _ = me.update(cx, |v, cx| {
                            v.fill(cx);
                            cx.notify();
                        });
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        // A field let the keys go (Esc), or the ask whose field had them
        // was answered (the field is gone): back to the tile itself.
        let answered = self.answered.take().and_then(|k| self.others.get(&k).cloned());
        if std::mem::take(&mut self.refocus) || answered.is_some_and(|i| i.read(cx).is_focused(window)) {
            window.focus(&self.focus, cx);
        }
        // Its keys apply (the list shows where they are).
        let focused = self.focus.contains_focused(window, cx) && !self.typing(window, cx);
        let root = div()
            .id(SharedString::from(format!("inbox-{}", self.ext)))
            .key_context("InboxTile")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            // A press in the tile gives it the keys, unless they are in it
            // already: a field pressed has just taken them (its own press
            // comes first), and selected text keeps them for copying.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                if !this.focus.contains_focused(window, cx) {
                    window.focus(&this.focus, cx);
                }
            }))
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(measure);
        let menu = self.menu.clone().map(|(at, replace)| self.place_menu(at, replace, &t, cx));
        let root = root.children(menu);
        let Some(m) = self.manifest(cx) else {
            let ext = self.ext.clone();
            return root
                .child(calm("extension", "This extension is gone", "Its folder isn't in Pitwall's extensions folder any more. The tile can be closed.".into(), &t))
                .child(div().flex().justify_center().pb(px(24.)).child(kit::small_btn("inbox-gone-close", "Close", &t).on_click(move |_, _, cx| {
                    let ext = ext.clone();
                    cx.defer(move |cx| close(&ext, cx));
                })))
                .into_any_element();
        };
        let h = hub::hub(cx);
        let hub_ref: &Hub = h.read(cx);
        // The hub's state, copied out (drawing calls back into this view).
        let snapshot = hub_ref.get(&self.ext).map(|s| s.snapshot());
        let header = self.header(&m, snapshot.as_ref(), &t, cx);
        let root = root.child(header);
        if let Some(state) = self.state(&m, snapshot.as_ref(), &t, cx) {
            return root.child(state).into_any_element();
        }
        let Some(st) = snapshot else { return root.into_any_element() };
        if self.narrow() {
            let body = if self.pushed && self.selected.is_some() { self.thread(&m, &st, &t, cx) } else { self.list(&m, &st, focused, &t, cx) };
            return root.child(body).into_any_element();
        }
        let list = self.list(&m, &st, focused, &t, cx);
        let thread = self.thread(&m, &st, &t, cx);
        root.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(div().w(px(300.)).flex_none().min_h_0().flex().flex_col().border_r_1().border_color(if t.is_glass() { t.g.line } else { t.line }).child(list))
                .child(thread),
        )
        .into_any_element()
    }
}

// ── in the window ────────────────────────────────────────────────────────

/// Every inbox view, one per extension, wherever it is drawn (a tile of
/// any Pitwall window's layout, or a window of its own): moving it keeps
/// it, with its thread, its drafts and its picks.
#[derive(Default)]
struct Inboxes(HashMap<String, Entity<InboxView>>);

impl gpui::Global for Inboxes {}

/// Extension `ext`'s inbox view (made when it isn't yet).
pub fn view(ext: &str, cx: &mut App) -> Entity<InboxView> {
    if let Some(v) = get(ext, cx) {
        return v;
    }
    let v = cx.new(|cx| InboxView::new(ext, cx));
    cx.default_global::<Inboxes>().0.insert(ext.to_string(), v.clone());
    v
}

/// Extension `ext`'s inbox view, if it is made.
pub fn get(ext: &str, cx: &App) -> Option<Entity<InboxView>> {
    cx.try_global::<Inboxes>()?.0.get(ext).cloned()
}

/// Answer `ask` of extension `ext`'s thread `thread` from outside its
/// inbox (the floating control), as the inbox answers it: through its view
/// when there is one, so it shows the answer as sent at once too, else
/// straight to the hub. Why it couldn't be sent goes on the thread, as in
/// the inbox. The request, when it went.
pub fn answer_ask(ext: &str, thread: &str, ask: &Ask, picks: Vec<u32>, text: Option<String>, declined: bool, cx: &mut App) -> Option<u64> {
    if let Some(v) = get(ext, cx) {
        return v.update(cx, |v, cx| v.answer(thread, ask, picks, text, declined, cx));
    }
    let h = hub::hub(cx);
    match h.update(cx, |h, cx| h.request(ext, thread, Request::Answer { ask: ask.number, picks, text, declined }, cx)) {
        Ok(req) => Some(req),
        Err(why) => {
            h.update(cx, |h, cx| h.set_note(ext, thread, why, cx));
            None
        }
    }
}

/// Extension `ext`'s inbox closed: out of every layout, its window (if it
/// is in one) closed, its view let go (the thread it showed closes in the
/// adapter; the one shown is kept for next time).
pub fn close(ext: &str, cx: &mut App) {
    crate::ui_state::update(cx, |ui| *ui = ui.clone().close_inbox(ext));
    super::inbox_window::close_for_move(ext, cx);
    if cx.has_global::<Inboxes>() {
        cx.global_mut::<Inboxes>().0.remove(ext);
    }
}

impl MainScreen {
    /// ⌘I, "Open <name> inbox", an agent's ask badge: extension `ext`'s
    /// inbox (the first that has one when `None`) focused where it is (its
    /// window brought forward when it is in one), else placed as a web page
    /// is (right of the focused tile, in it when it is empty); at `thread`
    /// when given.
    pub fn open_inbox(&mut self, ext: Option<&str>, thread: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let all = crate::extensions::inboxes(cx);
        let m = match ext {
            Some(e) => all.into_iter().find(|m| m.id == e),
            None => all.into_iter().next(),
        };
        let Some(m) = m else {
            self.toast(Toast::info("No inbox to open", Some("Turn on an extension that has one in Settings → Extensions.".into())), cx);
            return;
        };
        let v = view(&m.id, cx);
        if let Some(th) = thread {
            v.update(cx, |v, cx| v.show(th, true, cx));
        }
        let f = v.read(cx).focus.clone();
        // In a window of its own: that window, with the keys.
        if let Some(h) = crate::windows::inbox_window(&m.id, cx) {
            cx.defer(move |cx| {
                let _ = h.update(cx, |_, window, cx| {
                    // (A bench instance never comes forward.)
                    if !crate::bench::enabled() {
                        window.activate_window();
                    }
                    window.focus(&f, cx);
                });
            });
            return;
        }
        let what = Content::Inbox(m.id.clone());
        match self.ui.tile_of(&what) {
            Some((space, pane)) if self.ui.window_of_space(&space) == self.label => {
                self.active = space.clone();
                self.set_route(super::Route::Space, cx);
                self.update_ui(|s| s.focus_pane(&space, &pane), cx);
            }
            // Not here (or in another Pitwall window: it moves here).
            _ => {
                if !self.place_inbox(&m.id, cx) {
                    return;
                }
            }
        }
        window.focus(&f, cx);
        cx.notify();
    }

    /// Extension `ext`'s inbox placed in the active space, its tile the
    /// focused one.
    fn place_inbox(&mut self, ext: &str, cx: &mut Context<Self>) -> bool {
        let Some(space) = self.active_space().cloned() else { return false };
        let anchor = space.focused_pane_id.clone().and_then(|p| space.layout.find_pane(&p));
        let spot = match anchor {
            Some(p) if p.is_empty() => FilesSpot::Take(p.id),
            Some(p) => FilesSpot::Tile { pane: p.id, side: super::tree::Side::Right },
            None => FilesSpot::Auto,
        };
        let fit = self.fit_for(&space.id);
        let mut placed = false;
        self.update_ui(
            |s| {
                let (next, at) = s.inbox_to_layout(&space.id, ext, &spot, fit);
                placed = at.is_some();
                match at {
                    Some(pane) => next.focus_pane(&space.id, &pane),
                    None => next,
                }
            },
            cx,
        );
        placed
    }

    /// Extension `ext`'s inbox closed (its tile's close button).
    pub(super) fn close_inbox(&mut self, ext: &str, cx: &mut Context<Self>) {
        close(ext, cx);
        cx.notify();
    }

    /// Tile `pane` showing extension `ext`'s inbox: a tile like the others
    /// (dragged by its bar, Move…, its own window), and like a terminal's
    /// it is the space's focused one when pressed (⌘⏎ maximises it), with
    /// a focused border.
    pub(super) fn render_inbox_pane(&mut self, pane: &str, ext: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = view(ext, cx);
        let agents = self.agents(cx).to_vec();
        v.update(cx, |v, cx| v.held(false, agents, cx));
        let body = div().flex_1().min_w_0().min_h_0().flex().child(v).into_any_element();
        self.tile_shell(pane, Content::Inbox(ext.to_string()), format!("inbox-tile-{ext}"), body, None, t, cx)
    }

    /// The badge on agent `a`'s row: its open asks in an extension (amber
    /// when one blocks it); a click opens the inbox at the oldest. (One
    /// call in `agent_row`: remove it there to drop the badge.)
    pub(super) fn agent_ask_badge(&self, a: &AgentView, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let asks = crate::extensions::asks_for(a, cx)?;
        let tip = format!("{} question{} in {} — open the inbox", asks.asks, if asks.asks == 1 { "" } else { "s" }, asks.name);
        let (ext, thread) = (asks.ext.clone(), asks.thread.clone());
        Some(
            div()
                .id(SharedString::from(format!("ask-badge-{}", a.id)))
                .debug_selector({
                    let id = a.id.clone();
                    move || format!("ask-badge-{id}")
                })
                .flex_none()
                .flex()
                .items_center()
                .gap(px(3.))
                .h(px(18.))
                .px(px(6.))
                .rounded_full()
                .cursor_pointer()
                .bg(if asks.blocking { t.amber } else { t.surface_3 })
                .text_color(if asks.blocking { kit::on_primary(t) } else { t.text })
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .tooltip(kit::tooltip(tip))
                .child(kit::icon("inbox", 11., if asks.blocking { kit::on_primary(t) } else { t.text_2 }))
                .child(asks.asks.to_string())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_inbox(Some(&ext), Some(&thread), window, cx);
                }))
                .into_any_element(),
        )
    }

    /// Bench and debug builds, `inbox …` (`ui::MainView::bench`; nothing
    /// is clicked or typed on the screen): `open [ext]`, `show <thread>`,
    /// `filter all|needs`, `back`, `details <message>`, `pick <n>`,
    /// `reply <text>` (`\n` a new line), `close`; and through GPUI's own
    /// event dispatch, in the window the inbox is in (its tile's or its
    /// own), as real input arrives: `click reply|other` (a press in that
    /// field), `press` (on its bar, as a click in the tile), `drag <x> <y>`
    /// (its bar dragged to there in this window and let go), `key <k>…`
    /// (keystrokes: `cmd-enter`, `escape`, `j`…), `type <text>` (each
    /// character a key); `place right|left|above|below|replace|edge-right|
    /// edge-bottom|own|window [agent]` (Move… / Dock…), and `state` (on
    /// stderr: fields, keys, where it is).
    pub fn debug_inbox(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut words = cmd.split_whitespace();
        let what = words.next().unwrap_or_default();
        let arg = words.next().map(str::to_string);
        if what == "open" {
            self.open_inbox(arg.as_deref(), None, window, cx);
            return;
        }
        let Some(v) = cx.try_global::<Inboxes>().and_then(|i| i.0.values().next().cloned()) else {
            eprintln!("pitwall-debug: no inbox open");
            return;
        };
        let ext = v.read(cx).ext.clone();
        // Input for the window the inbox is in, after this update.
        let send = |input: Vec<gpui::PlatformInput>, keys: Vec<gpui::Keystroke>, window: &mut Window, cx: &mut Context<Self>| {
            let run = move |window: &mut Window, cx: &mut App| {
                for e in input {
                    window.dispatch_event(e, cx);
                }
                for k in keys {
                    window.dispatch_keystroke(k, cx);
                }
            };
            match crate::windows::inbox_window(&ext, cx) {
                Some(h) => cx.defer(move |cx| {
                    let _ = cx.update_window(h.into(), |_, window, cx| run(window, cx));
                }),
                None => window.defer(cx, run),
            }
        };
        let click = |at: Point<Pixels>| {
            let (button, modifiers) = (MouseButton::Left, gpui::Modifiers::default());
            vec![
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent { position: at, button, modifiers, click_count: 1, first_mouse: false }),
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent { position: at, button, modifiers, click_count: 1 }),
            ]
        };
        let keystroke = |k: &str| {
            let mut k = gpui::Keystroke::parse_with_os(k, gpui::OperatingSystem::Mac).ok()?;
            if k.key.chars().count() == 1 && !k.modifiers.modified() {
                k.key_char = Some(k.key.clone());
            }
            Some(k)
        };
        // Its bar's middle (on the name), in its tile here.
        let bar = |this: &Self| {
            let (_, pane) = this.ui.tile_of(&Content::Inbox(v.read(cx).ext.clone()))?;
            let b = this.files_boxes.pane(&pane)?;
            Some(gpui::point(b.left() + px(120.), b.top() + px(20.)))
        };
        match (what, arg) {
            ("place", Some(p)) => {
                let place = match p.as_str() {
                    "right" => FilesPlace::Next(super::tree::Side::Right),
                    "left" => FilesPlace::Next(super::tree::Side::Left),
                    "above" => FilesPlace::Next(super::tree::Side::Top),
                    "below" => FilesPlace::Next(super::tree::Side::Bottom),
                    "edge-right" => FilesPlace::Edge(super::tree::Side::Right),
                    "edge-bottom" => FilesPlace::Edge(super::tree::Side::Bottom),
                    "own" => FilesPlace::Own,
                    "window" => FilesPlace::Window,
                    "replace" => FilesPlace::Replace,
                    _ => return eprintln!("pitwall-debug: inbox no place {p}"),
                };
                let of = words.next().map(str::to_string);
                cx.defer(move |cx| match super::inbox_window::place(&ext, place, of.as_deref(), cx) {
                    Ok(at) => eprintln!("pitwall-debug: inbox placed: {at}"),
                    Err(e) => eprintln!("pitwall-debug: inbox not placed: {e}"),
                });
            }
            ("key", Some(first)) => {
                let keys: Vec<gpui::Keystroke> = std::iter::once(first.as_str()).chain(words).filter_map(keystroke).collect();
                send(vec![], keys, window, cx);
            }
            ("type", Some(_)) => {
                let text = cmd.trim_start().strip_prefix("type").unwrap_or_default().trim_start().to_string();
                let keys: Vec<gpui::Keystroke> = text.chars().filter_map(|c| keystroke(&if c == ' ' { "space".to_string() } else { c.to_string() })).collect();
                send(vec![], keys, window, cx);
            }
            ("press", _) => match bar(self) {
                Some(at) => send(click(at), vec![], window, cx),
                None => eprintln!("pitwall-debug: inbox has no tile here"),
            },
            ("drag", Some(x)) => {
                let (Some(from), Ok(x), Some(Ok(y))) = (bar(self), x.parse::<f32>(), words.next().map(str::parse::<f32>)) else {
                    return eprintln!("pitwall-debug: inbox drag <x> <y> (from its tile here)");
                };
                let to = gpui::point(px(x), px(y));
                let (button, modifiers) = (MouseButton::Left, gpui::Modifiers::default());
                let mv = |at: Point<Pixels>| gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent { position: at, pressed_button: Some(button), modifiers });
                let mid = gpui::point((from.x + to.x) / 2., (from.y + to.y) / 2.);
                let input = vec![
                    gpui::PlatformInput::MouseDown(gpui::MouseDownEvent { position: from, button, modifiers, click_count: 1, first_mouse: false }),
                    mv(gpui::point(from.x + px(10.), from.y + px(10.))),
                    mv(mid),
                    mv(to),
                    mv(to),
                    gpui::PlatformInput::MouseUp(gpui::MouseUpEvent { position: to, button, modifiers, click_count: 1 }),
                ];
                send(input, vec![], window, cx);
            }
            ("state", _) => {
                let in_window = crate::windows::inbox_window(&ext, cx);
                let at = super::inbox_window::described(&ext, cx);
                let ui = crate::ui_state::get(cx);
                let tile = ui.tile_of(&Content::Inbox(ext.clone()));
                let space = tile.as_ref().and_then(|(s, _)| ui.space(s));
                let (focused, maximized) = (space.and_then(|s| s.focused_pane_id.clone()), space.and_then(|s| s.maximized_pane_id.clone()));
                let report = move |window: &mut Window, cx: &mut App| {
                    let v = v.read(cx);
                    let fields: Vec<String> = std::iter::once(("reply".to_string(), v.reply.clone()))
                        .chain(v.others.iter().map(|((t, a), i)| (format!("other {t} {a}"), i.clone())))
                        .map(|(name, i)| {
                            let i = i.read(cx);
                            format!("{name} {:?}{}", i.text(), if i.is_focused(window) { " (focused)" } else { "" })
                        })
                        .collect();
                    eprintln!(
                        "pitwall-debug: inbox {}: selected {:?}, tile focused {}, fields [{}], filter {:?}, at {at}, tile {tile:?}, space's focused {focused:?}, maximized {maximized:?}, ui window {}",
                        v.ext,
                        v.selected,
                        v.focus.is_focused(window),
                        fields.join("; "),
                        v.filter,
                        ui.extension(&v.ext).window
                    );
                };
                match in_window {
                    Some(h) => {
                        let _ = h.update(cx, |_, window, cx| report(window, cx));
                    }
                    None => report(window, cx),
                }
            }
            (what, arg) => self.debug_inbox_view(&v, what, arg, cmd, window, cx),
        }
        cx.notify();
    }

    /// [`MainScreen::debug_inbox`]'s commands on the view itself.
    fn debug_inbox_view(&mut self, v: &Entity<InboxView>, what: &str, arg: Option<String>, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        match (what, arg) {
            ("show", Some(t)) => v.update(cx, |v, cx| v.show(&t, true, cx)),
            ("filter", Some(f)) => v.update(cx, |v, cx| v.set_filter(if f == "all" { Filter::All } else { Filter::NeedsYou }, cx)),
            ("back", _) => v.update(cx, |v, cx| v.back(cx)),
            // `connection all|<id>`, `connection menu`: which connections
            // are listed, or the menu open.
            ("connection", Some(c)) if c == "menu" => v.update(cx, |v, cx| {
                v.conn_menu = true;
                cx.notify();
            }),
            ("connection", Some(c)) => v.update(cx, |v, cx| v.set_connection((c != "all").then_some(c), cx)),
            ("details", Some(m)) => v.update(cx, |v, cx| {
                let key = format!("{}:{m}", v.selected.clone().unwrap_or_default());
                v.expanded.insert(key);
                cx.notify();
            }),
            ("pick", Some(n)) => v.update(cx, |v, cx| {
                if let (Some((thread, ask)), Ok(n)) = (v.first_open(cx), n.parse::<u32>()) {
                    v.pick(&thread, &ask, n.saturating_sub(1), cx);
                }
            }),
            ("reply", Some(_)) => {
                // The rest of the command, `\n` for a new line.
                let text = cmd.trim_start().strip_prefix("reply").unwrap_or_default().trim().replace("\\n", "\n");
                let reply = v.read(cx).reply.clone();
                reply.update(cx, |r, cx| r.replace_text(text, cx));
            }
            ("close", _) => {
                let ext = v.read(cx).ext.clone();
                self.close_inbox(&ext, cx);
            }
            // A left click in the middle of a field, through GPUI's own
            // mouse dispatch (as a real click comes in), to type in next
            // with `type …` or `key …`.
            ("click", Some(which)) => {
                let Some(field) = v.read(cx).debug_field(&which) else { return eprintln!("pitwall-debug: inbox has no {which} field") };
                let Some(b) = field.read(cx).drawn_bounds() else { return eprintln!("pitwall-debug: inbox {which} field isn't drawn") };
                let position = b.center();
                let run = move |window: &mut Window, cx: &mut App| {
                    let (button, modifiers) = (MouseButton::Left, gpui::Modifiers::default());
                    window.dispatch_event(gpui::PlatformInput::MouseDown(gpui::MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false }), cx);
                    window.dispatch_event(gpui::PlatformInput::MouseUp(gpui::MouseUpEvent { position, button, modifiers, click_count: 1 }), cx);
                };
                match crate::windows::inbox_window(&v.read(cx).ext.clone(), cx) {
                    Some(h) => cx.defer(move |cx| {
                        let _ = cx.update_window(h.into(), |_, window, cx| run(window, cx));
                    }),
                    None => window.defer(cx, run),
                }
            }
            _ => eprintln!("pitwall-debug: inbox {cmd}?"),
        }
    }

    /// Tests: extension `ext`'s inbox view.
    #[cfg(test)]
    pub fn inbox(&self, ext: &str, cx: &App) -> Option<Entity<InboxView>> {
        get(ext, cx)
    }
}

/// Settings opens next at its Extensions page.
pub(super) fn to_extensions_page(cx: &mut gpui::App) {
    crate::settings::data::set_page(cx, pitwall_proto::settings::Page::Extensions);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::extensions::protocol::From as Who;

    /// "Open Settings" in the tile opens Settings at Extensions.
    #[gpui::test]
    fn open_settings_goes_to_the_extensions_page(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::ui_state::store(cx);
            assert_eq!(crate::settings::data::page_of(&crate::ui_state::get(cx)), pitwall_proto::settings::Page::General);
            to_extensions_page(cx);
            assert_eq!(crate::settings::data::page_of(&crate::ui_state::get(cx)), pitwall_proto::settings::Page::Extensions);
        });
    }

    fn th(id: &str, asks: u32, blocking: bool, last: &str) -> Thread {
        Thread { id: id.into(), title: id.into(), from: Some(Who { agent: "a".into(), session: "s".into() }), unread: 0, open_asks: asks, blocking, last_activity: last.into() }
    }

    /// Blocking first, then those that ask, then the latest; Needs you
    /// keeps those that ask; archiving ones are out.
    #[test]
    fn threads_are_listed_needs_you_first() {
        let all = vec![th("old-ask", 1, false, "2026-01-01T00:00:00Z"), th("new", 0, false, "2026-01-05T00:00:00Z"), th("block", 1, true, "2025-12-01T00:00:00Z"), th("new-ask", 2, false, "2026-01-03T00:00:00Z")];
        let ids = |v: Vec<Thread>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(ordered(&all, Filter::All, &HashSet::new())), ["block", "new-ask", "old-ask", "new"]);
        assert_eq!(ids(ordered(&all, Filter::NeedsYou, &HashSet::new())), ["block", "new-ask", "old-ask"]);
        let hidden: HashSet<String> = ["block".to_string()].into();
        assert_eq!(ids(ordered(&all, Filter::NeedsYou, &hidden)), ["new-ask", "old-ask"]);
    }

    #[test]
    fn times_are_short_with_the_exact_one_kept() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-03-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let s = |iso: &str| when(iso, now).map(|(s, _)| s);
        assert_eq!(s("2026-03-10T11:59:30Z").as_deref(), Some("now"));
        assert_eq!(s("2026-03-10T11:55:00Z").as_deref(), Some("5m"));
        assert_eq!(s("2026-03-10T09:00:00Z").as_deref(), Some("3h"));
        assert_eq!(s("2026-03-08T12:00:00Z").as_deref(), Some("2d"));
        assert!(s("2026-01-02T12:00:00Z").is_some_and(|x| x.starts_with("Jan")));
        assert!(when("2026-03-10T11:55:00Z", now).is_some_and(|(_, exact)| exact.contains("2026")));
        assert_eq!(when("yesterday", now), None);
    }
}
