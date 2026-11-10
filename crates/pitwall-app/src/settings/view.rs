//! The Settings dialog: a compact page list on the left and one page on
//! the right (General, Agents, Rules, Appearance, Folder access, Peers, About;
//! `pitwall_proto::settings::Page`). Every option keeps what the React
//! dialog (`src/components/SettingsDialog.tsx` and its parts) does; the
//! page shown last is remembered in `ui.json` (`settingsPage`). ↑/↓ move
//! between pages; Esc, ✕ and a click outside close it.
//!
//! The pages themselves are in [`super::pages`].
//!
//! A page is drawn as a list of [`Part`]s (its sections, and the rows of
//! a long one) by `gpui::list`, in a view of its own ([`PageBody`]): a
//! scroll step notifies that view and builds, lays out and paints only the
//! parts on screen, not the whole page (docs/spec/perf.md "Settings").
//! Every part is measured again when Settings or what it shows changes.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    actions, div, prelude::*, px, AnyElement, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, KeyBinding, ListAlignment, ListState, Subscription, Task, WeakEntity, Window,
};

use pitwall_core::model::CodexHooksStatus;
use pitwall_core::permissions::PermissionsStatus;
use pitwall_proto::settings::Page;

use super::cli_install::{self, CliStatus};
use super::permissions::POLL;
use super::widgets::Ui;
use super::{data, SettingsHost};
use crate::kit::{tooltip, BtnKind, HoverText as _};
use crate::theme::{appearance, RADIUS_SM};

actions!(pw_settings, [PrevPage, NextPage]);

/// The key context of the page list.
pub const NAV_CONTEXT: &str = "SettingsNav";

pub fn bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", PrevPage, Some(NAV_CONTEXT)),
        KeyBinding::new("down", NextPage, Some(NAV_CONTEXT)),
    ]
}

/// The dialog's size: pages fit without scrolling, except Rules.
pub const WIDTH: f32 = 720.;
pub const HEIGHT: f32 = 540.;
/// The page list's width.
pub const NAV_WIDTH: f32 = 164.;

/// What builds one part of a page.
pub(super) type Build = Rc<dyn Fn(&mut SettingsView, &Ui, &mut Window, &mut Context<SettingsView>) -> AnyElement>;

/// One part of a page in its list: a section (a hairline above it), or a
/// row that continues the section above it (8 px under it), so that a
/// long section's rows are built only when on screen.
#[derive(Clone)]
pub(super) struct Part {
    /// Known by this while the page shows it.
    key: u64,
    cont: bool,
    build: Build,
}

fn hash(x: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

impl Part {
    pub(super) fn section(
        key: impl Hash,
        build: impl Fn(&mut SettingsView, &Ui, &mut Window, &mut Context<SettingsView>) -> AnyElement + 'static,
    ) -> Part {
        Part { key: hash(key), cont: false, build: Rc::new(build) }
    }

    pub(super) fn row(
        key: impl Hash,
        build: impl Fn(&mut SettingsView, &Ui, &mut Window, &mut Context<SettingsView>) -> AnyElement + 'static,
    ) -> Part {
        Part { key: hash(key), cont: true, build: Rc::new(build) }
    }
}

/// The list's first and last items: the page's title (and an error), and
/// the room under the last part.
const HEAD: u64 = 0;
const FOOT: u64 = 1;

/// The page's list, in a view of its own: its scroll steps (wheel,
/// scrollbar) notify this view, not [`SettingsView`], so they don't count
/// as Settings changing (which measures every part again).
pub struct PageBody {
    view: WeakEntity<SettingsView>,
    list: ListState,
}

impl Render for PageBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = appearance(cx).theme().float();
        let view = self.view.clone();
        let list = gpui::list(self.list.clone(), move |ix, window, cx| {
            view.update(cx, |v, cx| v.part(ix, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
        crate::kit::vlist_fill("settings-scroll", &self.list, &t, list).size_full()
    }
}

pub enum SettingsEvent {
    Close,
    /// An agent started from Settings (Accounts → Log in): show it.
    Started(Box<pitwall_proto::AgentView>),
    /// "Scan again" (opens the rescan screen in place of Settings).
    ScanAgain,
}

pub struct SettingsView {
    /// An engine runs (Projects & agents, hooks need it).
    pub(super) live: bool,
    pub(super) hooks: Option<CodexHooksStatus>,
    pub(super) hooks_confirm: bool,
    pub(super) hooks_busy: bool,
    pub(super) perms: Option<PermissionsStatus>,
    /// Folder access → Advanced: the Full Disk Access steps are open.
    pub(super) fda_open: bool,
    pub(super) cli: Option<CliStatus>,
    pub(super) cli_confirm: bool,
    pub(super) cli_dir: Option<String>,
    pub(super) cli_busy: bool,
    pub(super) error: Option<String>,
    /// Agents → Race Engineer.
    pub(super) engineer: super::engineer::Section,
    /// Accounts: agent logins and git identities.
    pub(super) accounts: super::accounts::Section,
    /// Peers.
    pub(super) peers: super::peers::Section,
    pub(super) focus: FocusHandle,
    /// The page's parts (rebuilt each render: what builds them, not
    /// their elements) and the list drawing them.
    list: ListState,
    body: Entity<PageBody>,
    pub(super) parts: Rc<Vec<Part>>,
    /// The list's items' keys (the title, the parts, the room under them).
    keys: Vec<u64>,
    /// The page the list holds.
    shown: Option<Page>,
    /// Settings changed since the parts were last measured.
    changed: bool,
    /// Each item's focus handle: a field in a part keeps it drawn while it
    /// has focus, scrolled out of view or not (a field not drawn loses it).
    part_focus: HashMap<u64, FocusHandle>,
    /// Parts built (tests).
    #[cfg(test)]
    pub(super) built: usize,
    _tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// "⌘K" as this desktop writes it (Ctrl+Shift elsewhere).
pub fn keys(s: &str) -> String {
    if cfg!(target_os = "macos") {
        s.into()
    } else {
        s.replace('⌘', "Ctrl+Shift+")
    }
}

/// The page before or after `p` (wrapping).
pub fn step(p: Page, delta: i32) -> Page {
    let all = Page::ALL;
    let i = all.iter().position(|x| *x == p).unwrap_or(0) as i32;
    all[(i + delta).rem_euclid(all.len() as i32) as usize]
}

impl SettingsView {
    pub fn new(live: bool, cx: &mut Context<Self>) -> SettingsView {
        let mut tasks = Vec::new();
        if let Some(t) = Self::read_hooks(cx) {
            tasks.push(t);
        }
        tasks.push(Self::read_cli(cx));
        // Full Disk Access, live while Settings is open.
        tasks.push(cx.spawn(async move |this, cx| loop {
            let s = cx
                .background_executor()
                .spawn(async { pitwall_core::permissions::status() })
                .await;
            let alive = this.update(cx, |v, cx| {
                if v.perms.as_ref() != Some(&s) {
                    v.perms = Some(s);
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
            cx.background_executor().timer(POLL).await;
        }));
        let store = crate::ui_state::store(cx);
        let outside = super::backend::outside(cx);
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            super::accounts::observe(cx),
            // A change, not a scroll step (those notify the page's view).
            cx.observe_self(|v, _| v.changed = true),
            // `pitwall settings` installed hooks or the CLI: read again.
            cx.observe(&outside, |v, _, cx| {
                if let Some(t) = Self::read_hooks(cx) {
                    v._tasks.push(t);
                }
                let t = Self::read_cli(cx);
                v._tasks.push(t);
            }),
        ];
        // Parts a little past the edges are measured, not drawn; each is
        // measured once, so the scrollbar is right from the start.
        let list = ListState::new(0, ListAlignment::Top, px(160.)).measure_all();
        let view = cx.weak_entity();
        let body = cx.new(|_| PageBody { view, list: list.clone() });
        SettingsView {
            live,
            hooks: None,
            hooks_confirm: false,
            hooks_busy: false,
            perms: None,
            fda_open: false,
            cli: None,
            cli_confirm: false,
            cli_dir: None,
            cli_busy: false,
            error: None,
            engineer: Default::default(),
            accounts: Default::default(),
            peers: Default::default(),
            focus: cx.focus_handle(),
            list,
            body,
            parts: Rc::default(),
            keys: Vec::new(),
            shown: None,
            changed: true,
            part_focus: HashMap::new(),
            #[cfg(test)]
            built: 0,
            _tasks: tasks,
            _subs: subs,
        }
    }

    /// Codex hooks status (read-only).
    fn read_hooks(cx: &mut Context<Self>) -> Option<Task<()>> {
        let engine = cx.try_global::<SettingsHost>().and_then(|h| h.engine.clone())?;
        let read = cx
            .background_executor()
            .spawn(async move { pitwall_core::hooks::codex_status(engine.paths()) });
        Some(cx.spawn(async move |this, cx| {
            let s = read.await;
            let _ = this.update(cx, |v, cx| {
                v.hooks = Some(s);
                cx.notify();
            });
        }))
    }

    /// The command-line tool.
    fn read_cli(cx: &mut Context<Self>) -> Task<()> {
        let read = cx
            .background_executor()
            .spawn(async { cli_install::current() });
        cx.spawn(async move |this, cx| {
            let s = read.await;
            let _ = this.update(cx, |v, cx| {
                if v.cli_dir.is_none() {
                    v.cli_dir = s.dirs.first().map(|d| d.path.clone());
                }
                v.cli = Some(s);
                cx.notify();
            });
        })
    }

    pub(super) fn install_hooks(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = cx
            .try_global::<SettingsHost>()
            .and_then(|h| h.engine.clone())
        else {
            return;
        };
        self.hooks_busy = true;
        cx.notify();
        let run = cx
            .background_executor()
            .spawn(async move { pitwall_core::hooks::install_codex(engine.paths()) });
        cx.spawn(async move |this, cx| {
            let r = run.await;
            let _ = this.update(cx, |v, cx| {
                v.hooks_busy = false;
                match r {
                    Ok(s) => {
                        v.hooks = Some(s);
                        v.hooks_confirm = false;
                    }
                    Err(e) => v.error = Some(format!("Couldn't install Codex hooks: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn install_cli(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.cli_dir.clone() else {
            return;
        };
        self.cli_busy = true;
        cx.notify();
        let run = cx
            .background_executor()
            .spawn(async move { cli_install::install_into(&dir) });
        cx.spawn(async move |this, cx| {
            let r = run.await;
            let _ = this.update(cx, |v, cx| {
                v.cli_busy = false;
                match r {
                    Ok(s) => {
                        v.cli = Some(s);
                        v.cli_confirm = false;
                    }
                    Err(e) => {
                        v.error = Some(format!("Couldn't install the command-line tool: {e}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn go(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if data::page(cx) != page {
            // The list starts at the top of the new page (`sync`).
            data::set_page(cx, page);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn prev_page(&mut self, _: &PrevPage, window: &mut Window, cx: &mut Context<Self>) {
        let p = step(data::page(cx), -1);
        self.go(p, window, cx);
    }

    fn next_page(&mut self, _: &NextPage, window: &mut Window, cx: &mut Context<Self>) {
        let p = step(data::page(cx), 1);
        self.go(p, window, cx);
    }

    /// Make the page's parts again and tell the list what changed: a new
    /// page starts at its top; parts that came or went are spliced in;
    /// after a change every part is measured again. A scroll step changes
    /// nothing here (only the parts on screen are built, by [`Self::part`]).
    fn sync(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let parts = self.page(page, cx);
        let keys: Vec<u64> = std::iter::once(HEAD)
            .chain(parts.iter().map(|p| p.key))
            .chain(std::iter::once(FOOT))
            .collect();
        self.parts = Rc::new(parts);
        if self.shown != Some(page) {
            self.shown = Some(page);
            self.changed = false;
            let handles: Vec<_> = keys.iter().map(|k| Some(self.focus_of(*k, window, cx))).collect();
            self.list.reset(0);
            self.list.splice_focusable(0..0, handles);
            self.keys = keys;
            return;
        }
        let old = &self.keys;
        let pre = old.iter().zip(&keys).take_while(|(a, b)| a == b).count();
        let most = old.len().min(keys.len()) - pre;
        let suf = old.iter().rev().zip(keys.iter().rev()).take(most).take_while(|(a, b)| a == b).count();
        let gone = pre..old.len() - suf;
        let came = pre..keys.len() - suf;
        if !gone.is_empty() || !came.is_empty() {
            let handles: Vec<_> = keys[came.clone()].iter().map(|k| Some(self.focus_of(*k, window, cx))).collect();
            self.list.splice_focusable(gone, handles);
            self.list.remeasure_items(came);
        }
        if std::mem::take(&mut self.changed) {
            self.list.remeasure_items(0..keys.len());
        }
        self.keys = keys;
    }

    /// The focus handle of the list item `key`. It isn't one to stop at:
    /// a click on a part's background (not on a field) gives focus back to
    /// the page list, as before the parts had handles.
    fn focus_of(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) -> FocusHandle {
        if let Some(h) = self.part_focus.get(&key) {
            return h.clone();
        }
        let h = cx.focus_handle();
        cx.on_focus(&h, window, |v, window, cx| window.focus(&v.focus, cx)).detach();
        self.part_focus.insert(key, h.clone());
        h
    }

    /// List item `ix`: the page's title, a part, or the room under them.
    fn part(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let ui = Ui::new(appearance(cx).theme().float());
        let t = ui.t.clone();
        let n = self.parts.len();
        let el = if ix == 0 {
            let page = data::page(cx);
            div()
                .pt(px(16.))
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(crate::kit::label(page.label(), t.text_3, 12.))
                .when_some(self.error.clone(), |b, e| {
                    b.child(
                        div()
                            .id("settings-error")
                            .flex()
                            .gap(px(8.))
                            .child(crate::kit::error_text(e, &t).flex_1())
                            .child(
                                crate::kit::text_button("err-x", "Dismiss", BtnKind::Link, false, &t)
                                    .on_click(cx.listener(|v, _, _, cx| {
                                        v.error = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                })
                .into_any_element()
        } else if ix > n {
            div().h(px(20.)).into_any_element()
        } else {
            let p = self.parts[ix - 1].clone();
            #[cfg(test)]
            {
                self.built += 1;
            }
            let el = (p.build)(self, &ui, window, cx);
            let d = div().flex().flex_col();
            if p.cont {
                d.pt(px(8.)).child(el)
            } else if ix == 1 {
                d.pt(px(14.)).child(el)
            } else {
                // The hairline between sections, 14 px each side.
                d.pt(px(14.)).child(div().flex().flex_col().pt(px(14.)).border_t_1().border_color(t.line).child(el))
            }
            .into_any_element()
        };
        let focus = self.keys.get(ix).and_then(|k| self.part_focus.get(k)).cloned();
        div()
            .w_full()
            .px(px(22.))
            .flex()
            .flex_col()
            .when_some(focus, |d, f| d.track_focus(&f))
            .child(el)
            .into_any_element()
    }

    fn nav(&self, ui: &Ui, page: Page, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = &ui.t;
        let (hover_bg, on_bg) = if t.is_glass() {
            (t.g.surface_2, t.g.surface_3)
        } else {
            (t.surface_2, t.surface_3)
        };
        div()
            .id("settings-nav")
            .track_focus(&self.focus)
            .key_context(NAV_CONTEXT)
            .on_action(cx.listener(Self::prev_page))
            .on_action(cx.listener(Self::next_page))
            .w(px(NAV_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(1.))
            .px(px(8.))
            .py(px(8.))
            .border_r_1()
            .border_color(t.line)
            .children(Page::ALL.into_iter().enumerate().map(|(i, p)| {
                let on = p == page;
                div()
                    .id(("settings-page", i))
                    .flex()
                    .items_center()
                    .h(px(28.))
                    .px(px(10.))
                    .rounded(RADIUS_SM)
                    .text_size(px(13.))
                    .cursor_pointer()
                    .text_color(if on { t.text } else { t.text_2 })
                    .when(on, |d| d.bg(on_bg).font_weight(crate::kit::WEIGHT_550))
                    .when(!on, |d| d.hover_text(t.text, |s| s.bg(hover_bg)))
                    .on_click(cx.listener(move |v, _, window, cx| v.go(p, window, cx)))
                    .child(p.label())
            }))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ap = appearance(cx);
        let ui = Ui::new(ap.theme().float());
        let t = ui.t.clone();
        let page = data::page(cx);
        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .pl(px(18.))
            .pr(px(12.))
            .pt(px(14.))
            .pb(px(10.))
            .border_b_1()
            .border_color(t.line)
            // `.label-lg`: Barlow Condensed 600, 15 px, 0.08em.
            .child(crate::kit::label_t("Settings", t.text, 15., 0.08))
            .child(
                crate::kit::glyph_btn("settings-close", "✕", &ui.t)
                    .tooltip(tooltip("Close (esc)"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
            );
        self.sync(page, window, cx);
        let body = div()
            .id("settings-page-body")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(self.body.clone());
        div()
            .flex()
            .flex_col()
            .h(px(HEIGHT))
            .max_h_full()
            .min_h_0()
            .child(head)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.nav(&ui, page, cx))
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_follow_the_desktop() {
        if cfg!(target_os = "macos") {
            assert_eq!(keys("⌘K"), "⌘K");
        } else {
            assert_eq!(keys("⌘+ / ⌘0"), "Ctrl+Shift++ / Ctrl+Shift+0");
        }
    }

    #[test]
    fn arrows_move_between_pages_and_wrap() {
        assert_eq!(step(Page::General, 1), Page::Agents);
        assert_eq!(step(Page::General, -1), Page::About);
        assert_eq!(step(Page::About, 1), Page::General);
        assert_eq!(step(Page::Rules, -1), Page::Accounts);
    }

    #[test]
    fn glass_is_named_per_desktop() {
        assert_eq!(crate::theme::GlassOffer::Mica.label(), "Glass · Mica");
    }
}
