//! The host's side of the components (ui.md §3.3): virtual lists, tables
//! and trees (their rows fetched with `guest.rows`), tabs, selects, menus,
//! Markdown and images. Their limits are checked here, on the worker, so
//! the app draws only what is well formed.

use std::collections::HashSet;

use crate::frame::Invalid;
use crate::limits::*;
use crate::ui::{Element, Frame, ImageSource, ListRow as Row, MenuEntry, Node, VirtualList};

/// What is wrong with component node `n` (node `i` of its frame), if
/// anything.
pub(crate) fn check(i: usize, n: &Node) -> Result<(), String> {
    match &n.element {
        Element::VirtualList(l) => list(l),
        Element::Tree(t) => list(&t.rows),
        Element::Table(t) => {
            if t.columns.is_empty() || t.columns.len() > MAX_COLUMNS {
                return Err(format!("a table has 1 to {MAX_COLUMNS} columns, not {}", t.columns.len()));
            }
            let mut keys = HashSet::new();
            for c in &t.columns {
                if !keys.insert(c.key.as_str()) {
                    return Err(format!("two columns have the key “{}”", cut(&c.key)));
                }
                if c.width.is_some_and(|w| !w.is_finite() || w < 0.) {
                    return Err(format!("column “{}” has a width that isn't a number of pixels", cut(&c.key)));
                }
            }
            list(&t.rows)
        }
        Element::Tabs(t) => items(t.items.len(), t.items.iter().map(|t| t.key.as_str())),
        Element::Select(s) => items(s.options.len(), s.options.iter().map(|o| o.key.as_str())),
        Element::MenuButton(m) => menu(&m.entries),
        Element::ContextMenu(_) if i == 0 => Err("a context menu belongs to the node it is under, so it can't be the root".into()),
        Element::ContextMenu(entries) => menu(entries),
        Element::Markdown(m) if m.text.len() > MAX_MARKDOWN_BYTES => Err(format!("its Markdown is over {} KiB", MAX_MARKDOWN_BYTES >> 10)),
        Element::Image(img) => match &img.source {
            ImageSource::Asset(path) => asset_path(path).map(|_| ()),
            ImageSource::Data(name) if name.is_empty() || name.len() > MAX_KEY_BYTES => Err("an image's name is 1 to 128 bytes".into()),
            ImageSource::Data(_) => Ok(()),
        },
        _ => Ok(()),
    }
}

fn list(l: &VirtualList) -> Result<(), String> {
    if l.count > MAX_LIST_ROWS {
        return Err(format!("a list has at most {MAX_LIST_ROWS} rows, not {}", l.count));
    }
    if !(MIN_ROW_HEIGHT..=MAX_ROW_HEIGHT).contains(&l.row_height) {
        return Err(format!("a row is {MIN_ROW_HEIGHT}–{MAX_ROW_HEIGHT} px tall, not {}", l.row_height));
    }
    if l.selected.len() > MAX_SELECTED {
        return Err(format!("at most {MAX_SELECTED} rows are selected at once"));
    }
    Ok(())
}

fn items<'a>(n: usize, keys: impl Iterator<Item = &'a str>) -> Result<(), String> {
    if n > MAX_ITEMS {
        return Err(format!("at most {MAX_ITEMS} items, not {n}"));
    }
    let mut seen = HashSet::new();
    for k in keys {
        if !seen.insert(k) {
            return Err(format!("two items have the key “{}”", cut(k)));
        }
    }
    Ok(())
}

fn menu(entries: &[MenuEntry]) -> Result<(), String> {
    items(
        entries.len(),
        entries.iter().filter_map(|e| match e {
            MenuEntry::Item(i) => Some(i.key.as_str()),
            MenuEntry::Separator => None,
        }),
    )
}

/// An image asset's path in the extension's folder, checked: relative,
/// with no `..`, no backslashes and no drive.
pub fn asset_path(path: &str) -> Result<&str, String> {
    let bad = path.is_empty()
        || path.len() > 512
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|p| p == ".." || p.is_empty());
    if bad {
        return Err(format!("the image asset “{}” must be a path inside the extension's folder (like assets/logo.png)", cut(path)));
    }
    Ok(path)
}

/// About how many bytes a component element takes through the canonical
/// ABI (for the frame's size limit).
pub(crate) fn bytes(e: &Element) -> usize {
    let list = |l: &VirtualList| 32 + l.selected.iter().map(|s| s.len() + 8).sum::<usize>();
    let icon = |i: &Option<crate::ui::Icon>| i.as_ref().map_or(0, |crate::ui::Icon::Named(n)| n.len() + 8);
    let entries = |es: &[MenuEntry]| {
        es.iter()
            .map(|e| match e {
                MenuEntry::Item(i) => 24 + i.key.len() + i.label.len() + icon(&i.icon) + i.hint.as_ref().map_or(0, String::len),
                MenuEntry::Separator => 4,
            })
            .sum::<usize>()
    };
    match e {
        Element::VirtualList(l) => list(l),
        Element::Tree(t) => list(&t.rows),
        Element::Table(t) => list(&t.rows) + t.columns.iter().map(|c| 24 + c.key.len() + c.title.len()).sum::<usize>(),
        Element::Tabs(t) => t.items.iter().map(|t| 16 + t.key.len() + t.label.len() + icon(&t.icon)).sum(),
        Element::Select(s) => s.options.iter().map(|o| 16 + o.key.len() + o.label.len()).sum::<usize>() + s.placeholder.as_ref().map_or(0, String::len),
        Element::MenuButton(m) => m.label.len() + icon(&m.icon) + entries(&m.entries),
        Element::ContextMenu(es) => entries(es),
        Element::Markdown(m) => m.text.len(),
        Element::Image(i) => match &i.source {
            ImageSource::Asset(s) | ImageSource::Data(s) => 8 + s.len(),
        },
        _ => 0,
    }
}

/// Whether a row may hold node `n`: boxes, text, icons, buttons, images,
/// context menus, badges, avatars and relative times.
fn allowed_in_row(e: &Element) -> bool {
    matches!(
        e,
        Element::Box | Element::Text(_) | Element::Icon(_) | Element::Button(_) | Element::Image(_) | Element::ContextMenu(_) | Element::Badge(_) | Element::Avatar(_) | Element::RelativeTime(_)
    )
}

/// Check the rows `guest.rows` returned: each a well-formed small tree of
/// the nodes a row may hold, keys unique.
pub fn check_rows(rows: &[Row]) -> Result<(), Invalid> {
    let mut keys = HashSet::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let bad = |why: String| Invalid::Component(r, format!("row {r} (“{}”): {why}", cut(&row.key)));
        if row.key.len() > MAX_KEY_BYTES {
            return Err(bad(format!("its key is longer than {MAX_KEY_BYTES} bytes")));
        }
        if !keys.insert(row.key.as_str()) {
            return Err(bad("another row has the same key".into()));
        }
        if row.nodes.is_empty() || row.nodes.len() > MAX_ROW_NODES {
            return Err(bad(format!("a row has 1 to {MAX_ROW_NODES} nodes, not {}", row.nodes.len())));
        }
        let f = Frame { nodes: row.nodes.clone(), focus: None, keymap: vec![] };
        crate::frame::check(&f).map_err(|e| bad(e.to_string()))?;
        if let Some(n) = row.nodes.iter().find(|n| !allowed_in_row(&n.element)) {
            return Err(bad(format!("a row holds boxes, text, icons, buttons, images, context menus, badges, avatars and relative times only, not “{}”", kind(&n.element))));
        }
    }
    Ok(())
}

/// An element's kind, in words, for messages.
pub fn kind(e: &Element) -> &'static str {
    match e {
        Element::Box => "box",
        Element::Text(_) => "text",
        Element::Button(_) => "button",
        Element::Icon(_) => "icon",
        Element::Scroll(_) => "scroll",
        Element::Input(_) => "input",
        Element::VirtualList(_) => "virtual-list",
        Element::Table(_) => "table",
        Element::Tree(_) => "tree",
        Element::Tabs(_) => "tabs",
        Element::Select(_) => "select",
        Element::MenuButton(_) => "menu-button",
        Element::ContextMenu(_) => "context-menu",
        Element::Markdown(_) => "markdown",
        Element::Image(_) => "image",
        #[allow(unreachable_patterns)]
        _ => "element",
    }
}

/// The rows of the list, table or tree node `n`, if it is one.
pub fn rows_of(n: &Node) -> Option<&VirtualList> {
    match &n.element {
        Element::VirtualList(l) => Some(l),
        Element::Table(t) => Some(&t.rows),
        Element::Tree(t) => Some(&t.rows),
        _ => None,
    }
}

/// The key path of each node of `nodes` (keys from the root down, joined
/// by `/`, without the root's).
pub fn key_paths(nodes: &[Node]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let path = match n.parent {
            None => String::new(),
            Some(p) => match out.get(p as usize) {
                Some(pp) if !pp.is_empty() => format!("{pp}/{}", n.key),
                _ => n.key.clone(),
            },
        };
        out.push(path);
    }
    out
}

/// The list, table or tree at key path `path` in `frame`.
pub fn list_at<'a>(frame: &'a Frame, path: &str) -> Option<&'a VirtualList> {
    let paths = key_paths(&frame.nodes);
    let i = paths.iter().position(|p| p == path)?;
    rows_of(&frame.nodes[i])
}

/// An image's format, from its first bytes: PNG, JPEG, GIF, WebP or SVG.
pub fn image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(ImageFormat::Png);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(ImageFormat::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(ImageFormat::Gif);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(ImageFormat::Webp);
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("<svg") || (head.starts_with("<?xml") || head.starts_with("<!--") || head.starts_with("<!doctype svg")) && head.contains("<svg") {
        return Some(ImageFormat::Svg);
    }
    None
}

/// The image formats tiles may show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
}

fn cut(s: &str) -> String {
    crate::host::cut(s, 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Column, Image, ImageFit, MenuItem, Table};

    fn node(parent: Option<u32>, key: &str, element: Element) -> Node {
        Node { parent, key: key.into(), element, style: vec![], on: vec![], focusable: false, tooltip: None, label: None }
    }

    fn vl(count: u32) -> VirtualList {
        VirtualList { count, row_height: 28., revision: 1, selected: vec![] }
    }

    #[test]
    fn components_are_checked() {
        assert!(check(1, &node(Some(0), "l", Element::VirtualList(vl(100_000)))).is_ok());
        assert!(check(1, &node(Some(0), "l", Element::VirtualList(vl(MAX_LIST_ROWS + 1)))).is_err());
        assert!(check(1, &node(Some(0), "l", Element::VirtualList(VirtualList { row_height: 4., ..vl(1) }))).is_err());
        let col = |k: &str| Column { key: k.into(), title: k.into(), width: None, align: crate::ui::Align::Start, sortable: true };
        let t = Table { columns: vec![col("a"), col("a")], sort: None, rows: vl(3) };
        assert!(check(1, &node(Some(0), "t", Element::Table(t))).unwrap_err().contains("two columns"));
        let item = |k: &str| MenuEntry::Item(MenuItem { key: k.into(), label: k.into(), icon: None, hint: None, disabled: false, danger: false });
        assert!(check(0, &node(None, "m", Element::ContextMenu(vec![item("a")]))).is_err());
        assert!(check(1, &node(Some(0), "m", Element::ContextMenu(vec![item("a"), MenuEntry::Separator, item("b")]))).is_ok());
        let img = |p: &str| Element::Image(Image { source: ImageSource::Asset(p.into()), fit: ImageFit::Contain });
        assert!(check(1, &node(Some(0), "i", img("assets/logo.png"))).is_ok());
        for bad in ["../x.png", "/etc/x.png", "a\\b.png", "C:x.png", ""] {
            assert!(check(1, &node(Some(0), "i", img(bad))).is_err(), "{bad}");
        }
    }

    #[test]
    fn rows_hold_only_what_a_row_may() {
        let ok = Row { key: "a".into(), nodes: vec![node(None, "r", Element::Box), node(Some(0), "t", Element::Text("x".into()))], depth: 0, expanded: None };
        assert!(check_rows(std::slice::from_ref(&ok)).is_ok());
        let twice = [ok.clone(), ok.clone()];
        assert!(check_rows(&twice).is_err());
        let field = Row { key: "b".into(), nodes: vec![node(None, "r", Element::Box), node(Some(0), "f", Element::Scroll(crate::ui::Scroll { axis: crate::ui::Axis::Vertical }))], depth: 0, expanded: None };
        assert!(check_rows(&[field]).unwrap_err().to_string().contains("not “scroll”"));
        let empty = Row { key: "c".into(), nodes: vec![], depth: 0, expanded: None };
        assert!(check_rows(&[empty]).is_err());
        // Badges, avatars and relative times too.
        let rich = Row {
            key: "d".into(),
            nodes: vec![
                node(None, "r", Element::Box),
                node(Some(0), "b", Element::Badge(crate::bindings::pitwall::tile::types::Badge { text: "3".into(), tone: None })),
                node(Some(0), "a", Element::Avatar(crate::ui::Avatar { name: "Ada Lovelace".into(), size: crate::ui::AvatarSize::Small })),
                node(Some(0), "t", Element::RelativeTime(crate::ui::RelativeTime { at: 0 })),
            ],
            depth: 0,
            expanded: None,
        };
        assert!(check_rows(&[rich]).is_ok());
    }

    #[test]
    fn image_formats_are_sniffed() {
        assert_eq!(image_format(b"\x89PNG\r\n\x1a\n...."), Some(ImageFormat::Png));
        assert_eq!(image_format(&[0xff, 0xd8, 0xff, 0xe0]), Some(ImageFormat::Jpeg));
        assert_eq!(image_format(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(image_format(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"\"/>"), Some(ImageFormat::Svg));
        assert_eq!(image_format(b"<html>"), None);
    }
}
