//! One of an extension's views in a tile (docs/extensions/views.md), a
//! peer of the inbox tile: opened with "Open <view>" in the command
//! palette (or a badge, a note or a notification that names it), placed,
//! moved, maximised and put in a window of its own as the inbox is
//! ([`super::inbox_window`], which takes either), and back after a
//! restart (`ui.json`: the pane's `view`, `<extension>/<view>`; a window's
//! in `extensions.<id>.viewWindows`).
//!
//! Pitwall draws what the adapter sends, the whole view each time: a list
//! (sections of rows: an icon, a title, a line under it, small badges, a
//! time) with the selected row's detail beside it, a detail (fields and
//! Markdown), Markdown, or a status (empty, loading, error). Its buttons
//! send an `action` to the adapter; the adapter answers and sends the view
//! again. A button Pitwall runs itself (open a file or a page, focus an
//! agent) is called as the extension, behind its permission
//! ([`hub::Hub::press`]); one whose permission isn't granted is drawn
//! disabled, saying which. A row about an agent names it (with
//! `agents.read`); a click on the name focuses it (with `agents.focus`).
//! A view it can't draw (too big, a form or a table) says why in
//! the tile. Keys while the tile has the keyboard: ↑↓ or J K, Enter (the
//! selected row's main button).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{div, prelude::*, px, AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla, KeyDownEvent, MouseButton, Pixels, Point, ScrollHandle, SharedString, Subscription, Window};
use pitwall_core::extensions::view::{Action, Detail, Item, Pane, Root, Status, StatusKind, Tone};
use pitwall_core::extensions::Manifest;

use super::files_tile::TileDrag;
use super::inbox_tile::{calm, output_tail, to_extensions_page, when};
use super::strip::Toast;
use super::tree::{Content, Surface};
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::explorer::dock::FilesPlace;
use crate::extensions::hub::{self, Run};
use crate::kit::{self, Ellipsis};
use crate::markdown_view::MarkdownView;
use crate::theme::{theme, Theme, RADIUS};

/// The list's width beside a detail.
const LIST_W: f32 = 340.;

type PlaceMenu = (Point<Pixels>, Vec<(String, String)>);

/// What the hub has for a view: how its extension runs, the view (none
/// before it is sent), why its last action failed, and the actions in
/// flight (view, item).
type Data = (Run, Option<hub::views::ViewData>, Option<String>, Vec<(String, String)>);

pub struct ViewTile {
    /// `<extension>/<view>`.
    key: String,
    ext: String,
    view: String,
    focus: FocusHandle,
    /// The row selected (its id as views know it).
    selected: Option<String>,
    list_scroll: ScrollHandle,
    md: HashMap<String, Entity<MarkdownView>>,
    in_window: bool,
    pinned: bool,
    menu: Option<PlaceMenu>,
    /// Why a button press didn't reach the adapter.
    error: Option<String>,
    _subs: Vec<Subscription>,
}

impl Focusable for ViewTile {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A tone's colour.
fn tone(tone: Tone, t: &Theme) -> Hsla {
    match tone {
        Tone::None => t.text_3,
        Tone::Good => t.green,
        Tone::Bad => t.red,
        Tone::Warn => t.amber,
        Tone::Busy => t.focus,
    }
}

/// An icon the adapter named, if this Pitwall has it.
pub(crate) fn icon_or(name: Option<&str>, fallback: &str) -> String {
    name.filter(|n| kit::has_icon(n)).unwrap_or(fallback).to_string()
}

/// The buttons a press on `item` (none: the view's own) can be of, each
/// with the agent its row is about.
pub(crate) fn buttons_of(root: &Root, item: Option<&str>) -> Vec<(Action, Option<String>)> {
    let mut out = Vec::new();
    match root {
        Root::List { list, detail } => {
            if let Some(i) = item.and_then(|id| list.item(id)) {
                out.extend(i.actions.iter().map(|a| (a.clone(), i.agent.clone())));
            }
            let agent = item.and_then(|id| list.item(id)).and_then(|i| i.agent.clone());
            match detail {
                Some(Pane::Detail(d)) => out.extend(d.actions.iter().map(|a| (a.clone(), agent.clone()))),
                Some(Pane::Status(s)) => out.extend(s.actions.iter().map(|a| (a.clone(), agent.clone()))),
                _ => {}
            }
        }
        Root::Pane(Pane::Detail(d)) => out.extend(d.actions.iter().map(|a| (a.clone(), None))),
        Root::Pane(Pane::Status(s)) => out.extend(s.actions.iter().map(|a| (a.clone(), None))),
        Root::Pane(Pane::Markdown(_)) => {}
    }
    out
}

/// Why button `a` of extension `ext` can't be pressed now: the
/// permission it needs and isn't granted.
pub(crate) fn blocked(ext: &str, a: &Action, cx: &App) -> Option<String> {
    let need = a.needs()?;
    let reg = cx.try_global::<crate::extensions::calls::Api>()?.registry.clone();
    (!reg.allows(ext, need)).then(|| format!("Needs the {need} permission (Settings → Extensions)"))
}

/// The name of agent `id` (with `agents.read`): what a row about it shows.
pub(crate) fn agent_name(ext: &str, id: &str, cx: &App) -> Option<String> {
    let reg = cx.try_global::<crate::extensions::calls::Api>()?.registry.clone();
    if !reg.allows(ext, pitwall_core::extensions::permissions::AGENTS_READ) {
        return None;
    }
    let engine = cx.try_global::<super::EngineHandle>()?.0.clone();
    engine.views().into_iter().find(|v| v.id == id).map(|v| v.name)
}

impl ViewTile {
    /// View `key`'s tile. Not tied to a window: one is drawn wherever the
    /// view is ([`view`]). While it lives, its adapters keep it current.
    pub fn new(key: &str, cx: &mut Context<Self>) -> Self {
        let s = Surface::parse(key);
        let (ext, view) = match &s {
            Surface::View(e, v) => (e.clone(), v.clone()),
            Surface::Inbox(e) => (e.clone(), String::new()),
            Surface::Settings | Surface::Device(_) | Surface::Editor => (String::new(), String::new()),
        };
        let h = hub::hub(cx);
        h.update(cx, |h, cx| h.show_view(&ext, &view, cx));
        let mut subs = vec![cx.observe(&h, |this, _, cx| {
            this.settle(cx);
            cx.notify();
        })];
        subs.push(cx.on_release(|this: &mut ViewTile, cx: &mut App| {
            if let Some(h) = hub::try_hub(cx) {
                let (ext, view) = (this.ext.clone(), this.view.clone());
                h.update(cx, |h, _| h.hide_view(&ext, &view));
            }
        }));
        ViewTile {
            key: key.to_string(),
            ext,
            view,
            focus: cx.focus_handle(),
            selected: None,
            list_scroll: ScrollHandle::new(),
            md: HashMap::new(),
            in_window: false,
            pinned: false,
            menu: None,
            error: None,
            _subs: subs,
        }
    }

    /// Who draws it now (a window of its own, or a tile).
    pub fn held(&mut self, in_window: bool, cx: &mut Context<Self>) {
        if self.in_window != in_window {
            self.in_window = in_window;
            self.menu = None;
            cx.notify();
        }
    }

    fn manifest(&self, cx: &App) -> Option<Manifest> {
        crate::extensions::manifest_of(&self.ext, cx)
    }

    /// What the adapters sent, copied out (drawing calls back into this).
    fn data(&self, cx: &App) -> Data {
        let Some(h) = hub::try_hub(cx) else { return (Run::Off, None, None, vec![]) };
        let Some(st) = h.read(cx).get(&self.ext) else { return (Run::Off, None, None, vec![]) };
        let busy = st.view_busy.iter().filter(|(v, _)| *v == self.view).cloned().collect();
        (st.run.clone(), st.views.get(&self.view).cloned(), st.view_notes.get(&self.view).cloned(), busy)
    }

    /// The rows of its list, in order.
    fn items(&self, cx: &App) -> Vec<Item> {
        match self.data(cx).1 {
            Some(Ok(Root::List { list, .. })) => list.items().cloned().collect(),
            _ => vec![],
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// A selected row that is gone isn't selected.
    fn settle(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &self.selected {
            if matches!(self.data(cx).1, Some(Ok(Root::List { .. }))) && !self.items(cx).iter().any(|i| i.id == *s) {
                self.selected = None;
            }
        }
    }

    /// Select row `id`: the adapter is told (it sends the detail beside).
    pub fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(id) {
            return;
        }
        self.selected = Some(id.to_string());
        let (ext, view, id) = (self.ext.clone(), self.view.clone(), id.to_string());
        hub::hub(cx).update(cx, |h, _| h.select(&ext, &view, &id));
        cx.notify();
    }

    /// Button `action` pressed (on row `item`, or the view's own).
    pub fn act(&mut self, item: Option<&str>, action: &str, cx: &mut Context<Self>) {
        // The button as drawn: the row's, else the view's own.
        let found = match self.data(cx).1 {
            Some(Ok(root)) => buttons_of(&root, item).into_iter().find(|(a, _)| a.id == action),
            _ => None,
        };
        let (ext, view) = (self.ext.clone(), self.view.clone());
        let res = match found {
            Some((a, agent)) => hub::hub(cx).update(cx, |h, cx| h.press(&ext, &view, item, agent.as_deref(), &a, cx)),
            None => hub::hub(cx).update(cx, |h, cx| h.view_action(&ext, &view, item, action, cx).map(|_| ())),
        };
        self.error = res.err();
        cx.notify();
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let items = self.items(cx);
        if items.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| items.iter().position(|i| i.id == *s));
        let next = match at {
            Some(i) => (i as isize + by).clamp(0, items.len() as isize - 1) as usize,
            None => 0,
        };
        let id = items[next].id.clone();
        self.select(&id, cx);
        self.list_scroll.scroll_to_item(next);
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            return;
        }
        let m = &e.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.function {
            return;
        }
        let handled = match e.keystroke.key.as_str() {
            "up" | "k" => {
                self.step(-1, cx);
                true
            }
            "down" | "j" => {
                self.step(1, cx);
                true
            }
            "enter" => {
                let items = self.items(cx);
                let item = self.selected.as_ref().and_then(|s| items.iter().find(|i| i.id == *s));
                match item.and_then(|i| i.actions.iter().find(|a| a.primary).map(|a| (i.id.clone(), a.id.clone()))) {
                    Some((item, action)) => self.act(Some(&item), &action, cx),
                    None if self.selected.is_none() => self.step(0, cx),
                    None => {}
                }
                true
            }
            "escape" if self.menu.is_some() => {
                self.menu = None;
                cx.notify();
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
}

// ── drawing ──────────────────────────────────────────────────────────────

/// The agent a row is about, by name (with `agents.read`): a click focuses
/// it, called as extension `ext` (with `agents.focus`; else it is plain
/// text).
pub(crate) fn agent_chip(ext: &str, item: &str, agent: &str, t: &Theme, cx: &mut App) -> Option<AnyElement> {
    let name = agent_name(ext, agent, cx)?;
    let focus = pitwall_core::extensions::view::Action { id: "focus".into(), label: name.clone(), primary: false, run: Some(pitwall_core::extensions::view::Run::Focus(agent.to_string())), needs: None };
    let may = blocked(ext, &focus, cx).is_none();
    let sel = format!("view-agent-{item}");
    let (ext, agent) = (ext.to_string(), agent.to_string());
    Some(
        div()
            .id(SharedString::from(sel.clone()))
            .debug_selector(move || sel.clone())
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(11.5))
            .text_color(if may { t.focus } else { t.text_3 })
            .child(kit::icon("terminal", 11., if may { t.focus } else { t.text_4 }))
            .child(kit::one_line(name))
            .when(may, |d| {
                d.cursor_pointer().tooltip(kit::tooltip("Focus this agent")).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    let _ = hub::hub(cx).update(cx, |h, cx| h.press(&ext, "", None, Some(&agent), &focus, cx));
                })
            })
            .into_any_element(),
    )
}

impl ViewTile {
    fn title(&self, m: &Manifest) -> String {
        let view = m.contributes.view(&self.view).map_or(self.view.clone(), |d| d.title.clone());
        format!("{view} · {}", m.name)
    }

    fn header(&self, m: &Manifest, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let in_window = self.in_window;
        let what = Content::View(self.key.clone());
        let maximized = {
            let ui = crate::ui_state::get(cx);
            ui.tile_of(&what).is_some_and(|(space, pane)| ui.space(&space).is_some_and(|s| s.maximized_pane_id.as_deref() == Some(pane.as_str())))
        };
        let btn = |name: &str, icon: &str, tip: String, on: Option<bool>| {
            let sel = format!("view-{name}");
            kit::icon_btn_state(sel.clone(), icon, tip, true, 13., on, t)
                .debug_selector(move || sel.clone())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let title = self.title(m);
        let icon = icon_or(m.contributes.view(&self.view).and_then(|d| d.icon.as_deref()), crate::extensions::icon_of(m));
        let name: SharedString = title.clone().into();
        div()
            .id(SharedString::from(format!("view-bar-{}", self.key)))
            .debug_selector(|| "view-bar".into())
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(4.))
            .pr(px(8.))
            .border_b_1()
            .border_color(if t.is_glass() { t.g.line } else { t.line })
            .when(!in_window, |d| d.on_drag(TileDrag { what: what.clone(), name: name.clone() }, |d, grab, w, cx| d.ghost(grab, w, cx)))
            .child(
                div()
                    .id("view-grip")
                    .debug_selector(|| "view-grip".into())
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(kit::tooltip(if in_window { "Drag into a Pitwall window to place it in the layout" } else { "Drag to move the view" }))
                    .when(in_window, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(|v, _, window, cx| {
                            cx.stop_propagation();
                            let key = v.key.clone();
                            window.defer(cx, move |window, cx| super::inbox_window::drag_start(&key, window, cx));
                        }))
                    })
                    .child(kit::icon("grip", 13., t.text_4)),
            )
            .child(kit::icon(&icon, 15., t.text_2))
            .child(div().flex_1().min_w_0().ellipsis().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(kit::one_line(title)))
            .child(
                btn(if in_window { "dock" } else { "move" }, "grid", (if in_window { "Place it in a Pitwall window's layout" } else { "Move it in the layout, or to a window" }).into(), Some(self.menu.is_some())).on_click(cx.listener(|v, e: &gpui::ClickEvent, _, cx| {
                    v.menu = match v.menu {
                        Some(_) => None,
                        None => Some((e.position(), super::inbox_window::replaceable(&v.key, cx))),
                    };
                    cx.notify();
                })),
            )
            .map(|d| {
                if in_window {
                    d.when(crate::platform::can_keep_on_top(), |d| {
                        let pinned = self.pinned;
                        d.child(btn("pin", "pin", (if pinned { "Kept on top of other windows: click to let it go behind them" } else { "Keep on top of other windows" }).into(), Some(pinned)).on_click(cx.listener(|v, _, window, cx| {
                            v.pinned = !v.pinned;
                            crate::platform::keep_on_top(window, v.pinned);
                            cx.notify();
                        })))
                    })
                } else {
                    d.child(btn("maximize", if maximized { "restore" } else { "maximize" }, kit::keys(if maximized { "Restore (⌘⏎)" } else { "Maximise (⌘⏎)" }, false), None).on_click(cx.listener(|v, _, window, cx| {
                        window.focus(&v.focus, cx);
                        window.dispatch_action(Box::new(super::ToggleMaximize), cx);
                    })))
                    .child(btn("undock", "window", "Open in a window of its own".into(), None).on_click(cx.listener(|v, _, _, cx| {
                        let key = v.key.clone();
                        cx.defer(move |cx| {
                            if let Err(e) = super::inbox_window::pop_out(&key, None, true, cx) {
                                eprintln!("pitwall: could not open the view's window: {e}");
                            }
                        });
                    })))
                    .child(btn("close", "x", "Close the view".into(), None).on_click(cx.listener(|v, _, _, cx| {
                        let key = v.key.clone();
                        cx.defer(move |cx| close(&key, cx));
                    })))
                }
            })
            .into_any_element()
    }

    fn place_menu(&self, at: Point<Pixels>, replace: Vec<(String, String)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let key = self.key.clone();
        let pick = move |p: Option<(FilesPlace, Option<String>)>, cx: &mut App| {
            let _ = me.update(cx, |v, cx| {
                v.menu = None;
                cx.notify();
            });
            if let Some((place, of)) = p {
                let key = key.clone();
                cx.defer(move |cx| {
                    if let Err(e) = super::inbox_window::place(&key, place, of.as_deref(), cx) {
                        eprintln!("pitwall: could not move the view: {e}");
                    }
                });
            }
        };
        crate::explorer::dock::menu(at, !self.in_window, None, replace, "view", t, pick)
    }

    /// The tile while the adapter can't draw it: why, and what to do.
    fn state(&self, m: &Manifest, run: &Run, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ext = self.ext.clone();
        let settings = |id: &'static str, t: &Theme| {
            kit::text_button(id, "Open Settings", kit::BtnKind::Primary, false, t).debug_selector(move || id.into()).on_click(|_, window, cx| {
                to_extensions_page(cx);
                window.dispatch_action(Box::new(crate::menu::OpenSettings), cx)
            })
        };
        let el = match run.clone() {
            Run::Off if !crate::ui_state::get(cx).extension_on(&m.id) => calm("extension", &format!("{} is off", m.name), "Turn it on in Settings → Extensions.".into(), t).child(settings("view-settings", t)),
            Run::Off => calm("alert", "It has no adapter", "This extension has nothing to show here.".into(), t),
            Run::Setup(why) => calm("lock", &format!("Connect {}", m.name), why, t).child(settings("view-connect", t)),
            Run::Stopped(why) => {
                let (head, said) = match why.split_once(" It said: ") {
                    Some((h, s)) => (h.to_string(), Some(s.to_string())),
                    None => (why, None),
                };
                calm("alert", &format!("{} stopped", m.name), format!("{head} Restart it, or check its settings in Settings → Extensions."), t).when_some(said, |d, said| d.child(output_tail(&said, t))).child(
                    kit::text_button("view-restart", "Restart", kit::BtnKind::Primary, false, t).debug_selector(|| "view-restart".into()).on_click(cx.listener(move |_, _, _, cx| {
                        let ext = ext.clone();
                        hub::hub(cx).update(cx, |h, cx| h.restart(&ext, cx));
                    })),
                )
            }
            Run::Starting => calm("refresh", "Starting…", format!("{} is starting.", m.name), t),
            Run::Running => return None,
        };
        Some(el.debug_selector(|| "view-state".into()).into_any_element())
    }

    fn buttons(&self, item: Option<&str>, actions: &[Action], busy: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(actions.iter().enumerate().map(|(n, a)| {
                let id = format!("view-action-{}-{}", item.unwrap_or(""), a.id);
                let (item, action) = (item.map(String::from), a.id.clone());
                let kind = if a.primary { kit::BtnKind::Primary } else { kit::BtnKind::Ghost };
                let why = blocked(&self.ext, a, cx);
                kit::text_button(SharedString::from(format!("{id}-{n}")), a.label.clone(), kind, busy || why.is_some(), t)
                    .debug_selector(move || id.clone())
                    .when_some(why.clone(), |d, why| d.tooltip(kit::tooltip(why)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(why.is_none(), |d| {
                        d.on_click(cx.listener(move |v, _, _, cx| {
                            cx.stop_propagation();
                            v.act(item.as_deref(), &action, cx);
                        }))
                    })
            }))
            .into_any_element()
    }

    fn row(&self, i: &Item, cursor: bool, busy: bool, now: chrono::DateTime<chrono::Utc>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.as_deref() == Some(i.id.as_str());
        let id = i.id.clone();
        let time = when(&i.time, now).map(|(short, _)| short).or_else(|| (!i.time.is_empty() && chrono::DateTime::parse_from_rfc3339(&i.time).is_err()).then(|| i.time.clone()));
        let icon = icon_or(i.icon.as_deref(), "");
        div()
            .id(SharedString::from(format!("view-row-{}", i.id)))
            .debug_selector({
                let id = i.id.clone();
                move || format!("view-row-{id}")
            })
            .flex()
            .gap(px(8.))
            .px(px(10.))
            .py(px(8.))
            .mx(px(6.))
            .rounded(RADIUS)
            .cursor_pointer()
            .map(|d| if selected { d.bg(t.surface_3) } else { d.hover(|s| s.bg(t.surface_2)) })
            .when(cursor && selected, |d| d.border_1().border_color(t.text.opacity(0.22)))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus, cx);
                this.select(&id, cx);
            }))
            .child(div().flex_none().pt(px(2.)).w(px(14.)).map(|d| if icon.is_empty() { d.child(div().mt(px(4.)).size(px(7.)).rounded_full().bg(tone(i.tone, t))) } else { d.child(kit::icon(&icon, 14., tone(i.tone, t))) }))
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
                            .child(div().flex_1().min_w_0().ellipsis().text_size(px(13.)).text_color(t.text).child(kit::one_line(i.title.clone())))
                            .children(i.badges.iter().map(|b| div().flex_none().px(px(5.)).rounded(px(4.)).bg(t.surface_2).text_size(px(11.)).text_color(if b.tone == Tone::None { t.text_2 } else { tone(b.tone, t) }).child(b.text.clone())))
                            .when_some(time, |d, s| d.child(div().flex_none().text_size(px(11.)).text_color(t.text_3).child(s))),
                    )
                    .when(!i.subtitle.is_empty(), |d| d.child(div().ellipsis().text_size(px(12.)).text_color(t.text_3).child(kit::one_line(i.subtitle.clone()))))
                    .children(i.agent.as_deref().and_then(|a| agent_chip(&self.ext, &i.id, a, t, cx)))
                    .when(selected && !i.actions.is_empty(), |d| d.child(div().pt(px(4.)).child(self.buttons(Some(&i.id), &i.actions, busy, t, cx)))),
            )
            .into_any_element()
    }

    fn list(&self, list: &pitwall_core::extensions::view::List, focused: bool, busy: &[(String, String)], t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if list.items().next().is_none() {
            let empty = if list.empty.is_empty() { "Nothing here".to_string() } else { list.empty.clone() };
            return calm("check", &empty, String::new(), t).debug_selector(|| "view-empty".into()).into_any_element();
        }
        let now = chrono::Utc::now();
        let mut rows: Vec<AnyElement> = Vec::new();
        for s in &list.sections {
            if !s.title.is_empty() {
                rows.push(div().px(px(16.)).pt(px(10.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text_3).child(kit::one_line(s.title.to_uppercase())).into_any_element());
            }
            for i in &s.items {
                let is_busy = busy.iter().any(|(_, b)| *b == i.id);
                rows.push(self.row(i, focused, is_busy, now, t, cx));
            }
        }
        div().id("view-list").track_scroll(&self.list_scroll).overflow_y_scroll().flex_1().min_h_0().py(px(6.)).flex().flex_col().gap(px(1.)).children(rows).into_any_element()
    }

    fn detail(&mut self, d: &Detail, busy: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = (!d.body.is_empty()).then(|| self.markdown(format!("{}-detail", self.key), &d.body, cx));
        let item = self.selected.clone();
        div()
            .id("view-detail")
            .debug_selector(|| "view-detail".into())
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .px(px(20.))
            .py(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .when(!d.title.is_empty(), |el| el.child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(d.title.clone())))
            .when(!d.fields.is_empty(), |el| {
                el.child(div().flex().flex_col().gap(px(6.)).children(d.fields.iter().enumerate().map(|(n, f)| {
                    let value: AnyElement = match &f.link {
                        Some(url) => {
                            let url = url.clone();
                            div()
                                .id(SharedString::from(format!("view-field-{n}")))
                                .text_color(t.focus)
                                .cursor_pointer()
                                .tooltip(kit::tooltip(url.clone()))
                                .on_click(move |_, _, cx| crate::platform::files::open_url(cx, &url))
                                .child(f.value.clone())
                                .into_any_element()
                        }
                        None => div().text_color(t.text).child(f.value.clone()).into_any_element(),
                    };
                    div().flex().gap(px(12.)).text_size(px(13.)).child(div().flex_none().w(px(120.)).text_color(t.text_3).child(f.label.clone())).child(div().flex_1().min_w_0().child(value))
                })))
            })
            .when_some(body, |el, b| el.child(b))
            .when(!d.actions.is_empty(), |el| el.child(self.buttons(item.as_deref(), &d.actions, busy, t, cx)))
            .into_any_element()
    }

    fn status(&self, s: &Status, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let fallback = match s.kind {
            StatusKind::Empty => "check",
            StatusKind::Loading => "refresh",
            StatusKind::Error => "alert",
        };
        let icon = icon_or(s.icon.as_deref(), fallback);
        calm(&icon, &s.title, s.text.clone(), t).debug_selector(|| "view-status".into()).when(!s.actions.is_empty(), |d| d.child(self.buttons(None, &s.actions, false, t, cx))).into_any_element()
    }

    fn pane(&mut self, p: &Pane, busy: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match p {
            Pane::Detail(d) => self.detail(d, busy, t, cx),
            Pane::Status(s) => self.status(s, t, cx),
            Pane::Markdown(text) => {
                let md = self.markdown(format!("{}-md", self.key), text, cx);
                div().id("view-markdown").debug_selector(|| "view-markdown".into()).overflow_y_scroll().flex_1().min_h_0().px(px(20.)).py(px(16.)).child(md).into_any_element()
            }
        }
    }
}

impl Render for ViewTile {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let focused = self.focus.contains_focused(window, cx);
        let root = div()
            .id(SharedString::from(format!("view-{}", self.key)))
            .key_context("ViewTile")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
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
            .flex_col();
        let menu = self.menu.clone().map(|(at, replace)| self.place_menu(at, replace, &t, cx));
        let root = root.children(menu);
        let m = self.manifest(cx).filter(|m| m.contributes.view(&self.view).is_some());
        let Some(m) = m else {
            let key = self.key.clone();
            return root
                .child(calm("extension", "This view is gone", "Its extension isn't found, or no longer has this view. The tile can be closed.".into(), &t))
                .child(div().flex().justify_center().pb(px(24.)).child(kit::small_btn("view-gone-close", "Close", &t).on_click(move |_, _, cx| {
                    let key = key.clone();
                    cx.defer(move |cx| close(&key, cx));
                })))
                .into_any_element();
        };
        let root = root.child(self.header(&m, &t, cx));
        let (run, data, note, busy) = self.data(cx);
        if let Some(s) = self.state(&m, &run, &t, cx) {
            return root.child(s).into_any_element();
        }
        let note = self.error.clone().or(note);
        let root = root.when_some(note, |d, n| d.child(div().debug_selector(|| "view-note".into()).px(px(14.)).py(px(6.)).text_size(px(12.)).text_color(t.text_2).bg(t.surface_2).child(n)));
        let body = match data {
            None => calm("refresh", "Loading…", format!("Waiting for {} to send this view.", m.name), &t).debug_selector(|| "view-loading".into()).into_any_element(),
            Some(Err(why)) => calm("alert", "This view can't be drawn", why, &t).debug_selector(|| "view-refused".into()).into_any_element(),
            Some(Ok(Root::Pane(p))) => {
                let b = busy.iter().any(|(_, i)| i.is_empty());
                self.pane(&p, b, &t, cx)
            }
            Some(Ok(Root::List { list, detail })) => {
                let list_el = self.list(&list, focused, &busy, &t, cx);
                match detail {
                    None => list_el,
                    Some(p) => {
                        let b = self.selected.as_ref().is_some_and(|s| busy.iter().any(|(_, i)| i == s));
                        let side = self.pane(&p, b, &t, cx);
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(div().w(px(LIST_W)).flex_none().min_h_0().flex().flex_col().border_r_1().border_color(if t.is_glass() { t.g.line } else { t.line }).child(list_el))
                            .child(div().flex_1().min_w_0().min_h_0().flex().flex_col().child(side))
                            .into_any_element()
                    }
                }
            }
        };
        root.child(body).into_any_element()
    }
}

// ── in the window ────────────────────────────────────────────────────────

/// Every view tile, one per view, wherever it is drawn: moving it keeps
/// it, with its selection.
#[derive(Default)]
struct Views(HashMap<String, Entity<ViewTile>>);

impl gpui::Global for Views {}

/// View `key`'s tile (made when it isn't yet).
pub fn view(key: &str, cx: &mut App) -> Entity<ViewTile> {
    if let Some(v) = get(key, cx) {
        return v;
    }
    let v = cx.new(|cx| ViewTile::new(key, cx));
    cx.default_global::<Views>().0.insert(key.to_string(), v.clone());
    v
}

/// View `key`'s tile, if it is made.
pub fn get(key: &str, cx: &App) -> Option<Entity<ViewTile>> {
    cx.try_global::<Views>()?.0.get(key).cloned()
}

/// View `key` closed: out of every layout, its window (if it is in one)
/// closed, its tile let go (the adapter is told it is hidden).
pub fn close(key: &str, cx: &mut App) {
    crate::ui_state::update(cx, |ui| *ui = ui.clone().close_surface(&Surface::parse(key)));
    super::inbox_window::close_for_move(key, cx);
    if cx.has_global::<Views>() {
        cx.global_mut::<Views>().0.remove(key);
    }
}

impl MainScreen {
    /// "Open <view>", a badge, a note or a notification: extension `ext`'s
    /// view `view` focused where it is (its window brought forward when it
    /// is in one), else placed as the inbox is; at row `item` when given.
    pub fn open_view(&mut self, ext: &str, view: &str, item: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = crate::extensions::manifest_of(ext, cx).filter(|m| m.contributes.view(view).is_some()) else {
            self.toast(Toast::info("No such view", Some("Its extension isn't found, or no longer has it.".into())), cx);
            return;
        };
        let s = Surface::View(m.id.clone(), view.to_string());
        let key = s.key();
        let v = self::view(&key, cx);
        if let Some(item) = item {
            v.update(cx, |v, cx| v.select(item, cx));
        }
        let f = v.read(cx).focus.clone();
        if let Some(h) = crate::windows::inbox_window(&key, cx) {
            cx.defer(move |cx| {
                let _ = h.update(cx, |_, window, cx| {
                    if !crate::bench::enabled() {
                        window.activate_window();
                    }
                    window.focus(&f, cx);
                });
            });
            return;
        }
        match self.ui.tile_of(&s.content()) {
            Some((space, pane)) if self.ui.window_of_space(&space) == self.label => {
                self.active = space.clone();
                self.set_route(super::Route::Space, cx);
                self.update_ui(|u| u.focus_pane(&space, &pane), cx);
            }
            _ => {
                let Some(space) = self.active_space().cloned() else { return };
                let anchor = space.focused_pane_id.clone().and_then(|p| space.layout.find_pane(&p));
                let spot = match anchor {
                    Some(p) if p.is_empty() => FilesSpot::Take(p.id),
                    Some(p) => FilesSpot::Tile { pane: p.id, side: super::tree::Side::Right },
                    None => FilesSpot::Auto,
                };
                let fit = self.fit_for(&space.id);
                let mut placed = false;
                self.update_ui(
                    |u| {
                        let (next, at) = u.surface_to_layout(&space.id, &s, &spot, fit);
                        placed = at.is_some();
                        match at {
                            Some(pane) => next.focus_pane(&space.id, &pane),
                            None => next,
                        }
                    },
                    cx,
                );
                if !placed {
                    return;
                }
            }
        }
        window.focus(&f, cx);
        cx.notify();
    }

    /// An extension's command picked in the palette: sent to its adapter
    /// (with the focused agent's id when it asks for one and may read
    /// agents); its answer comes back as a toast ([`crate::extensions`]).
    pub fn extension_command(&mut self, ext: &str, command: &str, title: &str, with_agent: bool, cx: &mut Context<Self>) {
        let agent = if with_agent { self.focused_agent_id() } else { None };
        if let Err(why) = hub::hub(cx).update(cx, |h, cx| h.command(ext, command, title, agent, cx)) {
            self.toast(Toast::error(format!("Couldn't run “{title}”"), why), cx);
        }
    }

    /// Tile `pane` showing view `key`: a tile like the inbox's.
    pub(super) fn render_view_pane(&mut self, pane: &str, key: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = view(key, cx);
        v.update(cx, |v, cx| v.held(false, cx));
        let body = div().flex_1().min_w_0().min_h_0().flex().child(v).into_any_element();
        self.tile_shell(pane, Content::View(key.to_string()), format!("view-tile-{key}"), body, None, t, cx)
    }

    /// The badges and notes extensions put on agent `a`'s row (beside the
    /// ask badge): a count and/or a few words, a click opens what it names.
    pub(super) fn agent_marks(&self, a: &pitwall_proto::AgentView, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let marks = crate::extensions::marks_for_agent(a, cx);
        marks.into_iter().enumerate().map(|(n, m)| self.mark(&format!("agent-{}-{n}", a.id), m, t, cx)).collect()
    }

    /// The badges and notes on project `path`'s row.
    pub(super) fn project_marks(&self, path: &str, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let marks = crate::extensions::marks_for_project(path, cx);
        marks.into_iter().enumerate().map(|(n, m)| self.mark(&format!("project-{path}-{n}"), m, t, cx)).collect()
    }

    fn mark(&self, id: &str, m: crate::extensions::RowMark, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use pitwall_core::extensions::view::Open;
        let b = &m.badge;
        let color = if b.tone == Tone::None { t.text_2 } else { tone(b.tone, t) };
        let tip = if b.tooltip.is_empty() { m.name.clone() } else { format!("{} — {}", m.name, b.tooltip) };
        let open = b.open.clone();
        let ext = m.ext.clone();
        let sel = format!("ext-mark-{id}");
        div()
            .id(SharedString::from(sel.clone()))
            .debug_selector(move || sel.clone())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(3.))
            .h(px(18.))
            .px(px(6.))
            .rounded_full()
            .bg(t.surface_3)
            .text_color(color)
            .text_size(px(11.))
            .max_w(px(120.))
            .tooltip(kit::tooltip(tip))
            .when_some(b.icon.as_deref().filter(|i| kit::has_icon(i)), |d, i| d.child(kit::icon(i, 11., color)))
            .when_some(b.count, |d, c| d.font_weight(FontWeight::SEMIBOLD).child(c.to_string()))
            .when(!b.note.is_empty(), |d| d.child(div().min_w_0().ellipsis().child(kit::one_line(b.note.clone()))))
            .when_some(open, |d, open| {
                d.cursor_pointer().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    match &open {
                        Open::View { view, item } => this.open_view(&ext, view, item.as_deref(), window, cx),
                        Open::Thread(thread) => this.open_inbox(Some(&ext), Some(thread), window, cx),
                    }
                }))
            })
            .into_any_element()
    }

    /// Bench and debug builds, `view …` (nothing clicked or typed on the
    /// screen): `open <ext>/<view> [item]`, `select <ext>/<view> <item>`,
    /// `action <ext>/<view> <item|-> <action>`, `close <ext>/<view>`,
    /// `state <ext>/<view>` (printed).
    pub fn debug_view(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut w = cmd.split_whitespace();
        let (Some(what), Some(key)) = (w.next(), w.next()) else { return };
        let Surface::View(ext, view_id) = Surface::parse(key) else { return };
        match what {
            "open" => {
                let item = w.next().map(String::from);
                self.open_view(&ext, &view_id, item.as_deref(), window, cx);
            }
            "select" => {
                if let (Some(item), Some(v)) = (w.next(), get(key, cx)) {
                    v.update(cx, |v, cx| v.select(item, cx));
                }
            }
            "action" => {
                if let (Some(item), Some(action), Some(v)) = (w.next(), w.next(), get(key, cx)) {
                    let item = (item != "-").then_some(item);
                    v.update(cx, |v, cx| v.act(item, action, cx));
                }
            }
            "close" => close(key, cx),
            "state" => {
                let v = get(key, cx);
                let sel = v.as_ref().and_then(|v| v.read(cx).selected().map(String::from));
                let st = hub::try_hub(cx).and_then(|h| h.read(cx).get(&ext).map(|s| (s.views.get(&view_id).map(|v| v.is_ok()), s.view_notes.get(&view_id).cloned())));
                eprintln!("pitwall: view {key}: open {}, selected {sel:?}, drawn {st:?}, at {}", v.is_some(), super::inbox_window::described(key, cx));
            }
            _ => {}
        }
    }
}
