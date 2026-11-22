//! Settings → Extensions (docs/spec/extensions.md): a page of its own.
//! Installed: one card per folder found in the extensions folder: icon,
//! name, version, a status pill and the switch (off until turned on);
//! Update when Pitwall ships a newer copy, and Remove; or why it can't be
//! used. A card that is on unfolds into a numbered checklist built from the
//! manifest (its settings, its setup actions, then "Connected"), each step
//! ticked as it is done, with its field or button right there; a setup
//! action shows its steps live while it runs. "What this extension does on
//! your computer" says what it runs and sets, folded. Available: the
//! extensions Pitwall ships that aren't installed, each with Install.
//!
//! **Connections** (docs/spec/profiles.md): an extension with settings
//! can have several, each its own settings, secrets and setup actions.
//! With one, the card looks as it always did (the checklist is that
//! connection's); "Add connection" in the footer names another. With two
//! or more, a row of tabs over the checklist picks which one it shows,
//! each with its state as a dot and the default marked; Rename, Make
//! default and Remove are beside them.

use std::collections::{HashMap, HashSet};

use gpui::{div, prelude::*, px, AnyElement, Context, Div, Entity, FontWeight, SharedString, Stateful, Subscription};
use pitwall_core::extensions::install::Status;
use pitwall_core::extensions::protocol::ConnState;
use pitwall_core::extensions::{Action, Manifest, SettingDef, SettingKind};

use super::pages::{head, section};
use super::view::{Part, SettingsView};
use super::widgets::Ui;
use crate::extensions::actions::{self, Outcome, Planned, RunState, StepState};
use crate::extensions::catalog::{self, Shipped};
use crate::extensions::hub::{self, Run};
use crate::extensions::connections;
use crate::kit::{BtnKind, InputEvent, TextInput, Tone};
use crate::main_screen::workspace::ExtensionPrefs;
use pitwall_core::accounts::Account;

/// The page's own state.
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
    pub(super) confirm: Option<(String, String, Vec<Planned>)>,
    /// The extensions Pitwall ships, with whether each installed one is
    /// as Pitwall installed it; read again when the folder is
    /// (`found`'s list changes).
    shipped: Option<(usize, Vec<Shipped>, HashMap<String, bool>)>,
    /// Being installed, updated or removed.
    working: HashSet<String>,
    /// Remove asked to confirm.
    confirm_remove: Option<String>,
    /// Update of a folder changed by hand asked to confirm.
    confirm_replace: Option<String>,
    /// Why Install, Update or Remove failed, by extension.
    failed: HashMap<String, String>,
    /// Cards the user folded or unfolded (else: open while on).
    open: HashMap<String, bool>,
    /// "What this extension does on your computer" unfolded.
    disclosed: HashSet<String>,
    /// The connection each card shows (else its default, else its first).
    conn: HashMap<String, String>,
    /// A connection being named: (extension, the connection renamed or
    /// `None` for a new one, its field).
    naming: Option<(String, Option<String>, Entity<TextInput>)>,
    /// Why naming failed.
    naming_note: Option<String>,
    /// Removing a connection asked to confirm: (extension, connection,
    /// the one to make the default when it is the default).
    remove_conn: Option<(String, String, Option<String>)>,
    _subs: Vec<Subscription>,
}

/// What a `url` field's text is saved as: nothing when empty, else the
/// web address it names.
pub fn url_value(text: &str) -> Result<Option<String>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    pitwall_proto::web::address(text).map(Some).map_err(|_| "That isn't a web address (like https://example.com).".to_string())
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
        Run::Starting => ("connecting", Tone::Subtle, String::new()),
        Run::Stopped(why) => ("stopped", Tone::Warn, format!("{why} Fix that, then try again.")),
        Run::Running => match conn {
            None | Some((ConnState::Connecting, _)) => ("connecting", Tone::Subtle, conn.map(|c| c.1.clone()).unwrap_or_default()),
            Some((ConnState::Ready, m)) => ("connected", Tone::Ok, m.clone()),
            Some((ConnState::SignedOut, m)) => ("signed out", Tone::Warn, if m.is_empty() { "Check its settings above.".into() } else { m.clone() }),
            Some((ConnState::Error, m)) => ("error", Tone::Warn, m.clone()),
        },
    }
}

/// One step of an extension's checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Setting(String),
    Action(String),
    Connected,
}

/// The checklist of `m` with `prefs`, `connected` saying whether its
/// adapter is connected: each step and whether it is done. A setting that
/// isn't required counts as done.
pub fn checklist(m: &Manifest, prefs: &ExtensionPrefs, connected: bool) -> Vec<(Step, bool)> {
    let mut out: Vec<(Step, bool)> = m
        .settings
        .iter()
        .map(|s| {
            let set = match s.kind {
                SettingKind::Url => prefs.values.get(&s.key).is_some_and(|v| !v.trim().is_empty()),
                SettingKind::Secret => prefs.secrets_set.contains(&s.key),
            };
            (Step::Setting(s.key.clone()), set || !s.required)
        })
        .collect();
    out.extend(m.actions.iter().filter(|a| a.setup).map(|a| (Step::Action(a.id.clone()), prefs.actions_done.contains(&a.id))));
    if m.adapter.is_some() {
        out.push((Step::Connected, connected));
    }
    out
}

/// A finished run of `m`'s `action` (for connection `conn`), kept: one
/// where a step ran and none failed ticks its checklist step; any other
/// unticks it. One that uses a setting is kept for its connection.
pub fn record_run(m: &Manifest, action: &str, conn: Option<&str>, st: &RunState, cx: &mut gpui::App) {
    let ok = !st.failed() && !st.cancelled && st.steps.iter().any(|(_, s)| matches!(s, StepState::Ended(Outcome::Done)));
    connections::record_action(m, action, conn, ok, cx);
}

/// What a run of `a` is kept under: per connection when it uses a
/// setting (`<action>@<connection>`), else the action's id.
pub fn run_key(a: &Action, conn: Option<&Account>) -> String {
    match conn {
        Some(c) if connections::uses_setting(a) => format!("{}@{}", a.id, c.id),
        _ => a.id.clone(),
    }
}

/// Where the system keeps secrets, in the user's words.
fn secure_store() -> &'static str {
    if cfg!(target_os = "macos") {
        "Saved in your keychain"
    } else if cfg!(windows) {
        "Saved in Windows Credential Manager"
    } else {
        "Saved in your keyring"
    }
}

impl SettingsView {
    /// Bench and debug builds: extension `ext`'s action `action` asked to
    /// confirm, as its button does (nothing runs).
    pub fn debug_action(&mut self, ext: &str, action: &str, cx: &mut Context<Self>) {
        let Some(m) = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == ext).cloned() else { return };
        let Some(a) = m.actions.iter().find(|a| a.id == action).cloned() else { return };
        self.ask_action(&m, &a, cx);
    }

    /// Bench and debug builds: `ext`'s `action` run as confirmed with
    /// `steps` (program and arguments, `|` between steps) in place of its
    /// own, so a run can be shown without changing anything.
    pub fn debug_run(&mut self, ext: &str, action: &str, steps: &str, cx: &mut Context<Self>) {
        let path = pitwall_core::shell::spawn_path();
        let plan: Vec<Planned> = steps
            .split('|')
            .map(|s| {
                let argv: Vec<String> = s.split_whitespace().map(str::to_string).collect();
                let program = crate::extensions::adapter::resolve(std::path::Path::new("."), &argv[0], path.as_deref()).ok();
                Planned { skip: program.is_none().then(|| format!("“{}” isn't installed", argv[0])), argv, program, for_kind: None, optional: false }
            })
            .collect();
        self.exts.confirm = Some((ext.to_string(), action.to_string(), plan));
        let Some(m) = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == ext).cloned() else { return };
        let Some(a) = m.actions.iter().find(|a| a.id == action).cloned() else { return };
        self.run_action(&m, &a, cx);
    }

    /// Bench and debug builds: `ext`'s card unfolded with "What this
    /// extension does on your computer" open.
    pub fn debug_disclose(&mut self, ext: &str, cx: &mut Context<Self>) {
        self.exts.open.insert(ext.to_string(), true);
        self.exts.disclosed.insert(ext.to_string());
        cx.notify();
    }

    /// Extension `id`'s card unfolded (from Accounts' "Edit in
    /// Extensions").
    pub(super) fn exts_open(&mut self, id: &str) {
        self.exts.open.insert(id.to_string(), true);
    }

    /// The Extensions page: what extensions are, each installed one in a
    /// card of its own, then those Pitwall ships that aren't installed.
    pub(super) fn extension_parts(&mut self, cx: &mut Context<Self>) -> Vec<Part> {
        let mut v = vec![Part::section("ext-intro", |s, ui, _, cx| s.ext_intro(ui, cx).into_any_element())];
        if crate::extensions::dir(cx).is_none() {
            return v;
        }
        let found = crate::extensions::found(cx);
        let none = found.is_empty();
        v.push(Part::section(("ext-installed", none), move |s, ui, _, cx| {
            let mut d = div().flex().flex_col().gap(px(6.)).pt(px(4.)).child(crate::kit::label("Installed", ui.t.text_3, 11.));
            if none {
                let shipped = !s.shipped_now(cx).0.is_empty();
                d = d.child(
                    div()
                        .p(px(14.))
                        .rounded(px(10.))
                        .border_1()
                        .border_dashed()
                        .border_color(ui.t.line)
                        .child(crate::kit::muted(if shipped { "Nothing installed yet. Pick one under Available below: installing turns nothing on." } else { "Nothing installed yet." }, &ui.t)),
                );
            }
            d.into_any_element()
        }));
        for f in found.iter() {
            let folder = f.folder.clone();
            v.push(Part::section(("ext", folder.clone()), move |s, ui, _, cx| s.ext_card(&folder, ui, cx).into_any_element()));
        }
        v.push(Part::section("ext-available", |s, ui, _, cx| s.ext_available(ui, cx).into_any_element()));
        v
    }

    /// The extensions Pitwall ships, and whether each installed one with a
    /// newer copy is as Pitwall installed it; read again when the
    /// extensions folder is.
    fn shipped_now(&mut self, cx: &mut Context<Self>) -> (Vec<Shipped>, HashMap<String, bool>) {
        let key = std::rc::Rc::as_ptr(&crate::extensions::found(cx)) as usize;
        if self.exts.shipped.as_ref().is_none_or(|(k, _, _)| *k != key) {
            let shipped = catalog::shipped(cx);
            let dir = crate::extensions::dir(cx);
            // Only one with a newer copy is asked about (the digest reads
            // its files).
            let local = shipped
                .iter()
                .filter(|s| matches!(s.status, Status::Newer(_)))
                .filter_map(|s| Some((s.offer.manifest.id.clone(), pitwall_core::extensions::install::local(&dir.as_ref()?.join(&s.offer.manifest.id)).as_installed)))
                .collect();
            self.exts.shipped = Some((key, shipped, local));
        }
        self.exts.shipped.as_ref().map(|(_, s, l)| (s.clone(), l.clone())).unwrap_or_default()
    }

    fn ext_intro(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        if !self.exts.watching {
            self.exts.watching = true;
            let h = hub::hub(cx);
            self.exts._subs.push(cx.observe(&h, |_, _, cx| cx.notify()));
            let runs = actions::runs(cx);
            self.exts._subs.push(cx.observe(&runs, |_, _, cx| cx.notify()));
        }
        let folder = crate::extensions::dir(cx).map(|d| pitwall_core::paths::tildify(&d.to_string_lossy()));
        section()
            .child(
                head(ui, "Extensions").child(
                    crate::kit::text_button("extensions-rescan", "Look again", BtnKind::GhostSm, false, &ui.t)
                        .tooltip(crate::kit::tooltip("Read the extensions folder again"))
                        .on_click(cx.listener(|_, _, _, cx| {
                            crate::extensions::rescan(cx);
                            cx.notify();
                        })),
                ),
            )
            .child(crate::kit::hint(
                match &folder {
                    Some(f) => format!("Extensions bring questions from your agents' tools into Pitwall, in an inbox. Installing one turns nothing on: its program runs only while it is on. An extension you got elsewhere goes in {f}."),
                    None => "Extensions need Pitwall's data folder, which isn't in use here.".to_string(),
                },
                &ui.t,
            ))
    }

    /// A card: one extension and everything about it.
    fn card(ui: &Ui, id: impl Into<SharedString>) -> Stateful<Div> {
        div().id(id.into()).flex().flex_col().gap(px(12.)).p(px(14.)).rounded(px(10.)).border_1().border_color(ui.t.line)
    }

    /// The status pill: Off, Needs setup, Connecting, Connected, Signed
    /// out, Stopped, Error (or On, without an adapter); its tone; the
    /// adapter's word on why.
    fn status_of(m: &Manifest, c: Option<&Account>, cx: &mut Context<Self>) -> (&'static str, Tone, String, bool) {
        let h = hub::hub(cx);
        let state = match c {
            Some(c) => h.read(cx).connection(&m.id, &c.id),
            None => h.read(cx).get(&m.id),
        };
        let (run, conn) = state.map(|s| (s.run.clone(), s.conn.clone())).unwrap_or_default();
        let connected = run == Run::Running && matches!(conn, Some((ConnState::Ready, _)));
        let prefs = connections::prefs_for(m, c, cx);
        if !prefs.enabled {
            return ("Off", Tone::Plain, String::new(), connected);
        }
        let list = checklist(m, &prefs, connected);
        let missing = list.iter().any(|(s, done)| !done && *s != Step::Connected);
        let (word, tone, why) = run_text(&run, conn.as_ref());
        if missing {
            return ("Needs setup", Tone::Warn, why, connected);
        }
        if m.adapter.is_none() {
            return ("On", Tone::Ok, String::new(), connected);
        }
        let word = match word {
            "connected" => "Connected",
            "connecting" => "Connecting",
            "signed out" => "Signed out",
            "stopped" => "Stopped",
            "needs setup" => "Needs setup",
            "error" => "Error",
            _ => "Off",
        };
        (word, tone, why, connected)
    }

    /// Installed extension `folder`'s card.
    fn ext_card(&mut self, folder: &str, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let found = crate::extensions::found(cx);
        let Some(f) = found.iter().find(|f| f.folder == folder) else { return div() };
        let m = match &f.manifest {
            Ok(m) => m.clone(),
            Err(why) => {
                return div().child(
                    Self::card(ui, format!("extension-bad-{}", f.folder)).child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .items_start()
                            .child(div().mt(px(3.)).child(crate::kit::icon("alert", 16., ui.t.text_3)))
                            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(head(ui, &f.folder)).child(crate::kit::hint(format!("Can't be used: {why}."), &ui.t))),
                    ),
                );
            }
        };
        let (shipped, local) = self.shipped_now(cx);
        let on = crate::ui_state::get(cx).extension_on(&m.id);
        let conns = if m.settings.is_empty() { vec![] } else { connections::list(&m.id, cx) };
        let chosen = connections::chosen(&m.id, self.exts.conn.get(&m.id).map(String::as_str), cx).filter(|_| !m.settings.is_empty());
        // The pill: that connection's, or with several the extension's as
        // a whole (the best of them; needs setup while none is set up).
        let (word, tone, why, connected) = Self::status_of(&m, chosen.as_ref(), cx);
        let needs_setup = if conns.len() > 1 { conns.iter().all(|c| Self::status_of(&m, Some(c), cx).0 == "Needs setup") } else { word == "Needs setup" };
        let (pill, tone): (SharedString, Tone) = if conns.len() > 1 && on { Self::overall(&m, &conns, cx) } else { (word.into(), tone) };
        let open = self.exts.open.get(&m.id).copied().unwrap_or(on || needs_setup);
        let (icon, text) = Self::ext_title(ui, &m, &f.folder);
        let id = m.id.clone();
        let switch = crate::kit::toggle(SharedString::from(format!("extension-switch-{}", m.id)), on, &ui.t).tooltip(crate::kit::tooltip(if on { "Turn off" } else { "Turn on" })).on_click(cx.listener(move |v, _, _, cx| {
            let id = id.clone();
            crate::ui_state::update(cx, |s| *s = s.clone().set_extension(&id, !on));
            // Turned on: unfolded; turned off: folded.
            v.exts.open.remove(&id);
            cx.notify();
        }));
        let id = m.id.clone();
        let chevron = div()
            .id(SharedString::from(format!("extension-fold-{}", m.id)))
            .cursor_pointer()
            .p(px(4.))
            .rounded(px(4.))
            .hover(|d| d.bg(ui.t.surface_2))
            .tooltip(crate::kit::tooltip(if open { "Fold" } else { "Unfold" }))
            .child(crate::kit::chevron_at(open, 12., ui.t.text_3))
            .on_click(cx.listener(move |v, _, _, cx| {
                v.exts.open.insert(id.clone(), !open);
                cx.notify();
            }));
        let top = div()
            .flex()
            .gap(px(12.))
            .items_start()
            .child(icon)
            .child(text)
            // A fixed width: the row doesn't move as the state changes.
            .child(div().w(px(110.)).flex().justify_end().mt(px(2.)).child(crate::kit::chip_tone(pill, tone, &ui.t)))
            .child(div().mt(px(4.)).child(switch))
            .child(div().mt(px(1.)).child(chevron));
        let mut card = Self::card(ui, format!("extension-card-{}", m.id)).child(top);
        let changed = local.get(&m.id) == Some(&false);
        card = card.children(self.installed_notes(ui, &m, shipped.iter().find(|s| s.offer.manifest.id == m.id), changed, cx));
        if open {
            if on {
                let new_here = self.exts.naming.as_ref().is_some_and(|(e, c, _)| *e == m.id && c.is_none());
                if conns.len() > 1 || new_here {
                    card = card.child(self.conn_bar(ui, &m, &conns, chosen.as_ref(), cx));
                }
                if let Some(ask) = self.remove_conn_card(ui, &m, &conns, cx) {
                    card = card.child(ask);
                }
                card = card.child(self.checklist_view(ui, &m, chosen.as_ref(), connected, &why, cx));
                let more: Vec<&Action> = m.actions.iter().filter(|a| !a.setup).collect();
                if !more.is_empty() {
                    let mut other = div().flex().flex_col().gap(px(10.)).pt(px(10.)).border_t_1().border_color(ui.t.line).child(crate::kit::label("More", ui.t.text_3, 11.));
                    for a in more {
                        other = other.child(self.action_block(ui, &m, a, chosen.as_ref(), None, cx));
                    }
                    card = card.child(other);
                }
            } else {
                card = card.child(crate::kit::hint(format!("Turn it on to set it up. While it is off, nothing of {} runs.", m.name), &ui.t));
            }
            card = card.child(self.card_footer(ui, &m, &f.folder, cx));
        }
        div().child(card)
    }

    /// With several connections, the extension's pill: Connected when all
    /// are, "1 of 2 connected" when some are, else the first one's state.
    fn overall(m: &Manifest, conns: &[Account], cx: &mut Context<Self>) -> (SharedString, Tone) {
        let states: Vec<(&'static str, Tone)> = conns.iter().map(|c| {
            let (w, t, _, _) = Self::status_of(m, Some(c), cx);
            (w, t)
        }).collect();
        let up = states.iter().filter(|(w, _)| *w == "Connected" || *w == "On").count();
        if up == states.len() {
            return (states[0].0.into(), Tone::Ok);
        }
        if up > 0 {
            return (format!("{up} of {} connected", states.len()).into(), Tone::Warn);
        }
        if states.iter().all(|(w, _)| *w == "Needs setup") {
            return ("Needs setup".into(), Tone::Warn);
        }
        if states.iter().any(|(w, _)| *w == "Connecting") {
            return ("Connecting".into(), Tone::Subtle);
        }
        let (w, t) = states.iter().find(|(w, _)| *w != "Needs setup").copied().unwrap_or(states[0]);
        (w.into(), t)
    }

    /// The dot of a connection's tab: green connected, amber when it needs
    /// the user, grey otherwise.
    fn dot(tone: Tone, ui: &Ui) -> Div {
        let color = match tone {
            Tone::Ok => ui.t.green,
            Tone::Warn => ui.t.amber,
            _ => ui.t.text_4,
        };
        div().flex_none().size(px(7.)).rounded_full().bg(color)
    }

    /// The connections of a card with two or more (or one being added):
    /// a tab each (its state as a dot, the default marked), the one shown
    /// picked; beside them Rename, Make default and Remove for it. A
    /// connection being named shows its field in place.
    fn conn_bar(&mut self, ui: &Ui, m: &Manifest, conns: &[Account], chosen: Option<&Account>, cx: &mut Context<Self>) -> Div {
        let default = connections::default_id(&m.id, cx);
        let list = crate::accounts::list(cx).cloned();
        let naming = self.exts.naming.as_ref().filter(|(e, _, _)| *e == m.id).map(|(_, c, i)| (c.clone(), i.clone()));
        let mut tabs = div().flex().flex_wrap().items_center().gap(px(6.));
        for c in conns {
            if let Some((Some(r), input)) = &naming {
                if *r == c.id {
                    tabs = tabs.child(div().w(px(180.)).child(input.clone()));
                    continue;
                }
            }
            let on = chosen.is_some_and(|x| x.id == c.id);
            let (word, tone, why, _) = Self::status_of(m, Some(c), cx);
            let profiles = list.as_ref().map(|l| l.profiles_of(&c.id)).unwrap_or_default();
            let mut tip = word.to_string();
            if !why.is_empty() {
                tip = format!("{tip}: {why}");
            }
            if default.as_deref() == Some(&c.id) {
                tip.push_str(". The default: projects that don't pick a connection use it");
            }
            if !profiles.is_empty() {
                tip.push_str(&format!(". Profile {} uses it", profiles.join(", ")));
            }
            let (ext, id) = (m.id.clone(), c.id.clone());
            let mut tab = div()
                .id(SharedString::from(format!("extension-conn-{}", c.id)))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(26.))
                .px(px(10.))
                .rounded(px(6.))
                .border_1()
                .cursor_pointer()
                .text_size(px(12.))
                .tooltip(crate::kit::tooltip(tip))
                .child(Self::dot(tone, ui))
                .child(c.name.clone());
            tab = if on { tab.bg(ui.t.surface_3).border_color(ui.t.line_strong).text_color(ui.t.text).font_weight(crate::kit::WEIGHT_550) } else { tab.border_color(ui.t.line).text_color(ui.t.text_2).hover(|d| d.bg(ui.t.surface_2)) };
            if default.as_deref() == Some(&c.id) && conns.len() > 1 {
                tab = tab.child(div().text_size(px(11.)).text_color(ui.t.text_3).font_weight(FontWeight::NORMAL).child("default"));
            }
            tabs = tabs.child(tab.on_click(cx.listener(move |v, _, _, cx| {
                v.exts.conn.insert(ext.clone(), id.clone());
                v.exts.remove_conn = None;
                cx.notify();
            })));
        }
        if let Some((None, input)) = &naming {
            tabs = tabs.child(div().w(px(180.)).child(input.clone()));
        }
        let mut row = div().flex().flex_wrap().items_center().gap(px(8.)).child(tabs).child(div().flex_1());
        if naming.is_some() {
            let ext = m.id.clone();
            row = row
                .child(crate::kit::text_button(SharedString::from(format!("extension-conn-name-cancel-{}", m.id)), "Cancel", BtnKind::GhostSm, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                    v.exts.naming = None;
                    v.exts.naming_note = None;
                    cx.notify();
                })))
                .child(crate::kit::text_button(SharedString::from(format!("extension-conn-name-ok-{}", m.id)), "Save", BtnKind::Small, false, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.finish_naming(&ext, cx))));
        } else if let Some(c) = chosen.filter(|_| conns.len() > 1) {
            let (ext, id) = (m.id.clone(), c.id.clone());
            row = row.child(crate::kit::text_button(SharedString::from(format!("extension-conn-rename-{}", m.id)), "Rename", BtnKind::GhostSm, false, &ui.t).on_click(cx.listener(move |v, _, w, cx| v.start_naming(&ext, Some(id.clone()), w, cx))));
            if default.as_deref() != Some(&c.id) {
                let (ext, id) = (m.id.clone(), c.id.clone());
                row = row.child(
                    crate::kit::text_button(SharedString::from(format!("extension-conn-default-{}", m.id)), "Make default", BtnKind::GhostSm, false, &ui.t)
                        .tooltip(crate::kit::tooltip("Projects that don't pick a connection use the default"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let model = crate::accounts::model(cx);
                            let _ = model.update(cx, |a, cx| a.set_default_connection(&ext, Some(&id), cx));
                        })),
                );
            }
            let (ext, id) = (m.id.clone(), c.id.clone());
            let next = conns.iter().find(|x| x.id != c.id).map(|x| x.id.clone());
            let was_default = default.as_deref() == Some(&c.id);
            row = row.child(crate::kit::text_button(SharedString::from(format!("extension-conn-remove-{}", m.id)), "Remove", BtnKind::GhostSm, false, &ui.t).on_click(cx.listener(move |v, _, _, cx| {
                v.exts.remove_conn = Some((ext.clone(), id.clone(), if was_default { next.clone() } else { None }));
                cx.notify();
            })));
        }
        let mut col = div().flex().flex_col().gap(px(6.)).child(row);
        if let Some(n) = self.exts.naming_note.as_ref().filter(|_| naming.is_some()) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else if matches!(naming, Some((None, _))) {
            col = col.child(crate::kit::hint("Another server or key, like Work or Personal. Each project then picks which one its agents use (in its Accounts…, or through its profile).", &ui.t));
        }
        col
    }

    /// Removing connection asked: what goes, who used it, and (when it is
    /// the default and others remain) which one becomes the default, or
    /// none; never one chosen silently.
    fn remove_conn_card(&mut self, ui: &Ui, m: &Manifest, conns: &[Account], cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, id, pick) = self.exts.remove_conn.clone().filter(|(e, _, _)| *e == m.id)?;
        let c = conns.iter().find(|c| c.id == id)?.clone();
        let list = crate::accounts::list(cx).cloned().unwrap_or_default();
        let default = connections::default_id(&m.id, cx).as_deref() == Some(&c.id);
        let profiles = list.profiles_of(&c.id);
        let projects = list.projects_of(&c.id);
        let mut text = format!("Remove the connection {}? Its settings and saved secrets are deleted.", c.name);
        let users: Vec<String> = profiles.iter().map(|p| format!("profile {p}")).chain(projects.iter().map(|p| pitwall_core::paths::tildify(p))).collect();
        if !users.is_empty() {
            text.push_str(&format!(" {} use it and go back to their profile's connection or the default.", users.join(", ")));
        }
        let mut card = div().id(SharedString::from(format!("extension-conn-ask-{}", m.id))).flex().flex_col().gap(px(8.)).p(px(10.)).rounded(px(8.)).border_1().border_color(ui.t.line_strong).bg(ui.t.surface_2).child(div().text_size(px(13.)).text_color(ui.t.text).child(text));
        let others: Vec<&Account> = conns.iter().filter(|x| x.id != c.id).collect();
        if default && !others.is_empty() {
            let mut opts = div().flex().flex_col().gap(px(4.)).child(div().text_size(px(12.)).text_color(ui.t.text_2).child("It is the default. Which one becomes the default?"));
            let choices: Vec<(Option<String>, String)> = others.iter().map(|o| (Some(o.id.clone()), o.name.clone())).chain(std::iter::once((None, "None: projects that don't pick a connection get none".to_string()))).collect();
            for (i, (val, label)) in choices.into_iter().enumerate() {
                let on = pick == val;
                let ext = m.id.clone();
                let cid = c.id.clone();
                opts = opts.child(
                    div()
                        .id(SharedString::from(format!("extension-conn-ask-default-{}-{i}", m.id)))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(ui.t.text)
                        .child(crate::kit::radio(on, &ui.t))
                        .child(label)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.exts.remove_conn = Some((ext.clone(), cid.clone(), val.clone()));
                            cx.notify();
                        })),
                );
            }
            card = card.child(opts);
        }
        let ask_id = m.id.clone();
        let cancel = crate::kit::text_button(SharedString::from(format!("extension-conn-ask-cancel-{ask_id}")), "Cancel", BtnKind::GhostSm, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
            v.exts.remove_conn = None;
            cx.notify();
        }));
        let (ext, cid) = (m.id.clone(), c.id.clone());
        let yes = crate::kit::text_button(SharedString::from(format!("extension-conn-ask-yes-{}", m.id)), "Remove", BtnKind::Danger, false, &ui.t).on_click(cx.listener(move |v, _, _, cx| {
            v.exts.remove_conn = None;
            v.exts.conn.remove(&ext);
            let me = cx.entity().downgrade();
            let ext2 = ext.clone();
            connections::remove_with(&cid, if default { pick.as_deref() } else { None }, cx, move |err, cx| {
                let _ = me.update(cx, |v, cx| {
                    if let Some(e) = err {
                        v.exts.failed.insert(ext2.clone(), e);
                    }
                    cx.notify();
                });
            });
            cx.notify();
        }));
        Some(card.child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).child(yes)).into_any_element())
    }

    /// Name a new connection of `ext` (`conn` None) or rename `conn`.
    fn start_naming(&mut self, ext: &str, conn: Option<String>, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let text = conn.as_deref().and_then(|c| crate::accounts::list(cx).and_then(|l| l.get(c)).map(|a| a.name.clone())).unwrap_or_default();
        let input = cx.new(|cx| TextInput::new(cx, &text, "Name, like Work"));
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all_text(cx);
        });
        let e = ext.to_string();
        let sub = cx.subscribe(&input, move |v, _, ev: &InputEvent, cx| match ev {
            InputEvent::Submit => v.finish_naming(&e, cx),
            InputEvent::Cancel => {
                v.exts.naming = None;
                v.exts.naming_note = None;
                cx.notify();
            }
            _ => {}
        });
        self.exts._subs.push(sub);
        self.exts.naming = Some((ext.to_string(), conn, input));
        self.exts.naming_note = None;
        self.exts.remove_conn = None;
        self.exts.open.insert(ext.to_string(), true);
        cx.notify();
    }

    /// The name typed: the connection made (and shown) or renamed.
    fn finish_naming(&mut self, ext: &str, cx: &mut Context<Self>) {
        let Some((e, conn, input)) = self.exts.naming.clone() else { return };
        if e != ext {
            return;
        }
        let name = input.read(cx).text().trim().to_string();
        let model = crate::accounts::model(cx);
        let res = match &conn {
            Some(id) => model.update(cx, |m, cx| m.rename(id, &name, cx)).map(|_| id.clone()),
            None => model.update(cx, |m, cx| m.add_connection(ext, &name, cx)).map(|a| a.id),
        };
        match res {
            Ok(id) => {
                self.exts.naming = None;
                self.exts.naming_note = None;
                self.exts.conn.insert(ext.to_string(), id);
            }
            Err(why) => self.exts.naming_note = Some(why.replace("an account", "a connection")),
        }
        cx.notify();
    }

    /// An extension's icon, name, version (and a demo mark) and description.
    fn ext_title(ui: &Ui, m: &Manifest, folder: &str) -> (AnyElement, Div) {
        let icon = div()
            .flex_none()
            .size(px(32.))
            .rounded(px(8.))
            .bg(ui.t.surface_2)
            .flex()
            .items_center()
            .justify_center()
            .child(crate::kit::icon(crate::extensions::icon_of(m), 16., ui.t.text_2))
            .into_any_element();
        let mut name = div().flex().items_center().gap(px(6.)).child(div().flex_none().font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(m.name.clone()));
        if let Some(v) = &m.version {
            name = name.child(div().flex_none().text_size(px(12.)).text_color(ui.t.text_3).child(v.clone()));
        }
        if m.demo {
            name = name.child(crate::kit::chip_tone("demo", Tone::Subtle, &ui.t));
        }
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(name)
            .child(crate::kit::hint(m.description.clone().unwrap_or_else(|| format!("Folder {folder}")), &ui.t));
        (icon, text)
    }

    /// The numbered steps of setting it up, each a tick once done, with
    /// its field or button.
    fn checklist_view(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, connected: bool, why: &str, cx: &mut Context<Self>) -> Div {
        let prefs = connections::prefs_for(m, c, cx);
        let list = checklist(m, &prefs, connected);
        let left = list.iter().filter(|(_, d)| !d).count();
        let mut col = div().flex().flex_col().gap(px(14.));
        col = col.child(div().text_size(px(12.)).text_color(ui.t.text_3).child(match left {
            0 => "All set".to_string(),
            1 => "One step left".to_string(),
            n => format!("{n} steps left"),
        }));
        for (i, (step, done)) in list.iter().enumerate() {
            let marker = {
                let d = div().flex_none().size(px(20.)).rounded_full().flex().items_center().justify_center().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD);
                if *done {
                    d.bg(ui.t.green.opacity(0.18)).child(crate::kit::icon("check", 12., ui.t.green))
                } else {
                    d.border_1().border_color(ui.t.line_strong).text_color(ui.t.text_2).child((i + 1).to_string())
                }
            };
            let body: AnyElement = match step {
                Step::Setting(key) => {
                    let Some(s) = m.settings.iter().find(|s| s.key == *key) else { continue };
                    match s.kind {
                        SettingKind::Url => self.url_row(ui, m, c, s, cx).into_any_element(),
                        SettingKind::Secret => self.secret_row(ui, m, c, s, cx).into_any_element(),
                    }
                }
                Step::Action(id) => {
                    let Some(a) = m.actions.iter().find(|a| a.id == *id) else { continue };
                    self.action_block(ui, m, a, c, Some(*done), cx).into_any_element()
                }
                Step::Connected => self.connected_row(ui, m, c, connected, why, cx).into_any_element(),
            };
            col = col.child(div().flex().gap(px(10.)).items_start().child(marker).child(div().flex_1().min_w_0().child(body)));
        }
        col
    }

    /// A step's title (13 px, medium).
    fn step_title(ui: &Ui, text: impl Into<SharedString>) -> Div {
        div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(ui.t.text).child(text.into())
    }

    /// The last step: connected, or what the adapter says, with Try again.
    fn connected_row(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, connected: bool, why: &str, cx: &mut Context<Self>) -> Div {
        let h = hub::hub(cx);
        let run = match c {
            Some(c) => h.read(cx).connection(&m.id, &c.id),
            None => h.read(cx).get(&m.id),
        }
        .map(|s| s.run.clone())
        .unwrap_or_default();
        let title = if connected {
            "Connected"
        } else {
            match run {
                Run::Starting | Run::Running if why.is_empty() => "Connecting…",
                Run::Setup(_) | Run::Off => "Connect",
                _ => "Not connected",
            }
        };
        let mut line = div().flex().items_center().gap(px(8.)).child(Self::step_title(ui, title)).child(div().flex_1());
        if matches!(run, Run::Running | Run::Stopped(_)) && !connected {
            let id = m.id.clone();
            line = line.child(crate::kit::text_button(SharedString::from(format!("extension-restart-{}", m.id)), "Try again", BtnKind::GhostSm, false, &ui.t).on_click(cx.listener(move |_, _, _, cx| {
                let h = hub::hub(cx);
                h.update(cx, |h, cx| h.restart(&id, cx));
            })));
        }
        let text = match (&run, connected) {
            (_, true) if why.is_empty() => "Its inbox is ready: open it with ⌘I.".to_string(),
            (_, true) => why.to_string(),
            (Run::Setup(_), _) => "Pitwall connects once the steps above are done.".to_string(),
            _ => why.to_string(),
        };
        let mut col = div().flex().flex_col().gap(px(3.)).child(line);
        if !text.is_empty() {
            let color = if connected || matches!(run, Run::Setup(_) | Run::Starting) { ui.t.text_3 } else { ui.t.amber };
            col = col.child(div().text_size(px(12.)).line_height(gpui::relative(1.45)).text_color(color).child(text));
        }
        col
    }

    /// Below the top row: Update when a newer copy ships, "changed
    /// locally", what failed, and the Remove or Replace question.
    fn installed_notes(&mut self, ui: &Ui, m: &Manifest, shipped: Option<&Shipped>, changed: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let working = self.exts.working.contains(&m.id);
        if let Some(Status::Newer(v)) = shipped.map(|s| &s.status) {
            let id = m.id.clone();
            let mut line = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(ui.t.surface_2)
                .child(div().flex_1().min_w(px(160.)).text_size(px(12.)).text_color(ui.t.text_2).child(format!("Version {v} is available. Its settings and secrets stay.")));
            if changed {
                line = line.child(div().id(SharedString::from(format!("extension-changed-{}", m.id))).tooltip(crate::kit::tooltip("Its folder isn't as Pitwall installed it: Update asks before replacing it.")).child(crate::kit::chip_tone("changed locally", Tone::Warn, &ui.t)));
            }
            line = line.child(
                crate::kit::text_button(SharedString::from(format!("extension-update-{}", m.id)), if working { "Updating…".to_string() } else { "Update".to_string() }, BtnKind::Small, working, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.start_update(&id, false, cx))),
            );
            out.push(line.into_any_element());
        }
        if let Some(why) = self.exts.failed.get(&m.id) {
            out.push(div().text_size(px(12.)).text_color(ui.t.amber).child(why.clone()).into_any_element());
        }
        let cancel = |id: &str, cx: &mut Context<Self>| {
            crate::kit::text_button(SharedString::from(format!("extension-ask-cancel-{id}")), "Cancel", BtnKind::GhostSm, false, &ui.t).on_click(cx.listener(|v, _, _, cx| {
                v.exts.confirm_remove = None;
                v.exts.confirm_replace = None;
                cx.notify();
            }))
        };
        let card = |id: &str, text: String, cancel: AnyElement, yes: AnyElement| {
            div()
                .id(SharedString::from(format!("extension-ask-{id}")))
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(ui.t.line_strong)
                .bg(ui.t.surface_2)
                .child(div().text_size(px(13.)).text_color(ui.t.text).child(text))
                .child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).child(yes))
                .into_any_element()
        };
        if self.exts.confirm_remove.as_deref() == Some(&m.id) {
            let on = crate::ui_state::get(cx).extension_on(&m.id);
            let n = connections::list(&m.id, cx).len();
            let what = if n > 1 { format!("its {n} connections with their saved secrets, and its folder are deleted") } else { "its settings, its saved secrets and its folder are deleted".to_string() };
            let text = if on { format!("Remove {}? It is turned off, and {what}.", m.name) } else { format!("Remove {}? {}{}.", m.name, what[..1].to_uppercase(), &what[1..]) };
            let m2 = m.clone();
            let yes = crate::kit::text_button(SharedString::from(format!("extension-ask-yes-{}", m.id)), "Remove", BtnKind::Danger, false, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.start_remove(&m2, cx)));
            out.push(card(&m.id, text, cancel(&m.id, cx).into_any_element(), yes.into_any_element()));
        }
        if self.exts.confirm_replace.as_deref() == Some(&m.id) {
            let text = format!("{}'s folder was changed since Pitwall installed it, or was put there by hand. Updating replaces it, and those changes are lost. Its settings and secrets stay.", m.name);
            let id = m.id.clone();
            let yes = crate::kit::text_button(SharedString::from(format!("extension-ask-yes-{}", m.id)), "Replace", BtnKind::Small, false, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.start_update(&id, true, cx)));
            out.push(card(&m.id, text, cancel(&m.id, cx).into_any_element(), yes.into_any_element()));
        }
        out
    }

    /// The card's last row: what it does on this computer (folded), its
    /// docs, and Remove.
    fn card_footer(&mut self, ui: &Ui, m: &Manifest, folder: &str, cx: &mut Context<Self>) -> Div {
        let shown = self.exts.disclosed.contains(&m.id);
        let id = m.id.clone();
        let toggle = div()
            .id(SharedString::from(format!("extension-what-{}", m.id)))
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(ui.t.text_3)
            .hover(|d| d.text_color(ui.t.text_2))
            .child(crate::kit::chevron_at(shown, 10., ui.t.text_3))
            .child("What this extension does on your computer")
            .on_click(cx.listener(move |v, _, _, cx| {
                if !v.exts.disclosed.remove(&id) {
                    v.exts.disclosed.insert(id.clone());
                }
                cx.notify();
            }));
        let mut row = div().flex().flex_wrap().items_center().gap(px(10.)).child(toggle).child(div().flex_1());
        // Another connection: only once there is one and it is on.
        let on = crate::ui_state::get(cx).extension_on(&m.id);
        if on && !m.settings.is_empty() && !connections::list(&m.id, cx).is_empty() {
            let id = m.id.clone();
            row = row.child(
                crate::kit::text_button(SharedString::from(format!("extension-conn-add-{}", m.id)), "Add connection", BtnKind::GhostSm, false, &ui.t)
                    .tooltip(crate::kit::tooltip("Another server or key for this extension; each project picks which one its agents use"))
                    .on_click(cx.listener(move |v, _, w, cx| v.start_naming(&id, None, w, cx))),
            );
        }
        if let Some(url) = m.docs.clone() {
            row = row.child(crate::kit::text_button(SharedString::from(format!("extension-docs-{}", m.id)), "Docs", BtnKind::Link, false, &ui.t).tooltip(crate::kit::tooltip(url.clone())).on_click(move |_, _, cx| crate::platform::files::open_url(cx, &url)));
        }
        let working = self.exts.working.contains(&m.id);
        let id = m.id.clone();
        row = row.child(crate::kit::text_button(SharedString::from(format!("extension-remove-{}", m.id)), if working { "Working…" } else { "Remove" }, BtnKind::GhostSm, working, &ui.t).on_click(cx.listener(move |v, _, _, cx| {
            v.exts.confirm_remove = Some(id.clone());
            v.exts.confirm_replace = None;
            cx.notify();
        })));
        let mut col = div().flex().flex_col().gap(px(8.)).pt(px(10.)).border_t_1().border_color(ui.t.line).child(row);
        if shown {
            col = col.child(self.disclosure(ui, m, folder, cx));
        }
        col
    }

    /// What it runs while on, what it sets on agents, and what its buttons
    /// can run.
    fn disclosure(&mut self, ui: &Ui, m: &Manifest, folder: &str, cx: &mut Context<Self>) -> Div {
        let mut col = div().flex().flex_col().gap(px(10.)).pl(px(14.));
        let block = |title: &str| div().flex().flex_col().gap(px(3.)).child(div().text_size(px(12.)).text_color(ui.t.text_2).child(title.to_string()));
        if let Some(a) = &m.adapter {
            col = col.child(block("While it is on, runs this in its folder:").child(crate::kit::mono_text(a.command.join(" "), &ui.t)));
        } else {
            col = col.child(block("Runs no program of its own."));
        }
        if !m.agent_env.is_empty() {
            let values = connections::chosen(&m.id, self.exts.conn.get(&m.id).map(String::as_str), cx).map(|c| c.values).unwrap_or_default();
            let mut b = block("Sets these on agents you start while it is on ({project}: the project folder's name; {agent_id}: the agent's id):");
            for line in env_preview(m, &values) {
                b = b.child(crate::kit::mono_text(line, &ui.t));
            }
            col = col.child(b);
        }
        for a in &m.actions {
            let mut b = block(&format!("“{}” runs, when you press it and confirm:", a.label));
            for s in &a.steps {
                b = b.child(crate::kit::mono_text(s.run.join(" "), &ui.t));
            }
            col = col.child(b);
        }
        if let Some(dir) = crate::extensions::dir(cx) {
            col = col.child(block("Its folder:").child(crate::kit::mono_text(pitwall_core::paths::tildify(&dir.join(folder).to_string_lossy()), &ui.t)));
        }
        col
    }

    /// Those Pitwall ships that aren't installed: a gallery, each with
    /// Install.
    fn ext_available(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let (shipped, _) = self.shipped_now(cx);
        let available: Vec<Manifest> = shipped.iter().filter(|s| s.status == Status::Available).map(|s| s.offer.manifest.clone()).collect();
        let mut d = div().flex().flex_col().gap(px(8.));
        if available.is_empty() {
            return d;
        }
        d = d.child(div().pt(px(8.)).child(crate::kit::label("Available", ui.t.text_3, 11.)));
        let mut grid = div().flex().flex_wrap().gap(px(10.));
        for m in &available {
            grid = grid.child(self.available_tile(ui, m, cx));
        }
        d.child(grid)
    }

    /// A shipped extension that isn't installed: Install.
    fn available_tile(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> AnyElement {
        let (icon, text) = Self::ext_title(ui, m, &m.id);
        let working = self.exts.working.contains(&m.id);
        let id = m.id.clone();
        let button = crate::kit::text_button(SharedString::from(format!("extension-install-{}", m.id)), if working { "Installing…" } else { "Install" }, BtnKind::Small, working, &ui.t).on_click(cx.listener(move |v, _, _, cx| {
            v.exts.working.insert(id.clone());
            v.exts.failed.remove(&id);
            cx.notify();
            let (me, id2) = (cx.entity().downgrade(), id.clone());
            catalog::install_one(&id, cx, move |o, cx| {
                let _ = me.update(cx, |v, cx| v.ext_done(&id2, o, cx));
            });
        }));
        let mut tile = Self::card(ui, format!("extension-available-{}", m.id))
            .flex_1()
            .min_w(px(260.))
            .child(div().flex().gap(px(12.)).items_start().child(icon).child(text).child(div().mt(px(2.)).child(button)));
        if let Some(why) = self.exts.failed.get(&m.id) {
            tile = tile.child(div().text_size(px(12.)).text_color(ui.t.amber).child(why.clone()));
        }
        tile.into_any_element()
    }

    fn start_update(&mut self, id: &str, replace_changed: bool, cx: &mut Context<Self>) {
        self.exts.working.insert(id.to_string());
        self.exts.failed.remove(id);
        self.exts.confirm_replace = None;
        cx.notify();
        let (me, id2) = (cx.entity().downgrade(), id.to_string());
        catalog::update_one(id, replace_changed, cx, move |o, cx| {
            let _ = me.update(cx, |v, cx| v.ext_done(&id2, o, cx));
        });
    }

    fn start_remove(&mut self, m: &Manifest, cx: &mut Context<Self>) {
        self.exts.working.insert(m.id.clone());
        self.exts.failed.remove(&m.id);
        self.exts.confirm_remove = None;
        cx.notify();
        let (me, id) = (cx.entity().downgrade(), m.id.clone());
        catalog::remove_one(m, cx, move |o, cx| {
            let _ = me.update(cx, |v, cx| v.ext_done(&id, o, cx));
        });
    }

    /// Install, Update or Remove of `id` is over.
    fn ext_done(&mut self, id: &str, o: catalog::Outcome, cx: &mut Context<Self>) {
        self.exts.working.remove(id);
        match o {
            catalog::Outcome::Done => {}
            catalog::Outcome::ChangedLocally => self.exts.confirm_replace = Some(id.to_string()),
            catalog::Outcome::Failed(why) => {
                self.exts.failed.insert(id.to_string(), why);
            }
        }
        cx.notify();
    }

    /// The plan of `a`, to confirm.
    fn ask_action(&mut self, m: &Manifest, a: &Action, cx: &mut Context<Self>) {
        let c = connections::chosen(&m.id, self.exts.conn.get(&m.id).map(String::as_str), cx).filter(|_| !m.settings.is_empty());
        let values = c.as_ref().map(|c| c.values.clone()).unwrap_or_default();
        let plan = actions::plan(m, a, &values, pitwall_core::shell::spawn_path().as_deref());
        self.exts.confirm = Some((m.id.clone(), run_key(a, c.as_ref()), plan));
        cx.notify();
    }

    /// A setup action: its title and description with its button; then
    /// the commands to confirm, its steps as they run, or how it went.
    /// `done`: its checklist step's state (None: not a checklist step).
    fn action_block(&mut self, ui: &Ui, m: &Manifest, a: &Action, c: Option<&Account>, done: Option<bool>, cx: &mut Context<Self>) -> Div {
        let key = run_key(a, c);
        let runs = actions::runs(cx);
        let state = runs.read(cx).get(&m.id, &key);
        let running = state.as_ref().is_some_and(|s| !s.over);
        let asking = self.exts.confirm.as_ref().is_some_and(|(e, id, _)| *e == m.id && *id == key);
        let label = match (done, &state) {
            (_, Some(s)) if s.over && (s.failed() || s.cancelled) => "Run again",
            (Some(true), _) => "Run again",
            _ => "Run…",
        };
        let open = {
            let (m, a) = (m.clone(), a.clone());
            crate::kit::text_button(SharedString::from(format!("extension-action-{}-{}", m.id, a.id)), label, BtnKind::Small, running || asking, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.ask_action(&m, &a, cx)))
        };
        let mut col = div().flex().flex_col().gap(px(6.)).child(
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(Self::step_title(ui, a.label.clone())).child(crate::kit::hint(a.description.clone(), &ui.t)))
                .child(open),
        );
        if asking {
            col = col.child(self.confirm_card(ui, m, a, cx));
        } else if let Some(st) = state {
            col = col.child(self.run_view(ui, m, &key, &st, cx));
        }
        col
    }

    /// The commands a press will run, to confirm.
    fn confirm_card(&mut self, ui: &Ui, m: &Manifest, a: &Action, cx: &mut Context<Self>) -> Stateful<Div> {
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
            .child(div().pb(px(2.)).text_size(px(12.)).text_color(ui.t.text_2).child(if runs { "These commands will run, in this order, when you press Run:" } else { "Nothing can run:" }));
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
        card.child(buttons)
    }

    /// A run, as it goes or as it ended: each step with its state, the
    /// running one's time and latest output, then how it ended.
    fn run_view(&mut self, ui: &Ui, m: &Manifest, key: &str, st: &RunState, cx: &mut Context<Self>) -> Div {
        let motion = crate::theme::motion_on(cx);
        let mut list = div().flex().flex_col().gap(px(6.)).p(px(10.)).rounded(px(8.)).border_1().border_color(ui.t.line).bg(ui.t.surface_2.opacity(0.5));
        for (i, (line, state)) in st.steps.iter().enumerate() {
            let slot = |d: AnyElement| div().flex_none().w(px(16.)).h(px(18.)).flex().items_center().justify_center().child(d);
            let (mark, word, color): (AnyElement, String, gpui::Hsla) = match state {
                StepState::Waiting => (div().size(px(8.)).rounded_full().border_1().border_color(ui.t.line_strong).into_any_element(), "waiting".into(), ui.t.text_4),
                StepState::Running(since) => (crate::kit::spinner(SharedString::from(format!("extension-run-spin-{}-{key}-{i}", m.id)), 12., motion, &ui.t), format!("running · {} s", since.elapsed().as_secs()), ui.t.text_2),
                StepState::Ended(Outcome::Done) => (crate::kit::icon("check", 13., ui.t.green).into_any_element(), "done".into(), ui.t.green),
                StepState::Ended(Outcome::Failed(_)) => (crate::kit::icon("x", 13., ui.t.amber).into_any_element(), "failed".into(), ui.t.amber),
                StepState::Ended(Outcome::Skipped(_)) => (div().w(px(8.)).h(px(1.5)).bg(ui.t.text_4).into_any_element(), "skipped".into(), ui.t.text_3),
            };
            let mut row = div().flex().flex_col().gap(px(3.)).child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(slot(mark))
                    .child(div().flex_1().min_w_0().child(crate::kit::mono_text(line.clone(), &ui.t)))
                    .child(div().flex_none().w(px(92.)).flex().justify_end().text_size(px(12.)).text_color(color).child(word)),
            );
            let reason = match state {
                StepState::Ended(Outcome::Failed(why)) | StepState::Ended(Outcome::Skipped(why)) => Some(why.clone()),
                _ => None,
            };
            if let Some(why) = reason {
                row = row.child(div().pl(px(24.)).text_size(px(12.)).text_color(ui.t.text_3).child(why));
            }
            if matches!(state, StepState::Running(_)) {
                let tail = st.output_tail();
                if !tail.is_empty() {
                    // Its latest lines, each on one line: the box keeps
                    // its height as output comes.
                    let mut out = div().ml(px(24.)).px(px(8.)).py(px(6.)).rounded(px(6.)).bg(ui.t.surface_3).flex().flex_col().overflow_hidden();
                    let lines: Vec<&str> = tail.lines().collect();
                    for l in &lines[lines.len().saturating_sub(5)..] {
                        out = out.child(crate::kit::mono_text(l.to_string(), &ui.t).whitespace_nowrap().overflow_hidden().text_ellipsis());
                    }
                    row = row.child(out);
                }
            }
            list = list.child(row);
        }
        let mut foot = div().flex().items_center().gap(px(8.)).pt(px(4.));
        match st.summary() {
            None => {
                let done = st.steps.iter().filter(|(_, s)| matches!(s, StepState::Ended(_))).count();
                foot = foot.child(div().text_size(px(12.)).text_color(ui.t.text_2).child(format!("Running: {done} of {} steps done", st.steps.len())));
                foot = foot.child(div().flex_1());
                let (e, ac) = (m.id.clone(), key.to_string());
                foot = foot.child(crate::kit::text_button(SharedString::from(format!("extension-run-cancel-{}-{key}", m.id)), "Cancel", BtnKind::GhostSm, st.cancelled, &ui.t).tooltip(crate::kit::tooltip("Stop the running step; the rest are skipped")).on_click(cx.listener(move |_, _, _, cx| {
                    actions::runs(cx).read(cx).cancel(&e, &ac);
                })));
            }
            Some(s) => {
                let color = if st.failed() || st.cancelled { ui.t.amber } else { ui.t.green };
                foot = foot.child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(color).child(s));
            }
        }
        list.child(foot)
    }

    /// Run the confirmed action off the UI thread; one that ends without a
    /// failure is recorded (its checklist step stays ticked).
    fn run_action(&mut self, m: &Manifest, a: &Action, cx: &mut Context<Self>) {
        let Some((_, key, plan)) = self.exts.confirm.take() else { return };
        let c = connections::chosen(&m.id, self.exts.conn.get(&m.id).map(String::as_str), cx).filter(|_| !m.settings.is_empty());
        let ext = m.id.clone();
        let home = pitwall_core::paths::home();
        let runs = actions::runs(cx);
        let record = {
            let (m, action, conn) = (m.clone(), a.id.clone(), c.map(|c| c.id));
            move |st: &RunState, cx: &mut gpui::App| record_run(&m, &action, conn.as_deref(), st, cx)
        };
        runs.update(cx, |r, cx| r.start((&ext, &key), plan, home, actions::STEP_TIMEOUT, record, cx));
        cx.notify();
    }

    /// What a card's fields are kept under: its connection's id, or the
    /// extension's while it has none.
    fn scope(m: &Manifest, c: Option<&Account>) -> String {
        c.map(|c| c.id.clone()).unwrap_or_else(|| m.id.clone())
    }

    fn url_input(&mut self, m: &Manifest, c: Option<&Account>, s: &SettingDef, cx: &mut Context<Self>) -> Entity<TextInput> {
        let k = (Self::scope(m, c), s.key.clone());
        if let Some(i) = self.exts.inputs.get(&k) {
            return i.clone();
        }
        let text = c.and_then(|c| c.values.get(&s.key).cloned()).unwrap_or_default();
        let (ext, conn) = (m.id.clone(), c.map(|c| c.id.clone()));
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
                    if let Err(why) = connections::set_value(&ext, conn.as_deref(), &key.1, val, cx) {
                        v.exts.notes.insert(key.clone(), why);
                    }
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

    fn setting_title(ui: &Ui, s: &SettingDef) -> Div {
        let mut d = div().flex().items_center().gap(px(6.)).child(Self::step_title(ui, s.label.clone()));
        if !s.required {
            d = d.child(div().text_size(px(12.)).text_color(ui.t.text_3).child("optional"));
        }
        d
    }

    /// A `url` setting: its title, the field, then its hint or what's
    /// wrong with what was typed.
    fn url_row(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, s: &SettingDef, cx: &mut Context<Self>) -> Div {
        let input = self.url_input(m, c, s, cx);
        let k = (Self::scope(m, c), s.key.clone());
        let mut col = div().flex().flex_col().gap(px(5.)).child(Self::setting_title(ui, s)).child(div().max_w(px(520.)).child(input));
        if let Some(n) = self.exts.notes.get(&k) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else if let Some(h) = &s.hint {
            col = col.child(crate::kit::hint(h.clone(), &ui.t));
        }
        col
    }

    /// A secret: "Saved in your keychain" with Replace and Remove once
    /// set (never the value), else Paste from clipboard; its hint under.
    fn secret_row(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, s: &SettingDef, cx: &mut Context<Self>) -> Div {
        let k = (Self::scope(m, c), s.key.clone());
        let set = c.is_some_and(|c| c.secrets_set.contains(&s.key));
        let (ext, conn) = (m.id.clone(), c.map(|c| c.id.clone()));
        let busy = self.exts.busy.contains(&k);
        let paste = {
            let k = k.clone();
            crate::kit::text_button(SharedString::from(format!("extension-secret-paste-{}-{}", m.id, s.key)), if set { "Replace" } else { "Paste from clipboard" }, BtnKind::Small, busy, &ui.t)
                .tooltip(crate::kit::tooltip("Copy it first; Pitwall takes it from the clipboard and never shows it"))
                .on_click(cx.listener(move |v, _, _, cx| {
                    let text = crate::platform::files::paste_text(cx).map(|t| t.trim().to_string()).unwrap_or_default();
                    if text.is_empty() {
                        v.exts.notes.insert(k.clone(), "Copy it first: the clipboard has no text.".into());
                        cx.notify();
                        return;
                    }
                    if text.chars().any(char::is_control) || text.len() > 4096 {
                        v.exts.notes.insert(k.clone(), "The clipboard doesn't hold a key (it has line breaks or is too long).".into());
                        cx.notify();
                        return;
                    }
                    v.secret_change(k.clone(), &ext, conn.clone(), Some(text), cx);
                }))
        };
        let state = if busy {
            div().text_size(px(12.)).text_color(ui.t.text_3).child("Saving…")
        } else if set {
            div().flex().items_center().gap(px(5.)).text_size(px(12.)).text_color(ui.t.text_2).child(crate::kit::icon("lock", 12., ui.t.text_3)).child(secure_store())
        } else {
            div().text_size(px(12.)).text_color(ui.t.text_3).child("Not set")
        };
        let mut line = div().flex().flex_wrap().items_center().gap(px(8.)).child(state).child(div().flex_1()).child(paste);
        if set {
            let (k, ext, conn) = (k.clone(), m.id.clone(), c.map(|c| c.id.clone()));
            line = line.child(crate::kit::text_button(SharedString::from(format!("extension-secret-remove-{}-{}", m.id, s.key)), "Remove", BtnKind::GhostSm, busy, &ui.t).on_click(cx.listener(move |v, _, _, cx| v.secret_change(k.clone(), &ext, conn.clone(), None, cx))));
        }
        let mut col = div().flex().flex_col().gap(px(5.)).child(Self::setting_title(ui, s)).child(div().max_w(px(520.)).child(line));
        if let Some(n) = self.exts.notes.get(&k) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else if let Some(h) = &s.hint {
            col = col.child(crate::kit::hint(h.clone(), &ui.t));
        }
        col
    }

    fn secret_change(&mut self, k: (String, String), ext: &str, conn: Option<String>, secret: Option<String>, cx: &mut Context<Self>) {
        self.exts.busy.insert(k.clone());
        self.exts.notes.remove(&k);
        cx.notify();
        let this = cx.entity().downgrade();
        let key = k.1.clone();
        let done = move |err: Option<String>, cx: &mut gpui::App| {
            let _ = this.update(cx, |v, cx| {
                v.exts.busy.remove(&k);
                if let Some(e) = err {
                    v.exts.notes.insert(k.clone(), e);
                }
                cx.notify();
            });
        };
        connections::set_secret_in(ext, conn.as_deref(), &key, secret, cx, done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_fields_and_states_say_what_to_do() {
        assert_eq!(url_value("  "), Ok(None));
        assert_eq!(url_value("inbox.example.com").unwrap().as_deref(), Some("https://inbox.example.com"));
        assert_eq!(url_value("javascript:alert(1)").unwrap_err(), "That isn't a web address (like https://example.com).");
        let (w, _, t) = run_text(&Run::Stopped("It exited.".into()), None);
        assert_eq!(w, "stopped");
        assert!(t.contains("try again"));
        let (w, _, t) = run_text(&Run::Running, Some(&(ConnState::SignedOut, String::new())));
        assert_eq!(w, "signed out");
        assert!(!t.is_empty());
    }

    /// The checklist: required settings, then setup actions, then
    /// Connected; ticked as each is done.
    #[test]
    fn the_checklist_follows_the_manifest() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../extensions/demo-inbox/extension.json")).unwrap();
        let mut m = pitwall_core::extensions::parse(&text).unwrap();
        m.settings.push(SettingDef { key: "apiKey".into(), kind: SettingKind::Secret, label: "API key".into(), hint: None, placeholder: None, required: true });
        m.actions.push(Action { id: "setup".into(), label: "Set up".into(), description: "d".into(), setup: true, steps: vec![] });
        let mut prefs = ExtensionPrefs::default();
        let steps = |p: &ExtensionPrefs, c| checklist(&m, p, c);
        assert_eq!(
            steps(&prefs, false),
            [(Step::Setting("greeting".into()), true), (Step::Setting("apiKey".into()), false), (Step::Action("setup".into()), false), (Step::Connected, false)],
            "an optional setting counts as done"
        );
        prefs.secrets_set.push("apiKey".into());
        prefs.actions_done.push("setup".into());
        assert!(steps(&prefs, true).iter().all(|(_, d)| *d));
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
