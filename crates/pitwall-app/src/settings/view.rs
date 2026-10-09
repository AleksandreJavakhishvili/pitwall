//! The Settings dialog: a compact page list on the left and one page on
//! the right (General, Agents, Rules, Appearance, Folder access, About;
//! `pitwall_proto::settings::Page`). Every option keeps what the React
//! dialog (`src/components/SettingsDialog.tsx` and its parts) does; the
//! page shown last is remembered in `ui.json` (`settingsPage`). ↑/↓ move
//! between pages; Esc, ✕ and a click outside close it.
//!
//! The pages themselves are in [`super::pages`].

use gpui::{
    actions, div, prelude::*, px, App, Context, EventEmitter, FocusHandle, Focusable, KeyBinding,
    ScrollHandle, Subscription, Task, Window,
};

use pitwall_core::model::CodexHooksStatus;
use pitwall_core::permissions::PermissionsStatus;
use pitwall_proto::settings::Page;

use super::cli_install::{self, CliStatus};
use super::permissions::POLL;
use super::widgets::Ui;
use super::{data, SettingsHost};
use crate::kit::{tooltip, BtnKind, HoverText as _};
use crate::theme::{appearance, RADIUS_SM};

actions!(pw_settings, [PrevPage, NextPage]);

/// The key context of the page list.
pub const NAV_CONTEXT: &str = "SettingsNav";

pub fn bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", PrevPage, Some(NAV_CONTEXT)),
        KeyBinding::new("down", NextPage, Some(NAV_CONTEXT)),
    ]
}

/// The dialog's size: pages fit without scrolling, except Rules.
pub const WIDTH: f32 = 720.;
pub const HEIGHT: f32 = 540.;
/// The page list's width.
pub const NAV_WIDTH: f32 = 164.;

pub enum SettingsEvent {
    Close,
    /// "Scan again" (opens the rescan screen in place of Settings).
    ScanAgain,
}

pub struct SettingsView {
    /// An engine runs (Projects & agents, hooks need it).
    pub(super) live: bool,
    pub(super) hooks: Option<CodexHooksStatus>,
    pub(super) hooks_confirm: bool,
    pub(super) hooks_busy: bool,
    pub(super) perms: Option<PermissionsStatus>,
    /// Folder access → Advanced: the Full Disk Access steps are open.
    pub(super) fda_open: bool,
    pub(super) cli: Option<CliStatus>,
    pub(super) cli_confirm: bool,
    pub(super) cli_dir: Option<String>,
    pub(super) cli_busy: bool,
    pub(super) error: Option<String>,
    /// Agents → Race Engineer.
    pub(super) engineer: super::engineer::Section,
    pub(super) focus: FocusHandle,
    scroll: ScrollHandle,
    _tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// "⌘K" as this desktop writes it (Ctrl+Shift elsewhere).
pub fn keys(s: &str) -> String {
    if cfg!(target_os = "macos") {
        s.into()
    } else {
        s.replace('⌘', "Ctrl+Shift+")
    }
}

/// The page before or after `p` (wrapping).
pub fn step(p: Page, delta: i32) -> Page {
    let all = Page::ALL;
    let i = all.iter().position(|x| *x == p).unwrap_or(0) as i32;
    all[(i + delta).rem_euclid(all.len() as i32) as usize]
}

impl SettingsView {
    pub fn new(live: bool, cx: &mut Context<Self>) -> SettingsView {
        let mut tasks = Vec::new();
        if let Some(t) = Self::read_hooks(cx) {
            tasks.push(t);
        }
        tasks.push(Self::read_cli(cx));
        // Full Disk Access, live while Settings is open.
        tasks.push(cx.spawn(async move |this, cx| loop {
            let s = cx
                .background_executor()
                .spawn(async { pitwall_core::permissions::status() })
                .await;
            let alive = this.update(cx, |v, cx| {
                if v.perms.as_ref() != Some(&s) {
                    v.perms = Some(s);
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
            cx.background_executor().timer(POLL).await;
        }));
        let store = crate::ui_state::store(cx);
        let outside = super::backend::outside(cx);
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            // `pitwall settings` installed hooks or the CLI: read again.
            cx.observe(&outside, |v, _, cx| {
                if let Some(t) = Self::read_hooks(cx) {
                    v._tasks.push(t);
                }
                let t = Self::read_cli(cx);
                v._tasks.push(t);
            }),
        ];
        SettingsView {
            live,
            hooks: None,
            hooks_confirm: false,
            hooks_busy: false,
            perms: None,
            fda_open: false,
            cli: None,
            cli_confirm: false,
            cli_dir: None,
            cli_busy: false,
            error: None,
            engineer: Default::default(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _tasks: tasks,
            _subs: subs,
        }
    }

    /// Codex hooks status (read-only).
    fn read_hooks(cx: &mut Context<Self>) -> Option<Task<()>> {
        let engine = cx.try_global::<SettingsHost>().and_then(|h| h.engine.clone())?;
        let read = cx
            .background_executor()
            .spawn(async move { pitwall_core::hooks::codex_status(engine.paths()) });
        Some(cx.spawn(async move |this, cx| {
            let s = read.await;
            let _ = this.update(cx, |v, cx| {
                v.hooks = Some(s);
                cx.notify();
            });
        }))
    }

    /// The command-line tool.
    fn read_cli(cx: &mut Context<Self>) -> Task<()> {
        let read = cx
            .background_executor()
            .spawn(async { cli_install::current() });
        cx.spawn(async move |this, cx| {
            let s = read.await;
            let _ = this.update(cx, |v, cx| {
                if v.cli_dir.is_none() {
                    v.cli_dir = s.dirs.first().map(|d| d.path.clone());
                }
                v.cli = Some(s);
                cx.notify();
            });
        })
    }

    pub(super) fn install_hooks(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = cx
            .try_global::<SettingsHost>()
            .and_then(|h| h.engine.clone())
        else {
            return;
        };
        self.hooks_busy = true;
        cx.notify();
        let run = cx
            .background_executor()
            .spawn(async move { pitwall_core::hooks::install_codex(engine.paths()) });
        cx.spawn(async move |this, cx| {
            let r = run.await;
            let _ = this.update(cx, |v, cx| {
                v.hooks_busy = false;
                match r {
                    Ok(s) => {
                        v.hooks = Some(s);
                        v.hooks_confirm = false;
                    }
                    Err(e) => v.error = Some(format!("Couldn't install Codex hooks: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn install_cli(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.cli_dir.clone() else {
            return;
        };
        self.cli_busy = true;
        cx.notify();
        let run = cx
            .background_executor()
            .spawn(async move { cli_install::install_into(&dir) });
        cx.spawn(async move |this, cx| {
            let r = run.await;
            let _ = this.update(cx, |v, cx| {
                v.cli_busy = false;
                match r {
                    Ok(s) => {
                        v.cli = Some(s);
                        v.cli_confirm = false;
                    }
                    Err(e) => {
                        v.error = Some(format!("Couldn't install the command-line tool: {e}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn go(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if data::page(cx) != page {
            data::set_page(cx, page);
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
        window.focus(&self.focus);
        cx.notify();
    }

    fn prev_page(&mut self, _: &PrevPage, window: &mut Window, cx: &mut Context<Self>) {
        let p = step(data::page(cx), -1);
        self.go(p, window, cx);
    }

    fn next_page(&mut self, _: &NextPage, window: &mut Window, cx: &mut Context<Self>) {
        let p = step(data::page(cx), 1);
        self.go(p, window, cx);
    }

    fn nav(&self, ui: &Ui, page: Page, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = &ui.t;
        let (hover_bg, on_bg) = if t.is_glass() {
            (t.g.surface_2, t.g.surface_3)
        } else {
            (t.surface_2, t.surface_3)
        };
        div()
            .id("settings-nav")
            .track_focus(&self.focus)
            .key_context(NAV_CONTEXT)
            .on_action(cx.listener(Self::prev_page))
            .on_action(cx.listener(Self::next_page))
            .w(px(NAV_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(1.))
            .px(px(8.))
            .py(px(8.))
            .border_r_1()
            .border_color(t.line)
            .children(Page::ALL.into_iter().enumerate().map(|(i, p)| {
                let on = p == page;
                div()
                    .id(("settings-page", i))
                    .flex()
                    .items_center()
                    .h(px(28.))
                    .px(px(10.))
                    .rounded(RADIUS_SM)
                    .text_size(px(13.))
                    .cursor_pointer()
                    .text_color(if on { t.text } else { t.text_2 })
                    .when(on, |d| d.bg(on_bg).font_weight(crate::kit::WEIGHT_550))
                    .when(!on, |d| d.hover_text(t.text, |s| s.bg(hover_bg)))
                    .on_click(cx.listener(move |v, _, window, cx| v.go(p, window, cx)))
                    .child(p.label())
            }))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ap = appearance(cx);
        let ui = Ui::new(ap.theme().float());
        let t = ui.t.clone();
        let page = data::page(cx);
        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .pl(px(18.))
            .pr(px(12.))
            .pt(px(14.))
            .pb(px(10.))
            .border_b_1()
            .border_color(t.line)
            // `.label-lg`: Barlow Condensed 600, 15 px, 0.08em.
            .child(crate::kit::label_t("Settings", t.text, 15., 0.08))
            .child(
                crate::kit::glyph_btn("settings-close", "✕", &ui.t)
                    .tooltip(tooltip("Close (esc)"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
            );
        let sections = self.page(page, &ui, window, cx);
        let mut body = div()
            .id("settings-page-body")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .px(px(22.))
            .pt(px(16.))
            .pb(px(20.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(crate::kit::label(page.label(), t.text_3, 12.))
            .when_some(self.error.clone(), |b, e| {
                b.child(
                    div()
                        .id("settings-error")
                        .flex()
                        .gap(px(8.))
                        .child(crate::kit::error_text(e, &ui.t).flex_1())
                        .child(
                            crate::kit::text_button("err-x", "Dismiss", BtnKind::Link, false, &ui.t)
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.error = None;
                                    cx.notify();
                                })),
                        ),
                )
            });
        let n = sections.len();
        for (i, s) in sections.into_iter().enumerate() {
            body = body.child(s.when(i + 1 < n, |s| s.pb(px(14.)).border_b_1().border_color(t.line)));
        }
        div()
            .flex()
            .flex_col()
            .h(px(HEIGHT))
            .max_h_full()
            .min_h_0()
            .child(head)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.nav(&ui, page, cx))
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_follow_the_desktop() {
        if cfg!(target_os = "macos") {
            assert_eq!(keys("⌘K"), "⌘K");
        } else {
            assert_eq!(keys("⌘+ / ⌘0"), "Ctrl+Shift++ / Ctrl+Shift+0");
        }
    }

    #[test]
    fn arrows_move_between_pages_and_wrap() {
        assert_eq!(step(Page::General, 1), Page::Agents);
        assert_eq!(step(Page::General, -1), Page::About);
        assert_eq!(step(Page::About, 1), Page::General);
        assert_eq!(step(Page::Appearance, -1), Page::Rules);
    }

    #[test]
    fn glass_is_named_per_desktop() {
        assert_eq!(crate::theme::GlassOffer::Mica.label(), "Glass · Mica");
    }
}
