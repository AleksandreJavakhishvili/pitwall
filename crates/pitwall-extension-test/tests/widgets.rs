//! The form controls, feedback, layout, data display and charts (ui.md
//! §3.10) through the harness, on the pages of the hello extension's
//! `gallery` tile.

use std::path::{Path, PathBuf};

use pitwall_extension_test::{Harness, Session};

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
}

fn page(h: Harness, title: &str) -> Session {
    let mut t = h.open("gallery");
    t.select_tab("pages", title);
    t
}

fn shows(t: &Session, what: &str) {
    let s = t.text_snapshot();
    assert!(s.contains(what), "no “{what}” in:\n{s}");
}

fn lacks(t: &Session, what: &str) {
    let s = t.text_snapshot();
    assert!(!s.contains(what), "“{what}” still in:\n{s}");
}

#[test]
fn checkboxes_switches_and_choices_change_when_used() {
    let mut t = page(Harness::load(hello()), "Form controls");
    shows(&t, r#"checkbox "Only failures" on"#);
    t.toggle("failures");
    shows(&t, r#"checkbox "Only failures" off"#);
    // Mixed: a click turns it on.
    shows(&t, r#"checkbox "All branches" mixed"#);
    t.toggle("branches");
    shows(&t, r#"checkbox "All branches" on"#);
    // Space toggles a focused switch.
    assert!(t.key_on("notify", "space"));
    shows(&t, r#"switch "Notify me when a run fails" off"#);
    t.choose("scope", "failed");
    shows(&t, "(•) Failed runs");
    // The arrow keys move a segmented control's choice, and wrap.
    assert!(t.key_on("range", "right"));
    shows(&t, "(•) Month");
    assert!(t.key_on("range", "right"));
    shows(&t, "(•) Day");
}

#[test]
fn sliders_and_number_fields_keep_their_bounds() {
    let mut t = page(Harness::load(hello()), "Form controls");
    t.set_slider("volume", 73.);
    shows(&t, "slider 75 in 0..100");
    assert!(t.key_on("volume", "end"));
    shows(&t, "slider 100 in 0..100");
    // Typed past its max: kept at it.
    assert!(t.type_number("retries", "12"));
    shows(&t, "number-field 10");
    // Not a number: nothing sent.
    assert!(!t.type_number("retries", "lots"));
    t.step_number("retries", false);
    shows(&t, "number-field 9");
    assert!(t.key_on("retries", "up"));
    shows(&t, "number-field 10");
    // Emptied.
    assert!(t.type_number("retries", ""));
    shows(&t, "number-field empty");
}

#[test]
fn search_fields_and_form_rows() {
    let mut t = page(Harness::load(hello()), "Form controls");
    shows(&t, r#"search-field placeholder="Filter branches""#);
    t.type_into("query", "fix");
    assert!(t.find_text("1 of 4 branches").is_some(), "{}", t.text_snapshot());
    t.clear_search("query");
    assert!(t.find_text("4 of 4 branches").is_some());
    shows(&t, r#"form-row "Branch" required help="Runs start from this branch" error="Name a branch""#);
    t.type_into("branch", "main");
    lacks(&t, "Name a branch");
}

#[test]
fn disabled_and_readonly_controls_cant_be_changed() {
    let t = page(Harness::load(hello()), "Form controls");
    shows(&t, r#"checkbox "Can't change this" on disabled"#);
    shows(&t, r#"switch "Turned off by your admin" off disabled"#);
    shows(&t, "number-field 3 readonly");
    let f = t.find_key("box").unwrap();
    assert!(!pitwall_tile_host::widgets::focusable(&f.node.element, false), "a disabled control is no Tab stop");
    assert!(pitwall_tile_host::widgets::on_click(&f.node.element).is_none());
}

#[test]
fn banners_and_chips_go_when_dismissed() {
    let mut t = page(Harness::load(hello()), "Feedback");
    shows(&t, r#"banner bad "The CI token was refused. Sign in again to load runs." title="Can't load runs" action="Sign in" dismissible"#);
    shows(&t, "progress 62%");
    shows(&t, "progress indeterminate");
    shows(&t, "skeleton 3 lines");
    shows(&t, r#"empty-state "No runs yet""#);
    t.dismiss("token");
    lacks(&t, "Can't load runs");
    shows(&t, r#"chip "web" removable"#);
    t.dismiss("web");
    lacks(&t, r#"chip "web""#);
    // Backspace removes a focused chip.
    assert!(t.key_on("api", "backspace"));
    lacks(&t, r#"chip "api""#);
    // A chip with a click listener is picked.
    t.click_key("flaky");
    shows(&t, r#"chip "Flaky only" selected"#);
    shows(&t, r#"kbd "cmd-k""#);
}

#[test]
fn splits_disclosures_and_breadcrumbs() {
    let mut t = page(Harness::load(hello()), "Layout");
    shows(&t, "split horizontal 0.40");
    t.resize_split("panes", 0.7);
    assert!(t.find_text("Last: Nothing yet · split at 70 % · deleted 0").is_some(), "{}", t.text_snapshot());
    shows(&t, r#"disclosure "Environment" open"#);
    assert!(t.key_on("env", "enter"));
    shows(&t, r#"disclosure "Environment" closed"#);
    shows(&t, "breadcrumbs acme / web / runs / 412");
    t.choose("crumbs", "web");
    shows(&t, "breadcrumbs acme / web");
}

#[test]
fn popovers_and_confirm_sheets_close_on_escape() {
    let mut t = page(Harness::load(hello()), "Layout");
    t.click_key("open");
    shows(&t, "popover #pop");
    assert!(t.key_on("pop", "escape"));
    lacks(&t, "popover #pop");
    t.click_key("delete");
    shows(&t, r#"confirm-sheet "Delete 3 notes?" confirm="Delete" text="They can't be brought back." danger"#);
    assert!(t.key_on("ask", "escape"));
    lacks(&t, "confirm-sheet \"");
    t.click_key("delete");
    assert!(t.key_on("ask", "enter"));
    assert!(t.find_text("3 notes deleted").is_some(), "{}", t.text_snapshot());
}

#[test]
fn copy_buttons_and_links() {
    let mut t = page(Harness::load(hello()), "Data display");
    shows(&t, r#"avatar "Ada Lovelace" AL"#);
    shows(&t, r#"key-values Branch="main" Commit="4f2a9c1e""#);
    shows(&t, "relative-time");
    t.copy("sha");
    assert_eq!(t.clipboard(), Some("4f2a9c1e7b"));
    // Without `show` the link isn't opened.
    t.open_link("all");
    assert!(t.find_text("Last: All runs · deleted 0").is_some(), "{}", t.text_snapshot());
    assert!(t.opened_pages().is_empty());
    let mut t = page(Harness::load(hello()).grant("show"), "Data display");
    t.open_link("web");
    assert_eq!(t.opened_pages(), ["https://ci.example.com/runs/412"]);
}

#[test]
fn charts_describe_their_data() {
    let mut t = page(Harness::load(hello()), "Charts");
    shows(&t, "sparkline 10 values");
    shows(&t, "bar-chart");
    shows(&t, "line-chart");
    shows(&t, "status-timeline");
    t.choose("history", "406");
    assert!(t.find_text("Last: Run #406 · deleted 0").is_some(), "{}", t.text_snapshot());
}

#[test]
fn a_view_shows_one_component_alone() {
    let t = Harness::load(hello()).open_with("gallery", serde_json::json!({"component": "confirm-sheet"}));
    shows(&t, "confirm-sheet");
    lacks(&t, "Lists and menus");
}
