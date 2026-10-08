//! Small pieces Review draws, after the React app's shared CSS
//! (`src/styles/base.css`, `panel.css`, `agents.css`): stroke icons,
//! Material file icons, buttons, chips, the diffstat, status letters and
//! glyphs, the keyboard hint, checkbox and segmented control. They live
//! here until the shared kit (phase 1) has them: promote them there.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{
    div, img, prelude::*, px, Div, FontWeight, Hsla, Image, ImageFormat, Img, SharedString,
    Stateful,
};
use serde::Deserialize;

use pitwall_proto::{FileStatus, Status};

use crate::code_view::style::{label_family, mono_family, ui_family};
use crate::theme::{Theme, RADIUS, RADIUS_SM};

// ── stroke icons (`src/components/Icon.tsx`) ───────────────────────────────

const PATHS: &[(&str, &str)] = &[
    ("refresh", "M15.5 10a5.5 5.5 0 1 1-1.7-4M16 3.5v3h-3"),
    (
        "branch",
        "M6 4v12M6 9c0-2 1.5-3 4-3h1M13 4a2 2 0 1 1 0 4 2 2 0 0 1 0-4zM6 16",
    ),
    ("chevron", "M8 5l5 5-5 5"),
    ("chevron-down", "M5 8l5 5 5-5"),
    ("x", "M5 5l10 10M15 5 5 15"),
    ("terminal", "M3 4.5h14v11H3zM6 8l2.5 2L6 12M10.5 12.5H14"),
    ("trash", "M4.5 6h11M8 6V4.5h4V6M6 6l.7 10h6.6L14 6"),
    ("review", "M4 3.5h8l4 4v9H4zM12 3.5v4h4M7 11h6M7 14h4"),
];

fn hex(c: Hsla) -> String {
    let rgba = c.to_rgb();
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgba.r * 255.).round() as u8,
        (rgba.g * 255.).round() as u8,
        (rgba.b * 255.).round() as u8
    )
}

fn cached(key: String, make: impl FnOnce() -> Vec<u8>, format: ImageFormat) -> Arc<Image> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Image>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry(key)
        .or_insert_with(|| Arc::new(Image::from_bytes(format, make())))
        .clone()
}

/// A stroke icon at `size` px in `color` (`<Icon name size />`).
pub fn icon(name: &str, size: f32, color: Hsla) -> Img {
    let d = PATHS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or("", |(_, d)| d);
    let (stroke, alpha) = (hex(color), color.a);
    let image = cached(
        format!("{name}:{stroke}:{alpha:.3}"),
        || {
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="{stroke}" stroke-opacity="{alpha}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="{d}"/></svg>"##
            )
            .into_bytes()
        },
        ImageFormat::Svg,
    );
    img(image).size(px(size)).flex_none()
}

/// An icon that takes `hover` colour while its `group` is hovered.
pub fn icon_hover(name: &str, size: f32, color: Hsla, hover: Hsla, group: &'static str) -> Div {
    div()
        .relative()
        .size(px(size))
        .flex_none()
        .child(
            div()
                .absolute()
                .inset_0()
                .group_hover(group, |s| s.invisible())
                .child(icon(name, size, color)),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .invisible()
                .group_hover(group, |s| s.visible())
                .child(icon(name, size, hover)),
        )
}

// ── file icons (`src/lib/fileIcons.ts`) ────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IconMap {
    file: String,
    folder: [String; 2],
    file_names: HashMap<String, String>,
    extensions: HashMap<String, String>,
    folders: HashMap<String, [String; 2]>,
}

fn icon_map() -> &'static IconMap {
    static MAP: OnceLock<IconMap> = OnceLock::new();
    MAP.get_or_init(|| {
        serde_json::from_str(include_str!("../../../../src/lib/fileIcons.map.json"))
            .expect("fileIcons.map.json")
    })
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// File name first, then the longest known extension, then the generic one.
pub fn file_icon_name(path: &str) -> &'static str {
    let m = icon_map();
    let name = base_name(path).to_lowercase();
    if let Some(n) = m.file_names.get(&name) {
        return n;
    }
    for (i, _) in name.match_indices('.') {
        let ext = &name[i + 1..];
        if let Some(n) = m.extensions.get(ext).filter(|_| !ext.is_empty()) {
            return n;
        }
    }
    &m.file
}

pub fn folder_icon_name(path: &str, open: bool) -> &'static str {
    let m = icon_map();
    let pair = m
        .folders
        .get(&base_name(path).to_lowercase())
        .unwrap_or(&m.folder);
    &pair[usize::from(open)]
}

fn material(name: &str, size: f32) -> Img {
    let svgs = super::file_icons::SVGS;
    let find = |n: &str| svgs.iter().find(|(k, _)| *k == n).map(|(_, b)| *b);
    let bytes = find(name)
        .or_else(|| find(&icon_map().file))
        .unwrap_or_default();
    let image = cached(
        format!("file-icon:{name}"),
        || bytes.to_vec(),
        ImageFormat::Svg,
    );
    img(image).size(px(size)).flex_none()
}

pub fn file_icon(path: &str) -> Img {
    material(file_icon_name(path), 16.)
}

pub fn folder_icon(path: &str, open: bool) -> Img {
    material(folder_icon_name(path, open), 16.)
}

// ── text pieces ────────────────────────────────────────────────────────────

/// Barlow Condensed, upper case (`.label`, `.chip`, …). GPUI has no
/// letter-spacing, so the text is spaced by the font's own width only.
pub fn label_text(text: &str, size: f32, weight: FontWeight, color: Hsla) -> Div {
    div()
        .font_family(label_family())
        .font_weight(weight)
        .text_size(px(size))
        .text_color(color)
        .whitespace_nowrap()
        .child(text.to_uppercase())
}

/// `.label`: 12px, 600, `--text-3`.
pub fn label(text: &str, t: &Theme) -> Div {
    label_text(text, 12., FontWeight::SEMIBOLD, t.text_3)
}

pub fn mono() -> Div {
    div().font_family(mono_family())
}

pub fn ui() -> Div {
    div().font_family(ui_family())
}

/// `.chip` (and `.chip-warn` / `.chip-subtle`).
pub fn chip(text: &str, kind: ChipKind, t: &Theme) -> Div {
    let (fg, bg, border) = match kind {
        ChipKind::Plain => (t.text_3, gpui::transparent_black(), t.line_strong),
        ChipKind::Warn => (t.amber, t.amber_soft, gpui::transparent_black()),
        ChipKind::Subtle => (t.text_3, t.surface_3, gpui::transparent_black()),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(px(5.))
        .pt(px(3.))
        .pb(px(2.))
        .rounded(px(3.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .child(label_text(text, 10.5, FontWeight::SEMIBOLD, fg).line_height(px(10.5)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    Plain,
    Warn,
    Subtle,
}

/// `.diffstat`: +a −r (or ±0).
pub fn diffstat(added: u32, removed: u32, t: &Theme) -> Div {
    let d = mono().flex().flex_none().gap(px(6.)).text_size(px(11.));
    if added == 0 && removed == 0 {
        return d.text_color(t.text_4).child("±0");
    }
    d.child(div().text_color(t.green).child(format!("+{added}")))
        .child(div().text_color(t.red).child(format!("−{removed}")))
}

/// The diffstat inside a file row: only the non-zero sides.
pub fn diffstat_inline(added: u32, removed: u32, t: &Theme) -> Div {
    mono()
        .flex()
        .flex_none()
        .gap(px(6.))
        .text_size(px(11.))
        .when(added > 0, |d| {
            d.child(div().text_color(t.green).child(format!("+{added}")))
        })
        .when(removed > 0, |d| {
            d.child(div().text_color(t.red).child(format!("−{removed}")))
        })
}

/// `.status-letter`: A/U green, D red, others `--text-2`.
pub fn status_letter(s: FileStatus, t: &Theme) -> Div {
    let color = match s {
        FileStatus::A | FileStatus::U => t.green,
        FileStatus::D => t.red,
        _ => t.text_2,
    };
    mono()
        .w(px(10.))
        .flex_none()
        .text_center()
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .line_height(px(11.))
        .text_color(color)
        .child(super::model::status_letter(s))
}

/// `<StatusGlyph size="sm">`: 12px box, working is a dot.
pub fn glyph_sm(s: Status, t: &Theme) -> Div {
    let color = t.status_color(s);
    let base = div()
        .size(px(12.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    match s {
        Status::Working => base.child(div().size(px(6.)).rounded_full().bg(t.green)),
        _ => {
            let size = match s {
                Status::Done => 9. * 1.15,
                Status::Idle => 9. * 0.8,
                Status::Exited | Status::Stopped => 9. * 0.75,
                _ => 9.,
            };
            base.text_size(px(size))
                .line_height(px(size))
                .text_color(color)
                .child(crate::agents::status_glyph(s))
        }
    }
}

/// `.kbd`.
pub fn kbd(text: &str, t: &Theme) -> Div {
    mono()
        .flex_none()
        .text_size(px(10.5))
        .line_height(px(10.5))
        .px(px(5.))
        .py(px(3.))
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(t.line_strong)
        .text_color(t.text_3)
        .bg(t.surface_2)
        .whitespace_nowrap()
        .child(text.to_string())
}

/// A shortcut as this desktop types it ("⌘↵" → "Ctrl+↵" off macOS).
pub fn keys(mac: &str) -> String {
    if cfg!(target_os = "macos") {
        return mac.to_string();
    }
    mac.replace("⌘⇧", "Ctrl+Shift+Alt+")
        .replace('⌘', "Ctrl+")
        .replace('⇧', "Shift+")
}

// ── buttons (`.ghost-btn`, `.primary-btn`, `.danger-btn`, `.small-btn`) ───

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Ghost,
    Primary,
    Danger,
    Small,
}

pub fn button(id: impl Into<SharedString>, kind: Btn, enabled: bool, t: &Theme) -> Stateful<Div> {
    let id: SharedString = id.into();
    let d = ui()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.))
        .rounded(RADIUS)
        .whitespace_nowrap()
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(12.5));
    let d = match kind {
        Btn::Small => d
            .h(px(26.))
            .px(px(9.))
            .text_size(px(12.))
            .text_color(t.text)
            .bg(t.surface_3)
            .border_1()
            .border_color(t.line_strong)
            .when(enabled, |d| d.hover(|s| s.border_color(t.text_4))),
        Btn::Ghost => d
            .h(px(30.))
            .px(px(12.))
            .text_color(t.text_2)
            .border_1()
            .border_color(t.line_strong)
            .when(enabled, |d| {
                d.hover(|s| s.text_color(t.text).bg(t.surface_3))
            }),
        Btn::Primary => {
            let hover = mix(t.text, t.bg, 0.88);
            d.h(px(30.))
                .px(px(12.))
                .bg(t.text)
                .text_color(t.bg)
                .when(enabled, |d| d.hover(|s| s.bg(hover)))
        }
        Btn::Danger => d.h(px(30.)).px(px(12.)).bg(t.red).text_color(gpui::white()),
    };
    if enabled {
        d.cursor_pointer()
    } else {
        d.opacity(0.45)
    }
}

/// `color-mix(in srgb, a p%, b)`.
pub fn mix(a: Hsla, b: Hsla, p: f32) -> Hsla {
    let (x, y) = (a.to_rgb(), b.to_rgb());
    gpui::Rgba {
        r: x.r * p + y.r * (1. - p),
        g: x.g * p + y.g * (1. - p),
        b: x.b * p + y.b * (1. - p),
        a: x.a * p + y.a * (1. - p),
    }
    .into()
}

/// `.icon-btn` (28px; `sm`: 22px) holding an icon or a character.
pub fn icon_button(id: impl Into<SharedString>, sm: bool, t: &Theme) -> Stateful<Div> {
    let id: SharedString = id.into();
    let size = if sm { 22. } else { 28. };
    div()
        .id(id)
        .size(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(RADIUS)
        .text_color(t.text_3)
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|s| s.text_color(t.text).bg(t.surface_3))
}

/// A 12px checkbox drawn like the native one with `accent-color`.
pub fn checkbox(checked: bool, accent: Hsla, t: &Theme) -> Div {
    div()
        .size(px(12.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .border_1()
        .border_color(if checked { accent } else { t.text_4 })
        .when(checked, |d| d.bg(accent))
        .when(checked, |d| {
            d.child(
                div()
                    .text_size(px(9.))
                    .line_height(px(9.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.bg)
                    .child("✓"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_icons_resolve_like_the_react_app() {
        assert_eq!(file_icon_name("package.json"), "nodejs");
        assert_eq!(file_icon_name("src/main.rs"), "rust");
        assert_eq!(file_icon_name("a/b/unknown.zzz"), "file");
        assert_eq!(folder_icon_name("src", false), "folder-src");
        assert_eq!(folder_icon_name("nothing-special", true), "folder-open");
        for n in [file_icon_name("x.ts"), folder_icon_name("src", true)] {
            assert!(
                super::super::file_icons::SVGS.iter().any(|(k, _)| *k == n),
                "{n} is bundled"
            );
        }
    }

    #[test]
    fn shortcuts_read_per_desktop() {
        if cfg!(target_os = "macos") {
            assert_eq!(keys("⌘⇧R"), "⌘⇧R");
        } else {
            assert_eq!(keys("⌘⇧R"), "Ctrl+Shift+Alt+R");
            assert_eq!(keys("⌘↵"), "Ctrl+↵");
        }
    }
}
