//! `pitwall extension list|status|log|…`: an agent (the Race Engineer, or
//! any) sees how the user's extensions stand and moves their setup forward
//! (docs/spec/extensions.md, "Setup by an agent").
//!
//! - Reading needs nothing: [`ExtList`], [`ExtStatus`] (the checklist
//!   Settings shows, as data, with [`ExtNext`]: the single next step) and
//!   [`ExtLog`] (the card's Log, secrets already taken out).
//! - Acting ([`ExtAct`]) always waits for the user's approval in Pitwall,
//!   which says exactly what will run; the same code as Settings' buttons
//!   then runs it, and the answer ([`ExtActed`]) says how it went.
//! - Secrets never cross: a connection says only *whether* each secret is
//!   set ([`ExtConnection::secrets`]); [`ExtSecret`] only shows the user
//!   the field in Pitwall. A first run's sign-in link is opened in the
//!   user's browser and never comes back.
//!
//! Serde only. [`next_of`] picks the next step from a status, so the app
//! and the tests agree on it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The shape of these answers.
pub const EXT_PROTO: u32 = 1;

/// The most log lines `extension.log` returns.
pub const MAX_LOG_LINES: usize = 500;

/// `extension.status`, `extension.checkUpdates`: one extension (by id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtRef {
    pub id: String,
    /// The connection whose checklist to show (name or id); default: the
    /// one Settings shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
}

/// `extension.log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtLogRequest {
    pub id: String,
    /// The newest this many (default 100, at most [`MAX_LOG_LINES`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
}

/// `extension.secret`: show the user the secret's field in Pitwall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtSecret {
    pub id: String,
    /// The secret setting's key.
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// Whose terminal the Settings tile goes next to (default: the caller's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// One extension in `extension.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtListItem {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// In Pitwall's extensions folder (else: available to install).
    pub installed: bool,
    pub on: bool,
    /// Where it came from, in words ("comes with Pitwall", "git …", "npm …",
    /// "development folder …", "put there by hand").
    pub source: String,
    /// The card's status word (Off, Needs setup, Connected, …).
    pub state: String,
    /// A newer version already known to be available (no fetch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
    pub demo: bool,
    /// Why it can't be used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// `extension.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtList {
    pub proto: u32,
    pub installed: Vec<ExtListItem>,
    /// Those Pitwall ships that aren't installed.
    pub available: Vec<ExtListItem>,
}

/// What a checklist step is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    #[default]
    Setting,
    Action,
    ServiceInstall,
    ServiceStart,
    ServiceFirstRun,
    Connected,
}

/// How a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StepState {
    Done,
    #[default]
    Todo,
    /// Can't be done yet: `blockedBy` says what first.
    Blocked,
}

/// One step of the checklist Settings shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtStep {
    pub kind: StepKind,
    /// The setting's key or the action's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub label: String,
    pub state: StepState,
    /// A secret setting: only the user enters it, in Pitwall.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    /// What the card says under it, in plain words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A setup action done, but not for these accounts of its tools yet.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_yet_for: Vec<String>,
}

/// One of its settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtSettingInfo {
    pub key: String,
    pub label: String,
    /// `url`, `text`, `number`, `choice`, `bool` or `secret`.
    pub kind: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// A `choice`'s values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

/// One connection (a set of its settings and secrets). Secrets appear only
/// as whether each is set: their values never leave the secure store here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtConnection {
    pub id: String,
    pub name: String,
    pub default: bool,
    /// Its plain settings that are set.
    pub values: BTreeMap<String, String>,
    /// Each secret setting: set or not.
    pub secrets: BTreeMap<String, bool>,
    /// Its status word.
    pub state: String,
}

/// The adapter's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AdapterState {
    #[default]
    Off,
    NeedsSetup,
    Connecting,
    Connected,
    SignedOut,
    Stopped,
    Error,
}

/// Its adapter (the program that connects it), for the connection shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtAdapter {
    pub state: AdapterState,
    /// Its own words on why, or what to do.
    pub reason: String,
}

/// Where its service stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ServiceState {
    #[default]
    Off,
    NotInstalled,
    /// Installed, below the version it needs.
    TooOld,
    /// Its commands wait for the user's confirmation.
    NeedsConfirmation,
    NeedsSetting,
    BadAddress,
    Checking,
    Starting,
    Restarting,
    Running,
    Stopped,
    /// The user pressed Stop.
    StoppedByUser,
    /// Something else listens on its address: the user runs it; Pitwall
    /// never starts or stops that one.
    RunningElsewhere,
}

/// Its first run (a first sign-in).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtFirstRun {
    pub label: String,
    /// Still to do: the service's data is fresh.
    pub pending: bool,
    /// It opens a one-time link in the user's browser.
    pub opens: bool,
    /// The last run here: `running`, `done` or `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    /// How it went, in words (never a link).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Its service: the program Pitwall keeps running while it is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtService {
    pub label: String,
    pub state: ServiceState,
    /// What the card says, in plain words (with the last lines when it
    /// stopped).
    pub detail: String,
    /// Its version, as its version command said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The lowest version it needs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs: Option<String>,
    /// The action that installs or updates it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_action: Option<String>,
    /// Running since (Unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// An update changed its commands (so it was stopped and asks again).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub changed: bool,
    /// Where something else already listens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_run: Option<ExtFirstRun>,
}

/// One permission it asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtPermission {
    pub name: String,
    pub title: String,
    /// Its own reason.
    pub reason: String,
    /// Granted, refused, or (`None`) not answered yet.
    pub granted: Option<bool>,
}

/// One of its actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtActionInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    /// A checklist step (else under More).
    pub setup: bool,
    pub done: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_yet_for: Vec<String>,
}

/// Who does the next step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Who {
    /// The agent runs `command` (it asks the user in Pitwall when it acts).
    Agent,
    /// Only the user can: in Pitwall, or in their browser.
    User,
    /// Nobody: it is under way; look again in a few seconds.
    Wait,
    /// Nothing left: all set.
    #[default]
    Nobody,
}

/// The single next step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtNext {
    pub who: Who,
    /// The checklist step (1-based) it is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<usize>,
    /// The exact command the agent can run (`<value>`: what the user said).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub text: String,
}

/// `extension.status`: what Settings shows on the extension's card, as
/// data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtStatus {
    pub proto: u32,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub installed: bool,
    pub on: bool,
    pub source: String,
    /// The card's status word.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// A newer version already known to be available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
    /// The connection the checklist is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    pub steps: Vec<ExtStep>,
    pub settings: Vec<ExtSettingInfo>,
    pub connections: Vec<ExtConnection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<ExtAdapter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ExtService>,
    pub permissions: Vec<ExtPermission>,
    pub actions: Vec<ExtActionInfo>,
    pub next: ExtNext,
}

/// What the service does when asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceOp {
    /// Confirm its exact commands (as the card's "Confirm and start").
    Confirm,
    Start,
    Stop,
    Restart,
    /// Its install action, then started again (a version too old).
    Update,
}

impl ServiceOp {
    pub fn parse(s: &str) -> Option<ServiceOp> {
        Some(match s {
            "confirm" => ServiceOp::Confirm,
            "start" => ServiceOp::Start,
            "stop" => ServiceOp::Stop,
            "restart" => ServiceOp::Restart,
            "update" => ServiceOp::Update,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        match self {
            ServiceOp::Confirm => "confirm",
            ServiceOp::Start => "start",
            ServiceOp::Stop => "stop",
            ServiceOp::Restart => "restart",
            ServiceOp::Update => "update",
        }
    }
}

/// What an agent asks to do to an extension. Never: grant permissions
/// beyond what turning on shows, delete its data, let it send without
/// asking, or see or set a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum ExtOp {
    /// Install one Pitwall ships (it stays off).
    Install,
    /// Turn it on, granting what the permission screen grants by default.
    On,
    Off,
    /// Update to the version found (`extension.checkUpdates` first).
    Update,
    /// Remove it: its settings and secrets go; its service's data stays.
    Remove,
    /// Run a setup action (`action`: its id; default the first not done).
    Setup {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection: Option<String>,
    },
    Service { what: ServiceOp },
    /// The service's first run (a first sign-in).
    FirstRun,
    /// A plain (non-secret) setting.
    Set {
        key: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection: Option<String>,
    },
    ConnectionAdd { name: String },
    /// Install an extension of tiles from the folder `path` on this
    /// computer: a preview, copied, unsigned and never updated
    /// (docs/spec/tile-sdk/packaging.md §6.3). The id is the manifest's.
    InstallFolder { path: String },
    ConnectionRename { connection: String, name: String },
    /// `default`: the connection that becomes the default when it was the
    /// default (else none).
    ConnectionRemove {
        connection: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    /// Install or update an extension of tiles from the package file
    /// `path` (`.pwtile`, its signature beside it as `.minisig`;
    /// docs/spec/tile-sdk/packaging.md §6.3). `unsigned`: one without a
    /// signature may install (as a preview); `newKey`: one signed by
    /// another key than the installed version (or not signed while it
    /// was) may replace it. The id is the manifest's.
    InstallPackage {
        path: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        unsigned: bool,
        #[serde(default, rename = "newKey", skip_serializing_if = "std::ops::Not::not")]
        new_key: bool,
    },
}

impl ExtOp {
    /// Its command word, for messages ("setup", "service confirm").
    pub fn word(&self) -> String {
        match self {
            ExtOp::Install => "install".into(),
            ExtOp::On => "on".into(),
            ExtOp::Off => "off".into(),
            ExtOp::Update => "update".into(),
            ExtOp::Remove => "remove".into(),
            ExtOp::Setup { .. } => "setup".into(),
            ExtOp::Service { what } => format!("service {}", what.word()),
            ExtOp::FirstRun => "first-run".into(),
            ExtOp::Set { .. } => "set".into(),
            ExtOp::ConnectionAdd { .. } => "connection add".into(),
            ExtOp::InstallFolder { .. } | ExtOp::InstallPackage { .. } => "install".into(),
            ExtOp::ConnectionRename { .. } => "connection rename".into(),
            ExtOp::ConnectionRemove { .. } => "connection remove".into(),
        }
    }
}

/// `extension.act`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtAct {
    pub id: String,
    #[serde(flatten)]
    pub op: ExtOp,
}

/// What an approved `extension.act` (or `extension.checkUpdates`) did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtActed {
    pub proto: u32,
    pub id: String,
    /// The command ("setup", "service confirm", …).
    pub op: String,
    /// The user approved it in Pitwall (false: it needed no approval).
    pub approved: bool,
    /// What happened, in plain words.
    pub text: String,
    /// Still under way when the answer came: look with `status`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub running: bool,
    /// How the extension stands now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ExtStatus>,
}

/// `extension.devReload`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtDevReload {
    /// The extension's folder (where its `extension.json` is), absolute.
    pub path: String,
}

/// What `extension.devReload` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtDevReloaded {
    pub proto: u32,
    pub id: String,
    pub name: String,
    /// Loaded and turned on now (the user approved it); false: it was
    /// loaded before and is reloaded.
    pub loaded: bool,
    /// Its tiles' ids.
    pub tiles: Vec<String>,
    /// The tiles that were running and were replaced.
    pub reloaded: Vec<String>,
    /// What happened, in plain words.
    pub text: String,
    /// The same words, for `pitwall extension dev` (it prints `message`).
    pub message: String,
}

/// One line of the card's Log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtLogLine {
    /// Unix ms.
    pub at: u64,
    /// `pitwall` (Pitwall's note), `to` (to the adapter), `from` (from
    /// it), `stderr`.
    pub dir: String,
    /// `service`, or the connection's name with several.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    pub text: String,
}

/// `extension.log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtLog {
    pub proto: u32,
    pub id: String,
    pub lines: Vec<ExtLogLine>,
}

fn agent(step: Option<usize>, command: String, text: impl Into<String>) -> ExtNext {
    ExtNext { who: Who::Agent, step, command: Some(command), text: text.into() }
}

fn user(step: Option<usize>, command: Option<String>, text: impl Into<String>) -> ExtNext {
    ExtNext { who: Who::User, step, command, text: text.into() }
}

fn wait(step: Option<usize>, text: impl Into<String>) -> ExtNext {
    ExtNext { who: Who::Wait, step, command: None, text: text.into() }
}

/// The single next step of `s`: the first step that isn't done, as a
/// command the agent can run or what only the user can do.
pub fn next_of(s: &ExtStatus) -> ExtNext {
    let id = &s.id;
    if !s.installed {
        return agent(None, format!("pitwall extension install {id}"), "Install it: it stays off and nothing runs until it is turned on.");
    }
    if let Some(p) = &s.problem {
        return user(None, None, format!("It can't be used: {p}. Its folder has to be fixed (or it removed) by the user."));
    }
    if !s.on {
        return agent(None, format!("pitwall extension on {id}"), "Turn it on: the user approves in Pitwall, seeing what it asks for and runs.");
    }
    let open = format!("pitwall open --settings extensions --extension {id}");
    let conn = s.connection.as_ref().filter(|_| s.connections.len() > 1).map(|c| format!(" --connection {}", quote(c))).unwrap_or_default();
    for (i, step) in s.steps.iter().enumerate() {
        if step.state == StepState::Done {
            continue;
        }
        let n = Some(i + 1);
        let key = step.key.clone().unwrap_or_default();
        let svc = s.service.as_ref();
        return match step.kind {
            StepKind::Setting if step.secret => user(
                n,
                Some(format!("pitwall extension secret {id} {key}{conn}")),
                format!("Only the user enters {} in Pitwall: run the command to show them the field, then check `status` until it is set. Never ask for it in the chat.", step.label),
            ),
            StepKind::Setting => {
                let hint = s.settings.iter().find(|x| x.key == key).and_then(|x| x.hint.clone()).map(|h| format!(" ({h})")).unwrap_or_default();
                agent(n, format!("pitwall extension set {id} {key} <value>{conn}"), format!("Ask the user for {}{hint}, then set it.", step.label))
            }
            StepKind::Action => {
                let more = if step.not_yet_for.is_empty() { String::new() } else { format!(" It is done, but not yet for: {}.", step.not_yet_for.join("; ")) };
                match &step.blocked_by {
                    Some(why) => user(n, None, format!("“{}” can't run yet: {why}.{more}", step.label)),
                    None => agent(n, format!("pitwall extension setup {id} --action {key}{conn}"), format!("Run “{}”: the user approves its exact commands in Pitwall.{more}", step.label)),
                }
            }
            StepKind::ServiceInstall => match svc {
                Some(v) if v.state == ServiceState::TooOld && v.install_action.is_some() => agent(n, format!("pitwall extension service {id} update"), format!("{} Update it (its install action), then it starts again.", v.detail)),
                Some(v) if v.install_action.is_some() => agent(n, format!("pitwall extension setup {id} --action {}", v.install_action.clone().unwrap_or_default()), format!("Install {}: the user approves its exact commands in Pitwall.", v.label)),
                Some(v) => user(n, Some(format!("pitwall extension service {id} restart")), format!("{} The user installs it themselves; then run the command so Pitwall looks again.", v.detail)),
                None => user(n, None, step.label.clone()),
            },
            StepKind::ServiceStart => match svc.map(|v| v.state).unwrap_or_default() {
                ServiceState::NeedsConfirmation => {
                    let why = if svc.is_some_and(|v| v.changed) { " An update changed what it runs, so it asks again." } else { "" };
                    agent(n, format!("pitwall extension service {id} confirm"), format!("Confirm and start it: the user approves its exact commands, variables and data folder in Pitwall.{why}"))
                }
                ServiceState::Stopped | ServiceState::StoppedByUser => agent(n, format!("pitwall extension service {id} start"), format!("It is stopped: {} Start it (or read `pitwall extension log {id}` first).", svc.map(|v| v.detail.as_str()).unwrap_or_default())),
                ServiceState::BadAddress | ServiceState::NeedsSetting => user(n, None, format!("It can't start: {}. Fix that setting (`pitwall extension set …`), then it starts.", svc.map(|v| v.detail.as_str()).unwrap_or_default())),
                ServiceState::NotInstalled | ServiceState::TooOld => user(n, None, step.blocked_by.clone().unwrap_or_else(|| "Install it first.".into())),
                _ => wait(n, "It is starting: run `status` again in a few seconds."),
            },
            StepKind::ServiceFirstRun => match svc.map(|v| v.state).unwrap_or_default() {
                ServiceState::Running if svc.and_then(|v| v.first_run.as_ref()).is_some_and(|f| f.last.as_deref() == Some("running")) => wait(n, "Its first run is under way."),
                ServiceState::Running => {
                    let opens = svc.and_then(|v| v.first_run.as_ref()).is_some_and(|f| f.opens);
                    let what = if opens { " Pitwall opens a one-time sign-in link in the user's browser (you never see it); the user signs in there." } else { "" };
                    agent(n, format!("pitwall extension first-run {id}"), format!("Run “{}”: the user approves it in Pitwall.{what}", step.label))
                }
                ServiceState::RunningElsewhere => user(n, None, "The server runs elsewhere (the user runs it): its first sign-in is done where it runs, not by Pitwall."),
                _ => wait(n, step.blocked_by.clone().unwrap_or_else(|| "Once it is running.".into())),
            },
            StepKind::Connected => {
                let a = s.adapter.clone().unwrap_or_default();
                match a.state {
                    AdapterState::SignedOut => {
                        let secret = s.settings.iter().find(|x| x.kind == "secret").map(|x| format!(" If its key is wrong, `pitwall extension secret {id} {}` lets the user replace it in Pitwall.", x.key)).unwrap_or_default();
                        let first = if s.service.as_ref().and_then(|v| v.first_run.as_ref()).is_some() { format!(" If it needs a sign-in again, `pitwall extension first-run {id}`.") } else { String::new() };
                        user(n, Some(open), format!("It says it is signed out: {}.{secret}{first}", a.reason.trim_end_matches('.')))
                    }
                    AdapterState::Error | AdapterState::Stopped => user(n, Some(open), format!("It isn't connected: {} Read `pitwall extension log {id}`; once fixed, the user presses Try again on its card.", a.reason)),
                    _ => wait(n, "It is connecting: run `status` again in a few seconds."),
                }
            }
        };
    }
    ExtNext { who: Who::Nobody, step: None, command: None, text: "All set.".into() }
}

/// `text` quoted for a shell when it has to be.
fn quote(text: &str) -> String {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@".contains(c)) {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

fn word(s: StepState) -> &'static str {
    match s {
        StepState::Done => "done",
        StepState::Todo => "todo",
        StepState::Blocked => "blocked",
    }
}

/// `status` in words (`--human`).
pub fn status_text(s: &ExtStatus) -> String {
    let mut out = format!("{} ({}){}: {}\n", s.name, s.id, s.version.as_ref().map(|v| format!(" {v}")).unwrap_or_default(), s.state);
    out.push_str(&format!("  from {}\n", s.source));
    if let Some(u) = &s.update {
        out.push_str(&format!("  version {u} is available\n"));
    }
    if let Some(p) = &s.problem {
        out.push_str(&format!("  can't be used: {p}\n"));
    }
    for (i, st) in s.steps.iter().enumerate() {
        out.push_str(&format!("  {}. [{}] {}\n", i + 1, word(st.state), st.label));
        if let Some(b) = &st.blocked_by {
            out.push_str(&format!("       waits for: {b}\n"));
        }
        if let Some(d) = &st.detail {
            out.push_str(&format!("       {d}\n"));
        }
    }
    for c in &s.connections {
        let secrets: Vec<String> = c.secrets.iter().map(|(k, set)| format!("{k} {}", if *set { "set" } else { "not set" })).collect();
        out.push_str(&format!("  connection {}{}: {}{}\n", c.name, if c.default { " (default)" } else { "" }, c.state, if secrets.is_empty() { String::new() } else { format!("; secrets: {}", secrets.join(", ")) }));
    }
    if let Some(v) = &s.service {
        out.push_str(&format!("  service {}: {:?}{}\n", v.label, v.state, if v.detail.is_empty() { String::new() } else { format!(" — {}", v.detail) }));
    }
    if let Some(a) = &s.adapter {
        out.push_str(&format!("  adapter: {:?}{}\n", a.state, if a.reason.is_empty() { String::new() } else { format!(" — {}", a.reason) }));
    }
    let who = match s.next.who {
        Who::Agent => "you",
        Who::User => "the user",
        Who::Wait => "wait",
        Who::Nobody => "nothing",
    };
    out.push_str(&format!("Next ({who}): {}\n", s.next.text));
    if let Some(c) = &s.next.command {
        out.push_str(&format!("  {c}\n"));
    }
    out
}

/// `list` in words.
pub fn list_text(l: &ExtList) -> String {
    let mut out = String::new();
    let line = |e: &ExtListItem| {
        let mut s = format!("{:<20} {:<24} {:<10} {}", e.id, e.name, e.version.clone().unwrap_or_default(), e.state);
        if let Some(u) = &e.update {
            s.push_str(&format!(" (update: {u})"));
        }
        format!("{s}  [{}]\n", e.source)
    };
    out.push_str("Installed:\n");
    if l.installed.is_empty() {
        out.push_str("  none\n");
    }
    for e in &l.installed {
        out.push_str(&format!("  {}", line(e)));
    }
    if !l.available.is_empty() {
        out.push_str("Available:\n");
        for e in &l.available {
            out.push_str(&format!("  {}", line(e)));
        }
    }
    out
}

/// `log` in words.
pub fn log_text(l: &ExtLog) -> String {
    if l.lines.is_empty() {
        return "Nothing in its log yet.\n".into();
    }
    l.lines.iter().map(|x| format!("{} {}{} {}\n", x.at, x.dir, if x.source.is_empty() { String::new() } else { format!(" [{}]", x.source) }, x.text)).collect()
}

/// An action's answer in words.
pub fn acted_text(a: &ExtActed) -> String {
    let mut out = a.text.trim_end().to_string();
    out.push('\n');
    if let Some(s) = &a.status {
        out.push_str(&format!("Next: {}\n", s.next.text));
        if let Some(c) = &s.next.command {
            out.push_str(&format!("  {c}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests;
