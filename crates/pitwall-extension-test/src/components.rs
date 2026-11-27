//! The components (docs/spec/tile-sdk/ui.md §3.3) as a test sees and uses
//! them: a virtual list's, table's or tree's rows (asked of the tile as
//! Pitwall asks, a page at a time), selecting and opening rows, opening
//! and closing tree branches, choosing a menu item, a select's option or a
//! tab, sorting and resizing a table's columns, and a Markdown text's
//! links and task boxes.

use pitwall_tile_host::components as hc;
use pitwall_tile_host::limits::MAX_ROWS_PER_CALL;
use pitwall_tile_host::ui::{self, Event, MenuEntry, Modifiers};

use crate::{node_text, Element, Found, Listen, Node, Session};

/// A row of a virtual list, table or tree, as the tile rendered it.
#[derive(Debug, Clone)]
pub struct ListRow {
    /// Its index in the list.
    pub index: u32,
    /// Its key.
    pub key: String,
    /// Its texts and button labels, in order, joined by a space (a
    /// table's cells too).
    pub text: String,
    /// A tree row's level (0 at the top).
    pub depth: u16,
    /// A tree row with children: open (`Some(true)`) or closed.
    pub expanded: Option<bool>,
    /// Its nodes (the first is its root).
    pub nodes: Vec<Node>,
}

impl ListRow {
    fn new(index: u32, r: ui::ListRow) -> ListRow {
        let text = r.nodes.iter().filter_map(node_text).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        ListRow { index, key: r.key, text, depth: r.depth, expanded: r.expanded, nodes: r.nodes }
    }

    /// The items of its context menu, when it has one.
    pub fn menu(&self) -> Option<Vec<String>> {
        self.nodes.iter().find_map(|n| match &n.element {
            Element::ContextMenu(e) => Some(labels(e)),
            _ => None,
        })
    }
}

fn labels(entries: &[MenuEntry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|e| match e {
            MenuEntry::Item(i) => Some(i.label.clone()),
            MenuEntry::Separator => None,
        })
        .collect()
}

/// The menu item labelled `label` of `entries`: its key and its index
/// among the items.
fn item(entries: &[MenuEntry], label: &str, what: &str) -> (String, u32) {
    let items: Vec<&ui::MenuItem> = entries
        .iter()
        .filter_map(|e| match e {
            MenuEntry::Item(i) => Some(i),
            MenuEntry::Separator => None,
        })
        .collect();
    let Some((ix, it)) = items.iter().enumerate().find(|(_, i)| i.label == label) else {
        panic!("{what} has no item “{label}” (it has {:?})", items.iter().map(|i| &i.label).collect::<Vec<_>>())
    };
    assert!(!it.disabled, "the item “{label}” of {what} is disabled");
    (it.key.clone(), ix as u32)
}

fn handler(n: &Node, on: Listen) -> Option<u32> {
    n.on.iter().find(|l| l.on == on).map(|l| l.handler)
}

impl Session {
    /// The node at `key_path` in the main view, which must be a virtual
    /// list, table or tree.
    fn list(&self, key_path: &str) -> (Found, ui::VirtualList) {
        let Some(f) = self.find_key(key_path) else { panic!("there is no list “{key_path}”; it shows:\n{}", self.text_snapshot()) };
        let Some(l) = hc::rows_of(&f.node).cloned() else { panic!("“{key_path}” is a {}, not a list, table or tree", hc::kind(&f.node.element)) };
        (f, l)
    }

    /// Rows `start..start + count` of the list, table or tree at
    /// `key_path` (fewer at its end), asked of the tile as Pitwall asks
    /// for the rows coming into sight.
    pub fn rows(&mut self, key_path: &str, start: u32, count: u32) -> Vec<ListRow> {
        let (f, l) = self.list(key_path);
        let mut out = Vec::new();
        let end = start.saturating_add(count).min(l.count);
        let mut at = start;
        while at < end {
            let n = (end - at).min(MAX_ROWS_PER_CALL);
            self.check_running();
            self.got_rows.clear();
            assert!(self.handle.rows(&f.view, &f.key_path, at, n), "the tile isn't running");
            self.settle();
            let got = std::mem::take(&mut self.got_rows).into_iter().rfind(|u| u.view == f.view && u.list == f.key_path && u.start == at);
            let Some(got) = got else { panic!("the tile sent no rows {at}..{} of “{key_path}”", at + n) };
            out.extend(got.rows.iter().cloned().enumerate().map(|(k, r)| ListRow::new(at + k as u32, r)));
            at += n;
        }
        out
    }

    /// How many rows the list, table or tree at `key_path` has.
    pub fn row_count(&self, key_path: &str) -> u32 {
        self.list(key_path).1.count
    }

    /// The keys of the selected rows of the list at `key_path`.
    pub fn selected_rows(&self, key_path: &str) -> Vec<String> {
        self.list(key_path).1.selected
    }

    /// The first row of the list at `key_path` whose text contains `text`,
    /// looked for page by page from the top.
    pub fn find_row(&mut self, key_path: &str, text: &str) -> Option<ListRow> {
        let count = self.row_count(key_path);
        let mut at = 0;
        while at < count {
            if let Some(r) = self.rows(key_path, at, MAX_ROWS_PER_CALL).into_iter().find(|r| r.text.contains(text)) {
                return Some(r);
            }
            at += MAX_ROWS_PER_CALL;
        }
        None
    }

    fn row_by_text(&mut self, key_path: &str, text: &str) -> ListRow {
        match self.find_row(key_path, text) {
            Some(r) => r,
            None => panic!("the list “{key_path}” has no row showing “{text}”"),
        }
    }

    fn pick_row(&mut self, key_path: &str, text: &str, activate: bool) -> &mut Self {
        let row = self.row_by_text(key_path, text);
        let (f, _) = self.list(key_path);
        let Some(h) = handler(&f.node, Listen::Select) else { panic!("the list “{key_path}” doesn't listen for select") };
        self.take_focus();
        self.send(Event::Select(ui::Selection { handler: h, key: row.key, index: row.index, modifiers: Modifiers::empty(), activate }));
        self.settle()
    }

    /// Click the first row of the list at `key_path` showing `text`: the
    /// list's `on_select`.
    pub fn select_row(&mut self, key_path: &str, text: &str) -> &mut Self {
        self.pick_row(key_path, text, false)
    }

    /// Double-click (or press Enter on) the first row showing `text`: the
    /// list's `on_select` with `activate`.
    pub fn activate_row(&mut self, key_path: &str, text: &str) -> &mut Self {
        self.pick_row(key_path, text, true)
    }

    fn toggle_row(&mut self, key_path: &str, text: &str, open: bool) -> &mut Self {
        let row = self.row_by_text(key_path, text);
        let (f, _) = self.list(key_path);
        let Some(was) = row.expanded else { panic!("the row “{}” of “{key_path}” has no children to open or close", row.text) };
        if was == open {
            return self;
        }
        let Some(h) = handler(&f.node, Listen::Toggle) else { panic!("the tree “{key_path}” doesn't listen for toggle") };
        self.take_focus();
        self.send(Event::Toggle(ui::Toggled { handler: h, key: row.key, index: row.index, on: open }));
        self.settle()
    }

    /// Open the branch of tree `key_path` showing `text` (its chevron, or
    /// → on it).
    pub fn expand_row(&mut self, key_path: &str, text: &str) -> &mut Self {
        self.toggle_row(key_path, text, true)
    }

    /// Close the branch of tree `key_path` showing `text`.
    pub fn collapse_row(&mut self, key_path: &str, text: &str) -> &mut Self {
        self.toggle_row(key_path, text, false)
    }

    /// Click the text or button showing `what` in the first row of list
    /// `key_path` showing `row` (a button in a row, say).
    pub fn click_in_row(&mut self, key_path: &str, row: &str, what: &str) -> &mut Self {
        let r = self.row_by_text(key_path, row);
        let Some(i) = r.nodes.iter().position(|n| node_text(n).is_some_and(|t| t.contains(what))) else { panic!("the row “{}” shows no “{what}”", r.text) };
        let mut at = Some(i);
        let h = loop {
            let Some(k) = at else { panic!("“{what}” in the row “{}” (and what is around it) doesn't listen for clicks", r.text) };
            if let Some(h) = handler(&r.nodes[k], Listen::Click) {
                break h;
            }
            at = r.nodes[k].parent.map(|p| p as usize);
        };
        self.take_focus();
        self.send(Event::Click(ui::Click { handler: h, button: ui::MouseButton::Left, modifiers: Modifiers::empty(), count: 1, at: ui::Point { x: 1.0, y: 1.0 } }));
        self.settle()
    }

    /// Right-click the first row of list `key_path` showing `row` and
    /// choose `label` in its context menu.
    pub fn choose_row_menu_item(&mut self, key_path: &str, row: &str, label: &str) -> &mut Self {
        let r = self.row_by_text(key_path, row);
        let Some(menu) = r.nodes.iter().find(|n| matches!(n.element, Element::ContextMenu(_))) else { panic!("the row “{}” has no context menu", r.text) };
        let Element::ContextMenu(entries) = &menu.element else { unreachable!() };
        let (key, index) = item(entries, label, &format!("the row “{}”’s menu", r.text));
        let Some(h) = handler(menu, Listen::Select) else { panic!("the row “{}”’s menu doesn't listen for select", r.text) };
        self.take_focus();
        self.send(Event::Select(ui::Selection { handler: h, key, index, modifiers: Modifiers::empty(), activate: false }));
        self.settle()
    }

    /// The menu of the node at `key_path`: a menu button's, else the
    /// context menu put on it. Its entries and the node that listens.
    fn menu_of(&self, key_path: &str) -> (Vec<MenuEntry>, Node) {
        let Some(f) = self.find_key(key_path) else { panic!("there is no node “{key_path}”; it shows:\n{}", self.text_snapshot()) };
        if let Element::MenuButton(m) = &f.node.element {
            assert!(!m.disabled, "the menu button “{key_path}” is disabled");
            return (m.entries.clone(), f.node);
        }
        let frame = self.frame_of(&f.view);
        let menu = frame.nodes.iter().find(|n| n.parent == Some(f.index as u32) && matches!(n.element, Element::ContextMenu(_)));
        match menu {
            Some(n @ Node { element: Element::ContextMenu(e), .. }) => (e.clone(), n.clone()),
            _ => panic!("“{key_path}” is no menu button and has no context menu"),
        }
    }

    /// The labels of the items of the menu at `key_path` (a menu button,
    /// or a node with a context menu).
    pub fn menu_items(&self, key_path: &str) -> Vec<String> {
        labels(&self.menu_of(key_path).0)
    }

    /// Open the menu at `key_path` (click a menu button, right-click a
    /// node with a context menu) and choose `label`.
    pub fn choose_menu_item(&mut self, key_path: &str, label: &str) -> &mut Self {
        let (entries, node) = self.menu_of(key_path);
        let (key, index) = item(&entries, label, &format!("the menu of “{key_path}”"));
        let Some(h) = handler(&node, Listen::Select) else { panic!("the menu of “{key_path}” doesn't listen for select") };
        self.take_focus();
        self.send(Event::Select(ui::Selection { handler: h, key, index, modifiers: Modifiers::empty(), activate: false }));
        self.settle()
    }

    /// Choose the option labelled `label` of the select at `key_path`.
    pub fn select_option(&mut self, key_path: &str, label: &str) -> &mut Self {
        let Some(f) = self.find_key(key_path) else { panic!("there is no select “{key_path}”") };
        let Element::Select(s) = &f.node.element else { panic!("“{key_path}” isn't a select") };
        assert!(!s.disabled, "the select “{key_path}” is disabled");
        let Some((ix, o)) = s.options.iter().enumerate().find(|(_, o)| o.label == label) else { panic!("the select “{key_path}” has no option “{label}” (it has {:?})", s.options.iter().map(|o| &o.label).collect::<Vec<_>>()) };
        let Some(h) = handler(&f.node, Listen::Select) else { panic!("the select “{key_path}” doesn't listen for select") };
        let key = o.key.clone();
        self.take_focus();
        self.send(Event::Select(ui::Selection { handler: h, key, index: ix as u32, modifiers: Modifiers::empty(), activate: false }));
        self.settle()
    }

    /// Click the tab labelled `label` of the tabs at `key_path`.
    pub fn select_tab(&mut self, key_path: &str, label: &str) -> &mut Self {
        let Some(f) = self.find_key(key_path) else { panic!("there are no tabs “{key_path}”") };
        let Element::Tabs(t) = &f.node.element else { panic!("“{key_path}” isn't tabs") };
        let Some((ix, tab)) = t.items.iter().enumerate().find(|(_, t)| t.label == label) else { panic!("the tabs “{key_path}” have no “{label}”") };
        let Some(h) = handler(&f.node, Listen::Select) else { panic!("the tabs “{key_path}” don't listen for select") };
        let key = tab.key.clone();
        self.take_focus();
        self.send(Event::Select(ui::Selection { handler: h, key, index: ix as u32, modifiers: Modifiers::empty(), activate: false }));
        self.settle()
    }

    /// Click the column titled `title` of the table at `key_path`: sorted
    /// by it ascending, or descending when it is sorted ascending now.
    pub fn sort_by(&mut self, key_path: &str, title: &str) -> &mut Self {
        let Some(f) = self.find_key(key_path) else { panic!("there is no table “{key_path}”") };
        let Element::Table(t) = &f.node.element else { panic!("“{key_path}” isn't a table") };
        let Some(c) = t.columns.iter().find(|c| c.title == title) else { panic!("the table “{key_path}” has no column “{title}”") };
        assert!(c.sortable, "the column “{title}” isn't sortable");
        let direction = match &t.sort {
            Some(s) if s.column == c.key && s.direction == ui::SortDirection::Ascending => ui::SortDirection::Descending,
            _ => ui::SortDirection::Ascending,
        };
        let Some(h) = handler(&f.node, Listen::Sort) else { panic!("the table “{key_path}” doesn't listen for sort") };
        let column = c.key.clone();
        self.take_focus();
        self.send(Event::Sort(ui::SortChange { handler: h, column, direction }));
        self.settle()
    }

    /// Drag the right edge of column `title` of the table at `key_path`
    /// until it is `width` wide.
    pub fn resize_column(&mut self, key_path: &str, title: &str, width: f32) -> &mut Self {
        let Some(f) = self.find_key(key_path) else { panic!("there is no table “{key_path}”") };
        let Element::Table(t) = &f.node.element else { panic!("“{key_path}” isn't a table") };
        let Some(c) = t.columns.iter().find(|c| c.title == title) else { panic!("the table “{key_path}” has no column “{title}”") };
        let Some(h) = handler(&f.node, Listen::ColumnResized) else { panic!("the table “{key_path}” doesn't listen for column resizes") };
        let column = c.key.clone();
        self.send(Event::ColumnResized(ui::ColumnWidth { handler: h, column, width: width.max(40.) }));
        self.settle()
    }

    /// Click the link to `url` in the Markdown at `key_path`.
    pub fn click_link(&mut self, key_path: &str, url: &str) -> &mut Self {
        let Some(f) = self.find_key(key_path) else { panic!("there is no Markdown “{key_path}”") };
        let Element::Markdown(m) = &f.node.element else { panic!("“{key_path}” isn't Markdown") };
        assert!(m.text.contains(&format!("]({url}")) || m.text.contains(&format!("<{url}>")) || m.text.contains(url), "the Markdown “{key_path}” has no link to “{url}”");
        let Some(h) = handler(&f.node, Listen::Link) else { panic!("the Markdown “{key_path}” doesn't listen for links") };
        self.take_focus();
        self.send(Event::Link(ui::LinkClick { handler: h, url: url.to_string(), modifiers: Modifiers::empty() }));
        self.settle()
    }

    /// Click the box of the task on line `line` (0-based, of its text) in
    /// the Markdown at `key_path`.
    pub fn toggle_task(&mut self, key_path: &str, line: usize) -> &mut Self {
        let Some(f) = self.find_key(key_path) else { panic!("there is no Markdown “{key_path}”") };
        let Element::Markdown(m) = &f.node.element else { panic!("“{key_path}” isn't Markdown") };
        let Some(l) = m.text.lines().nth(line) else { panic!("the Markdown “{key_path}” has no line {line}") };
        let t = l.trim_start().trim_start_matches(['-', '*', '+']).trim_start();
        let on = if t.starts_with("[ ]") {
            true
        } else if t.starts_with("[x]") || t.starts_with("[X]") {
            false
        } else {
            panic!("line {line} of the Markdown “{key_path}” isn't a task: {l:?}")
        };
        let Some(h) = handler(&f.node, Listen::Toggle) else { panic!("the Markdown “{key_path}” doesn't listen for task boxes") };
        self.take_focus();
        self.send(Event::Toggle(ui::Toggled { handler: h, key: String::new(), index: line as u32, on }));
        self.settle()
    }
}

/// How a component node reads in a text snapshot.
pub(crate) fn describe(e: &Element) -> Option<String> {
    Some(match e {
        Element::VirtualList(l) => format!("list rows={}{}", l.count, sel(&l.selected)),
        Element::Table(t) => format!(
            "table rows={} columns=[{}]{}{}",
            t.rows.count,
            t.columns.iter().map(|c| c.title.clone()).collect::<Vec<_>>().join(", "),
            t.sort.as_ref().map(|s| format!(" sort={}{}", s.column, if s.direction == ui::SortDirection::Ascending { "↑" } else { "↓" })).unwrap_or_default(),
            sel(&t.rows.selected)
        ),
        Element::Tree(t) => format!("tree rows={}{}", t.rows.count, sel(&t.rows.selected)),
        Element::Tabs(t) => format!(
            "tabs [{}]",
            t.items.iter().map(|i| if t.selected.as_deref() == Some(i.key.as_str()) { format!("*{}", i.label) } else { i.label.clone() }).collect::<Vec<_>>().join(", ")
        ),
        Element::Select(s) => {
            let chosen = s.selected.as_ref().and_then(|k| s.options.iter().find(|o| &o.key == k)).map(|o| o.label.clone());
            format!("select {:?}{}", chosen.or_else(|| s.placeholder.clone()).unwrap_or_default(), if s.disabled { " disabled" } else { "" })
        }
        Element::MenuButton(m) => format!("menu-button {:?} [{}]{}", m.label, labels(&m.entries).join(", "), if m.disabled { " disabled" } else { "" }),
        Element::ContextMenu(e) => format!("context-menu [{}]", labels(e).join(", ")),
        Element::Markdown(m) => format!("markdown {:?}", m.text.lines().next().unwrap_or_default()),
        Element::Image(i) => match &i.source {
            ui::ImageSource::Asset(p) => format!("image asset={p}"),
            ui::ImageSource::Data(n) => format!("image data={n}"),
        },
        _ => return None,
    })
}

fn sel(keys: &[String]) -> String {
    if keys.is_empty() {
        String::new()
    } else {
        format!(" selected=[{}]", keys.join(", "))
    }
}

/// The icons a component node names.
pub(crate) fn icons(e: &Element) -> Vec<&str> {
    fn named(i: &Option<ui::Icon>) -> Option<&str> {
        i.as_ref().map(|ui::Icon::Named(n)| n.as_str())
    }
    fn menu(es: &[MenuEntry]) -> Vec<&str> {
        es.iter()
            .filter_map(|e| match e {
                MenuEntry::Item(i) => named(&i.icon),
                MenuEntry::Separator => None,
            })
            .collect()
    }
    match e {
        Element::Tabs(t) => t.items.iter().filter_map(|t| named(&t.icon)).collect(),
        Element::MenuButton(m) => {
            let mut v = menu(&m.entries);
            v.extend(named(&m.icon));
            v
        }
        Element::ContextMenu(es) => menu(es),
        _ => vec![],
    }
}
