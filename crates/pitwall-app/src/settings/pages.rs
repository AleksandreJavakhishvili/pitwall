//! The Settings pages. Each is a list of sections (hairlines between them,
//! drawn by the dialog); the options and words are the React dialog's,
//! regrouped: General (scan again, agents elsewhere, the CLI), Agents
//! (Claude Code, Codex hooks, the Race Engineer), Rules, Appearance (theme, look, motion,
//! density, terminal font), Folder access and About.

use std::rc::Rc;

use gpui::{div, prelude::*, px, AnyElement, App, ClipboardItem, Context, Div, FontWeight, SharedString, Window};

use pitwall_core::permissions::Access;
use pitwall_proto::settings::Page;

use super::view::{keys, SettingsEvent, SettingsView};
use super::widgets::Ui;
use super::{about, permissions, prefs, set_prefs, ExtraSections};
use crate::kit::{tooltip, BtnKind, SegItem};
use crate::theme::{appearance, clamp_font, Density, Look, ThemePref, DEFAULT_FONT, FONT_FLOOR, FONT_MAX, FONT_MIN};

/// A section: its parts 8 px apart.
fn section() -> Div {
    div().flex().flex_col().gap(px(8.))
}

/// A section's title row (sentence case, 13 px medium), controls after it.
fn head(ui: &Ui, title: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(26.))
        .child(
            div()
                .font_weight(crate::kit::WEIGHT_550)
                .text_color(ui.t.text)
                .child(title.to_string()),
        )
        .child(div().flex_1())
}

/// A labelled row inside a section ("Theme  [seg]").
fn row(ui: &Ui, name: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(28.))
        .child(crate::kit::muted(name.to_string(), &ui.t))
        .child(div().flex_1())
}

fn status_chip(ui: &Ui, installed: Option<bool>) -> Div {
    match installed {
        None => crate::kit::hint("checking…", &ui.t),
        Some(true) => crate::kit::chip_tone("installed", crate::kit::Tone::Ok, &ui.t),
        Some(false) => crate::kit::chip_tone("not installed", crate::kit::Tone::Subtle, &ui.t),
    }
}

/// External links on About.
pub const WEBSITE: &str = "https://aleksandrejavakhishvili.github.io/pitwall/";
pub const SOURCE: &str = "https://github.com/AleksandreJavakhishvili/pitwall";
pub const ISSUES: &str = "https://github.com/AleksandreJavakhishvili/pitwall/issues";
pub const LICENSE: &str = "https://github.com/AleksandreJavakhishvili/pitwall/blob/main/LICENSE";
pub const THIRD_PARTY: &str = "https://github.com/AleksandreJavakhishvili/pitwall/tree/main/LICENSES";

impl SettingsView {
    /// The sections of `page`.
    pub(super) fn page(&mut self, page: Page, ui: &Ui, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        match page {
            Page::General => {
                let mut v: Vec<Div> = self.projects(ui, cx).into_iter().collect();
                v.push(Self::elsewhere(ui, cx));
                v.push(self.cli(ui, cx));
                v
            }
            Page::Agents => {
                let codex = self.codex(ui, cx);
                let engineer = self.engineer(ui, head(ui, "Race Engineer"), |name| row(ui, name), cx);
                vec![Self::claude(ui), codex, engineer]
            }
            Page::Rules => {
                let extra: Vec<AnyElement> = cx
                    .try_global::<ExtraSections>()
                    .map(|s| s.0.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f| f(window, cx))
                    .collect();
                if extra.is_empty() {
                    return vec![section().child(crate::kit::muted("Rules need Pitwall's engine, which isn't running.", &ui.t))];
                }
                extra.into_iter().map(|e| div().flex().flex_col().child(e)).collect()
            }
            Page::Appearance => vec![Self::appearance_section(ui, cx), Self::tiles(ui, cx)],
            Page::FolderAccess => self.folder_access(ui, cx),
            Page::About => vec![Self::about(ui, cx)],
        }
    }

    fn projects(&self, ui: &Ui, cx: &mut Context<Self>) -> Option<Div> {
        if !self.live {
            return None;
        }
        Some(
            section()
                .child(
                    head(ui, "Projects & agents").child(
                        crate::kit::text_button("scan-again", "Scan again", BtnKind::Small, false, &ui.t)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::ScanAgain))),
                    ),
                )
                .child(crate::kit::muted("Look for new projects, conversations you can continue and installed agents. Read-only.", &ui.t)),
        )
    }

    fn elsewhere(ui: &Ui, cx: &mut Context<Self>) -> Div {
        let show = !prefs(cx).hide_elsewhere;
        section().child(
            div()
                .id("elsewhere")
                .flex()
                .gap(px(12.))
                .items_start()
                .cursor_pointer()
                .on_click(cx.listener(|_, _, window, cx| {
                    set_prefs(window, cx, |p| p.hide_elsewhere = !p.hide_elsewhere);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(head(ui, "Show agents running elsewhere"))
                        .child(crate::kit::hint("A sidebar group with Claude Code, Codex and other agents running in other terminal apps (checked every ~10 s, read-only), each with “Bring in”.", &ui.t)),
                )
                .child(div().mt(px(6.)).child(crate::kit::checkbox_box(show, false, 14., &ui.t))),
        )
    }

    fn claude(ui: &Ui) -> Div {
        section()
            .child(head(ui, "Claude Code").child(crate::kit::chip_tone("automatic", crate::kit::Tone::Ok, &ui.t)))
            .child(crate::kit::muted("Hooks are passed per launch. Your ~/.claude settings are never edited.", &ui.t))
    }

    fn codex(&self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let s = self.hooks.as_ref();
        let mut sec = section()
            .child(head(ui, "Exact Codex status (hooks)").child(if self.live {
                status_chip(ui, s.map(|s| s.installed))
            } else {
                crate::kit::hint("unavailable", &ui.t)
            }))
            .child(crate::kit::muted("Without hooks, Pitwall reads Codex's screen to tell working, blocked and done apart. Hooks make it exact.", &ui.t));
        if let Some(s) = s {
            sec = sec.child(crate::kit::mono_text(pitwall_core::paths::tildify(&s.path), &ui.t));
            if !s.installed && !self.hooks_confirm {
                sec = sec.child(
                    div().flex().child(
                        crate::kit::text_button("hooks-install", "Install hooks…", BtnKind::Small, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                            v.hooks_confirm = true;
                            cx.notify();
                        })),
                    ),
                );
            }
        }
        if self.hooks_confirm {
            sec = sec.child(
                ui.confirm_box()
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .child("This edits")
                            .child(div().font_family(crate::kit::MONO_FONT).text_size(px(12.)).child("~/.codex/hooks.json"))
                            .child(":"),
                    )
                    .child(ui.list(
                        vec![
                            "A backup of the current file is made first.".into_any_element(),
                            "Pitwall entries are appended; your existing hooks stay.".into_any_element(),
                            "Outside Pitwall the hook does nothing and exits silently.".into_any_element(),
                        ],
                        false,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                crate::kit::text_button("hooks-yes", if self.hooks_busy { "Installing…" } else { "Install" }, BtnKind::Primary, self.hooks_busy, &ui.t)
                                    .on_click(cx.listener(|v, _, _, cx| {
                                        if !v.hooks_busy {
                                            v.install_hooks(cx)
                                        }
                                    })),
                            )
                            .child(crate::kit::text_button("hooks-no", "Cancel", BtnKind::Ghost, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                                v.hooks_confirm = false;
                                cx.notify();
                            }))),
                    )
                    .text_color(ui.t.text),
            );
        }
        sec
    }

    fn cli(&self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let s = self.cli.as_ref();
        let mut sec = section()
            .child(head(ui, "Command-line tool").child(status_chip(ui, s.map(|s| s.installed.is_some()))))
            .child(div().child(crate::kit::rich(
                &[
                    ("pitwall", crate::kit::Span::Mono),
                    (" lets you and your agents add agents and agw sessions to Pitwall and change these settings from a terminal. Anything that changes another machine or something outside Pitwall waits for your OK here.", crate::kit::Span::Text),
                ],
                ui.t.text_2,
            )));
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
                    crate::kit::text_button("cli-install", "Install command-line tool…", BtnKind::Small, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                        v.cli_confirm = true;
                        cx.notify();
                    })),
                ),
            );
        }
        if self.cli_confirm {
            if let Some(bin) = &s.bin {
                let chosen = s.dirs.iter().find(|d| Some(&d.path) == self.cli_dir.as_ref());
                let mut items: Vec<AnyElement> = vec![
                    format!("It points at {bin}; nothing else changes.").into_any_element(),
                    "An existing “pitwall” that isn't Pitwall's is never replaced.".into_any_element(),
                    "To uninstall, delete the link.".into_any_element(),
                ];
                if chosen.is_some_and(|d| !d.on_path) {
                    items.push("Add this folder to your shell's PATH to type just “pitwall”.".into_any_element());
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
                            crate::kit::text_button("cli-yes", if self.cli_busy { "Installing…" } else { "Install" }, BtnKind::Primary, self.cli_busy || self.cli_dir.is_none(), &ui.t)
                                .on_click(cx.listener(|v, _, _, cx| {
                                    if !v.cli_busy {
                                        v.install_cli(cx)
                                    }
                                })),
                        )
                        .child(crate::kit::text_button("cli-no", "Cancel", BtnKind::Ghost, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                            v.cli_confirm = false;
                            cx.notify();
                        }))),
                );
                sec = sec.child(boxed);
            }
        }
        sec
    }

    fn appearance_section(ui: &Ui, cx: &mut Context<Self>) -> Div {
        let ap = appearance(cx).clone();
        let p = prefs(cx);
        let theme = crate::kit::seg(
            "theme",
            ThemePref::ALL
                .iter()
                .map(|&v| SegItem {
                    label: v.label().into(),
                    on: p.theme == v,
                    tooltip: (v == ThemePref::System).then(|| SharedString::from(if cfg!(target_os = "macos") { "Follow macOS" } else { "Follow the system" })),
                })
                .collect(),
            &ui.t,
            Rc::new(|i, window, cx| set_prefs(window, cx, |p| p.theme = ThemePref::ALL[i])),
        );
        let look = crate::kit::seg(
            "look",
            Look::ALL
                .iter()
                .map(|&v| SegItem {
                    label: match v {
                        Look::Flat => "Flat".into(),
                        Look::Glass => ap.offer.label().into(),
                    },
                    on: p.look == v,
                    tooltip: (v == Look::Glass).then(|| ap.offer.hint().into()),
                })
                .collect(),
            &ui.t,
            Rc::new(|i, window, cx| set_prefs(window, cx, |p| p.look = Look::ALL[i])),
        );
        let mut sec = section().gap(px(4.)).child(row(ui, "Theme").child(theme)).child(row(ui, "Look").child(look));
        if p.look == Look::Glass && ap.os.reduce_transparency {
            sec = sec.child(crate::kit::hint("The system's Reduce transparency is on, so windows show Flat.", &ui.t));
        }
        sec.child(
            row(ui, "Reduce motion").child(
                crate::kit::toggle("reduce-motion", p.reduce_motion, &ui.t)
                    .tooltip(tooltip("Fewer animations (the system setting also applies)"))
                    .on_click(cx.listener(|_, _, window, cx| {
                        set_prefs(window, cx, |p| p.reduce_motion = !p.reduce_motion);
                        cx.notify();
                    })),
            ),
        )
    }

    fn tiles(ui: &Ui, cx: &mut Context<Self>) -> Div {
        let p = prefs(cx);
        let density = crate::kit::seg(
            "density",
            Density::ALL
                .iter()
                .map(|&d| {
                    let c = d.cells();
                    SegItem {
                        label: d.label().into(),
                        on: p.density == d,
                        tooltip: Some(format!("Smallest tile {} cols × {} rows", c.cols, c.rows).into()),
                    }
                })
                .collect(),
            &ui.t,
            Rc::new(|i, window, cx| set_prefs(window, cx, |p| p.density = Density::ALL[i])),
        );
        let font = p.font_size;
        let step = |delta: i32| {
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| set_prefs(window, cx, |p| p.font_size = clamp_font(p.font_size as i32 + delta))
        };
        let font_row = row(ui, "Terminal font")
            .when(font != DEFAULT_FONT, |r| {
                r.child(
                    crate::kit::text_button("font-reset", "Reset", BtnKind::GhostSm, false, &ui.t)
                        .on_click(|_, window, cx| set_prefs(window, cx, |p| p.font_size = DEFAULT_FONT)),
                )
            })
            .child(crate::kit::text_button("font-down", "−", BtnKind::Small, font <= FONT_MIN, &ui.t).on_click(step(-1)))
            .child(
                div()
                    .min_w(px(38.))
                    .flex()
                    .justify_center()
                    .font_family(crate::kit::MONO_FONT)
                    .text_size(px(12.5))
                    .child(format!("{font}px")),
            )
            .child(crate::kit::text_button("font-up", "+", BtnKind::Small, font >= FONT_MAX, &ui.t).on_click(step(1)));
        section()
            .gap(px(4.))
            .child(row(ui, "Density").child(density))
            .child(font_row)
            .child(div().pt(px(4.)).child(crate::kit::hint(
                format!(
                    "Density is the smallest tile before extra agents fold into chips (overridable per space in its bar or {}). Tiles shrink their font down to {FONT_FLOOR}px first; {} change the focused tile only.",
                    keys("⌘K"),
                    keys("⌘+ / ⌘− / ⌘0"),
                ),
                &ui.t,
            )))
    }

    /// Folder access: macOS's per-folder prompts are the normal path; Full
    /// Disk Access is an advanced option with an honest warning.
    fn folder_access(&self, ui: &Ui, cx: &mut Context<Self>) -> Vec<Div> {
        let s = self.perms.as_ref();
        if s.is_some_and(|s| !s.applies) {
            return vec![section().child(crate::kit::muted("This system doesn't guard folders the way macOS does: Pitwall reads your projects without asking.", &ui.t))];
        }
        let granted = s.is_some_and(|s| s.full_disk_access == Access::Granted);
        let prompts = section()
            .child(head(ui, "Ask per folder").child(if granted {
                crate::kit::chip_tone("not needed now", crate::kit::Tone::Subtle, &ui.t)
            } else {
                crate::kit::chip_tone("recommended", crate::kit::Tone::Ok, &ui.t)
            }))
            .child(crate::kit::muted("macOS asks “Pitwall would like to access…” the first time Pitwall or an agent reads a project in Desktop, Documents or Downloads: at most three prompts, once each. Projects anywhere else never prompt.", &ui.t))
            .child(crate::kit::hint("Answered “Don't Allow” by mistake? Turn Pitwall on in System Settings → Privacy & Security → Files & Folders.", &ui.t));
        let (text, tone) = permissions::badge(s);
        let mut fda = section()
            .child(div().flex().items_center().child(crate::kit::label("Advanced", ui.t.text_3, 11.5)))
            .child(head(ui, "Full Disk Access").child(crate::kit::chip_tone(text, tone, &ui.t)))
            .child(crate::kit::muted(
                if granted {
                    "On: macOS won't ask about any folder. Pitwall and the agents it starts can read everything on this Mac. Turn it off in System Settings any time."
                } else {
                    "Skips the prompts, but lets Pitwall and every agent it starts read everything on this Mac, including mail and other apps' data."
                },
                &ui.t,
            ));
        if s.is_some() && !granted {
            if self.fda_open {
                fda = fda
                    .child(ui.steps(vec![
                        "Open Settings: Privacy & Security → Full Disk Access.".into_any_element(),
                        "Turn on Pitwall (or click + and choose it in Applications).".into_any_element(),
                        "Come back; this row updates by itself.".into_any_element(),
                    ]))
                    .child(
                        div().flex().gap(px(8.)).child(
                            crate::kit::text_button("fda-open", "Open Settings", BtnKind::Small, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                                if let Err(e) = permissions::open_full_disk_access() {
                                    v.error = Some(format!("Couldn't open System Settings: {e}"));
                                    cx.notify();
                                }
                            })),
                        ),
                    );
            } else {
                fda = fda.child(
                    div().flex().child(
                        crate::kit::text_button("fda-more", "Use Full Disk Access instead…", BtnKind::Link, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                            v.fda_open = true;
                            cx.notify();
                        })),
                    ),
                );
            }
        }
        vec![prompts, fda]
    }

    fn about(ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let facts = about::facts(cx);
        let text = about::as_text(&facts);
        let link = |id: &'static str, label: &'static str, url: &'static str| {
            crate::kit::text_button(id, label, BtnKind::Link, false, t).on_click(move |_, _, cx| crate::platform::files::open_url(cx, url))
        };
        section()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(about::board(t))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).child("PITWALL"))
                            .child(crate::kit::hint("You call the strategy. Agents drive.", t)),
                    ),
            )
            .child(
                div().flex().flex_col().gap(px(3.)).children(facts.into_iter().map(|(k, v)| {
                    div()
                        .flex()
                        .gap(px(10.))
                        .text_size(px(12.5))
                        .child(div().w(px(96.)).flex_none().text_color(t.text_3).child(k))
                        .child(div().flex_1().min_w_0().text_color(t.text).child(SharedString::from(v)))
                })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(14.))
                    .child(link("about-web", "Website", WEBSITE))
                    .child(link("about-src", "Source code", SOURCE))
                    .child(link("about-issues", "Report a problem", ISSUES))
                    .child(div().flex_1())
                    .child(
                        crate::kit::text_button("about-copy", "Copy details", BtnKind::GhostSm, false, t)
                            .tooltip(tooltip("For bug reports"))
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))),
                    ),
            )
            .child(
                div()
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(t.line)
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(crate::kit::hint("Pitwall is open source under the Apache License 2.0. Copyright 2026 the Pitwall authors.", t))
                    .child(crate::kit::hint("Built with GPUI (Apache-2.0). Fonts: Inter, Barlow Condensed and JetBrains Mono (SIL OFL 1.1). File icons: Material Icon Theme (MIT).", t))
                    .child(
                        div()
                            .flex()
                            .gap(px(14.))
                            .child(link("about-license", "Licence", LICENSE))
                            .child(link("about-third", "Third-party licences", THIRD_PARTY)),
                    ),
            )
    }
}
