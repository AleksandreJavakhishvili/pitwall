//! The Settings dialog (`src/components/SettingsDialog.tsx` and its parts:
//! `PermissionsSettings`, `CliSettings`, `AppearanceSettings`,
//! `LayoutSettings`). Sections and words follow the React dialog; every
//! choice is shared by all windows and saved in `ui.json` (`prefs`).

use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, App, Context, EventEmitter, FontWeight, ScrollHandle,
    SharedString, Task, Window,
};

use pitwall_core::model::CodexHooksStatus;
use pitwall_core::permissions::{Access, PermissionsStatus};

use crate::theme::{
    appearance, clamp_font, Density, Look, ThemePref, DEFAULT_FONT, FONT_FLOOR, FONT_MAX, FONT_MIN,
};
use super::cli_install::{self, CliStatus};
use super::permissions::{self, POLL};
use super::widgets::Ui;
use crate::kit::{tooltip, BtnKind, SegItem};
use super::{prefs, set_prefs, ExtraSections, SettingsHost};

pub enum SettingsEvent {
    Close,
    /// "Scan again" (opens the rescan screen in place of Settings).
    ScanAgain,
    About,
}

pub struct SettingsView {
    /// An engine runs (Projects & agents, hooks need it).
    live: bool,
    hooks: Option<CodexHooksStatus>,
    hooks_confirm: bool,
    hooks_busy: bool,
    perms: Option<PermissionsStatus>,
    cli: Option<CliStatus>,
    cli_confirm: bool,
    cli_dir: Option<String>,
    cli_busy: bool,
    error: Option<String>,
    scroll: ScrollHandle,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

/// "⌘K" as this desktop writes it (Ctrl+Shift elsewhere).
pub fn keys(s: &str) -> String {
    if cfg!(target_os = "macos") {
        s.into()
    } else {
        s.replace('⌘', "Ctrl+Shift+")
    }
}

impl SettingsView {
    pub fn new(live: bool, cx: &mut Context<Self>) -> SettingsView {
        let mut tasks = Vec::new();
        let engine = cx
            .try_global::<SettingsHost>()
            .and_then(|h| h.engine.clone());
        // Codex hooks status (read-only).
        if let Some(engine) = engine {
            let read = cx
                .background_executor()
                .spawn(async move { pitwall_core::hooks::codex_status(engine.paths()) });
            tasks.push(cx.spawn(async move |this, cx| {
                let s = read.await;
                let _ = this.update(cx, |v, cx| {
                    v.hooks = Some(s);
                    cx.notify();
                });
            }));
        }
        // The command-line tool.
        let read = cx
            .background_executor()
            .spawn(async { cli_install::current() });
        tasks.push(cx.spawn(async move |this, cx| {
            let s = read.await;
            let _ = this.update(cx, |v, cx| {
                v.cli_dir = s.dirs.first().map(|d| d.path.clone());
                v.cli = Some(s);
                cx.notify();
            });
        }));
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
        SettingsView {
            live,
            hooks: None,
            hooks_confirm: false,
            hooks_busy: false,
            perms: None,
            cli: None,
            cli_confirm: false,
            cli_dir: None,
            cli_busy: false,
            error: None,
            scroll: {
                let s = ScrollHandle::new();
                // Debug builds: `PITWALL_DEBUG_OPEN=settings-end` (screenshots).
                if cfg!(debug_assertions)
                    && std::env::var("PITWALL_DEBUG_OPEN").as_deref() == Ok("settings-end")
                {
                    s.scroll_to_bottom();
                }
                s
            },
            _tasks: tasks,
        }
    }

    fn install_hooks(&mut self, cx: &mut Context<Self>) {
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

    fn install_cli(&mut self, cx: &mut Context<Self>) {
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

    fn section(ui: &Ui) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pb(px(14.))
            .border_b_1()
            .border_color(ui.t.line)
    }

    fn head(ui: &Ui, label: &str) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(crate::kit::label(label.to_string(), ui.t.text_3, 12.))
    }

    fn row(ui: &Ui, name: &str) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(crate::kit::muted(name.to_string(), &ui.t))
            .child(div().flex_1())
    }

    fn status_chip(ui: &Ui, installed: Option<bool>) -> gpui::Div {
        match installed {
            None => crate::kit::hint("checking…", &ui.t),
            Some(true) => crate::kit::chip_tone("installed", crate::kit::Tone::Ok, &ui.t),
            Some(false) => crate::kit::chip_tone("not installed", crate::kit::Tone::Subtle, &ui.t),
        }
    }

    fn projects(&self, ui: &Ui, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if !self.live {
            return None;
        }
        Some(
            Self::section(ui)
                .child(
                    Self::head(ui, "Projects & agents")
                        .child(div().flex_1())
                        .child(
                            crate::kit::text_button("scan-again", "Scan again", BtnKind::Small, false, &ui.t)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::ScanAgain))),
                        ),
                )
                .child(crate::kit::muted("Look for new projects, conversations you can continue and installed agents. Read-only.", &ui.t)),
        )
    }

    fn codex(&self, ui: &Ui, cx: &mut Context<Self>) -> gpui::Div {
        let s = self.hooks.as_ref();
        let mut sec = Self::section(ui)
            .child(
                Self::head(ui, "Exact Codex status (hooks)")
                    .child(div().flex_1())
                    .child(if self.live {
                        Self::status_chip(ui, s.map(|s| s.installed))
                    } else {
                        crate::kit::hint("unavailable", &ui.t)
                    }),
            )
            .child(crate::kit::muted("Without hooks, Pitwall reads Codex's screen to tell working, blocked and done apart. Hooks make it exact.", &ui.t));
        if let Some(s) = s {
            sec = sec.child(crate::kit::mono_text(pitwall_core::paths::tildify(&s.path), &ui.t));
            if !s.installed && !self.hooks_confirm {
                sec = sec.child(
                    div().flex().child(
                        crate::kit::text_button("hooks-install", "Install hooks…", BtnKind::Small, false, &ui.t)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.hooks_confirm = true;
                                cx.notify();
                            })),
                    ),
                );
            }
        }
        if self.hooks_confirm {
            let t = &ui.t;
            sec = sec.child(
                ui.confirm_box()
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .child("This edits")
                            .child(
                                div()
                                    .font_family(crate::kit::MONO_FONT)
                                    .text_size(px(12.))
                                    .child("~/.codex/hooks.json"),
                            )
                            .child(":"),
                    )
                    .child(
                        ui.list(
                            vec![
                                "A backup of the current file is made first.".into_any_element(),
                                "Pitwall entries are appended; your existing hooks stay."
                                    .into_any_element(),
                                "Outside Pitwall the hook does nothing and exits silently."
                                    .into_any_element(),
                            ],
                            false,
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                crate::kit::text_button("hooks-yes", if self.hooks_busy {
                                        "Installing…"
                                    } else {
                                        "Install"
                                    }, BtnKind::Primary, self.hooks_busy, &ui.t)
                                .on_click(cx.listener(
                                    |v, _, _, cx| {
                                        if !v.hooks_busy {
                                            v.install_hooks(cx)
                                        }
                                    },
                                )),
                            )
                            .child(crate::kit::text_button("hooks-no", "Cancel", BtnKind::Ghost, false, &ui.t).on_click(
                                cx.listener(|v, _, _, cx| {
                                    v.hooks_confirm = false;
                                    cx.notify();
                                }),
                            )),
                    )
                    .text_color(t.text),
            );
        }
        sec
    }

    fn claude(ui: &Ui) -> gpui::Div {
        Self::section(ui)
            .child(
                Self::head(ui, "Claude Code")
                    .child(div().flex_1())
                    .child(crate::kit::chip_tone("automatic", crate::kit::Tone::Ok, &ui.t)),
            )
            .child(
                crate::kit::muted("Hooks are passed per launch. Your ~/.claude settings are never edited.", &ui.t),
            )
    }

    fn elsewhere(ui: &Ui, cx: &mut Context<Self>) -> gpui::Div {
        let show = !prefs(cx).hide_elsewhere;
        Self::section(ui).child(
            div()
                .id("elsewhere")
                .flex()
                .gap(px(10.))
                .items_start()
                .cursor_pointer()
                .on_click(cx.listener(|_, _, window, cx| {
                    set_prefs(window, cx, |p| p.hide_elsewhere = !p.hide_elsewhere);
                    cx.notify();
                }))
                .child(div().mt(px(2.)).child(crate::kit::checkbox_box(show, false, 14., &ui.t)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child("Show agents running elsewhere")
                        .child(crate::kit::hint("A sidebar group with Claude Code, Codex and other agents running in other terminal apps (checked every ~10 s, read-only), each with “Bring in”.", &ui.t)),
                ),
        )
    }

    fn permissions(&self, ui: &Ui, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let s = self.perms.as_ref();
        if s.is_some_and(|s| !s.applies) {
            return None;
        }
        let (text, tone) = permissions::badge(s);
        let granted = s.is_some_and(|s| s.full_disk_access == Access::Granted);
        let mut sec = Self::section(ui)
            .child(
                Self::head(ui, "Permissions · Full Disk Access")
                    .child(div().flex_1())
                    .child(crate::kit::chip_tone(text, tone, &ui.t)),
            )
            .child(crate::kit::muted(if granted {
                "macOS won't ask about Desktop, Documents or Downloads. Turn it off in System Settings any time."
            } else {
                "Without it, macOS asks once for each protected folder (Desktop, Documents, Downloads) your projects are in."
            }, &ui.t));
        if s.is_some() && !granted {
            sec = sec
                .child(
                    ui.steps(vec![
                        "Open Settings: Privacy & Security → Full Disk Access.".into_any_element(),
                        "Turn on Pitwall (or click + and choose it in Applications)."
                            .into_any_element(),
                        "Come back; this row updates by itself.".into_any_element(),
                    ]),
                )
                .child(
                    div().flex().child(
                        crate::kit::text_button("fda-open", "Open Settings", BtnKind::Small, false, &ui.t)
                            .on_click(cx.listener(|v, _, _, cx| {
                                if let Err(e) = permissions::open_full_disk_access() {
                                    v.error = Some(format!("Couldn't open System Settings: {e}"));
                                    cx.notify();
                                }
                            })),
                    ),
                );
        }
        Some(sec)
    }

    fn cli(&self, ui: &Ui, cx: &mut Context<Self>) -> gpui::Div {
        let s = self.cli.as_ref();
        let mut sec = Self::section(ui)
            .child(
                Self::head(ui, "Command-line tool")
                    .child(div().flex_1())
                    .child(Self::status_chip(ui, s.map(|s| s.installed.is_some()))),
            )
            .child(
                div()
                    .text_color(ui.t.text_2)
                    .child("pitwall lets you and your agents add agents and agw sessions to Pitwall from a terminal. Anything that changes another machine waits for your OK here."),
            );
        let Some(s) = s else {
            return sec;
        };
        if let Some(link) = &s.installed {
            sec = sec.child(crate::kit::mono_text(link.clone(), &ui.t));
        }
        if s.bin.is_none() {
            sec = sec.child(crate::kit::hint("This build doesn't include the tool.", &ui.t));
        }
        if s.bin.is_some() && s.installed.is_none() && !self.cli_confirm {
            sec = sec.child(
                div().flex().child(
                    crate::kit::text_button("cli-install", "Install command-line tool…", BtnKind::Small, false, &ui.t)
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.cli_confirm = true;
                        cx.notify();
                    })),
                ),
            );
        }
        if self.cli_confirm {
            if let Some(bin) = &s.bin {
                let chosen = s
                    .dirs
                    .iter()
                    .find(|d| Some(&d.path) == self.cli_dir.as_ref());
                let mut items: Vec<AnyElement> = vec![
                    format!("It points at {bin}; nothing else changes.").into_any_element(),
                    "An existing “pitwall” that isn't Pitwall's is never replaced."
                        .into_any_element(),
                    "To uninstall, delete the link.".into_any_element(),
                ];
                if chosen.is_some_and(|d| !d.on_path) {
                    items.push(
                        "Add this folder to your shell's PATH to type just “pitwall”."
                            .into_any_element(),
                    );
                }
                let mut boxed = ui.confirm_box().child("Pitwall will create one link:");
                for (i, d) in s.dirs.iter().enumerate() {
                    let path = d.path.clone();
                    let on = self.cli_dir.as_deref() == Some(d.path.as_str());
                    boxed = boxed.child(
                        div()
                            .id(("cli-dir", i))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.cli_dir = Some(path.clone());
                                cx.notify();
                            }))
                            .child(crate::kit::radio(on, &ui.t))
                            .child(crate::kit::mono_text(format!("{}/pitwall", d.path), &ui.t).text_color(ui.t.text)),
                    );
                }
                boxed = boxed.child(ui.list(items, false)).child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            crate::kit::text_button("cli-yes", if self.cli_busy {
                                    "Installing…"
                                } else {
                                    "Install"
                                }, BtnKind::Primary, self.cli_busy || self.cli_dir.is_none(), &ui.t)
                            .on_click(cx.listener(|v, _, _, cx| {
                                if !v.cli_busy {
                                    v.install_cli(cx)
                                }
                            })),
                        )
                        .child(crate::kit::text_button("cli-no", "Cancel", BtnKind::Ghost, false, &ui.t).on_click(
                            cx.listener(|v, _, _, cx| {
                                v.cli_confirm = false;
                                cx.notify();
                            }),
                        )),
                );
                sec = sec.child(boxed);
            }
        }
        sec
    }

    fn appearance_section(ui: &Ui, cx: &mut Context<Self>) -> gpui::Div {
        let ap = appearance(cx).clone();
        let p = prefs(cx);
        let theme = crate::kit::seg("theme", ThemePref::ALL
                .iter()
                .map(|&v| SegItem {
                    label: v.label().into(),
                    on: p.theme == v,
                    tooltip: (v == ThemePref::System).then(|| {
                        SharedString::from(if cfg!(target_os = "macos") {
                            "Follow macOS"
                        } else {
                            "Follow the system"
                        })
                    }),
                })
                .collect(), &ui.t, Rc::new(|i, window, cx| set_prefs(window, cx, |p| p.theme = ThemePref::ALL[i])));
        let look = crate::kit::seg("look", Look::ALL
                .iter()
                .map(|&v| SegItem {
                    label: match v {
                        Look::Flat => "Flat".into(),
                        Look::Glass => ap.offer.label().into(),
                    },
                    on: p.look == v,
                    tooltip: (v == Look::Glass).then(|| ap.offer.hint().into()),
                })
                .collect(), &ui.t, Rc::new(|i, window, cx| set_prefs(window, cx, |p| p.look = Look::ALL[i])));
        let reduce = p.reduce_motion;
        let mut sec = Self::section(ui)
            .child(Self::head(ui, "Appearance"))
            .child(Self::row(ui, "Theme").child(theme))
            .child(Self::row(ui, "Look").child(look));
        if p.look == Look::Glass && ap.os.reduce_transparency {
            sec =
                sec.child(crate::kit::hint("The system's Reduce transparency is on, so windows show Flat.", &ui.t));
        }
        sec.child(
            Self::row(ui, "Reduce motion").child(
                crate::kit::toggle("reduce-motion", reduce, &ui.t)
                    .tooltip(tooltip("Fewer animations (the system setting also applies)"))
                    .on_click(cx.listener(|_, _, window, cx| {
                        set_prefs(window, cx, |p| p.reduce_motion = !p.reduce_motion);
                        cx.notify();
                    })),
            ),
        )
    }

    fn tiles(ui: &Ui, cx: &mut Context<Self>) -> gpui::Div {
        let p = prefs(cx);
        let density = crate::kit::seg("density", Density::ALL
                .iter()
                .map(|&d| {
                    let c = d.cells();
                    SegItem {
                        label: d.label().into(),
                        on: p.density == d,
                        tooltip: Some(
                            format!("Smallest tile {} cols × {} rows", c.cols, c.rows).into(),
                        ),
                    }
                })
                .collect(), &ui.t, Rc::new(|i, window, cx| set_prefs(window, cx, |p| p.density = Density::ALL[i])));
        let font = p.font_size;
        let step = |delta: i32| {
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                set_prefs(window, cx, |p| {
                    p.font_size = clamp_font(p.font_size as i32 + delta)
                })
            }
        };
        let font_row = Self::row(ui, "Terminal font")
            .child(
                crate::kit::text_button("font-down", "−", BtnKind::Small, font <= FONT_MIN, &ui.t)
                    .on_click(step(-1)),
            )
            .child(
                div()
                    .min_w(px(38.))
                    .flex()
                    .justify_center()
                    .font_family(crate::kit::MONO_FONT)
                    .text_size(px(12.5))
                    .child(format!("{font}px")),
            )
            .child(
                crate::kit::text_button("font-up", "+", BtnKind::Small, font >= FONT_MAX, &ui.t)
                    .on_click(step(1)),
            )
            .when(font != DEFAULT_FONT, |r| {
                r.child(
                    crate::kit::text_button("font-reset", "Reset", BtnKind::GhostSm, false, &ui.t)
                        .on_click(|_, window, cx| {
                            set_prefs(window, cx, |p| p.font_size = DEFAULT_FONT)
                        }),
                )
            });
        Self::section(ui)
            .child(Self::head(ui, "Tiles"))
            .child(Self::row(ui, "Density").child(density))
            .child(font_row)
            .child(crate::kit::hint(format!(
                "Smallest tile before extra agents fold into chips (overridable per space in its bar or {}). Tiles shrink their font down to {FONT_FLOOR}px first; {} change the focused tile only.",
                keys("⌘K"),
                keys("⌘+ / ⌘− / ⌘0"),
            ), &ui.t))
    }

    fn about_row(ui: &Ui, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(crate::kit::hint(format!("Pitwall {}", super::about::version_label()), &ui.t))
            .child(div().flex_1())
            .child(
                crate::kit::text_button("about", "About Pitwall", BtnKind::Link, false, &ui.t)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::About))),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ap = appearance(cx);
        let ui = Ui::new(ap.theme().float());
        let t = ui.t.clone();
        let extra: Vec<AnyElement> = cx
            .try_global::<ExtraSections>()
            .map(|s| s.0.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|f| f(window, cx))
            .collect();
        if cfg!(debug_assertions)
            && std::env::var("PITWALL_DEBUG_OPEN").as_deref() == Ok("settings-end")
        {
            self.scroll.scroll_to_bottom();
        }
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(18.))
            .pr(px(12.))
            .pt(px(14.))
            .pb(px(6.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child("SETTINGS"),
            )
            .child(
                crate::kit::glyph_btn("settings-close", "✕", &ui.t)
                    .tooltip(tooltip("Close (esc)"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
            );
        let body = div()
            .id("settings-body")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(18.))
            .pt(px(8.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .text_size(px(13.))
            .line_height(gpui::relative(1.45))
            .text_color(t.text)
            .when_some(self.error.clone(), |b, e| {
                b.child(
                    div()
                        .id("settings-error")
                        .flex()
                        .gap(px(8.))
                        .child(crate::kit::error_text(e, &ui.t).flex_1())
                        .child(crate::kit::text_button("err-x", "Dismiss", BtnKind::Link, false, &ui.t).on_click(
                            cx.listener(|v, _, _, cx| {
                                v.error = None;
                                cx.notify();
                            }),
                        )),
                )
            })
            .children(self.projects(&ui, cx))
            .child(self.codex(&ui, cx))
            .child(Self::claude(&ui))
            .child(Self::elsewhere(&ui, cx))
            .children(self.permissions(&ui, cx))
            .child(self.cli(&ui, cx))
            .child(Self::appearance_section(&ui, cx))
            .child(Self::tiles(&ui, cx))
            .children(extra)
            .child(Self::about_row(&ui, cx));
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .max_h_full()
            .child(head)
            .child(body)
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
    fn glass_is_named_per_desktop() {
        assert_eq!(
            crate::theme::GlassOffer::Mica.label(),
            "Glass · Mica"
        );
    }
}
