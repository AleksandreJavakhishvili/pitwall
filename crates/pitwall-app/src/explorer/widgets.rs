//! Small pieces the explorer draws with, matching the React app's CSS
//! (`base.css`, `panel.css`, `review.css`): stroke icons, kbd chip, small
//! and ghost buttons, icon button, checkbox, change letter, refresh control,
//! labels, tooltip. Local until the shared kit (phase 1) has them;
//! candidates to promote.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{
    div, img, prelude::*, px, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla, Image,
    ImageFormat, Img, Rgba, SharedString, Stateful, Window,
};

use pitwall_proto::FileStatus;

use super::logic::{ago, status_letter, status_title};
use crate::theme::{Theme, RADIUS, RADIUS_SM};

/// The monospace font for paths and file text (the embedded JetBrains Mono
/// arrives with the kit's fonts).
pub fn mono() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

/// A shortcut as this OS writes it: ⌘ on macOS, Ctrl+Shift elsewhere
/// (`HostInfo::shortcuts`).
pub fn keys(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

// ── stroke icons (src/components/Icon.tsx) ───────────────────────────────

fn icon_path(name: &str) -> &'static str {
    match name {
        "search" => "M8.5 14a5.5 5.5 0 1 1 0-11 5.5 5.5 0 0 1 0 11zM12.5 12.5 17 17",
        "x" => "M5 5l10 10M15 5 5 15",
        "refresh" => "M15.5 10a5.5 5.5 0 1 1-1.7-4M16 3.5v3h-3",
        "branch" => "M6 4v12M6 9c0-2 1.5-3 4-3h1M13 4a2 2 0 1 1 0 4 2 2 0 0 1 0-4zM6 16",
        "folder" => "M3 5.5h5l1.5 1.5H17v8.5H3z",
        "chevron" => "M8 5l5 5-5 5",
        "chevron-down" => "M5 8l5 5 5-5",
        "review" => "M4 3.5h8l4 4v9H4zM12 3.5v4h4M7 11h6M7 14h4",
        "file" => "M5 3h6.5L15 6.5V17H5zM11.5 3v3.5H15",
        "expand" => "M11 4h5v5M16 4l-6 6M9 16H4v-5M4 16l6-6",
        "copy" => "M7.5 7.5h8.5v8.5H7.5zM4.5 12.5V4h8.5",
        "check" => "M5 10.5l3.2 3L15 6.5",
        _ => "",
    }
}

fn hex(c: Hsla) -> String {
    let r = Rgba::from(c);
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(r.r), b(r.g), b(r.b))
}

/// One of the app's 20×20 stroke icons in `color`, `size` points square
/// (drawn at 4× so it stays crisp on Retina).
pub fn icon(name: &str, size: f32, color: Hsla) -> Img {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Image>>>> = OnceLock::new();
    let key = format!("{name}{}{}", hex(color), color.a);
    let image = {
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache
            .entry(key)
            .or_insert_with(|| {
                let svg = format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80" viewBox="0 0 20 20" fill="none" stroke="{}" stroke-opacity="{}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="{}"/></svg>"#,
                    hex(color),
                    color.a,
                    icon_path(name)
                );
                let svg = super::icons::swap_red_blue(&svg);
                Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))
            })
            .clone()
    };
    img(image).size(px(size)).flex_none()
}

// ── text styles ──────────────────────────────────────────────────────────

/// `.label`: small uppercase heading ("FILES").
pub fn label(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_3)
        .child(SharedString::from(text.into().to_uppercase()))
}

/// `.kbd`: a key chip.
pub fn kbd(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .h(px(17.))
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(t.line_strong)
        .bg(t.surface_2)
        .font_family(mono())
        .text_size(px(10.))
        .text_color(t.text_3)
        .child(text.into())
}

/// `.small-btn` ("Go to file ⌘P", "Back esc", "Load anyway").
pub fn small_btn(
    t: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    key: Option<&'static str>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = t.text_4;
    div()
        .id(id)
        .h(px(26.))
        .px(px(9.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.))
        .rounded(RADIUS)
        .border_1()
        .border_color(t.line_strong)
        .bg(t.surface_3)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.border_color(hover))
        .child(label.into())
        .when_some(key, |d, k| d.child(kbd(t, k)))
        .on_click(on_click)
}

/// `.ghost-btn` with a stroke icon ("Copy path", "Show diff").
pub fn ghost_btn(
    t: &Theme,
    id: impl Into<ElementId>,
    glyph: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = t.surface_3;
    let text = t.text;
    div()
        .id(id)
        .h(px(26.))
        .px(px(9.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .rounded(RADIUS)
        .border_1()
        .border_color(t.line_strong)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text_2)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(text))
        .child(icon(glyph, 13., t.text_2))
        .child(label.into())
        .on_click(on_click)
}

/// `.icon-btn.icon-btn-sm` with a stroke icon (↻, ⤢).
pub fn icon_btn(
    t: &Theme,
    id: impl Into<ElementId>,
    glyph: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = t.surface_3;
    div()
        .id(id)
        .size(px(22.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(RADIUS)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(glyph, 13., t.text_3))
        .on_click(on_click)
}

/// `.ex-check`: a small labelled checkbox ("Ignored").
pub fn checkbox(
    t: &Theme,
    id: impl Into<ElementId>,
    on: bool,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = t.text_2;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .text_size(px(11.5))
        .text_color(t.text_3)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.text_color(hover))
        .child(
            div()
                .size(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if on { t.text_2 } else { t.text_4 })
                .bg(if on { t.text_2 } else { t.raised })
                .when(on, |d| d.child(icon("check", 10., t.raised))),
        )
        .child(label.into())
        .on_click(on_click)
}

/// A change letter's colour (`.status-letter`): only where it means
/// something (A/U green, D red).
pub fn status_color(t: &Theme, s: FileStatus) -> Hsla {
    match s {
        FileStatus::A | FileStatus::U => t.green,
        FileStatus::D => t.red,
        FileStatus::M | FileStatus::R => t.text_2,
    }
}

/// `.status-letter` with its tooltip ("Modified").
pub fn status_mark(t: &Theme, id: impl Into<ElementId>, s: FileStatus) -> AnyElement {
    let title = SharedString::from(status_title(s));
    div()
        .id(id)
        .w(px(10.))
        .flex_none()
        .flex()
        .justify_center()
        .font_family(mono())
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(status_color(t, s))
        .child(status_letter(s))
        .tooltip(move |_, cx| tooltip(title.clone(), cx))
        .into_any_element()
}

/// The refresh control (`.fresh`): "updated … ago" (or "refreshing…") and ↻.
pub fn refresh_control(
    t: &Theme,
    id: impl Into<ElementId>,
    refreshing: bool,
    updated_at: Option<u64>,
    now_ms: u64,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label: SharedString = if refreshing {
        "refreshing…".into()
    } else if let Some(at) = updated_at {
        format!("updated {}", ago(now_ms, at)).into()
    } else {
        "".into()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.text_3)
                .whitespace_nowrap()
                .child(label),
        )
        .child(
            icon_btn(t, id, "refresh", on_click)
                .tooltip(|_, cx| tooltip("Refresh files".into(), cx)),
        )
        .into_any_element()
}

/// A plain tooltip view.
pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx);
        div()
            .px(px(7.))
            .py(px(4.))
            .rounded(RADIUS_SM)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .text_size(px(11.5))
            .text_color(t.text_2)
            .child(self.0.clone())
    }
}

pub fn tooltip(text: SharedString, cx: &mut App) -> gpui::AnyView {
    cx.new(|_| Tip(text)).into()
}
