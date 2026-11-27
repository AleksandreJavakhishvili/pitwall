//! The gallery tile: every component of `pitwall:tile@0.1.0` at once, for
//! the host's, the harness's and the app's tests and the docs' examples
//! (docs/reference/extensions/ui.md includes its parts by their `docs:start gallery-…` marks).
//! Its tabs at the top switch to the other pages ([`crate::widgets`]).

use pitwall_extension::prelude::*;

/// The 100 000-row list's length.
pub const ROWS: usize = 100_000;

pub struct Gallery {
    /// The list's selected row.
    picked: Option<String>,
    /// What happened last, shown at the top.
    said: String,
    tab: String,
    size: String,
    /// The table's sort and its rows' order.
    sort: (String, SortDirection),
    files: Vec<(String, u64, String)>,
    /// The tree's open folders.
    open: Vec<String>,
    tasks: [bool; 2],
    /// The page shown: `lists` (this one) or one of [`crate::widgets::PAGES`].
    page: String,
}

impl Default for Gallery {
    fn default() -> Self {
        let files = [("notes.md", 2_400, "Today"), ("logo.svg", 860, "Yesterday"), ("build.rs", 13_100, "Monday"), ("README.md", 5_300, "Last week")];
        let mut g = Gallery {
            picked: None,
            said: "Nothing chosen yet".into(),
            tab: "overview".into(),
            size: "m".into(),
            sort: ("name".into(), SortDirection::Ascending),
            files: files.iter().map(|(n, s, d)| (n.to_string(), *s, d.to_string())).collect(),
            open: vec!["src".into()],
            tasks: [true, false],
            page: "lists".into(),
        };
        g.sorted();
        g
    }
}

/// The tree's items: (path, name, folder).
const TREE: &[(&str, &str, bool)] = &[("src", "src", true), ("src/lib.rs", "lib.rs", false), ("src/view", "view", true), ("src/view/list.rs", "list.rs", false), ("assets", "assets", true), ("assets/logo.svg", "logo.svg", false), ("README.md", "README.md", false)];

impl Gallery {
    /// The tree rows shown: each item whose folders are open.
    fn shown(&self) -> Vec<(&'static str, &'static str, bool, usize)> {
        TREE.iter()
            .filter(|(p, _, _)| {
                let parts: Vec<&str> = p.split('/').collect();
                (1..parts.len()).all(|k| self.open.contains(&parts[..k].join("/")))
            })
            .map(|(p, n, d)| (*p, *n, *d, p.matches('/').count()))
            .collect()
    }

    fn sorted(&mut self) {
        let (col, dir) = &self.sort;
        match col.as_str() {
            "size" => self.files.sort_by_key(|f| f.1),
            _ => self.files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase())),
        }
        if *dir == SortDirection::Descending {
            self.files.reverse();
        }
    }

    pub fn render(&mut self, widgets: &crate::widgets::Widgets, cx: &mut Cx<crate::Hello>) -> AnyElement {
        let pages = std::iter::once(tab("lists", "Lists and menus")).chain(crate::widgets::PAGES.iter().map(|(key, title, _)| tab(*key, *title)));
        let pages = tabs("pages", pages).selected(self.page.clone()).on_select(cx.listener(|t: &mut crate::Hello, key: &str, cx| {
            t.gallery.page = key.to_string();
            cx.notify();
        }));
        let root = v_flex().size_full().gap(px(10.0)).p(px(12.0)).bg(Token::Surface).child(pages);
        if self.page != "lists" {
            return root.child(div().id("other").flex_1().child(widgets.page(&self.page, cx))).into_any();
        }
        root
            .child(self.toolbar(cx))
            .child(text(self.said.clone()).id("said").text_sm().text_color(Token::Text3))
            .child(
                h_flex()
                    .id("panes")
                    .flex_1()
                    .gap(px(12.0))
                    .child(v_flex().id("left").flex_1().h_full().gap(px(10.0)).child(self.list(cx)).child(self.table(cx)))
                    .child(v_flex().id("right").flex_1().h_full().gap(px(10.0)).child(self.tree(cx)).child(self.notes(cx)).child(images())),
            )
            .into_any()
    }

    fn toolbar(&self, cx: &mut Cx<crate::Hello>) -> AnyElement {
        // docs:start gallery-tabs
        let tabs = tabs("tabs", [tab("overview", "Overview").icon(Icon::Grid), tab("files", "Files").icon(Icon::File), tab("log", "Log")])
            .selected(self.tab.clone())
            .on_select(cx.listener(|t: &mut crate::Hello, key: &str, cx| {
                t.gallery.tab = key.to_string();
                cx.notify();
            }));
        // docs:end
        // docs:start gallery-select
        let size = select("size", [choice("s", "Small"), choice("m", "Medium"), choice("l", "Large")])
            .selected(self.size.clone())
            .on_select(cx.listener(|t: &mut crate::Hello, key: &str, cx| {
                t.gallery.size = key.to_string();
                t.gallery.said = format!("Size: {key}");
                cx.notify();
            }));
        // docs:end
        // docs:start gallery-menu-button
        let actions = menu_button("Actions", vec![menu_item("copy", "Copy").icon(Icon::Copy).hint("⌘C"), menu_item("refresh", "Refresh").icon(Icon::Refresh), separator(), menu_item("delete", "Delete").icon(Icon::Trash).danger()])
            .on_select(cx.listener(|t: &mut crate::Hello, key: &str, cx| {
                t.gallery.said = format!("Menu: {key}");
                cx.notify();
            }));
        // docs:end
        h_flex().id("toolbar").gap(px(8.0)).items_center().child(tabs).child(div().flex_1()).child(size).child(actions).into_any()
    }

    fn list(&self, cx: &mut Cx<crate::Hello>) -> AnyElement {
        // docs:start gallery-virtual-list
        let rows = cx.rows(|_: &mut crate::Hello, i, cx| {
            h_flex()
                .id(format!("r{i}"))
                .gap(px(8.0))
                .items_center()
                .child(icon(Icon::File).text_color(Token::Text3))
                .child(text(format!("Row {i}")).flex_1())
                .child(text(format!("{} KB", (i * 37) % 1000)).text_sm().text_color(Token::Text3))
                // Rows hold badges, avatars and relative times too.
                .when(i % 4 == 0, |r| r.child(badge("new").tone(Tone::Info)))
                .context_menu(
                    vec![menu_item("open", "Open"), menu_item("copy", "Copy name"), separator(), menu_item("remove", "Remove").danger()],
                    cx.listener(move |t: &mut crate::Hello, key: &str, cx| {
                        t.gallery.said = format!("Row {i}: {key}");
                        cx.notify();
                    }),
                )
        });
        let list = virtual_list("rows", ROWS, rows)
            .revision(1)
            .selected(self.picked.clone())
            .on_select(cx.listener(|t: &mut crate::Hello, e: &Picked, cx| {
                t.gallery.picked = Some(e.key.clone());
                t.gallery.said = if e.activate { format!("Opened {}", e.key) } else { format!("Selected {}", e.key) };
                cx.notify();
            }));
        // docs:end
        v_flex().id("list-box").flex_1().border_1().border_color(Token::Line).rounded_md().p(px(4.0)).child(list).into_any()
    }

    fn table(&self, cx: &mut Cx<crate::Hello>) -> AnyElement {
        // docs:start gallery-table
        let rows = cx.rows(|t: &mut crate::Hello, i, _| {
            let (name, size, when) = t.gallery.files[i].clone();
            h_flex().id(name.clone()).child(text(name)).child(text(format!("{:.1} KB", size as f64 / 1000.0)).text_color(Token::Text2)).child(text(when).text_color(Token::Text3))
        });
        let (col, dir) = self.sort.clone();
        let table = table("files", vec![column("name", "Name").sortable(), column("size", "Size").width(90.0).align_end().sortable(), column("when", "Modified").width(110.0)], self.files.len(), rows)
            .sort(col, dir)
            .on_sort(cx.listener(|t: &mut crate::Hello, e: &SortChange, cx| {
                t.gallery.sort = (e.column.clone(), e.direction);
                t.gallery.sorted();
                cx.notify();
            }))
            .on_column_resize(cx.listener(|t: &mut crate::Hello, e: &ColumnWidth, cx| {
                t.gallery.said = format!("{} is {} wide", e.column, e.width.round());
                cx.notify();
            }));
        // docs:end
        v_flex().id("table-box").h(px(150.0)).border_1().border_color(Token::Line).rounded_md().child(table).into_any()
    }

    fn tree(&self, cx: &mut Cx<crate::Hello>) -> AnyElement {
        // docs:start gallery-tree
        let shown = self.shown();
        let rows = cx.rows(|t: &mut crate::Hello, i, _| {
            let (path, name, folder, depth) = t.gallery.shown()[i];
            let open = t.gallery.open.iter().any(|o| o == path);
            // A key has no `/` (it joins key paths).
            let row = h_flex().id(path.replace('/', ":")).gap(px(6.0)).items_center().child(icon(if folder { Icon::Folder } else { Icon::File }).text_color(Token::Text3)).child(text(name));
            let row = tree_row(row).depth(depth);
            if folder {
                row.branch(open)
            } else {
                row
            }
        });
        let tree = tree("tree", shown.len(), rows).on_toggle(cx.listener(|t: &mut crate::Hello, e: &Toggled, cx| {
            let path = e.key.replace(':', "/");
            if e.on {
                t.gallery.open.push(path);
            } else {
                t.gallery.open.retain(|o| o != &path);
            }
            cx.notify();
        }));
        // docs:end
        v_flex().id("tree-box").h(px(170.0)).border_1().border_color(Token::Line).rounded_md().p(px(4.0)).child(tree).into_any()
    }

    fn notes(&self, cx: &mut Cx<crate::Hello>) -> AnyElement {
        let box_ = |on: bool| if on { "x" } else { " " };
        let text = format!("## Notes\n\nPitwall draws **Markdown**; see the [docs](https://example.com/docs) or [another note](notes/next.md).\n\n- [{}] Draw the gallery\n- [{}] Ship it\n", box_(self.tasks[0]), box_(self.tasks[1]));
        // docs:start gallery-markdown
        let md = markdown(text)
            .on_link(cx.listener(|t: &mut crate::Hello, e: &LinkClick, cx| {
                t.gallery.said = format!("Link: {}", e.url);
                cx.notify();
            }))
            .on_toggle(cx.listener(|t: &mut crate::Hello, e: &Toggled, cx| {
                // The task's line: 4 is the first.
                if let Some(done) = t.gallery.tasks.get_mut((e.index as usize).saturating_sub(4)) {
                    *done = e.on;
                }
                cx.notify();
            }));
        // docs:end
        v_flex().id("notes").flex_1().border_1().border_color(Token::Line).rounded_md().p(px(8.0)).child(md).into_any()
    }
}

fn images() -> AnyElement {
    // docs:start gallery-image
    let logo = image_asset("assets/logo.svg").contain().w(px(48.0)).h(px(48.0));
    let dot = image_data("dot").contain().w(px(48.0)).h(px(48.0));
    // docs:end
    h_flex().id("images").gap(px(8.0)).items_center().child(logo.id("logo")).child(dot.id("dot")).child(text("From the extension's folder and from bytes").text_sm().text_color(Token::Text3)).into_any()
}

/// An 8×8 teal PNG for `ui.put-image` (a real tile would decode or draw
/// one; the fixture carries its bytes).
pub const DOT_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x08, 0x02, 0x00, 0x00, 0x00, 0x4b, 0x6d, 0x29, 0xdc, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xd0, 0x5e, 0x31, 0x0f, 0x2b, 0x62, 0x18, 0x5a, 0x12, 0x00, 0xef, 0x15, 0x5c, 0x41, 0xab, 0xcd, 0x43, 0x52, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
