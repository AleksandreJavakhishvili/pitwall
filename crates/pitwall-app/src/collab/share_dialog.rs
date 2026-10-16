//! The Share dialog (collaboration/sharing.md §5.4, ui.md §11): seven short
//! steps, one page each with Back / Next, every answer marked **Enforced**
//! or **Not guaranteed**; then the plain-language Summary, Try as guest
//! (the probe buttons, sent verbatim and only when clicked) and Go live.
//! Editing a share opens the same steps; Save shows the exact diff first,
//! widening lines in amber.
//!
//! Also Usage (the sidebar heading's gauge): per caller today and over 30
//! days against the limits, the share's agent time, the vendor-limit
//! banner, and per person: change their limit, Block.

use gpui::{
    div, prelude::*, px, App, AppContext, Context, Div, Entity, FontWeight, HighlightStyle, IntoElement, SharedString, StyledText, Subscription,
    Window,
};
use serde_json::json;

use pitwall_proto::api::collab::{
    capability_words, describe, AuditEntryView, ProbeResult, SensitiveFile, ShareChanged, ShareSummary, ShareView, SummaryTopic,
};
use pitwall_proto::collab::{
    Access, Audience, Capability, Identity, LimitOverride, NetworkAccess, Programs, Scope, ShareDiff, ShareSettings, ShareState, Start, TeamId,
};
use pitwall_proto::{method, AgentView};

use crate::kit::dialog::{body, foot, form_error};
use crate::kit::{check, chip_tone, radio, text_button, BtnKind, Modal, TextInput, Tone};
use crate::theme::{self, Theme, RADIUS};

/// The pages, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Who,
    Capability,
    See,
    Start,
    Access,
    OffLimits,
    Limits,
    Summary,
    Guest,
    /// Editing: the diff Save would apply.
    Diff,
}

pub const STEPS: [Step; 7] = [Step::Who, Step::Capability, Step::See, Step::Start, Step::Access, Step::OffLimits, Step::Limits];

impl Step {
    pub fn title(self) -> &'static str {
        match self {
            Step::Who => "Who",
            Step::Capability => "Capability",
            Step::See => "What it can see",
            Step::Start => "Start from this agent's conversation?",
            Step::Access => "Secrets and access",
            Step::OffLimits => "Off-limits topics",
            Step::Limits => "Limits",
            Step::Summary => "Summary",
            Step::Guest => "Try as guest",
            Step::Diff => "Save changes",
        }
    }

    /// Enforced by Pitwall or the OS sandbox, or only instructions (§9.1).
    pub fn enforced(self) -> Option<bool> {
        match self {
            Step::OffLimits => Some(false),
            Step::Summary | Step::Guest | Step::Diff => None,
            _ => Some(true),
        }
    }
}

/// The standard guest probes (the daemon's list, §5.4).
pub use pitwall_daemon::collab::service::PROBES;

/// Settings for a first share of `a`: the dialog's defaults.
pub fn defaults(a: &AgentView, exclude: Vec<String>) -> ShareSettings {
    ShareSettings {
        agent: a.id.clone(),
        slug: pitwall_daemon::collab::diff::slugify(&a.name),
        name: a.name.clone(),
        description: String::new(),
        state: ShareState::Draft,
        capability: Capability::Read,
        audience: Audience::default(),
        scope: Scope { repo: a.project.clone(), git_ref: a.branch.clone().unwrap_or_else(|| "HEAD".into()), exclude, ..Default::default() },
        start: Start::Fresh,
        access: Access { env: vec![], network: NetworkAccess::default(), account: a.account.clone(), programs: None },
        off_limits: String::new(),
        limits: Default::default(),
    }
}

fn list_of(text: &str) -> Vec<String> {
    text.split([',', '\n']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

fn number(text: &str, fallback: u32) -> u32 {
    text.trim().parse().unwrap_or(fallback)
}

pub struct Inputs {
    pub person: Entity<TextInput>,
    pub name: Entity<TextInput>,
    pub description: Entity<TextInput>,
    pub paths: Entity<TextInput>,
    pub summary: Entity<TextInput>,
    pub env: Entity<TextInput>,
    pub hosts: Entity<TextInput>,
    pub programs: Entity<TextInput>,
    pub off_limits: Entity<TextInput>,
    pub messages: Entity<TextInput>,
    pub sessions: Entity<TextInput>,
    pub timeout: Entity<TextInput>,
    pub minutes: Entity<TextInput>,
    pub queue: Entity<TextInput>,
    pub override_: Entity<TextInput>,
    pub guest: Entity<TextInput>,
}

pub struct ShareDialog {
    pub agent: AgentView,
    /// The share being edited (`None`: a new one, not saved yet).
    pub existing: Option<ShareView>,
    pub draft: ShareSettings,
    pub step: Step,
    pub inputs: Inputs,
    /// The scan of the picked scope.
    pub sensitive: Vec<SensitiveFile>,
    pub files: Option<u32>,
    pub summary: Option<ShareSummary>,
    pub diff: Option<ShareDiff>,
    pub probes: Vec<ProbeResult>,
    /// "Programs it may run": allowlist, denylist or the defaults.
    pub programs_mode: Option<bool>,
    pub busy: bool,
    pub error: Option<String>,
    /// What keeps the draft from going live (the daemon's check: the
    /// owner's own login, Gemini CLI without an API key, §6.3).
    pub live_blocks: Vec<String>,
    _subs: Vec<Subscription>,
}

/// Open Share… for agent `id` (editing when it is shared already).
pub fn open(id: &str, window: &mut Window, cx: &mut App) {
    let store = cx.try_global::<crate::agents::StoreHandle>().map(|h| h.0.clone());
    let Some(agent) = store.and_then(|s| s.read(cx).agents.iter().find(|a| a.id == id).cloned()) else { return };
    let existing = super::share_of(id, cx);
    let view = cx.new(|cx| ShareDialog::new(agent, existing, cx));
    view.update(cx, |d, cx| d.rescan(cx));
    super::show_dialog(view.into(), window, cx);
}

/// Open Usage for the share of agent `id`.
pub fn open_usage(id: &str, window: &mut Window, cx: &mut App) {
    let Some(share) = super::share_of(id, cx) else { return };
    let view = cx.new(|cx| UsageDialog::new(share, cx));
    super::show_dialog(view.into(), window, cx);
}

impl ShareDialog {
    pub fn new(agent: AgentView, existing: Option<ShareView>, cx: &mut Context<Self>) -> ShareDialog {
        let draft = existing.as_ref().map(|v| v.settings.clone()).unwrap_or_else(|| defaults(&agent, vec![]));
        let field = |cx: &mut Context<Self>, text: &str, ph: &str| cx.new(|cx| TextInput::new(cx, text, ph));
        let multi = |cx: &mut Context<Self>, text: &str, ph: &str, rows: usize| cx.new(|cx| TextInput::new(cx, text, ph).multiline(rows));
        let l = &draft.limits;
        let (programs_text, programs_mode) = match &draft.access.programs {
            Some(Programs::Allow(x)) => (x.join(", "), Some(true)),
            Some(Programs::Deny(x)) => (x.join(", "), Some(false)),
            None => (String::new(), None),
        };
        let summary_text = match &draft.start {
            Start::Summary { text, .. } => text.clone(),
            Start::Fresh => String::new(),
        };
        let inputs = Inputs {
            person: field(cx, "", "Add a person by login (bob@example.com), then Enter"),
            name: field(cx, &draft.name, "Name"),
            description: field(cx, &draft.description, "What to ask it about"),
            paths: field(cx, &draft.scope.paths.join(", "), "All tracked files (or: src/, docs/)"),
            summary: multi(cx, &summary_text, "A short summary for teammates: only this text reaches sessions", 5),
            env: field(cx, &draft.access.env.join(", "), "None (names, comma-separated)"),
            hosts: field(cx, &draft.access.network.allow.join(", "), "None beyond its model (e.g. registry.npmjs.org)"),
            programs: field(cx, &programs_text, "git, rg, cargo"),
            off_limits: multi(cx, &draft.off_limits, "Don't discuss pricing or the reorg", 4),
            messages: field(cx, &l.per_caller.messages_per_day.to_string(), "20"),
            sessions: field(cx, &l.sessions_running.to_string(), "2"),
            timeout: field(cx, &l.turn_timeout_min.to_string(), "15"),
            minutes: field(cx, &l.agent_minutes_per_day.to_string(), "120"),
            queue: field(cx, &l.queue.to_string(), "10"),
            override_: field(cx, "", "bob@example.com=50"),
            guest: field(cx, "", "Your own message, sent as a guest"),
        };
        let subs = vec![
            cx.subscribe(&inputs.person, |this, _, e: &crate::kit::InputEvent, cx| {
                if matches!(e, crate::kit::InputEvent::Submit) {
                    this.add_person(cx);
                }
            }),
            cx.subscribe(&inputs.override_, |this, _, e: &crate::kit::InputEvent, cx| {
                if matches!(e, crate::kit::InputEvent::Submit) {
                    this.add_override(cx);
                }
            }),
        ];
        let probes = existing.as_ref().map(|v| v.probes.clone()).unwrap_or_default();
        ShareDialog {
            agent,
            existing,
            draft,
            step: Step::Who,
            inputs,
            sensitive: vec![],
            files: None,
            summary: None,
            diff: None,
            probes,
            programs_mode,
            busy: false,
            error: None,
            live_blocks: Vec::new(),
            _subs: subs,
        }
    }

    pub fn editing(&self) -> bool {
        self.existing.is_some()
    }

    /// The scan of the scope as picked now; a new share leaves out what it finds.
    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(svc) = super::service(cx) else { return };
        let scope = self.draft.scope.clone();
        let first = self.existing.is_none() && self.sensitive.is_empty();
        let task = cx.background_executor().spawn(async move { svc.backend().scan(&scope) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |d, cx| {
                if first {
                    for f in &r.sensitive {
                        if !d.draft.scope.exclude.contains(&f.path) {
                            d.draft.scope.exclude.push(f.path.clone());
                        }
                    }
                }
                d.sensitive = r.sensitive;
                d.files = r.files;
                cx.notify();
            });
        })
        .detach();
    }

    /// The inputs into the draft.
    pub fn read_inputs(&mut self, cx: &App) {
        let text = |e: &Entity<TextInput>| e.read(cx).text().to_string();
        let d = &mut self.draft;
        let name = text(&self.inputs.name);
        if !name.trim().is_empty() {
            d.name = name.trim().into();
        }
        d.description = text(&self.inputs.description).trim().into();
        d.scope.paths = list_of(&text(&self.inputs.paths));
        d.access.env = list_of(&text(&self.inputs.env));
        d.access.network.allow = list_of(&text(&self.inputs.hosts));
        let programs = list_of(&text(&self.inputs.programs));
        d.access.programs = match self.programs_mode {
            Some(true) => Some(Programs::Allow(programs)),
            Some(false) => Some(Programs::Deny(programs)),
            None => None,
        };
        d.off_limits = text(&self.inputs.off_limits).trim().into();
        let l = &mut d.limits;
        l.per_caller.messages_per_day = number(&text(&self.inputs.messages), l.per_caller.messages_per_day);
        l.sessions_running = number(&text(&self.inputs.sessions), l.sessions_running).max(1);
        l.turn_timeout_min = number(&text(&self.inputs.timeout), l.turn_timeout_min).max(1);
        l.agent_minutes_per_day = number(&text(&self.inputs.minutes), l.agent_minutes_per_day);
        l.queue = number(&text(&self.inputs.queue), l.queue);
        if let Start::Summary { text: t, .. } = &mut d.start {
            *t = self.inputs.summary.read(cx).text().trim().to_string();
        }
    }

    fn add_person(&mut self, cx: &mut Context<Self>) {
        let login = self.inputs.person.read(cx).text().trim().to_string();
        if login.is_empty() {
            return;
        }
        let provider = super::snapshot(cx).settings.and_then(|s| s.networks.first().map(|n| n.id.provider().to_string())).unwrap_or_default();
        let mut id = Identity::new(&provider, &login);
        if login.contains('@') {
            id.email = Some(login.clone());
        }
        if !self.draft.audience.people.iter().any(|p| p.subject == login) {
            self.draft.audience.people.push(id);
        }
        self.inputs.person.update(cx, |i, cx| i.set_text("", cx));
        cx.notify();
    }

    fn add_override(&mut self, cx: &mut Context<Self>) {
        let text = self.inputs.override_.read(cx).text().trim().to_string();
        let Some((who, n)) = text.split_once('=') else {
            self.error = Some("A person's limit is login=N (bob@example.com=50)".into());
            cx.notify();
            return;
        };
        let Ok(n) = n.trim().parse::<u32>() else {
            self.error = Some("A limit is a number of messages a day".into());
            cx.notify();
            return;
        };
        let provider = super::snapshot(cx).settings.and_then(|s| s.networks.first().map(|n| n.id.provider().to_string())).unwrap_or_else(|| "tailscale".into());
        let key = if who.contains(':') { who.trim().to_string() } else { format!("{provider}:{}", who.trim()) };
        self.draft.limits.overrides.insert(key, LimitOverride { messages_per_day: Some(n), blocked: false });
        self.inputs.override_.update(cx, |i, cx| i.set_text("", cx));
        self.error = None;
        cx.notify();
    }

    /// Back / Next between the seven steps (and back from Summary).
    pub fn go(&mut self, to: Step, cx: &mut Context<Self>) {
        self.read_inputs(cx);
        self.error = None;
        if self.step == Step::See && to != Step::See {
            self.rescan(cx);
        }
        self.step = to;
        if to == Step::Summary {
            self.load_summary(cx);
        }
        cx.notify();
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        let i = STEPS.iter().position(|s| *s == self.step);
        let to = match i {
            Some(i) if i + 1 < STEPS.len() => STEPS[i + 1],
            _ => Step::Summary,
        };
        self.go(to, cx);
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        let to = match self.step {
            Step::Summary | Step::Diff => Step::Limits,
            Step::Guest => Step::Summary,
            s => {
                let i = STEPS.iter().position(|x| *x == s).unwrap_or(0);
                STEPS[i.saturating_sub(1)]
            }
        };
        self.go(to, cx);
    }

    fn load_summary(&mut self, cx: &mut Context<Self>) {
        self.check_live(cx);
        let this = cx.entity().downgrade();
        super::call::<ShareSummary>(method::SHARE_SUMMARY, json!({"settings": self.draft}), cx, move |r, cx| {
            let _ = this.update(cx, |d, cx| {
                match r {
                    Ok(s) => d.summary = Some(s),
                    Err(e) => d.error = Some(e.message),
                }
                cx.notify();
            });
        });
    }

    /// What keeps the draft from going live, as it is now.
    pub fn check_live(&mut self, cx: &mut Context<Self>) {
        let Some(svc) = super::service(cx) else { return };
        let draft = self.draft.clone();
        let task = cx.background_executor().spawn(async move { svc.backend().live_check(&draft).blocks });
        cx.spawn(async move |this, cx| {
            let blocks = task.await;
            let _ = this.update(cx, |d, cx| {
                d.live_blocks = blocks;
                cx.notify();
            });
        })
        .detach();
    }

    /// The account sessions run on (`None`: the agent's own login).
    fn pick_account(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.draft.access.account = id;
        self.error = None;
        self.check_live(cx);
        cx.notify();
    }

    /// "Add an account for sharing": a fresh login folder named Sharing,
    /// picked for this share, and an agent on it to log in with (the
    /// tool asks for its login on its first run there).
    fn add_sharing_account(&mut self, cx: &mut Context<Self>) {
        let kind = self.agent.kind.clone();
        let model = crate::accounts::model(cx);
        let taken: Vec<String> = model.read(cx).of_kind(&kind).into_iter().map(|a| a.name.to_lowercase()).collect();
        let name = (1..).map(|n| if n == 1 { "Sharing".to_string() } else { format!("Sharing {n}") }).find(|n| !taken.contains(&n.to_lowercase())).unwrap_or_default();
        match model.update(cx, |m, cx| m.add(&kind, &name, None, cx)) {
            Ok(a) => {
                self.pick_account(Some(a.id.clone()), cx);
                let Some(engine) = cx.try_global::<crate::main_screen::EngineHandle>().map(|e| e.0.clone()) else { return };
                let req = crate::accounts::login_request(&kind, &a.id, &pitwall_core::paths::home().to_string_lossy());
                let task = cx.background_executor().spawn(async move { pitwall_core::engine::lifecycle::create(&engine, req) });
                let this = cx.entity().downgrade();
                cx.spawn(async move |_, cx| {
                    if let Err(e) = task.await {
                        let _ = this.update(cx, |d, cx| {
                            d.error = Some(format!("Added {name}, but its login agent didn't start: {e}"));
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }

    /// Pass `var` (an API key variable) to the sessions.
    fn pass_key(&mut self, var: &str, cx: &mut Context<Self>) {
        self.read_inputs(cx);
        if !self.draft.access.env.iter().any(|v| v == var) {
            self.draft.access.env.push(var.to_string());
        }
        let text = self.draft.access.env.join(", ");
        self.inputs.env.update(cx, |i, cx| i.set_text(&text, cx));
        self.check_live(cx);
        cx.notify();
    }

    /// Save as it is now: create the draft, or (editing) apply the change.
    /// `then` runs once saved.
    pub(crate) fn save(&mut self, cx: &mut Context<Self>, then: impl FnOnce(&mut ShareDialog, &mut Context<ShareDialog>) + 'static) {
        self.read_inputs(cx);
        self.busy = true;
        self.error = None;
        cx.notify();
        let this = cx.entity().downgrade();
        let done = move |r: Result<ShareView, pitwall_proto::ErrorBody>, cx: &mut App| {
            let _ = this.update(cx, |d, cx| {
                d.busy = false;
                match r {
                    Ok(v) => {
                        d.draft = v.settings.clone();
                        d.live_blocks = v.live_blocks.clone();
                        d.existing = Some(v);
                        then(d, cx);
                    }
                    Err(e) => d.error = Some(e.message),
                }
                cx.notify();
            });
        };
        match &self.existing {
            None => super::call::<ShareView>(method::SHARE_CREATE, json!({"agent": self.agent.id, "patch": {"settings": self.draft}}), cx, done),
            Some(v) => super::call::<ShareChanged>(
                method::SHARE_SET,
                json!({"share": v.settings.slug, "patch": {"settings": self.draft}}),
                cx,
                move |r, cx| done(r.map(|c| c.share), cx),
            ),
        }
    }

    /// Save (editing): the diff first.
    pub(crate) fn review_changes(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.existing.clone() else { return };
        self.read_inputs(cx);
        let this = cx.entity().downgrade();
        super::call::<ShareChanged>(method::SHARE_SET, json!({"share": v.settings.slug, "patch": {"settings": self.draft}, "dryRun": true}), cx, move |r, cx| {
            let _ = this.update(cx, |d, cx| {
                match r {
                    Ok(c) => {
                        d.diff = Some(c.diff);
                        d.step = Step::Diff;
                    }
                    Err(e) => d.error = Some(e.message),
                }
                cx.notify();
            });
        });
    }

    pub(crate) fn try_as_guest(&mut self, cx: &mut Context<Self>) {
        let draft = self.existing.as_ref().is_none_or(|v| v.settings.state == ShareState::Draft);
        if !draft {
            self.error = Some("Try as guest works on a draft. Pause it and ask it from the Ask panel instead.".into());
            cx.notify();
            return;
        }
        self.save(cx, |d, cx| {
            d.step = Step::Guest;
            cx.notify();
        });
    }

    pub(crate) fn probe(&mut self, id: &'static str, message: Option<String>, cx: &mut Context<Self>) {
        let Some(v) = self.existing.clone() else { return };
        self.busy = true;
        cx.notify();
        let params = match message {
            Some(m) => json!({"share": v.settings.slug, "message": m}),
            None => json!({"share": v.settings.slug, "probes": true, "only": [id]}),
        };
        let this = cx.entity().downgrade();
        super::call::<Vec<ProbeResult>>(method::SHARE_GUEST, params, cx, move |r, cx| {
            let _ = this.update(cx, |d, cx| {
                d.busy = false;
                match r {
                    Ok(list) => d.probes.extend(list),
                    Err(e) => d.error = Some(e.message),
                }
                cx.notify();
            });
        });
    }

    pub(crate) fn go_live(&mut self, cx: &mut Context<Self>) {
        self.save(cx, |d, cx| {
            let Some(v) = d.existing.clone() else { return };
            d.busy = true;
            let this = cx.entity().downgrade();
            super::call::<ShareView>(method::SHARE_GO_LIVE, json!({"share": v.settings.slug}), cx, move |r, cx| {
                match r {
                    Ok(_) => super::close_dialog(cx),
                    Err(e) => {
                        let _ = this.update(cx, |d, cx| {
                            d.busy = false;
                            d.error = Some(e.message);
                            cx.notify();
                        });
                    }
                }
            });
        });
    }

    fn simple(&mut self, m: &'static str, cx: &mut Context<Self>) {
        let Some(v) = self.existing.clone() else { return };
        let this = cx.entity().downgrade();
        super::call::<serde_json::Value>(m, json!({"share": v.settings.slug}), cx, move |r, cx| match r {
            Ok(_) if m == method::SHARE_STOP => super::close_dialog(cx),
            Ok(v) => {
                let _ = this.update(cx, |d, cx| {
                    if let Ok(view) = serde_json::from_value::<ShareView>(v) {
                        d.draft = view.settings.clone();
                        d.existing = Some(view);
                    }
                    cx.notify();
                });
            }
            Err(e) => {
                let _ = this.update(cx, |d, cx| {
                    d.error = Some(e.message);
                    cx.notify();
                });
            }
        });
    }
}

/// A field's label with its Enforced / Not guaranteed chip.
pub fn field_head(text: &str, enforced: Option<bool>, t: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child(text.to_string()))
        .child(div().flex_1())
        .children(enforced.map(|on| if on { chip_tone("Enforced", Tone::Ok, t) } else { chip_tone("Not guaranteed", Tone::Warn, t) }))
}

fn hint(text: impl Into<SharedString>, t: &Theme) -> Div {
    div().text_size(px(12.)).text_color(t.text_3).child(text.into())
}

fn small_label(text: &str, t: &Theme) -> Div {
    div().text_size(px(12.)).text_color(t.text_3).child(text.to_string())
}

/// A sentence with its bold parts (`SummarySentence`).
pub fn sentence(s: &pitwall_proto::api::collab::SummarySentence, t: &Theme) -> Div {
    let text: String = s.parts.iter().map(|p| p.text.as_str()).collect();
    let mut at = 0;
    let mut bold = Vec::new();
    for p in &s.parts {
        if p.strong {
            bold.push((at..at + p.text.len(), HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() }));
        }
        at += p.text.len();
    }
    let color = if s.topic == SummaryTopic::Soft { t.amber } else { t.text };
    div().text_size(px(13.)).line_height(px(20.)).text_color(color).child(StyledText::new(text).with_highlights(bold))
}

/// One diff line: amber when it widens.
pub fn diff_line(l: &pitwall_proto::collab::ShareDiffLine, t: &Theme) -> Div {
    div()
        .px(px(8.))
        .py(px(3.))
        .rounded(RADIUS)
        .font_family(crate::kit::MONO_FONT)
        .text_size(px(12.))
        .text_color(if l.widens { t.amber } else { t.text_2 })
        .when(l.widens, |d| d.bg(t.amber_soft))
        .child(describe(l))
}

impl ShareDialog {
    fn toggle_team(&mut self, team: TeamId, cx: &mut Context<Self>) {
        let teams = &mut self.draft.audience.teams;
        if let Some(i) = teams.iter().position(|x| *x == team) {
            teams.remove(i);
        } else {
            teams.push(team);
        }
        cx.notify();
    }

    fn page(&mut self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let snap = super::snapshot(cx);
        let page = div().flex().flex_col().gap(px(10.));
        match self.step {
            Step::Who => {
                let mine: Vec<_> = snap.teams.iter().filter(|x| x.mine.is_some()).cloned().collect();
                let mut p = page.child(hint("Checked on every message against who the network says is calling, and current team membership.", t));
                p = p.child(small_label("Teams", t));
                if mine.is_empty() {
                    p = p.child(hint("You aren't in any team yet (People → Teams).", t));
                }
                for (i, team) in mine.iter().enumerate() {
                    let on = self.draft.audience.teams.contains(&team.team);
                    let id = team.team.clone();
                    p = p.child(
                        div()
                            .id(("who-team", i))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |d, _, _, cx| d.toggle_team(id.clone(), cx)))
                            .child(check(on, t))
                            .child(format!("team {} ({} people)", team.name, team.members))
                            .child(div().text_color(t.text_4).font_family(crate::kit::MONO_FONT).text_size(px(11.)).child(team.fingerprint.clone())),
                    );
                }
                p = p.child(small_label("People", t));
                for (i, who) in self.draft.audience.people.clone().into_iter().enumerate() {
                    let subject = who.subject.clone();
                    p = p.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().child(who.email.clone().unwrap_or(who.subject.clone())))
                            .child(text_button(("who-rm", i), "Remove", BtnKind::Link, false, t).on_click(cx.listener(move |d, _, _, cx| {
                                d.draft.audience.people.retain(|x| x.subject != subject);
                                cx.notify();
                            }))),
                    );
                }
                p = p.child(self.inputs.person.clone());
                if let Some(s) = &snap.settings {
                    for (i, n) in s.networks.iter().enumerate() {
                        let on = self.draft.audience.networks.contains(&n.id);
                        let id = n.id.clone();
                        let name = n.id.as_str().split_once(':').map_or(n.id.as_str(), |(_, x)| x).to_string();
                        p = p.child(
                            div()
                                .id(("who-net", i))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .cursor_pointer()
                                .text_color(t.amber)
                                .on_click(cx.listener(move |d, _, _, cx| {
                                    let nets = &mut d.draft.audience.networks;
                                    if let Some(i) = nets.iter().position(|x| *x == id) {
                                        nets.remove(i);
                                    } else {
                                        nets.push(id.clone());
                                    }
                                    cx.notify();
                                }))
                                .child(check(on, t))
                                .child(format!("Everyone on {name}")),
                        );
                    }
                }
                p
            }
            Step::Capability => {
                let rows: [(Capability, &str, bool); 3] = [
                    (Capability::Read, "Replies only: the sandbox lets it read its copy, never write.", true),
                    (Capability::Write, "Changes come back as a patch. Comes with “Can change code” (not in this version).", false),
                    (Capability::WriteApproval, "Changes wait for your Send. Comes with “Can change code” (not in this version).", false),
                ];
                let mut p = page.child(hint("Enforced by the OS sandbox around the session, for every agent kind.", t));
                for (i, (c, what, can)) in rows.into_iter().enumerate() {
                    let on = self.draft.capability == c;
                    p = p.child(
                        div()
                            .id(("cap", i))
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .p(px(8.))
                            .rounded(RADIUS)
                            .border_1()
                            .border_color(if on { t.text_3 } else { t.line })
                            .when(!can, |d| d.opacity(0.5))
                            .when(can, |d| {
                                d.cursor_pointer().on_click(cx.listener(move |d, _, _, cx| {
                                    d.draft.capability = c;
                                    cx.notify();
                                }))
                            })
                            .child(div().pt(px(2.)).child(radio(on, t)))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().text_color(t.text).child(upper_first(capability_words(c))))
                                    .child(hint(what.to_string(), t)),
                            ),
                    );
                }
                p
            }
            Step::See => {
                let repo = crate::kit::project_name(&self.draft.scope.repo).to_string();
                let mut p = page
                    .child(hint(format!("Each conversation gets a copy of {repo} with only these paths, minus what's ticked below, and no git history."), t))
                    .child(small_label("Folders (relative to the repository)", t))
                    .child(self.inputs.paths.clone());
                p = p.child(div().flex().items_center().gap(px(8.)).child(small_label("Sensitive files found", t)).child(div().flex_1()).child(
                    text_button("rescan", "Scan again", BtnKind::Link, false, t).on_click(cx.listener(|d, _, _, cx| {
                        d.read_inputs(cx);
                        d.rescan(cx);
                    })),
                ));
                if self.sensitive.is_empty() {
                    p = p.child(hint("None found in this scope.", t));
                }
                for (i, f) in self.sensitive.clone().into_iter().enumerate() {
                    let on = self.draft.scope.exclude.contains(&f.path);
                    let path = f.path.clone();
                    p = p.child(
                        div()
                            .id(("sensitive", i))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |d, _, _, cx| {
                                let ex = &mut d.draft.scope.exclude;
                                if let Some(i) = ex.iter().position(|x| *x == path) {
                                    ex.remove(i);
                                } else {
                                    ex.push(path.clone());
                                }
                                cx.notify();
                            }))
                            .child(check(on, t))
                            .child(div().font_family(crate::kit::MONO_FONT).text_size(px(12.)).child(f.path.clone()))
                            .child(hint(f.why.clone(), t)),
                    );
                }
                if let Some(n) = self.files {
                    p = p.child(hint(format!("{n} files in the copy."), t));
                }
                let unc = self.draft.scope.uncommitted;
                p.child(
                    div()
                        .id("uncommitted")
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .on_click(cx.listener(|d, _, _, cx| {
                            d.draft.scope.uncommitted = !d.draft.scope.uncommitted;
                            cx.notify();
                        }))
                        .child(check(unc, t))
                        .child("Include uncommitted changes"),
                )
                .child(div().flex().items_center().gap(px(8.)).opacity(0.5).child(check(false, t)).child("Include git history (not in this version)"))
            }
            Step::Start => {
                let fresh = self.draft.start == Start::Fresh;
                page.child(hint("Shared sessions never see this agent's conversation; at most a summary you approved.", t))
                    .child(
                        div()
                            .id("start-fresh")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(|d, _, _, cx| {
                                d.draft.start = Start::Fresh;
                                cx.notify();
                            }))
                            .child(radio(fresh, t))
                            .child("Fresh (recommended)"),
                    )
                    .child(
                        div()
                            .id("start-summary")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(|d, _, _, cx| {
                                if d.draft.start == Start::Fresh {
                                    d.draft.start = Start::Summary { text: String::new(), approved_at: String::new() };
                                }
                                cx.notify();
                            }))
                            .child(radio(!fresh, t))
                            .child("A short summary I write and approve"),
                    )
                    .when(!fresh, |d| {
                        let approved = matches!(&self.draft.start, Start::Summary { approved_at, .. } if !approved_at.is_empty());
                        d.child(self.inputs.summary.clone()).child(
                            div().flex().items_center().gap(px(8.)).child(
                                text_button("approve-summary", if approved { "Approved ✓" } else { "Approve" }, BtnKind::Small, false, t).on_click(cx.listener(|d, _, _, cx| {
                                    let text = d.inputs.summary.read(cx).text().trim().to_string();
                                    let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                                    d.draft.start = Start::Summary { text, approved_at: at };
                                    cx.notify();
                                })),
                            )
                            .child(hint("Only the approved text reaches sessions.", t)),
                        )
                    })
            }
            Step::Access => {
                let mode = self.programs_mode;
                let seg = crate::kit::seg(
                    "programs-mode",
                    vec![
                        crate::kit::SegItem::new("Defaults", mode.is_none()),
                        crate::kit::SegItem::new("Allow only", mode == Some(true)),
                        crate::kit::SegItem::new("Deny", mode == Some(false)),
                    ],
                    t,
                    {
                        let this = cx.entity().downgrade();
                        std::rc::Rc::new(move |i, _, cx| {
                            let _ = this.update(cx, |d, cx| {
                                d.programs_mode = [None, Some(true), Some(false)][i];
                                cx.notify();
                            });
                        })
                    },
                );
                let login = self.login_part(t, cx);
                page.child(hint("A clean environment: none of your variables, tokens or logins, no SSH agent, a session-local HOME. Network only to its model through Pitwall's proxy.", t))
                    .child(login)
                    .child(small_label("Environment variables to pass", t))
                    .child(self.inputs.env.clone())
                    .child(small_label("Extra network hosts", t))
                    .child(self.inputs.hosts.clone())
                    .child(div().flex().items_center().gap(px(8.)).child(small_label("Programs it may run", t)).child(div().flex_1()).child(seg))
                    .when(mode.is_some(), |d| d.child(self.inputs.programs.clone()))
            }
            Step::OffLimits => page
                .child(hint("Written into the session's instructions. The agent may not follow them: not guaranteed.", t))
                .child(small_label("Name and description (what callers see)", t))
                .child(self.inputs.name.clone())
                .child(self.inputs.description.clone())
                .child(small_label("Off-limits topics and the role", t))
                .child(self.inputs.off_limits.clone()),
            Step::Limits => {
                let row = |label: &str, input: &Entity<TextInput>| div().flex().items_center().gap(px(10.)).child(div().w(px(220.)).child(small_label(label, t))).child(div().w(px(110.)).child(input.clone()));
                let mut p = page
                    .child(hint("Enforced by Pitwall for every caller, on both fronts.", t))
                    .child(row("Messages per person per day", &self.inputs.messages))
                    .child(row("Conversations at once", &self.inputs.sessions))
                    .child(row("Minutes per turn", &self.inputs.timeout))
                    .child(row("Agent minutes per day, everyone", &self.inputs.minutes))
                    .child(row("Waiting messages before “busy”", &self.inputs.queue))
                    .child(small_label("Per-person limits", t));
                for (i, (who, o)) in self.draft.limits.overrides.clone().into_iter().enumerate() {
                    let key = who.clone();
                    let what = if o.blocked { "blocked".to_string() } else { format!("{} a day", o.messages_per_day.unwrap_or(self.draft.limits.per_caller.messages_per_day)) };
                    p = p.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().child(who.split_once(':').map_or(who.clone(), |(_, x)| x.to_string())))
                            .child(hint(what, t))
                            .child(text_button(("ovr-rm", i), "Remove", BtnKind::Link, false, t).on_click(cx.listener(move |d, _, _, cx| {
                                d.draft.limits.overrides.remove(&key);
                                cx.notify();
                            }))),
                    );
                }
                p.child(self.inputs.override_.clone())
            }
            Step::Summary => {
                let mut p = page.child(hint("Made by Pitwall from your answers: fixed sentences, not written by a model.", t));
                match &self.summary {
                    None => p = p.child(hint("…", t)),
                    Some(s) => {
                        for (i, x) in s.sentences.iter().enumerate() {
                            p = p.child(sentence(x, t).id(("summary", i)).debug_selector(move || format!("summary-{i}")));
                        }
                    }
                }
                p = p.children(self.blocks(t));
                if self.editing() {
                    p = p.child(hint("Changes apply from each conversation's next message; removing people, pausing and stopping apply at once.", t));
                } else {
                    p = p.child(hint("Until you press Go live this is a draft: nobody else can see it.", t));
                }
                p
            }
            Step::Guest => {
                let read_only = self.draft.capability == Capability::Read;
                let mut buttons = div().flex().flex_wrap().gap(px(6.));
                for (id, text) in PROBES.iter().copied() {
                    if id == "change-readme" && !read_only {
                        continue;
                    }
                    buttons = buttons.child(text_button(SharedString::from(format!("probe-{id}")), text, BtnKind::GhostSm, self.busy, t).on_click(cx.listener(move |d, _, _, cx| d.probe(id, None, cx))));
                }
                let mut p = page.child(hint("The same pipeline as a real caller (copy, sandbox, clean environment), asked from here. Each probe is sent verbatim, only when you click it.", t));
                if !self.live_blocks.is_empty() {
                    p = p.child(amber("Your guest tries run on this login as it is. Going live needs what's below.", t)).children(self.blocks(t));
                }
                let mut p = p
                    .child(buttons)
                    .child(div().flex().gap(px(6.)).child(div().flex_1().child(self.inputs.guest.clone())).child(
                        text_button("guest-send", "Send", BtnKind::Small, self.busy, t).on_click(cx.listener(|d, _, _, cx| {
                            let m = d.inputs.guest.read(cx).text().trim().to_string();
                            if !m.is_empty() {
                                d.inputs.guest.update(cx, |i, cx| i.set_text("", cx));
                                d.probe("message", Some(m), cx);
                            }
                        })),
                    ));
                for r in self.probes.iter().rev() {
                    p = p.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .p(px(8.))
                            .rounded(RADIUS)
                            .bg(t.surface_2)
                            .child(div().text_color(t.text_3).text_size(px(12.)).child(format!("» {}", r.message)))
                            .child(div().text_size(px(12.5)).child(r.reply.clone().unwrap_or_else(|| "(no reply)".into())))
                            .children(r.tried.iter().map(|x| div().font_family(crate::kit::MONO_FONT).text_size(px(11.)).text_color(t.text_3).child(format!("tried: {x}")))),
                    );
                }
                p
            }
            Step::Diff => {
                let mut p = page;
                match &self.diff {
                    Some(d) if d.0.is_empty() => p = p.child(hint("Nothing changed.", t)),
                    Some(d) => {
                        if d.widens() {
                            p = p.child(hint("Amber lines widen who can reach the agent or what it can do.", t));
                        }
                        for (i, l) in d.0.iter().enumerate() {
                            let tag = if l.widens { "widens" } else { "narrows" };
                            p = p.child(diff_line(l, t).id(("diff-line", i)).debug_selector(move || format!("diff-line-{i}-{tag}")));
                        }
                    }
                    None => p = p.child(hint("…", t)),
                }
                p
            }
        }
    }
}

/// An amber line (a warning the owner should read).
fn amber(text: impl Into<SharedString>, t: &Theme) -> Div {
    div().px(px(8.)).py(px(5.)).rounded(RADIUS).bg(t.amber_soft).text_size(px(12.5)).line_height(px(18.)).text_color(t.amber).child(text.into())
}

/// A kind's definition: the engine's (with the user's overrides), else
/// the built-in one.
fn kind_def(id: &str, cx: &App) -> Option<pitwall_core::kind::AgentKind> {
    if let Some(e) = cx.try_global::<crate::main_screen::EngineHandle>() {
        if let Some(k) = e.0.kinds().find(id) {
            return Some(k);
        }
    }
    pitwall_core::kind::load_kinds(std::path::Path::new("")).into_iter().find(|k| k.id == id)
}

impl ShareDialog {
    /// What keeps it from going live, one amber line each.
    fn blocks(&self, t: &Theme) -> Vec<Div> {
        self.live_blocks.iter().enumerate().map(|(i, b)| amber(b.clone(), t).debug_selector(move || format!("live-block-{i}"))).collect()
    }

    /// Secrets and access → the login sessions run on (§6.3): a separate
    /// account for kinds with accounts, an API key for Gemini CLI.
    fn login_part(&mut self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let mut p = div().flex().flex_col().gap(px(6.));
        let Some(kind) = kind_def(&self.agent.kind, cx) else { return p };
        let share = kind.share.clone().unwrap_or_default();
        if !share.api_key_env.is_empty() {
            let dir = share.login_dir.clone().unwrap_or_else(|| "its own folder".into());
            let passed = share.api_key_env.iter().find(|v| self.draft.access.env.contains(v)).cloned();
            p = p.child(small_label("Agent login", t));
            p = match passed {
                Some(var) => p.child(hint(format!("Sessions log in with the API key in {var}, read from your environment when a session starts. {dir} is never used."), t)),
                None => {
                    let mut row = div().flex().flex_wrap().gap(px(6.));
                    for (i, var) in share.api_key_env.iter().enumerate() {
                        let v = var.clone();
                        row = row.child(text_button(("pass-key", i), format!("Pass {var}"), BtnKind::Small, false, t).on_click(cx.listener(move |d, _, _, cx| d.pass_key(&v, cx))));
                    }
                    p.child(amber(format!("{} keeps its login in {dir}, which shared sessions can't use; add an API key for sharing.", kind.name), t).debug_selector(|| "login-needs-key".into()))
                        .child(row)
                }
            };
            return p;
        }
        if kind.account_env.is_none() {
            return p;
        }
        let model = crate::accounts::model(cx);
        let accounts = model.read(cx).of_kind(&kind.id);
        let mine = self.agent.account.clone().filter(|a| a != pitwall_core::accounts::DEFAULT);
        let current = self.draft.access.account.clone().or_else(|| mine.clone()).filter(|a| a != pitwall_core::accounts::DEFAULT);
        let own_dir = kind.account_home.clone().unwrap_or_default();
        p = p
            .child(small_label("Agent account", t))
            .child(hint(
                "A shared session can read and write its account's folder, and that folder holds the account's other conversations. Sharing needs a separate account, so only shared sessions live there.",
                t,
            ));
        let row = |id: SharedString, on: bool, label: String, note: Option<String>, t: &Theme| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .child(radio(on, t))
                .child(label)
                .children(note.map(|n| hint(n, t)))
        };
        p = p.child(
            row("account-own".into(), current.is_none(), format!("Your own login ({own_dir})"), Some("guest tries only".into()), t)
                .on_click(cx.listener(|d, _, _, cx| d.pick_account(Some(pitwall_core::accounts::DEFAULT.into()), cx))),
        );
        for a in accounts {
            let on = current.as_deref() == Some(a.id.as_str());
            let note = (mine.as_deref() == Some(a.id.as_str())).then(|| "this agent's own account: guest tries only".to_string());
            let id = a.id.clone();
            p = p.child(
                row(SharedString::from(format!("account-{}", a.id)), on, a.name.clone(), note, t)
                    .on_click(cx.listener(move |d, _, _, cx| d.pick_account(Some(id.clone()), cx))),
            );
        }
        let separate = current.as_ref().is_some_and(|c| mine.as_ref() != Some(c));
        if !separate {
            p = p.child(amber("Fine for trying it as a guest; going live needs a separate account.", t).debug_selector(|| "login-own-account".into()));
        }
        p.child(
            div().flex().child(
                text_button("add-sharing-account", "Add an account for sharing…", BtnKind::Small, !model.read(cx).live(), t)
                    .on_click(cx.listener(|d, _, _, cx| d.add_sharing_account(cx))),
            ),
        )
    }
}

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

impl gpui::Render for ShareDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).float();
        let title = if self.editing() { format!("Sharing {}", self.agent.name) } else { format!("Share {}", self.agent.name) };
        let step_no = STEPS.iter().position(|s| *s == self.step);
        let mut head = div().flex().items_center().gap(px(8.));
        if let Some(i) = step_no {
            head = head.child(div().text_size(px(12.)).text_color(t.text_3).font_family(crate::kit::MONO_FONT).child(format!("{} / {}", i + 1, STEPS.len())));
        }
        let page = self.page(&t, cx);
        let content = body()
            .child(field_head(self.step.title(), self.step.enforced(), &t).when(step_no.is_some(), |d| d.child(head)))
            .child(page)
            .when_some(self.error.clone(), |d, e| d.child(form_error(e, &t)));
        let mut footer = foot(&t);
        let busy = self.busy;
        if let Some(v) = &self.existing {
            let state = v.settings.state;
            if state != ShareState::Draft {
                footer = footer.child(text_button("stop-sharing", "Stop sharing", BtnKind::Danger, busy, &t).on_click(cx.listener(|d, _, _, cx| d.simple(method::SHARE_STOP, cx))));
                footer = footer.child(if state == ShareState::Live {
                    text_button("pause", "Pause", BtnKind::Ghost, busy, &t).on_click(cx.listener(|d, _, _, cx| d.simple(method::SHARE_PAUSE, cx)))
                } else {
                    text_button("resume", "Resume", BtnKind::Ghost, busy, &t).on_click(cx.listener(|d, _, _, cx| d.simple(method::SHARE_RESUME, cx)))
                });
            }
        }
        footer = footer.child(div().flex_1());
        if self.step != Step::Who {
            footer = footer.child(text_button("back", "Back", BtnKind::Ghost, busy, &t).on_click(cx.listener(|d, _, _, cx| d.back(cx))));
        }
        let live = self.existing.as_ref().is_some_and(|v| v.settings.state != ShareState::Draft);
        let blocked = !self.live_blocks.is_empty();
        footer = match self.step {
            Step::Summary => {
                let mut f = footer;
                if !live {
                    f = f.child(text_button("try-guest", "Try as guest", BtnKind::Ghost, busy, &t).on_click(cx.listener(|d, _, _, cx| d.try_as_guest(cx))));
                }
                if self.editing() {
                    f = f.child(text_button("save", "Save…", BtnKind::Primary, busy, &t).on_click(cx.listener(|d, _, _, cx| d.review_changes(cx))));
                } else {
                    f = f.child(text_button("save-draft", "Save draft", BtnKind::Ghost, busy, &t).on_click(cx.listener(|d, _, _, cx| d.save(cx, |_, _| {}))));
                }
                if !live {
                    f = f.child(text_button("go-live", "Go live", BtnKind::Primary, busy || blocked, &t).on_click(cx.listener(|d, _, _, cx| d.go_live(cx))));
                }
                f
            }
            Step::Guest => footer.child(text_button("go-live", "Go live", BtnKind::Primary, busy || blocked, &t).on_click(cx.listener(|d, _, _, cx| d.go_live(cx)))),
            Step::Diff => {
                let any = self.diff.as_ref().is_some_and(|d| !d.0.is_empty());
                footer.child(text_button("apply", "Apply", BtnKind::Primary, busy || !any, &t).on_click(cx.listener(|d, _, _, cx| {
                    d.save(cx, |_, cx| {
                        super::close_dialog(cx);
                    })
                })))
            }
            _ => footer.child(text_button("next", "Next", BtnKind::Primary, busy, &t).on_click(cx.listener(|d, _, _, cx| d.next(cx)))),
        };
        let scroll = crate::kit::dialog::scrolling("share-body", content, &t);
        Modal::new("share-dialog", 600., |_, cx| super::close_dialog(cx))
            .title(title)
            .motion(theme::motion_on(cx))
            .render(&t, div().flex().flex_col().min_h_0().max_h(px(640.)).child(scroll).child(footer))
    }
}

// ------------------------------------------------------------ Usage

pub struct UsageDialog {
    pub share: ShareView,
    limit: Entity<TextInput>,
    error: Option<String>,
    /// The audit log's lines about this share, newest first (§11 Usage).
    pub audit: Vec<AuditEntryView>,
}

/// Audit lines shown in Usage.
const AUDIT_SHOWN: u32 = 50;

impl UsageDialog {
    pub fn new(share: ShareView, cx: &mut Context<Self>) -> UsageDialog {
        let mut d = UsageDialog { share, limit: cx.new(|cx| TextInput::new(cx, "", "bob@example.com=50")), error: None, audit: Vec::new() };
        d.load_audit(cx);
        d
    }

    /// The audit log filtered to this share (`collab/audit.jsonl`).
    pub fn load_audit(&mut self, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let params = json!({"share": self.share.settings.slug, "limit": AUDIT_SHOWN});
        super::call::<Vec<AuditEntryView>>(method::SHARE_AUDIT, params, cx, move |r, cx| {
            let _ = this.update(cx, |d, cx| {
                if let Ok(lines) = r {
                    d.audit = lines;
                }
                cx.notify();
            });
        });
    }

    fn block(&mut self, person: String, blocked: bool, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let slug = self.share.settings.slug.clone();
        super::call::<serde_json::Value>(method::SHARE_BLOCK, json!({"person": person, "share": slug, "blocked": blocked}), cx, move |r, cx| {
            let _ = this.update(cx, |d, cx| {
                d.error = r.err().map(|e| e.message);
                d.reload(cx);
            });
        });
    }

    fn set_limit(&mut self, cx: &mut Context<Self>) {
        let text = self.limit.read(cx).text().trim().to_string();
        let Some((who, n)) = text.split_once('=') else {
            self.error = Some("login=N (bob@example.com=50)".into());
            cx.notify();
            return;
        };
        let Ok(n) = n.trim().parse::<u32>() else {
            self.error = Some("A limit is a number of messages a day".into());
            cx.notify();
            return;
        };
        let this = cx.entity().downgrade();
        let slug = self.share.settings.slug.clone();
        let patch = json!({"overrides": {who.trim(): {"messagesPerDay": n}}});
        super::call::<ShareChanged>(method::SHARE_SET, json!({"share": slug, "patch": patch}), cx, move |r, cx| {
            let _ = this.update(cx, |d, cx| {
                match r {
                    Ok(c) => {
                        d.share = c.share;
                        d.error = None;
                        d.limit.update(cx, |i, cx| i.set_text("", cx));
                    }
                    Err(e) => d.error = Some(e.message),
                }
                cx.notify();
            });
        });
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        super::call::<ShareView>(method::SHARE_GET, json!({"share": self.share.settings.slug}), cx, move |r, cx| {
            let _ = this.update(cx, |d, cx| {
                if let Ok(v) = r {
                    d.share = v;
                }
                cx.notify();
            });
        });
    }
}

impl gpui::Render for UsageDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).float();
        let u = self.share.usage.clone();
        let l = self.share.settings.limits.clone();
        let mut content = body();
        if let Some(until) = &u.limited_until {
            content = content.child(
                div().px(px(10.)).py(px(8.)).rounded(RADIUS).bg(t.amber_soft).text_color(t.amber).text_size(px(12.5)).child(format!(
                    "Limited by {} until {until}: new messages are refused with “agent_limit”.",
                    if self.share.kind_name.is_empty() { "the agent's vendor".into() } else { self.share.kind_name.clone() }
                )),
            );
        }
        if u.agent_minutes_today >= l.agent_minutes_per_day {
            content = content.child(amber(
                "The agent time for today is used up: new messages are refused with “share_total” until midnight.",
                &t,
            ));
        }
        content = content.child(
            div()
                .flex()
                .gap(px(16.))
                .child(stat("Agent time today", format!("{} / {} min", u.agent_minutes_today, l.agent_minutes_per_day), &t))
                .child(stat("Running", format!("{} / {}", u.running, l.sessions_running), &t))
                .child(stat("Each person", format!("{} msg/day", l.per_caller.messages_per_day), &t)),
        );
        let header = div()
            .flex()
            .gap(px(8.))
            .text_size(px(11.))
            .text_color(t.text_4)
            .child(div().flex_1().child("PERSON"))
            .child(div().w(px(90.)).child("TODAY"))
            .child(div().w(px(110.)).child("30 DAYS"))
            .child(div().w(px(70.)));
        content = content.child(header);
        if u.callers.is_empty() {
            content = content.child(div().text_size(px(12.)).text_color(t.text_3).child("Nobody has asked yet."));
        }
        for (i, c) in u.callers.iter().enumerate() {
            let person = c.person.clone();
            let blocked = c.blocked;
            let tokens = c.today.tokens.map(|n| format!(" · {n} tok")).unwrap_or_default();
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .child(div().flex_1().min_w_0().overflow_hidden().child(c.login.clone()).when(blocked, |d| d.text_color(t.text_4)))
                    .child(div().w(px(90.)).font_family(crate::kit::MONO_FONT).text_size(px(11.5)).child(format!("{}/{} · {}m", c.today.messages, c.messages_per_day, c.today.minutes)))
                    .child(div().w(px(110.)).font_family(crate::kit::MONO_FONT).text_size(px(11.5)).child(format!("{} msg · {} res{tokens}", c.days30.messages, c.days30.results)))
                    .child(
                        div().w(px(70.)).child(
                            text_button(("usage-block", i), if blocked { "Unblock" } else { "Block" }, BtnKind::Link, false, &t)
                                .on_click(cx.listener(move |d, _, _, cx| d.block(person.clone(), !blocked, cx))),
                        ),
                    ),
            );
        }
        content = content
            .child(div().text_size(px(12.)).text_color(t.text_3).child("Change a person's daily limit"))
            .child(div().flex().gap(px(6.)).child(div().flex_1().child(self.limit.clone())).child(
                text_button("usage-limit", "Set", BtnKind::Small, false, &t).on_click(cx.listener(|d, _, _, cx| d.set_limit(cx))),
            ))
            .when_some(self.error.clone(), |d, e| d.child(form_error(e, &t)));
        content = content.child(crate::kit::label_t("AUDIT LOG", t.text_3, 11., 0.08));
        if self.audit.is_empty() {
            content = content.child(hint("Nothing recorded for this share yet.", &t));
        }
        for (i, a) in self.audit.iter().enumerate() {
            content = content.child(audit_row(i, a, &t));
        }
        let scroll = crate::kit::dialog::scrolling("usage-body", content, &t);
        Modal::new("usage-dialog", 560., |_, cx| super::close_dialog(cx))
            .title(format!("Usage · {}", self.share.address))
            .motion(theme::motion_on(cx))
            .render(&t, div().flex().flex_col().min_h_0().max_h(px(640.)).child(scroll))
    }
}

/// One audit line: when, who, what, how it ended (refusals and failures amber).
fn audit_row(i: usize, a: &AuditEntryView, t: &Theme) -> Div {
    let when = chrono::DateTime::parse_from_rfc3339(&a.at)
        .map(|d| d.with_timezone(&chrono::Local).format("%b %-d %H:%M").to_string())
        .unwrap_or_else(|_| a.at.clone());
    let who = match a.who.as_str() {
        "owner" => "you".to_string(),
        w => w.split_once(':').map_or(w, |(_, s)| s).to_string(),
    };
    let front = a.front.as_deref().map(|f| format!(" ({})", f.to_uppercase())).unwrap_or_default();
    let bad = a.outcome.starts_with("refused") || a.outcome.starts_with("failed");
    div()
        .debug_selector(move || format!("audit-{i}"))
        .flex()
        .gap(px(8.))
        .text_size(px(11.5))
        .font_family(crate::kit::MONO_FONT)
        .child(div().w(px(92.)).flex_none().text_color(t.text_4).child(when))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(t.text_2).child(format!("{who}{front} · {}", a.method)))
        .child(div().flex_none().text_color(if bad { t.amber } else { t.text_3 }).child(a.outcome.clone()))
}

fn stat(label: &str, value: String, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().text_size(px(11.)).text_color(t.text_4).child(label.to_string()))
        .child(div().font_family(crate::kit::MONO_FONT).text_size(px(13.)).child(value))
}
