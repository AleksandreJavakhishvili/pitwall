//! Extensions' sections in the right panel (docs/extensions/views.md,
//! "Right-panel sections"): each section a manifest contributes
//! (`contributes.panels`), for the focused agent, below Last sent. Drawn
//! from the same components as a view tile: a small list (at most
//! [`PANEL_ROWS`] rows, the rest counted), a detail, Markdown or a status,
//! in at most [`MAX_H`] of height; each folds like the panel's other
//! sections (a folded one isn't sent: its adapter is told `hide`).
//!
//! A section shows only for an extension that is on and running, granted
//! `agents.read` (a section is about one agent), and an agent of this
//! computer (not a peer's, shown here read-only). Its adapter is told
//! `show` with the agent's id while it is drawn, and `hide` after. Its
//! buttons go through [`hub::Hub::press`]: one Pitwall runs itself (open a
//! file or a page, focus an agent) is called as the extension, behind its
//! permission; the agent it acts for is the section's unless it names
//! another.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{div, prelude::*, px, AnyView, App, AppContext, Context, Entity, FontWeight, Global, IntoElement, MouseButton, SharedString, Subscription, Window, WindowId};
use pitwall_core::extensions::permissions;
use pitwall_core::extensions::view::{Action, Pane, Root, PANEL_ROWS};
use pitwall_core::extensions::{Manifest, PanelDef};
use pitwall_proto::AgentView;

use super::hub::{self, panel_key, Run};
use crate::kit::{self, Ellipsis};
use crate::main_screen::view_tile::{agent_chip, blocked, icon_or};
use crate::main_screen::{register_panel_section, PanelSlot};
use crate::markdown_view::MarkdownView;
use crate::theme::{self, Theme, RADIUS};

/// The most height a section's body takes (it scrolls inside).
pub const MAX_H: f32 = 280.;

/// One window's sections.
#[derive(Default)]
struct Views(HashMap<WindowId, Entity<Sections>>);

impl Global for Views {}

/// Sections folded by the user (extension, section), for every window
/// while Pitwall runs.
#[derive(Default)]
struct Folded(HashSet<(String, String)>);

impl Global for Folded {}

/// Draw extensions' sections below Last sent (call once at start).
pub fn register(cx: &mut App) {
    register_panel_section(cx, PanelSlot::Extensions, |a, window, cx| {
        let id = window.window_handle().window_id();
        let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        cx.default_global::<Views>().0.retain(|w, _| open.contains(w));
        let view = match cx.default_global::<Views>().0.get(&id).cloned() {
            Some(v) => v,
            None => {
                let v = cx.new(Sections::new);
                cx.default_global::<Views>().0.insert(id, v.clone());
                v
            }
        };
        view.update(cx, |v, cx| v.set_agent(a, cx));
        AnyView::from(view)
    });
}

/// The sections agent `a` gets now: (extension, its section), for each
/// extension that is on and running and granted `agents.read`; none for
/// a peer's agent.
pub fn sections_for(a: &AgentView, cx: &App) -> Vec<(Manifest, PanelDef)> {
    if crate::agents::mirrored(a) {
        return vec![];
    }
    let Some(reg) = cx.try_global::<super::calls::Api>().map(|api| api.registry.clone()) else { return vec![] };
    let Some(h) = hub::try_hub(cx) else { return vec![] };
    let ui = crate::ui_state::get(cx);
    let mut out = Vec::new();
    for m in super::enabled(&super::found(cx), &ui) {
        if m.contributes.panels.is_empty() || !reg.allows(&m.id, permissions::AGENTS_READ) {
            continue;
        }
        if !h.read(cx).get(&m.id).is_some_and(|st| st.run == Run::Running) {
            continue;
        }
        for p in &m.contributes.panels {
            out.push((m.clone(), p.clone()));
        }
    }
    out
}

/// Whether section `panel` of `ext` is folded.
fn folded(ext: &str, panel: &str, cx: &App) -> bool {
    cx.try_global::<Folded>().is_some_and(|f| f.0.contains(&(ext.to_string(), panel.to_string())))
}

/// Fold (or unfold) section `panel` of `ext` everywhere.
pub fn set_folded(ext: &str, panel: &str, fold: bool, cx: &mut App) {
    let k = (ext.to_string(), panel.to_string());
    let f = &mut cx.default_global::<Folded>().0;
    if fold {
        f.insert(k);
    } else {
        f.remove(&k);
    }
}

/// The extensions' sections of one window, following its focused agent.
pub struct Sections {
    agent: Option<AgentView>,
    /// What its adapters are told is shown: (extension, view key).
    shown: Vec<(String, String)>,
    md: HashMap<String, Entity<MarkdownView>>,
    /// Why a press didn't reach the extension, by view key.
    errors: HashMap<String, String>,
    _subs: Vec<Subscription>,
}

impl Sections {
    fn new(cx: &mut Context<Self>) -> Sections {
        let h = hub::hub(cx);
        let mut subs = vec![cx.observe(&h, |this, _, cx| {
            this.follow(cx);
            cx.notify();
        })];
        subs.push(cx.on_release(|this: &mut Sections, cx: &mut App| {
            if let Some(h) = hub::try_hub(cx) {
                let shown = std::mem::take(&mut this.shown);
                h.update(cx, |h, _| {
                    for (ext, key) in shown {
                        h.hide_view(&ext, &key);
                    }
                });
            }
        }));
        Sections { agent: None, shown: vec![], md: HashMap::new(), errors: HashMap::new(), _subs: subs }
    }

    fn set_agent(&mut self, a: &AgentView, cx: &mut Context<Self>) {
        let same = self.agent.as_ref().is_some_and(|x| x.id == a.id && x.name == a.name && x.machine == a.machine);
        if !same {
            self.agent = Some(a.clone());
            self.errors.clear();
        }
        self.follow(cx);
    }

    /// Tell the adapters which sections are drawn now (later: not while
    /// the hub is being updated).
    fn follow(&mut self, cx: &mut Context<Self>) {
        let want: Vec<(String, String)> = match &self.agent {
            Some(a) => sections_for(a, cx).into_iter().filter(|(m, p)| !folded(&m.id, &p.id, cx)).map(|(m, p)| (m.id.clone(), panel_key(&p.id, &a.id))).collect(),
            None => vec![],
        };
        if want == self.shown {
            return;
        }
        let gone: Vec<(String, String)> = self.shown.iter().filter(|s| !want.contains(s)).cloned().collect();
        let new: Vec<(String, String)> = want.iter().filter(|s| !self.shown.contains(s)).cloned().collect();
        self.shown = want;
        cx.defer(move |cx| {
            let h = hub::hub(cx);
            h.update(cx, |h, cx| {
                for (ext, key) in &gone {
                    h.hide_view(ext, key);
                }
                for (ext, key) in &new {
                    h.show_view(ext, key, cx);
                }
            });
        });
    }

    /// Press button `a` of section `key` (on `item`, whose agent is
    /// `agent`).
    fn press(&mut self, ext: &str, key: &str, item: Option<&str>, agent: Option<&str>, a: &Action, cx: &mut Context<Self>) {
        let res = hub::hub(cx).update(cx, |h, cx| h.press(ext, key, item, agent, a, cx));
        match res {
            Ok(()) => self.errors.remove(key),
            Err(e) => self.errors.insert(key.to_string(), e),
        };
        cx.notify();
    }

    fn markdown(&mut self, key: &str, text: &str, cx: &mut Context<Self>) -> Entity<MarkdownView> {
        let v = self.md.entry(key.to_string()).or_insert_with(|| cx.new(|cx| MarkdownView::new(cx).inline())).clone();
        let text: Arc<str> = Arc::from(text);
        let k = key.to_string();
        v.update(cx, |v, cx| v.set(&k, text, None, None, cx));
        v
    }

    /// Section `key` of extension `ext`: the buttons of `item` (or of its pane).
    fn buttons(&self, (ext, key): (&str, &str), item: Option<&str>, agent: Option<&str>, actions: &[Action], t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        div().flex().flex_wrap().gap(px(4.)).children(actions.iter().enumerate().map(|(n, a)| {
            let why = blocked(ext, a, cx);
            let sel = format!("panel-action-{key}-{}-{}", item.unwrap_or(""), a.id);
            let kind = if a.primary { kit::BtnKind::Small } else { kit::BtnKind::GhostSm };
            let (ext, key, item, agent, a2) = (ext.to_string(), key.to_string(), item.map(String::from), agent.map(String::from), a.clone());
            kit::text_button(SharedString::from(format!("{sel}-{n}")), a.label.clone(), kind, why.is_some(), t)
                .debug_selector(move || sel.clone())
                .when_some(why.clone(), |d, why| d.tooltip(kit::tooltip(why)))
                .when(why.is_none(), |d| {
                    d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.press(&ext, &key, item.as_deref(), agent.as_deref(), &a2, cx);
                    }))
                })
        }))
    }

    /// One section's body: what its adapter sent for this agent.
    fn body(&mut self, m: &Manifest, key: &str, agent: &str, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let data = hub::try_hub(cx).and_then(|h| h.read(cx).get(&m.id).and_then(|st| st.views.get(key).cloned()));
        let quiet = |text: String| div().px(px(14.)).py(px(4.)).text_size(px(12.)).text_color(t.text_3).child(text).into_any_element();
        let root = match data {
            None => return quiet("Loading…".into()),
            Some(Err(why)) => return quiet(why),
            Some(Ok(r)) => r,
        };
        match root {
            Root::List { list, .. } => {
                let items: Vec<_> = list.items().cloned().collect();
                if items.is_empty() {
                    return quiet(if list.empty.is_empty() { "Nothing here".into() } else { list.empty.clone() });
                }
                let more = items.len().saturating_sub(PANEL_ROWS);
                let mut col = div().flex().flex_col().gap(px(2.)).px(px(8.));
                for i in items.iter().take(PANEL_ROWS) {
                    let icon = icon_or(i.icon.as_deref(), "");
                    let sel = format!("panel-row-{key}-{}", i.id);
                    let row_agent = i.agent.clone().unwrap_or_else(|| agent.to_string());
                    let buttons = (!i.actions.is_empty()).then(|| self.buttons((&m.id, key), Some(&i.id), Some(&row_agent), &i.actions, t, cx));
                    let chip = i.agent.as_deref().filter(|a| *a != agent).and_then(|a| agent_chip(&m.id, &i.id, a, t, cx));
                    col = col.child(
                        div()
                            .debug_selector(move || sel.clone())
                            .flex()
                            .gap(px(8.))
                            .px(px(6.))
                            .py(px(5.))
                            .rounded(RADIUS)
                            .hover(|s| s.bg(t.surface_2))
                            .child(div().flex_none().pt(px(2.)).w(px(14.)).map(|d| if icon.is_empty() { d.child(div().mt(px(4.)).size(px(6.)).rounded_full().bg(t.text_3)) } else { d.child(kit::icon(&icon, 13., t.text_3)) }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(div().ellipsis().text_size(px(12.5)).text_color(t.text).child(kit::one_line(i.title.clone())))
                                    .when(!i.subtitle.is_empty(), |d| d.child(div().ellipsis().text_size(px(11.5)).text_color(t.text_3).child(kit::one_line(i.subtitle.clone()))))
                                    .children(chip)
                                    .children(buttons.map(|b| b.pt(px(3.)))),
                            ),
                    );
                }
                if more > 0 {
                    col = col.child(div().px(px(6.)).py(px(3.)).text_size(px(11.5)).text_color(t.text_3).child(format!("and {more} more")));
                }
                col.into_any_element()
            }
            Root::Pane(Pane::Markdown(text)) => {
                let md = self.markdown(key, &text, cx);
                div().px(px(14.)).child(md).into_any_element()
            }
            Root::Pane(Pane::Detail(d)) => {
                let body = (!d.body.is_empty()).then(|| self.markdown(&format!("{key}-detail"), &d.body, cx));
                let buttons = (!d.actions.is_empty()).then(|| self.buttons((&m.id, key), None, Some(agent), &d.actions, t, cx));
                div()
                    .px(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .when(!d.title.is_empty(), |el| el.child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(d.title.clone())))
                    .children(d.fields.iter().map(|f| div().flex().gap(px(8.)).text_size(px(12.)).child(div().flex_none().w(px(90.)).text_color(t.text_3).child(f.label.clone())).child(div().flex_1().min_w_0().text_color(t.text).child(f.value.clone()))))
                    .children(body)
                    .children(buttons)
                    .into_any_element()
            }
            Root::Pane(Pane::Status(s)) => {
                let buttons = (!s.actions.is_empty()).then(|| self.buttons((&m.id, key), None, Some(agent), &s.actions, t, cx));
                div()
                    .px(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(12.5)).text_color(t.text_2).child(s.title.clone()))
                    .when(!s.text.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(t.text_3).child(s.text.clone())))
                    .children(buttons)
                    .into_any_element()
            }
        }
    }
}

impl Render for Sections {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::panel(theme::theme(cx), cx);
        let Some(a) = self.agent.clone() else { return div().into_any_element() };
        let mut col = div().flex_none().flex().flex_col();
        for (m, p) in sections_for(&a, cx) {
            let key = panel_key(&p.id, &a.id);
            let is_folded = folded(&m.id, &p.id, cx);
            let note = self.errors.get(&key).cloned().or_else(|| hub::try_hub(cx).and_then(|h| h.read(cx).get(&m.id).and_then(|st| st.view_notes.get(&key).cloned())));
            let icon = icon_or(p.icon.as_deref(), super::icon_of(&m));
            let (ext, pid) = (m.id.clone(), p.id.clone());
            let head_sel = format!("panel-section-{}-{}", m.id, p.id);
            let head = div()
                .id(SharedString::from(head_sel.clone()))
                .debug_selector(move || head_sel.clone())
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(14.))
                .pr(px(10.))
                .pt(px(12.))
                .pb(px(6.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    let fold = !folded(&ext, &pid, cx);
                    set_folded(&ext, &pid, fold, cx);
                    this.follow(cx);
                    cx.notify();
                }))
                .child(kit::icon(if is_folded { "chevron" } else { "chevron-down" }, 10., t.text_4))
                .child(kit::icon(&icon, 12., t.text_3))
                .child(kit::label(p.title.clone(), t.text_3, 12.))
                .child(div().flex_1())
                .child(div().text_size(px(11.)).text_color(t.text_4).child(m.name.clone()));
            let mut section = div().flex_none().flex().flex_col().pb(px(10.)).border_t_1().border_color(t.line).child(head);
            if !is_folded {
                let body = self.body(&m, &key, &a.id, &t, cx);
                let scroll = format!("panel-body-{}-{}", m.id, p.id);
                section = section.child(div().id(SharedString::from(scroll)).max_h(px(MAX_H)).overflow_y_scroll().child(body));
                if let Some(n) = note {
                    section = section.child(div().px(px(14.)).pt(px(4.)).text_size(px(11.5)).text_color(t.amber).child(n));
                }
                if let Some(more) = p.more.clone() {
                    let (ext, sel) = (m.id.clone(), format!("panel-more-{}-{}", m.id, p.id));
                    section = section.child(
                        div()
                            .id(SharedString::from(sel.clone()))
                            .debug_selector(move || sel.clone())
                            .px(px(14.))
                            .pt(px(4.))
                            .text_size(px(11.5))
                            .text_color(t.focus)
                            .cursor_pointer()
                            .child("More…")
                            .on_click(move |_, window, cx| {
                                if let Some(Some(root)) = window.root::<crate::ui::MainView>() {
                                    if let Some(screen) = root.read(cx).screen.clone() {
                                        screen.update(cx, |s, cx| s.open_view(&ext, &more, None, window, cx));
                                    }
                                }
                            }),
                    );
                }
            }
            col = col.child(section);
        }
        let _ = window;
        col.into_any_element()
    }
}
