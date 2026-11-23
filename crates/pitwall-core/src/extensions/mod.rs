//! Extensions (docs/spec/extensions.md): a folder in Pitwall's data folder
//! (`<data>/extensions/<id>/`) with a declarative manifest,
//! `extension.json`. No code from it is loaded into Pitwall: the app draws
//! everything itself, from the manifest and from what the extension's
//! adapter (a separate process) sends over a small protocol
//! ([`protocol`]). This module finds the folders, reads and checks their
//! manifests, and expands the manifest's name templates ([`expand`]);
//! which extensions are on is the app's (`ui.json` `extensions`).
//!
//! Version 1 of the manifest knows `id`, `name`, `version`, `demo`, `docs`,
//! `icon`, `description`, `settings`, `adapter`, `agentEnv`, `matchAgents`,
//! `actions` and `source`. Keys it doesn't
//! know are ignored, so a later version can add its own without breaking
//! this one; a manifest asking for a newer format (`v`) is refused with a
//! word on why.
//!
//! Format 2 (docs/spec/extension-api.md, "The manifest, format 2") adds
//! `pitwall` (the lowest Pitwall it needs: an older one refuses it, saying
//! so), `permissions` (each with the extension's own reason:
//! [`permissions`]) and `contributes` (what it adds; a kind this Pitwall
//! doesn't draw is refused, saying it needs a newer one). A Pitwall that
//! reads only format 1 refuses `"v": 2`, which is what makes these keys
//! safe: none is ignored silently. Settings know `text`, `choice`, `bool`
//! and `number` besides `url` and `secret`, in either format.
//!
//! What format 2 contributes ([`Contributes`]): the inbox, view tiles drawn
//! from Pitwall's own components ([`view`]), command palette commands,
//! badges and short notes on sidebar rows, notifications (with the
//! `notify` permission), sections in the right panel for the focused agent
//! (with `agents.read`) and web views: an https (or this computer's own
//! http) page in a web tile, with no way to reach Pitwall from the page.

pub mod activity;
pub mod check;
pub mod install;
pub mod log;
pub mod npm;
pub mod permissions;
pub mod plan;
pub mod process;
pub mod protocol;
pub mod source;
pub mod starter;
pub mod view;

use std::path::Path;

use serde_json::Value;

/// The manifest's file name in an extension's folder.
pub const MANIFEST: &str = "extension.json";

/// The newest manifest format this Pitwall reads (it reads 1 too).
pub const FORMAT: u64 = 2;

/// This Pitwall's version, which a format 2 manifest's `pitwall` is
/// compared with (every crate of Pitwall has the same version).
pub const PITWALL: &str = env!("CARGO_PKG_VERSION");

/// What a format 2 manifest may contribute in this Pitwall.
pub const CONTRIBUTES: [&str; 7] = ["inbox", "views", "commands", "badges", "notifications", "panels", "webViews"];

/// Contributions a later Pitwall draws (a manifest naming one needs it).
/// Floating-control cards come from the inbox's blocking asks alone.
const LATER_CONTRIBUTIONS: [&str; 1] = ["cards"];

/// The most views and commands a manifest contributes.
pub const MAX_VIEWS: usize = 10;
pub const MAX_COMMANDS: usize = 20;
/// The most right-panel sections and web views a manifest contributes.
pub const MAX_PANELS: usize = 3;
pub const MAX_WEB_VIEWS: usize = 5;
/// The longest view or command title.
pub const MAX_TITLE: usize = 40;

/// The longest id, icon name and name.
pub const MAX_ID: usize = 40;
pub const MAX_NAME: usize = 40;
/// The longest description.
pub const MAX_DESCRIPTION: usize = 200;

/// A manifest bigger than this isn't read (a manifest is a few lines).
const MAX_BYTES: u64 = 64 * 1024;

/// An extension's manifest, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Its id: lower-case letters, digits and `-`, starting with a letter;
    /// also its folder's name.
    pub id: String,
    /// What the sidebar and Settings call it.
    pub name: String,
    /// Its version (dot-separated numbers, maybe `-` and a pre-release):
    /// how Pitwall tells a newer bundled copy ([`install`]).
    pub version: Option<String>,
    /// Made to show what an extension looks like, not for use: listed last
    /// and marked so among the extensions Pitwall ships.
    pub demo: bool,
    /// Where its documentation is (an https address), linked in Settings.
    pub docs: Option<String>,
    /// One of Pitwall's icon names (an app without it draws its own
    /// extension icon).
    pub icon: Option<String>,
    /// One line for Settings.
    pub description: Option<String>,
    /// What the user fills in for it in Settings.
    pub settings: Vec<SettingDef>,
    /// The process Pitwall starts while it is on.
    pub adapter: Option<Adapter>,
    /// Variables set on the agents Pitwall starts while it is on.
    pub agent_env: Vec<EnvVar>,
    /// How the names a thread is from match Pitwall's agents.
    pub match_agents: Option<MatchAgents>,
    /// Setup actions the user may run from Settings (never by themselves).
    pub actions: Vec<Action>,
    /// Where its updates come from ([`source`]).
    pub source: Option<Source>,
    /// Its format (`v`): 1 or 2.
    pub format: u64,
    /// Format 2: the lowest Pitwall it needs (`>=X.Y.Z`, the version kept).
    pub pitwall: Option<String>,
    /// Format 2: the permissions it asks for, each with its reason.
    pub permissions: Vec<PermissionAsk>,
    /// Format 2: what it adds to Pitwall.
    pub contributes: Contributes,
}

/// A permission a manifest asks for, and why, in the extension's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionAsk {
    pub name: String,
    pub reason: String,
}

/// What a format 2 manifest adds to Pitwall.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contributes {
    /// An inbox, from its adapter's threads.
    pub inbox: bool,
    /// View tiles, drawn from what its adapter sends ([`view`]).
    pub views: Vec<ViewDef>,
    /// Command palette commands, sent to its adapter when picked.
    pub commands: Vec<CommandDef>,
    /// Badges and short notes on agents' and projects' sidebar rows.
    pub badges: bool,
    /// Notifications (with the `notify` permission).
    pub notifications: bool,
    /// Sections in the right panel for the focused agent (with
    /// `agents.read`), drawn from what its adapter sends for that agent.
    pub panels: Vec<PanelDef>,
    /// Web pages in web tiles, with no bridge to Pitwall.
    pub web_views: Vec<WebViewDef>,
}

/// A right-panel section a manifest declares: its id (what the adapter's
/// `show`, `view` and `action` lines name, with the agent's id), its title
/// (the section's header) and maybe a view of its own that "More…" opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelDef {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    /// One of its views, opened by "More…" under the section.
    pub more: Option<String>,
}

/// A web view a manifest declares: a page in a web tile. Its address is
/// `url` (https, or http on this computer; it may start with
/// `{setting:KEY}`, a `url` setting), or what the adapter sends in a
/// `webView` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebViewDef {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub url: Option<String>,
}

/// A view a manifest declares: its id (what the adapter's `view` lines
/// name), its title (the tile's bar, "Open <title>") and its icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDef {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
}

/// Where a command applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandContext {
    /// Anywhere.
    #[default]
    None,
    /// Only while an agent is focused; its message names the agent (with
    /// `agents.read`).
    Agent,
}

/// A command palette command a manifest declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDef {
    pub id: String,
    pub title: String,
    pub context: CommandContext,
}

impl Contributes {
    /// What it adds, one line each, in plain words (`pitwall extension
    /// check`, an install's and an update's "what changes").
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.inbox {
            out.push("an inbox, from its adapter's threads".to_string());
        }
        out.extend(self.views.iter().map(|v| format!("a view tile “{}” ({})", v.title, v.id)));
        out.extend(self.commands.iter().map(|c| match c.context {
            CommandContext::None => format!("a command “{}” ({}) in the command palette", c.title, c.id),
            CommandContext::Agent => format!("a command “{}” ({}) in the command palette while an agent is focused", c.title, c.id),
        }));
        if self.badges {
            out.push("badges and short notes on agents' and projects' rows in the sidebar".to_string());
        }
        if self.notifications {
            out.push("notifications (a few a minute at most)".to_string());
        }
        out.extend(self.panels.iter().map(|p| format!("a section “{}” ({}) in the right panel, for the focused agent", p.title, p.id)));
        out.extend(self.web_views.iter().map(|w| match &w.url {
            Some(u) => format!("a web page “{}” ({}) in a tile: {u}, with a browser store of its own", w.title, w.id),
            None => format!("a web page “{}” ({}) in a tile, at an address its adapter gives, with a browser store of its own", w.title, w.id),
        }));
        out
    }

    /// Right-panel section `id`, when it declares one.
    pub fn panel(&self, id: &str) -> Option<&PanelDef> {
        self.panels.iter().find(|p| p.id == id)
    }

    /// Web view `id`, when it declares one.
    pub fn web_view(&self, id: &str) -> Option<&WebViewDef> {
        self.web_views.iter().find(|w| w.id == id)
    }

    /// View `id`, when it declares one.
    pub fn view(&self, id: &str) -> Option<&ViewDef> {
        self.views.iter().find(|v| v.id == id)
    }

    /// Command `id`, when it declares one.
    pub fn command(&self, id: &str) -> Option<&CommandDef> {
        self.commands.iter().find(|c| c.id == id)
    }
}

/// Where an extension's updates come from: the tags of a git repository,
/// or the versions of an npm package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Git {
        /// The repository: an `https://` address without credentials.
        git: String,
        /// The folder in it that holds `extension.json`, `/`-separated
        /// (empty: its root).
        dir: String,
        /// The tags that are its releases: a pattern with `{version}`,
        /// such as `my-ext-v{version}`.
        tags: String,
    },
    Npm {
        /// The package's name (`@scope/name` or `name`), checked by
        /// [`check_npm_package`].
        package: String,
        /// The registry: an `https://` address without credentials or a
        /// trailing `/` ([`NPM_REGISTRY`] unless the manifest names one).
        registry: String,
    },
}

/// The placeholder of a git [`Source`]'s tag pattern.
pub const VERSION_PLACEHOLDER: &str = "{version}";

/// The npm registry used when a source names none.
pub const NPM_REGISTRY: &str = "https://registry.npmjs.org";

impl Manifest {
    /// A manifest with only these (tests and fixtures).
    pub fn named(id: &str, name: &str) -> Manifest {
        Manifest {
            id: id.into(),
            name: name.into(),
            version: None,
            demo: false,
            docs: None,
            icon: None,
            description: None,
            settings: vec![],
            adapter: None,
            agent_env: vec![],
            match_agents: None,
            actions: vec![],
            source: None,
            format: 1,
            pitwall: None,
            permissions: vec![],
            contributes: Contributes::default(),
        }
    }

    /// It has an inbox: in format 1 every extension with an adapter does;
    /// in format 2 one that contributes it.
    pub fn has_inbox(&self) -> bool {
        self.adapter.is_some() && (self.format < 2 || self.contributes.inbox)
    }
}

/// A setup action: a button in Settings that, once the user confirms the
/// exact commands, runs its steps in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    /// The button's text.
    pub label: String,
    /// One line under it.
    pub description: String,
    /// Part of setting the extension up: a step of its checklist in
    /// Settings, ticked once it has run without a failure.
    pub setup: bool,
    pub steps: Vec<Step>,
}

/// A digest of action `a`'s steps (programs, arguments, kinds and
/// whether each is optional): a recorded run counts only while its steps
/// are the ones that ran (FNV-1a, enough to notice a change).
pub fn steps_digest(a: &Action) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for s in &a.steps {
        eat(s.for_kind.as_deref().unwrap_or("").as_bytes());
        eat(&[0, s.optional as u8]);
        for arg in &s.run {
            eat(&(arg.len() as u64).to_le_bytes());
            eat(arg.as_bytes());
        }
        eat(&[0xff]);
    }
    format!("{h:016x}")
}

/// One program run: no shell, its arguments as they are (after
/// `{setting:KEY}` is put in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The program (looked up on the login shell's PATH), then its
    /// arguments.
    pub run: Vec<String>,
    /// The agent kind it is for (`claude`, `codex`, …): skipped, saying
    /// so, when its program isn't installed, and after an earlier step for
    /// the same kind failed.
    pub for_kind: Option<String>,
    /// Its failure doesn't stop the steps after it (removing what may not
    /// be there).
    pub optional: bool,
}

/// Step `run`'s arguments with the extension's plain settings put in;
/// `Err(label)` naming a setting that isn't set.
pub fn expand_args(run: &[String], m: &Manifest, values: &std::collections::BTreeMap<String, String>) -> Result<Vec<String>, String> {
    run.iter()
        .map(|a| {
            let mut out = a.clone();
            for key in setting_keys(a) {
                let v = values.get(key).filter(|v| !v.trim().is_empty()).ok_or_else(|| m.settings.iter().find(|s| s.key == key).map_or(key.to_string(), |s| s.label.clone()))?;
                out = out.replace(&format!("{SETTING_PLACEHOLDER}{key}}}"), v);
            }
            Ok(out)
        })
        .collect()
}

/// A value the user sets for an extension in Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingDef {
    /// Its key in the adapter's `hello` (`[A-Za-z][A-Za-z0-9_]{0,31}`).
    pub key: String,
    pub kind: SettingKind,
    pub label: String,
    /// One line under it.
    pub hint: Option<String>,
    /// Shown in an empty field.
    pub placeholder: Option<String>,
    /// The adapter isn't started until it is set.
    pub required: bool,
    /// Its value until the user sets one (never for a secret), as it is
    /// kept: `true`/`false` for a switch, a number's digits, a choice's
    /// value.
    pub default: Option<String>,
    /// A `choice`'s options, in order.
    pub options: Vec<Choice>,
    /// A `number`'s bounds.
    pub min: Option<serde_json::Number>,
    pub max: Option<serde_json::Number>,
}

/// One option of a `choice` setting: what is kept and sent, and what
/// Settings shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

impl SettingDef {
    /// A setting with only these (tests and fixtures).
    pub fn new(key: &str, kind: SettingKind, label: &str) -> SettingDef {
        SettingDef { key: key.into(), kind, label: label.into(), hint: None, placeholder: None, required: false, default: None, options: vec![], min: None, max: None }
    }

    /// The value it has with `stored` kept for it (else its default), as
    /// `hello` sends it: a string, `true`/`false` for a switch, a number
    /// for a number. `None` when it has none (or is a secret: those are
    /// sent apart).
    pub fn json(&self, stored: Option<&str>) -> Option<Value> {
        let v = stored.filter(|v| !v.trim().is_empty()).or(self.default.as_deref())?;
        Some(match self.kind {
            SettingKind::Secret => return None,
            SettingKind::Bool => Value::Bool(v == "true"),
            SettingKind::Number => serde_json::from_str::<serde_json::Number>(v.trim()).map(Value::Number).ok()?,
            SettingKind::Url | SettingKind::Text | SettingKind::Choice => Value::String(v.to_string()),
        })
    }

    /// Whether it counts as set with `stored` kept (a default counts; a
    /// switch always is); a secret's is told by `secret_set`.
    pub fn is_set(&self, stored: Option<&str>, secret_set: bool) -> bool {
        match self.kind {
            SettingKind::Secret => secret_set,
            SettingKind::Bool => true,
            _ => stored.is_some_and(|v| !v.trim().is_empty()) || self.default.is_some(),
        }
    }

    /// What `text` typed (or picked) for it is kept as: nothing when empty,
    /// else the value; why not, in plain words. Not for `url` (the app
    /// checks web addresses) or secrets.
    pub fn check_value(&self, text: &str) -> Result<Option<String>, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        match self.kind {
            SettingKind::Text => {
                if t.chars().count() > MAX_TEXT_SETTING || t.chars().any(char::is_control) {
                    return Err(format!("Keep it to one line of at most {MAX_TEXT_SETTING} characters."));
                }
                Ok(Some(t.to_string()))
            }
            SettingKind::Choice => self.options.iter().find(|o| o.value == t).map(|o| Some(o.value.clone())).ok_or_else(|| "Pick one of its options.".to_string()),
            SettingKind::Bool => match t {
                "true" | "false" => Ok(Some(t.to_string())),
                _ => Err("On or off.".into()),
            },
            SettingKind::Number => {
                let n: f64 = t.parse().ok().filter(|n: &f64| n.is_finite()).ok_or("That isn't a number.")?;
                let (lo, hi) = (self.min.as_ref().and_then(serde_json::Number::as_f64), self.max.as_ref().and_then(serde_json::Number::as_f64));
                match (lo, hi) {
                    (Some(lo), Some(hi)) if n < lo || n > hi => Err(format!("Between {lo} and {hi}.")),
                    (Some(lo), None) if n < lo => Err(format!("At least {lo}.")),
                    (None, Some(hi)) if n > hi => Err(format!("At most {hi}.")),
                    _ => Ok(Some(match t.parse::<i64>() {
                        Ok(i) => i.to_string(),
                        Err(_) => n.to_string(),
                    })),
                }
            }
            SettingKind::Url | SettingKind::Secret => Ok(Some(t.to_string())),
        }
    }
}

/// The longest `text` setting.
pub const MAX_TEXT_SETTING: usize = 500;

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// An http(s) address, kept in `ui.json`.
    Url,
    /// A key or token, kept in the system's secure store (`ui.json` only
    /// says that it is set).
    Secret,
    /// One line of text (at most [`MAX_TEXT_SETTING`] characters).
    Text,
    /// One of its `options`.
    Choice,
    /// A switch: on or off.
    Bool,
    /// A number, within its `min` and `max`.
    Number,
}

impl SettingKind {
    /// Its name in a manifest.
    pub fn name(self) -> &'static str {
        match self {
            SettingKind::Url => "url",
            SettingKind::Secret => "secret",
            SettingKind::Text => "text",
            SettingKind::Choice => "choice",
            SettingKind::Bool => "bool",
            SettingKind::Number => "number",
        }
    }

    /// What it is, in a few words (`check`, the update review).
    pub fn words(self) -> &'static str {
        match self {
            SettingKind::Url => "a web address",
            SettingKind::Secret => "a secret, kept in the system's secure store",
            SettingKind::Text => "a line of text",
            SettingKind::Choice => "a choice",
            SettingKind::Bool => "a switch",
            SettingKind::Number => "a number",
        }
    }
}

/// The adapter: a program and its arguments, run in the extension's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// The program (looked up on the login shell's PATH, or a path in the
    /// extension's folder when it has a `/`), then its arguments.
    pub command: Vec<String>,
}

/// A variable for the agents Pitwall starts: its name, and its value as a
/// template ([`expand`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Templates ([`expand`]) whose values for a Pitwall agent are the names a
/// thread from that agent carries (`from.agent`, `from.session`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchAgents {
    pub agent: String,
    pub session: String,
}

/// What a template's placeholders stand for, for one agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts<'a> {
    /// The agent's project folder (its last part is used).
    pub project: &'a str,
    /// The agent's name.
    pub agent: &'a str,
    /// Pitwall's id for the agent (kept across restarts).
    pub agent_id: &'a str,
    /// The extension's plain settings that are set, by key (`{setting:KEY}`
    /// in an agent variable).
    pub settings: Option<&'a std::collections::BTreeMap<String, String>>,
}

/// The placeholders a template may use.
pub const PLACEHOLDERS: [&str; 3] = ["{project}", "{agent}", "{agent_id}"];

/// `raw` as a name: lower case; runs of anything but `a-z`, `0-9`, `_` and
/// `-` become one `-`; it starts with a letter or digit, doesn't end with
/// `-` and has at most 63 characters (`agent` when nothing is left).
pub fn name_form(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' { c } else { '-' };
        if (c == '-' || c == '_') && out.is_empty() {
            continue;
        }
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out.truncate(63);
    while out.ends_with('-') || out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("agent");
    }
    out
}

/// The prefix of a placeholder for one of the extension's plain settings,
/// `{setting:KEY}` (agent variables only; never a secret).
pub const SETTING_PLACEHOLDER: &str = "{setting:";

/// The setting keys template `t` names with `{setting:KEY}`.
fn setting_keys(t: &str) -> Vec<&str> {
    t.match_indices(SETTING_PLACEHOLDER).filter_map(|(i, _)| t[i + SETTING_PLACEHOLDER.len()..].split_once('}').map(|(k, _)| k)).collect()
}

/// Template `t` for an agent: each name placeholder replaced by its fact
/// in [`name_form`] (the project by its folder's last part), each
/// `{setting:KEY}` by that setting as it is; other text as it is. `None`
/// when it names a setting that isn't set (the variable isn't set then).
pub fn expand(t: &str, f: &Facts) -> Option<String> {
    let project = f.project.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
    let mut out = t.replace("{project}", &name_form(project)).replace("{agent_id}", &name_form(f.agent_id)).replace("{agent}", &name_form(f.agent));
    for key in setting_keys(t) {
        let v = f.settings.and_then(|s| s.get(key)).filter(|v| !v.trim().is_empty() && !v.chars().any(char::is_control))?;
        out = out.replace(&format!("{SETTING_PLACEHOLDER}{key}}}"), v);
    }
    Some(out)
}

/// The variables extension `m` sets on an agent with facts `f` (one naming
/// a setting that isn't set is left out).
pub fn agent_env(m: &Manifest, f: &Facts) -> Vec<(String, String)> {
    m.agent_env.iter().filter_map(|e| Some((e.name.clone(), expand(&e.value, f)?))).collect()
}

/// Why template `t` (field `what`) can't be used: an unknown placeholder,
/// a brace left over, a control character, too long. `settings`: the
/// manifest's settings when `{setting:KEY}` may be used (agent variables),
/// which must name a plain one, never a secret.
fn check_template(what: &str, t: &str, settings: Option<&[SettingDef]>) -> Result<(), String> {
    if t.is_empty() || t.chars().count() > 200 || t.chars().any(char::is_control) {
        return Err(format!("{what} must be one line of 1–200 characters"));
    }
    let mut rest = t.to_string();
    for p in PLACEHOLDERS {
        rest = rest.replace(p, "");
    }
    if let Some(settings) = settings {
        for key in setting_keys(t) {
            match settings.iter().find(|s| s.key == key) {
                None => return Err(format!("{what} names setting “{key}”, which the manifest doesn't have")),
                Some(s) if s.kind == SettingKind::Secret => return Err(format!("{what} names secret “{key}”: secrets are never given to agents")),
                Some(_) => rest = rest.replace(&format!("{SETTING_PLACEHOLDER}{key}}}"), ""),
            }
        }
    }
    if rest.contains('{') || rest.contains('}') {
        let mut allowed = PLACEHOLDERS.join(", ");
        if settings.is_some() {
            allowed.push_str(", {setting:KEY}");
        }
        return Err(format!("{what} “{t}” may only use {allowed}"));
    }
    Ok(())
}

/// Variables an extension may not set on agents: Pitwall's own and the
/// ones starting a program depends on.
fn reserved_env(name: &str) -> bool {
    const NAMES: [&str; 12] = ["PATH", "HOME", "SHELL", "USER", "LOGNAME", "TERM", "COLORTERM", "PWD", "TMPDIR", "LANG", "IFS", "ENV"];
    ["PITWALL_", "LD_", "DYLD_", "LC_"].iter().any(|p| name.starts_with(p)) || NAMES.contains(&name)
}

/// One folder of the extensions folder: its manifest, or why it can't be
/// used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The folder's name.
    pub folder: String,
    pub manifest: Result<Manifest, String>,
}

impl Found {
    /// The id when it is usable, else the folder's name.
    pub fn key(&self) -> &str {
        self.manifest.as_ref().map_or(self.folder.as_str(), |m| m.id.as_str())
    }
}

/// Why `id` can't be an extension's id (`what`: "id" or "icon").
fn check_word(what: &str, id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err(format!("\"{what}\" is empty"));
    }
    if id.len() > MAX_ID {
        return Err(format!("\"{what}\" is longer than {MAX_ID} characters"));
    }
    let ok = id.starts_with(|c: char| c.is_ascii_lowercase())
        && !id.ends_with('-')
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(format!("\"{what}\" “{id}” must be lower-case letters, digits and -, starting with a letter"));
    }
    Ok(())
}

/// Whether `id` is a usable extension id.
pub fn check_id(id: &str) -> Result<(), String> {
    check_word("id", id)
}

/// One line of text, trimmed, at most `max` characters.
fn line(what: &str, v: &str, max: usize) -> Result<String, String> {
    let t = v.trim();
    if t.is_empty() {
        return Err(format!("\"{what}\" is empty"));
    }
    if t.chars().any(char::is_control) {
        return Err(format!("\"{what}\" must be one line"));
    }
    if t.chars().count() > max {
        return Err(format!("\"{what}\" is longer than {max} characters"));
    }
    Ok(t.to_string())
}

/// An optional string field.
fn opt_str<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("\"{key}\" must be a string")),
    }
}

/// Read and check manifest `text`, for this Pitwall.
pub fn parse(text: &str) -> Result<Manifest, String> {
    parse_for(text, PITWALL)
}

/// [`parse`] for Pitwall version `pitwall` (a format 2 manifest's
/// `pitwall` is compared with it).
pub fn parse_for(text: &str, pitwall: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{MANIFEST} isn't valid JSON: {e}"))?;
    let Value::Object(obj) = v else {
        return Err(format!("{MANIFEST} must be a JSON object"));
    };
    let format = match obj.get("v") {
        None => 1,
        Some(n) => match n.as_u64() {
            Some(n @ 1..=FORMAT) => n,
            Some(n) if n > FORMAT => return Err(format!("it needs a newer Pitwall (manifest format {n}; this one reads 1 and {FORMAT})")),
            _ => return Err(format!("\"v\" must be 1 or {FORMAT}")),
        },
    };
    // Format 2: the Pitwall it needs first, so an older one says that
    // rather than what else it can't read.
    let min_pitwall = if format >= 2 { parse_pitwall(obj.get("pitwall"), pitwall)? } else { None };
    let id = opt_str(&obj, "id")?.ok_or("\"id\" is missing")?;
    check_id(id)?;
    let name = line("name", opt_str(&obj, "name")?.ok_or("\"name\" is missing")?, MAX_NAME)?;
    let icon = match opt_str(&obj, "icon")? {
        Some(i) => {
            check_word("icon", i)?;
            Some(i.to_string())
        }
        None => None,
    };
    let description = opt_str(&obj, "description")?.map(|d| line("description", d, MAX_DESCRIPTION)).transpose()?;
    let version = match opt_str(&obj, "version")? {
        Some(v) => {
            let ok = v.len() <= 32 && v.starts_with(|c: char| c.is_ascii_digit()) && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c));
            if !ok {
                return Err(format!("\"version\" “{v}” must be like 1.2.0 (digits and dots, maybe -beta.1), at most 32"));
            }
            Some(v.to_string())
        }
        None => None,
    };
    let docs = match opt_str(&obj, "docs")? {
        Some(d) if d.starts_with("https://") && d.len() <= 300 && !d.chars().any(|c| c.is_whitespace() || c.is_control()) => Some(d.to_string()),
        Some(_) => return Err("\"docs\" must be an https address of at most 300 characters".into()),
        None => None,
    };
    let demo = match obj.get("demo") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("\"demo\" must be true or false".into()),
    };
    let settings = parse_settings(obj.get("settings"))?;
    let adapter = parse_adapter(obj.get("adapter"))?;
    let agent_env = parse_env(obj.get("agentEnv"), &settings)?;
    let match_agents = match obj.get("matchAgents") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => {
            let get = |k: &str| -> Result<String, String> {
                let t = opt_str(m, k)?.ok_or(format!("\"matchAgents\" needs \"{k}\""))?;
                check_template(&format!("\"matchAgents.{k}\""), t, None)?;
                Ok(t.to_string())
            };
            Some(MatchAgents { agent: get("agent")?, session: get("session")? })
        }
        Some(_) => return Err("\"matchAgents\" must be an object".into()),
    };
    let actions = parse_actions(obj.get("actions"), &settings)?;
    let source = parse_source(obj.get("source"))?;
    // Format 1 ignores these keys, as it always has (a Pitwall reading
    // only format 1 never saw them); `check` says so.
    let (permissions, contributes) = if format >= 2 { (parse_permissions(obj.get("permissions"))?, parse_contributes(obj.get("contributes"))?) } else { (vec![], Contributes::default()) };
    if contributes.inbox && adapter.is_none() {
        return Err("\"contributes.inbox\" needs an \"adapter\": the inbox shows what it sends".into());
    }
    if adapter.is_none() {
        let what = [("views", !contributes.views.is_empty()), ("commands", !contributes.commands.is_empty()), ("badges", contributes.badges), ("notifications", contributes.notifications), ("panels", !contributes.panels.is_empty()), ("webViews", contributes.web_views.iter().any(|w| w.url.is_none()))].into_iter().find(|(_, has)| *has);
        if let Some((what, _)) = what {
            return Err(format!("\"contributes.{what}\" needs an \"adapter\": Pitwall draws what it sends"));
        }
    }
    if contributes.notifications && !permissions.iter().any(|p| p.name == permissions::NOTIFY) {
        return Err("\"contributes.notifications\" needs the \"notify\" permission, with your reason for it".into());
    }
    if !contributes.panels.is_empty() && !permissions.iter().any(|p| p.name == permissions::AGENTS_READ) {
        return Err("\"contributes.panels\" needs the \"agents.read\" permission, with your reason for it: a section is about one agent".into());
    }
    check_web_views(&contributes, &settings)?;
    Ok(Manifest { id: id.to_string(), name, version, demo, docs, icon, description, settings, adapter, agent_env, match_agents, actions, source, format, pitwall: min_pitwall, permissions, contributes })
}

/// `pitwall`: `>=X.Y.Z`; `Err` when this Pitwall (`ours`) is older.
fn parse_pitwall(v: Option<&Value>, ours: &str) -> Result<Option<String>, String> {
    let s = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s.trim(),
        Some(_) => return Err("\"pitwall\" must be a string like >=0.9.0".into()),
    };
    let want = s.strip_prefix(">=").map(str::trim).filter(|w| w.len() <= 32 && w.starts_with(|c: char| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)));
    let Some(want) = want else {
        return Err(format!("\"pitwall\" “{s}” must be like >=0.9.0 (the lowest Pitwall it needs)"));
    };
    if install::compare_versions(ours, want) == std::cmp::Ordering::Less {
        return Err(format!("it needs Pitwall {want} or later (this is {ours})"));
    }
    Ok(Some(want.to_string()))
}

/// The most permissions a manifest asks for.
pub const MAX_PERMISSIONS: usize = 16;

/// `permissions`: name → reason (one line, at most 200 characters). A name
/// this Pitwall doesn't know is kept: shown as not available, never
/// granted.
fn parse_permissions(v: Option<&Value>) -> Result<Vec<PermissionAsk>, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("\"permissions\" must be an object: each permission's name and your reason for it".into()),
    };
    if o.len() > MAX_PERMISSIONS {
        return Err(format!("\"permissions\" has more than {MAX_PERMISSIONS}"));
    }
    let mut out = Vec::new();
    for (name, reason) in o {
        let ok = name.len() <= 40 && name.split('.').all(|w| w.starts_with(|c: char| c.is_ascii_lowercase()) && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        if !ok {
            return Err(format!("permission “{name}” must be lower-case words with dots, like agents.read"));
        }
        let Value::String(reason) = reason else { return Err(format!("permission “{name}” needs your reason, a string")) };
        let reason = line(&format!("permissions.{name}"), reason, 200)?;
        out.push(PermissionAsk { name: name.clone(), reason });
    }
    Ok(out)
}

/// `contributes`: what it adds. A kind this Pitwall doesn't draw is
/// refused, saying so, rather than dropped without a word.
fn parse_contributes(v: Option<&Value>) -> Result<Contributes, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(Contributes::default()),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("\"contributes\" must be an object".into()),
    };
    let mut c = Contributes::default();
    let switch = |k: &str, val: &Value| match val {
        Value::Bool(b) => Ok(*b),
        _ => Err(format!("\"contributes.{k}\" must be true or false")),
    };
    for (k, val) in o {
        match k.as_str() {
            "inbox" => c.inbox = switch(k, val)?,
            "badges" => c.badges = switch(k, val)?,
            "notifications" => c.notifications = switch(k, val)?,
            "views" => c.views = parse_views(val)?,
            "commands" => c.commands = parse_commands(val)?,
            "panels" => c.panels = parse_panels(val)?,
            "webViews" => c.web_views = parse_web_views(val)?,
            later if LATER_CONTRIBUTIONS.contains(&later) => return Err(format!("it contributes “{later}”, which needs a newer Pitwall (this one draws: {})", CONTRIBUTES.join(", "))),
            other => return Err(format!("\"contributes\" has “{other}”, which this Pitwall doesn't know (it draws: {})", CONTRIBUTES.join(", "))),
        }
    }
    // Views, sections and web views are named in the same lines: one id
    // each among them.
    let mut ids: Vec<&str> = Vec::new();
    for id in c.views.iter().map(|v| v.id.as_str()).chain(c.panels.iter().map(|p| p.id.as_str())).chain(c.web_views.iter().map(|w| w.id.as_str())) {
        if ids.contains(&id) {
            return Err(format!("“{id}” names a view, a panel or a web view twice: they share their ids"));
        }
        ids.push(id);
    }
    for p in &c.panels {
        if let Some(more) = &p.more {
            if c.view(more).is_none() {
                return Err(format!("panel “{}” has \"more\": “{more}”, which isn't one of its views", p.id));
            }
        }
    }
    Ok(c)
}

/// An id, a title and maybe an icon of a list item in `contributes.<what>`
/// (its `n`th).
fn id_title_icon<'a>(what: &str, n: usize, o: &'a serde_json::Map<String, Value>) -> Result<(&'a str, String, Option<String>), String> {
    let id = opt_str(o, "id")?.ok_or(format!("\"contributes.{what}\" {n} needs an \"id\""))?;
    check_word(&format!("{what} id"), id)?;
    let title = line(&format!("contributes.{what} {id}: title"), opt_str(o, "title")?.ok_or(format!("“{id}” in \"contributes.{what}\" needs a \"title\""))?, MAX_TITLE)?;
    let icon = match opt_str(o, "icon")? {
        Some(i) => {
            check_word("icon", i)?;
            Some(i.to_string())
        }
        None => None,
    };
    Ok((id, title, icon))
}

/// `contributes.panels`: at most [`MAX_PANELS`] right-panel sections.
fn parse_panels(v: &Value) -> Result<Vec<PanelDef>, String> {
    let Value::Array(list) = v else { return Err("\"contributes.panels\" must be a list".into()) };
    if list.len() > MAX_PANELS {
        return Err(format!("\"contributes.panels\" has more than {MAX_PANELS}"));
    }
    let mut out: Vec<PanelDef> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Value::Object(o) = item else { return Err(format!("\"contributes.panels\" {} must be an object", i + 1)) };
        let (id, title, icon) = id_title_icon("panels", i + 1, o)?;
        let more = match opt_str(o, "more")? {
            Some(m) => {
                check_word("panel more", m)?;
                Some(m.to_string())
            }
            None => None,
        };
        out.push(PanelDef { id: id.to_string(), title, icon, more });
    }
    Ok(out)
}

/// `contributes.webViews`: at most [`MAX_WEB_VIEWS`] pages, each with an
/// address Pitwall may open ([`view::web_view_url`]) or none (its adapter
/// gives one).
fn parse_web_views(v: &Value) -> Result<Vec<WebViewDef>, String> {
    let Value::Array(list) = v else { return Err("\"contributes.webViews\" must be a list".into()) };
    if list.len() > MAX_WEB_VIEWS {
        return Err(format!("\"contributes.webViews\" has more than {MAX_WEB_VIEWS}"));
    }
    let mut out: Vec<WebViewDef> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Value::Object(o) = item else { return Err(format!("\"contributes.webViews\" {} must be an object", i + 1)) };
        let (id, title, icon) = id_title_icon("webViews", i + 1, o)?;
        if o.contains_key("bridge") || o.contains_key("scripts") || o.contains_key("preload") {
            return Err(format!("web view “{id}” asks for a bridge or scripts: a web view is a plain page, with no way to reach Pitwall"));
        }
        let url = match opt_str(o, "url")? {
            None => None,
            Some(u) => {
                let rest = match u.strip_prefix("{setting:") {
                    Some(r) => r.split_once('}').map(|(_, rest)| rest).ok_or(format!("web view “{id}”: its url's {{setting:…}} isn't closed"))?,
                    None => {
                        view::web_view_url(u).map_err(|e| format!("web view “{id}”: {e}"))?;
                        ""
                    }
                };
                if rest.chars().any(|c| c.is_whitespace() || c.is_control()) || rest.contains('{') {
                    return Err(format!("web view “{id}”: its url has spaces or another placeholder"));
                }
                Some(line(&format!("contributes.webViews {id}: url"), u, 2000)?)
            }
        };
        out.push(WebViewDef { id: id.to_string(), title, icon, url });
    }
    Ok(out)
}

/// A web view's `{setting:KEY}` must name one of its `url` settings (never
/// a secret: a secret is never in an address).
fn check_web_views(c: &Contributes, settings: &[SettingDef]) -> Result<(), String> {
    for w in &c.web_views {
        let Some(key) = w.url.as_deref().and_then(|u| u.strip_prefix("{setting:")).and_then(|r| r.split_once('}')).map(|(k, _)| k) else { continue };
        match settings.iter().find(|s| s.key == key) {
            Some(s) if s.kind == SettingKind::Url => {}
            Some(_) => return Err(format!("web view “{}”: {{setting:{key}}} must be a url setting (a secret or another kind is never put in an address)", w.id)),
            None => return Err(format!("web view “{}”: no setting “{key}”", w.id)),
        }
    }
    Ok(())
}

/// `contributes.views`: at most [`MAX_VIEWS`], each with a unique id (like
/// an extension's), a title and maybe an icon.
fn parse_views(v: &Value) -> Result<Vec<ViewDef>, String> {
    let Value::Array(list) = v else { return Err("\"contributes.views\" must be a list".into()) };
    if list.len() > MAX_VIEWS {
        return Err(format!("\"contributes.views\" has more than {MAX_VIEWS}"));
    }
    let mut out: Vec<ViewDef> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Value::Object(o) = item else { return Err(format!("\"contributes.views\" {} must be an object", i + 1)) };
        let id = opt_str(o, "id")?.ok_or(format!("\"contributes.views\" {} needs an \"id\"", i + 1))?;
        check_word("view id", id)?;
        if out.iter().any(|d| d.id == id) {
            return Err(format!("view “{id}” is there twice"));
        }
        let title = line(&format!("contributes.views {id}: title"), opt_str(o, "title")?.ok_or(format!("view “{id}” needs a \"title\""))?, MAX_TITLE)?;
        let icon = match opt_str(o, "icon")? {
            Some(i) => {
                check_word("icon", i)?;
                Some(i.to_string())
            }
            None => None,
        };
        out.push(ViewDef { id: id.to_string(), title, icon });
    }
    Ok(out)
}

/// `contributes.commands`: at most [`MAX_COMMANDS`], each with a unique id,
/// a title and a context (`none` or `agent`). No keybindings: a command is
/// picked from the palette.
fn parse_commands(v: &Value) -> Result<Vec<CommandDef>, String> {
    let Value::Array(list) = v else { return Err("\"contributes.commands\" must be a list".into()) };
    if list.len() > MAX_COMMANDS {
        return Err(format!("\"contributes.commands\" has more than {MAX_COMMANDS}"));
    }
    let mut out: Vec<CommandDef> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Value::Object(o) = item else { return Err(format!("\"contributes.commands\" {} must be an object", i + 1)) };
        let id = opt_str(o, "id")?.ok_or(format!("\"contributes.commands\" {} needs an \"id\"", i + 1))?;
        check_word("command id", id)?;
        if out.iter().any(|d| d.id == id) {
            return Err(format!("command “{id}” is there twice"));
        }
        let title = line(&format!("contributes.commands {id}: title"), opt_str(o, "title")?.ok_or(format!("command “{id}” needs a \"title\""))?, MAX_TITLE)?;
        let context = match opt_str(o, "context")? {
            None | Some("none") => CommandContext::None,
            Some("agent") => CommandContext::Agent,
            Some(other) => return Err(format!("command “{id}” has context “{other}”: it must be none or agent")),
        };
        if o.contains_key("keys") || o.contains_key("keybinding") {
            return Err(format!("command “{id}” names keys: extensions' commands have no shortcuts (they are picked from the command palette)"));
        }
        out.push(CommandDef { id: id.to_string(), title, context });
    }
    Ok(out)
}

/// Why `url` can't be a source's repository: only `https://`, without a
/// user or password, spaces, a query or a fragment, at most 300
/// characters.
pub fn check_git_url(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("it must be an https:// address".into());
    };
    if url.len() > 300 || url.chars().any(|c| c.is_whitespace() || c.is_control()) || url.contains(['?', '#', '\\']) {
        return Err("it must be an https address of at most 300 characters, without spaces".into());
    }
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() || host.contains('@') {
        return Err("it can't carry a user or password: your git credential helper gives those".into());
    }
    if rest.trim_end_matches('/').len() <= host.len() {
        return Err("it must name a repository, not only a host".into());
    }
    Ok(())
}

/// Why `dir` can't be a folder in a repository: relative, `/`-separated,
/// no `.` or `..` parts; letters, digits and `._-`.
pub fn check_source_dir(dir: &str) -> Result<(), String> {
    if dir.is_empty() {
        return Ok(());
    }
    let ok = dir.len() <= 200
        && dir.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)));
    if !ok {
        return Err(format!("the folder “{dir}” must be a folder in the repository, like extensions/my-ext"));
    }
    Ok(())
}

/// Why `name` can't be an npm package's name. npm's rules for a new
/// package, kept strict: at most 214 characters; lower-case ASCII letters,
/// digits and `-._`, starting with a letter or digit; optionally under a
/// scope (`@scope/name`, the scope by the same rules). So no paths, no
/// addresses and no version ranges (`name@1.x`, `../x`, `https://…`), and
/// not `node_modules` or `favicon.ico`.
pub fn check_npm_package(name: &str) -> Result<(), String> {
    let part = |p: &str| !p.is_empty() && p.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-._".contains(c));
    let ok = match name.strip_prefix('@') {
        Some(scoped) => scoped.split_once('/').is_some_and(|(scope, n)| part(scope) && part(n)),
        None => part(name),
    };
    if !ok || name.len() > 214 {
        return Err(format!("“{name}” isn't an npm package name: lower-case letters, digits and -._, like my-ext or @me/my-ext (no path, address or version)"));
    }
    if ["node_modules", "favicon.ico"].contains(&name) {
        return Err(format!("“{name}” can't be an npm package's name"));
    }
    Ok(())
}

/// Why `url` can't be an npm registry: as [`check_git_url`] (`https://`,
/// no user or password, spaces, query or fragment), a host and optionally
/// a path.
pub fn check_npm_registry(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("it must be an https:// address".into());
    };
    if url.len() > 300 || url.chars().any(|c| c.is_whitespace() || c.is_control()) || url.contains(['?', '#', '\\']) {
        return Err("it must be an https address of at most 300 characters, without spaces".into());
    }
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() || host.contains('@') {
        return Err("it can't carry a user or password: Pitwall sends no credentials to a registry".into());
    }
    Ok(())
}

fn parse_source(v: Option<&Value>) -> Result<Option<Source>, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("\"source\" must be an object".into()),
    };
    if o.contains_key("npm") {
        if o.contains_key("git") {
            return Err("\"source\" names both \"git\" and \"npm\": it must be one of them".into());
        }
        let package = opt_str(o, "npm")?.unwrap_or_default();
        check_npm_package(package).map_err(|e| format!("\"source.npm\": {e}"))?;
        let registry = opt_str(o, "registry")?.unwrap_or(NPM_REGISTRY).trim_end_matches('/');
        check_npm_registry(registry).map_err(|e| format!("\"source.registry\": {e}"))?;
        for key in ["dir", "tags"] {
            if o.contains_key(key) {
                return Err(format!("\"source.{key}\" is for a git source: an npm package's extension.json is at its root and its versions are its releases"));
            }
        }
        return Ok(Some(Source::Npm { package: package.to_string(), registry: registry.to_string() }));
    }
    if o.contains_key("registry") {
        return Err("\"source.registry\" is for an npm source (\"npm\": the package's name)".into());
    }
    let git = opt_str(o, "git")?.ok_or("\"source\" needs \"git\" (the repository's https address) or \"npm\" (the package's name)")?;
    check_git_url(git).map_err(|e| format!("\"source.git\": {e}"))?;
    let dir = opt_str(o, "dir")?.unwrap_or("").trim_matches('/');
    check_source_dir(dir).map_err(|e| format!("\"source.dir\": {e}"))?;
    let tags = opt_str(o, "tags")?.ok_or("\"source\" needs \"tags\", like my-ext-v{version}")?;
    let rest = tags.replacen(VERSION_PLACEHOLDER, "", 1);
    let ok = tags.matches(VERSION_PLACEHOLDER).count() == 1 && tags.len() <= 100 && rest.chars().all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c));
    if !ok {
        return Err(format!("\"source.tags\" “{tags}” must hold {VERSION_PLACEHOLDER} once, with letters, digits and ._/- around it"));
    }
    Ok(Some(Source::Git { git: git.to_string(), dir: dir.to_string(), tags: tags.to_string() }))
}

/// An array field (absent: empty) of at most `max` items.
fn array<'a>(v: Option<&'a Value>, what: &str, max: usize) -> Result<&'a [Value], String> {
    match v {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) if a.len() <= max => Ok(a),
        Some(Value::Array(_)) => Err(format!("\"{what}\" has more than {max} items")),
        Some(_) => Err(format!("\"{what}\" must be an array")),
    }
}

fn parse_settings(v: Option<&Value>) -> Result<Vec<SettingDef>, String> {
    let mut out: Vec<SettingDef> = Vec::new();
    for item in array(v, "settings", 10)? {
        let Value::Object(o) = item else { return Err("each of \"settings\" must be an object".into()) };
        let key = opt_str(o, "key")?.ok_or("a setting needs \"key\"")?;
        let key_ok = key.len() <= 32 && key.starts_with(|c: char| c.is_ascii_alphabetic()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !key_ok {
            return Err(format!("setting key “{key}” must be letters, digits and _, starting with a letter, at most 32"));
        }
        if out.iter().any(|s| s.key == key) {
            return Err(format!("setting “{key}” is there twice"));
        }
        let kind = match opt_str(o, "type")? {
            Some("url") => SettingKind::Url,
            Some("secret") => SettingKind::Secret,
            Some("text") => SettingKind::Text,
            Some("choice") => SettingKind::Choice,
            Some("bool") => SettingKind::Bool,
            Some("number") => SettingKind::Number,
            Some(t) => return Err(format!("setting “{key}” has type “{t}”: url, secret, text, choice, bool or number")),
            None => return Err(format!("setting “{key}” needs \"type\"")),
        };
        let label = line("label", opt_str(o, "label")?.ok_or(format!("setting “{key}” needs \"label\""))?, 60)?;
        let hint = opt_str(o, "hint")?.map(|h| line("hint", h, 300)).transpose()?;
        let placeholder = opt_str(o, "placeholder")?.map(|h| line("placeholder", h, 100)).transpose()?;
        let required = match o.get("required") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(format!("setting “{key}”: \"required\" must be true or false")),
        };
        let mut def = SettingDef { key: key.to_string(), kind, label, hint, placeholder, required, default: None, options: vec![], min: None, max: None };
        if kind == SettingKind::Choice {
            for opt in array(o.get("options"), &format!("settings.{key}.options"), 20)? {
                let (value, label) = match opt {
                    Value::String(s) => (s.clone(), s.clone()),
                    Value::Object(c) => {
                        let v = opt_str(c, "value")?.ok_or(format!("an option of setting “{key}” needs \"value\""))?.to_string();
                        let l = opt_str(c, "label")?.map(str::to_string).unwrap_or_else(|| v.clone());
                        (v, l)
                    }
                    _ => return Err(format!("each option of setting “{key}” must be a string or {{\"value\", \"label\"}}")),
                };
                let value = line(&format!("an option of setting “{key}”"), &value, 100)?;
                let label = line(&format!("an option of setting “{key}”"), &label, 60)?;
                if def.options.iter().any(|c| c.value == value) {
                    return Err(format!("setting “{key}” has option “{value}” twice"));
                }
                def.options.push(Choice { value, label });
            }
            if def.options.len() < 2 {
                return Err(format!("setting “{key}” is a choice: give it 2–20 \"options\""));
            }
        }
        if kind == SettingKind::Number {
            let bound = |k: &str| -> Result<Option<serde_json::Number>, String> {
                match o.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::Number(n)) => Ok(Some(n.clone())),
                    Some(_) => Err(format!("setting “{key}”: \"{k}\" must be a number")),
                }
            };
            def.min = bound("min")?;
            def.max = bound("max")?;
            if let (Some(lo), Some(hi)) = (def.min.as_ref().and_then(serde_json::Number::as_f64), def.max.as_ref().and_then(serde_json::Number::as_f64)) {
                if lo > hi {
                    return Err(format!("setting “{key}”: \"min\" is above \"max\""));
                }
            }
        }
        def.default = match (o.get("default"), kind) {
            (None | Some(Value::Null), _) => None,
            (Some(_), SettingKind::Secret) => return Err(format!("setting “{key}” is a secret: it can't have a \"default\"")),
            (Some(Value::Bool(b)), SettingKind::Bool) => Some(b.to_string()),
            (Some(Value::Number(n)), SettingKind::Number) => Some(def.check_value(&n.to_string()).map_err(|e| format!("setting “{key}”: its \"default\": {e}"))?.unwrap_or_default()),
            (Some(Value::String(s)), SettingKind::Url | SettingKind::Text | SettingKind::Choice) => {
                if kind == SettingKind::Url && !(s.starts_with("https://") || s.starts_with("http://")) {
                    return Err(format!("setting “{key}”: its \"default\" must be an http(s) address"));
                }
                Some(def.check_value(s).map_err(|e| format!("setting “{key}”: its \"default\": {e}"))?.ok_or(format!("setting “{key}”: its \"default\" is empty"))?)
            }
            (Some(_), k) => return Err(format!("setting “{key}”: its \"default\" must be a {}", match k {
                SettingKind::Bool => "true or false",
                SettingKind::Number => "number",
                _ => "string",
            })),
        };
        out.push(def);
    }
    Ok(out)
}

fn parse_adapter(v: Option<&Value>) -> Result<Option<Adapter>, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("\"adapter\" must be an object".into()),
    };
    let mut command = Vec::new();
    for p in array(o.get("command"), "adapter.command", 16)? {
        match p {
            Value::String(s) if !s.is_empty() && s.len() <= 500 && !s.contains('\0') => command.push(s.clone()),
            _ => return Err("\"adapter.command\" must be strings of 1–500 characters".into()),
        }
    }
    if command.is_empty() {
        return Err("\"adapter.command\" is empty".into());
    }
    Ok(Some(Adapter { command }))
}

fn parse_env(v: Option<&Value>, settings: &[SettingDef]) -> Result<Vec<EnvVar>, String> {
    let mut out: Vec<EnvVar> = Vec::new();
    for item in array(v, "agentEnv", 10)? {
        let Value::Object(o) = item else { return Err("each of \"agentEnv\" must be an object".into()) };
        let name = opt_str(o, "name")?.ok_or("an agent variable needs \"name\"")?;
        let ok = name.len() <= 64
            && name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
            && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !ok {
            return Err(format!("agent variable “{name}” must be upper-case letters, digits and _"));
        }
        if reserved_env(name) {
            return Err(format!("agent variable “{name}” is one an extension may not set"));
        }
        if out.iter().any(|e| e.name == name) {
            return Err(format!("agent variable “{name}” is there twice"));
        }
        let value = opt_str(o, "value")?.ok_or(format!("agent variable “{name}” needs \"value\""))?;
        check_template(&format!("the value of {name}"), value, Some(settings))?;
        out.push(EnvVar { name: name.to_string(), value: value.to_string() });
    }
    Ok(out)
}

fn parse_actions(v: Option<&Value>, settings: &[SettingDef]) -> Result<Vec<Action>, String> {
    let mut out: Vec<Action> = Vec::new();
    for item in array(v, "actions", 5)? {
        let Value::Object(o) = item else { return Err("each of \"actions\" must be an object".into()) };
        let id = opt_str(o, "id")?.ok_or("an action needs \"id\"")?;
        check_word("action id", id)?;
        if out.iter().any(|a| a.id == id) {
            return Err(format!("action “{id}” is there twice"));
        }
        let label = line("label", opt_str(o, "label")?.ok_or(format!("action “{id}” needs \"label\""))?, 40)?;
        let description = line("description", opt_str(o, "description")?.ok_or(format!("action “{id}” needs \"description\""))?, 200)?;
        let setup = match o.get("setup") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(format!("action “{id}”: \"setup\" must be true or false")),
        };
        let mut steps = Vec::new();
        let list = array(o.get("steps"), &format!("actions.{id}.steps"), 12)?;
        if list.is_empty() {
            return Err(format!("action “{id}” has no steps"));
        }
        for st in list {
            let Value::Object(so) = st else { return Err(format!("each step of action “{id}” must be an object")) };
            let run = match so.get("run") {
                Some(Value::Array(a)) if !a.is_empty() && a.len() <= 32 => a,
                Some(Value::String(_)) => return Err(format!("a step of action “{id}” must give \"run\" as the program and its arguments, a list, not one shell line")),
                _ => return Err(format!("a step of action “{id}” needs \"run\": the program and its arguments, 1–32 strings")),
            };
            let mut argv = Vec::new();
            for (i, a) in run.iter().enumerate() {
                let Value::String(a) = a else { return Err(format!("a step of action “{id}”: \"run\" must be strings")) };
                if (i == 0 && a.is_empty()) || a.chars().count() > 500 || a.chars().any(char::is_control) {
                    return Err(format!("a step of action “{id}”: each of \"run\" must be one line of at most 500 characters"));
                }
                if i == 0 && (a.contains('{') || a.contains('/') || a.contains('\\')) {
                    return Err(format!("a step of action “{id}”: the program must be a name found on PATH, not a path or a placeholder"));
                }
                check_args_template(&format!("an argument of action “{id}”"), a, settings)?;
                argv.push(a.clone());
            }
            let for_kind = opt_str(so, "for")?.map(|k| check_word("for", k).map(|_| k.to_string())).transpose()?;
            let optional = match so.get("optional") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(format!("a step of action “{id}”: \"optional\" must be true or false")),
            };
            steps.push(Step { run: argv, for_kind, optional });
        }
        out.push(Action { id: id.to_string(), label, description, setup, steps });
    }
    Ok(out)
}

/// An action's argument: only `{setting:KEY}` placeholders, of plain
/// settings.
fn check_args_template(what: &str, a: &str, settings: &[SettingDef]) -> Result<(), String> {
    let mut rest = a.to_string();
    for key in setting_keys(a) {
        match settings.iter().find(|s| s.key == key) {
            None => return Err(format!("{what} names setting “{key}”, which the manifest doesn't have")),
            Some(s) if s.kind == SettingKind::Secret => return Err(format!("{what} names secret “{key}”: secrets are never put in commands")),
            Some(_) => rest = rest.replace(&format!("{SETTING_PLACEHOLDER}{key}}}"), ""),
        }
    }
    if rest.contains('{') || rest.contains('}') {
        return Err(format!("{what} “{a}” may only use {{setting:KEY}}"));
    }
    Ok(())
}

/// Read and check the manifest in folder `dir`, whatever the folder is
/// called (a development folder is used where it is: `read_dev`).
pub fn read_in_place(dir: &Path) -> Result<Manifest, String> {
    let file = dir.join(MANIFEST);
    let meta = std::fs::metadata(&file).map_err(|_| format!("there is no {MANIFEST} in it"))?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{MANIFEST} is bigger than {} KiB", MAX_BYTES / 1024));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{MANIFEST} can't be read: {e}"))?;
    parse(&text)
}

/// Read and check the manifest in folder `dir` (named `folder`): its id
/// must be the folder's name.
pub fn read(dir: &Path, folder: &str) -> Result<Manifest, String> {
    let m = read_in_place(dir)?;
    if m.id != folder {
        return Err(format!("its folder must be named after its id (“{}”), not “{folder}”", m.id));
    }
    Ok(m)
}

/// The extensions in `extensions_dir` (`Paths::extensions_dir`), by folder
/// name: one entry per folder (hidden ones and files are skipped), usable
/// or not. None when the folder isn't there.
pub fn discover(extensions_dir: &Path) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return Vec::new();
    };
    let mut found: Vec<Found> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()) && e.path().is_dir())
        .filter_map(|e| {
            let folder = e.file_name().to_str()?.to_string();
            if folder.starts_with('.') {
                return None;
            }
            let manifest = read(&e.path(), &folder);
            Some(Found { folder, manifest })
        })
        .collect();
    found.sort_by(|a, b| a.folder.cmp(&b.folder));
    found
}

/// A development folder loaded in place (docs/spec/extensions.md, "Dev
/// mode") as extension `id`, the id it was loaded as: its manifest, or why
/// it can't be used (its id must still be `id`).
pub fn read_dev(dir: &Path, id: &str) -> Found {
    let manifest = if dir.is_dir() {
        read_in_place(dir).and_then(|m| if m.id == id { Ok(m) } else { Err(format!("its id is now “{}”, not “{id}”: remove it from the list and load the folder again", m.id)) })
    } else {
        Err(format!("its folder {} isn't there any more", dir.display()))
    };
    Found { folder: id.to_string(), manifest }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_is_read_and_checked() {
        let m = parse(r#"{"v": 1, "id": "inbox", "name": " Inbox ", "icon": "inbox", "description": "Questions from agents", "panels": [{"later": true}]}"#).unwrap();
        assert_eq!(m, Manifest { icon: Some("inbox".into()), description: Some("Questions from agents".into()), ..Manifest::named("inbox", "Inbox") });
        // `v` is optional; icon and description too; unknown keys are ignored.
        let m = parse(r#"{"id": "a-2", "name": "A", "badge": 3}"#).unwrap();
        assert_eq!((m.icon, m.description), (None, None));
        assert!(parse(r#"{"id": "a", "name": "A", "icon": null}"#).is_ok());
        let m = parse(r#"{"id": "a", "name": "A", "version": "0.4.0-beta.1", "demo": true}"#).unwrap();
        assert_eq!((m.version.as_deref(), m.demo), (Some("0.4.0-beta.1"), true));
        assert!(parse(r#"{"id": "a", "name": "A", "version": "latest"}"#).unwrap_err().contains("like 1.2.0"));
        assert!(parse(r#"{"id": "a", "name": "A", "demo": "yes"}"#).is_err());
        assert_eq!(parse(r#"{"id": "a", "name": "A", "docs": "https://example.com/a"}"#).unwrap().docs.as_deref(), Some("https://example.com/a"));
        assert!(parse(r#"{"id": "a", "name": "A", "docs": "http://example.com"}"#).unwrap_err().contains("https"));
    }

    #[test]
    fn settings_adapter_and_agent_variables_are_read_and_checked() {
        let m = parse(
            r#"{"id": "inbox", "name": "Inbox",
                "settings": [{"key": "server", "type": "url", "label": "Server", "placeholder": "https://inbox.example.com", "required": true},
                             {"key": "apiKey", "type": "secret", "label": "API key", "hint": "From the server's Settings"}],
                "adapter": {"command": ["node", "adapter/main.mjs"]},
                "agentEnv": [{"name": "INBOX_AGENT", "value": "{project}"}, {"name": "INBOX_SESSION", "value": "{agent_id}"}],
                "matchAgents": {"agent": "{project}", "session": "{agent_id}"}}"#,
        )
        .unwrap();
        assert_eq!(m.settings.len(), 2);
        assert_eq!((m.settings[0].kind, m.settings[0].required), (SettingKind::Url, true));
        assert_eq!((m.settings[1].kind, m.settings[1].required), (SettingKind::Secret, false));
        assert_eq!(m.adapter.unwrap().command, ["node", "adapter/main.mjs"]);
        assert_eq!(m.agent_env[1], EnvVar { name: "INBOX_SESSION".into(), value: "{agent_id}".into() });
        assert_eq!(m.match_agents.unwrap().session, "{agent_id}");

        let err = |extra: &str| parse(&format!(r#"{{"id": "x", "name": "X", {extra}}}"#)).unwrap_err();
        assert!(err(r#""settings": [{"key": "a", "type": "color", "label": "A"}]"#).contains("url, secret, text, choice, bool or number"));
        assert!(err(r#""settings": [{"key": "1a", "type": "url", "label": "A"}]"#).contains("letters, digits"));
        assert!(err(r#""settings": [{"key": "a", "type": "url", "label": "A"}, {"key": "a", "type": "url", "label": "B"}]"#).contains("twice"));
        assert!(err(r#""settings": [{"key": "a", "type": "url"}]"#).contains("needs \"label\""));
        assert!(err(r#""settings": {}"#).contains("must be an array"));
        assert!(err(r#""adapter": {"command": []}"#).contains("empty"));
        assert!(err(r#""adapter": {"command": "node x"}"#).contains("must be an array"));
        assert!(err(r#""adapter": {"command": [""]}"#).contains("1–500"));
        assert!(err(r#""agentEnv": [{"name": "PATH", "value": "x"}]"#).contains("may not set"));
        assert!(err(r#""agentEnv": [{"name": "PITWALL_X", "value": "x"}]"#).contains("may not set"));
        assert!(err(r#""agentEnv": [{"name": "DYLD_INSERT_LIBRARIES", "value": "x"}]"#).contains("may not set"));
        assert!(err(r#""agentEnv": [{"name": "lower", "value": "x"}]"#).contains("upper-case"));
        assert!(err(r#""agentEnv": [{"name": "A", "value": "{home}"}]"#).contains("may only use"));
        assert!(err(r#""agentEnv": [{"name": "A", "value": "a\nb"}]"#).contains("one line"));
        assert!(err(r#""matchAgents": {"agent": "{project}"}"#).contains("needs \"session\""));
        // A plain setting may be put in a variable; a secret never.
        let both = r#""settings": [{"key": "server", "type": "url", "label": "S"}, {"key": "key", "type": "secret", "label": "K"}]"#;
        let ok = parse(&format!(r#"{{"id": "x", "name": "X", {both}, "agentEnv": [{{"name": "X_URL", "value": "{{setting:server}}"}}]}}"#)).unwrap();
        assert_eq!(ok.agent_env[0].value, "{setting:server}");
        assert!(err(&format!(r#"{both}, "agentEnv": [{{"name": "X_KEY", "value": "{{setting:key}}"}}]"#)).contains("secrets are never given to agents"));
        assert!(err(&format!(r#"{both}, "agentEnv": [{{"name": "X_Y", "value": "{{setting:nope}}"}}]"#)).contains("doesn't have"));
        assert!(err(&format!(r#"{both}, "matchAgents": {{"agent": "{{setting:server}}", "session": "s"}}"#)).contains("may only use"));
    }

    /// Setup actions: a program and its arguments per step (never a shell
    /// line), plain settings only, within bounds.
    #[test]
    fn setup_actions_are_read_and_checked() {
        let both = r#""settings": [{"key": "server", "type": "url", "label": "Server"}, {"key": "key", "type": "secret", "label": "K"}]"#;
        let with = |steps: &str| parse(&format!(r#"{{"id": "x", "name": "X", {both}, "actions": [{{"id": "setup", "label": "Set up", "description": "Does it.", "steps": [{steps}]}}]}}"#));
        let m = with(r#"{"for": "claude", "run": ["claude", "plugin", "add", "--url={setting:server}"], "optional": true}, {"run": ["tool", ""]}"#).unwrap();
        let a = &m.actions[0];
        assert_eq!((a.label.as_str(), a.steps.len()), ("Set up", 2));
        assert_eq!(a.steps[0], Step { run: vec!["claude".into(), "plugin".into(), "add".into(), "--url={setting:server}".into()], for_kind: Some("claude".into()), optional: true });
        let values = std::collections::BTreeMap::from([("server".to_string(), "https://s.example".to_string())]);
        assert_eq!(expand_args(&a.steps[0].run, &m, &values).unwrap()[3], "--url=https://s.example");
        assert_eq!(expand_args(&a.steps[0].run, &m, &Default::default()).unwrap_err(), "Server");

        let refused = |steps: &str, why: &str| {
            let e = with(steps).unwrap_err();
            assert!(e.contains(why), "{steps}: {e}");
        };
        refused(r#"{"run": "claude plugin add x"}"#, "not one shell line");
        refused(r#"{"run": []}"#, "1–32 strings");
        refused(r#"{"run": ["claude", 3]}"#, "must be strings");
        refused(r#"{"run": ["/bin/rm", "x"]}"#, "not a path");
        refused(r#"{"run": ["{setting:server}"]}"#, "not a path or a placeholder");
        refused(r#"{"run": ["curl", "{setting:key}"]}"#, "secrets are never put in commands");
        refused(r#"{"run": ["curl", "{setting:nope}"]}"#, "doesn't have");
        refused(r#"{"run": ["curl", "{home}"]}"#, "may only use");
        refused(r#"{"run": ["a\nb"]}"#, "one line");
        refused(r#"{"run": ["x"], "optional": "yes"}"#, "true or false");
        let many = vec![r#"{"run": ["x"]}"#; 13].join(",");
        refused(&many, "more than 12");
        assert!(parse(r#"{"id": "x", "name": "X", "actions": [{"id": "a", "label": "A", "description": "d", "steps": []}]}"#).unwrap_err().contains("no steps"));
        assert!(parse(r#"{"id": "x", "name": "X", "actions": [{"id": "a", "label": "A", "steps": [{"run": ["x"]}]}]}"#).unwrap_err().contains("needs \"description\""));
    }

    /// A source: an https repository, a folder in it and a tag pattern
    /// with `{version}`; nothing else.
    #[test]
    fn a_source_is_read_and_checked() {
        let with = |src: &str| parse(&format!(r#"{{"id": "x", "name": "X", "source": {src}}}"#));
        let s = with(r#"{"git": "https://example.com/me/repo", "dir": "/extensions/x/", "tags": "x-v{version}"}"#).unwrap().source.unwrap();
        assert_eq!(s, Source::Git { git: "https://example.com/me/repo".into(), dir: "extensions/x".into(), tags: "x-v{version}".into() });
        assert!(matches!(with(r#"{"git": "https://example.com/me/repo.git", "tags": "{version}"}"#).unwrap().source.unwrap(), Source::Git { dir, .. } if dir.is_empty()));
        assert_eq!(parse(r#"{"id": "x", "name": "X"}"#).unwrap().source, None);
        let refused = |src: &str, why: &str| {
            let e = with(src).unwrap_err();
            assert!(e.contains(why), "{src}: {e}");
        };
        refused(r#"{"git": "http://example.com/a", "tags": "{version}"}"#, "https://");
        refused(r#"{"git": "git@example.com:me/a", "tags": "{version}"}"#, "https://");
        refused(r#"{"git": "file:///tmp/a", "tags": "{version}"}"#, "https://");
        refused(r#"{"git": "https://me:token@example.com/a", "tags": "{version}"}"#, "user or password");
        refused(r#"{"git": "https://example.com/", "tags": "{version}"}"#, "not only a host");
        refused(r#"{"git": "https://example.com/a b", "tags": "{version}"}"#, "without spaces");
        refused(r#"{"git": "https://example.com/a", "dir": "../up", "tags": "{version}"}"#, "a folder in the repository");
        refused(r#"{"git": "https://example.com/a", "tags": "v1"}"#, "{version} once");
        refused(r#"{"git": "https://example.com/a", "tags": "{version}-{version}"}"#, "{version} once");
        refused(r#"{"git": "https://example.com/a", "tags": "a b{version}"}"#, "{version} once");
        refused(r#"{"git": "https://example.com/a"}"#, "needs \"tags\"");
        refused(r#""https://example.com/a""#, "must be an object");
        refused(r#"{"git": "https://example.com/a", "tags": "{version}", "registry": "https://r.example.com"}"#, "is for an npm source");
        refused(r#"{"dir": "x"}"#, "needs \"git\"");
    }

    /// An npm source: a package's name by npm's rules (strictly), and an
    /// https registry (npm's own when none is named); never both git and
    /// npm.
    #[test]
    fn an_npm_source_is_read_and_checked() {
        let with = |src: &str| parse(&format!(r#"{{"id": "x", "name": "X", "source": {src}}}"#));
        assert_eq!(with(r#"{"npm": "@me/my-ext"}"#).unwrap().source.unwrap(), Source::Npm { package: "@me/my-ext".into(), registry: NPM_REGISTRY.into() });
        assert_eq!(with(r#"{"npm": "my.ext_2", "registry": "https://npm.example.com/r/"}"#).unwrap().source.unwrap(), Source::Npm { package: "my.ext_2".into(), registry: "https://npm.example.com/r".into() });
        let refused = |src: &str, why: &str| {
            let e = with(src).unwrap_err();
            assert!(e.contains(why), "{src}: {e}");
        };
        for bad in ["My-Ext", "@me/", "@/x", "@me", "me/x", "../x", "./x", "x/../y", "@me/x/y", ".x", "_x", "-x", "x@1.0.0", "x@^1", "^1.0.0", "https://example.com/x.tgz", "git+https://example.com/x", "file:x", "x y", "", "x~y", "x!", "@ME/x", "node_modules", "favicon.ico", "/x", "x\\y"] {
            refused(&format!(r#"{{"npm": {}}}"#, serde_json::to_string(bad).unwrap()), if bad == "node_modules" || bad == "favicon.ico" { "can't be an npm package" } else { "isn't an npm package name" });
        }
        refused(&format!(r#"{{"npm": "{}"}}"#, "a".repeat(215)), "isn't an npm package name");
        assert!(with(&format!(r#"{{"npm": "{}"}}"#, "a".repeat(214))).is_ok());
        refused(r#"{"npm": "x", "registry": "http://registry.example.com"}"#, "https://");
        refused(r#"{"npm": "x", "registry": "https://me:token@registry.example.com"}"#, "user or password");
        refused(r#"{"npm": "x", "registry": "https://registry.example.com/?a=b"}"#, "without spaces");
        refused(r#"{"npm": "x", "registry": "file:///tmp/r"}"#, "https://");
        refused(r#"{"npm": "x", "git": "https://example.com/a", "tags": "{version}"}"#, "both \"git\" and \"npm\"");
        refused(r#"{"npm": "x", "dir": "ext"}"#, "is for a git source");
        refused(r#"{"npm": "x", "tags": "v{version}"}"#, "is for a git source");
        refused(r#"{"npm": 3}"#, "\"npm\"");
    }

    /// A setup action's digest changes with its steps, and only with them.
    #[test]
    fn an_actions_digest_follows_its_steps() {
        let m = |pin: &str, label: &str| parse(&format!(r#"{{"id": "x", "name": "X", "actions": [{{"id": "setup", "label": "{label}", "description": "d", "steps": [{{"run": ["tool", "add", "x#{pin}"]}}]}}]}}"#)).unwrap();
        let d = |m: &Manifest| steps_digest(&m.actions[0]);
        assert_eq!(d(&m("v1", "A")), d(&m("v1", "B")), "the label isn't a step");
        assert_ne!(d(&m("v1", "A")), d(&m("v2", "A")), "a moved pin is");
        let mut opt = m("v1", "A");
        opt.actions[0].steps[0].optional = true;
        assert_ne!(d(&opt), d(&m("v1", "A")));
        // Arguments don't run together: ["ab", "c"] isn't ["a", "bc"].
        let mut a = m("v1", "A");
        a.actions[0].steps[0].run = vec!["ab".into(), "c".into()];
        let mut b = a.clone();
        b.actions[0].steps[0].run = vec!["a".into(), "bc".into()];
        assert_ne!(d(&a), d(&b));
    }

    #[test]
    fn templates_expand_to_names() {
        let f = Facts { project: "/Users/x/code/My App/", agent: "Fix login: 2nd try", agent_id: "3F2A-01", settings: None };
        let ex = |t: &str, f: &Facts| expand(t, f).unwrap();
        assert_eq!(ex("{project}", &f), "my-app");
        assert_eq!(ex("{agent}", &f), "fix-login-2nd-try");
        assert_eq!(ex("{agent_id}", &f), "3f2a-01");
        assert_eq!(ex("pw-{project}-{agent_id}", &f), "pw-my-app-3f2a-01");
        assert_eq!(ex("{project}", &Facts { project: r"C:\code\api", ..f }), "api");
        // A plain setting goes in as it is; one not set leaves the variable out.
        let set = std::collections::BTreeMap::from([("server".to_string(), "https://Inbox.example.com/x".to_string())]);
        assert_eq!(expand("{setting:server}", &Facts { settings: Some(&set), ..f }).as_deref(), Some("https://Inbox.example.com/x"));
        assert_eq!(expand("{setting:server}", &f), None);
        assert_eq!(name_form("--Ünïcode__"), "n-code");
        assert_eq!(name_form("!!!"), "agent");
        assert_eq!(name_form(&"a".repeat(80)).len(), 63);
        // Every name it makes passes the usual name rule.
        let rule = regex::Regex::new("^[a-z0-9][a-z0-9_-]{0,62}$").unwrap();
        for raw in ["My App", "_x", "x-", "a b c", "日本", "A--B", &"z".repeat(70)] {
            assert!(rule.is_match(&name_form(raw)), "{raw:?} → {}", name_form(raw));
        }
    }

    #[test]
    fn bad_ids_are_refused() {
        for id in ["", "Inbox", "2fa", "-a", "a-", "a_b", "a.b", "a b", "ä", "../x", &"a".repeat(41)] {
            let text = serde_json::json!({"id": id, "name": "X"}).to_string();
            assert!(parse(&text).unwrap_err().contains("\"id\""), "{id:?}");
        }
        assert!(check_id(&"a".repeat(40)).is_ok());
        assert!(parse(r#"{"id": 3, "name": "X"}"#).unwrap_err().contains("must be a string"));
    }

    #[test]
    fn missing_and_bad_fields_are_refused() {
        let err = |t: &str| parse(t).unwrap_err();
        assert!(err("not json").contains("isn't valid JSON"));
        assert!(err("[]").contains("JSON object"));
        assert!(err(r#"{"name": "X"}"#).contains("\"id\" is missing"));
        assert!(err(r#"{"id": "x"}"#).contains("\"name\" is missing"));
        assert!(err(r#"{"id": "x", "name": "  "}"#).contains("\"name\" is empty"));
        assert!(err(r#"{"id": "x", "name": "a\nb"}"#).contains("one line"));
        assert!(err(&serde_json::json!({"id": "x", "name": "n".repeat(41)}).to_string()).contains("longer than 40"));
        assert!(err(r#"{"id": "x", "name": "X", "icon": "Big Icon"}"#).contains("\"icon\""));
        assert!(err(r#"{"id": "x", "name": "X", "icon": 1}"#).contains("\"icon\" must be a string"));
        assert!(err(r#"{"id": "x", "name": "X", "description": ""}"#).contains("\"description\" is empty"));
        assert!(err(r#"{"v": 3, "id": "x", "name": "X"}"#).contains("needs a newer Pitwall"));
        assert!(err(r#"{"v": 0, "id": "x", "name": "X"}"#).contains("\"v\" must be 1 or 2"));
        assert!(err(r#"{"v": "1", "id": "x", "name": "X"}"#).contains("\"v\" must be 1 or 2"));
    }

    /// Right-panel sections and web views: bounded, ids shared with views,
    /// a section needs `agents.read` and an adapter, a web view's address
    /// is https or this computer's, a setting in it is a url setting.
    #[test]
    fn panels_and_web_views_are_read_and_checked() {
        let m = |extra: &str| format!(r#"{{"v": 2, "id": "ci", "name": "CI", "adapter": {{"command": ["node", "a.mjs"]}}, "permissions": {{"agents.read": "To match runs"}}, "settings": [{{"key": "server", "type": "url", "label": "Server"}}, {{"key": "token", "type": "secret", "label": "Token"}}], "contributes": {{{extra}}}}}"#);
        let ok = parse(&m(r#""views": [{"id": "runs", "title": "Runs"}], "panels": [{"id": "ci", "title": "CI", "more": "runs"}], "webViews": [{"id": "dash", "title": "Dashboard", "url": "https://ci.example.com/dash"}, {"id": "local", "title": "Local", "url": "http://localhost:4000/"}, {"id": "own", "title": "Own", "url": "{setting:server}/board"}, {"id": "given", "title": "Given"}]"#)).unwrap();
        assert_eq!(ok.contributes.panel("ci").unwrap().more.as_deref(), Some("runs"));
        assert_eq!(ok.contributes.web_views.len(), 4);
        let lines = ok.contributes.lines();
        assert!(lines.iter().any(|l| l == "a section “CI” (ci) in the right panel, for the focused agent"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("“Dashboard” (dash) in a tile: https://ci.example.com/dash, with a browser store of its own")), "{lines:?}");
        let err = |extra: &str| parse(&m(extra)).unwrap_err();
        assert!(err(r#""views": [{"id": "ci", "title": "Runs"}], "panels": [{"id": "ci", "title": "CI"}]"#).contains("share their ids"));
        assert!(err(r#""panels": [{"id": "ci", "title": "CI", "more": "nope"}]"#).contains("isn't one of its views"));
        assert!(err(r#""panels": [{"id": "a", "title": "A"}, {"id": "b", "title": "B"}, {"id": "c", "title": "C"}, {"id": "d", "title": "D"}]"#).contains("more than 3"));
        for bad in ["http://ci.example.com/", "file:///etc/hosts", "javascript:alert(1)", "https://u:p@x.com/"] {
            assert!(err(&format!(r#""webViews": [{{"id": "w", "title": "W", "url": "{bad}"}}]"#)).contains("web view “w”"), "{bad}");
        }
        assert!(err(r#""webViews": [{"id": "w", "title": "W", "url": "{setting:token}/x"}]"#).contains("must be a url setting"));
        assert!(err(r#""webViews": [{"id": "w", "title": "W", "url": "{setting:nope}"}]"#).contains("no setting “nope”"));
        assert!(err(r#""webViews": [{"id": "w", "title": "W", "bridge": true}]"#).contains("no way to reach Pitwall"));
        // A section needs agents.read; one without an adapter, an adapter.
        let no_read = r#"{"v": 2, "id": "ci", "name": "CI", "adapter": {"command": ["node", "a.mjs"]}, "contributes": {"panels": [{"id": "ci", "title": "CI"}]}}"#;
        assert!(parse(no_read).unwrap_err().contains("needs the \"agents.read\" permission"));
        let no_adapter = r#"{"v": 2, "id": "ci", "name": "CI", "contributes": {"webViews": [{"id": "w", "title": "W"}]}}"#;
        assert!(parse(no_adapter).unwrap_err().contains("adapter"));
        let fixed = r#"{"v": 2, "id": "ci", "name": "CI", "contributes": {"webViews": [{"id": "w", "title": "W", "url": "https://x.example.com"}]}}"#;
        assert!(parse(fixed).is_ok(), "a fixed page needs no adapter");
    }

    /// Format 2: the Pitwall it needs (an older one refuses it, saying
    /// which), its permissions with reasons, what it contributes (a kind
    /// this Pitwall doesn't draw is refused, saying so).
    #[test]
    fn format_2_is_read_and_checked() {
        let v2 = |extra: &str| format!(r#"{{"v": 2, "id": "ci", "name": "CI", "adapter": {{"command": ["node", "a.mjs"]}}{extra}}}"#);
        let m = parse_for(&v2(r#", "pitwall": ">=0.9.0", "permissions": {"agents.read": "To match runs to agents", "weather.read": "Later"}, "contributes": {"inbox": true}"#), "0.9.0").unwrap();
        assert_eq!((m.format, m.pitwall.as_deref(), m.contributes.inbox), (2, Some("0.9.0"), true));
        assert_eq!(m.permissions, [PermissionAsk { name: "agents.read".into(), reason: "To match runs to agents".into() }, PermissionAsk { name: "weather.read".into(), reason: "Later".into() }]);
        assert!(m.has_inbox());
        // An older Pitwall refuses it, saying which it needs.
        assert_eq!(parse_for(&v2(r#", "pitwall": ">=0.10.0""#), "0.9.4").unwrap_err(), "it needs Pitwall 0.10.0 or later (this is 0.9.4)");
        assert!(parse_for(&v2(r#", "pitwall": ">=0.10.0""#), "0.10.0").is_ok());
        assert!(parse(&v2(r#", "pitwall": "0.9""#)).unwrap_err().contains("like >=0.9.0"));
        // Without an inbox contribution, format 2 has none (its adapter
        // still runs, for what it reads).
        let m = parse(&v2("")).unwrap();
        assert!(!m.has_inbox() && m.adapter.is_some());
        assert!(parse(&v2(r#", "contributes": {"cards": [{"id": "runs"}]}"#)).unwrap_err().contains("contributes “cards”, which needs a newer Pitwall (this one draws: inbox, views, commands, badges, notifications, panels, webViews)"));
        assert!(parse(&v2(r#", "contributes": {"toasts": true}"#)).unwrap_err().contains("“toasts”, which this Pitwall doesn't know"));
        assert!(parse(r#"{"v": 2, "id": "x", "name": "X", "contributes": {"inbox": true}}"#).unwrap_err().contains("needs an \"adapter\""));
        assert!(parse(&v2(r#", "permissions": {"Agents": "x"}"#)).unwrap_err().contains("lower-case words"));
        assert!(parse(&v2(r#", "permissions": {"agents.read": ""}"#)).unwrap_err().contains("is empty"));
        assert!(parse(&v2(r#", "permissions": {"agents.read": true}"#)).unwrap_err().contains("your reason"));
        assert!(parse(&v2(r#", "permissions": ["agents.read"]"#)).unwrap_err().contains("must be an object"));
        // Format 1 reads exactly as before: these keys are ignored, and an
        // adapter is an inbox.
        let m = parse(r#"{"id": "x", "name": "X", "adapter": {"command": ["node", "a.mjs"]}, "pitwall": ">=99.0.0", "permissions": {"agents.read": "r"}, "contributes": {"views": []}}"#).unwrap();
        assert_eq!((m.format, m.pitwall.clone(), m.permissions.len(), m.has_inbox()), (1, None, 0, true));
    }

    /// Views, commands, badges and notifications: within their limits, each
    /// needing an adapter, notifications the `notify` permission, commands
    /// without keys.
    #[test]
    fn contributions_are_read_and_checked() {
        let v2 = |extra: &str| format!(r#"{{"v": 2, "id": "ci", "name": "CI", "adapter": {{"command": ["node", "a.mjs"]}}{extra}}}"#);
        let m = parse(&v2(r#", "permissions": {"notify": "To say when a run fails"}, "contributes": {"views": [{"id": "runs", "title": "CI runs", "icon": "check"}, {"id": "jobs", "title": "Jobs"}],
            "commands": [{"id": "rerun", "title": "Re-run failed jobs", "context": "agent"}, {"id": "refresh", "title": "Refresh"}], "badges": true, "notifications": true}"#))
        .unwrap();
        let c = &m.contributes;
        assert_eq!(c.views[0], ViewDef { id: "runs".into(), title: "CI runs".into(), icon: Some("check".into()) });
        assert_eq!((c.views[1].icon.clone(), c.commands[0].context, c.commands[1].context), (None, CommandContext::Agent, CommandContext::None));
        assert!(c.badges && c.notifications && !c.inbox && !m.has_inbox());
        assert_eq!(c.view("jobs").map(|v| v.title.as_str()), Some("Jobs"));
        assert_eq!(c.lines().len(), 6);
        assert!(c.lines()[2].contains("while an agent is focused"), "{:?}", c.lines());
        let err = |extra: &str| parse(&v2(extra)).unwrap_err();
        assert!(err(r#", "contributes": {"notifications": true}"#).contains("needs the \"notify\" permission"));
        assert!(err(r#", "contributes": {"views": [{"id": "a", "title": "A"}, {"id": "a", "title": "B"}]}"#).contains("view “a” is there twice"));
        assert!(err(r#", "contributes": {"views": [{"id": "Runs", "title": "A"}]}"#).contains("lower-case"));
        assert!(err(r#", "contributes": {"views": [{"id": "runs"}]}"#).contains("needs a \"title\""));
        assert!(err(&format!(r#", "contributes": {{"views": [{{"id": "runs", "title": "{}"}}]}}"#, "t".repeat(41))).contains("longer than 40"));
        let many: Vec<String> = (0..11).map(|i| format!(r#"{{"id": "v{i}", "title": "V"}}"#)).collect();
        assert!(err(&format!(r#", "contributes": {{"views": [{}]}}"#, many.join(","))).contains("more than 10"));
        assert!(err(r#", "contributes": {"commands": [{"id": "go", "title": "Go", "keys": "cmd-g"}]}"#).contains("no shortcuts"));
        assert!(err(r#", "contributes": {"commands": [{"id": "go", "title": "Go", "context": "project"}]}"#).contains("none or agent"));
        assert!(err(r#", "contributes": {"badges": "yes"}"#).contains("true or false"));
        assert!(parse(r#"{"v": 2, "id": "x", "name": "X", "contributes": {"views": [{"id": "a", "title": "A"}]}}"#).unwrap_err().contains("\"contributes.views\" needs an \"adapter\""));
        assert!(parse(r#"{"v": 2, "id": "x", "name": "X", "contributes": {"badges": true}}"#).unwrap_err().contains("\"contributes.badges\" needs an \"adapter\""));
    }

    /// Text, choice, switch and number settings: each with its checks and
    /// default, kept as text, sent typed.
    #[test]
    fn setting_types_are_read_checked_and_sent_typed() {
        let m = parse(
            r#"{"v": 2, "id": "x", "name": "X", "settings": [
                {"key": "team", "type": "text", "label": "Team", "default": "core"},
                {"key": "level", "type": "choice", "label": "Level", "options": ["low", {"value": "high", "label": "High only"}], "default": "low"},
                {"key": "branchesOnly", "type": "bool", "label": "Only agents' branches", "default": true},
                {"key": "limit", "type": "number", "label": "Limit", "min": 1, "max": 50, "default": 10, "required": true}]}"#,
        )
        .unwrap();
        let s = |k: &str| m.settings.iter().find(|s| s.key == k).unwrap();
        assert_eq!(s("level").options[1], Choice { value: "high".into(), label: "High only".into() });
        assert_eq!((s("branchesOnly").default.as_deref(), s("limit").default.as_deref()), (Some("true"), Some("10")));
        // Sent typed, the kept value over the default.
        assert_eq!(s("team").json(None), Some(serde_json::json!("core")));
        assert_eq!(s("team").json(Some("web")), Some(serde_json::json!("web")));
        assert_eq!(s("branchesOnly").json(Some("false")), Some(serde_json::json!(false)));
        assert_eq!(s("limit").json(Some("25")), Some(serde_json::json!(25)));
        assert_eq!(SettingDef::new("k", SettingKind::Secret, "K").json(Some("x")), None, "secrets go apart");
        assert_eq!(SettingDef::new("k", SettingKind::Text, "K").json(None), None);
        // What typing keeps, or why not.
        assert_eq!(s("limit").check_value(" 7 "), Ok(Some("7".into())));
        assert_eq!(s("limit").check_value("0").unwrap_err(), "Between 1 and 50.");
        assert_eq!(s("limit").check_value("lots").unwrap_err(), "That isn't a number.");
        assert_eq!(s("limit").check_value(""), Ok(None));
        assert_eq!(s("level").check_value("high"), Ok(Some("high".into())));
        assert!(s("level").check_value("High only").is_err(), "the value, not its label");
        assert!(s("team").check_value(&"t".repeat(501)).is_err());
        assert_eq!(s("branchesOnly").check_value("true"), Ok(Some("true".into())));
        // Set: a default counts, a switch always is.
        assert!(s("limit").is_set(None, false) && s("branchesOnly").is_set(None, false));
        assert!(!SettingDef::new("k", SettingKind::Text, "K").is_set(Some("  "), false));
        // A plain one may go in an agent variable, as url ones do.
        assert!(parse(r#"{"id": "x", "name": "X", "settings": [{"key": "team", "type": "text", "label": "T"}], "agentEnv": [{"name": "X_TEAM", "value": "{setting:team}"}]}"#).is_ok());

        let err = |s: &str| parse(&format!(r#"{{"id": "x", "name": "X", "settings": [{s}]}}"#)).unwrap_err();
        assert!(err(r#"{"key": "a", "type": "choice", "label": "A", "options": ["one"]}"#).contains("2–20"));
        assert!(err(r#"{"key": "a", "type": "choice", "label": "A", "options": ["one", "one"]}"#).contains("twice"));
        assert!(err(r#"{"key": "a", "type": "choice", "label": "A", "options": ["one", "two"], "default": "three"}"#).contains("its options"));
        assert!(err(r#"{"key": "a", "type": "number", "label": "A", "min": 5, "max": 1}"#).contains("above"));
        assert!(err(r#"{"key": "a", "type": "number", "label": "A", "max": 3, "default": 9}"#).contains("At most 3"));
        assert!(err(r#"{"key": "a", "type": "bool", "label": "A", "default": "yes"}"#).contains("true or false"));
        assert!(err(r#"{"key": "a", "type": "secret", "label": "A", "default": "k"}"#).contains("can't have a \"default\""));
        assert!(err(r#"{"key": "a", "type": "url", "label": "A", "default": "ftp://x"}"#).contains("http(s)"));
    }

    /// The examples in the repository's `extensions/` folder are usable.
    #[test]
    fn the_example_extensions_are_usable() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
        let found = discover(&dir);
        assert!(!found.is_empty());
        for f in found {
            assert!(f.manifest.is_ok(), "{}: {:?}", f.folder, f.manifest);
        }
    }

    #[test]
    fn extensions_are_found_in_their_folders() {
        let t = crate::testing::TempDir::new("extensions");
        let dir = t.path().join("extensions");
        assert!(discover(&dir).is_empty(), "no folder: none");
        let put = |folder: &str, text: Option<&str>| {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
            if let Some(text) = text {
                std::fs::write(dir.join(folder).join(MANIFEST), text).unwrap();
            }
        };
        put("zed", Some(r#"{"id": "zed", "name": "Zed"}"#));
        put("inbox", Some(r#"{"id": "inbox", "name": "Inbox", "icon": "inbox"}"#));
        put("renamed", Some(r#"{"id": "other", "name": "Other"}"#));
        put("empty", None);
        put(".hidden", Some(r#"{"id": "hidden", "name": "Hidden"}"#));
        std::fs::write(dir.join("stray.json"), "{}").unwrap();
        put("broken", Some("{"));
        let found = discover(&dir);
        let folders: Vec<&str> = found.iter().map(|f| f.folder.as_str()).collect();
        assert_eq!(folders, ["broken", "empty", "inbox", "renamed", "zed"]);
        assert!(found[0].manifest.as_ref().unwrap_err().contains("isn't valid JSON"));
        assert!(found[1].manifest.as_ref().unwrap_err().contains("no extension.json"));
        assert_eq!(found[2].manifest.as_ref().unwrap().name, "Inbox");
        assert!(found[3].manifest.as_ref().unwrap_err().contains("named after its id"));
        assert_eq!(found[3].key(), "renamed");
        assert_eq!(found[4].key(), "zed");
    }
}
