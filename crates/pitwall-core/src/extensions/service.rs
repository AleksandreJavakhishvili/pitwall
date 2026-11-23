//! An extension's service (docs/spec/extensions.md, "A service"): one
//! long-running program on this computer that Pitwall keeps running while
//! the extension is on, such as the server its agents talk to. A format 2
//! manifest declares it under `service`:
//!
//! ```json
//! "service": {
//!   "label": "The inbox server",
//!   "run": ["inbox", "serve", "--data", "{data}", "--listen", "{setting:listen}"],
//!   "install": "server",
//!   "version": { "run": ["inbox", "version"], "match": "(\\d+\\.\\d+\\.\\d+)", "requires": ">=0.6.0" },
//!   "ready": { "listen": "{setting:listen}" },
//!   "env": [{ "name": "INBOX_KEY", "secret": "apiKey" }],
//!   "firstRun": { "label": "Sign in for the first time", "run": ["inbox", "link", "--data", "{data}"], "opens": true }
//! }
//! ```
//!
//! Every command is a program and its arguments, never a shell line, with
//! the placeholders actions have (`{setting:KEY}`, a plain setting) and
//! `{data}`: the service's private folder in Pitwall's data folder
//! (`Paths::extension_data_dir`, made with mode 700), outside the
//! extension's own folder. This module reads and checks the block, says
//! what it runs in plain words ([`lines`]) and decides, from what is found
//! on this computer, whether Pitwall starts it ([`verdict`]); the app runs
//! it (`crate::extensions::service` in the app). Nothing here runs a
//! program.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use serde_json::Value;

use super::{array, line, opt_str, reserved_env, setting_keys, Action, Manifest, SettingDef, SettingKind, SETTING_PLACEHOLDER};

/// The placeholder for the service's private data folder.
pub const DATA_PLACEHOLDER: &str = "{data}";

/// The most variables a service is given besides the minimal environment.
pub const MAX_ENV: usize = 8;

/// A long-running program Pitwall runs for the extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// What it is called on its card ("The inbox server").
    pub label: String,
    /// The program (found on the login shell's PATH) and its arguments.
    pub run: Vec<String>,
    /// The action that installs its program (one of `actions`).
    pub install: Option<String>,
    /// How its version is told, and the lowest it needs.
    pub version: Option<VersionProbe>,
    /// How Pitwall tells it is up.
    pub ready: Ready,
    /// Variables it is given on top of the minimal environment.
    pub env: Vec<ServiceEnv>,
    /// A command for a fresh service, run on a press (a first sign-in).
    pub first_run: Option<FirstRun>,
}

/// A command whose output holds the program's version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionProbe {
    pub run: Vec<String>,
    /// A regular expression with one group: the version.
    pub pattern: String,
    /// The lowest version the extension needs (`>=X.Y.Z`, the version
    /// kept).
    pub requires: Option<String>,
}

/// How Pitwall tells the service is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    /// Once it has run a moment without exiting.
    Alive,
    /// Once a line of its output matches this regular expression.
    Line(String),
    /// Once something listens on this address of this computer (a
    /// template: `{setting:KEY}` or `127.0.0.1:8787`); before it starts,
    /// the address already taken means it runs elsewhere.
    Listen(String),
}

/// A variable the service gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEnv {
    pub name: String,
    pub from: EnvFrom,
}

/// Where a service's variable comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvFrom {
    /// A template (`{setting:KEY}`, `{data}`).
    Value(String),
    /// A secret setting, by key: shown on the confirmation, never logged.
    Secret(String),
}

/// A command for a fresh service, run only on a press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstRun {
    pub label: String,
    pub run: Vec<String>,
    /// The first https address it prints is opened in the browser (a
    /// one-time link: never logged or shown).
    pub opens: bool,
    /// What running it again does, said before it does (a new link that
    /// replaces the old sign-in, say).
    pub again: Option<String>,
}

/// Read and check `service` (`settings` and `actions`: the manifest's).
pub(super) fn parse(v: &Value, settings: &[SettingDef], actions: &[Action]) -> Result<Service, String> {
    let Value::Object(o) = v else { return Err("\"service\" must be an object".into()) };
    let label = line("service.label", opt_str(o, "label")?.ok_or("\"service\" needs \"label\": what it is called")?, 40)?;
    let run = argv("service.run", o.get("run"), settings)?;
    let install = match opt_str(o, "install")? {
        Some(id) if actions.iter().any(|a| a.id == id) => Some(id.to_string()),
        Some(id) => return Err(format!("\"service.install\" names action “{id}”, which the manifest doesn't have")),
        None => None,
    };
    let version = match o.get("version") {
        None | Some(Value::Null) => None,
        Some(Value::Object(vo)) => {
            let run = argv("service.version.run", vo.get("run"), settings)?;
            let pattern = opt_str(vo, "match")?.ok_or("\"service.version\" needs \"match\": a pattern with one group, the version")?.to_string();
            let re = regex::Regex::new(&pattern).map_err(|e| format!("\"service.version.match\" isn't a valid pattern: {e}"))?;
            if re.captures_len() != 2 {
                return Err("\"service.version.match\" must have exactly one group: the version".into());
            }
            let requires = match opt_str(vo, "requires")? {
                Some(r) => Some(min_version(r).ok_or(format!("\"service.version.requires\" “{r}” must be like >=0.6.0"))?),
                None => None,
            };
            Some(VersionProbe { run, pattern, requires })
        }
        Some(_) => return Err("\"service.version\" must be an object: \"run\", \"match\" and \"requires\"".into()),
    };
    let ready = match o.get("ready") {
        None | Some(Value::Null) => Ready::Alive,
        Some(Value::Object(r)) => match (opt_str(r, "listen")?, opt_str(r, "line")?) {
            (Some(addr), None) => {
                check_args("\"service.ready.listen\"", addr, settings, false)?;
                if setting_keys(addr).is_empty() {
                    loopback(addr).map_err(|e| format!("\"service.ready.listen\": {e}"))?;
                }
                Ready::Listen(addr.to_string())
            }
            (None, Some(pat)) => {
                regex::Regex::new(pat).map_err(|e| format!("\"service.ready.line\" isn't a valid pattern: {e}"))?;
                Ready::Line(pat.to_string())
            }
            _ => return Err("\"service.ready\" needs one of \"listen\" (an address on this computer) or \"line\" (a pattern its output matches)".into()),
        },
        Some(_) => return Err("\"service.ready\" must be an object".into()),
    };
    let mut env: Vec<ServiceEnv> = Vec::new();
    for item in array(o.get("env"), "service.env", MAX_ENV)? {
        let Value::Object(e) = item else { return Err("each of \"service.env\" must be an object".into()) };
        let name = opt_str(e, "name")?.ok_or("a variable of \"service.env\" needs \"name\"")?;
        let ok = name.len() <= 64 && name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_') && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !ok {
            return Err(format!("service variable “{name}” must be upper-case letters, digits and _"));
        }
        if reserved_env(name) || env.iter().any(|x| x.name == name) {
            return Err(format!("service variable “{name}” is one an extension may not set, or is there twice"));
        }
        let from = match (opt_str(e, "value")?, opt_str(e, "secret")?) {
            (Some(v), None) => {
                check_args(&format!("the value of service variable {name}"), v, settings, true)?;
                EnvFrom::Value(v.to_string())
            }
            (None, Some(key)) => match settings.iter().find(|s| s.key == key) {
                Some(s) if s.kind == SettingKind::Secret => EnvFrom::Secret(key.to_string()),
                Some(_) => return Err(format!("service variable {name} names “{key}” as a secret, but it isn't one: use \"value\": \"{{setting:{key}}}\"")),
                None => return Err(format!("service variable {name} names secret “{key}”, which the manifest doesn't have")),
            },
            _ => return Err(format!("service variable {name} needs one of \"value\" or \"secret\"")),
        };
        env.push(ServiceEnv { name: name.to_string(), from });
    }
    let first_run = match o.get("firstRun") {
        None | Some(Value::Null) => None,
        Some(Value::Object(f)) => {
            let label = line("service.firstRun.label", opt_str(f, "label")?.ok_or("\"service.firstRun\" needs \"label\"")?, 40)?;
            let run = argv("service.firstRun.run", f.get("run"), settings)?;
            let opens = match f.get("opens") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err("\"service.firstRun.opens\" must be true or false".into()),
            };
            let again = opt_str(f, "again")?.map(|a| line("service.firstRun.again", a, 200)).transpose()?;
            Some(FirstRun { label, run, opens, again })
        }
        Some(_) => return Err("\"service.firstRun\" must be an object".into()),
    };
    Ok(Service { label, run, install, version, ready, env, first_run })
}

/// `>=X.Y.Z` → `X.Y.Z`.
fn min_version(r: &str) -> Option<String> {
    let w = r.trim().strip_prefix(">=")?.trim();
    (w.len() <= 32 && w.starts_with(|c: char| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))).then(|| w.to_string())
}

/// A command: the program (a name on PATH) and its arguments, 1–32
/// strings of one line, using only `{setting:KEY}` and `{data}`.
fn argv(what: &str, v: Option<&Value>, settings: &[SettingDef]) -> Result<Vec<String>, String> {
    let list = match v {
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= 32 => a,
        Some(Value::String(_)) => return Err(format!("\"{what}\" must be the program and its arguments, a list, not one shell line")),
        _ => return Err(format!("\"{what}\" needs the program and its arguments, 1–32 strings")),
    };
    let mut out = Vec::new();
    for (i, a) in list.iter().enumerate() {
        let Value::String(a) = a else { return Err(format!("\"{what}\" must be strings")) };
        if (i == 0 && a.is_empty()) || a.chars().count() > 500 || a.chars().any(char::is_control) {
            return Err(format!("each of \"{what}\" must be one line of at most 500 characters"));
        }
        if i == 0 {
            check_word_program(what, a)?;
        }
        check_args(&format!("an argument of \"{what}\""), a, settings, true)?;
        out.push(a.clone());
    }
    Ok(out)
}

fn check_word_program(what: &str, a: &str) -> Result<(), String> {
    if a.contains('{') || a.contains('/') || a.contains('\\') {
        return Err(format!("\"{what}\": the program must be a name found on PATH, not a path or a placeholder"));
    }
    Ok(())
}

/// Template `a` may use `{setting:KEY}` of plain settings, and `{data}`
/// when `data`.
fn check_args(what: &str, a: &str, settings: &[SettingDef], data: bool) -> Result<(), String> {
    let mut rest = a.to_string();
    for key in setting_keys(a) {
        match settings.iter().find(|s| s.key == key) {
            None => return Err(format!("{what} names setting “{key}”, which the manifest doesn't have")),
            Some(s) if s.kind == SettingKind::Secret => return Err(format!("{what} names secret “{key}”: a secret goes to a service only as a variable (\"env\" with \"secret\")")),
            Some(_) => rest = rest.replace(&format!("{SETTING_PLACEHOLDER}{key}}}"), ""),
        }
    }
    if data {
        rest = rest.replace(DATA_PLACEHOLDER, "");
    }
    if rest.contains('{') || rest.contains('}') {
        return Err(format!("{what} “{a}” may only use {{setting:KEY}}{}", if data { " and {data}" } else { "" }));
    }
    Ok(())
}

/// A digest of everything the user confirms of `s`: its commands, how it
/// is told up, the variables it gets and where from (FNV-1a, as
/// [`super::steps_digest`]). The settings put in aren't part of it.
pub fn digest(s: &Service) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    let list = |tag: &str, argv: &[String], eat: &mut dyn FnMut(&[u8])| {
        eat(tag.as_bytes());
        for a in argv {
            eat(&(a.len() as u64).to_le_bytes());
            eat(a.as_bytes());
        }
    };
    list("run", &s.run, &mut eat);
    if let Some(v) = &s.version {
        list("version", &v.run, &mut eat);
        eat(v.pattern.as_bytes());
    }
    match &s.ready {
        Ready::Alive => eat(b"alive"),
        Ready::Line(p) => eat(format!("line {p}").as_bytes()),
        Ready::Listen(a) => eat(format!("listen {a}").as_bytes()),
    }
    for e in &s.env {
        match &e.from {
            EnvFrom::Value(v) => eat(format!("{}={v}", e.name).as_bytes()),
            EnvFrom::Secret(k) => eat(format!("{}=secret:{k}", e.name).as_bytes()),
        }
    }
    if let Some(f) = &s.first_run {
        list("first", &f.run, &mut eat);
        eat(&[f.opens as u8]);
    }
    format!("{h:016x}")
}

/// Template `run` with `m`'s plain settings `values` (a setting's default
/// while it isn't set) and `data` put in; `Err(label)` naming a setting
/// that isn't set and has no default.
pub fn expand(run: &[String], m: &Manifest, values: &BTreeMap<String, String>, data: &str) -> Result<Vec<String>, String> {
    let mut all = values.clone();
    for s in m.settings.iter().filter(|s| s.kind != SettingKind::Secret) {
        if let Some(d) = &s.default {
            let unset = all.get(&s.key).is_none_or(|v| v.trim().is_empty());
            if unset {
                all.insert(s.key.clone(), d.clone());
            }
        }
    }
    Ok(super::expand_args(run, m, &all)?.into_iter().map(|a| a.replace(DATA_PLACEHOLDER, data)).collect())
}

/// One template, as [`expand`].
pub fn expand_one(t: &str, m: &Manifest, values: &BTreeMap<String, String>, data: &str) -> Result<String, String> {
    Ok(expand(&[t.to_string()], m, values, data)?.remove(0))
}

/// The commands of `m`'s service with `values` and `data` put in, as they
/// will run (a setting not set: its placeholder left as it is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commands {
    pub run: Vec<String>,
    pub version: Option<Vec<String>>,
    pub first_run: Option<Vec<String>>,
    /// Each variable as shown: `NAME=value`, or for a secret `NAME` and
    /// which secret ("your API key, a secret").
    pub env: Vec<String>,
    /// The label of a setting it needs that isn't set.
    pub missing: Option<String>,
}

/// [`Commands`] of service `s` of `m`.
pub fn commands(m: &Manifest, s: &Service, values: &BTreeMap<String, String>, data: &str) -> Commands {
    let mut missing = None;
    let mut one = |run: &[String]| match expand(run, m, values, data) {
        Ok(v) => v,
        Err(label) => {
            missing.get_or_insert(label);
            run.iter().map(|a| a.replace(DATA_PLACEHOLDER, data)).collect()
        }
    };
    let run = one(&s.run);
    let version = s.version.as_ref().map(|v| one(&v.run));
    let first_run = s.first_run.as_ref().map(|f| one(&f.run));
    let mut env = Vec::new();
    for e in &s.env {
        match &e.from {
            EnvFrom::Value(v) => env.push(format!("{}={}", e.name, one(std::slice::from_ref(v)).remove(0))),
            EnvFrom::Secret(k) => env.push(format!("{} = your {} (a secret, from the system's secure store)", e.name, m.settings.iter().find(|s| s.key == *k).map_or(k.as_str(), |s| s.label.as_str()))),
        }
    }
    if let Ready::Listen(a) = &s.ready {
        one(std::slice::from_ref(a));
    }
    Commands { run, version, first_run, env, missing }
}

/// What `m`'s service does, in plain words, one heading and its lines
/// each: the confirmation, "What this extension does" and `check` say
/// these.
pub fn lines(m: &Manifest, s: &Service, values: &BTreeMap<String, String>, data: &str) -> Vec<(String, Vec<String>)> {
    let c = commands(m, s, values, data);
    let mut out = vec![(format!("While it is on, Pitwall keeps {} running (it runs continuously, as you):", s.label), vec![super::plan::line(&c.run)])];
    if let Some(v) = &c.version {
        out.push(("Checks its version first with:".to_string(), vec![super::plan::line(v)]));
    }
    if let Some(f) = c.first_run.as_ref().zip(s.first_run.as_ref()) {
        let what = if f.1.opens { ", opening the link it prints in your browser" } else { "" };
        out.push((format!("“{}” runs, only when you press it{what}:", f.1.label), vec![super::plan::line(f.0)]));
    }
    if !c.env.is_empty() {
        out.push(("With these variables (and only your PATH, home and language besides):".to_string(), c.env));
    }
    out.push(("Its data is kept in (private to you; updates never touch it):".to_string(), vec![data.to_string()]));
    out
}

/// The version in `output`, by `pattern`'s group.
pub fn version_in(output: &str, pattern: &str) -> Option<String> {
    let re = regex::Regex::new(pattern).ok()?;
    re.captures(output)?.get(1).map(|g| g.as_str().trim().to_string()).filter(|v| !v.is_empty())
}

/// Whether version `found` is below `need`.
pub fn too_old(found: &str, need: &str) -> bool {
    super::install::compare_versions(found, need) == std::cmp::Ordering::Less
}

/// `text` as an address on this computer (`127.0.0.1:8787`,
/// `localhost:8787`, `[::1]:8787`); why not, in plain words. Pitwall
/// checks and runs only those: nothing it starts listens for others.
pub fn loopback(text: &str) -> Result<SocketAddr, String> {
    let t = text.trim();
    let t2 = t.strip_prefix("localhost:").map(|p| format!("127.0.0.1:{p}"));
    let addr: SocketAddr = t2.as_deref().unwrap_or(t).parse().map_err(|_| format!("“{t}” isn't an address and port like 127.0.0.1:8787"))?;
    let local = match addr.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    };
    if !local {
        return Err(format!("“{t}” isn't on this computer only: use 127.0.0.1 and a port"));
    }
    if addr.port() == 0 {
        return Err(format!("“{t}” needs a port"));
    }
    Ok(addr)
}

/// The address service `s` of `m` listens on with `values` (`None`: it
/// isn't told by an address; `Err`: a setting isn't set or it isn't on
/// this computer).
pub fn listen_of(m: &Manifest, s: &Service, values: &BTreeMap<String, String>) -> Option<Result<SocketAddr, String>> {
    let Ready::Listen(t) = &s.ready else { return None };
    Some(match expand_one(t, m, values, "") {
        Ok(a) => loopback(&a),
        Err(label) => Err(format!("set {label} first")),
    })
}

/// Whether something already listens on `addr` (this computer only):
/// binding it fails because it is in use. Nothing is connected to.
pub fn taken(addr: SocketAddr) -> bool {
    matches!(std::net::TcpListener::bind(addr), Err(e) if e.kind() == std::io::ErrorKind::AddrInUse)
}

/// The first https address in `text` (a first run's one-time link).
pub fn first_https_url(text: &str) -> Option<String> {
    for (i, _) in text.match_indices("https://") {
        let rest = &text[i..];
        let end = rest.find(|c: char| c.is_whitespace() || "\"'<>`".contains(c)).unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(['.', ',', ';', ':', ')', ']', '}', '!', '?']);
        if url.len() > "https://".len() + 1 && url["https://".len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '[') {
            return Some(url.to_string());
        }
    }
    None
}

/// `text` with every web address taken out (`[link]`): what a first run
/// printed, before any of it is kept or shown.
pub fn scrub_links(text: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?i)\b[a-z][a-z0-9+.-]*://[^\s\x22'<>`]+").expect("a valid pattern"));
    re.replace_all(text, "[link]").into_owned()
}

/// Make the service's data folder `dir` (mode 700, its parents too) when
/// it isn't there: whether it is fresh (made now, or empty).
pub fn prepare_data(dir: &Path) -> std::io::Result<bool> {
    let fresh = match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => {
            std::fs::create_dir_all(dir)?;
            true
        }
    };
    crate::platform::make_private(dir)?;
    Ok(fresh)
}

/// The variables of this process a service keeps: what a program needs to
/// run as the user (home, user, language, temporary folder, and on
/// Windows its system folders), nothing else. Names compared ignoring
/// case (Windows).
const KEEP_ENV: [&str; 26] = [
    "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "LC_MESSAGES", "TMPDIR", "TZ", "SHELL", "XDG_RUNTIME_DIR",
    "SYSTEMROOT", "WINDIR", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP", "COMSPEC", "PATHEXT", "HOMEDRIVE", "HOMEPATH", "USERNAME", "PROGRAMDATA", "PROGRAMFILES", "SYSTEMDRIVE",
];

/// The minimal environment a service starts with: [`KEEP_ENV`] from
/// `vars` (this process's), and `PATH` set to `path` (the login shell's).
pub fn minimal_env(vars: impl Iterator<Item = (String, String)>, path: Option<&str>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vars.filter(|(k, _)| KEEP_ENV.iter().any(|w| w.eq_ignore_ascii_case(k))).collect();
    out.sort();
    if let Some(p) = path {
        out.push(("PATH".into(), p.to_string()));
    }
    out
}

/// What is found on this computer for a service, before it starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    /// Its program is on the PATH.
    pub installed: bool,
    /// What its version command printed (`None`: no command, or it
    /// couldn't run).
    pub version_output: Option<String>,
    /// Its address is already taken.
    pub port_taken: bool,
}

/// What Pitwall does with a service, from what is found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A setting it uses isn't set (its label): not started.
    NeedsSetting(String),
    /// Its address isn't on this computer (why): not started.
    BadAddress(String),
    /// Something else already listens on its address: that one is used,
    /// nothing is started and nothing is stopped.
    Elsewhere(String),
    /// Its program isn't installed (its name): its install action offered.
    NotInstalled(String),
    /// Its version is below the one needed: its install action offered,
    /// not started.
    TooOld { found: String, need: String },
    /// Start it; its version when told.
    Start { version: Option<String> },
}

/// [`Verdict`] for service `s` of `m` with `values` and `probe`.
pub fn verdict(m: &Manifest, s: &Service, values: &BTreeMap<String, String>, probe: &Probe) -> Verdict {
    if let Some(label) = commands(m, s, values, "").missing {
        return Verdict::NeedsSetting(label);
    }
    if let Some(addr) = listen_of(m, s, values) {
        match addr {
            Err(why) => return Verdict::BadAddress(why),
            Ok(a) if probe.port_taken => return Verdict::Elsewhere(a.to_string()),
            Ok(_) => {}
        }
    }
    if !probe.installed {
        return Verdict::NotInstalled(s.run[0].clone());
    }
    let version = s.version.as_ref().and_then(|v| version_in(probe.version_output.as_deref()?, &v.pattern));
    if let (Some(found), Some(need)) = (&version, s.version.as_ref().and_then(|v| v.requires.as_ref())) {
        if too_old(found, need) {
            return Verdict::TooOld { found: found.clone(), need: need.clone() };
        }
    }
    Verdict::Start { version }
}

impl Verdict {
    /// In plain words, for `check` and `test` (service `s`'s).
    pub fn words(&self, s: &Service) -> String {
        let label = &s.label;
        let offered = if s.install.is_some() { ": its install action is offered" } else { "" };
        match self {
            Verdict::NeedsSetting(l) => format!("waits for {l} to be set"),
            Verdict::BadAddress(why) => format!("isn't started: {why}"),
            Verdict::Elsewhere(a) => format!("something else is already listening on {a}: that one is used, nothing is started"),
            Verdict::NotInstalled(p) => format!("“{p}” isn't installed{offered}"),
            Verdict::TooOld { found, need } => format!("{label} {found}; this extension needs {need} or later{offered}"),
            Verdict::Start { version: Some(v) } => format!("starts ({label} {v})"),
            Verdict::Start { version: None } => "starts".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(service: &str) -> Result<Manifest, String> {
        super::super::parse(&format!(
            r#"{{"v": 2, "id": "x", "name": "X", "settings": [
                {{"key": "server", "type": "url", "label": "Server address"}},
                {{"key": "listen", "type": "text", "label": "Listen on", "default": "127.0.0.1:8787"}},
                {{"key": "apiKey", "type": "secret", "label": "API key"}}],
              "actions": [{{"id": "server", "label": "Install", "description": "d", "steps": [{{"run": ["npm", "i", "-g", "srv"]}}]}}],
              "service": {service}}}"#
        ))
    }

    const GOOD: &str = r#"{"label": "The server", "run": ["srv", "serve", "--data", "{data}", "--origin", "{setting:server}", "--listen", "{setting:listen}"],
        "install": "server", "version": {"run": ["srv", "version"], "match": "v?(\\d+\\.\\d+\\.\\d+)", "requires": ">=0.6.0"},
        "ready": {"listen": "{setting:listen}"}, "env": [{"name": "SRV_KEY", "secret": "apiKey"}, {"name": "SRV_HOME", "value": "{data}/home"}],
        "firstRun": {"label": "Sign in for the first time", "run": ["srv", "link", "--data", "{data}"], "opens": true, "again": "A new link replaces your sign-in."}}"#;

    #[test]
    fn a_service_is_read_and_checked() {
        let m = manifest(GOOD).unwrap();
        let s = m.service.as_ref().unwrap();
        assert_eq!(s.install.as_deref(), Some("server"));
        assert_eq!(s.ready, Ready::Listen("{setting:listen}".into()));
        assert_eq!(s.version.as_ref().unwrap().requires.as_deref(), Some("0.6.0"));
        assert!(s.first_run.as_ref().unwrap().opens);
        assert_eq!(s.env[0].from, EnvFrom::Secret("apiKey".into()));
        let bad = |svc: &str| manifest(svc).unwrap_err();
        assert!(bad(r#"{"label": "S", "run": "srv serve --data {data}"}"#).contains("not one shell line"));
        assert!(bad(r#"{"label": "S", "run": ["/usr/bin/srv"]}"#).contains("not a path"));
        assert!(bad(r#"{"label": "S", "run": ["{data}"]}"#).contains("not a path or a placeholder"));
        assert!(bad(r#"{"label": "S", "run": ["srv", "{project}"]}"#).contains("may only use {setting:KEY} and {data}"));
        assert!(bad(r#"{"label": "S", "run": ["srv", "{setting:nope}"]}"#).contains("doesn't have"));
        assert!(bad(r#"{"label": "S", "run": ["srv", "--key", "{setting:apiKey}"]}"#).contains("only as a variable"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "install": "nope"}"#).contains("action “nope”"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "ready": {"listen": "0.0.0.0:80"}}"#).contains("this computer only"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "ready": {"listen": "{data}"}}"#).contains("may only use {setting:KEY}"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "ready": {"line": "("}}"#).contains("isn't a valid pattern"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "ready": {}}"#).contains("one of"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "version": {"run": ["srv", "-v"], "match": "\\d+"}}"#).contains("exactly one group"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "version": {"run": ["srv", "-v"], "match": "(\\d"}}"#).contains("isn't a valid pattern"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "version": {"run": ["srv", "-v"], "match": "(.*)", "requires": "0.6"}}"#).contains("like >=0.6.0"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "env": [{"name": "PATH", "value": "x"}]}"#).contains("may not set"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "env": [{"name": "K", "secret": "server"}]}"#).contains("isn't one"));
        assert!(bad(r#"{"label": "S", "run": ["srv"], "firstRun": {"label": "L", "run": ["srv"], "opens": "yes"}}"#).contains("true or false"));
        assert!(bad(r#"[]"#).contains("must be an object"));
        // Format 1 ignores it, as it ignores every format 2 key.
        let m = super::super::parse(r#"{"id": "x", "name": "X", "service": {"run": "anything"}}"#).unwrap();
        assert!(m.service.is_none());
    }

    #[test]
    fn the_digest_follows_the_commands_not_the_settings() {
        let a = manifest(GOOD).unwrap().service.unwrap();
        let mut b = a.clone();
        assert_eq!(digest(&a), digest(&b));
        b.run.push("--quiet".into());
        assert_ne!(digest(&a), digest(&b));
        let mut c = a.clone();
        c.first_run.as_mut().unwrap().run[1] = "link2".into();
        assert_ne!(digest(&a), digest(&c));
        let mut d = a.clone();
        d.env[0].from = EnvFrom::Value("x".into());
        assert_ne!(digest(&a), digest(&d));
        let mut e = a.clone();
        e.label = "Another name".into();
        assert_eq!(digest(&a), digest(&e), "the label isn't a command");
    }

    #[test]
    fn commands_are_shown_as_they_run() {
        let m = manifest(GOOD).unwrap();
        let s = m.service.as_ref().unwrap();
        let mut values = BTreeMap::new();
        let c = commands(&m, s, &values, "/data/x");
        assert_eq!(c.missing.as_deref(), Some("Server address"));
        values.insert("server".to_string(), "https://inbox.example.ts.net".to_string());
        values.insert("listen".to_string(), "127.0.0.1:9911".to_string());
        let c = commands(&m, s, &values, "/data/x");
        assert_eq!(c.missing, None);
        assert_eq!(super::super::plan::line(&c.run), "srv serve --data /data/x --origin https://inbox.example.ts.net --listen 127.0.0.1:9911");
        assert_eq!(c.first_run.unwrap(), ["srv", "link", "--data", "/data/x"]);
        assert_eq!(c.env, ["SRV_KEY = your API key (a secret, from the system's secure store)", "SRV_HOME=/data/x/home"]);
        let text: Vec<String> = lines(&m, s, &values, "/data/x").into_iter().map(|(h, l)| format!("{h} {}", l.join(" | "))).collect();
        assert!(text[0].contains("keeps The server running (it runs continuously, as you)"), "{text:?}");
        assert!(text.iter().any(|t| t.contains("“Sign in for the first time” runs, only when you press it, opening the link")), "{text:?}");
    }

    #[test]
    fn versions_are_read_and_compared() {
        assert_eq!(version_in("agent server v0.5.2\n", r"v?(\d+\.\d+\.\d+)").as_deref(), Some("0.5.2"));
        assert_eq!(version_in("nothing here", r"(\d+\.\d+\.\d+)"), None);
        assert!(too_old("0.5.9", "0.6.0"));
        assert!(!too_old("0.6.0", "0.6.0"));
        assert!(!too_old("0.10.0", "0.6.0"));
        assert!(too_old("0.6.0-beta.1", "0.6.0"));
    }

    #[test]
    fn only_addresses_on_this_computer() {
        assert_eq!(loopback("127.0.0.1:8787").unwrap().port(), 8787);
        assert!(loopback("localhost:8787").unwrap().ip().is_loopback());
        assert!(loopback("[::1]:8787").is_ok());
        assert!(loopback("0.0.0.0:8787").unwrap_err().contains("this computer only"));
        assert!(loopback("192.168.1.2:8787").is_err());
        assert!(loopback("127.0.0.1").unwrap_err().contains("like 127.0.0.1:8787"));
        assert!(loopback("127.0.0.1:0").unwrap_err().contains("needs a port"));
    }

    /// A port in use reads as taken (bound here on a port of the system's
    /// choosing); a free one doesn't. Nothing connects to it.
    #[test]
    fn a_taken_port_is_told_by_binding() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = held.local_addr().unwrap();
        assert!(taken(addr));
        drop(held);
        assert!(!taken(addr));
    }

    #[test]
    fn the_first_https_link_is_found_and_links_are_scrubbed() {
        let out = "Open this once:\n  http://insecure.example/x\n  https://inbox.example.ts.net/link/abc123?t=9. Then https://second.example\n";
        assert_eq!(first_https_url(out).as_deref(), Some("https://inbox.example.ts.net/link/abc123?t=9"));
        assert_eq!(first_https_url("http://only.example/plain"), None, "https only");
        assert_eq!(first_https_url("no link at all"), None);
        assert_eq!(first_https_url("“https://” alone"), None);
        assert_eq!(first_https_url("<https://a.example/x>"), Some("https://a.example/x".into()));
        let s = scrub_links(out);
        assert!(!s.contains("abc123") && !s.contains("insecure") && s.contains("[link]"), "{s}");
    }

    #[test]
    fn its_data_folder_is_private_and_fresh_once() {
        let t = crate::testing::TempDir::new("svc-data");
        let d = t.path().join("extension-data").join("x");
        assert!(prepare_data(&d).unwrap(), "made now: fresh");
        if let Some(mode) = crate::platform::mode(&d) {
            assert_eq!(mode & 0o777, 0o700);
        }
        assert!(prepare_data(&d).unwrap(), "still empty: fresh");
        std::fs::write(d.join("db"), "x").unwrap();
        assert!(!prepare_data(&d).unwrap(), "used: not fresh");
    }

    #[test]
    fn the_environment_is_minimal() {
        let vars = vec![("HOME".to_string(), "/h".to_string()), ("AWS_SECRET_ACCESS_KEY".to_string(), "s".to_string()), ("PATH".to_string(), "/old".to_string()), ("SystemRoot".to_string(), "C:\\W".to_string())];
        let env = minimal_env(vars.into_iter(), Some("/login/bin"));
        assert_eq!(env, [("HOME".to_string(), "/h".to_string()), ("SystemRoot".to_string(), "C:\\W".to_string()), ("PATH".to_string(), "/login/bin".to_string())]);
    }

    #[test]
    fn the_verdict_follows_what_is_found() {
        let m = manifest(GOOD).unwrap();
        let s = m.service.as_ref().unwrap();
        let mut values = BTreeMap::new();
        let p = |installed: bool, v: Option<&str>, taken: bool| Probe { installed, version_output: v.map(String::from), port_taken: taken };
        assert_eq!(verdict(&m, s, &values, &p(true, None, false)), Verdict::NeedsSetting("Server address".into()));
        values.insert("server".to_string(), "https://s.example".to_string());
        values.insert("listen".to_string(), "127.0.0.1:9912".to_string());
        assert_eq!(verdict(&m, s, &values, &p(false, None, true)), Verdict::Elsewhere("127.0.0.1:9912".into()), "someone else's server: used, even without the program here");
        assert_eq!(verdict(&m, s, &values, &p(false, None, false)), Verdict::NotInstalled("srv".into()));
        assert_eq!(verdict(&m, s, &values, &p(true, Some("srv 0.5.0"), false)), Verdict::TooOld { found: "0.5.0".into(), need: "0.6.0".into() });
        assert_eq!(verdict(&m, s, &values, &p(true, Some("srv 0.6.1"), false)), Verdict::Start { version: Some("0.6.1".into()) });
        assert_eq!(verdict(&m, s, &values, &p(true, Some("garbled"), false)), Verdict::Start { version: None }, "a version it can't read doesn't stop it");
        values.insert("listen".to_string(), "0.0.0.0:9912".to_string());
        assert!(matches!(verdict(&m, s, &values, &p(true, None, false)), Verdict::BadAddress(_)));
    }
}
