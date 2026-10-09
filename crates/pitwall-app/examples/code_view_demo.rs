//! The code/diff view on made-up text, for looking at it:
//! `cargo run -p pitwall-app --example code_view_demo [-- unified|file|light]`.

use gpui::{
    div, point, prelude::*, px, size, App, Application, Bounds, Context, Entity, Focusable, Window,
    WindowBounds, WindowOptions,
};

use pitwall_app::code_view::{CodeView, Layout};
use pitwall_app::theme::{Mode, Theme};

const OLD: &str = r#"use std::collections::HashMap;

/// A small cache of greetings.
pub struct Greeter {
    names: HashMap<String, usize>,
}

impl Greeter {
    pub fn new() -> Self {
        Greeter { names: HashMap::new() }
    }

    pub fn greet(&mut self, name: &str) -> String {
        let n = self.names.entry(name.to_string()).or_insert(0);
        *n += 1;
        format!("Hello, {name}!")
    }

    pub fn count(&self, name: &str) -> usize {
        self.names.get(name).copied().unwrap_or(0)
    }

    pub fn reset(&mut self) {
        self.names.clear();
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}
"#;

const NEW: &str = r#"use std::collections::HashMap;

/// A small cache of greetings, in any language.
pub struct Greeter {
    names: HashMap<String, usize>,
    language: Language,
}

#[derive(Clone, Copy)]
pub enum Language {
    English,
    Georgian,
}

impl Greeter {
    pub fn new(language: Language) -> Self {
        Greeter { names: HashMap::new(), language }
    }

    pub fn greet(&mut self, name: &str) -> String {
        let n = self.names.entry(name.to_string()).or_insert(0);
        *n += 1;
        match self.language {
            Language::English => format!("Hello, {name}!"),
            Language::Georgian => format!("გამარჯობა, {name}!"),
        }
    }

    pub fn count(&self, name: &str) -> usize {
        self.names.get(name).copied().unwrap_or(0)
    }

    pub fn reset(&mut self) {
        self.names.clear();
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}
"#;

struct Demo {
    view: Entity<CodeView>,
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    Application::new().run(move |cx: &mut App| {
        let mode = if arg == "light" {
            Mode::Light
        } else {
            Mode::Dark
        };
        cx.set_global(Theme::for_mode(mode));
        pitwall_app::code_view::register(cx);
        let bounds = Bounds::new(point(px(80.), px(80.)), size(px(1100.), px(640.)));
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| {
                    let mut v = CodeView::new(cx);
                    v.set_commentable(true, cx);
                    match arg.as_str() {
                        "file" => v.set_file("src/greeter.rs", NEW.to_string(), cx),
                        _ => v.set_diff("src/greeter.rs", OLD.to_string(), NEW.to_string(), cx),
                    }
                    if arg == "unified" {
                        v.set_layout(Layout::Unified, cx);
                    }
                    v.set_comments(
                        [(21, "Say which language when it is unknown.".to_string())],
                        cx,
                    );
                    v
                });
                window.focus(&view.focus_handle(cx));
                cx.new(|_| Demo { view })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
