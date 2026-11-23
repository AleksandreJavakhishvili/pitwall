//! Checks for an extension's folder, for its author and for Pitwall's own
//! tests (docs/extensions/checks.md): [`check`] reads the folder without
//! running anything (the manifest, its limits, the files it names, and
//! what it would run and set, in plain words); [`test`] runs what the
//! folder's `test/pitwall.json` expects. `pitwall extension check|test`
//! print these, offline, without the app.
//!
//! `test/pitwall.json` (each part optional):
//! - `actions`: `installed`, programs taken as installed (fakes on an
//!   empty PATH), and `expect`, per action id, its `label` and each step's
//!   exact `lines`, `skips` (why a step won't run, or null) and
//!   `optional`. Planned only: nothing runs.
//! - `env`: the variables shown `unset` (no settings), then `set` with
//!   `values` (plain settings) filled in.
//! - `run`: the adapter started with `settings` and `secrets` (literal
//!   values): it answers `hello`, says `status` (`ready` unless given),
//!   lists at least `threads` threads; `open` (a thread id) comes back
//!   with its messages; `reply` (text) is sent there and answered `ok`.
//! - `agreement`: a fake server (`fake`, a command run in the folder) that
//!   prints one JSON line (`thread`, and what `{field}` in `settings` and
//!   `secrets` stands for) and stops when its stdin closes; the adapter,
//!   given those, lists that thread with one blocking ask, sends its
//!   counts, opens it, and carries `answer` (`pick`, its `label`, `text`)
//!   back.
//! - `calls`: a fake Pitwall side (`grants`, `answers`, `events`) and what
//!   the adapter must call, subscribe to and show (`expect`).
//! - `views`: after `show` (a view id), a view of `kind` with at least
//!   `items` items; after `select` (an item), a `detail` of that kind
//!   beside the list; `action` (`item`, `action`) and `command` answered
//!   ok; at least `badges` badges and notes sent.
//!
//! An adapter's program that isn't installed (Node, say) skips the runs,
//! saying so; it doesn't fail them.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value};

use super::plan::{env_preview, line, plan, resolve};
use super::process::Process;
use super::protocol::{clean, negotiate, parse_line, read_line, CallError, ConnState, FromAdapter, FromInbox, Hello, LineGuard, ToAdapter, ToInbox, PROTOCOL_2};
use super::{read_in_place, Manifest, SettingKind, MANIFEST};

/// The fixture's file in an extension's folder.
pub const FIXTURE: &str = "test/pitwall.json";

/// How long an adapter (or a fake server) has for each answer.
pub const WAIT: Duration = Duration::from_secs(10);

/// How a finding went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    /// Works, but worth a look.
    Warn,
    Fail,
    /// Not checked here, and why (a program that isn't installed).
    Skip,
}

/// One thing found, in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub level: Level,
    pub text: String,
}

/// What [`check`] or [`test`] found.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub folder: PathBuf,
    /// The extension's id and name, when its manifest is usable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub findings: Vec<Finding>,
    /// What it would run and set ([`check`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub does: Vec<Does>,
}

/// One thing an extension would run or set: a heading and its lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Does {
    pub what: String,
    pub lines: Vec<String>,
}

impl Report {
    /// Nothing failed.
    pub fn passed(&self) -> bool {
        !self.findings.iter().any(|f| f.level == Level::Fail)
    }

    fn add(&mut self, level: Level, text: impl Into<String>) {
        self.findings.push(Finding { level, text: text.into() });
    }

    /// The report as text: each finding with its mark, then what it does.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let who = match (&self.name, &self.id) {
            (Some(n), Some(id)) => format!("{n} ({id})"),
            _ => self.folder.display().to_string(),
        };
        out.push_str(&format!("{who}\n"));
        for f in &self.findings {
            let mark = match f.level {
                Level::Ok => "ok  ",
                Level::Warn => "note",
                Level::Fail => "FAIL",
                Level::Skip => "skip",
            };
            out.push_str(&format!("  {mark}  {}\n", f.text));
        }
        for d in &self.does {
            out.push_str(&format!("\n{}\n", d.what));
            for l in &d.lines {
                out.push_str(&format!("  {l}\n"));
            }
        }
        let failed = self.findings.iter().filter(|f| f.level == Level::Fail).count();
        out.push_str(&match failed {
            0 => "\nPassed.\n".to_string(),
            1 => "\n1 check failed.\n".to_string(),
            n => format!("\n{n} checks failed.\n"),
        });
        out
    }
}

/// The folder's manifest, or the report saying why there is none.
fn manifest(folder: &Path, r: &mut Report) -> Option<Manifest> {
    r.folder = folder.to_path_buf();
    if !folder.is_dir() {
        r.add(Level::Fail, format!("{} isn't a folder", folder.display()));
        return None;
    }
    match read_in_place(folder) {
        Ok(m) => {
            r.id = Some(m.id.clone());
            r.name = Some(m.name.clone());
            Some(m)
        }
        Err(why) => {
            r.add(Level::Fail, format!("{MANIFEST} can't be used: {why}"));
            None
        }
    }
}

/// Arguments of an adapter's command that name a file in its folder: not
/// an option, and with a `/` or a file extension.
fn file_args(command: &[String]) -> Vec<&str> {
    command[1..]
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-') && !a.contains('{'))
        .filter(|a| a.contains('/') || Path::new(a).extension().is_some_and(|e| e.len() <= 4 && e.to_string_lossy().chars().all(|c| c.is_ascii_alphanumeric())))
        .collect()
}

/// Read `folder` without running anything: its manifest, the files it
/// names, and what it would run and set. `path`: where programs are looked
/// up (this process's PATH when `None`).
pub fn check(folder: &Path, path: Option<&str>) -> Report {
    let mut r = Report::default();
    let Some(m) = manifest(folder, &mut r) else { return r };
    r.add(Level::Ok, format!("{MANIFEST} is valid and within Pitwall's limits (format {})", m.format));
    if m.format < 2 {
        let raw: Value = std::fs::read_to_string(folder.join(MANIFEST)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for k in ["pitwall", "permissions", "contributes"] {
            if !raw[k].is_null() {
                r.add(Level::Warn, format!("it has \"{k}\", which format 1 ignores: add \"v\": 2 to use it"));
            }
        }
    } else {
        match &m.pitwall {
            Some(v) => r.add(Level::Ok, format!("it needs Pitwall {v} or later (this is {})", super::PITWALL)),
            None => r.add(Level::Warn, "it doesn't say which Pitwall it needs: add \"pitwall\": \">=0.9.0\" (a lower one would refuse it plainly)"),
        }
        for p in &m.permissions {
            if let Some(why) = super::permissions::unavailable_why(&p.name) {
                r.add(Level::Warn, format!("it asks for “{}”, {why}: never granted here", p.name));
            }
        }
        if m.adapter.is_some() && m.contributes == super::Contributes::default() && m.permissions.is_empty() {
            r.add(Level::Warn, "its adapter contributes nothing and asks for no permissions: add \"contributes\": {\"inbox\": true} (or views, commands, badges) or a permission");
        }
        let asks = |p: &str| m.permissions.iter().any(|x| x.name == p);
        if asks(super::permissions::NOTIFY) && !m.contributes.notifications {
            r.add(Level::Warn, "it asks for “notify” but doesn't contribute \"notifications\": add \"notifications\": true to \"contributes\"");
        }
        if m.contributes.badges && m.match_agents.is_none() && !asks(super::permissions::AGENTS_READ) && !asks(super::permissions::PROJECTS_READ) {
            r.add(Level::Warn, "its badges can name no row: ask for agents.read (agents by id) or projects.read (projects), or add \"matchAgents\" (agents by name)");
        }
    }
    let named = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if named != m.id {
        r.add(Level::Warn, format!("its folder is named “{named}”, not its id “{}”: fine to load in development, but a folder installed or shipped must be named after its id", m.id));
    }
    match &m.version {
        Some(v) => r.add(Level::Ok, format!("version {v}")),
        None => r.add(Level::Warn, "it has no \"version\": Pitwall can't tell a newer copy (add one like 1.0.0)"),
    }
    if let Some(s) = &m.source {
        let at = super::install::source_line(s);
        match &m.version {
            Some(v) => r.add(Level::Ok, format!("its updates come from {at}: this version's release is the tag {}", s.tags.replace(super::VERSION_PLACEHOLDER, v))),
            None => r.add(Level::Fail, format!("its updates come from {at}, but without a \"version\" no release tag can be told newer")),
        }
    }
    if let Some(a) = &m.adapter {
        let program = &a.command[0];
        if program.contains('/') || program.contains('\\') {
            match resolve(folder, program, path) {
                Ok(_) => r.add(Level::Ok, format!("its adapter's program {program} is in its folder")),
                Err(why) => r.add(Level::Fail, format!("its adapter's program: {why}")),
            }
        } else {
            match resolve(folder, program, path) {
                Ok(p) => r.add(Level::Ok, format!("its adapter runs {program}, found at {}", p.display())),
                Err(_) => r.add(Level::Warn, format!("its adapter runs “{program}”, which isn't on the PATH here: users need it installed")),
            }
        }
        for f in file_args(&a.command) {
            let p = folder.join(f);
            let inside = match (p.canonicalize(), folder.canonicalize()) {
                (Ok(full), Ok(base)) => full.starts_with(base),
                _ => false,
            };
            if !p.exists() {
                r.add(Level::Fail, format!("its adapter's command names {f}, which isn't in its folder"));
            } else if !inside {
                r.add(Level::Fail, format!("its adapter's command names {f}, which is outside its folder"));
            } else {
                r.add(Level::Ok, format!("{f} is there"));
            }
        }
    }
    if folder.join("README.md").is_file() {
        r.add(Level::Ok, "README.md is there");
    } else {
        r.add(Level::Warn, "there is no README.md: say what it does and how to set it up");
    }
    match std::fs::read_to_string(folder.join(FIXTURE)) {
        Ok(t) => match serde_json::from_str::<Value>(&t) {
            Ok(Value::Object(_)) => r.add(Level::Ok, format!("{FIXTURE} is there: `pitwall extension test` runs it")),
            Ok(_) => r.add(Level::Fail, format!("{FIXTURE} must be a JSON object")),
            Err(e) => r.add(Level::Fail, format!("{FIXTURE} isn't valid JSON: {e}")),
        },
        Err(_) => r.add(Level::Warn, format!("there is no {FIXTURE}: `pitwall extension test` has nothing to run")),
    }
    r.does = does(&m, path);
    r
}

/// What `m` would run and set, in plain words, as Settings shows it.
pub fn does(m: &Manifest, path: Option<&str>) -> Vec<Does> {
    let mut out = Vec::new();
    let mut push = |what: String, lines: Vec<String>| out.push(Does { what, lines });
    if !m.settings.is_empty() {
        let lines = m
            .settings
            .iter()
            .map(|s| {
                let kind = s.kind.words();
                let need = if s.required { "required" } else { "optional" };
                let mut more = String::new();
                if s.kind == SettingKind::Choice {
                    more += &format!(", one of {}", s.options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>().join(", "));
                }
                match (&s.min, &s.max) {
                    (Some(lo), Some(hi)) => more += &format!(", {lo} to {hi}"),
                    (Some(lo), None) => more += &format!(", at least {lo}"),
                    (None, Some(hi)) => more += &format!(", at most {hi}"),
                    (None, None) => {}
                }
                if let Some(d) = &s.default {
                    more += &format!(", {d} unless set");
                }
                format!("{} ({}): {kind}{more}, {need}", s.label, s.key)
            })
            .collect();
        push("The user sets (they reach the adapter only in its hello):".to_string(), lines);
    }
    if m.format >= 2 {
        let lines = m
            .permissions
            .iter()
            .map(|p| match super::permissions::unavailable_why(&p.name) {
                None => format!("{}: {}", p.name, p.reason),
                Some(why) => format!("{}: {} ({why}: never granted here)", p.name, p.reason),
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            push("It asks for no permissions: Pitwall answers none of its calls.".to_string(), vec![]);
        } else {
            push("It asks for (the user grants or refuses each when turning it on):".to_string(), lines);
        }
        let adds = m.contributes.lines();
        if !adds.is_empty() {
            push("It contributes:".to_string(), adds);
        }
    }
    match &m.adapter {
        Some(a) => push("While it is on, it runs this in its folder:".to_string(), vec![line(&a.command)]),
        None => push("It runs no program of its own.".to_string(), vec![]),
    }
    if !m.agent_env.is_empty() {
        push("It sets these on agents started while it is on ({project}: the project folder's name; {agent_id}: the agent's id):".to_string(), env_preview(m, &BTreeMap::new()));
    }
    if let Some(r) = &m.match_agents {
        push("A thread is an agent's when its names are:".to_string(), vec![format!("agent {}, session {}", r.agent, r.session)]);
    }
    for a in &m.actions {
        let lines = plan(m, a, &BTreeMap::new(), path)
            .iter()
            .map(|p| match &p.skip {
                Some(why) => format!("{}   (skipped here: {why})", p.line()),
                None => p.line(),
            })
            .collect();
        let kind = if a.setup { "a setup step" } else { "a button" };
        push(format!("“{}” ({kind}) runs, only when pressed and confirmed:", a.label), lines);
    }
    if let Some(s) = &m.source {
        push("Its updates come from (a newer release tag, looked at and installed only when the user presses Update):".to_string(), vec![super::install::source_line(s)]);
    }
    out
}

/// Run what `folder`'s `test/pitwall.json` expects. `path`: where
/// programs are looked up (this process's PATH when `None`).
pub fn test(folder: &Path, path: Option<&str>) -> Report {
    let mut r = Report::default();
    let Some(m) = manifest(folder, &mut r) else { return r };
    let fixture = match std::fs::read_to_string(folder.join(FIXTURE)) {
        Err(_) => {
            r.add(Level::Warn, format!("there is no {FIXTURE}: nothing to run (`pitwall extension new` writes a starting one)"));
            return r;
        }
        Ok(t) => match serde_json::from_str::<Value>(&t) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) => {
                r.add(Level::Fail, format!("{FIXTURE} must be a JSON object"));
                return r;
            }
            Err(e) => {
                r.add(Level::Fail, format!("{FIXTURE} isn't valid JSON: {e}"));
                return r;
            }
        },
    };
    let before = r.findings.len();
    actions(&m, &fixture["actions"], &mut r);
    env(&m, &fixture["env"], &mut r);
    if !fixture["run"].is_null() {
        run(folder, &m, &fixture["run"], path, &mut r);
    }
    if !fixture["agreement"].is_null() {
        agreement(folder, &m, &fixture["agreement"], path, &mut r);
    }
    if !fixture["calls"].is_null() {
        calls(folder, &m, &fixture["calls"], path, &mut r);
    }
    if !fixture["views"].is_null() {
        views(folder, &m, &fixture["views"], path, &mut r);
    }
    if r.findings.len() == before {
        r.add(Level::Warn, format!("{FIXTURE} expects nothing (actions, env, run, agreement, calls or views)"));
    }
    r
}

/// `views`: what its view tiles, commands and badges send, against a fake
/// Pitwall side (as `calls`): after `show`, a view of the `kind` expected
/// with at least `items` items; after `select`, the view with a `detail`
/// of that kind beside the list; `action` (`item`, `action`) and
/// `command` answered ok; at least `badges` badges sent.
fn views(folder: &Path, m: &Manifest, v: &Value, path: Option<&str>, r: &mut Report) {
    use super::view::{Pane, Root};
    let Some(command) = command_of(m, folder, path, "views", r) else { return };
    let Some(show) = v["show"].as_str().map(str::to_string) else {
        r.add(Level::Fail, format!("{FIXTURE}'s “views” needs “show”: the id of the view to show"));
        return;
    };
    if m.contributes.view(&show).is_none() {
        r.add(Level::Fail, format!("views: its manifest declares no view “{show}” (contributes.views)"));
        return;
    }
    let obj = |k: &str| v[k].as_object().cloned().unwrap_or_default();
    let grants: Vec<String> = v["grants"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let mut d = match Driver::start(folder, &command, &hello_with(obj("settings"), obj("secrets"), grants), path) {
        Ok(d) => d,
        Err(why) => {
            r.add(Level::Fail, format!("views: its adapter: {why}"));
            return;
        }
    };
    d.answers = obj("answers");
    let kind_of = |root: &Root| match root {
        Root::List { .. } => "list",
        Root::Pane(Pane::Detail(_)) => "detail",
        Root::Pane(Pane::Markdown(_)) => "markdown",
        Root::Pane(Pane::Status(_)) => "status",
    };
    let pane_kind = |p: &Pane| match p {
        Pane::Detail(_) => "detail",
        Pane::Markdown(_) => "markdown",
        Pane::Status(_) => "status",
    };
    let result = (|| -> Result<(), String> {
        if said_hello(&mut d, r).is_none() {
            return Ok(());
        }
        d.send(&ToAdapter::Show { view: show.clone() })?;
        let want_kind = v["kind"].as_str().unwrap_or_default().to_string();
        let at_least = v["items"].as_u64().unwrap_or(0) as usize;
        let FromAdapter::View(view) = d.until(&format!("view “{show}”"), |m| matches!(m, FromAdapter::View(x) if x.id == show))? else { unreachable!() };
        let root = view.root.map_err(|why| format!("view “{show}” is refused: {why}"))?;
        let kind = kind_of(&root);
        if !want_kind.is_empty() && kind != want_kind {
            return Err(format!("view “{show}” is a {kind}, not a {want_kind}"));
        }
        let items = match &root {
            Root::List { list, .. } => list.items().count(),
            _ => 0,
        };
        if items < at_least {
            return Err(format!("view “{show}” has {items} item(s), fewer than {at_least}"));
        }
        r.add(Level::Ok, format!("views: after show, view “{show}” is a {kind}{}", if kind == "list" { format!(" of {items} item(s)") } else { String::new() }));
        if let Some(item) = v["select"].as_str().map(str::to_string) {
            let want = v["detail"].as_str().unwrap_or("detail").to_string();
            d.send(&ToAdapter::Select { view: show.clone(), item: item.clone() })?;
            d.until(&format!("view “{show}” with a {want} for “{item}”"), |m| matches!(m, FromAdapter::View(x) if x.id == show && matches!(&x.root, Ok(Root::List { detail: Some(p), .. }) if pane_kind(p) == want)))?;
            r.add(Level::Ok, format!("views: after selecting “{item}”, a {want} is beside the list"));
        }
        let mut req = 100;
        if let Some(a) = v["action"].as_object() {
            req += 1;
            let action = a.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
            let item = a.get("item").and_then(Value::as_str).map(str::to_string);
            d.send(&ToAdapter::Action { req, view: show.clone(), item: item.clone(), action: action.clone() })?;
            match d.until(&format!("the result of action “{action}”"), |m| matches!(m, FromAdapter::Result { req: x, .. } if *x == req))? {
                FromAdapter::Result { ok: true, .. } => r.add(Level::Ok, format!("views: action “{action}”{} is answered ok", item.map(|i| format!(" on “{i}”")).unwrap_or_default())),
                FromAdapter::Result { message, .. } => r.add(Level::Fail, format!("views: action “{action}” is refused: {message}")),
                _ => {}
            }
        }
        if let Some(c) = v["command"].as_str().map(str::to_string) {
            if m.contributes.command(&c).is_none() {
                return Err(format!("its manifest declares no command “{c}” (contributes.commands)"));
            }
            req += 1;
            d.send(&ToAdapter::Command { req, command: c.clone(), agent_id: None })?;
            match d.until(&format!("the result of command “{c}”"), |m| matches!(m, FromAdapter::Result { req: x, .. } if *x == req))? {
                FromAdapter::Result { ok: true, .. } => r.add(Level::Ok, format!("views: command “{c}” is answered ok")),
                FromAdapter::Result { message, .. } => r.add(Level::Fail, format!("views: command “{c}” is refused: {message}")),
                _ => {}
            }
        }
        if let Some(n) = v["badges"].as_u64() {
            let n = n as usize;
            if !d.badges.is_some_and(|b| b >= n) {
                d.until(&format!("at least {n} badge(s)"), |m| matches!(m, FromAdapter::Badges(b) if b.len() >= n))?;
            }
            r.add(Level::Ok, format!("views: it sends {} badge(s) or note(s)", d.badges.unwrap_or(n)));
        }
        Ok(())
    })();
    if let Err(why) = result {
        r.add(Level::Fail, format!("views: {why}"));
    }
    finish(d, r);
}

/// `calls`: the adapter against a fake Pitwall side that answers its
/// calls from the fixture (protocol 2): it answers hello with protocol 2,
/// makes the calls expected, subscribes to the topics expected, lists its
/// threads and shows what it read in the thread expected; then, after the
/// fixture's `events`, what they changed.
fn calls(folder: &Path, m: &Manifest, v: &Value, path: Option<&str>, r: &mut Report) {
    let Some(command) = command_of(m, folder, path, "calls", r) else { return };
    let obj = |k: &str| v[k].as_object().cloned().unwrap_or_default();
    let grants: Vec<String> = v["grants"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    for g in &grants {
        if !m.permissions.iter().any(|p| p.name == *g) {
            r.add(Level::Fail, format!("calls: it is granted “{g}”, which its manifest doesn't ask for (Pitwall never grants that)"));
        }
    }
    let mut d = match Driver::start(folder, &command, &hello_with(obj("settings"), obj("secrets"), grants), path) {
        Ok(d) => d,
        Err(why) => {
            r.add(Level::Fail, format!("calls: its adapter: {why}"));
            return;
        }
    };
    d.answers = obj("answers");
    if let Err(why) = exchange(&mut d, &v["expect"], &v["events"], r) {
        r.add(Level::Fail, format!("calls: {why}"));
    }
    finish(d, r);
}

/// The calls' exchange, step by step.
fn exchange(d: &mut Driver, expect: &Value, events: &Value, r: &mut Report) -> Result<(), String> {
    match said_hello(d, r) {
        None => return Ok(()),
        Some(p) if p < PROTOCOL_2 => return Err("it answers hello with protocol 1: calls need protocol 2".into()),
        Some(_) => {}
    }
    let strs = |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() };
    let at_least = expect["threads"].as_u64().unwrap_or(0) as usize;
    let list = d.until("its threads", |m| matches!(m, FromAdapter::Inbox(FromInbox::Threads(l)) if l.len() >= at_least.max(1)));
    // Its calls come before its list, or soon after.
    let want = strs(&expect["called"]);
    let end = Instant::now() + WAIT;
    while !want.iter().all(|w| d.called.contains(w)) && Instant::now() < end {
        let _ = d.until("its calls", |_| false);
    }
    for w in &want {
        if d.called.contains(w) {
            r.add(Level::Ok, format!("calls: it calls {w}"));
        } else {
            r.add(Level::Fail, format!("calls: it never calls {w} (it called {})", if d.called.is_empty() { "nothing".to_string() } else { d.called.join(", ") }));
        }
    }
    for t in strs(&expect["subscribes"]) {
        if d.topics.contains(&t) {
            r.add(Level::Ok, format!("calls: it subscribes to {t}"));
        } else {
            r.add(Level::Fail, format!("calls: it never subscribes to {t}"));
        }
    }
    match list {
        Ok(FromAdapter::Inbox(FromInbox::Threads(l))) => r.add(Level::Ok, format!("calls: it lists {} thread(s)", l.len())),
        Ok(_) => {}
        Err(why) if at_least > 0 => return Err(why),
        Err(_) => {}
    }
    let Some(id) = expect["open"].as_str().map(str::to_string) else { return Ok(()) };
    let text = expect["text"].as_str().unwrap_or_default().to_string();
    d.send(&ToAdapter::Inbox(ToInbox::Open { thread: id.clone() }))?;
    let has = |t: &super::protocol::ThreadDetail, s: &str| t.messages.iter().any(|m| m.text.contains(s) || m.detail.contains(s));
    let FromAdapter::Inbox(FromInbox::Thread(t)) = d.until(&format!("thread “{id}”"), |m| matches!(m, FromAdapter::Inbox(FromInbox::Thread(t)) if t.id == id))? else { unreachable!() };
    if !has(&t, &text) {
        return Err(format!("thread “{id}” doesn't show “{text}”, what the fake Pitwall answered"));
    }
    r.add(Level::Ok, format!("calls: thread “{id}” shows what it read (“{text}”)"));
    let after = expect["afterEvents"].as_str().unwrap_or_default().to_string();
    let list = events.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return Ok(());
    }
    for e in &list {
        d.send(&ToAdapter::Event { event: e["event"].as_str().unwrap_or_default().to_string(), data: e["data"].clone() })?;
    }
    if after.is_empty() {
        return Ok(());
    }
    d.until(&format!("thread “{id}” showing “{after}” after the events"), |m| matches!(m, FromAdapter::Inbox(FromInbox::Thread(t)) if t.id == id && has(t, &after)))?;
    r.add(Level::Ok, format!("calls: after {} event(s), thread “{id}” shows “{after}”", list.len()));
    Ok(())
}

fn strings(v: &Value) -> Option<Vec<Option<String>>> {
    Some(v.as_array()?.iter().map(|s| s.as_str().map(str::to_string)).collect())
}

/// Each action plans exactly the lines expected, with the programs named
/// `installed` as fakes on an empty PATH.
fn actions(m: &Manifest, v: &Value, r: &mut Report) {
    if v.is_null() {
        return;
    }
    let Ok(t) = scratch("actions") else {
        r.add(Level::Fail, "a temporary folder couldn't be made");
        return;
    };
    for program in v["installed"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let _ = std::fs::write(t.0.join(program), "");
        if std::env::consts::OS == "windows" {
            let _ = std::fs::write(t.0.join(format!("{program}.exe")), "");
        }
    }
    let path = t.0.to_string_lossy().into_owned();
    let expect = v["expect"].as_object().cloned().unwrap_or_default();
    for id in expect.keys() {
        if !m.actions.iter().any(|a| &a.id == id) {
            r.add(Level::Fail, format!("action “{id}” is expected but the manifest has none by that id"));
        }
    }
    for a in &m.actions {
        let p = plan(m, a, &BTreeMap::new(), Some(&path));
        let Some(want) = expect.get(&a.id) else { continue };
        let mut wrong = Vec::new();
        if want["label"].as_str().is_some_and(|l| l != a.label) {
            wrong.push(format!("its label is “{}”, not “{}”", a.label, want["label"].as_str().unwrap_or("")));
        }
        let lines: Vec<Option<String>> = p.iter().map(|s| Some(s.line())).collect();
        if let Some(w) = strings(&want["lines"]).filter(|w| *w != lines) {
            wrong.push(format!("it runs\n      {}\n    not\n      {}", lines.iter().flatten().cloned().collect::<Vec<_>>().join("\n      "), w.iter().flatten().cloned().collect::<Vec<_>>().join("\n      ")));
        }
        let skips: Vec<Option<String>> = p.iter().map(|s| s.skip.clone()).collect();
        if let Some(w) = strings(&want["skips"]).filter(|w| *w != skips) {
            wrong.push(format!("its skips are {skips:?}, not {w:?}"));
        }
        let optional: Vec<bool> = p.iter().map(|s| s.optional).collect();
        if let Some(w) = want["optional"].as_array().map(|a| a.iter().map(|b| b.as_bool().unwrap_or(false)).collect::<Vec<_>>()).filter(|w| *w != optional) {
            wrong.push(format!("its optional steps are {optional:?}, not {w:?}"));
        }
        if wrong.is_empty() {
            r.add(Level::Ok, format!("action “{}” plans the {} step(s) expected", a.id, p.len()));
        } else {
            r.add(Level::Fail, format!("action “{}”: {}", a.id, wrong.join("; ")));
        }
    }
}

/// The variables shown before and after the settings are set.
fn env(m: &Manifest, v: &Value, r: &mut Report) {
    if v.is_null() {
        return;
    }
    let lines = |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() };
    let mut values = BTreeMap::new();
    let before = env_preview(m, &values);
    if before == lines(&v["unset"]) {
        r.add(Level::Ok, "the variables shown before its settings are set are the ones expected");
    } else {
        r.add(Level::Fail, format!("before its settings are set, the variables are {before:?}, not {:?}", lines(&v["unset"])));
    }
    for (k, val) in v["values"].as_object().cloned().unwrap_or_default() {
        values.insert(k, val.as_str().unwrap_or_default().to_string());
    }
    let after = env_preview(m, &values);
    if after == lines(&v["set"]) {
        r.add(Level::Ok, "the variables shown with its settings set are the ones expected");
    } else {
        r.add(Level::Fail, format!("with its settings set, the variables are {after:?}, not {:?}", lines(&v["set"])));
    }
}

/// A temporary folder of its own, removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(what: &str) -> std::io::Result<Scratch> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pitwall-check-{what}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(Scratch(dir))
}

/// A running adapter, driven line by line.
struct Driver {
    proc: Process,
    stdin: Option<Box<dyn Write + Send>>,
    rx: mpsc::Receiver<Result<FromAdapter, String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// Lines it sent that Pitwall can't read.
    bad: Vec<String>,
    /// The protocol it answered hello with.
    protocol: u64,
    /// The fake Pitwall side of `calls`: what each method answers.
    answers: Map<String, Value>,
    /// The methods it called, in order.
    called: Vec<String>,
    /// The topics it subscribed to.
    topics: Vec<String>,
    /// How many badges and notes its last `badges` line had.
    badges: Option<usize>,
}

/// What the fake Pitwall side answers call `c` with: the fixture's
/// `answers` (an object with `error` is an error), `events.subscribe` the
/// topics asked for, anything else "not in the fixture".
fn fake_answer(answers: &Map<String, Value>, method: &str, params: &Value) -> Result<Value, CallError> {
    match answers.get(method) {
        Some(Value::Object(o)) if o.contains_key("error") => {
            let e = &o["error"];
            Err(CallError::new(e["code"].as_str().unwrap_or("denied"), e["message"].as_str().unwrap_or("Not allowed")))
        }
        Some(v) => Ok(v.clone()),
        None if method == "events.subscribe" => Ok(serde_json::json!({"topics": params["topics"].clone()})),
        None => Err(CallError::new("unknown_method", format!("{method} isn't in {FIXTURE}'s calls.answers"))),
    }
}

impl Driver {
    fn start(folder: &Path, command: &[String], hello: &ToAdapter, path: Option<&str>) -> Result<Driver, String> {
        let program = resolve(folder, &command[0], path)?;
        let (proc, pipes) = super::process::start(&program, &command[1..], folder, path).map_err(|e| format!("it couldn't start: {e}"))?;
        let (tx, rx) = mpsc::channel();
        let stdout = pipes.stdout;
        std::thread::spawn(move || {
            let mut out = BufReader::new(stdout);
            let mut buf = Vec::new();
            let mut guard = LineGuard::default();
            let start = Instant::now();
            loop {
                match read_line(&mut out, &mut buf) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        let _ = tx.send(Err(format!("it sent {e}")));
                        return;
                    }
                }
                if let Err(e) = guard.line(start.elapsed().as_millis() as u64) {
                    let _ = tx.send(Err(e));
                    return;
                }
                let text = String::from_utf8_lossy(&buf).into_owned();
                let msg = match parse_line(&text) {
                    Ok(FromAdapter::Unknown) => continue,
                    Ok(m) => Ok(m),
                    Err(why) => Err(format!("{why}: {}", clean(&text, 120, false))),
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        });
        let stderr: Arc<Mutex<Vec<u8>>> = Arc::default();
        let tail = stderr.clone();
        let mut err = pipes.stderr;
        std::thread::spawn(move || {
            let mut b = [0u8; 1024];
            while let Ok(n) = err.read(&mut b) {
                if n == 0 {
                    return;
                }
                let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                t.extend_from_slice(&b[..n]);
                let over = t.len().saturating_sub(4096);
                t.drain(..over);
            }
        });
        let mut d = Driver { proc, stdin: Some(pipes.stdin), rx, stderr, bad: Vec::new(), protocol: 0, answers: Map::new(), called: Vec::new(), topics: Vec::new(), badges: None };
        d.send(hello)?;
        Ok(d)
    }

    /// Answer call `c` as the fixture says (a call on protocol 1 is
    /// refused, as Pitwall refuses it).
    fn answer(&mut self, id: String, method: String, params: Value) -> Result<(), String> {
        let result = if self.protocol < PROTOCOL_2 {
            Err(CallError::new("unsupported", "calls need protocol 2: answer hello with \"protocol\": 2"))
        } else {
            if method == "events.subscribe" {
                self.topics.extend(params["topics"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
            }
            fake_answer(&self.answers, &method, &params)
        };
        self.called.push(method);
        self.send(&ToAdapter::Reply { id, result })
    }

    fn send(&mut self, m: &ToAdapter) -> Result<(), String> {
        let w = self.stdin.as_mut().ok_or("its stdin is closed")?;
        w.write_all(m.line().as_bytes()).and_then(|_| w.write_all(b"\n")).and_then(|_| w.flush()).map_err(|e| format!("it can't be written to: {e}{}", self.said()))
    }

    /// What it printed on stderr, as a note to add to a failure.
    fn said(&self) -> String {
        let t = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        let t = clean(String::from_utf8_lossy(&t).trim(), 600, true);
        if t.is_empty() {
            String::new()
        } else {
            format!(". It said on stderr: {t}")
        }
    }

    /// The next message `want` accepts (others skipped), within [`WAIT`].
    fn until(&mut self, what: &str, want: impl Fn(&FromAdapter) -> bool) -> Result<FromAdapter, String> {
        let end = Instant::now() + WAIT;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let got = self.rx.recv_timeout(left);
            if let Ok(Ok(FromAdapter::Badges(b))) = &got {
                self.badges = Some(b.len());
            }
            match got {
                Ok(Ok(m)) if want(&m) => return Ok(m),
                // Calls are answered as they come, whatever is awaited.
                Ok(Ok(FromAdapter::Call(c))) => self.answer(c.id, c.method, c.params)?,
                Ok(Ok(FromAdapter::BadCall { id, why })) => {
                    self.bad.push(format!("a call Pitwall can't make: {why}"));
                    self.send(&ToAdapter::Reply { id, result: Err(CallError::new("bad_params", why)) })?;
                }
                Ok(Ok(_)) => {}
                Ok(Err(why)) if why.starts_with("it sent") || why.contains("lines in a second") => return Err(format!("waiting for {what}, it was stopped: {why}")),
                Ok(Err(why)) => self.bad.push(why),
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(format!("it didn't send {what} within {} seconds{}", WAIT.as_secs(), self.said())),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(Duration::from_millis(200));
                    let how = match self.proc.exited() {
                        Some(Some(c)) => format!("it exited with code {c}"),
                        _ => "it stopped".to_string(),
                    };
                    return Err(format!("waiting for {what}, {how}{}", self.said()));
                }
            }
        }
    }

    /// Close its stdin and give it 2 s to exit, as Pitwall does: whether
    /// it did. Then it, and anything it started, is stopped.
    fn stop(&mut self) -> bool {
        self.stdin = None;
        let end = Instant::now() + Duration::from_secs(2);
        let mut exited = false;
        while Instant::now() < end {
            if self.proc.exited().is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.proc.stop();
        exited
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.proc.stop();
    }
}

/// `settings` and `secrets` as `hello` carries them.
fn hello(settings: Map<String, Value>, secrets: Map<String, Value>) -> ToAdapter {
    hello_with(settings, secrets, vec![])
}

/// [`hello`] with the permissions granted (`calls`).
fn hello_with(settings: Map<String, Value>, secrets: Map<String, Value>, grants: Vec<String>) -> ToAdapter {
    ToAdapter::Hello(Hello { pitwall: super::PITWALL.to_string(), settings, secrets, connection: None, grants })
}

/// The adapter's command, or the report says why there is none to run.
fn command_of(m: &Manifest, folder: &Path, path: Option<&str>, what: &str, r: &mut Report) -> Option<Vec<String>> {
    let Some(a) = &m.adapter else {
        r.add(Level::Fail, format!("{FIXTURE} has “{what}”, but the manifest has no adapter"));
        return None;
    };
    if let Err(why) = resolve(folder, &a.command[0], path) {
        r.add(Level::Skip, format!("{what}: {why}"));
        return None;
    }
    Some(a.command.clone())
}

/// After its `hello`: what it said, as findings; the protocol it speaks,
/// when Pitwall speaks it.
fn said_hello(d: &mut Driver, r: &mut Report) -> Option<u64> {
    match d.until("its hello", |m| matches!(m, FromAdapter::Hello { .. })) {
        Ok(FromAdapter::Hello { protocol, uses }) => match negotiate(protocol) {
            Ok(p) => {
                let uses = if uses.is_empty() { String::new() } else { format!(", using {}", uses.join(", ")) };
                r.add(Level::Ok, format!("it answers hello with protocol {p}{uses}"));
                d.protocol = p;
                Some(p)
            }
            Err(why) => {
                r.add(Level::Fail, format!("it answers hello with protocol {protocol}: {why}"));
                None
            }
        },
        Ok(_) => None,
        Err(why) => {
            r.add(Level::Fail, why);
            None
        }
    }
}

/// The end of a run: lines it couldn't read, and whether it exits when its
/// stdin closes.
fn finish(mut d: Driver, r: &mut Report) {
    for b in d.bad.iter().take(3) {
        r.add(Level::Warn, format!("it sent a line Pitwall ignores ({b})"));
    }
    if d.stop() {
        r.add(Level::Ok, "it exits when its stdin closes");
    } else {
        r.add(Level::Fail, "it didn't exit within 2 seconds of its stdin closing (Pitwall kills it then)");
    }
}

/// `run`: the adapter on its own, with literal settings and secrets.
fn run(folder: &Path, m: &Manifest, v: &Value, path: Option<&str>, r: &mut Report) {
    let Some(command) = command_of(m, folder, path, "run", r) else { return };
    let obj = |k: &str| v[k].as_object().cloned().unwrap_or_default();
    let mut d = match Driver::start(folder, &command, &hello(obj("settings"), obj("secrets")), path) {
        Ok(d) => d,
        Err(why) => {
            r.add(Level::Fail, format!("its adapter: {why}"));
            return;
        }
    };
    if said_hello(&mut d, r).is_none() {
        finish(d, r);
        return;
    }
    let state = v["status"].as_str().unwrap_or("ready").to_string();
    match d.until(&format!("status “{state}”"), |m| matches!(m, FromAdapter::Status { state: s, .. } if serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_string)).as_deref() == Some(state.as_str()))) {
        Ok(_) => r.add(Level::Ok, format!("it says status “{state}”")),
        Err(why) => {
            r.add(Level::Fail, why);
            finish(d, r);
            return;
        }
    }
    let at_least = v["threads"].as_u64().unwrap_or(0) as usize;
    match d.until("its threads", |m| matches!(m, FromAdapter::Inbox(FromInbox::Threads(_)))) {
        Ok(FromAdapter::Inbox(FromInbox::Threads(list))) if list.len() >= at_least => r.add(Level::Ok, format!("it lists {} thread(s)", list.len())),
        Ok(FromAdapter::Inbox(FromInbox::Threads(list))) => r.add(Level::Fail, format!("it lists {} thread(s), fewer than {at_least}", list.len())),
        Ok(_) => {}
        Err(why) => {
            r.add(Level::Fail, why);
            finish(d, r);
            return;
        }
    }
    if let Some(id) = v["open"].as_str() {
        let id = id.to_string();
        let opened = d.send(&ToAdapter::Inbox(ToInbox::Open { thread: id.clone() })).and_then(|_| d.until(&format!("thread “{id}”"), |m| matches!(m, FromAdapter::Inbox(FromInbox::Thread(t)) if t.id == id)));
        match opened {
            Ok(FromAdapter::Inbox(FromInbox::Thread(t))) if !t.messages.is_empty() => r.add(Level::Ok, format!("opened, thread “{id}” comes with {} message(s)", t.messages.len())),
            Ok(_) => r.add(Level::Fail, format!("opened, thread “{id}” comes with no messages")),
            Err(why) => r.add(Level::Fail, why),
        }
        if let Some(text) = v["reply"].as_str() {
            let sent = d.send(&ToAdapter::Inbox(ToInbox::Message { req: 1, thread: id.clone(), text: text.to_string() })).and_then(|_| d.until("the result of a reply", |m| matches!(m, FromAdapter::Result { req: 1, .. })));
            match sent {
                Ok(FromAdapter::Result { ok: true, .. }) => r.add(Level::Ok, "a reply is answered ok"),
                Ok(FromAdapter::Result { message, .. }) => r.add(Level::Fail, format!("a reply is refused: {message}")),
                Ok(_) => {}
                Err(why) => r.add(Level::Fail, why),
            }
        }
    }
    finish(d, r);
}

/// A fake server started for the agreement, stopped when dropped.
struct Fake(Process, Option<Box<dyn Write + Send>>);

impl Drop for Fake {
    fn drop(&mut self) {
        self.1 = None;
        let end = Instant::now() + Duration::from_secs(2);
        while self.0.exited().is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(25));
        }
        self.0.stop();
    }
}

/// Start the fake server: it and its first line.
fn fake(folder: &Path, cmd: &[String], path: Option<&str>) -> Result<(Fake, Value), String> {
    let program = resolve(folder, &cmd[0], path)?;
    let (proc, pipes) = super::process::start(&program, &cmd[1..], folder, path).map_err(|e| format!("the fake server couldn't start: {e}"))?;
    let fake = Fake(proc, Some(pipes.stdin));
    let (tx, rx) = mpsc::channel();
    let out = pipes.stdout;
    std::thread::spawn(move || {
        let mut first = String::new();
        let mut reader = BufReader::new(out);
        let _ = reader.read_line(&mut first);
        let _ = tx.send(first);
        // Keep reading so it never blocks on a full pipe.
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });
    let first = rx.recv_timeout(WAIT).map_err(|_| format!("the fake server didn't print its line within {} seconds", WAIT.as_secs()))?;
    let v: Value = serde_json::from_str(first.trim()).map_err(|e| format!("the fake server's first line isn't JSON: {e}"))?;
    Ok((fake, v))
}

/// `agreement`: the real adapter against the folder's fake server.
fn agreement(folder: &Path, m: &Manifest, v: &Value, path: Option<&str>, r: &mut Report) {
    let Some(command) = command_of(m, folder, path, "agreement", r) else { return };
    let cmd: Vec<String> = v["fake"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    if cmd.is_empty() {
        r.add(Level::Fail, "agreement: \"fake\" must be the fake server's command, a list of strings");
        return;
    }
    if let Err(why) = resolve(folder, &cmd[0], path) {
        r.add(Level::Skip, format!("agreement: the fake server: {why}"));
        return;
    }
    let (server, fake) = match fake(folder, &cmd, path) {
        Ok(f) => f,
        Err(why) => {
            r.add(Level::Fail, format!("agreement: {why}"));
            return;
        }
    };
    let fill = |v: &Value| -> Map<String, Value> {
        let mut out = Map::new();
        for (k, val) in v.as_object().cloned().unwrap_or_default() {
            let field = val.as_str().unwrap_or_default().trim_start_matches('{').trim_end_matches('}');
            out.insert(k, fake[field].clone());
        }
        out
    };
    let Some(thread) = fake["thread"].as_str().map(str::to_string) else {
        r.add(Level::Fail, "agreement: the fake server's line has no \"thread\"");
        return;
    };
    let mut d = match Driver::start(folder, &command, &hello(fill(&v["settings"]), fill(&v["secrets"])), path) {
        Ok(d) => d,
        Err(why) => {
            r.add(Level::Fail, format!("agreement: its adapter: {why}"));
            return;
        }
    };
    let res = agree(&mut d, &thread, &v["answer"], r);
    if let Err(why) = res {
        r.add(Level::Fail, format!("agreement: {why}"));
    }
    finish(d, r);
    drop(server);
}

/// The agreement's exchange, step by step.
fn agree(d: &mut Driver, thread: &str, answer: &Value, r: &mut Report) -> Result<(), String> {
    if said_hello(d, r).is_none() {
        return Ok(());
    }
    d.until("status “ready”", |m| matches!(m, FromAdapter::Status { state: ConnState::Ready, .. }))?;
    r.add(Level::Ok, "agreement: it connects to the fake server and says ready");
    let FromAdapter::Inbox(FromInbox::Threads(list)) = d.until("its threads", |m| matches!(m, FromAdapter::Inbox(FromInbox::Threads(_))))? else { unreachable!() };
    let t = list.iter().find(|t| t.id == thread).ok_or(format!("its list doesn't have the fake's thread “{thread}”"))?;
    if !(t.blocking && t.open_asks == 1 && t.unread >= 1 && t.from.is_some()) {
        return Err(format!("the fake's thread should be unread, from an agent, with one blocking ask; it is {t:?}"));
    }
    r.add(Level::Ok, "agreement: it lists the fake's thread, unread, with its blocking ask");
    let FromAdapter::Inbox(FromInbox::Counts(c)) = d.until("its counts", |m| matches!(m, FromAdapter::Inbox(FromInbox::Counts(_))))? else { unreachable!() };
    if (c.open_asks, c.blocking) != (1, 1) {
        return Err(format!("its counts should be one open ask, one blocking; they are {c:?}"));
    }
    d.send(&ToAdapter::Inbox(ToInbox::Open { thread: thread.to_string() }))?;
    let FromAdapter::Inbox(FromInbox::Thread(t)) = d.until("the thread", |m| matches!(m, FromAdapter::Inbox(FromInbox::Thread(_))))? else { unreachable!() };
    let ask = t.messages.first().and_then(|m| m.asks.first()).ok_or("the thread's first message has no ask")?;
    let pick = answer["pick"].as_u64().unwrap_or(0) as u32;
    let (label, text) = (answer["label"].as_str().unwrap_or_default(), answer["text"].as_str().unwrap_or_default());
    if !(ask.open && ask.blocking && ask.suggestions.get(pick as usize).is_some_and(|s| s.label == label)) {
        return Err(format!("its ask should be open and blocking, suggestion {pick} “{label}”; it is {ask:?}"));
    }
    r.add(Level::Ok, "agreement: the thread opens with its ask and suggestions");
    let number = ask.number;
    d.send(&ToAdapter::Inbox(ToInbox::Answer { req: 3, thread: thread.to_string(), ask: number, picks: vec![pick], text: Some(text.into()), declined: false }))?;
    match d.until("the answer's result", |m| matches!(m, FromAdapter::Result { req: 3, .. }))? {
        FromAdapter::Result { ok: true, .. } => {}
        FromAdapter::Result { message, .. } => return Err(format!("the answer was refused: {message}")),
        _ => {}
    }
    let FromAdapter::Inbox(FromInbox::Thread(t)) = d.until("the answered thread", |m| matches!(m, FromAdapter::Inbox(FromInbox::Thread(t)) if t.messages.len() > 1))? else { unreachable!() };
    let answered = t.messages.iter().flat_map(|m| &m.answers).next().ok_or("the answered thread carries no answer")?;
    if answered.labels != [label.to_string()] || answered.text != text {
        return Err(format!("the answer comes back as {:?} “{}”, not [“{label}”] “{text}”", answered.labels, answered.text));
    }
    r.add(Level::Ok, "agreement: an answer goes through and comes back in the thread");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(t: &crate::testing::TempDir, name: &str, manifest: &str) -> PathBuf {
        let d = t.path().join(name);
        std::fs::create_dir_all(d.join("test")).unwrap();
        std::fs::write(d.join(MANIFEST), manifest).unwrap();
        d
    }

    #[test]
    fn check_says_what_is_wrong_and_what_it_does() {
        let t = crate::testing::TempDir::new("check");
        let r = check(&t.path().join("nope"), None);
        assert!(!r.passed() && r.findings[0].text.contains("isn't a folder"));
        let d = folder(&t, "broken", "{");
        let r = check(&d, None);
        assert!(!r.passed() && r.findings[0].text.contains("isn't valid JSON"), "{r:?}");
        let d = folder(&t, "mine", r#"{"id": "other", "name": "Other", "adapter": {"command": ["node", "adapter/main.mjs", "--flag"]},
            "agentEnv": [{"name": "OTHER_AGENT", "value": "{project}"}],
            "actions": [{"id": "setup", "label": "Set up", "description": "d", "setup": true, "steps": [{"run": ["no-such-tool-here", "add"]}]}]}"#);
        let r = check(&d, Some(""));
        let text = r.text();
        assert!(!r.passed(), "{text}");
        assert!(text.contains("named “mine”, not its id “other”"), "{text}");
        assert!(text.contains("no \"version\""), "{text}");
        assert!(text.contains("isn't on the PATH here"), "{text}");
        assert!(text.contains("FAIL  its adapter's command names adapter/main.mjs, which isn't in its folder"), "{text}");
        assert!(text.contains("OTHER_AGENT={project}"), "{text}");
        assert!(text.contains("no-such-tool-here add   (skipped here: “no-such-tool-here” isn't installed)"), "{text}");
        std::fs::create_dir_all(d.join("adapter")).unwrap();
        std::fs::write(d.join("adapter/main.mjs"), "").unwrap();
        std::fs::write(d.join("README.md"), "").unwrap();
        std::fs::write(d.join(FIXTURE), "{}").unwrap();
        let r = check(&d, Some(""));
        assert!(r.passed(), "{}", r.text());
        assert_eq!(file_args(&["node".into(), "a.mjs".into(), "--x".into(), "lib/b".into(), "2.5".into()]), ["a.mjs", "lib/b", "2.5"]);
    }

    /// A `source` is described with the tag of this version's release; one
    /// without a version fails; a pattern that can't be used fails the
    /// manifest.
    #[test]
    fn check_describes_where_updates_come_from() {
        let t = crate::testing::TempDir::new("check-source");
        let source = r#""source": {"git": "https://example.com/r", "dir": "exts/x", "tags": "x-v{version}"}"#;
        let d = folder(&t, "x", &format!(r#"{{"id": "x", "name": "X", "version": "1.2.0", {source}}}"#));
        std::fs::write(d.join("README.md"), "").unwrap();
        let r = check(&d, Some(""));
        let text = r.text();
        assert!(r.passed(), "{text}");
        assert!(text.contains("its updates come from example.com/r (exts/x), tags x-v{version}: this version's release is the tag x-v1.2.0"), "{text}");
        assert!(text.contains("Its updates come from") && text.contains("example.com/r (exts/x), tags x-v{version}"), "{text}");
        std::fs::write(d.join(MANIFEST), format!(r#"{{"id": "x", "name": "X", {source}}}"#)).unwrap();
        let r = check(&d, Some(""));
        assert!(!r.passed() && r.text().contains("no release tag can be told newer"), "{}", r.text());
        std::fs::write(d.join(MANIFEST), r#"{"id": "x", "name": "X", "version": "1.0.0", "source": {"git": "http://example.com/r", "tags": "v{version}"}}"#).unwrap();
        assert!(!check(&d, Some("")).passed());
    }

    #[test]
    fn test_runs_the_fixture_s_expectations() {
        let t = crate::testing::TempDir::new("check-test");
        let d = folder(&t, "x", r#"{"id": "x", "name": "X", "settings": [{"key": "server", "type": "url", "label": "Server"}],
            "agentEnv": [{"name": "X_URL", "value": "{setting:server}"}],
            "actions": [{"id": "setup", "label": "Set up", "description": "d", "steps": [{"for": "claude", "run": ["claude", "add", "{setting:server}"]}, {"for": "codex", "run": ["codex", "add"]}]}]}"#);
        let r = test(&d, None);
        assert!(r.passed() && r.findings[0].text.contains("nothing to run"), "{r:?}");
        let fixture = serde_json::json!({
            "actions": {"installed": ["claude"], "expect": {"setup": {"label": "Set up", "lines": ["claude add '{setting:server}'", "codex add"], "skips": ["set Server first", "Codex isn't installed"], "optional": [false, false]}}},
            "env": {"unset": ["X_URL (not set until Server is)"], "values": {"server": "https://s.example"}, "set": ["X_URL=https://s.example"]}
        });
        std::fs::write(d.join(FIXTURE), fixture.to_string()).unwrap();
        let r = test(&d, None);
        assert!(r.passed(), "{}", r.text());
        assert_eq!(r.findings.len(), 3);
        let mut wrong = fixture.clone();
        wrong["actions"]["expect"]["setup"]["lines"][1] = "codex remove".into();
        wrong["env"]["set"][0] = "X_URL=https://other".into();
        wrong["actions"]["expect"]["gone"] = serde_json::json!({});
        std::fs::write(d.join(FIXTURE), wrong.to_string()).unwrap();
        let r = test(&d, None);
        let text = r.text();
        assert!(text.contains("action “gone” is expected"), "{text}");
        assert!(text.contains("not\n      claude add '{setting:server}'\n      codex remove"), "{text}");
        assert!(text.contains("3 checks failed"), "{text}");
        // A run whose program isn't installed is skipped, not failed.
        std::fs::write(d.join(MANIFEST), r#"{"id": "x", "name": "X", "adapter": {"command": ["no-such-runtime", "a.mjs"]}}"#).unwrap();
        std::fs::write(d.join(FIXTURE), r#"{"run": {"threads": 1}}"#).unwrap();
        let r = test(&d, Some(""));
        assert!(r.passed() && r.findings.iter().any(|f| f.level == Level::Skip), "{}", r.text());
    }

    /// A shell adapter, to drive a run without Node.
    #[test]
    fn a_run_drives_the_adapter_and_says_what_went_wrong() {
        if resolve(Path::new("."), "sh", None).is_err() {
            return;
        }
        let t = crate::testing::TempDir::new("check-run");
        let d = folder(&t, "sh", r#"{"id": "sh", "name": "Sh", "adapter": {"command": ["sh", "a.sh"]}}"#);
        let good = "read line\necho '{\"type\":\"hello\",\"protocol\":1}'\necho 'not json'\necho '{\"type\":\"status\",\"state\":\"ready\"}'\necho '{\"type\":\"threads\",\"threads\":[{\"id\":\"t1\",\"title\":\"One\"}]}'\nread line\necho '{\"type\":\"thread\",\"thread\":{\"id\":\"t1\",\"title\":\"One\",\"messages\":[{\"id\":\"m\",\"author\":{\"kind\":\"agent\",\"name\":\"a\"},\"text\":\"hi\"}]}}'\nread line\necho '{\"type\":\"result\",\"req\":1,\"ok\":true}'\nwhile read line; do :; done\n";
        std::fs::write(d.join("a.sh"), good).unwrap();
        std::fs::write(d.join(FIXTURE), r#"{"run": {"threads": 1, "open": "t1", "reply": "hello"}}"#).unwrap();
        let r = test(&d, None);
        let text = r.text();
        assert!(r.passed(), "{text}");
        for want in ["answers hello", "status “ready”", "lists 1 thread", "with 1 message", "reply is answered ok", "exits when its stdin closes", "a line Pitwall ignores (a line that isn't JSON: not json)"] {
            assert!(text.contains(want), "{want}: {text}");
        }
        // One that never answers hello, and one that ignores its stdin closing.
        std::fs::write(d.join("a.sh"), "read line\necho oops >&2\nexit 4\n").unwrap();
        let text = test(&d, None).text();
        assert!(text.contains("exited with code 4") && text.contains("oops"), "{text}");
        std::fs::write(d.join("a.sh"), "read line\necho '{\"type\":\"hello\",\"protocol\":1}'\necho '{\"type\":\"status\",\"state\":\"ready\"}'\necho '{\"type\":\"threads\",\"threads\":[]}'\ntrap '' HUP\nsleep 30\n").unwrap();
        std::fs::write(d.join(FIXTURE), r#"{"run": {"threads": 1}}"#).unwrap();
        let text = test(&d, None).text();
        assert!(text.contains("fewer than 1") && text.contains("didn't exit within 2 seconds"), "{text}");
    }
}
