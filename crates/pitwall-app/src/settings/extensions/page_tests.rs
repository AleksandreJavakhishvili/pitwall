//! Settings → Extensions as a page, headless (GPUI's test platform): the
//! Installed tab's search, filters, order, rows and keys, an extension's
//! detail page, and Browse with a fixture index (its states, search,
//! categories, Verified only, a listing's page, Report a problem) and an
//! install from the index that stops at the package's trust screen.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use super::page::{Filters, OnOff, Sort, Tab};
use crate::extensions::catalog;
use crate::extensions::index;
use pitwall_core::testing::index::IndexFixture;

const WAIT: Duration = Duration::from_secs(60);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// Folder `from` put in `exts` as `id`, with only `files` (and `dirs`).
fn put(exts: &Path, id: &str, from: &Path, files: &[&str], dirs: &[&str]) -> PathBuf {
    let to = exts.join(id);
    std::fs::create_dir_all(&to).unwrap();
    for f in files {
        if from.join(f).exists() {
            std::fs::copy(from.join(f), to.join(f)).unwrap();
        }
    }
    for d in dirs {
        if from.join(d).is_dir() {
            copy_dir(&from.join(d), &to.join(d));
        }
    }
    to
}

/// A png of `w`×`h` in two tones, for screenshots.
pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| if (x / 24 + y / 24) % 2 == 0 { image::Rgba([70, 90, 140, 255]) } else { image::Rgba([40, 50, 80, 255]) });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// Three extensions put in `exts` by hand: C4 diagrams (tiles, agent
/// tools, a README), Demo inbox (an inbox, settings) and Hello (tiles,
/// agent tools, a README, licence, changelog and a screenshot). Their
/// folders' times: C4 newest, then Hello, then Demo inbox.
pub(crate) fn installed(exts: &Path, shot: Option<&Path>) {
    let c4 = put(exts, "c4", &repo().join("tiles/c4"), &["extension.json", "tile.wasm", "input.schema.json", "README.md"], &["assets"]);
    let demo = put(exts, "demo-inbox", &repo().join("extensions/demo-inbox"), &["extension.json", "adapter.mjs"], &[]);
    let hello = put(exts, "hello", &repo().join("crates/pitwall-tile-host/tests/fixtures/hello"), &["extension.json", "tile.wasm", "input.schema.json"], &["assets"]);
    std::fs::write(hello.join("README.md"), "# Hello\n\nA counter, a greeting and a text field: the tile host's own test fixture.\n\n- Open it from the palette\n- Agents can show a message in it\n").unwrap();
    std::fs::write(hello.join("LICENSE"), "Apache License\nVersion 2.0\n").unwrap();
    std::fs::write(hello.join("CHANGELOG.md"), "## 0.1.0\n\nThe first version.\n").unwrap();
    std::fs::create_dir_all(hello.join("screenshots")).unwrap();
    match shot {
        Some(s) => std::fs::copy(s, hello.join("screenshots/main.png")).map(|_| ()).unwrap(),
        None => std::fs::write(hello.join("screenshots/main.png"), png(96, 60)).unwrap(),
    }
    let now = SystemTime::now();
    for (dir, ago) in [(&c4, 10), (&hello, 3_600), (&demo, 86_400)] {
        std::fs::File::open(dir).unwrap().set_modified(now - Duration::from_secs(ago)).unwrap();
    }
}

/// A fixture index (built long ago, so stale): CI status (1.0.0
/// Verified, 1.1.0 not yet reviewed), DB browser (never reviewed, in
/// Databases), Old thing (needs a newer Pitwall). Its screenshots served.
pub(crate) fn fixture_index() -> IndexFixture {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", index::tests::CI_PERMS);
    f.release("ci-status", "1.1.0", index::tests::CI_PERMS);
    f.review("ci-status", "1.0.0");
    f.release("db-browser", "0.3.0", r#""files.read": {"reason": "Open the SQLite files in your project"}"#);
    f.edit("db-browser", |e| e.categories = vec!["databases".into()]);
    f.release("old-thing", "2.0.0", "");
    f.edit("old-thing", |e| e.categories = vec!["fun".into()]);
    publish(&f, "2026-01-02T10:00:00Z");
    f
}

/// Built at `now`, its screenshots served too.
pub(crate) fn publish(f: &IndexFixture, now: &str) {
    let b = f.publish(now);
    for (path, bytes) in b.screens {
        f.http.put(&format!("{}{path}", pitwall_core::testing::index::BASE), bytes);
    }
}

/// Now, as the index writes it.
pub(crate) fn recent() -> String {
    pitwall_core::extensions::index::format_time(index::now())
}

/// Browse reading `f`.
pub(crate) fn setup_of(f: &IndexFixture) -> index::Setup {
    use pitwall_core::extensions::index::IndexSource as _;
    let got = index::tests::needs_newer(f.source().fetch().unwrap());
    index::Setup::new(Arc::new(index::FixtureSource::new(got.index, got.blocked)), f.config(), f.http.clone(), "the test index")
}

pub(crate) fn init(root: &Path, cx: &mut TestAppContext) {
    let root = root.to_path_buf();
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        cx.bind_keys(super::page::bindings());
        cx.bind_keys(super::super::view::bindings());
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(root.clone()), cx);
        catalog::set_bundle(None, cx);
        index::set_setup(None, cx);
        cx.set_global(crate::tiles::DataDir(root.clone()));
        crate::settings::data::set_page(cx, Page::Extensions);
    });
}

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        frame(cx);
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A selector as `debug_bounds` takes it (tests only: leaked).
fn sel(id: &str) -> &'static str {
    Box::leak(id.to_string().into_boxed_str())
}

fn click(cx: &mut VisualTestContext, id: &str) {
    let id = sel(id);
    frame(cx);
    let b = cx.debug_bounds(id).unwrap_or_else(|| panic!("{id} isn't drawn"));
    cx.simulate_mouse_move(b.center(), None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
    frame(cx);
}

fn drawn(cx: &mut VisualTestContext, id: &str) -> bool {
    frame(cx);
    cx.debug_bounds(sel(id)).is_some()
}

fn state(view: &Entity<SettingsView>, cx: &mut VisualTestContext) -> (Tab, Option<String>, Vec<String>, Option<String>) {
    view.update(cx, |v, cx| v.extensions_page_state(cx))
}

fn ids(view: &Entity<SettingsView>, cx: &mut VisualTestContext) -> Vec<String> {
    state(view, cx).2
}

fn open_settings(view: &Entity<SettingsView>, cx: &mut VisualTestContext) {
    // The page at the window's full height (as in a tile), so a test sees
    // every card without scrolling.
    view.update(cx, |v, cx| v.set_tiled(true, cx));
    cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(2400.)));
    frame(cx);
}

#[gpui::test]
fn installed_search_filters_order_and_keys(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-page-installed");
    let root = t.path().join("data");
    let exts = root.join("extensions");
    installed(&exts, None);
    init(&root, cx);
    cx.update(|cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("hello", true)));
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    open_settings(&view, cx);
    assert!(drawn(cx, "extensions-tabs"));

    // By name; the cards as always, each with its switch.
    assert_eq!(ids(&view, cx), ["c4", "demo-inbox", "hello"]);
    assert!(drawn(cx, "extension-switch-c4") && drawn(cx, "extension-details-hello"));
    // Recently updated: the newest folder first.
    view.update(cx, |v, cx| v.set_extension_filters(Filters::default(), Sort::Updated, false, cx));
    assert_eq!(ids(&view, cx), ["c4", "hello", "demo-inbox"]);

    // The search: words in any order, in its name, line or tiles.
    view.update(cx, |v, cx| v.set_extension_filters(Filters::default(), Sort::Name, false, cx));
    view.update_in(cx, |v, w, cx| v.search_extensions("diagrams agents", w, cx));
    assert_eq!(ids(&view, cx), ["c4"]);
    view.update_in(cx, |v, w, cx| v.search_extensions("nothing like this", w, cx));
    assert!(ids(&view, cx).is_empty());
    assert!(drawn(cx, "extensions-empty"));
    click(cx, "extensions-clear");
    assert_eq!(ids(&view, cx).len(), 3, "Clear brings them all back");

    // The filters.
    let only = |f: Filters| f;
    view.update(cx, |v, cx| v.set_extension_filters(only(Filters { on: OnOff::On, ..Default::default() }), Sort::Name, false, cx));
    assert_eq!(ids(&view, cx), ["hello"]);
    view.update(cx, |v, cx| v.set_extension_filters(only(Filters { on: OnOff::Off, ..Default::default() }), Sort::Name, false, cx));
    assert_eq!(ids(&view, cx), ["c4", "demo-inbox"]);
    view.update(cx, |v, cx| v.set_extension_filters(only(Filters { tools: true, ..Default::default() }), Sort::Name, false, cx));
    assert_eq!(ids(&view, cx), ["c4", "hello"]);
    view.update(cx, |v, cx| v.set_extension_filters(only(Filters { tiles: true, on: OnOff::Off, ..Default::default() }), Sort::Name, false, cx));
    assert_eq!(ids(&view, cx), ["c4"]);
    view.update(cx, |v, cx| v.set_extension_filters(only(Filters { updates: true, ..Default::default() }), Sort::Name, false, cx));
    assert!(ids(&view, cx).is_empty(), "no update offered");
    view.update(cx, |v, cx| v.set_extension_filters(Filters::default(), Sort::Name, false, cx));
    click(cx, "extensions-filter-services");
    assert!(ids(&view, cx).is_empty(), "none keeps a service");
    click(cx, "extensions-filter-services");
    assert_eq!(ids(&view, cx).len(), 3);

    // Rows: a compact row each, with its switch; a click opens its page.
    view.update(cx, |v, cx| v.set_extension_filters(Filters::default(), Sort::Name, true, cx));
    assert!(drawn(cx, "extension-row-c4") && drawn(cx, "extension-switch-demo-inbox"));
    assert!(!drawn(cx, "extension-details-c4"), "no cards in rows");

    // The keys: ⌘F focuses the search, ↓ moves the selection, Enter opens it.
    view.update_in(cx, |v, w, cx| w.focus(&v.focus, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-f" } else { "ctrl-f" });
    frame(cx);
    let focused = view.update_in(cx, |v, w, cx| v.exts.page.search.as_ref().is_some_and(|i| i.read(cx).focus_handle(cx).is_focused(w)));
    assert!(focused, "⌘F focuses the search");
    cx.simulate_keystrokes("down down");
    assert_eq!(state(&view, cx).3.as_deref(), Some("demo-inbox"));
    cx.simulate_keystrokes("up up");
    assert_eq!(state(&view, cx).3.as_deref(), Some("hello"), "↑ wraps");
    cx.simulate_keystrokes("enter");
    frame(cx);
    assert_eq!(state(&view, cx).1.as_deref(), Some("hello"), "Enter opens the selected one");
}

#[gpui::test]
fn the_detail_page_has_everything_about_it(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-page-detail");
    let root = t.path().join("data");
    installed(&root.join("extensions"), None);
    init(&root, cx);
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    open_settings(&view, cx);
    click(cx, "extension-details-hello");
    assert_eq!(state(&view, cx).1.as_deref(), Some("hello"));
    // Its card, unfolded, with its switch and its footer (Remove …).
    for id in ["extension-card-hello", "extension-switch-hello", "extension-remove-hello", "extension-trust-hello", "extension-readme", "extension-screenshots", "extension-detail-permissions", "extension-adds-hello", "extension-detail-versions", "extension-detail-licence", "extension-licence-open-hello"] {
        assert!(drawn(cx, id), "{id} isn't drawn");
    }
    assert!(!drawn(cx, "extension-card-c4"), "only its own card");
    // Turned on, its permissions have switches on the card.
    click(cx, "extension-switch-hello");
    click(cx, "extension-permissions-on-hello");
    assert!(drawn(cx, "extension-agents-hello"), "agent tools and their switches");
    assert!(!drawn(cx, "extension-detail-permissions"), "on: the card has them");
    click(cx, "extension-detail-back");
    assert_eq!(state(&view, cx).1, None);
    assert!(drawn(cx, "extension-card-c4"));
    // An agent's show_settings naming it opens its page.
    view.update(cx, |v, cx| v.open_extension("c4", cx));
    assert_eq!(state(&view, cx).1.as_deref(), Some("c4"));
    assert!(drawn(cx, "extension-readme"), "C4's README");
}

#[gpui::test]
fn browse_reads_the_index_and_says_how_it_stands(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-page-browse");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    init(&root, cx);
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    open_settings(&view, cx);

    // No index set up: it says so.
    view.update(cx, |v, cx| v.set_extensions_tab(Tab::Browse, cx));
    assert!(drawn(cx, "extensions-browse-none"));

    // Offline, then an index that didn't check: each with Try again.
    let f = fixture_index();
    let failing = |why: &str| index::Setup::new(Arc::new(index::FixtureSource::failing(why)), f.config(), f.http.clone(), "the test index");
    cx.update(|_, cx| index::set_setup(Some(failing("Couldn't reach the extensions index: this computer seems to be offline.")), cx));
    view.update(cx, |v, cx| v.load_index(true, cx));
    until(cx, "offline", |cx| cx.debug_bounds("extensions-browse-offline").is_some());
    cx.update(|_, cx| index::set_setup(Some(failing("index.json didn't check: not signed by the index key.")), cx));
    click(cx, "extensions-browse-again");
    until(cx, "the error", |cx| cx.debug_bounds("extensions-browse-error").is_some());
    cx.update(|_, cx| index::set_setup(Some(setup_of(&f)), cx));
    click(cx, "extensions-browse-again");
    until(cx, "the index", |cx| cx.debug_bounds("extensions-browse-ready").is_some());
    assert!(drawn(cx, "extensions-browse-stale"), "built long ago: it says it may be out of date");

    // Verified only, at first.
    let shown = |view: &Entity<SettingsView>, cx: &mut VisualTestContext| view.update(cx, |v, cx| v.browse_state_now(cx).1);
    assert_eq!(shown(&view, cx), ["ci-status"]);
    assert!(drawn(cx, "extension-listing-ci-status") && drawn(cx, "extension-verified-ci-status"));
    click(cx, "extensions-browse-verified");
    assert_eq!(shown(&view, cx), ["ci-status", "db-browser", "old-thing"]);
    click(cx, "extensions-browse-works");
    assert_eq!(shown(&view, cx), ["ci-status", "db-browser"]);
    click(cx, "extensions-category-databases");
    assert_eq!(shown(&view, cx), ["db-browser"]);
    click(cx, "extensions-category-all");
    view.update_in(cx, |v, w, cx| v.search_extensions("octo ci", w, cx));
    assert_eq!(shown(&view, cx), ["ci-status"]);
    view.update_in(cx, |v, w, cx| v.search_extensions("zzz", w, cx));
    assert!(drawn(cx, "extensions-browse-nothing"));
    click(cx, "extensions-browse-clear");
    assert_eq!(shown(&view, cx), ["ci-status"], "Clear: Verified only again");

    // A listing's page: screenshots, what it does, permissions with their
    // reasons and risk notes, versions and how each stands, licence,
    // Report a problem.
    click(cx, "extension-listing-ci-status");
    assert_eq!(state(&view, cx).1.as_deref(), Some("ci-status"));
    until(cx, "its screenshot asked for", |_| f.http.requests().iter().any(|(u, _)| u.contains("/screens/ci-status/")));
    for id in ["extension-listing-page-ci-status", "extension-listing-about", "extension-listing-permissions", "extension-permission-risk-agents.queue", "extension-listing-versions", "extension-listing-licence", "extension-listing-report", "extension-report-ci-status"] {
        assert!(drawn(cx, id), "{id} isn't drawn");
    }
    let url = view.read_with(cx, |v, _| v.report_url_of("ci-status")).unwrap();
    assert!(url.starts_with("https://github.com/pitwalls/pitwall-extensions/issues/new?") && url.contains("ci-status") && url.contains("broken"), "{url}");
    click(cx, "extension-detail-back");
    assert_eq!(state(&view, cx).1, None);
    // The newest isn't reviewed yet: Install offers the Verified one.
    assert_eq!(view.read_with(cx, |v, cx| v.offer_of(&v.listing_of("ci-status").unwrap(), cx)), super::browse::Offer::Install("1.0.0".into()));
}

#[gpui::test]
fn installing_from_the_index_goes_through_the_trust_screen(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-page-install");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    init(&root, cx);
    let f = fixture_index();
    cx.update(|cx| index::set_setup(Some(setup_of(&f)), cx));
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    open_settings(&view, cx);
    view.update(cx, |v, cx| v.set_extensions_tab(Tab::Browse, cx));
    until(cx, "the index", |cx| cx.debug_bounds("extensions-browse-ready").is_some());

    // Install on the card: downloaded, checked, then its trust screen on
    // its page; nothing is installed yet.
    click(cx, "extension-browse-install-ci-status");
    until(cx, "the trust screen", |cx| view.read_with(cx, |v, _| v.listing_trust_shown("ci-status")) && cx.debug_bounds("extension-add-file-install").is_some());
    assert_eq!(state(&view, cx).1.as_deref(), Some("ci-status"));
    assert!(drawn(cx, "extension-listing-key"));
    assert!(!root.join("extensions/ci-status").exists(), "not installed before Install");
    click(cx, "extension-add-file-install");
    until(cx, "the install", |cx| view.read_with(cx, |v, _| v.add_file_state().is_none()));
    assert!(root.join("extensions/ci-status/extension.json").exists());
    assert_eq!(view.read_with(cx, |v, cx| v.offer_of(&v.listing_of("ci-status").unwrap(), cx)), super::browse::Offer::Installed);
    // Recorded as from the index: its updates come from there.
    match pitwall_core::extensions::install::origin(&root.join("extensions/ci-status")) {
        Some(pitwall_core::extensions::install::Origin::Package { index, .. }) => assert_eq!(index.as_deref(), Some("pitwalls/pitwall-extensions")),
        o => panic!("{o:?}"),
    }

    // The daily check: the block list now covers it, so it is turned off
    // and its card says why.
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("ci-status", true)));
    f.block(pitwall_core::extensions::index::Block { id: "ci-status".into(), versions: vec!["1.0.0".into()], sha256: vec![], reason: "sends tokens away".into(), date: "2026-11-27".into() });
    publish(&f, &recent());
    cx.update(|_, cx| index::set_setup(Some(setup_of(&f)), cx));
    view.update(cx, |v, cx| v.load_index(true, cx));
    until(cx, "the block applied", |cx| cx.update(|_, cx| !crate::ui_state::get(cx).extension_on("ci-status")));
    view.update(cx, |v, cx| v.set_extensions_tab(Tab::Installed, cx));
    assert!(drawn(cx, "extension-index-note-ci-status"), "the turned-off line on its card");

    // A download that isn't what the index lists is refused.
    let url = format!("{}/releases/download/v0.3.0/db-browser-0.3.0.pwtile", IndexFixture::repository("db-browser"));
    f.http.put(&url, b"not it".to_vec());
    view.update(cx, |v, cx| v.show_listing(None, cx));
    click(cx, "extensions-browse-verified");
    click(cx, "extension-browse-install-db-browser");
    until(cx, "the refusal", |cx| cx.debug_bounds("extension-listing-note").is_some());
    assert!(!root.join("extensions/db-browser").exists());
}

#[gpui::test]
fn the_extensions_surface_is_two_columns_when_wide(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-page-surface");
    let root = t.path().join("data");
    installed(&root.join("extensions"), None);
    init(&root, cx);
    // The surface shows Extensions whatever page Settings shows.
    cx.update(|cx| crate::settings::data::set_page(cx, Page::General));
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::extensions_surface(cx));
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(900.)));
    until(cx, "the wide surface", |cx| cx.debug_bounds("extensions-list").is_some());
    assert!(drawn(cx, "extensions-surface") && !drawn(cx, "settings-nav"), "no page list");
    // The list beside the page, rows there; a row opens its page beside.
    assert!(drawn(cx, "extension-row-hello") && !drawn(cx, "extensions-layout"));
    click(cx, "extension-row-hello");
    assert_eq!(state(&view, cx).1.as_deref(), Some("hello"));
    assert!(drawn(cx, "extension-row-c4"), "the list stays");
    assert!(drawn(cx, "extension-detail-permissions"));

    // Narrow: one column, the page alone as in Settings.
    cx.simulate_resize(gpui::size(gpui::px(640.), gpui::px(900.)));
    until(cx, "one column", |cx| cx.debug_bounds("extensions-list").is_none());
    assert!(drawn(cx, "extension-detail-back"));
    click(cx, "extension-detail-back");
    assert!(drawn(cx, "extensions-tabs"));
}

#[gpui::test]
fn the_dialogs_page_is_compact(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-page-compact");
    let root = t.path().join("data");
    installed(&root.join("extensions"), None);
    init(&root, cx);
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut v = SettingsView::new(true, cx);
        v.set_compact_extensions(true, cx);
        v
    });
    open_settings(&view, cx);
    assert!(drawn(cx, "extensions-compact") && drawn(cx, "extensions-open-surface"));
    assert!(!drawn(cx, "extensions-tabs"), "Browse is in the Extensions tile");
    assert!(drawn(cx, "extension-row-c4") && !drawn(cx, "extension-details-c4"), "rows, not cards");
    // The search still narrows it.
    view.update_in(cx, |v, w, cx| v.search_extensions("diagrams", w, cx));
    assert_eq!(ids(&view, cx), ["c4"]);
    // A row's page is the tile's: this page stays the list.
    click(cx, "extension-row-c4");
    assert_eq!(state(&view, cx).1, None);
    assert!(drawn(cx, "extensions-compact"));
}

#[test]
fn the_dialog_is_sized_to_the_window() {
    use super::super::view::{fit_size, MIN_HEIGHT, MIN_WIDTH};
    assert_eq!(fit_size((100., 100.), (2000., 1400.)), (MIN_WIDTH, MIN_HEIGHT));
    assert_eq!(fit_size((5000., 5000.), (2000., 1400.)), (1952., 1352.));
    assert_eq!(fit_size((1000., 700.), (2000., 1400.)), (1000., 700.));
    // A window smaller than the smallest dialog: the smallest.
    assert_eq!(fit_size((1000., 700.), (600., 400.)), (MIN_WIDTH, MIN_HEIGHT));
}
