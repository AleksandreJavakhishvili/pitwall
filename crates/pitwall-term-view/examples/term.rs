//! A terminal window: `$SHELL` (or the given command) on a local PTY.
//!
//!   cargo run -p pitwall-term-view --example term
//!   cargo run -p pitwall-term-view --example term -- [--light] [--no-blink] [--font-file F.ttf]… [--type TEXT] [-- CMD ARGS…]
//!
//! `--font-file` registers a font (e.g. JetBrains Mono, which Pitwall's web
//! UI bundles) before the window opens. `--type TEXT` types TEXT into the terminal once it has started (`\n` is
//! Enter), handy for demos and screenshots.

use std::time::Duration;

use gpui::{
    div, prelude::*, px, size, App, Application, Bounds, Context, Entity, Focusable, KeyBinding, Window, WindowBounds,
    WindowOptions,
};
use pitwall_term_view::pty::LocalPty;
use pitwall_term_view::{
    default_key_bindings, TermEvent, TermSize, TermTheme, Terminal, TerminalConfig, TerminalView, ViewMode,
    ViewSettings,
};
use portable_pty::CommandBuilder;

gpui::actions!(example, [Quit]);

struct Root {
    term: Entity<TerminalView>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.term.clone())
    }
}

struct Args {
    light: bool,
    no_blink: bool,
    fonts: Vec<String>,
    typed: Option<String>,
    command: Vec<String>,
}

fn args() -> Args {
    let mut a = Args { light: false, no_blink: false, fonts: Vec::new(), typed: None, command: Vec::new() };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--light" => a.light = true,
            "--no-blink" => a.no_blink = true,
            "--font-file" => a.fonts.extend(it.next()),
            "--type" => a.typed = it.next().map(|t| t.replace("\\n", "\n")),
            "--" => {
                a.command = it.collect();
                break;
            }
            _ => {
                eprintln!("usage: term [--light] [--no-blink] [--font-file F.ttf]… [--type TEXT] [-- CMD ARGS…]");
                std::process::exit(2);
            }
        }
    }
    a
}

fn main() {
    let args = args();
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(default_key_bindings());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        // E.g. the JetBrains Mono TTF the app bundles; otherwise Menlo is used.
        let fonts = args.fonts.iter().map(|f| std::fs::read(f).expect("read font file").into()).collect();
        pitwall_term_view::register_fonts(cx, fonts).expect("register fonts");

        let size0 = TermSize::new(100, 30);
        let pty = if args.command.is_empty() {
            LocalPty::shell(size0)
        } else {
            let mut cmd = CommandBuilder::new(&args.command[0]);
            cmd.args(&args.command[1..]);
            if let Ok(dir) = std::env::current_dir() {
                cmd.cwd(dir);
            }
            LocalPty::spawn(cmd, size0)
        }
        .expect("start the shell");
        let theme = if args.light { TermTheme::pitwall_light() } else { TermTheme::pitwall_dark() };
        let terminal = Terminal::new(pty, size0, TerminalConfig { theme, ..Default::default() });

        let bounds = Bounds::centered(None, size(px(920.), px(600.)), cx);
        let typed = args.typed.clone();
        let settings = ViewSettings { cursor_blink: !args.no_blink, ..Default::default() };
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |window, cx| {
                window.set_window_title("Terminal");
                let term =
                    cx.new(|cx| TerminalView::new(terminal.clone(), ViewMode::Interactive, settings, window, cx));
                window.focus(&term.read(cx).focus_handle(cx));
                if let Some(text) = typed {
                    let t = terminal.clone();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(Duration::from_millis(700)).await;
                        t.write(text.as_bytes());
                    })
                    .detach();
                }
                cx.new(|cx| {
                    cx.subscribe_in(&term, window, |_, _, ev: &TermEvent, window, cx| match ev {
                        TermEvent::Title(t) => window.set_window_title(t.as_deref().unwrap_or("Terminal")),
                        TermEvent::Exited => cx.quit(),
                        _ => {}
                    })
                    .detach();
                    Root { term }
                })
            },
        )
        .expect("open window");
        cx.activate(true);
    });
}
