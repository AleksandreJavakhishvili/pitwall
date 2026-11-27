//! Settings → Extensions as a page with tabs (docs/spec/tile-sdk/
//! marketplace.md §7.4): **Installed** and **Browse** ([`super::browse`]).
//!
//! Installed: a search (word-and-substring, as the palette matches), the
//! filters (on or off, an update available, needs attention, has tiles,
//! agent tools or a service), the order (name, recently updated), and the
//! extensions as their cards (everything each card always did) or as
//! compact rows. ↑/↓ in the search move the selection, Enter opens it,
//! Esc clears the search; ⌘F focuses the search.
//!
//! An extension's **detail page**: its card at full size with who signed
//! it (and Verified, when the index says so) under its name, then its
//! README (Pitwall's inert Markdown Preview: no HTML, no remote images),
//! screenshots, permissions and what it adds while it is off (on, the
//! card has them with their switches), versions with the changelog, the
//! licence and repository.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{actions, div, img, prelude::*, px, AnyElement, App, Context, Div, Entity, KeyBinding, ObjectFit, SharedString, Window};
use pitwall_core::extensions::install::{self, Origin};
use pitwall_core::extensions::{package, Manifest};

use super::super::view::{Part, SettingsView, NAV_CONTEXT};
use super::super::widgets::Ui;
use crate::extensions::catalog;
use crate::kit::{BtnKind, InputEvent, SegItem, TextInput, Tone};
use crate::markdown_view::MarkdownView;

actions!(pw_extensions, [SelectPrev, SelectNext, FindExtension]);

/// The key context of the page's search fields.
pub const SEARCH_CONTEXT: &str = "ExtensionsSearch";

pub fn bindings() -> Vec<KeyBinding> {
    let find = if cfg!(target_os = "macos") { "cmd-f" } else { "ctrl-f" };
    vec![
        KeyBinding::new("up", SelectPrev, Some(SEARCH_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(SEARCH_CONTEXT)),
        KeyBinding::new(find, FindExtension, Some(SEARCH_CONTEXT)),
        KeyBinding::new(find, FindExtension, Some(NAV_CONTEXT)),
    ]
}

/// The page's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Installed,
    Browse,
}

/// On, off or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnOff {
    #[default]
    All,
    On,
    Off,
}

/// The order of the installed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Name,
    Updated,
}

/// The installed list's filters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub on: OnOff,
    pub updates: bool,
    pub attention: bool,
    pub tiles: bool,
    pub tools: bool,
    pub services: bool,
}

/// What an installed extension is, for the filters and the order.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub on: bool,
    pub update: bool,
    /// Why it needs the user, in a few words.
    pub attention: Option<String>,
    pub tiles: bool,
    pub tools: bool,
    pub service: bool,
    pub updated: Option<SystemTime>,
}

/// The files of an installed extension's folder its detail page shows.
#[derive(Debug, Clone, Default)]
pub struct Files {
    pub folder: Option<PathBuf>,
    pub readme: Option<Arc<str>>,
    pub changelog: Option<Arc<str>>,
    /// The licence's file and its first line ("MIT License").
    pub licence: Option<(PathBuf, String)>,
    pub screenshots: Vec<PathBuf>,
    pub installed_at: Option<SystemTime>,
}

impl Files {
    /// Read `folder` (small files only).
    pub fn read(folder: Option<PathBuf>) -> Files {
        let Some(dir) = folder else { return Files::default() };
        let text = |name: &str| -> Option<Arc<str>> {
            let p = dir.join(name);
            let meta = std::fs::metadata(&p).ok()?;
            if !meta.is_file() || meta.len() > package::MAX_TEXT_BYTES {
                return None;
            }
            std::fs::read_to_string(p).ok().map(Arc::from)
        };
        let licence = ["LICENSE", "LICENSE.md", "LICENSE.txt"].into_iter().find_map(|n| {
            let t = text(n)?;
            let first = t.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_start_matches('#').trim().chars().take(80).collect::<String>();
            Some((dir.join(n), first))
        });
        let mut screenshots: Vec<PathBuf> = std::fs::read_dir(dir.join("screenshots"))
            .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("webp"))).collect())
            .unwrap_or_default();
        screenshots.sort();
        screenshots.truncate(package::MAX_SCREENSHOTS);
        let installed_at = std::fs::metadata(dir.join(install::MARKER)).or_else(|_| std::fs::metadata(&dir)).and_then(|m| m.modified()).ok();
        Files { readme: text(package::README), changelog: text("CHANGELOG.md"), licence, screenshots, installed_at, folder: Some(dir) }
    }
}

/// The page's own state.
#[derive(Default)]
pub struct State {
    pub tab: Tab,
    /// The installed extension whose detail page is open.
    pub detail: Option<String>,
    pub search: Option<Entity<TextInput>>,
    pub query: String,
    pub filters: Filters,
    pub sort: Sort,
    pub sort_open: bool,
    /// Compact rows (else cards).
    pub rows: bool,
    /// The selected extension (↑/↓).
    pub cursor: Option<String>,
    /// Each extension's folder files, read again when the folder is.
    files: Option<(usize, HashMap<String, Files>)>,
    /// READMEs and changelogs drawn, by `<id>/<file>`.
    md: HashMap<String, Entity<MarkdownView>>,
    pub browse: super::browse::Browse,
}

/// A small pill that is on or off (a filter).
pub(super) fn filter_chip(id: String, label: impl Into<SharedString>, on: bool, ui: &Ui) -> gpui::Stateful<Div> {
    let sel = id.clone();
    let d = div()
        .id(SharedString::from(id))
        .debug_selector(move || sel)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(24.))
        .px(px(9.))
        .rounded(px(12.))
        .border_1()
        .cursor_pointer()
        .text_size(px(12.));
    let d = if on { d.bg(ui.t.surface_3).border_color(ui.t.line_strong).text_color(ui.t.text).child(crate::kit::icon("check", 11., ui.t.text)) } else { d.border_color(ui.t.line).text_color(ui.t.text_2).hover(|d| d.bg(ui.t.surface_2)) };
    d.child(label.into())
}

/// The search field's box: a search icon, the field, ⌘F.
pub(super) fn search_box(input: Entity<TextInput>, ui: &Ui) -> Div {
    div()
        .flex_1()
        .min_w(px(160.))
        .h(px(30.))
        .pl(px(9.))
        .pr(px(6.))
        .flex()
        .items_center()
        .gap(px(7.))
        .rounded(crate::theme::RADIUS)
        .border_1()
        .border_color(ui.t.line_strong)
        .bg(ui.t.bg)
        .text_size(px(13.))
        .child(crate::kit::icon("search", 13., ui.t.text_3))
        .child(div().flex_1().min_w_0().child(input))
        .child(crate::kit::kbd(&super::super::view::keys("⌘F"), &ui.t))
}

/// A button tests find by its id.
pub(super) fn btn(id: String, text: impl Into<SharedString>, kind: BtnKind, disabled: bool, ui: &Ui) -> gpui::Stateful<Div> {
    let sel = id.clone();
    crate::kit::text_button(SharedString::from(id), text, kind, disabled, &ui.t).debug_selector(move || sel)
}

/// A detail page's section title.
pub(super) fn title(ui: &Ui, text: impl Into<SharedString>) -> Div {
    div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(text.into())
}

/// "When", from a time: "today", "3 days ago", "on 2026-11-20".
fn when(at: SystemTime) -> String {
    super::updates::ago(at)
}

impl SettingsView {
    /// The page's search field for the tab shown, made once.
    fn search_input(&mut self, browse: bool, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        let have = if browse { self.exts.page.browse.search.clone() } else { self.exts.page.search.clone() };
        if let Some(i) = have {
            return i;
        }
        let placeholder = if browse { "Search the index: name, author, category" } else { "Search installed extensions" };
        let input = cx.new(|cx| TextInput::new(cx, "", placeholder).bare());
        let sub = cx.subscribe_in(&input, window, move |v, input, e: &InputEvent, window, cx| {
            let text = input.read(cx).text().to_string();
            match e {
                InputEvent::Changed => {
                    if browse {
                        v.exts.page.browse.query.text = text;
                        v.exts.page.browse.cursor = None;
                    } else {
                        v.exts.page.query = text;
                        v.exts.page.cursor = None;
                    }
                    cx.notify();
                }
                InputEvent::Submit => v.open_selected(window, cx),
                InputEvent::Cancel => {
                    input.update(cx, |i, cx| i.set_text("", cx));
                }
                _ => {}
            }
        });
        self.exts._subs.push(sub);
        if browse {
            self.exts.page.browse.search = Some(input.clone());
        } else {
            self.exts.page.search = Some(input.clone());
        }
        input
    }

    /// ⌘F: the search of the tab shown, focused (a detail page closes).
    pub(crate) fn find_extension(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::settings::data::page(cx) != pitwall_proto::settings::Page::Extensions {
            return;
        }
        self.exts.page.detail = None;
        self.exts.page.browse.detail = None;
        let browse = self.exts.page.tab == Tab::Browse;
        let input = self.search_input(browse, window, cx);
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all_text(cx);
        });
        self.scroll_to_top();
        cx.notify();
    }

    /// The tab shown (tests and bench commands too).
    pub fn set_extensions_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.exts.page.tab = tab;
        self.exts.page.detail = None;
        self.exts.page.browse.detail = None;
        if tab == Tab::Browse {
            self.load_index(false, cx);
        }
        cx.notify();
    }

    /// Extension `id`'s detail page (`None`: the list).
    pub fn show_extension_detail(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        // Compact here: its page is in the Extensions tile.
        if let Some(id) = id.filter(|_| self.compact_exts) {
            let (id, open) = (id.to_string(), self.exts.open.get(id).copied().unwrap_or(false));
            self.exts.page.cursor = Some(id.clone());
            self.in_surface(
                move |v, cx| {
                    if open {
                        v.exts_open(&id);
                    }
                    v.show_extension_detail(Some(&id), cx)
                },
                cx,
            );
            return;
        }
        self.exts.page.detail = id.map(str::to_string);
        if let Some(id) = id {
            self.exts.page.tab = Tab::Installed;
            self.exts.page.cursor = Some(id.to_string());
            // Its index entry, for who wrote it and Verified.
            self.load_index(false, cx);
        }
        self.scroll_to_top();
        cx.notify();
    }

    /// `then` run on the Extensions tile's page, the tile opened (or shown)
    /// first; the dialog closes (the tile is where it went on).
    pub(in crate::settings) fn in_surface(&mut self, then: impl FnOnce(&mut SettingsView, &mut Context<SettingsView>) + 'static, cx: &mut Context<Self>) {
        if !self.tiled() {
            cx.emit(crate::settings::view::SettingsEvent::Close);
        }
        cx.defer(move |cx| crate::main_screen::extensions_tile::show(then, cx));
    }

    /// [`Self::in_surface`] for what the user didn't press (an agent, a
    /// tile): the Extensions tile placed without taking the keyboard;
    /// this page stays as it is.
    pub(in crate::settings) fn in_surface_quietly(&mut self, then: impl FnOnce(&mut SettingsView, &mut Context<SettingsView>) + 'static, cx: &mut Context<Self>) {
        cx.defer(move |cx| crate::main_screen::extensions_tile::show_quietly(then, cx));
    }

    /// The compact page (the dialog, the Settings tile): "Open
    /// Extensions", the search, and a row per installed extension; a row
    /// opens its page in the Extensions tile.
    pub(super) fn compact_parts(&mut self, cx: &mut Context<Self>) -> Vec<Part> {
        let mut v = vec![Part::section("ext-compact-head", |s, ui, w, cx| s.compact_head(ui, w, cx).into_any_element())];
        v.push(Part::row("ext-installed-empty", |s, ui, _, cx| s.installed_empty(ui, cx).map(|d| d.into_any_element()).unwrap_or_else(|| div().into_any_element())));
        for (_, folder) in self.installed_shown(cx) {
            let f2 = folder.clone();
            v.push(Part::row(("ext", folder), move |s, ui, _, cx| s.ext_row(&f2, ui, cx).into_any_element()));
        }
        v
    }

    fn compact_head(&mut self, ui: &Ui, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let n = crate::extensions::found(cx).len();
        let open = btn("extensions-open-surface".into(), "Open Extensions", BtnKind::Small, false, ui).tooltip(crate::kit::tooltip("Browse, details, permissions and versions, at full size in a tile")).on_click(cx.listener(|v, _, _, cx| v.in_surface(|_, _| {}, cx)));
        let input = self.search_input(false, window, cx);
        let search = div()
            .key_context(SEARCH_CONTEXT)
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(|v, _: &FindExtension, w, cx| v.find_extension(w, cx)))
            .child(search_box(input, ui));
        div()
            .debug_selector(|| "extensions-compact".into())
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().flex().items_center().gap(px(8.)).child(div().flex_1().child(title(ui, if n > 0 { format!("Installed  {n}") } else { "Installed".to_string() }))).child(open))
            .child(search)
    }

    /// The Extensions tile, wide: the page beside the list; nothing
    /// picked, what extensions are and those Pitwall ships.
    pub(super) fn wide_parts(&mut self, intro: Part, cx: &mut Context<Self>) -> Vec<Part> {
        let mut v = Vec::new();
        match self.exts.page.tab {
            Tab::Installed => match self.exts.page.detail.clone() {
                Some(id) => v.extend(self.installed_detail_parts(&id, cx)),
                None => {
                    v.push(intro);
                    v.push(Part::section("ext-available", |s, ui, _, cx| s.ext_available(ui, cx).into_any_element()));
                }
            },
            Tab::Browse => match self.exts.page.browse.detail.clone() {
                Some(id) => v.extend(self.listing_detail_parts(&id, cx)),
                None => {
                    v.push(intro);
                    v.push(Part::section("ext-pick", |_, ui, _, _| div().debug_selector(|| "extensions-pick".into()).child(crate::kit::muted("Pick an extension on the left to see what it does, what it may use and its versions.", &ui.t)).into_any_element()));
                }
            },
        }
        v
    }

    /// The Extensions tile, wide: the list column beside the page (the
    /// tabs, the search and filters, the rows or the index's cards).
    pub(in crate::settings) fn ext_list_column(&mut self, ui: &Ui, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut col = div().flex().flex_col().gap(px(8.)).child(self.ext_tabs(ui, window, cx));
        match self.exts.page.tab {
            Tab::Installed => {
                col = col.children(self.installed_empty(ui, cx));
                for (_, folder) in self.installed_shown(cx) {
                    col = col.child(self.ext_row(&folder, ui, cx));
                }
            }
            Tab::Browse => {
                self.load_index(false, cx);
                col = col.child(self.browse_state(ui, cx));
                for id in self.listings_shown(cx) {
                    col = col.child(self.listing_card(&id, ui, cx));
                }
            }
        }
        div()
            .id("extensions-list")
            .debug_selector(|| "extensions-list".into())
            .flex_none()
            .w(px(crate::settings::view::LIST_WIDTH))
            .h_full()
            .min_h_0()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(ui.t.line)
            .px(px(14.))
            .py(px(14.))
            .text_size(px(13.))
            .text_color(ui.t.text)
            .child(col)
            .into_any_element()
    }

    /// Tests: the search typed (as the field would).
    pub fn search_extensions(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let browse = self.exts.page.tab == Tab::Browse;
        let input = self.search_input(browse, window, cx);
        input.update(cx, |i, cx| i.set_text(text.to_string(), cx));
        if browse {
            self.exts.page.browse.query.text = text.to_string();
        } else {
            self.exts.page.query = text.to_string();
        }
        cx.notify();
    }

    /// Tests and bench commands: the installed list's filters and order.
    pub fn set_extension_filters(&mut self, f: Filters, sort: Sort, rows: bool, cx: &mut Context<Self>) {
        self.exts.page.filters = f;
        self.exts.page.sort = sort;
        self.exts.page.rows = rows;
        cx.notify();
    }

    /// What the page shows now (tests): the tab, the detail page open, the
    /// installed list's ids in order, and the selection.
    pub fn extensions_page_state(&mut self, cx: &mut Context<Self>) -> (Tab, Option<String>, Vec<String>, Option<String>) {
        let ids = self.installed_shown(cx).into_iter().map(|(id, _)| id).collect();
        let detail = match self.exts.page.tab {
            Tab::Installed => self.exts.page.detail.clone(),
            Tab::Browse => self.exts.page.browse.detail.clone(),
        };
        let cursor = match self.exts.page.tab {
            Tab::Installed => self.exts.page.cursor.clone(),
            Tab::Browse => self.exts.page.browse.cursor.clone(),
        };
        (self.exts.page.tab, detail, ids, cursor)
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    /// ↑/↓: the selection moves through the list shown (wrapping), and
    /// comes into view.
    pub fn move_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        let browse = self.exts.page.tab == Tab::Browse;
        let ids: Vec<String> = if browse { self.listings_shown(cx) } else { self.installed_shown(cx).into_iter().map(|(id, _)| id).collect() };
        if ids.is_empty() {
            return;
        }
        let cur = if browse { &self.exts.page.browse.cursor } else { &self.exts.page.cursor };
        let i = match cur.as_ref().and_then(|c| ids.iter().position(|x| x == c)) {
            Some(i) => (i as i32 + delta).rem_euclid(ids.len() as i32) as usize,
            None if delta > 0 => 0,
            None => ids.len() - 1,
        };
        let id = ids[i].clone();
        if browse {
            self.exts.page.browse.cursor = Some(id.clone());
        } else {
            self.exts.page.cursor = Some(id.clone());
        }
        let key = if browse { super::browse::part_key(&id) } else { ("ext", id) };
        self.reveal_part(key);
        cx.notify();
    }

    /// Enter: the selected extension's detail page (the first shown when
    /// none is selected).
    fn open_selected(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let browse = self.exts.page.tab == Tab::Browse;
        if browse {
            let ids = self.listings_shown(cx);
            let id = self.exts.page.browse.cursor.clone().filter(|c| ids.contains(c)).or_else(|| ids.first().cloned());
            if let Some(id) = id {
                self.show_listing(Some(&id), cx);
            }
            return;
        }
        let ids: Vec<String> = self.installed_shown(cx).into_iter().map(|(id, _)| id).collect();
        let id = self.exts.page.cursor.clone().filter(|c| ids.contains(c)).or_else(|| ids.first().cloned());
        if let Some(id) = id {
            self.show_extension_detail(Some(&id), cx);
        }
    }

    /// An installed extension's folder files (read again when the
    /// extensions folder is).
    pub(super) fn files_of(&mut self, id: &str, cx: &mut Context<Self>) -> Files {
        let key = std::rc::Rc::as_ptr(&crate::extensions::found(cx)) as usize;
        if self.exts.page.files.as_ref().is_none_or(|(k, _)| *k != key) {
            self.exts.page.files = Some((key, HashMap::new()));
        }
        let folder = crate::extensions::folder_of(id, cx);
        let (_, map) = self.exts.page.files.as_mut().expect("set above");
        map.entry(id.to_string()).or_insert_with(|| Files::read(folder)).clone()
    }

    /// What extension `m` is, for the filters.
    fn facts(&mut self, m: &Manifest, cx: &mut Context<Self>) -> Facts {
        let on = crate::ui_state::get(cx).extension_on(&m.id);
        let (shipped, _) = self.shipped_now(cx);
        let (origin, _) = self.origins_now(cx).remove(&m.id).unwrap_or_default();
        let dev = Self::is_dev(&m.id, cx);
        let shipped_one = shipped.iter().find(|s| s.offer.manifest.id == m.id).filter(|_| !dev);
        let update = !dev && (catalog::update_offer(m, origin.as_ref(), shipped_one, cx).is_some() || self.index_update(m, cx).is_some());
        let (word, _, _, _) = Self::card_status_of(m, cx);
        let mut attention = match word {
            "Needs setup" | "Stopped" | "Error" | "Signed out" if on => Some(word.to_lowercase()),
            _ => None,
        };
        if attention.is_none() && on && !pitwall_core::extensions::permissions::undecided(m, &crate::ui_state::get(cx).extension(&m.id).grants).is_empty() {
            attention = Some("a new permission to answer".into());
        }
        if attention.is_none() && !crate::extensions::stale::stale_agents(&m.id, cx).is_empty() {
            attention = Some("agents on its previous setup".into());
        }
        if attention.is_none() && self.exts.failed.contains_key(&m.id) {
            attention = Some("something failed".into());
        }
        let files = self.files_of(&m.id, cx);
        Facts {
            on,
            update,
            attention,
            tiles: crate::tiles::has_tiles(m),
            tools: m.contributes.tiles.iter().any(|t| t.agents),
            service: m.service.is_some(),
            updated: files.installed_at,
        }
    }

    fn card_status_of(m: &Manifest, cx: &mut Context<Self>) -> (&'static str, Tone, String, bool) {
        let c = crate::extensions::connections::chosen(&m.id, None, cx).filter(|_| !m.settings.is_empty());
        super::card_status(m, c.as_ref(), cx)
    }

    /// The installed extensions the search and filters keep, in the order
    /// picked: (id, folder). One that can't be read is kept unless a filter
    /// asks for something only a usable one has.
    pub(super) fn installed_shown(&mut self, cx: &mut Context<Self>) -> Vec<(String, String)> {
        let found = crate::extensions::found(cx);
        let q = self.exts.page.query.clone();
        let f = self.exts.page.filters.clone();
        let mut out: Vec<(String, String, String, Option<SystemTime>)> = Vec::new();
        for item in found.iter() {
            let m = match &item.manifest {
                Ok(m) => m.clone(),
                Err(_) => {
                    let bare = Filters { attention: f.attention, ..Filters::default() };
                    if crate::palette::matching::matches(&item.folder, &q) && (f == bare) {
                        out.push((item.folder.clone(), item.folder.clone(), item.folder.to_lowercase(), None));
                    }
                    continue;
                }
            };
            let text = format!("{} {} {} {}", m.name, m.id, m.description.clone().unwrap_or_default(), m.contributes.tiles.iter().map(|t| t.title.as_str()).collect::<Vec<_>>().join(" "));
            if !crate::palette::matching::matches(&text, &q) {
                continue;
            }
            let facts = self.facts(&m, cx);
            let keep = match f.on {
                OnOff::All => true,
                OnOff::On => facts.on,
                OnOff::Off => !facts.on,
            } && (!f.updates || facts.update)
                && (!f.attention || facts.attention.is_some())
                && (!f.tiles || facts.tiles)
                && (!f.tools || facts.tools)
                && (!f.services || facts.service);
            if keep {
                out.push((m.id.clone(), item.folder.clone(), m.name.to_lowercase(), facts.updated));
            }
        }
        match self.exts.page.sort {
            Sort::Name => out.sort_by(|a, b| a.2.cmp(&b.2)),
            Sort::Updated => out.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.2.cmp(&b.2))),
        }
        out.into_iter().map(|(id, folder, _, _)| (id, folder)).collect()
    }

    /// The tabs and, under them, the tab's search, filters and order.
    pub(super) fn ext_tabs(&mut self, ui: &Ui, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let tab = self.exts.page.tab;
        let n = crate::extensions::found(cx).len();
        let me = cx.entity().downgrade();
        let tabs = crate::kit::seg(
            "extensions-tab",
            vec![SegItem::new(if n > 0 { format!("Installed  {n}") } else { "Installed".into() }, tab == Tab::Installed), SegItem::new("Browse", tab == Tab::Browse)],
            &ui.t,
            std::rc::Rc::new(move |i, _, cx| {
                let _ = me.update(cx, |v, cx| v.set_extensions_tab(if i == 0 { Tab::Installed } else { Tab::Browse }, cx));
            }),
        )
        .debug_selector(|| "extensions-tabs".into());
        let mut top = div().flex().items_center().gap(px(8.)).child(tabs).child(div().flex_1());
        if tab == Tab::Installed && !self.wide() {
            let rows = self.exts.page.rows;
            let me = cx.entity().downgrade();
            top = top.child(
                crate::kit::seg(
                    "extensions-layout",
                    vec![
                        SegItem { icon: Some("grid".into()), tooltip: Some("Cards: each with its setup".into()), ..SegItem::new("Cards", !rows) },
                        SegItem { icon: Some("list".into()), tooltip: Some("Compact rows".into()), ..SegItem::new("Rows", rows) },
                    ],
                    &ui.t,
                    std::rc::Rc::new(move |i, _, cx| {
                        let _ = me.update(cx, |v, cx| {
                            v.exts.page.rows = i == 1;
                            cx.notify();
                        });
                    }),
                )
                .debug_selector(|| "extensions-layout".into()),
            );
        }
        let input = self.search_input(tab == Tab::Browse, window, cx);
        let search_row = div()
            .key_context(SEARCH_CONTEXT)
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(|v, _: &FindExtension, w, cx| v.find_extension(w, cx)))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(search_box(input, ui));
        let mut col = div().flex().flex_col().gap(px(10.)).child(top);
        match tab {
            Tab::Installed => {
                let sort = self.sort_select(ui, cx);
                col = col.child(search_row.child(sort)).child(self.installed_filters(ui, cx));
            }
            Tab::Browse => {
                col = col.child(search_row).child(self.browse_filters(ui, cx));
            }
        }
        col
    }

    /// Name or Recently updated.
    fn sort_select(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let sort = self.exts.page.sort;
        let open = self.exts.page.sort_open;
        let me = cx.entity().downgrade();
        let (m2, m3) = (me.clone(), me.clone());
        crate::kit::select(
            "extensions-sort",
            match sort {
                Sort::Name => "Name",
                Sort::Updated => "Recently updated",
            },
            30.,
            12.5,
            false,
            open,
            vec![("Name".into(), sort == Sort::Name), ("Recently updated".into(), sort == Sort::Updated)],
            &ui.t,
            move |_, cx| {
                let _ = me.update(cx, |v, cx| {
                    v.exts.page.sort_open = !v.exts.page.sort_open;
                    cx.notify();
                });
            },
            std::rc::Rc::new(move |i, _, cx| {
                let _ = m2.update(cx, |v, cx| {
                    v.exts.page.sort = if i == 0 { Sort::Name } else { Sort::Updated };
                    v.exts.page.sort_open = false;
                    cx.notify();
                });
            }),
            std::rc::Rc::new(move |_, cx| {
                let _ = m3.update(cx, |v, cx| {
                    v.exts.page.sort_open = false;
                    cx.notify();
                });
            }),
        )
    }

    /// All / On / Off, then the filters that are on or off.
    fn installed_filters(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let f = self.exts.page.filters.clone();
        let me = cx.entity().downgrade();
        let onoff = crate::kit::seg(
            "extensions-onoff",
            vec![SegItem::new("All", f.on == OnOff::All), SegItem::new("On", f.on == OnOff::On), SegItem::new("Off", f.on == OnOff::Off)],
            &ui.t,
            std::rc::Rc::new(move |i, _, cx| {
                let _ = me.update(cx, |v, cx| {
                    v.exts.page.filters.on = [OnOff::All, OnOff::On, OnOff::Off][i.min(2)];
                    cx.notify();
                });
            }),
        );
        let mut row = div().flex().flex_wrap().items_center().gap(px(6.)).child(onoff).child(div().w(px(4.)));
        type Pick = fn(&mut Filters) -> &mut bool;
        let chips: [(&str, &str, bool, Pick); 5] = [
            ("updates", "Updates", f.updates, |f| &mut f.updates),
            ("attention", "Needs attention", f.attention, |f| &mut f.attention),
            ("tiles", "Tiles", f.tiles, |f| &mut f.tiles),
            ("tools", "Agent tools", f.tools, |f| &mut f.tools),
            ("services", "Services", f.services, |f| &mut f.services),
        ];
        for (key, label, on, pick) in chips {
            row = row.child(filter_chip(format!("extensions-filter-{key}"), label, on, ui).on_click(cx.listener(move |v, _, _, cx| {
                let b = pick(&mut v.exts.page.filters);
                *b = !*b;
                v.exts.page.cursor = None;
                cx.notify();
            })));
        }
        row
    }

    /// Nothing installed, or nothing the search and filters keep.
    pub(super) fn installed_empty(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Option<Div> {
        let none = crate::extensions::found(cx).is_empty();
        let shown = self.installed_shown(cx).len();
        if !none && shown > 0 {
            return None;
        }
        let boxed = |d: Div| div().debug_selector(|| "extensions-empty".into()).flex().flex_col().gap(px(8.)).p(px(14.)).rounded(px(10.)).border_1().border_dashed().border_color(ui.t.line).child(d);
        if none {
            let shipped = !self.shipped_now(cx).0.is_empty();
            let text = if shipped { "Nothing installed yet. Pick one under Available below, or in Browse: installing turns nothing on." } else { "Nothing installed yet. Find one in Browse, or install one from a file: installing turns nothing on." };
            return Some(boxed(crate::kit::muted(text, &ui.t)));
        }
        let q = self.exts.page.query.trim().to_string();
        let text = if q.is_empty() { "No extension fits these filters.".to_string() } else { format!("No installed extension matches “{q}”.") };
        let clear = btn("extensions-clear".into(), "Clear search and filters", BtnKind::GhostSm, false, ui).on_click(cx.listener(|v, _, _, cx| {
            v.exts.page.filters = Filters::default();
            v.exts.page.query.clear();
            if let Some(i) = v.exts.page.search.clone() {
                i.update(cx, |i, cx| i.set_text("", cx));
            }
            cx.notify();
        }));
        Some(boxed(div().flex().items_center().gap(px(10.)).child(div().flex_1().child(crate::kit::muted(text, &ui.t))).child(clear)))
    }

    /// The parts of the Installed tab: each extension kept, as its card or
    /// a row, the selected one marked.
    pub(super) fn installed_parts(&mut self, cx: &mut Context<Self>) -> Vec<Part> {
        let shown = self.installed_shown(cx);
        let rows = self.exts.page.rows;
        let mut v = Vec::new();
        v.push(Part::row("ext-installed-empty", |s, ui, _, cx| s.installed_empty(ui, cx).map(|d| d.into_any_element()).unwrap_or_else(|| div().into_any_element())));
        for (i, (_, folder)) in shown.into_iter().enumerate() {
            let f2 = folder.clone();
            let key = ("ext", folder.clone());
            if rows {
                v.push(Part::row(key, move |s, ui, _, cx| s.ext_row(&f2, ui, cx).into_any_element()));
            } else if i == 0 {
                v.push(Part::row(key, move |s, ui, _, cx| s.ext_card(&f2, ui, cx).into_any_element()));
            } else {
                v.push(Part::section(key, move |s, ui, _, cx| s.ext_card(&f2, ui, cx).into_any_element()));
            }
        }
        v
    }

    /// Whether `id` is the selected one (its card or row marked).
    pub(super) fn is_selected(&self, id: &str) -> bool {
        self.exts.page.cursor.as_deref() == Some(id)
    }

    /// A compact row: icon, name, version and line, its marks (update,
    /// needs attention, preview), its state and switch; a click opens its
    /// detail page.
    fn ext_row(&mut self, folder: &str, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let found = crate::extensions::found(cx);
        let Some(f) = found.iter().find(|f| f.folder == folder) else { return div() };
        let Ok(m) = f.manifest.clone() else {
            return div().child(Self::card(ui, format!("extension-bad-{}", f.folder)).child(crate::kit::hint(format!("{}: can't be used. Open Cards to see why.", f.folder), &ui.t)));
        };
        let facts = self.facts(&m, cx);
        let (word, tone, _, _) = Self::card_status_of(&m, cx);
        let selected = self.is_selected(&m.id);
        let icon = div().flex_none().size(px(26.)).rounded(px(7.)).bg(ui.t.surface_2).flex().items_center().justify_center().child(crate::kit::icon(crate::extensions::icon_of(&m), 14., ui.t.text_2));
        let mut name = div().flex().items_center().gap(px(6.)).min_w_0().child(div().flex_none().font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(m.name.clone()));
        if let Some(v) = &m.version {
            name = name.child(div().flex_none().text_size(px(12.)).text_color(ui.t.text_3).child(v.clone()));
        }
        let mut marks = div().flex().flex_none().items_center().gap(px(4.));
        if facts.update {
            marks = marks.child(crate::kit::chip_tone("update", Tone::Ok, &ui.t));
        }
        if let Some(why) = &facts.attention {
            let w = why.clone();
            marks = marks.child(div().id(SharedString::from(format!("extension-row-attention-{}", m.id))).tooltip(crate::kit::tooltip(format!("Needs attention: {w}"))).child(crate::kit::chip_tone("needs attention", Tone::Warn, &ui.t)));
        }
        if self.is_preview(&m.id, cx) {
            marks = marks.child(crate::kit::chip_tone("preview", Tone::Subtle, &ui.t));
        }
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(name)
            .child(div().text_size(px(12.)).text_color(ui.t.text_3).overflow_hidden().whitespace_nowrap().text_ellipsis().child(m.description.clone().unwrap_or_default()));
        let on = facts.on;
        let m2 = m.clone();
        let sel = format!("extension-switch-{}", m.id);
        let switch = crate::kit::toggle(SharedString::from(sel.clone()), on, &ui.t).debug_selector(move || sel).tooltip(crate::kit::tooltip(if on { "Turn off" } else { "Turn on" })).on_click(cx.listener(move |v, _, _, cx| {
            cx.stop_propagation();
            if on {
                crate::ui_state::update(cx, |s| *s = s.clone().set_extension(&m2.id, false));
            } else {
                v.ask_to_turn_on(&m2, cx);
                // Its permissions are answered on its page.
                if v.exts.asking.as_ref().is_some_and(|(e, _)| *e == m2.id) {
                    v.show_extension_detail(Some(&m2.id), cx);
                }
            }
            cx.notify();
        }));
        let id = m.id.clone();
        let row_id = format!("extension-row-{}", m.id);
        let sel = row_id.clone();
        let row = div()
            .id(SharedString::from(row_id))
            .debug_selector(move || sel)
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(9.))
            .border_1()
            .border_color(if selected { ui.t.line_strong } else { ui.t.line })
            .when(selected, |d| d.bg(ui.t.surface_2))
            .cursor_pointer()
            .hover(|d| d.bg(ui.t.surface_2))
            .child(icon)
            .child(text)
            .child(marks)
            .when(!self.wide(), |d| d.child(div().w(px(96.)).flex().justify_end().child(crate::kit::chip_tone(SharedString::from(word), tone, &ui.t))))
            .child(switch)
            .when(!self.wide(), |d| d.child(crate::kit::icon("chevron", 11., ui.t.text_3)))
            .on_click(cx.listener(move |v, _, _, cx| v.show_extension_detail(Some(&id), cx)));
        div().child(row)
    }

    /// The bar over a detail page: back to the list.
    pub(super) fn detail_bar(&mut self, ui: &Ui, what: &str, browse: bool, cx: &mut Context<Self>) -> Div {
        let back = div()
            .id("extension-detail-back")
            .debug_selector(|| "extension-detail-back".into())
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .px(px(8.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(ui.t.text_2)
            .hover(|d| d.bg(ui.t.surface_2).text_color(ui.t.text))
            .child(crate::kit::icon("back", 12., ui.t.text_2))
            .child(if browse { "Browse" } else { "Installed" })
            .on_click(cx.listener(move |v, _, _, cx| {
                if browse {
                    v.show_listing(None, cx);
                } else {
                    v.show_extension_detail(None, cx);
                }
            }));
        div().flex().items_center().gap(px(6.)).pt(px(2.)).child(back).child(div().text_size(px(12.5)).text_color(ui.t.text_4).child("/")).child(div().text_size(px(12.5)).text_color(ui.t.text_2).child(what.to_string()))
    }

    /// Under the name on a detail page: who wrote and signed it, and
    /// Verified when the index says so of this very file.
    pub(super) fn trust_line(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let (origin, _) = self.origins_now(cx).remove(&m.id).unwrap_or_default();
        let listing = self.listing_of(&m.id);
        let mut line = div().flex().flex_wrap().items_center().gap(px(8.)).text_size(px(12.)).text_color(ui.t.text_3);
        if let Some(l) = &listing {
            line = line.child(format!("by {}", l.author));
        }
        let (icon, color, text): (&str, gpui::Hsla, String) = match &origin {
            Some(Origin::Package { signer: Some(s), .. }) => ("lock", ui.t.green, format!("Signed · key {}", s.key_id)),
            Some(Origin::Package { signer: None, .. }) | Some(Origin::Folder { .. }) => ("alert", ui.t.amber, "Unsigned preview".into()),
            Some(Origin::Bundled) => ("lock", ui.t.green, "Comes with Pitwall".into()),
            Some(o @ (Origin::Git { .. } | Origin::Npm { .. })) => ("branch", ui.t.text_3, format!("From {}", o.place())),
            None if Self::is_dev(&m.id, cx) => ("folder", ui.t.text_3, "Development folder".into()),
            None => ("folder", ui.t.text_3, "Put here by hand".into()),
        };
        let sel = format!("extension-trust-{}", m.id);
        line = line.child(div().debug_selector(move || sel).flex().items_center().gap(px(4.)).child(crate::kit::icon(icon, 11., color)).child(text));
        let sha = match &origin {
            Some(Origin::Package { sha256, .. }) => Some(sha256.clone()),
            _ => None,
        };
        if matches!(&origin, Some(Origin::Package { index: Some(_), .. })) {
            line = line.child(div().debug_selector({
                let s = format!("extension-from-index-{}", m.id);
                move || s
            }).child("from the extensions index"));
        }
        if let (Some(l), Some(sha), Some(read)) = (&listing, sha, self.index_now()) {
            if let Some(v) = l.versions.iter().find(|v| v.sha256.eq_ignore_ascii_case(&sha)) {
                line = line.child(match l.standing(v, &read.judge()) {
                    crate::extensions::index::Standing::Verified => super::browse::verified_badge(ui, &m.id).into_any_element(),
                    s => super::browse::standing_chip(ui, &s),
                });
            }
        }
        // Blocked (and so turned off), withdrawn or no longer listed.
        line.children(Self::index_note(ui, &m.id, cx))
    }

    /// What the index says of installed copy `id` when it matters: turned
    /// off as blocked, withdrawn, no longer listed (on its card and page).
    pub(super) fn index_note(ui: &Ui, id: &str, cx: &App) -> Option<Div> {
        let n = crate::extensions::index::note(id, cx)?;
        let sel = format!("extension-index-note-{id}");
        Some(div().debug_selector(move || sel).w_full().flex().items_center().gap(px(4.)).text_size(px(12.)).text_color(ui.t.amber).child(crate::kit::icon("alert", 11., ui.t.amber)).child(n))
    }

    /// A README or changelog drawn inline by Pitwall's Markdown Preview:
    /// no HTML, no remote images; images beside it in `base` only.
    pub(super) fn markdown(&mut self, key: &str, text: Arc<str>, base: Option<PathBuf>, cx: &mut Context<Self>) -> Entity<MarkdownView> {
        let md = self.exts.page.md.entry(key.to_string()).or_insert_with(|| cx.new(|cx| MarkdownView::new(cx).inline())).clone();
        let k = key.to_string();
        md.update(cx, |v, cx| v.set(&k, text, None, base, cx));
        md
    }

    /// The detail page of installed extension `id`: the bar, its card at
    /// full size, then what its folder says about it.
    pub(super) fn installed_detail_parts(&mut self, id: &str, cx: &mut Context<Self>) -> Vec<Part> {
        let found = crate::extensions::found(cx);
        let Some(f) = found.iter().find(|f| f.key() == id || f.folder == id) else { return vec![] };
        let folder = f.folder.clone();
        let name = f.manifest.as_ref().map(|m| m.name.clone()).unwrap_or_else(|_| f.folder.clone());
        let mut v = vec![Part::section("ext-detail-bar", move |s, ui, _, cx| s.detail_bar(ui, &name, false, cx).into_any_element())];
        let f2 = folder.clone();
        v.push(Part::row(("ext", folder.clone()), move |s, ui, _, cx| s.ext_card_in(&f2, ui, true, cx).into_any_element()));
        let Ok(m) = f.manifest.clone() else { return v };
        let on = crate::ui_state::get(cx).extension_on(&m.id);
        let files = self.files_of(&m.id, cx);
        if !files.screenshots.is_empty() {
            let shots = files.screenshots.clone();
            v.push(Part::section(("ext-detail-shots", m.id.clone()), move |_, ui, _, _| screenshots_row(ui, shots.iter().map(|p| img(p.clone())).collect()).into_any_element()));
        }
        {
            let (m, files) = (m.clone(), files.clone());
            v.push(Part::section(("ext-detail-readme", m.id.clone()), move |s, ui, _, cx| {
                let mut col = div().flex().flex_col().gap(px(8.)).child(title(ui, "About"));
                col = match files.readme.clone() {
                    Some(t) => {
                        let md = s.markdown(&format!("{}/README.md", m.id), t, files.folder.clone(), cx);
                        col.child(div().debug_selector(|| "extension-readme".into()).child(md))
                    }
                    None => col.child(crate::kit::hint(m.description.clone().unwrap_or_else(|| "It has no README.".into()), &ui.t)),
                };
                col.into_any_element()
            }));
        }
        // While it is off its card is folded to its switch: what it asks
        // for and adds are said here (on, the card has them).
        if !on {
            let m2 = m.clone();
            v.push(Part::section(("ext-detail-off", m.id.clone()), move |s, ui, _, cx| s.off_facts(ui, &m2, cx).into_any_element()));
        }
        let m2 = m.clone();
        v.push(Part::section(("ext-detail-versions", m.id.clone()), move |s, ui, _, cx| s.versions_view(ui, &m2, cx).into_any_element()));
        let m2 = m.clone();
        v.push(Part::section(("ext-detail-about", m.id.clone()), move |s, ui, _, cx| s.licence_view(ui, &m2, cx).into_any_element()));
        v
    }

    /// Off: the permissions it asks for, each with its reason and risk
    /// note (answered when it is turned on), what it adds, and its agent
    /// tools.
    fn off_facts(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let mut col = div().flex().flex_col().gap(px(12.));
        let mut perms = div().debug_selector(|| "extension-detail-permissions".into()).flex().flex_col().gap(px(10.)).child(title(ui, "Permissions"));
        if m.permissions.is_empty() {
            perms = perms.child(crate::kit::hint("It asks for no permissions.", &ui.t));
        } else {
            for p in &m.permissions {
                perms = perms.child(Self::permission_words_of(ui, m, &p.name, &p.reason, cx));
            }
            perms = perms.child(crate::kit::hint("You allow or refuse each when you turn it on; its switches are on its card then.", &ui.t));
        }
        col = col.child(perms);
        if let Some(adds) = Self::contributions(ui, m) {
            col = col.child(adds);
        }
        let tiles: Vec<_> = m.contributes.tiles.iter().filter(|t| t.agents).collect();
        if !tiles.is_empty() {
            let mut tools = div().flex().flex_col().gap(px(4.)).child(crate::kit::label("Agent tools", ui.t.text_3, 11.));
            for t in tiles {
                let mut names = vec![pitwall_proto::tile_tools::mcp_name(&m.id, None)];
                names.extend(t.tools.iter().map(|d| pitwall_proto::tile_tools::mcp_name(&m.id, Some(&d.name))));
                tools = tools.child(div().text_size(px(12.)).text_color(ui.t.text_2).child(format!("Agents can open {} and use its tools once it is on:", t.title))).child(crate::kit::mono_text(names.join("  "), &ui.t).text_color(ui.t.text_3));
            }
            col = col.child(tools);
        }
        col
    }

    /// Its versions: the installed one and when, the kept one Go back
    /// returns to, the index's with their dates and what changed, and its
    /// CHANGELOG.md.
    fn versions_view(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let files = self.files_of(&m.id, cx);
        let (_, back) = self.origins_now(cx).remove(&m.id).unwrap_or_default();
        let mut col = div().debug_selector(|| "extension-detail-versions".into()).flex().flex_col().gap(px(8.)).child(title(ui, "Versions"));
        let installed = m.version.clone().unwrap_or_else(|| "no version".into());
        let when_in = files.installed_at.map(|t| format!(", installed {}", when(t))).unwrap_or_default();
        col = col.child(div().flex().items_center().gap(px(8.)).child(crate::kit::chip_tone(SharedString::from(installed), Tone::Ok, &ui.t)).child(div().text_size(px(12.)).text_color(ui.t.text_2).child(format!("Installed{when_in}"))));
        if let Some(b) = back {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.text_3).child(format!("{b} is kept: Go back on its card returns to it.")));
        }
        if let Some(l) = self.listing_of(&m.id) {
            if let Some(read) = self.index_now() {
                col = col.child(super::browse::versions_list(ui, &l, &read.judge(), m.version.as_deref()));
            }
        }
        if let Some(t) = files.changelog.clone() {
            let md = self.markdown(&format!("{}/CHANGELOG.md", m.id), t, files.folder.clone(), cx);
            col = col.child(crate::kit::label("Changelog", ui.t.text_3, 11.)).child(div().max_h(px(360.)).overflow_hidden().child(md));
        }
        col
    }

    /// Its licence (the first line of its LICENSE, with Open) and where its
    /// source is.
    fn licence_view(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let files = self.files_of(&m.id, cx);
        let listing = self.listing_of(&m.id);
        let (origin, _) = self.origins_now(cx).remove(&m.id).unwrap_or_default();
        let mut col = div().debug_selector(|| "extension-detail-licence".into()).flex().flex_col().gap(px(8.)).child(title(ui, "Licence and source"));
        let row = |label: &str, value: AnyElement| div().flex().items_center().gap(px(10.)).child(div().w(px(90.)).flex_none().text_size(px(12.)).text_color(ui.t.text_3).child(label.to_string())).child(value);
        let licence = match (&files.licence, listing.as_ref().map(|l| l.license.clone()).filter(|l| !l.trim().is_empty())) {
            (Some((path, first)), _) => {
                let p = path.clone();
                div().flex().items_center().gap(px(8.)).child(div().text_size(px(12.5)).text_color(ui.t.text).child(if first.is_empty() { "LICENSE".to_string() } else { first.clone() })).child(
                    btn(format!("extension-licence-open-{}", m.id), "Open", BtnKind::Link, false, ui).on_click(move |_, _, cx| crate::platform::files::open_in_default_app(cx, &p)),
                )
            }
            (None, Some(l)) => div().text_size(px(12.5)).text_color(ui.t.text).child(l),
            (None, None) => crate::kit::hint("None given", &ui.t),
        };
        col = col.child(row("Licence", licence.into_any_element()));
        let repo = listing.as_ref().map(|l| l.repository.clone()).or(match &origin {
            Some(Origin::Git { url, .. }) => Some(url.clone()),
            _ => None,
        });
        if let Some(url) = repo {
            let u = url.clone();
            col = col.child(row("Repository", btn(format!("extension-repo-{}", m.id), url.trim_start_matches("https://").to_string(), BtnKind::Link, false, ui).tooltip(crate::kit::tooltip("Open in the browser")).on_click(move |_, _, cx| crate::platform::files::open_url(cx, &u)).into_any_element()));
        }
        if let Some(d) = m.docs.clone() {
            let u = d.clone();
            col = col.child(row("Docs", btn(format!("extension-docs-link-{}", m.id), d.trim_start_matches("https://").to_string(), BtnKind::Link, false, ui).on_click(move |_, _, cx| crate::platform::files::open_url(cx, &u)).into_any_element()));
        }
        if let Some(dir) = files.folder.clone() {
            col = col.child(row("Folder", crate::kit::mono_text(pitwall_core::paths::tildify(&dir.to_string_lossy()), &ui.t).text_color(ui.t.text_2).into_any_element()));
        }
        col
    }
}

/// Screenshots side by side, scrolling sideways when they don't fit.
pub(super) fn screenshots_row(ui: &Ui, images: Vec<gpui::Img>) -> Div {
    let mut row = div().flex().gap(px(10.));
    for i in images {
        row = row.child(div().flex_none().w(px(300.)).h(px(188.)).rounded(px(8.)).overflow_hidden().border_1().border_color(ui.t.line).bg(ui.t.surface_2).child(i.size_full().object_fit(ObjectFit::Cover)));
    }
    div().debug_selector(|| "extension-screenshots".into()).flex().flex_col().gap(px(8.)).child(title(ui, "Screenshots")).child(div().id("extension-screenshots-scroll").overflow_x_scroll().child(row))
}

/// Registered on the page list: ⌘F there too.
pub(in crate::settings) fn on_nav<E: InteractiveElement>(el: E, cx: &mut Context<SettingsView>) -> E {
    el.on_action(cx.listener(|v, _: &FindExtension, w, cx| v.find_extension(w, cx)))
}
