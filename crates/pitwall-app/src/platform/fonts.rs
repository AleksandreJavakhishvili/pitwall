//! The CSS font stacks of tokens.css (`--font-label`, `--font-mono`),
//! resolved to the first family this machine has, since gpui takes one
//! family and its fallback for a missing one loses the weight. Until the app
//! bundles Barlow Condensed and JetBrains Mono (promote to theme.rs with font
//! loading), the label falls back to the UI font and mono to Menlo / the
//! platform's monospace.

use std::sync::OnceLock;

use gpui::{App, SharedString};

const LABEL: &[&str] = &["Barlow Condensed", "Arial Narrow"];
const MONO: &[&str] = &[
    "JetBrains Mono",
    "Menlo",
    "Consolas",
    "DejaVu Sans Mono",
    "Liberation Mono",
];

/// The first of `stack` that `installed` lists.
fn pick(stack: &[&'static str], installed: &[String]) -> Option<SharedString> {
    stack
        .iter()
        .find(|f| installed.iter().any(|i| i == *f))
        .map(|f| SharedString::new_static(f))
}

fn installed(cx: &App) -> &'static Vec<String> {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| cx.text_system().all_font_names())
}

/// `--font-label`, or `None` for the UI font.
pub fn label(cx: &App) -> Option<SharedString> {
    pick(LABEL, installed(cx))
}

/// `--font-mono`, or `None` for the UI font.
pub fn mono(cx: &App) -> Option<SharedString> {
    pick(MONO, installed(cx))
}

/// Both stacks, resolved once for a render.
#[derive(Debug, Clone, Default)]
pub struct Fonts {
    pub label: Option<SharedString>,
    pub mono: Option<SharedString>,
}

impl Fonts {
    pub fn get(cx: &App) -> Fonts {
        Fonts {
            label: label(cx),
            mono: mono(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_installed_family_wins() {
        let have = vec!["Menlo".to_string(), "Helvetica".to_string()];
        assert_eq!(pick(MONO, &have), Some(SharedString::from("Menlo")));
        assert_eq!(pick(LABEL, &have), None);
    }
}
