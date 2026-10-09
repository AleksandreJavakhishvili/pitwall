//! About Pitwall: version and build facts (the app menu's "About Pitwall"
//! where the OS has no About panel, and Settings → About). "Copy" puts them on the clipboard
//! for bug reports.

use gpui::{
    div, prelude::*, px, AnyElement, App, ClickEvent, ClipboardItem, FontWeight, SharedString,
    Window,
};

use pitwall_core::paths::tildify;

use crate::theme::{appearance, GlassTier};
use super::widgets::Ui;
use crate::kit::BtnKind;
use super::SettingsHost;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The version as Settings and About show it; preview-channel bundles
/// (`PITWALL_CHANNEL=preview`, `dev.pitwall.app.preview`) say so.
pub fn version_label() -> String {
    if crate::platform::BUNDLE_ID.ends_with(".preview") {
        format!("{VERSION} (preview)")
    } else {
        VERSION.to_string()
    }
}
/// The GPUI release this build pins (Cargo.toml `gpui = "=0.2.2"`).
pub const GPUI_VERSION: &str = "0.2.2";

/// The facts, as (label, value) rows.
pub fn facts(cx: &App) -> Vec<(&'static str, String)> {
    let ap = appearance(cx);
    let renderer = if cfg!(target_os = "macos") {
        "Metal"
    } else if cfg!(windows) {
        "DirectX"
    } else {
        "Vulkan"
    };
    let material = match ap.glass {
        None => "Flat".to_string(),
        Some(GlassTier::Liquid) => "Liquid Glass".into(),
        Some(GlassTier::Native) => "Glass (vibrancy)".into(),
        Some(GlassTier::Mica) => "Glass (Mica)".into(),
        Some(GlassTier::Lite) => "Glass lite".into(),
    };
    let data = cx
        .try_global::<SettingsHost>()
        .and_then(|h| h.root.as_ref())
        .map(|r| tildify(&r.to_string_lossy()))
        .unwrap_or_else(|| "none (not started)".into());
    vec![
        ("Version", version_label()),
        (
            "Build",
            format!(
                "{} · {}-{}",
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
                std::env::consts::ARCH,
                std::env::consts::OS
            ),
        ),
        ("UI", format!("GPUI {GPUI_VERSION} · {renderer}")),
        ("Look", material),
        ("Data folder", data),
    ]
}

/// Plain text for the clipboard.
pub fn as_text(facts: &[(&'static str, String)]) -> String {
    let mut out = String::from("Pitwall\n");
    for (k, v) in facts {
        out.push_str(&format!("{k}: {v}\n"));
    }
    out
}

/// The PIT|WALL timing board.
pub fn board(t: &crate::theme::Theme) -> gpui::Div {
    div()
        .flex()
        .gap(px(4.))
        .p(px(5.))
        .rounded(px(6.))
        .bg(t.surface_2)
        .border_1()
        .border_color(t.line_strong)
        .children(["PIT", "WALL"].into_iter().enumerate().map(|(i, s)| {
            div()
                .px(px(8.))
                .py(px(2.))
                .rounded(px(3.))
                .bg(t.bg)
                .text_size(px(14.))
                .font_weight(FontWeight::BOLD)
                .text_color(if i == 1 { t.text } else { t.text_2 })
                .child(s)
        }))
}

pub fn render(
    _window: &mut Window,
    cx: &App,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let ap = appearance(cx);
    let ui = Ui::new(ap.theme().float());
    let t = ui.t.clone();
    let facts = facts(cx);
    let text = as_text(&facts);
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(18.))
                .pt(px(20.))
                .pb(px(6.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().flex().child(board(&t)))
                .child(
                    div()
                        .mt(px(8.))
                        .text_size(px(22.))
                        .font_weight(FontWeight::BOLD)
                        .child("PITWALL"),
                )
                .child(crate::kit::hint("You call the strategy. Agents drive.", &ui.t)),
        )
        .child(
            div()
                .px(px(18.))
                .py(px(10.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(facts.into_iter().map(|(k, v)| {
                    div()
                        .flex()
                        .gap(px(10.))
                        .text_size(px(12.5))
                        .child(div().w(px(96.)).flex_none().text_color(t.text_3).child(k))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(t.text)
                                .child(SharedString::from(v)),
                        )
                })),
        )
        .child(
            div()
                .px(px(18.))
                .pb(px(4.))
                .child(crate::kit::hint("Apache-2.0 · Copyright 2026 the Pitwall authors", &ui.t)),
        )
        .child(
            div()
                .mt(px(8.))
                .px(px(18.))
                .py(px(12.))
                .flex()
                .gap(px(8.))
                .justify_end()
                .border_t_1()
                .border_color(t.line)
                .child(crate::kit::text_button("about-copy", "Copy", BtnKind::Ghost, false, &ui.t).on_click(
                    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())),
                ))
                .child(
                    crate::kit::text_button("about-close", "Close", BtnKind::Primary, false, &ui.t)
                        .on_click(on_close),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_has_every_fact() {
        let f = vec![
            ("Version", "1.2.3".to_string()),
            ("Look", "Flat".into()),
        ];
        let s = as_text(&f);
        assert!(s.starts_with("Pitwall\n"));
        assert!(s.contains("Version: 1.2.3\n") && s.contains("Look: Flat\n"));
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        let toml = include_str!("../../Cargo.toml");
        assert!(
            toml.contains(&format!("gpui = \"={GPUI_VERSION}\"")),
            "update GPUI_VERSION"
        );
    }
}
