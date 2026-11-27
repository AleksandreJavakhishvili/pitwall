//! Settings → Extensions → **Browse** (docs/spec/tile-sdk/marketplace.md
//! §7.4, §7.5): the extensions index ([`crate::extensions::index`], over
//! `pitwall_core::extensions::index`), searched on this computer (name,
//! description, author, categories; as the palette matches), by category,
//! "Verified only" (on at first) and "Works with this Pitwall". A card per
//! extension: its icon, name, author, line, Verified, what it may do in
//! words, and Install, Installed or Update. Its detail page: screenshots,
//! what it does, permissions with their reasons and risk notes, versions
//! with how each stands (Verified, not yet reviewed, withdrawn, blocked),
//! dates and changes, licence, repository, Install and "Report a problem"
//! (a prefilled issue on the index's repository, opened in the browser;
//! Pitwall sends nothing).
//!
//! Install downloads the version the index offers (the newest Verified,
//! else the newest: `Listing::default_release`) with `index::download`,
//! which refuses a blocked or withdrawn one and checks the file's SHA-256
//! and its author's signature; then Install from file…'s trust screen for
//! it. Nothing is installed until the user presses Install there, and the
//! copy installed is recorded as from the index, so its updates come from
//! there.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{div, img, prelude::*, px, Context, Div, SharedString};
use pitwall_core::extensions::install::{compare_versions, Origin};
use pitwall_core::extensions::package::Key;
use pitwall_core::extensions::{permissions, Manifest};

use super::super::view::{Part, SettingsView};
use super::super::widgets::Ui;
use super::page::{btn, filter_chip, screenshots_row, title};
use crate::extensions::catalog;
use crate::extensions::index::{self, Judge, Listing, Query, Read, Release, ReportKind, Standing, CATEGORIES};
use crate::kit::{BtnKind, SegItem, TextInput, Tone};

/// This Pitwall's version, as the index's `pitwall` ranges compare it.
pub fn ours() -> &'static str {
    pitwall_core::extensions::PITWALL
}

/// How reading the index stands.
#[derive(Clone, Default)]
pub enum Load {
    #[default]
    Idle,
    /// No index is set up in this Pitwall.
    NoSource,
    Loading,
    Ready(Read),
    /// Why there is none (offline, or it didn't check).
    Failed(String),
}

/// Whether `why` says this computer is offline.
fn is_offline(why: &str) -> bool {
    why.contains("offline") || why.contains("Couldn't reach")
}

/// The Browse tab's state.
pub struct Browse {
    pub load: Load,
    pub search: Option<gpui::Entity<TextInput>>,
    pub query: Query,
    /// The listing whose detail page is open.
    pub detail: Option<String>,
    pub cursor: Option<String>,
    /// Screenshots, by path.
    screens: HashMap<String, Result<Arc<gpui::Image>, String>>,
    fetching: HashSet<String>,
    /// Being downloaded.
    pub downloading: Option<String>,
    /// Why a download failed, by listing.
    pub notes: HashMap<String, String>,
    /// The kind of problem picked for "Report a problem", by listing.
    report: HashMap<String, usize>,
    /// The package downloaded for each listing (its trust screen is the
    /// one Install from file… shows for that file).
    pub downloaded: HashMap<String, std::path::PathBuf>,
}

impl Default for Browse {
    fn default() -> Self {
        Browse {
            load: Load::Idle,
            search: None,
            query: Query { verified_only: true, ..Query::default() },
            detail: None,
            cursor: None,
            screens: HashMap::new(),
            fetching: HashSet::new(),
            downloading: None,
            notes: HashMap::new(),
            report: HashMap::new(),
            downloaded: HashMap::new(),
        }
    }
}

/// A listing's part on the Browse tab.
pub(super) fn part_key(id: &str) -> (&'static str, String) {
    ("listing", id.to_string())
}

/// The Verified badge.
pub(super) fn verified_badge(ui: &Ui, id: &str) -> gpui::Stateful<Div> {
    let sel = format!("extension-verified-{id}");
    div()
        .id(SharedString::from(sel.clone()))
        .debug_selector(move || sel)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(3.))
        .px(px(6.))
        .h(px(18.))
        .rounded(px(9.))
        .bg(ui.t.green.opacity(0.14))
        .text_size(px(11.))
        .font_weight(crate::kit::WEIGHT_550)
        .text_color(ui.t.green)
        .tooltip(crate::kit::tooltip("A Pitwall maintainer read this version's source and signed that it does what it says"))
        .child(crate::kit::icon("check", 10., ui.t.green))
        .child("Verified")
}

/// How version `r` stands, as a chip.
pub(super) fn standing_chip(ui: &Ui, s: &Standing) -> gpui::AnyElement {
    match s {
        Standing::Verified => crate::kit::chip_tone("Verified", Tone::Ok, &ui.t).into_any_element(),
        Standing::NotYetReviewed => crate::kit::chip_tone("not yet reviewed", Tone::Subtle, &ui.t).into_any_element(),
        Standing::NotVerified => crate::kit::chip_tone("not verified", Tone::Subtle, &ui.t).into_any_element(),
        Standing::Yanked => crate::kit::chip_tone("withdrawn", Tone::Warn, &ui.t).into_any_element(),
        Standing::Blocked(_) => crate::kit::chip_tone("blocked", Tone::Warn, &ui.t).into_any_element(),
    }
}

/// A listing's versions, newest first: date, how it stands, why one
/// doesn't work here, the one installed, what changed.
pub(super) fn versions_list(ui: &Ui, l: &Listing, j: &Judge, installed: Option<&str>) -> Div {
    let mut col = div().flex().flex_col().gap(px(6.));
    for v in index::newest_first(l) {
        let s = l.standing(v, j);
        let mut line = div().flex().flex_wrap().items_center().gap(px(8.)).child(crate::kit::mono_text(v.version.clone(), &ui.t).text_color(ui.t.text));
        if !v.date.is_empty() {
            line = line.child(div().text_size(px(12.)).text_color(ui.t.text_3).child(v.date.clone()));
        }
        line = line.child(standing_chip(ui, &s));
        if installed == Some(v.version.as_str()) {
            line = line.child(crate::kit::chip_tone("installed", Tone::Plain, &ui.t));
        }
        if let Standing::Blocked(why) = &s {
            line = line.child(div().text_size(px(12.)).text_color(ui.t.red).child(why.clone()));
        } else if let Some(why) = index::not_here(v, j) {
            line = line.child(div().text_size(px(12.)).text_color(ui.t.amber).child(why));
        }
        if v.size > 0 {
            line = line.child(div().text_size(px(12.)).text_color(ui.t.text_4).child(size_words(v.size)));
        }
        let mut item = div().flex().flex_col().gap(px(2.)).child(line);
        if let Some(c) = v.changelog.as_ref().filter(|c| !c.trim().is_empty()) {
            item = item.child(div().pl(px(2.)).text_size(px(12.)).line_height(gpui::relative(1.45)).text_color(ui.t.text_2).child(c.trim().to_string()));
        }
        col = col.child(item);
    }
    col
}

fn size_words(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", (bytes / 1024).max(1))
    }
}

/// The image format of `bytes`, by their first bytes.
fn format_of(bytes: &[u8]) -> Option<gpui::ImageFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(gpui::ImageFormat::Png)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(gpui::ImageFormat::Webp)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Some(gpui::ImageFormat::Jpeg)
    } else {
        None
    }
}

/// What the card's button does for a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    Install(String),
    Update(String),
    Installed,
    /// Nothing to install here, and why.
    None(String),
}

impl SettingsView {
    /// Read the index (again when `force`), off the UI thread. What it
    /// says of installed copies is applied too (blocked ones turned off).
    pub(super) fn load_index(&mut self, force: bool, cx: &mut Context<Self>) {
        let b = &self.exts.page.browse;
        // A failure stays until Try again (no fetch on every frame).
        if !force && matches!(b.load, Load::Loading | Load::Ready(..) | Load::NoSource | Load::Failed(_)) {
            return;
        }
        // Read already (the daily check): shown at once.
        if !force {
            if let Some(r) = index::last(cx) {
                self.exts.page.browse.load = Load::Ready(r);
                cx.notify();
                return;
            }
        }
        let Some(setup) = index::setup(cx) else {
            self.exts.page.browse.load = Load::NoSource;
            cx.notify();
            return;
        };
        self.exts.page.browse.load = Load::Loading;
        cx.notify();
        let read = index::fetch(setup, cx);
        cx.spawn(async move |this, cx| {
            let r = read.await;
            let _ = this.update(cx, |v, cx| {
                v.exts.page.browse.load = match r {
                    Ok(read) => {
                        index::apply(read.clone(), cx);
                        Load::Ready(read)
                    }
                    Err(e) => Load::Failed(e),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// The index, once read.
    pub(super) fn index_now(&self) -> Option<Read> {
        match &self.exts.page.browse.load {
            Load::Ready(r) => Some(r.clone()),
            _ => None,
        }
    }

    /// Extension `id`'s entry in the index, once read.
    pub(super) fn listing_of(&self, id: &str) -> Option<Listing> {
        self.index_now()?.index().listing(id).cloned()
    }

    /// A newer version of installed `m` the index offers, when that copy
    /// came from the index (its updates come from there).
    pub(super) fn index_update(&self, m: &Manifest, cx: &mut Context<Self>) -> Option<Release> {
        let dir = crate::extensions::dir(cx)?;
        let folder = crate::extensions::found(cx).iter().find(|f| f.manifest.as_ref().is_ok_and(|x| x.id == m.id))?.folder.clone();
        let from_index = matches!(pitwall_core::extensions::install::origin(&dir.join(folder)), Some(Origin::Package { index: Some(_), .. }));
        if !from_index {
            return None;
        }
        let read = self.index_now().or_else(|| index::last(cx))?;
        let l = read.index().listing(&m.id)?;
        let r = l.default_release(&read.judge())?.clone();
        let have = m.version.as_deref()?;
        (compare_versions(&r.version, have) == std::cmp::Ordering::Greater).then_some(r)
    }

    /// What Browse offers for `l`.
    pub fn offer_of(&self, l: &Listing, cx: &gpui::App) -> Offer {
        let Some(read) = self.index_now() else { return Offer::None("The index isn't read yet".into()) };
        let j = read.judge();
        let installed = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == l.id).cloned();
        let offered = l.default_release(&j);
        match (installed, offered) {
            (Some(m), Some(v)) => match m.version.as_deref() {
                Some(have) if compare_versions(&v.version, have) == std::cmp::Ordering::Greater => Offer::Update(v.version.clone()),
                _ => Offer::Installed,
            },
            (Some(_), None) => Offer::Installed,
            (None, Some(v)) => Offer::Install(v.version.clone()),
            (None, None) => Offer::None(match index::newest_first(l).first() {
                Some(r) => match l.standing(r, &j) {
                    s @ (Standing::Blocked(_) | Standing::Yanked) => s.words(),
                    _ => index::not_here(r, &j).unwrap_or_else(|| "No version to install".into()),
                },
                None => "No version listed".into(),
            }),
        }
    }

    /// The listings the search and filters keep, by id.
    pub(super) fn listings_shown(&mut self, _: &mut Context<Self>) -> Vec<String> {
        let Some(r) = self.index_now() else { return vec![] };
        index::search(r.index(), &self.exts.page.browse.query, &r.judge()).into_iter().map(|l| l.id.clone()).collect()
    }

    /// Listing `id`'s detail page (`None`: the list).
    pub fn show_listing(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        self.exts.page.tab = super::page::Tab::Browse;
        self.exts.page.browse.detail = id.map(str::to_string);
        if let Some(id) = id {
            self.exts.page.browse.cursor = Some(id.to_string());
            self.fetch_listing_files(id, cx);
        }
        self.scroll_to_top();
        cx.notify();
    }

    /// Its screenshots, fetched once.
    fn fetch_listing_files(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(l) = self.listing_of(id) else { return };
        let Some(read) = self.index_now() else { return };
        for path in l.screens.iter().take(pitwall_core::extensions::package::MAX_SCREENSHOTS).cloned() {
            let b = &mut self.exts.page.browse;
            if b.screens.contains_key(&path) || !b.fetching.insert(path.clone()) {
                continue;
            }
            let (setup, p) = (read.setup.clone(), path.clone());
            let get = cx.background_executor().spawn(async move { setup.get(&p, pitwall_core::extensions::package::MAX_SCREENSHOT_BYTES) });
            cx.spawn(async move |this, cx| {
                let r = get.await;
                let _ = this.update(cx, |v, cx| {
                    let b = &mut v.exts.page.browse;
                    b.fetching.remove(&path);
                    let img = r.and_then(|bytes| format_of(&bytes).map(|f| Arc::new(gpui::Image::from_bytes(f, bytes))).ok_or_else(|| "Not a png or webp image.".to_string()));
                    b.screens.insert(path, img);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The Browse tab's category and filters.
    pub(super) fn browse_filters(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let q = self.exts.page.browse.query.clone();
        let mut row = div().flex().flex_wrap().items_center().gap(px(6.));
        row = row.child(filter_chip("extensions-browse-verified".into(), "Verified only", q.verified_only, ui).tooltip(crate::kit::tooltip("Only those with a version a Pitwall maintainer has read the source of")).on_click(cx.listener(|v, _, _, cx| {
            v.exts.page.browse.query.verified_only = !v.exts.page.browse.query.verified_only;
            cx.notify();
        })));
        row = row.child(filter_chip("extensions-browse-works".into(), "Works with this Pitwall", q.works_here, ui).tooltip(crate::kit::tooltip(format!("Only those this Pitwall ({}) can run", ours()))).on_click(cx.listener(|v, _, _, cx| {
            v.exts.page.browse.query.works_here = !v.exts.page.browse.query.works_here;
            cx.notify();
        })));
        let mut cats = div().flex().flex_wrap().items_center().gap(px(4.));
        let all = q.category.is_none();
        cats = cats.child(category_chip("all", "All", all, ui).on_click(cx.listener(|v, _, _, cx| {
            v.exts.page.browse.query.category = None;
            cx.notify();
        })));
        for (id, name) in CATEGORIES {
            let on = q.category.as_deref() == Some(id);
            cats = cats.child(category_chip(id, name, on, ui).on_click(cx.listener(move |v, _, _, cx| {
                v.exts.page.browse.query.category = if on { None } else { Some(id.to_string()) };
                v.exts.page.browse.cursor = None;
                cx.notify();
            })));
        }
        div().flex().flex_col().gap(px(8.)).child(cats).child(row)
    }

    /// The Browse tab's parts: how the index stands, then a card per
    /// listing kept.
    pub(super) fn browse_parts(&mut self, cx: &mut Context<Self>) -> Vec<Part> {
        self.load_index(false, cx);
        let mut v = vec![Part::row("browse-state", |s, ui, _, cx| s.browse_state(ui, cx).into_any_element())];
        for id in self.listings_shown(cx) {
            let i2 = id.clone();
            v.push(Part::row(part_key(&id), move |s, ui, _, cx| s.listing_card(&i2, ui, cx).into_any_element()));
        }
        v
    }

    /// Over the cards: where the index is from, how old, how many it
    /// lists; or that it is loading, offline, failed, empty or not set up.
    pub(super) fn browse_state(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let boxed = |sel: &'static str| div().debug_selector(move || sel.into()).flex().flex_col().gap(px(8.)).p(px(14.)).rounded(px(10.)).border_1().border_dashed().border_color(ui.t.line);
        let again = |ui: &Ui, cx: &mut Context<Self>| btn("extensions-browse-again".into(), "Try again", BtnKind::Small, false, ui).on_click(cx.listener(|v, _, _, cx| v.load_index(true, cx)));
        match self.exts.page.browse.load.clone() {
            Load::Idle | Load::Loading => {
                let motion = crate::theme::motion_on(cx);
                let mut col = div().debug_selector(|| "extensions-browse-loading".into()).flex().flex_col().gap(px(8.)).child(div().flex().items_center().gap(px(8.)).child(crate::kit::spinner("extensions-browse-spin", 12., motion, &ui.t)).child(crate::kit::muted("Reading the index…", &ui.t)));
                for _ in 0..3 {
                    col = col.child(div().p(px(14.)).rounded(px(10.)).border_1().border_color(ui.t.line).flex().flex_col().gap(px(8.)).child(crate::kit::feedback::skeleton_bar(12., &ui.t).w(px(160.))).child(crate::kit::feedback::skeleton_bar(10., &ui.t).w(px(320.))));
                }
                col
            }
            Load::NoSource => boxed("extensions-browse-none")
                .child(title(ui, "Browse isn't open yet"))
                .child(crate::kit::hint("The extension index opens to everyone after its security review. Until then, install a package you were given with Install from file… on the Installed tab.", &ui.t))
                .child(crate::kit::hint(format!("Authors in the SDK preview: set {} to an index's https address or dist/v1 folder, and {} to its public key, to try it.", index::ENV, index::ENV_KEY), &ui.t)),
            Load::Failed(why) if is_offline(&why) => boxed("extensions-browse-offline")
                .child(div().flex().items_center().gap(px(6.)).child(crate::kit::icon("alert", 13., ui.t.amber)).child(title(ui, "You're offline")))
                .child(crate::kit::hint("Browse reads the extension index from the internet, and there is no copy of it here yet. What is installed keeps working.", &ui.t))
                .child(div().text_size(px(12.)).text_color(ui.t.text_3).child(why))
                .child(div().flex().child(again(ui, cx))),
            Load::Failed(why) => boxed("extensions-browse-error")
                .child(div().flex().items_center().gap(px(6.)).child(crate::kit::icon("alert", 13., ui.t.amber)).child(title(ui, "Couldn't read the index")))
                .child(div().text_size(px(12.)).text_color(ui.t.amber).child(why))
                .child(div().flex().child(again(ui, cx))),
            Load::Ready(read) => {
                let shown = self.listings_shown(cx).len();
                let all = read.index().extensions.len();
                // A copy kept from before (the fetch failed), or an old one.
                let mut warn = Vec::new();
                if let Some(why) = read.cached_because() {
                    let sel = if is_offline(why) { "extensions-browse-cached-offline" } else { "extensions-browse-cached" };
                    warn.push(div().debug_selector(move || sel.into()).flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(ui.t.amber).child(crate::kit::icon("alert", 12., ui.t.amber)).child(format!("Showing the copy Pitwall kept: {why}")));
                }
                if read.stale() {
                    let d = read.index().generated.get(..10).unwrap_or_default().to_string();
                    warn.push(div().debug_selector(|| "extensions-browse-stale".into()).flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(ui.t.amber).child(crate::kit::icon("alert", 12., ui.t.amber)).child(format!("This index is from {d}, more than {} days ago: what it says may be out of date, and new blocks may be missing.", pitwall_core::extensions::index::STALE_AFTER_DAYS)));
                }
                if all == 0 {
                    return boxed("extensions-browse-empty").children(warn).child(crate::kit::muted("The index lists no extensions yet.", &ui.t));
                }
                if shown == 0 {
                    let q = &self.exts.page.browse.query;
                    let hidden = q.verified_only && !index::search(read.index(), &Query { verified_only: false, ..q.clone() }, &read.judge()).is_empty();
                    let text = if hidden { "Nothing Verified matches. Turn off Verified only to see the others.".to_string() } else if q.text.trim().is_empty() { "No extension fits these filters.".to_string() } else { format!("Nothing matches “{}”.", q.text.trim()) };
                    let clear = btn("extensions-browse-clear".into(), "Clear search and filters", BtnKind::GhostSm, false, ui).on_click(cx.listener(|v, _, _, cx| {
                        let b = &mut v.exts.page.browse;
                        b.query = Query { verified_only: true, ..Query::default() };
                        if let Some(i) = b.search.clone() {
                            i.update(cx, |i, cx| i.set_text("", cx));
                        }
                        cx.notify();
                    }));
                    return boxed("extensions-browse-nothing").children(warn).child(div().flex().items_center().gap(px(10.)).child(div().flex_1().child(crate::kit::muted(text, &ui.t))).child(clear));
                }
                let when = read.index().generated.get(..10).map(|d| format!(" · updated {d}")).unwrap_or_default();
                let reload = crate::kit::icon_btn("extensions-browse-reload", "refresh", "Read the index again", true, &ui.t).on_click(cx.listener(|v, _, _, cx| v.load_index(true, cx)));
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(warn)
                    .child(
                        div()
                            .debug_selector(|| "extensions-browse-ready".into())
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(ui.t.text_3).child(format!("{shown} of {all} · from {}{when} · no install counts or ratings: Pitwall sends nothing", read.setup.place)))
                            .child(reload),
                    )
            }
        }
    }

    /// The button for `l`: Install, Update, Installed (disabled), or why
    /// not. Install and Update open its page with the trust screen.
    fn offer_button(&mut self, ui: &Ui, l: &Listing, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let id = format!("extension-browse-install-{}", l.id);
        let downloading = self.exts.page.browse.downloading.as_deref() == Some(l.id.as_str());
        let lid = l.id.clone();
        match self.offer_of(l, cx) {
            _ if downloading => btn(id, "Downloading…", BtnKind::Small, true, ui),
            Offer::Install(v) => btn(id, "Install", BtnKind::Small, false, ui).tooltip(crate::kit::tooltip(format!("Download {v} and look at it first: nothing is installed until you press Install"))).on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                v.install_listing(&lid, cx);
            })),
            Offer::Update(ver) => btn(id, "Update", BtnKind::Small, false, ui).tooltip(crate::kit::tooltip(format!("Download {ver} and look at what changes first"))).on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                v.install_listing(&lid, cx);
            })),
            Offer::Installed => btn(id, "Installed", BtnKind::GhostSm, true, ui),
            Offer::None(why) => btn(id, "Install", BtnKind::Small, true, ui).tooltip(crate::kit::tooltip(why)),
        }
    }

    /// A listing's card.
    pub(super) fn listing_card(&mut self, id: &str, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let Some(l) = self.listing_of(id) else { return div() };
        let Some(read) = self.index_now() else { return div() };
        let selected = self.exts.page.browse.cursor.as_deref() == Some(id);
        let icon = listing_icon(ui, &l, 32.);
        let mut name = div().flex().flex_wrap().items_center().gap(px(6.)).child(div().font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(l.name.clone())).child(div().text_size(px(12.)).text_color(ui.t.text_3).child(format!("by {}", l.author)));
        if l.verified_release(&read.judge()).is_some() {
            name = name.child(verified_badge(ui, &l.id));
        }
        let summary = div().w_full().flex().items_start().gap(px(5.)).text_size(px(12.)).text_color(ui.t.text_2).child(div().flex_none().mt(px(3.)).child(crate::kit::icon("lock", 11., ui.t.text_3))).child(div().flex_1().min_w_0().child(index::summary(&l)));
        let text = div().flex_1().min_w_0().flex().flex_col().gap(px(3.)).child(name).child(crate::kit::hint(l.description.clone(), &ui.t)).child(summary);
        let button = self.offer_button(ui, &l, cx);
        let lid = l.id.clone();
        let card_id = format!("extension-listing-{}", l.id);
        let sel = card_id.clone();
        let mut card = div()
            .id(SharedString::from(card_id))
            .debug_selector(move || sel)
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(14.))
            .rounded(px(10.))
            .border_1()
            .border_color(if selected { ui.t.line_strong } else { ui.t.line })
            .when(selected, |d| d.bg(ui.t.surface_2))
            .cursor_pointer()
            .hover(|d| d.bg(ui.t.surface_2))
            .child(div().flex().gap(px(12.)).items_start().child(icon).child(text).child(div().flex_none().mt(px(2.)).child(button)))
            .on_click(cx.listener(move |v, _, _, cx| v.show_listing(Some(&lid), cx)));
        if let Some(n) = self.exts.page.browse.notes.get(&l.id) {
            card = card.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        }
        div().child(card)
    }

    /// Download the version the index offers for listing `id`
    /// (`index::download`: refused when blocked or withdrawn, its SHA-256
    /// and its author's signature checked), then show its trust screen on
    /// its page.
    pub fn install_listing(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(read) = self.index_now() else { return };
        let Some(l) = read.index().listing(id).cloned() else { return };
        let Some(v) = l.default_release(&read.judge()).cloned() else { return };
        let Some(dir) = index::downloads_dir(cx) else { return };
        self.exts.page.browse.downloading = Some(l.id.clone());
        self.exts.page.browse.notes.remove(&l.id);
        self.show_listing(Some(&l.id), cx);
        let (r2, lid, version) = (read.clone(), l.id.clone(), v.version.clone());
        let fetch = cx.background_executor().spawn(async move { index::download(r2.setup.http.as_ref(), r2.index(), &r2.fetched.blocked, &lid, &version, &dir) });
        let repo = read.setup.config.repo.clone();
        cx.spawn(async move |this, cx| {
            let r = fetch.await;
            let _ = this.update(cx, |s, cx| {
                s.exts.page.browse.downloading = None;
                match r {
                    Ok(file) => {
                        s.exts.page.browse.downloaded.insert(l.id.clone(), file.clone());
                        s.look_at_package_from(file, Some(repo), cx);
                    }
                    Err(why) => {
                        s.exts.page.browse.notes.insert(l.id.clone(), why);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The trust screen open for listing `l`'s download, if it is.
    fn listing_trust_open(&self, l: &Listing) -> bool {
        let Some(file) = self.exts.page.browse.downloaded.get(&l.id) else { return false };
        self.exts.add_file.as_ref().is_some_and(|a| a.path.as_ref() == Some(file))
    }

    /// The detail page of listing `id`.
    pub(super) fn listing_detail_parts(&mut self, id: &str, _: &mut Context<Self>) -> Vec<Part> {
        let Some(l) = self.listing_of(id) else { return vec![] };
        let name = l.name.clone();
        let mut v = vec![Part::section("ext-detail-bar", move |s, ui, _, cx| s.detail_bar(ui, &name, true, cx).into_any_element())];
        let i2 = l.id.clone();
        v.push(Part::row(("listing-head", l.id.clone()), move |s, ui, _, cx| s.listing_head(&i2, ui, cx).into_any_element()));
        if !l.screens.is_empty() {
            let i2 = l.id.clone();
            v.push(Part::section(("listing-shots", l.id.clone()), move |s, ui, _, cx| s.listing_shots(&i2, ui, cx).into_any_element()));
        }
        let l2 = l.clone();
        v.push(Part::section(("listing-about", l.id.clone()), move |_, ui, _, _| listing_about(ui, &l2).into_any_element()));
        let l2 = l.clone();
        v.push(Part::section(("listing-perms", l.id.clone()), move |_, ui, _, _| listing_permissions(ui, &l2).into_any_element()));
        let l2 = l.clone();
        v.push(Part::section(("listing-versions", l.id.clone()), move |s, ui, _, cx| {
            let installed = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == l2.id).and_then(|m| m.version.clone());
            let Some(read) = s.index_now() else { return div().into_any_element() };
            div().debug_selector(|| "extension-listing-versions".into()).flex().flex_col().gap(px(8.)).child(title(ui, "Versions")).child(versions_list(ui, &l2, &read.judge(), installed.as_deref())).into_any_element()
        }));
        let l2 = l.clone();
        v.push(Part::section(("listing-licence", l.id.clone()), move |_, ui, _, _| listing_licence(ui, &l2).into_any_element()));
        let i2 = l.id.clone();
        v.push(Part::section(("listing-report", l.id.clone()), move |s, ui, _, cx| s.report_view(&i2, ui, cx).into_any_element()));
        v
    }

    /// The detail page's head: icon, name, author, version, Verified, the
    /// button; then what the download went to: its trust screen.
    fn listing_head(&mut self, id: &str, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let Some(l) = self.listing_of(id) else { return div() };
        let Some(read) = self.index_now() else { return div() };
        let j = read.judge();
        let offered = l.default_release(&j).cloned();
        let mut name = div().flex().flex_wrap().items_center().gap(px(8.)).child(div().text_size(px(16.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(l.name.clone()));
        if let Some(v) = &offered {
            name = name.child(div().text_size(px(12.)).text_color(ui.t.text_3).child(v.version.clone()));
            name = name.child(match l.standing(v, &j) {
                Standing::Verified => verified_badge(ui, &l.id).into_any_element(),
                s => standing_chip(ui, &s),
            });
        }
        let cats = index::category_names(&l).join(", ");
        let by = if cats.is_empty() { format!("by {}", l.author) } else { format!("by {} · {cats}", l.author) };
        let text = div().flex_1().min_w_0().flex().flex_col().gap(px(3.)).child(name).child(div().text_size(px(12.)).text_color(ui.t.text_3).child(by)).child(div().text_size(px(13.)).text_color(ui.t.text_2).child(l.description.clone()));
        let button = self.offer_button(ui, &l, cx);
        let mut col = div()
            .id(SharedString::from(format!("extension-listing-page-{}", l.id)))
            .debug_selector({
                let s = format!("extension-listing-page-{}", l.id);
                move || s
            })
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(10.))
            .border_1()
            .border_color(ui.t.line)
            .child(div().flex().gap(px(14.)).items_start().child(listing_icon(ui, &l, 44.)).child(text).child(div().flex_none().mt(px(2.)).child(button)))
            .child(div().flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(ui.t.text_2).child(crate::kit::icon("lock", 11., ui.t.text_3)).child(index::summary(&l)));
        if self.exts.page.browse.downloading.as_deref() == Some(l.id.as_str()) {
            let motion = crate::theme::motion_on(cx);
            col = col.child(div().flex().items_center().gap(px(8.)).child(crate::kit::spinner("extension-listing-dl", 12., motion, &ui.t)).child(crate::kit::muted("Downloading and checking it against the index…", &ui.t)));
        }
        if let Some(n) = self.exts.page.browse.notes.get(&l.id) {
            col = col.child(div().debug_selector(|| "extension-listing-note".into()).text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        }
        if self.listing_trust_open(&l) {
            // The key the package is signed by: the download checked it is
            // one the index lists for its author.
            let key = self.exts.add_file.as_ref().and_then(|a| a.prepared.clone()).and_then(|p| catalog::package_key(&p, cx));
            let signer = match &key {
                Some(Key::New(s) | Key::Same(s)) | Some(Key::Changed { now: Some(s), .. }) => Some(s.clone()),
                _ => None,
            };
            if let Some(s) = signer {
                let listed = l.keys.iter().any(|k| k.trim() == s.public_key.trim());
                let (icon, color, text) = if listed { ("lock", ui.t.green, format!("Signed by a key the index lists for {}.", l.author)) } else { ("alert", ui.t.amber, format!("Signed by a key the index doesn't list for {}: someone else may have signed it.", l.author)) };
                col = col.child(div().debug_selector(|| "extension-listing-key".into()).flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(ui.t.text_2).child(crate::kit::icon(icon, 12., color)).child(text));
            }
            if let Some(card) = self.add_file_view(ui, cx) {
                col = col.child(card);
            }
        }
        div().child(col)
    }

    fn listing_shots(&mut self, id: &str, ui: &Ui, _: &mut Context<Self>) -> Div {
        let Some(l) = self.listing_of(id) else { return div() };
        let mut images = Vec::new();
        for p in l.screens.iter().take(pitwall_core::extensions::package::MAX_SCREENSHOTS) {
            if let Some(Ok(i)) = self.exts.page.browse.screens.get(p) {
                images.push(img(i.clone()));
            }
        }
        if images.is_empty() {
            let loading = l.screens.iter().any(|p| !self.exts.page.browse.screens.contains_key(p));
            return div().flex().flex_col().gap(px(8.)).child(title(ui, "Screenshots")).child(crate::kit::hint(if loading { "Loading…" } else { "They couldn't be read." }, &ui.t));
        }
        screenshots_row(ui, images)
    }

    /// The version a report is about: the one offered, else the newest.
    fn report_release(&self, l: &Listing) -> Option<Release> {
        let read = self.index_now()?;
        l.default_release(&read.judge()).or_else(|| index::newest_first(l).into_iter().next()).cloned()
    }

    fn report_address(&self, l: &Listing) -> Option<String> {
        let read = self.index_now()?;
        let kind = ReportKind::ALL[self.exts.page.browse.report.get(&l.id).copied().unwrap_or(2).min(ReportKind::ALL.len() - 1)];
        let v = self.report_release(l);
        Some(index::report_url(&read.setup.config.repo, &l.id, v.as_ref().map_or("", |v| v.version.as_str()), v.as_ref().map_or("", |v| v.sha256.as_str()), kind))
    }

    /// "Report a problem" (§7.5): a kind, then the issue form opened in
    /// the browser, filled in. Nothing is sent from Pitwall.
    fn report_view(&mut self, id: &str, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let Some(l) = self.listing_of(id) else { return div() };
        let kind = self.exts.page.browse.report.get(&l.id).copied().unwrap_or(2);
        let me = cx.entity().downgrade();
        let lid = l.id.clone();
        let kinds = crate::kit::seg(
            SharedString::from(format!("extension-report-kind-{}", l.id)),
            ReportKind::ALL.iter().enumerate().map(|(i, k)| SegItem::new(k.words(), i == kind)).collect(),
            &ui.t,
            std::rc::Rc::new(move |i, _, cx| {
                let _ = me.update(cx, |v, cx| {
                    v.exts.page.browse.report.insert(lid.clone(), i);
                    cx.notify();
                });
            }),
        );
        let url = self.report_address(&l).unwrap_or_default();
        let open = btn(format!("extension-report-{}", l.id), "Report a problem…", BtnKind::GhostSm, false, ui).tooltip(crate::kit::tooltip("Opens the index's issue form in your browser")).on_click(move |_, _, cx| crate::platform::files::open_url(cx, &url));
        div()
            .debug_selector(|| "extension-listing-report".into())
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(title(ui, "Report a problem"))
            .child(div().flex().flex_wrap().items_center().gap(px(8.)).child(kinds).child(div().flex_1()).child(open))
            .child(crate::kit::hint("Opens a new issue on the index's GitHub repository in your browser, filled in with its id, version and file's SHA-256. Nothing is sent from Pitwall. Security problems: the index's SECURITY.md has a private address.", &ui.t))
    }

    /// Tests: the "Report a problem" address shown for listing `id`.
    pub fn report_url_of(&self, id: &str) -> Option<String> {
        self.report_address(&self.listing_of(id)?)
    }

    /// Tests: Browse's filters.
    pub fn set_browse_query(&mut self, category: Option<&str>, verified_only: bool, works_here: bool, cx: &mut Context<Self>) {
        let q = &mut self.exts.page.browse.query;
        q.category = category.map(str::to_string);
        q.verified_only = verified_only;
        q.works_here = works_here;
        cx.notify();
    }

    /// Tests: how reading the index stands, and the listings shown.
    pub fn browse_state_now(&mut self, cx: &mut Context<Self>) -> (&'static str, Vec<String>) {
        let word = match &self.exts.page.browse.load {
            Load::Idle => "idle",
            Load::NoSource => "none",
            Load::Loading => "loading",
            Load::Ready(..) => "ready",
            Load::Failed(why) if is_offline(why) => "offline",
            Load::Failed(_) => "error",
        };
        (word, self.listings_shown(cx))
    }

    /// Tests: whether listing `id`'s download is being looked at on the
    /// trust screen.
    pub fn listing_trust_shown(&self, id: &str) -> bool {
        self.listing_of(id).is_some_and(|l| self.listing_trust_open(&l))
    }
}

/// A category's chip.
fn category_chip(id: &str, name: &str, on: bool, ui: &Ui) -> gpui::Stateful<Div> {
    let sid = format!("extensions-category-{id}");
    let sel = sid.clone();
    let d = div().id(SharedString::from(sid)).debug_selector(move || sel).h(px(22.)).px(px(8.)).flex().items_center().rounded(px(6.)).cursor_pointer().text_size(px(12.));
    if on {
        d.bg(ui.t.surface_3).text_color(ui.t.text).font_weight(crate::kit::WEIGHT_550)
    } else {
        d.text_color(ui.t.text_3).hover(|d| d.bg(ui.t.surface_2).text_color(ui.t.text))
    }
    .child(name.to_string())
}

/// A listing's icon in a square.
fn listing_icon(ui: &Ui, l: &Listing, size: f32) -> Div {
    let name = l.icon.as_deref().filter(|i| crate::kit::has_icon(i)).unwrap_or(crate::extensions::DEFAULT_ICON);
    div().flex_none().size(px(size)).rounded(px(size / 4.)).bg(ui.t.surface_2).flex().items_center().justify_center().child(crate::kit::icon(name, size / 2., ui.t.text_2))
}

/// What it does: its line, and where its README is (the index carries no
/// README; its repository has it).
fn listing_about(ui: &Ui, l: &Listing) -> Div {
    let col = div().debug_selector(|| "extension-listing-about".into()).flex().flex_col().gap(px(8.)).child(title(ui, "About")).child(div().text_size(px(13.)).text_color(ui.t.text).child(l.description.clone()));
    let u = format!("{}#readme", l.repository);
    col.child(div().flex().child(btn(format!("extension-listing-readme-{}", l.id), "Read its README on GitHub", BtnKind::Link, false, ui).tooltip(crate::kit::tooltip("Opens its repository in your browser")).on_click(move |_, _, cx| crate::platform::files::open_url(cx, &u))))
}

/// What a listing asks for, each with its reason and what it gives (and,
/// for those that act on agents or files, what can go wrong).
fn listing_permissions(ui: &Ui, l: &Listing) -> Div {
    let mut col = div().debug_selector(|| "extension-listing-permissions".into()).flex().flex_col().gap(px(10.)).child(title(ui, "Permissions"));
    let asks = index::asks(l);
    if asks.is_empty() {
        return col.child(crate::kit::hint("It asks for no permissions.", &ui.t));
    }
    for p in asks {
        let reason = if p.reason.trim().is_empty() { "none given".to_string() } else { p.reason.clone() };
        let mut words = SettingsView::permission_words(ui, &p.name, &reason);
        if p.name == permissions::NET && !p.hosts.is_empty() {
            words = words.child(div().mt(px(2.)).flex().flex_wrap().gap(px(4.)).children(p.hosts.iter().map(|h| div().px(px(6.)).py(px(1.)).rounded(px(4.)).bg(ui.t.surface_3).child(crate::kit::mono_text(h.clone(), &ui.t).text_color(ui.t.text_2)))));
        }
        if let Some(a) = &p.amount {
            words = words.child(div().text_size(px(12.)).text_color(ui.t.text_3).child(format!("Up to {a}")));
        }
        col = col.child(words);
    }
    col.child(crate::kit::hint("Installing grants none of them: you allow or refuse each when you turn it on.", &ui.t))
}

/// Licence and repository.
fn listing_licence(ui: &Ui, l: &Listing) -> Div {
    let row = |label: &str, value: gpui::AnyElement| div().flex().items_center().gap(px(10.)).child(div().w(px(90.)).flex_none().text_size(px(12.)).text_color(ui.t.text_3).child(label.to_string())).child(value);
    let mut col = div().debug_selector(|| "extension-listing-licence".into()).flex().flex_col().gap(px(8.)).child(title(ui, "Licence and source"));
    let licence = if l.license.trim().is_empty() { "None given".to_string() } else { l.license.clone() };
    col = col.child(row("Licence", div().text_size(px(12.5)).text_color(ui.t.text).child(licence).into_any_element()));
    let u = l.repository.clone();
    col = col.child(row("Repository", btn(format!("extension-listing-repo-{}", l.id), l.repository.trim_start_matches("https://").to_string(), BtnKind::Link, false, ui).tooltip(crate::kit::tooltip("Open in the browser")).on_click(move |_, _, cx| crate::platform::files::open_url(cx, &u)).into_any_element()));
    col.child(row("Id", crate::kit::mono_text(l.id.clone(), &ui.t).text_color(ui.t.text_2).into_any_element()))
}
