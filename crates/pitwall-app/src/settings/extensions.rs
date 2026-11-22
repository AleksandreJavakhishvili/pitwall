//! Settings → General → Extensions (docs/spec/extensions.md): each folder
//! found in the extensions folder, with its switch (off until turned on),
//! or why it can't be used; "Look again" reads the folder again. One that
//! is on shows its adapter's state (with Restart), its settings drawn
//! natively (`url`: a field; `secret`: Set / Not set, filled from the
//! clipboard so it is never shown), what it runs and the variables it sets
//! on agents Pitwall starts.

use std::collections::{HashMap, HashSet};

use gpui::{div, prelude::*, px, Context, Div, Entity, SharedString, Subscription};
use pitwall_core::extensions::protocol::ConnState;
use pitwall_core::extensions::{Manifest, SettingDef, SettingKind};

use super::pages::{head, section};
use super::view::SettingsView;
use super::widgets::Ui;
use crate::extensions::hub::{self, Run};
use crate::kit::{BtnKind, InputEvent, TextInput, Tone};
use crate::extensions::actions::{self, Outcome, Planned};

/// The section's own state.
#[derive(Default)]
pub struct Section {
    /// `url` fields by (extension, key).
    inputs: HashMap<(String, String), Entity<TextInput>>,
    /// Why a setting wasn't saved, by (extension, key).
    notes: HashMap<(String, String), String>,
    /// Secrets being saved or removed.
    busy: HashSet<(String, String)>,
    watching: bool,
    /// The action waiting for the user to confirm its commands.
    confirm: Option<(String, String, Vec<Planned>)>,
    /// Actions running.
    running: HashSet<(String, String)>,
    /// How the last run of each action went.
    results: HashMap<(String, String), Vec<(String, Outcome)>>,
    _subs: Vec<Subscription>,
}

/// What a `url` field's text is saved as: nothing when empty, else the
/// web address it names.
pub fn url_value(text: &str) -> Result<Option<String>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    pitwall_proto::web::address(text).map(Some)
}

/// Each variable extension `m` sets, as `NAME=value`: its settings put in
/// (the per-agent placeholders left as they are), or why it isn't set.
pub fn env_preview(m: &Manifest, values: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    m.agent_env
        .iter()
        .map(|v| {
            let mut value = v.value.clone();
            for s in &m.settings {
                let p = format!("{}{}}}", pitwall_core::extensions::SETTING_PLACEHOLDER, s.key);
                if value.contains(&p) {
                    match values.get(&s.key).filter(|x| !x.trim().is_empty()) {
                        Some(x) => value = value.replace(&p, x),
                        None => return format!("{} (not set until {} is)", v.name, s.label),
                    }
                }
            }
            format!("{}={value}", v.name)
        })
        .collect()
}

/// The adapter's state in a few words, its tone, and what to do.
pub fn run_text(run: &Run, conn: Option<&(ConnState, String)>) -> (&'static str, Tone, String) {
    match run {
        Run::Off => ("off", Tone::Plain, String::new()),
        Run::Setup(why) => ("needs setup", Tone::Warn, why.clone()),
        Run::Starting => ("starting", Tone::Subtle, String::new()),
        Run::Stopped(why) => ("stopped", Tone::Warn, format!("{why} Fix that, then Restart.")),
        Run::Running => match conn {
            None | Some((ConnState::Connecting, _)) => ("connecting", Tone::Subtle, conn.map(|c| c.1.clone()).unwrap_or_default()),
            Some((ConnState::Ready, m)) => ("connected", Tone::Ok, m.clone()),
            Some((ConnState::SignedOut, m)) => ("signed out", Tone::Warn, if m.is_empty() { "Check its settings below.".into() } else { m.clone() }),
            Some((ConnState::Error, m)) => ("error", Tone::Warn, m.clone()),
        },
    }
}

impl SettingsView {
    /// Bench and debug builds: extension `ext`'s action `action` asked to
    /// confirm, as its button does (nothing runs).
    pub fn debug_action(&mut self, ext: &str, action: &str, cx: &mut Context<Self>) {
        let Some(m) = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == ext).cloned() else { return };
        let Some(a) = m.actions.iter().find(|a| a.id == action).cloned() else { return };
        let values = crate::ui_state::get(cx).extension(&m.id).values;
        let plan = actions::plan(&m, &a, &values, pitwall_core::shell::spawn_path().as_deref());
        self.exts.confirm = Some((m.id.clone(), a.id.clone(), plan));
        cx.notify();
        let me = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = me.update(cx, |v, _| v.scroll_to_end());
        });
    }

    /// The Extensions section.
    pub(super) fn extensions(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        if !self.exts.watching {
            self.exts.watching = true;
            let h = hub::hub(cx);
            self.exts._subs.push(cx.observe(&h, |_, _, cx| cx.notify()));
        }
        let found = crate::extensions::found(cx);
        let state = crate::ui_state::get(cx);
        let folder = crate::extensions::dir(cx).map(|d| pitwall_core::paths::tildify(&d.to_string_lossy()));
        let mut sec = section()
            .child(
                head(ui, "Extensions").child(
                    crate::kit::text_button("extensions-rescan", "Look again", BtnKind::Small, false, &ui.t).on_click(cx.listener(|_, _, _, cx| {
                        crate::extensions::rescan(cx);
                        cx.notify();
                    })),
                ),
            )
            .child(crate::kit::hint(
                match &folder {
                    Some(f) => format!("An extension is a folder with an extension.json, put in {f}. One that is on gets a section in the sidebar, and its adapter runs while it is on."),
                    None => "Extensions need Pitwall's data folder, which isn't in use here.".to_string(),
                },
                &ui.t,
            ));
        if found.is_empty() {
            return sec.child(crate::kit::muted("No extensions found.", &ui.t));
        }
        for f in found.iter() {
            let row = match &f.manifest {
                Ok(m) => {
                    let on = state.extension_on(&m.id);
                    let id = m.id.clone();
                    let top = div()
                        .id(SharedString::from(format!("extension-{}", m.id)))
                        .flex()
                        .gap(px(12.))
                        .items_start()
                        .cursor_pointer()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let id = id.clone();
                            crate::ui_state::update(cx, |s| *s = s.clone().set_extension(&id, !on));
                            cx.notify();
                        }))
                        .child(div().mt(px(5.)).child(crate::kit::icon(crate::extensions::icon_of(m), 14., ui.t.text_2)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(head(ui, &m.name))
                                .child(crate::kit::hint(m.description.clone().unwrap_or_else(|| format!("Folder {}", f.folder)), &ui.t)),
                        )
                        .child(div().mt(px(6.)).child(crate::kit::checkbox_box(on, false, 14., &ui.t)));
                    let mut col = div().flex().flex_col().gap(px(8.)).child(top);
                    if on {
                        col = col.child(self.extension_details(ui, m, cx));
                    }
                    col.into_any_element()
                }
                Err(why) => div()
                    .id(SharedString::from(format!("extension-bad-{}", f.folder)))
                    .flex()
                    .gap(px(12.))
                    .items_start()
                    .child(div().mt(px(5.)).child(crate::kit::icon("alert", 14., ui.t.text_3)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(head(ui, &f.folder))
                            .child(crate::kit::hint(format!("Can't be used: {why}."), &ui.t)),
                    )
                    .into_any_element(),
            };
            sec = sec.child(row);
        }
        sec
    }

    /// An extension that is on: its adapter, settings and variables.
    fn extension_details(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let mut col = div().ml(px(26.)).flex().flex_col().gap(px(8.));
        if m.adapter.is_some() {
            let h = hub::hub(cx);
            let (run, conn) = h.read(cx).get(&m.id).map(|s| (s.run.clone(), s.conn.clone())).unwrap_or_default();
            let (word, tone, text) = run_text(&run, conn.as_ref());
            let id = m.id.clone();
            let mut line = div().flex().items_center().gap(px(8.)).child(crate::kit::muted("Adapter", &ui.t)).child(crate::kit::chip_tone(word, tone, &ui.t)).child(div().flex_1());
            if matches!(run, Run::Running | Run::Stopped(_)) {
                line = line.child(
                    crate::kit::text_button(SharedString::from(format!("extension-restart-{}", m.id)), "Restart", BtnKind::Small, false, &ui.t).on_click(cx.listener(move |_, _, _, cx| {
                        let h = hub::hub(cx);
                        h.update(cx, |h, cx| h.restart(&id, cx));
                    })),
                );
            }
            col = col.child(line);
            if !text.is_empty() {
                col = col.child(crate::kit::hint(text, &ui.t));
            }
        }
        for s in &m.settings {
            col = col.child(match s.kind {
                SettingKind::Url => self.url_row(ui, m, s, cx),
                SettingKind::Secret => self.secret_row(ui, m, s, cx),
            });
        }
        if let Some(a) = &m.adapter {
            col = col.child(crate::kit::hint(format!("Runs {} in its folder.", a.command.join(" ")), &ui.t));
        }
        if !m.agent_env.is_empty() {
            let mut env = div().flex().flex_col().gap(px(2.)).child(crate::kit::hint("Sets on agents Pitwall starts while it is on ({project}: the project folder's name; {agent}: the agent's name; {agent_id}: its id; each made lowercase letters, digits, - and _):", &ui.t));
            let values = crate::ui_state::get(cx).extension(&m.id).values;
            for line in env_preview(m, &values) {
                env = env.child(crate::kit::mono_text(line, &ui.t));
            }
            col = col.child(env);
        }
        for a in &m.actions {
            col = col.child(self.action_row(ui, m, a, cx));
        }
        col
    }

    /// A setup action: its button and description; then the commands to
    /// confirm, its progress, or how it went.
    fn action_row(&mut self, ui: &Ui, m: &Manifest, a: &pitwall_core::extensions::Action, cx: &mut Context<Self>) -> Div {
        let k = (m.id.clone(), a.id.clone());
        let running = self.exts.running.contains(&k);
        let asking = self.exts.confirm.as_ref().is_some_and(|(e, id, _)| *e == k.0 && *id == k.1);
        let open = {
            let (m, a) = (m.clone(), a.clone());
            crate::kit::text_button(SharedString::from(format!("extension-action-{}-{}", m.id, a.id)), a.label.clone(), BtnKind::Small, running || asking, &ui.t).on_click(cx.listener(move |v, _, _, cx| {
                let values = crate::ui_state::get(cx).extension(&m.id).values;
                let plan = actions::plan(&m, &a, &values, pitwall_core::shell::spawn_path().as_deref());
                v.exts.results.remove(&(m.id.clone(), a.id.clone()));
                v.exts.confirm = Some((m.id.clone(), a.id.clone(), plan));
                cx.notify();
            }))
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().flex().items_center().gap(px(8.)).child(open).child(div().flex_1().min_w_0().child(crate::kit::hint(a.description.clone(), &ui.t))));
        if asking {
            let plan = self.exts.confirm.as_ref().map(|c| c.2.clone()).unwrap_or_default();
            let runs = plan.iter().any(|p| p.skip.is_none());
            let mut card = div()
                .id(SharedString::from(format!("extension-confirm-{}-{}", m.id, a.id)))
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(ui.t.line_strong)
                .bg(ui.t.surface_2)
                .child(crate::kit::muted(if runs { "These commands will run, in this order, when you press Run:" } else { "Nothing can run:" }, &ui.t));
            for p in &plan {
                card = card.child(match &p.skip {
                    None => crate::kit::mono_text(p.line(), &ui.t),
                    Some(why) => div().flex().flex_col().opacity(0.6).child(crate::kit::mono_text(p.line(), &ui.t)).child(crate::kit::hint(format!("Skipped: {why}."), &ui.t)),
                });
            }
            if plan.iter().any(|p| p.optional && p.skip.is_none()) {
                card = card.child(crate::kit::hint("A failing remove step is fine: there was nothing to remove.", &ui.t));
            }
            let (m2, a2) = (m.clone(), a.clone());
            let mut buttons = div().flex().gap(px(8.)).pt(px(4.)).child(div().flex_1());
            buttons = buttons.child(crate::kit::text_button(SharedString::from(format!("extension-cancel-{}-{}", m.id, a.id)), "Cancel", BtnKind::GhostSm, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                v.exts.confirm = None;
                cx.notify();
            })));
            if runs {
                buttons = buttons.child(crate::kit::text_button(SharedString::from(format!("extension-run-{}-{}", m.id, a.id)), "Run", BtnKind::Small, false, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.run_action(&m2, &a2, cx))));
            }
            col = col.child(card.child(buttons));
        } else if running {
            col = col.child(crate::kit::hint("Running…", &ui.t));
        } else if let Some(results) = self.exts.results.get(&k) {
            let mut list = div().flex().flex_col().gap(px(2.));
            for (line, o) in results {
                let (word, color) = match o {
                    Outcome::Done => ("Done".to_string(), ui.t.green),
                    Outcome::Failed(why) => (format!("Failed: {why}"), ui.t.amber),
                    Outcome::Skipped(why) => (format!("Skipped: {why}"), ui.t.text_3),
                };
                list = list.child(div().flex().flex_col().child(crate::kit::mono_text(line.clone(), &ui.t)).child(div().text_size(px(12.)).text_color(color).child(word)));
            }
            col = col.child(list);
        }
        col
    }

    /// Run the confirmed action off the UI thread.
    fn run_action(&mut self, m: &Manifest, a: &pitwall_core::extensions::Action, cx: &mut Context<Self>) {
        let Some((_, _, plan)) = self.exts.confirm.take() else { return };
        let k = (m.id.clone(), a.id.clone());
        self.exts.running.insert(k.clone());
        cx.notify();
        let home = pitwall_core::paths::home();
        let job = cx.background_executor().spawn(async move {
            let out = actions::run(&plan, &home, actions::STEP_TIMEOUT);
            plan.iter().map(Planned::line).zip(out).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let results = job.await;
            let _ = this.update(cx, |v, cx| {
                v.exts.running.remove(&k);
                v.exts.results.insert(k, results);
                cx.notify();
            });
        })
        .detach();
    }

    fn url_input(&mut self, m: &Manifest, s: &SettingDef, cx: &mut Context<Self>) -> Entity<TextInput> {
        let k = (m.id.clone(), s.key.clone());
        if let Some(i) = self.exts.inputs.get(&k) {
            return i.clone();
        }
        let text = crate::ui_state::get(cx).extension(&m.id).values.get(&s.key).cloned().unwrap_or_default();
        let placeholder = s.placeholder.clone().unwrap_or_else(|| "https://".into());
        let input = cx.new(|cx| TextInput::new(cx, &text, &placeholder).mono());
        let key = k.clone();
        let sub = cx.subscribe(&input, move |v, input, e: &InputEvent, cx| {
            if !matches!(e, InputEvent::Blur | InputEvent::Submit) {
                return;
            }
            let text = input.read(cx).text().to_string();
            match url_value(&text) {
                Ok(val) => {
                    v.exts.notes.remove(&key);
                    let (id, k) = key.clone();
                    crate::ui_state::update(cx, |ui| {
                        *ui = std::mem::take(ui).with_extension(&id, |e| match val {
                            Some(val) => {
                                e.values.insert(k, val);
                            }
                            None => {
                                e.values.remove(&k);
                            }
                        });
                    });
                }
                Err(why) => {
                    v.exts.notes.insert(key.clone(), why);
                }
            }
            cx.notify();
        });
        self.exts._subs.push(sub);
        self.exts.inputs.insert(k, input.clone());
        input
    }

    fn label(ui: &Ui, s: &SettingDef) -> Div {
        let text = if s.required { s.label.clone() } else { format!("{} (optional)", s.label) };
        div().w(px(120.)).flex_none().child(crate::kit::muted(text, &ui.t))
    }

    fn url_row(&mut self, ui: &Ui, m: &Manifest, s: &SettingDef, cx: &mut Context<Self>) -> Div {
        let input = self.url_input(m, s, cx);
        let k = (m.id.clone(), s.key.clone());
        let mut col = div().flex().flex_col().gap(px(4.)).child(div().flex().items_center().gap(px(8.)).child(Self::label(ui, s)).child(div().flex_1().min_w_0().child(input)));
        if let Some(n) = self.exts.notes.get(&k) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else if let Some(h) = &s.hint {
            col = col.child(crate::kit::hint(h.clone(), &ui.t));
        }
        col
    }

    fn secret_row(&mut self, ui: &Ui, m: &Manifest, s: &SettingDef, cx: &mut Context<Self>) -> Div {
        let k = (m.id.clone(), s.key.clone());
        let set = crate::ui_state::get(cx).extension(&m.id).secrets_set.contains(&s.key);
        let busy = self.exts.busy.contains(&k);
        let paste = {
            let k = k.clone();
            crate::kit::text_button(SharedString::from(format!("extension-secret-paste-{}-{}", m.id, s.key)), "Paste from clipboard", BtnKind::Small, busy, &ui.t).on_click(cx.listener(move |v, _, _, cx| {
                let text = crate::platform::files::paste_text(cx).map(|t| t.trim().to_string()).unwrap_or_default();
                if text.is_empty() {
                    v.exts.notes.insert(k.clone(), "Copy the secret first: the clipboard has no text.".into());
                    cx.notify();
                    return;
                }
                if text.chars().any(char::is_control) || text.len() > 4096 {
                    v.exts.notes.insert(k.clone(), "The clipboard doesn't hold a secret (it has line breaks or is too long).".into());
                    cx.notify();
                    return;
                }
                v.secret_change(k.clone(), Some(text), cx);
            }))
        };
        let mut line = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(Self::label(ui, s))
            .child(crate::kit::chip_tone(if busy { "saving…" } else if set { "set" } else { "not set" }, if set { Tone::Ok } else { Tone::Plain }, &ui.t))
            .child(div().flex_1())
            .child(paste);
        if set {
            let k = k.clone();
            line = line.child(
                crate::kit::text_button(SharedString::from(format!("extension-secret-remove-{}-{}", m.id, s.key)), "Remove", BtnKind::GhostSm, busy, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.secret_change(k.clone(), None, cx))),
            );
        }
        let mut col = div().flex().flex_col().gap(px(4.)).child(line);
        if let Some(n) = self.exts.notes.get(&k) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else {
            let base = "Kept in the system's secure store; Pitwall's files only record that it is set.";
            col = col.child(crate::kit::hint(match &s.hint { Some(h) => format!("{h} {base}"), None => base.to_string() }, &ui.t));
        }
        col
    }

    fn secret_change(&mut self, k: (String, String), secret: Option<String>, cx: &mut Context<Self>) {
        self.exts.busy.insert(k.clone());
        self.exts.notes.remove(&k);
        cx.notify();
        let this = cx.entity().downgrade();
        let (id, key) = (k.0.clone(), k.1.clone());
        let done = move |err: Option<String>, cx: &mut gpui::App| {
            let _ = this.update(cx, |v, cx| {
                v.exts.busy.remove(&k);
                if let Some(e) = err {
                    v.exts.notes.insert(k.clone(), e);
                }
                cx.notify();
            });
        };
        match secret {
            Some(s) => crate::extensions::set_secret(&id, &key, s, cx, done),
            None => crate::extensions::remove_secret(&id, &key, cx, done),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_fields_and_states_say_what_to_do() {
        assert_eq!(url_value("  "), Ok(None));
        assert_eq!(url_value("inbox.example.com").unwrap().as_deref(), Some("https://inbox.example.com"));
        assert!(url_value("javascript:alert(1)").is_err());
        let (w, _, t) = run_text(&Run::Stopped("It exited.".into()), None);
        assert_eq!(w, "stopped");
        assert!(t.contains("Restart"));
        let (w, _, t) = run_text(&Run::Running, Some(&(ConnState::SignedOut, String::new())));
        assert_eq!(w, "signed out");
        assert!(!t.is_empty());
    }

    /// Settings shows exactly what each variable will be: for each
    /// extension in the repo whose `test/pitwall.json` says so, before and
    /// after its settings are filled in.
    #[test]
    fn the_variables_shown_are_the_ones_set() {
        let mut checked = 0;
        for (dir, m, fixture) in crate::extensions::shipped_all() {
            let env = &fixture["env"];
            if env.is_null() {
                continue;
            }
            let lines = |v: &serde_json::Value| -> Vec<String> { v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect() };
            let mut values = std::collections::BTreeMap::new();
            assert_eq!(env_preview(&m, &values), lines(&env["unset"]), "{}", dir.display());
            for (k, v) in env["values"].as_object().unwrap() {
                values.insert(k.clone(), v.as_str().unwrap().to_string());
            }
            assert_eq!(env_preview(&m, &values), lines(&env["set"]), "{}", dir.display());
            checked += 1;
        }
        assert!(checked > 0, "no extension says what its variables are");
    }
}
