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
//! `icon`, `description`, `settings`, `adapter`, `agentEnv`, `matchAgents` and
//! `actions`. Keys it doesn't
//! know are ignored, so a later version can add its own without breaking
//! this one; a manifest asking for a newer format (`v`) is refused with a
//! word on why.

pub mod install;
pub mod protocol;

use std::path::Path;

use serde_json::Value;

/// The manifest's file name in an extension's folder.
pub const MANIFEST: &str = "extension.json";

/// The manifest format this Pitwall reads.
pub const FORMAT: u64 = 1;

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
}

impl Manifest {
    /// A manifest with only these (tests and fixtures).
    pub fn named(id: &str, name: &str) -> Manifest {
        Manifest { id: id.into(), name: name.into(), version: None, demo: false, docs: None, icon: None, description: None, settings: vec![], adapter: None, agent_env: vec![], match_agents: None, actions: vec![] }
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
}

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// An http(s) address, kept in `ui.json`.
    Url,
    /// A key or token, kept in the system's secure store (`ui.json` only
    /// says that it is set).
    Secret,
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

/// Read and check manifest `text`.
pub fn parse(text: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{MANIFEST} isn't valid JSON: {e}"))?;
    let Value::Object(obj) = v else {
        return Err(format!("{MANIFEST} must be a JSON object"));
    };
    match obj.get("v") {
        None => {}
        Some(n) => match n.as_u64() {
            Some(n) if n == FORMAT => {}
            Some(n) if n > FORMAT => return Err(format!("it needs a newer Pitwall (manifest format {n}; this one reads {FORMAT})")),
            _ => return Err(format!("\"v\" must be {FORMAT}")),
        },
    }
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
    Ok(Manifest { id: id.to_string(), name, version, demo, docs, icon, description, settings, adapter, agent_env, match_agents, actions })
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
            Some(t) => return Err(format!("setting “{key}” has type “{t}”: url or secret")),
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
        out.push(SettingDef { key: key.to_string(), kind, label, hint, placeholder, required });
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

/// Read and check the manifest in folder `dir` (named `folder`): its id
/// must be the folder's name.
pub fn read(dir: &Path, folder: &str) -> Result<Manifest, String> {
    let file = dir.join(MANIFEST);
    let meta = std::fs::metadata(&file).map_err(|_| format!("there is no {MANIFEST} in it"))?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{MANIFEST} is bigger than {} KiB", MAX_BYTES / 1024));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{MANIFEST} can't be read: {e}"))?;
    let m = parse(&text)?;
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
        assert!(err(r#""settings": [{"key": "a", "type": "color", "label": "A"}]"#).contains("url or secret"));
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
        assert!(err(r#"{"v": 2, "id": "x", "name": "X"}"#).contains("needs a newer Pitwall"));
        assert!(err(r#"{"v": 0, "id": "x", "name": "X"}"#).contains("\"v\" must be 1"));
        assert!(err(r#"{"v": "1", "id": "x", "name": "X"}"#).contains("\"v\" must be 1"));
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
