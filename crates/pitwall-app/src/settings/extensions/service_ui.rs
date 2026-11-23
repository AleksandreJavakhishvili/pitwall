//! An extension's service on its card (docs/extensions/service.md): its
//! checklist steps (install, confirm and start, first sign-in), the row
//! with its state and Start, Stop, Restart and Log, the card that asks to
//! confirm its exact commands, and its first run again under More.
//! Everything it does goes through `crate::extensions::service`, never
//! back into this view from its own click.

use gpui::{div, prelude::*, px, AnyElement, Context, Div, SharedString};
use pitwall_core::extensions::service as core;
use pitwall_core::extensions::Manifest;

use super::super::view::SettingsView;
use super::super::widgets::Ui;
use crate::extensions::actions;
use crate::extensions::service::{self, FirstRun, State};
use crate::kit::BtnKind;
use pitwall_core::accounts::Account;

/// What the checklist needs of a service: its program is there (and new
/// enough), and it is up (Pitwall's or another's).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServiceFacts {
    pub installed: bool,
    pub up: bool,
}

impl ServiceFacts {
    /// `m`'s service as it stands.
    pub fn of(m: &Manifest, cx: &gpui::App) -> ServiceFacts {
        if m.service.is_none() {
            return ServiceFacts::default();
        }
        let svc = service::try_services(cx);
        let st = svc.as_ref().and_then(|s| s.read(cx).get(&m.id));
        let state = st.map(|s| s.state.clone()).unwrap_or_default();
        let installed = match &state {
            State::NotInstalled(_) | State::TooOld { .. } => false,
            State::Elsewhere(_) | State::Running { .. } | State::Starting => true,
            _ => st.and_then(|s| s.installed).unwrap_or(false),
        };
        ServiceFacts { installed, up: state.up() }
    }
}

/// `m`'s service uses setting `key` in its command or address.
pub fn service_uses(m: &Manifest, key: &str) -> bool {
    let Some(s) = &m.service else { return false };
    let p = format!("{{setting:{key}}}");
    let ready = match &s.ready {
        core::Ready::Listen(t) => t.contains(&p),
        _ => false,
    };
    ready || s.run.iter().any(|a| a.contains(&p)) || s.env.iter().any(|e| matches!(&e.from, core::EnvFrom::Value(v) if v.contains(&p)))
}

/// Since when, as the clock here shows it.
fn since(at: std::time::SystemTime) -> String {
    let t: chrono::DateTime<chrono::Local> = at.into();
    t.format("%H:%M").to_string()
}

impl SettingsView {
    fn svc_button(ui: &Ui, sel: String, text: &'static str, kind: BtnKind, disabled: bool) -> gpui::Stateful<Div> {
        let s2 = sel.clone();
        crate::kit::text_button(SharedString::from(sel), text, kind, disabled, &ui.t).debug_selector(move || s2)
    }

    /// The install step: its program is there and new enough; else its
    /// install action (to confirm as any), started again once it ends.
    pub(super) fn service_install_step(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, done: bool, cx: &mut Context<Self>) -> Div {
        let Some(s) = m.service.clone() else { return div() };
        let st = service::try_services(cx).and_then(|x| x.read(cx).get(&m.id).map(|v| (v.state.clone(), v.version.clone()))).unwrap_or_default();
        let install = s.install.as_ref().and_then(|id| m.actions.iter().find(|a| a.id == *id)).cloned();
        let (title, text, warn) = match &st.0 {
            State::TooOld { found, need } => (format!("Update {}", s.label), format!("{} {found}; this extension needs {need} or later.", s.label), true),
            State::NotInstalled(p) => (format!("Install {}", s.label), format!("“{p}” isn't installed for your login shell yet."), false),
            _ if done => (format!("{} is installed", s.label), st.1.as_ref().map(|v| format!("Version {v}.")).unwrap_or_default(), false),
            _ => (format!("Install {}", s.label), format!("“{}” isn't installed for your login shell yet.", s.run[0]), false),
        };
        let mut head = div().flex().items_start().gap(px(10.)).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(Self::step_title(ui, title))
                .when(!text.is_empty(), |d| d.child(div().debug_selector({
                    let sel = format!("extension-service-install-text-{}", m.id);
                    move || sel
                }).text_size(px(12.)).text_color(if warn { ui.t.amber } else { ui.t.text_3 }).child(text))),
        );
        let mut col = div().flex().flex_col().gap(px(6.));
        match &install {
            Some(a) if !done => {
                let key = super::run_key(a, c);
                let state = actions::runs(cx).read(cx).get(&m.id, &key);
                let running = state.as_ref().is_some_and(|s| !s.over);
                let asking = self.exts.confirm.as_ref().is_some_and(|(e, id, _)| *e == m.id && *id == key);
                let label = if matches!(st.0, State::TooOld { .. }) { "Update…" } else { "Install…" };
                let a2 = a.clone();
                let m2 = m.clone();
                head = head.child(Self::svc_button(ui, format!("extension-service-install-{}", m.id), label, BtnKind::Small, running || asking).on_click(cx.listener(move |v, _, _, cx| v.ask_action(&m2, &a2, cx))));
                col = col.child(head);
                if asking {
                    col = col.child(self.confirm_card(ui, m, a, cx));
                } else if let Some(run) = state {
                    col = col.child(self.run_view(ui, m, &key, &run, cx));
                }
            }
            None if !done => {
                let id = m.id.clone();
                head = head.child(Self::svc_button(ui, format!("extension-service-check-{}", m.id), "Check again", BtnKind::GhostSm, false).on_click(cx.listener(move |_, _, _, cx| {
                    let id = id.clone();
                    cx.defer(move |cx| service::recheck(&id, cx));
                })));
                col = col.child(head).child(crate::kit::hint("Install it yourself, then press Check again.", &ui.t));
            }
            _ => col = col.child(head),
        }
        col
    }

    /// The start step: the card to confirm its commands, else its state
    /// with Start, Stop, Restart.
    pub(super) fn service_start_step(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, cx: &mut Context<Self>) -> Div {
        let Some(s) = m.service.clone() else { return div() };
        let state = service::try_services(cx).and_then(|x| x.read(cx).get(&m.id).map(|v| v.state.clone())).unwrap_or_default();
        if let State::NeedsConfirm { changed } = state {
            return div().flex().flex_col().gap(px(6.)).child(Self::step_title(ui, format!("Confirm and start {}", s.label))).child(self.service_confirm_card(ui, m, c, changed, cx));
        }
        let (title, text, warn): (String, String, bool) = match &state {
            State::Running { since: at } => (format!("Running since {}", since(*at)), String::new(), false),
            State::Starting | State::Checking => ("Starting…".into(), String::new(), false),
            State::Restarting { attempt } => ("Starting again…".into(), format!("It stopped by itself ({attempt} of {} tries).", service::BACKOFF.len()), true),
            State::Elsewhere(a) => ("Running elsewhere".into(), format!("Something else is already listening on {a}. Pitwall uses it, and never starts or stops it."), false),
            State::Stopped(why) => ("Stopped".into(), why.clone(), true),
            State::UserStopped => ("Stopped".into(), "You stopped it: it isn't started until you press Start.".into(), false),
            State::Updating => ("Stopped for the update".into(), "The extension is being updated: it starts again once its new files are in place.".into(), false),
            State::NeedsSetting(l) => (format!("Start {}", s.label), format!("Set {l} above first."), false),
            State::BadAddress(why) => ("Not started".into(), why.clone(), true),
            State::NotInstalled(_) | State::TooOld { .. } => (format!("Start {}", s.label), "It starts once the step above is done.".into(), false),
            State::Off | State::NeedsConfirm { .. } => (format!("Start {}", s.label), String::new(), false),
        };
        let mut head = div().flex().items_center().gap(px(8.)).child(Self::step_title(ui, title)).child(div().flex_1());
        let id = m.id.clone();
        match &state {
            State::Running { .. } | State::Starting | State::Restarting { .. } => {
                let i2 = id.clone();
                head = head.child(Self::svc_button(ui, format!("extension-service-restart-{id}"), "Restart", BtnKind::GhostSm, false).on_click(cx.listener(move |_, _, _, cx| {
                    let id = i2.clone();
                    cx.defer(move |cx| service::recheck(&id, cx));
                })));
                head = head.child(Self::svc_button(ui, format!("extension-service-stop-{id}"), "Stop", BtnKind::GhostSm, false).on_click(cx.listener(move |_, _, _, cx| {
                    let id = id.clone();
                    cx.defer(move |cx| service::set_stopped(&id, true, cx));
                })));
            }
            State::Stopped(_) | State::UserStopped | State::BadAddress(_) => {
                head = head.child(Self::svc_button(ui, format!("extension-service-start-{id}"), "Start", BtnKind::Small, false).on_click(cx.listener(move |_, _, _, cx| {
                    let id = id.clone();
                    cx.defer(move |cx| {
                        service::set_stopped(&id, false, cx);
                        service::recheck(&id, cx);
                    });
                })));
            }
            State::Elsewhere(_) => {
                head = head.child(Self::svc_button(ui, format!("extension-service-check-{id}"), "Check again", BtnKind::GhostSm, false).on_click(cx.listener(move |_, _, _, cx| {
                    let id = id.clone();
                    cx.defer(move |cx| service::recheck(&id, cx));
                })));
            }
            _ => {}
        }
        let sel = format!("extension-service-state-{}", m.id);
        let mut col = div().debug_selector(move || sel).flex().flex_col().gap(px(3.)).child(head);
        if !text.is_empty() {
            let sel = format!("extension-service-why-{}", m.id);
            col = col.child(div().debug_selector(move || sel).text_size(px(12.)).line_height(gpui::relative(1.45)).text_color(if warn { ui.t.amber } else { ui.t.text_3 }).child(text));
        }
        col
    }

    /// Its commands as they will run, to confirm once; after an update
    /// that changed them, what changed is said first.
    fn service_confirm_card(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, changed: bool, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let s = m.service.clone().unwrap_or_else(|| unreachable!());
        let values = c.map(|c| c.values.clone()).unwrap_or_default();
        let data = service::data_dir(&m.id, cx).map(|d| service::shown(&d)).unwrap_or_default();
        let cmds = core::commands(m, &s, &values, &data);
        let sel = format!("extension-service-confirm-{}", m.id);
        let mut card = div()
            .id(SharedString::from(sel.clone()))
            .debug_selector(move || sel)
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(ui.t.line_strong)
            .bg(ui.t.surface_2);
        if changed {
            card = card.child(div().pb(px(2.)).text_size(px(12.)).text_color(ui.t.amber).child(format!("An update changed what it runs, so it was stopped. {}", pitwall_core::extensions::install::SERVICE_RISK)));
        }
        card = card.child(div().pb(px(2.)).text_size(px(12.)).text_color(ui.t.text_2).child("Pitwall will keep this running while the extension is on:"));
        card = card.child(crate::kit::mono_text(pitwall_core::extensions::plan::line(&cmds.run), &ui.t));
        for (title, lines) in core::lines(m, &s, &values, &data).into_iter().skip(1) {
            card = card.child(div().pt(px(4.)).text_size(px(12.)).text_color(ui.t.text_2).child(title));
            for l in lines {
                card = card.child(crate::kit::mono_text(l, &ui.t));
            }
        }
        if let Some(why) = &cmds.missing {
            card = card.child(crate::kit::hint(format!("It starts once {why} is set."), &ui.t));
        }
        card = card.child(crate::kit::hint("It runs as you, with your files, and stops when you turn the extension off or quit Pitwall.", &ui.t));
        let m2 = m.clone();
        let buttons = div().flex().gap(px(8.)).pt(px(4.)).child(div().flex_1()).child(Self::svc_button(ui, format!("extension-service-confirm-yes-{}", m.id), "Confirm and start", BtnKind::Small, false).on_click(cx.listener(move |_, _, _, cx| {
            let m = m2.clone();
            cx.defer(move |cx| service::confirm(&m, cx));
        })));
        card.child(buttons)
    }

    /// The first-run step: its button while the service is fresh (it
    /// must be running), then how it went.
    pub(super) fn service_first_step(&mut self, ui: &Ui, m: &Manifest, done: bool, cx: &mut Context<Self>) -> Div {
        let Some(f) = m.service.as_ref().and_then(|s| s.first_run.clone()) else { return div() };
        let svc = service::try_services(cx).and_then(|x| x.read(cx).get(&m.id).map(|v| (v.state.clone(), v.first_run.clone())));
        let (state, run) = svc.unwrap_or_default();
        let running = matches!(state, State::Running { .. });
        let busy = run == Some(FirstRun::Running);
        let mut head = div().flex().items_start().gap(px(10.)).child(Self::step_title(ui, f.label.clone())).child(div().flex_1());
        let text = match (&run, done) {
            (Some(FirstRun::Running), _) => "Running…".to_string(),
            (Some(FirstRun::Done(what)), _) => what.clone(),
            (Some(FirstRun::Failed(why)), _) => why.clone(),
            (None, true) => "Done before. It is under More if you need it again.".to_string(),
            (None, false) if matches!(state, State::Elsewhere(_)) => "It is for a server Pitwall started; the one running elsewhere is set up where it runs.".to_string(),
            (None, false) if !running => "Once it is running.".to_string(),
            (None, false) => if f.opens { "Opens a one-time link in your browser.".to_string() } else { String::new() },
        };
        if !done {
            let m2 = m.clone();
            head = head.child(Self::svc_button(ui, format!("extension-service-first-{}", m.id), "Run", BtnKind::Small, !running || busy).on_click(cx.listener(move |_, _, _, cx| {
                let m = m2.clone();
                cx.defer(move |cx| {
                    let s = service::services(cx);
                    s.update(cx, |s, cx| s.first_run(&m, cx));
                });
            })));
        }
        let failed = matches!(run, Some(FirstRun::Failed(_)));
        let sel = format!("extension-service-first-text-{}", m.id);
        div().flex().flex_col().gap(px(3.)).child(head).when(!text.is_empty(), |d| d.child(div().debug_selector(move || sel).text_size(px(12.)).line_height(gpui::relative(1.45)).text_color(if failed { ui.t.amber } else { ui.t.text_3 }).child(text)))
    }

    /// Under More, once it is done: the first run again, asked first
    /// (with what running it again does).
    pub(super) fn service_first_again(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<AnyElement> {
        let f = m.service.as_ref()?.first_run.clone()?;
        if crate::ui_state::get(cx).extension(&m.id).service.fresh {
            return None;
        }
        let svc = service::try_services(cx).and_then(|x| x.read(cx).get(&m.id).map(|v| (v.state.clone(), v.first_run.clone())));
        let (state, run) = svc.unwrap_or_default();
        let running = matches!(state, State::Running { .. });
        let asking = self.exts.first_again.as_deref() == Some(m.id.as_str());
        let id = m.id.clone();
        let mut head = div()
            .flex()
            .items_start()
            .gap(px(10.))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(Self::step_title(ui, f.label.clone())).child(crate::kit::hint(f.again.clone().unwrap_or_else(|| "Runs its first-run command again.".into()), &ui.t)));
        head = head.child(Self::svc_button(ui, format!("extension-service-again-{}", m.id), "Run…", BtnKind::Small, !running || asking || run == Some(FirstRun::Running)).on_click(cx.listener(move |v, _, _, cx| {
            v.exts.first_again = Some(id.clone());
            cx.notify();
        })));
        let mut col = div().flex().flex_col().gap(px(6.)).child(head);
        if asking {
            let m2 = m.clone();
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(ui.t.line_strong)
                    .bg(ui.t.surface_2)
                    .child(div().text_size(px(12.)).text_color(ui.t.text_2).child(format!("Run “{}” again?", f.label)))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(div().flex_1())
                            .child(Self::svc_button(ui, format!("extension-service-again-cancel-{}", m.id), "Cancel", BtnKind::GhostSm, false).on_click(cx.listener(|v, _, _, cx| {
                                v.exts.first_again = None;
                                cx.notify();
                            })))
                            .child(Self::svc_button(ui, format!("extension-service-again-yes-{}", m.id), "Run", BtnKind::Small, false).on_click(cx.listener(move |v, _, _, cx| {
                                v.exts.first_again = None;
                                let m = m2.clone();
                                cx.defer(move |cx| {
                                    let s = service::services(cx);
                                    s.update(cx, |s, cx| s.first_run(&m, cx));
                                });
                                cx.notify();
                            }))),
                    ),
            );
        } else if let Some(r) = run {
            let (text, warn) = match r {
                FirstRun::Running => ("Running…".to_string(), false),
                FirstRun::Done(w) => (w, false),
                FirstRun::Failed(w) => (w, true),
            };
            let sel = format!("extension-service-first-text-{}", m.id);
            col = col.child(div().debug_selector(move || sel).text_size(px(12.)).text_color(if warn { ui.t.amber } else { ui.t.text_3 }).child(text));
        }
        Some(col.into_any_element())
    }

    /// "What this extension does": what its service runs, its variables
    /// and its data folder.
    pub(super) fn service_disclosure(ui: &Ui, m: &Manifest, values: &std::collections::BTreeMap<String, String>, cx: &gpui::App) -> Option<Div> {
        let s = m.service.as_ref()?;
        let data = service::data_dir(&m.id, cx).map(|d| service::shown(&d)).unwrap_or_default();
        let mut col = div().flex().flex_col().gap(px(10.));
        for (title, lines) in core::lines(m, s, values, &data) {
            let mut b = div().flex().flex_col().gap(px(3.)).child(div().text_size(px(12.)).text_color(ui.t.text_2).child(title));
            for l in lines {
                b = b.child(crate::kit::mono_text(l, &ui.t));
            }
            col = col.child(b);
        }
        Some(col)
    }

    /// The install action of `m`'s service: it is a step of its own, not
    /// listed under More.
    pub(super) fn is_service_install(m: &Manifest, action: &str) -> bool {
        m.service.as_ref().and_then(|s| s.install.as_deref()) == Some(action)
    }

    /// The remove card's choice for `m`'s data folder (kept unless ticked).
    pub(super) fn service_remove_choice(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<AnyElement> {
        m.service.as_ref()?;
        let data = service::data_dir(&m.id, cx)?;
        let ticked = self.exts.remove_data.contains(&m.id);
        let id = m.id.clone();
        let sel = format!("extension-remove-data-{}", m.id);
        let row = div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(crate::kit::toggle(SharedString::from(sel.clone()), ticked, &ui.t).debug_selector(move || sel).on_click(cx.listener(move |v, _, _, cx| {
                if !v.exts.remove_data.remove(&id) {
                    v.exts.remove_data.insert(id.clone());
                }
                cx.notify();
            })))
            .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(ui.t.text_2).child(format!("Also delete its service's data in {}. Off: it is kept there.", service::shown(&data))));
        Some(row.into_any_element())
    }
}
