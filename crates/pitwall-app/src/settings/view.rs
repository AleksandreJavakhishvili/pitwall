//! The Settings dialog: a compact page list on the left and one page on
//! the right (General, Agents, Rules, Appearance, Folder access, Peers, About;
//! `pitwall_proto::settings::Page`). Every option keeps what the React
//! dialog (`src/components/SettingsDialog.tsx` and its parts) does; the
//! page shown last is remembered in `ui.json` (`settingsPage`). ↑/↓ move
//! between pages; Esc, ✕ and a click outside close it.
//!
//! The pages themselves are in [`super::pages`].
//!
//! The same view is also drawn in a tile of the layout, or a window of its
//! own (`crate::main_screen::settings_tile`, docs/spec/layout.md "The
//! Settings tile"): without its header (the tile's bar stands in), at the
//! tile's full size, and in a narrow tile with the page list as a row of
//! pages over the page instead of a column beside it.
//!
//! A page is drawn as a list of [`Part`]s (its sections, and the rows of
//! a long one) by `gpui::list`, in a view of its own ([`PageBody`]): a
//! scroll step notifies that view and builds, lays out and paints only the
//! parts on screen, not the whole page (docs/spec/perf.md "Settings").
//! Every part is measured again when Settings or what it shows changes.

use std::collections::hash_map::DefaultHasher;
use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    actions, canvas, div, prelude::*, px, AnyElement, App, Context, Entity, EventEmitter, FocusHandle,
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
use crate::theme::RADIUS_SM;

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
/// In a tile narrower than this, the page list is a row over the page.
pub const NARROW: f32 = 560.;
/// The Extensions surface at least this wide has its list beside the page.
pub const WIDE: f32 = 900.;
/// The list's width then.
pub const LIST_WIDTH: f32 = 360.;
/// The dialog at its smallest.
pub const MIN_WIDTH: f32 = 720.;
pub const MIN_HEIGHT: f32 = 540.;
/// The `ui.json` key of the dialog's size, `[width, height]`.
pub const SIZE_KEY: &str = "settingsSize";

/// The dialog's size: as the user last left it, else scaled to the
/// window (`window`: its size), always within it and never under the
/// smallest.
pub fn dialog_size(window: (f32, f32), cx: &App) -> (f32, f32) {
    let kept = crate::ui_state::get(cx).rest.get(SIZE_KEY).and_then(|v| v.as_array().cloned()).and_then(|a| Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)));
    let (w, h) = kept.unwrap_or(((window.0 * 0.72).clamp(860., 1080.), (window.1 * 0.78).clamp(600., 820.)));
    fit_size((w, h), window)
}

/// `size` within the window (a margin around it), never under the
/// smallest.
pub fn fit_size(size: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    let (mw, mh) = ((window.0 - 48.).max(MIN_WIDTH), (window.1 - 48.).max(MIN_HEIGHT));
    (size.0.clamp(MIN_WIDTH, mw), size.1.clamp(MIN_HEIGHT, mh))
}

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::kit::pop::theme_in(window, cx);
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
    /// The dialog's "Open as a tile": this same view (its unsaved fields
    /// with it) goes into a tile of the layout and the dialog closes.
    ToTile,
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
    /// Agents → how each kind shows files; the kind whose MCP file change
    /// is being confirmed or made.
    pub(super) show_routes: Vec<pitwall_core::show::KindRoute>,
    pub(super) show_confirm: Option<String>,
    pub(super) show_busy: bool,
    pub(super) error: Option<String>,
    /// Agents → Race Engineer.
    pub(super) engineer: super::engineer::Section,
    /// Extensions.
    pub(super) exts: super::extensions::Section,
    /// Accounts: agent logins and git identities.
    pub(super) accounts: super::accounts::Section,
    pub(super) profiles: super::profiles::Section,
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
    /// Drawn in a tile or a window of its own (no header, its full size).
    tiled: bool,
    /// The Extensions surface (`main_screen::extensions_tile`): the
    /// Extensions page alone, no page list, at full size; wide, its list
    /// beside the page.
    pub(super) surface: bool,
    /// The dialog's and the Settings tile's Extensions page: the compact
    /// installed list and "Open Extensions" (the surface has the rest).
    pub(super) compact_exts: bool,
    /// The dialog's size while its corner is dragged.
    dragging_size: Option<(f32, f32)>,
    /// The dialog: sized to its window, its corner resizing it (a view
    /// made for a test keeps the fixed size).
    pub(super) sized: bool,
    /// Its width as last drawn (`-1`: not yet), for a narrow tile.
    width: Rc<Cell<f32>>,
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

/// The page before or after `p` among `all` (wrapping).
pub fn step(all: &[Page], p: Page, delta: i32) -> Page {
    let i = all.iter().position(|x| *x == p).unwrap_or(0) as i32;
    all[(i + delta).rem_euclid(all.len() as i32) as usize]
}

/// The pages in the nav: Profiles only once there is something to bundle
/// or a profile exists (or it is the page open).
pub fn shown_pages(cx: &gpui::App) -> Vec<Page> {
    let open = data::page(cx);
    Page::ALL.into_iter().filter(|p| *p != Page::Profiles || open == Page::Profiles || SettingsView::profiles_useful(cx)).collect()
}

impl SettingsView {
    /// The page scrolled to its end (bench commands).
    pub fn scroll_to_end(&self) {
        let n = self.list.item_count();
        self.list.scroll_to_reveal_item(n.saturating_sub(1));
    }

    /// The page scrolled to its top (a detail page opened or closed).
    pub(super) fn scroll_to_top(&self) {
        self.list.scroll_to(gpui::ListOffset { item_ix: 0, offset_in_item: px(0.) });
    }

    /// The part known by `key` brought into view (the Extensions page's
    /// selection).
    pub(super) fn reveal_part(&self, key: impl Hash) {
        let k = hash(key);
        if let Some(ix) = self.keys.iter().position(|x| *x == k) {
            self.list.scroll_to_reveal_item(ix);
        }
    }

    /// The page scrolled by `dy` (tests and the bench's `settings-scroll`:
    /// a control brought into view).
    pub fn scroll_by(&self, dy: gpui::Pixels) {
        self.list.scroll_by(dy);
    }

    pub fn new(live: bool, cx: &mut Context<Self>) -> SettingsView {
        let mut tasks = Vec::new();
        if let Some(t) = Self::read_hooks(cx) {
            tasks.push(t);
        }
        tasks.push(Self::read_cli(cx));
        if let Some(t) = Self::read_show_routes(cx) {
            tasks.push(t);
        }
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
                if let Some(t) = Self::read_show_routes(cx) {
                    v._tasks.push(t);
                }
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
            show_routes: Vec::new(),
            show_confirm: None,
            show_busy: false,
            error: None,
            engineer: Default::default(),
            exts: Default::default(),
            accounts: Default::default(),
            profiles: Default::default(),
            peers: Default::default(),
            focus: cx.focus_handle(),
            list,
            body,
            parts: Rc::default(),
            keys: Vec::new(),
            shown: None,
            changed: true,
            part_focus: HashMap::new(),
            tiled: false,
            surface: false,
            compact_exts: false,
            dragging_size: None,
            sized: false,
            width: Rc::new(Cell::new(-1.)),
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

    /// How each agent kind shows files (its MCP file read off the UI
    /// thread).
    fn read_show_routes(cx: &mut Context<Self>) -> Option<Task<()>> {
        let engine = cx.try_global::<SettingsHost>().and_then(|h| h.engine.clone())?;
        let read = cx.background_executor().spawn(async move { engine.show_files_routes() });
        Some(cx.spawn(async move |this, cx| {
            let r = read.await;
            let _ = this.update(cx, |v, cx| {
                v.show_routes = r;
                cx.notify();
            });
        }))
    }

    /// Add Pitwall's server to `kind`'s MCP file, or take it out.
    pub(super) fn set_show_file(&mut self, kind: String, add: bool, cx: &mut Context<Self>) {
        let Some(engine) = cx.try_global::<SettingsHost>().and_then(|h| h.engine.clone()) else {
            return;
        };
        self.show_busy = true;
        cx.notify();
        let run = cx.background_executor().spawn(async move {
            let r = engine.set_show_files_file(&kind, add);
            (r, engine.show_files_routes())
        });
        cx.spawn(async move |this, cx| {
            let (r, routes) = run.await;
            let _ = this.update(cx, |v, cx| {
                v.show_busy = false;
                v.show_confirm = None;
                v.show_routes = routes;
                if let Err(e) = r {
                    v.error = Some(format!("Couldn't change the MCP file: {e}"));
                }
                cx.notify();
            });
        })
        .detach();
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

    /// Drawn in a tile (or a window of its own), or in the dialog.
    pub fn set_tiled(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.tiled != on {
            self.tiled = on;
            self.width.set(-1.);
            cx.notify();
        }
    }

    pub fn tiled(&self) -> bool {
        self.tiled
    }

    /// The Extensions surface: Settings → Extensions at full size, alone
    /// (docs/spec/layout.md "The Extensions tile").
    pub fn extensions_surface(cx: &mut Context<Self>) -> SettingsView {
        let mut v = SettingsView::new(true, cx);
        v.surface = true;
        v.tiled = true;
        v
    }

    /// Whether this is the Extensions surface.
    pub fn is_surface(&self) -> bool {
        self.surface
    }

    /// The Extensions page compact here (the dialog, the Settings tile).
    pub fn set_compact_extensions(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.compact_exts != on {
            self.compact_exts = on;
            cx.notify();
        }
    }

    /// The Extensions surface wide enough for its list beside the page.
    pub fn wide(&self) -> bool {
        self.surface && self.width.get() >= WIDE
    }

    /// The dialog's size now: while its corner is dragged, where it is;
    /// else as kept (`window`: the window's size).
    pub fn dialog_size(&self, window: (f32, f32), cx: &App) -> (f32, f32) {
        self.dragging_size.unwrap_or_else(|| dialog_size(window, cx))
    }

    /// The corner let go: the size kept in `ui.json` for next time.
    fn keep_size(&mut self, cx: &mut Context<Self>) {
        if let Some((w, h)) = self.dragging_size.take() {
            crate::ui_state::update(cx, |s| {
                s.rest.insert(SIZE_KEY.into(), serde_json::json!([w.round(), h.round()]));
            });
            cx.notify();
        }
    }

    /// The page this view shows: the Extensions surface only Extensions.
    pub(super) fn page_shown(&self, cx: &App) -> Page {
        if self.surface {
            Page::Extensions
        } else {
            data::page(cx)
        }
    }

    /// In a tile too narrow for the page list beside the page.
    pub fn narrow(&self) -> bool {
        self.tiled && (0. ..NARROW).contains(&self.width.get())
    }

    /// Extension `id`'s card unfolded on the Extensions page (an agent's
    /// `show_settings` naming it).
    pub fn open_extension(&mut self, id: &str, cx: &mut Context<Self>) {
        // Compact here: its page in the Extensions tile.
        if self.compact_exts {
            let id = id.to_string();
            self.exts.page.cursor = Some(id.clone());
            self.in_surface_quietly(move |v, cx| v.open_extension(&id, cx), cx);
            return;
        }
        self.exts_open(id);
        self.show_extension_detail(Some(id), cx);
    }

    pub(super) fn go_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.go(page, window, cx);
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
        let p = step(&shown_pages(cx), data::page(cx), -1);
        self.go(p, window, cx);
    }

    fn next_page(&mut self, _: &NextPage, window: &mut Window, cx: &mut Context<Self>) {
        let p = step(&shown_pages(cx), data::page(cx), 1);
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
        let ui = Ui::new(crate::kit::pop::theme_in(window, cx));
        let t = ui.t.clone();
        let n = self.parts.len();
        let el = if ix == 0 {
            let page = self.page_shown(cx);
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

    /// The page list: a column beside the page, or (`row`: a narrow tile)
    /// the pages wrapped in rows over it.
    fn nav(&self, ui: &Ui, page: Page, row: bool, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = &ui.t;
        let (hover_bg, on_bg) = if t.is_glass() {
            (t.g.surface_2, t.g.surface_3)
        } else {
            (t.surface_2, t.surface_3)
        };
        div()
            .id("settings-nav")
            .debug_selector(|| "settings-nav".into())
            .track_focus(&self.focus)
            .key_context(NAV_CONTEXT)
            .on_action(cx.listener(Self::prev_page))
            .on_action(cx.listener(Self::next_page))
            .map(|d| super::extensions::page::on_nav(d, cx))
            .flex_none()
            .flex()
            .gap(px(1.))
            .px(px(8.))
            .py(px(8.))
            .border_color(t.line)
            .map(|d| if row { d.w_full().flex_row().flex_wrap().border_b_1() } else { d.w(px(NAV_WIDTH)).flex_col().border_r_1() })
            .children(shown_pages(cx).into_iter().enumerate().map(|(i, p)| {
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
                    // (After the hover: a test build's probe names it too.)
                    .debug_selector(move || format!("settings-page-{}", p.id()))
                    .on_click(cx.listener(move |v, _, window, cx| v.go(p, window, cx)))
                    .child(p.label())
            }))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = Ui::new(crate::kit::pop::theme_in(window, cx));
        let t = ui.t.clone();
        let page = self.page_shown(cx);
        // "Open as a tile": only where there is a layout (an engine runs).
        let to_tile = self.live.then(|| {
            crate::kit::icon_btn("settings-to-tile", "grid", "Open as a tile in the layout", true, &ui.t)
                .debug_selector(|| "settings-to-tile".into())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::ToTile)))
        });
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
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .children(to_tile)
                    .child(
                        crate::kit::glyph_btn("settings-close", "✕", &ui.t)
                            .tooltip(tooltip("Close (esc)"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                    ),
            );
        self.sync(page, window, cx);
        let body = div()
            .id("settings-page-body")
            .debug_selector(|| "settings-page-body".into())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(self.body.clone());
        if self.tiled {
            // Its full size, measured: a narrow tile has the page list
            // over the page.
            let width = self.width.clone();
            let me = cx.entity().downgrade();
            let measure = canvas(
                move |b, _, cx| {
                    let w = f32::from(b.size.width);
                    if (width.get() - w).abs() > 0.5 {
                        let was = (0. ..NARROW).contains(&width.get());
                        let was_wide = width.get() >= WIDE;
                        width.set(w);
                        if was != (w < NARROW) || was_wide != (w >= WIDE) {
                            // Drawn again at its width (a view is redrawn
                            // only when it says so).
                            let me = me.clone();
                            cx.defer(move |cx| {
                                let _ = me.update(cx, |_, cx| cx.notify());
                            });
                        }
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full();
            if self.surface {
                // The Extensions surface: no page list; wide, the list
                // beside the page.
                let list = self.wide().then(|| self.ext_list_column(&ui, window, cx));
                return div()
                    .id("extensions-surface")
                    .debug_selector(|| "extensions-surface".into())
                    // Its keys are here, as the page list's are in
                    // Settings: drawn, so the tile's keys (⌘⏎, ⌘F) reach
                    // it and the screen around it.
                    .track_focus(&self.focus)
                    .key_context(NAV_CONTEXT)
                    .map(|d| super::extensions::page::on_nav(d, cx))
                    .relative()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .child(measure)
                    .children(list)
                    .child(body)
                    .into_any_element();
            }
            let narrow = self.narrow();
            return div()
                .id("settings-tiled")
                .debug_selector(|| "settings-tiled".into())
                .relative()
                .size_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .when(narrow, |d| d.flex_col())
                .child(measure)
                .child(self.nav(&ui, page, narrow, cx))
                .child(body)
                .into_any_element();
        }
        // The dialog: as large as the user left it (its corner dragged),
        // else scaled to the window.
        let vp = window.viewport_size();
        let h = if self.sized { self.dialog_size((f32::from(vp.width), f32::from(vp.height)), cx).1 } else { HEIGHT };
        let grip = crate::kit::resize_grip("settings-resize", t.text_4)
            .debug_selector(|| "settings-resize".into())
            .cursor_nwse_resize()
            .tooltip(tooltip("Drag to resize Settings"));
        div()
            .id("settings-dialog")
            .relative()
            .flex()
            .flex_col()
            .h(px(h))
            .max_h_full()
            .min_h_0()
            .on_drag_move::<crate::kit::GripDrag>(cx.listener(|v, e: &gpui::DragMoveEvent<crate::kit::GripDrag>, window, cx| {
                // The box is centred: it grows both ways from its middle.
                let b = e.bounds;
                let mid = b.center();
                let p = e.event.position;
                let vp = window.viewport_size();
                let size = fit_size((f32::from(p.x - mid.x) * 2., f32::from(p.y - mid.y) * 2.), (f32::from(vp.width), f32::from(vp.height)));
                v.dragging_size = Some(size);
                cx.notify();
            }))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(|v, _, _, cx| v.keep_size(cx)))
            .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(|v, _, _, cx| v.keep_size(cx)))
            .when(self.sized, |d| d.child(grip))
            .child(head)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.nav(&ui, page, false, cx))
                    .child(body),
            )
            .into_any_element()
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
        assert_eq!(step(&Page::ALL, Page::General, 1), Page::Agents);
        assert_eq!(step(&Page::ALL, Page::General, -1), Page::About);
        assert_eq!(step(&Page::ALL, Page::About, 1), Page::General);
        assert_eq!(step(&Page::ALL, Page::Rules, -1), Page::Extensions);
        assert_eq!(step(&Page::ALL, Page::Accounts, 1), Page::Extensions);
        // Profiles hidden: Agents is next to Accounts.
        let shown: Vec<Page> = Page::ALL.into_iter().filter(|p| *p != Page::Profiles).collect();
        assert_eq!(step(&shown, Page::Agents, 1), Page::Accounts);
        assert_eq!(step(&Page::ALL, Page::Agents, 1), Page::Profiles);
    }

    #[test]
    fn glass_is_named_per_desktop() {
        assert_eq!(crate::theme::GlassOffer::Mica.label(), "Glass · Mica");
    }
}
