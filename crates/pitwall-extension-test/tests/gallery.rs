//! The components (ui.md §3.3) through the harness, on the hello
//! extension's `gallery` tile: a 100 000-row list, a sortable table, a
//! tree, tabs, a select, menus, Markdown and images.

use std::path::{Path, PathBuf};

use pitwall_extension_test::Harness;

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
}

fn path(t: &pitwall_extension_test::Session, key: &str) -> String {
    t.find_key(key).unwrap_or_else(|| panic!("no {key}")).key_path
}

#[test]
fn a_long_list_renders_rows_on_demand() {
    let mut t = Harness::load(hello()).open("gallery");
    let list = path(&t, "rows");
    assert_eq!(t.row_count(&list), 100_000);
    let rows = t.rows(&list, 99_990, 64);
    assert_eq!(rows.len(), 10, "only to the end");
    assert_eq!((rows[0].index, rows[0].key.as_str()), (99_990, "r99990"));
    assert!(rows[0].text.contains("Row 99990"), "{:?}", rows[0].text);
    assert_eq!(rows[0].menu().unwrap(), ["Open", "Copy name", "Remove"]);
    // Selecting and opening.
    t.select_row(&list, "Row 12");
    assert_eq!(t.selected_rows(&list), ["r12"]);
    assert!(t.find_text("Selected r12").is_some());
    t.activate_row(&list, "Row 3");
    assert!(t.find_text("Opened r3").is_some());
    // A row's context menu.
    t.choose_row_menu_item(&list, "Row 7", "Copy name");
    assert!(t.find_text("Row 7: copy").is_some(), "{}", t.text_snapshot());
}

#[test]
fn tables_sort_and_resize() {
    let mut t = Harness::load(hello()).open("gallery");
    let table = path(&t, "files");
    let names = |t: &mut pitwall_extension_test::Session| t.rows(&table, 0, 10).into_iter().map(|r| r.key).collect::<Vec<_>>();
    assert_eq!(names(&mut t), ["build.rs", "logo.svg", "notes.md", "README.md"]);
    assert_eq!(t.rows(&table, 0, 1)[0].text, "build.rs 13.1 KB Monday");
    t.sort_by(&table, "Size");
    assert_eq!(names(&mut t), ["logo.svg", "notes.md", "README.md", "build.rs"]);
    t.sort_by(&table, "Size");
    assert_eq!(names(&mut t), ["build.rs", "README.md", "notes.md", "logo.svg"]);
    assert!(t.text_snapshot().contains("table rows=4 columns=[Name, Size, Modified] sort=size↓ #files [sort, column-resized]"), "{}", t.text_snapshot());
    t.resize_column(&table, "Modified", 150.0);
    assert!(t.find_text("when is 150 wide").is_some());
}

#[test]
fn trees_open_and_close() {
    let mut t = Harness::load(hello()).open("gallery");
    let tree = path(&t, "tree");
    let shown = |t: &mut pitwall_extension_test::Session| t.rows(&tree, 0, 20).into_iter().map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.text)).collect::<Vec<_>>();
    assert_eq!(shown(&mut t), ["src", "  lib.rs", "  view", "assets", "README.md"]);
    let src = t.rows(&tree, 0, 1).remove(0);
    assert_eq!(src.expanded, Some(true));
    t.expand_row(&tree, "view").expand_row(&tree, "assets");
    assert_eq!(shown(&mut t), ["src", "  lib.rs", "  view", "    list.rs", "assets", "  logo.svg", "README.md"]);
    t.collapse_row(&tree, "src");
    assert_eq!(shown(&mut t), ["src", "assets", "  logo.svg", "README.md"]);
    assert_eq!(t.rows(&tree, 3, 1)[0].expanded, None, "a file has no children");
}

#[test]
fn tabs_selects_and_menus() {
    let mut t = Harness::load(hello()).open("gallery");
    let snap = t.text_snapshot();
    assert!(snap.contains("tabs [*Overview, Files, Log] #tabs [select]"), "{snap}");
    assert!(snap.contains("select \"Medium\" #size [select]"), "{snap}");
    assert!(snap.contains("menu-button \"Actions\" [Copy, Refresh, Delete] #Actions [select]"), "{snap}");
    let tabs = path(&t, "tabs");
    t.select_tab(&tabs, "Files");
    assert!(t.text_snapshot().contains("tabs [Overview, *Files, Log]"));
    let size = path(&t, "size");
    t.select_option(&size, "Large");
    assert!(t.find_text("Size: l").is_some());
    assert!(t.text_snapshot().contains("select \"Large\""));
    let actions = path(&t, "Actions");
    assert_eq!(t.menu_items(&actions), ["Copy", "Refresh", "Delete"]);
    t.choose_menu_item(&actions, "Delete");
    assert!(t.find_text("Menu: delete").is_some());
    assert!(t.unknown_icons().is_empty(), "{:?}", t.unknown_icons());
}

#[test]
fn markdown_links_tasks_and_images() {
    let mut t = Harness::load(hello()).open("gallery");
    let md = path(&t, "markdown");
    assert!(t.text_snapshot().contains("markdown \"## Notes\" #markdown [link, toggle]"), "{}", t.text_snapshot());
    t.click_link(&md, "notes/next.md");
    assert!(t.find_text("Link: notes/next.md").is_some());
    t.toggle_task(&md, 5);
    let f = t.find_key(&md).unwrap();
    let pitwall_extension_test::Element::Markdown(m) = &f.node.element else { panic!() };
    assert!(m.text.contains("- [x] Ship it"), "{}", m.text);
    let snap = t.text_snapshot();
    assert!(snap.contains("image asset=assets/logo.svg #logo") && snap.contains("image data=dot #dot"), "{snap}");
    assert!(t.ui_requests().iter().any(|r| matches!(r, pitwall_tile_host::UiRequest::PutImage { name, .. } if name == "dot")));
}
