//! First-launch auto-detect (docs/spec/onboarding.md). Strictly read-only:
//! it looks at agent transcripts, editor "recently opened" lists, a few
//! well-known code folders and the process table. The only programs it runs
//! are `<agent> --version`, `ps`, `lsof` and `sqlite3` (read-only,
//! immutable), plus each provider's read-only `discover` (agw: its listings).
//! Every step is independent and a failing step never aborts the scan.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use super::places::ScannedPlace;
use super::project_list::{self, ProjectList};
use crate::kind::{HookMode, KindCatalog};
use crate::model::KindView;
use crate::model::CodexHooksStatus;
use crate::paths::{self, Paths};
use crate::exec::local_stdout;
use crate::shell;
use crate::{hooks, platform, procs};

/// Lines read from a transcript before giving up on finding what we need.
const MAX_LINES: usize = 400;
const TITLE_CHARS: usize = 80;
const CONVERSATIONS_PER_PROJECT: usize = 5;
const MAX_CONVERSATIONS: usize = 40;
/// Conversations started outside a project (e.g. in `~`), listed separately.
const MAX_OUTSIDE_CONVERSATIONS: usize = 15;
const MAX_PROJECTS: usize = 80;
const MAX_CODEX_FILES: usize = 400;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// Folders whose immediate children are treated as projects (if they exist).
const CODE_ROOTS: &[&str] = &["projects", "code", "Developer", "src", "dev", "work"];

// ------------------------------------------------------------------ shapes

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedAgent {
    pub kind: String,
    pub name: String,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RulesInfo {
    pub rulesync: bool,
    pub claude_md: bool,
    pub agents_md: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedProject {
    pub path: String,
    pub display: String,
    pub is_git: bool,
    /// Unix ms; `None` when the source has no timestamp (editor recents).
    pub last_used: Option<u64>,
    /// "claude" | "codex" | "vscode" | "cursor" | "folder"
    pub sources: Vec<&'static str>,
    /// Agents have worked here: some source is an agent's conversations (not
    /// an editor's recents or a plain folder).
    pub agent_history: bool,
    /// Already in Pitwall's project list.
    pub added: bool,
    pub rules: RulesInfo,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub kind: String,
    pub kind_name: String,
    pub session_id: String,
    pub project_path: String,
    pub project_display: String,
    pub title: String,
    pub last_used: u64,
    /// A Pitwall agent already uses this session (set by `mark_in_pitwall`).
    pub in_pitwall: bool,
    /// Started in a folder that isn't a project (`~`, `/`, a temp folder);
    /// `project_path` is still where it must resume.
    pub outside_project: bool,
    /// Project the user chose to show it under last time (by session id).
    pub display_project: Option<String>,
    /// A process outside Pitwall is running this session right now.
    pub running_elsewhere: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningAgent {
    pub pid: u32,
    pub kind: String,
    pub kind_name: String,
    pub cwd: Option<String>,
    pub cwd_display: Option<String>,
    /// From the process args (`--resume <id>` / `--session-id <id>` /
    /// `codex resume <id>`), else the newest transcript for that cwd.
    pub session_id: Option<String>,
    /// Title of that conversation, when known.
    pub title: Option<String>,
    /// A Pitwall agent already uses this session.
    pub in_pitwall: bool,
    /// Its cwd isn't a project (`~` etc.); the UI offers "Show under project…".
    pub outside_project: bool,
    /// Project the user chose for this session last time.
    pub display_project: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub agents: Vec<ScannedAgent>,
    pub projects: Vec<ScannedProject>,
    pub conversations: Vec<Conversation>,
    pub running: Vec<RunningAgent>,
    /// Other places agents run (agw VMs, …) and the sessions there that
    /// can be added to Pitwall, from each provider's `discover`.
    pub places: Vec<ScannedPlace>,
    pub codex_hooks: Option<CodexHooksStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub step: &'static str,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

// ------------------------------------------------------------------ scan

/// Run every step, reporting progress as it goes. `projects` is Pitwall's
/// own list (marks what is already added, and remembered project choices).
/// `views`: the kinds on this machine, installed resolved (`Engine::list_kinds`).
/// `places`: the other places agents run, with their sessions (read-only
/// `discover` of each provider, see [`super::places`]).
pub fn scan(
    paths: &Paths,
    kinds: &KindCatalog,
    views: &[KindView],
    projects: &ProjectList,
    places: &(dyn Fn() -> Vec<ScannedPlace> + Sync),
    progress: &dyn Fn(ScanProgress),
) -> ScanResult {
    let mut result = ScanResult::default();
    let mut found = Found::default();
    let folders = Folders::new(paths);

    let step = |name: &'static str, f: &mut dyn FnMut() -> StepOutcome| {
        progress(ScanProgress { step: name, status: "running", summary: None });
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| StepOutcome::Error("failed".into()));
        let (status, summary) = match outcome {
            StepOutcome::Done(s) => ("done", s),
            StepOutcome::Error(s) => ("error", s),
        };
        progress(ScanProgress { step: name, status, summary: Some(summary) });
    };

    // Other places agents run (agw, …) are part of this step, not separate ones.
    step("agents", &mut || {
        let (agents, places) = std::thread::scope(|s| {
            let places = s.spawn(places);
            (scan_agents(views), places.join().unwrap_or_default())
        });
        result.agents = agents;
        result.places = places;
        let mut found: Vec<String> = result
            .agents
            .iter()
            .filter(|a| a.installed)
            .map(|a| match &a.version {
                Some(v) => format!("{} {v}", a.name),
                None => a.name.clone(),
            })
            .collect();
        found.extend(result.places.iter().map(place_summary));
        StepOutcome::Done(if found.is_empty() { "none found".into() } else { found.join(" · ") })
    });

    step("projects", &mut || {
        found = discover();
        result.projects = found.projects(&projects.known_paths(), &folders);
        StepOutcome::Done(plural(result.projects.len(), "project"))
    });

    step("conversations", &mut || {
        result.conversations = conversations(&found.conversations, &folders);
        StepOutcome::Done(format!("{} you can continue", result.conversations.len()))
    });

    step("running", &mut || {
        let elsewhere: Vec<String> = result
            .places
            .iter()
            .filter_map(|p| {
                let n: usize = p.machines.as_ref()?.iter().map(|m| m.sessions.len()).sum();
                (n > 0).then(|| format!("{} on {}", plural(n, "session"), p.label))
            })
            .collect();
        match running_elsewhere(&folders, kinds) {
            Some(list) => {
                result.running = list;
                let here = (!result.running.is_empty()).then(|| format!("{} on this Mac", result.running.len()));
                let parts: Vec<String> = here.into_iter().chain(elsewhere).collect();
                StepOutcome::Done(if parts.is_empty() { "none".into() } else { parts.join(" · ") })
            }
            None => StepOutcome::Error(
                std::iter::once("could not read the process list".to_string()).chain(elsewhere).collect::<Vec<_>>().join(" · "),
            ),
        }
    });
    fill_running_sessions(&mut result.running, &result.conversations);
    mark_running_elsewhere(&mut result.conversations, &result.running);
    apply_project_choices(&mut result, &projects.conversation_projects());

    step("rules", &mut || {
        for p in &mut result.projects {
            p.rules = rules_in(Path::new(&p.path));
        }
        let n = result.projects.iter().filter(|p| p.rules != RulesInfo::default()).count();
        StepOutcome::Done(format!("{} with rule files", plural(n, "project")))
    });

    step("hooks", &mut || {
        // Only when an installed agent takes its hooks from Codex's global file.
        let installed = |id: &str| views.iter().any(|v| v.id == id && v.installed);
        if !kinds.kinds().iter().any(|k| k.hooks == HookMode::CodexGlobal && installed(&k.id)) {
            return StepOutcome::Done("No agent needs global hooks".into());
        }
        let status = hooks::codex_status(paths);
        let summary = if status.installed { "Codex hooks installed" } else { "Codex hooks not installed" };
        result.codex_hooks = Some(status);
        StepOutcome::Done(summary.into())
    });

    result
}

enum StepOutcome {
    Done(String),
    Error(String),
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// "agw 0.19.0 (2 machines)" for the Agents step.
fn place_summary(p: &ScannedPlace) -> String {
    let mut s = match &p.version {
        Some(v) => format!("{} {v}", p.label),
        None => p.label.clone(),
    };
    if let Some(m) = &p.machines {
        s.push_str(&format!(" ({})", plural(m.len(), "machine")));
    }
    s
}

// ------------------------------------------------------------------ 1. agents

fn scan_agents(views: &[KindView]) -> Vec<ScannedAgent> {
    let views: Vec<_> = views
        .iter()
        .filter(|k| !k.caps.custom_command && k.id != crate::engine::terminals::TERMINAL_KIND)
        .cloned()
        .collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = views
            .into_iter()
            .map(|k| {
                s.spawn(move || {
                    let version = k.path.as_deref().and_then(|p| {
                        login_shell(&shell::LoginShell::current().invoke(p, "--version")).and_then(|o| version_line(&o))
                    });
                    ScannedAgent {
                        kind: k.id,
                        name: k.name,
                        installed: k.installed,
                        path: k.path,
                        version,
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

/// The version in the last output line that has one: "codex-cli 0.142.0" →
/// "0.142.0", "2.1.3 (Claude Code)" → "2.1.3".
pub fn version_line(out: &str) -> Option<String> {
    let line = out
        .lines()
        .map(str::trim)
        .rfind(|l| l.split_whitespace().any(looks_like_version))?;
    let word = line.split_whitespace().find(|w| looks_like_version(w))?;
    Some(truncate(word.trim_start_matches(['v', 'V']), 40))
}

fn looks_like_version(word: &str) -> bool {
    let w = word.trim_start_matches(['v', 'V']);
    w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.')
}

/// Run `script` in the user's login shell (PATH as in their terminal), time-boxed.
fn login_shell(script: &str) -> Option<String> {
    login_shell_within(script, COMMAND_TIMEOUT)
}

fn login_shell_within(script: &str, timeout: Duration) -> Option<String> {
    shell::LoginShell::current().output(script, timeout)
}

// ------------------------------------------------------------------ 2. projects

/// Project sources that are not agents' conversations.
const EDITOR_SOURCES: [&str; 3] = ["vscode", "cursor", "folder"];

#[derive(Default)]
struct Candidate {
    last_used: Option<u64>,
    sources: Vec<&'static str>,
}

#[derive(Default)]
struct Found {
    order: Vec<String>,
    candidates: HashMap<String, Candidate>,
    /// Transcripts to read titles from in the conversations step.
    conversations: Vec<(&'static str, PathBuf, u64)>,
}

impl Found {
    fn add(&mut self, path: &str, source: &'static str, last_used: Option<u64>) {
        let path = project_list::normalize(path);
        if !self.candidates.contains_key(&path) {
            self.order.push(path.clone());
        }
        let c = self.candidates.entry(path).or_default();
        if !c.sources.contains(&source) {
            c.sources.push(source);
        }
        c.last_used = match (c.last_used, last_used) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }

    fn projects(&self, known: &[String], folders: &Folders) -> Vec<ScannedProject> {
        let mut list: Vec<ScannedProject> = self
            .order
            .iter()
            .chain(known.iter().filter(|k| !self.candidates.contains_key(*k)))
            .filter(|p| folders.usable(p))
            .map(|p| {
                let c = self.candidates.get(p);
                ScannedProject {
                    display: paths::tildify(p),
                    is_git: Path::new(p).join(".git").exists(),
                    last_used: c.and_then(|c| c.last_used),
                    sources: c.map(|c| c.sources.clone()).unwrap_or_default(),
                    agent_history: c.is_some_and(|c| c.sources.iter().any(|s| !EDITOR_SOURCES.contains(s))),
                    added: known.contains(p),
                    rules: RulesInfo::default(),
                    path: p.clone(),
                }
            })
            .collect();
        // Recent first; untimed (editor recents) keep their own order after.
        list.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        list.truncate(MAX_PROJECTS);
        list
    }
}

/// Is `path` a folder the scan would list as a project (not `~`, `/`, temp)?
pub fn is_project_folder(paths: &Paths, path: &str) -> bool {
    Folders::new(paths).usable(&project_list::normalize(path))
}

/// Which folders are projects: not `/`, home, scratch folders or Pitwall's own.
struct Folders {
    home: String,
    pitwall: String,
}

impl Folders {
    fn new(paths: &Paths) -> Folders {
        Folders {
            home: paths::home().to_string_lossy().into_owned(),
            pitwall: paths.root().to_string_lossy().into_owned(),
        }
    }

    fn usable(&self, path: &str) -> bool {
        let tmp = std::env::temp_dir().to_string_lossy().trim_end_matches('/').to_string();
        // Scratch folders (agents run in temp dirs a lot) and Pitwall's own worktrees aren't projects.
        let scratch = ["/tmp", "/private/tmp", "/var/folders", "/private/var/folders", tmp.as_str(), self.pitwall.as_str()];
        path != "/"
            && path != self.home
            && !scratch.iter().any(|s| path == *s || path.starts_with(&format!("{s}/")))
            && Path::new(path).is_dir()
    }

    /// A conversation's folder that isn't a project but still worth offering
    /// (`~`, `/`, a temp folder that still exists). Pitwall's own dir is never.
    fn outside(&self, path: &str) -> bool {
        let pitwall = &self.pitwall;
        !(path == pitwall || path.starts_with(&format!("{pitwall}/"))) && Path::new(path).is_dir()
    }
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `*.jsonl` files directly in `dir`, newest first.
fn jsonl_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|e| Some((e.path(), mtime_ms(&e.metadata().ok()?))))
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files
}

fn open_lines(path: &Path) -> Option<BufReader<std::fs::File>> {
    std::fs::File::open(path).ok().map(BufReader::new)
}

fn discover() -> Found {
    let mut found = Found::default();
    let home = paths::home();

    // Claude Code: ~/.claude/projects/<encoded cwd>/<session>.jsonl
    if let Ok(dirs) = std::fs::read_dir(home.join(".claude").join("projects")) {
        for dir in dirs.flatten() {
            let files = jsonl_files(&dir.path());
            let Some(cwd) = files
                .iter()
                .take(3)
                .find_map(|(f, _)| open_lines(f).and_then(|r| parse_claude_transcript(r, false).cwd))
            else {
                continue;
            };
            found.add(&cwd, "claude", files.first().map(|f| f.1));
            for (f, t) in files.into_iter().take(CONVERSATIONS_PER_PROJECT) {
                found.conversations.push(("claude", f, t));
            }
        }
    }

    // Codex: ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl
    let mut codex_files = Vec::new();
    collect_jsonl(&home.join(".codex").join("sessions"), 4, &mut codex_files);
    codex_files.sort_by(|a, b| b.1.cmp(&a.1));
    codex_files.truncate(MAX_CODEX_FILES);
    let mut per_cwd: HashMap<String, usize> = HashMap::new();
    for (file, t) in codex_files {
        let Some(cwd) = open_lines(&file).and_then(|r| parse_codex_session(r, false).cwd) else { continue };
        found.add(&cwd, "codex", Some(t));
        let n = per_cwd.entry(cwd).or_default();
        if *n < CONVERSATIONS_PER_PROJECT {
            *n += 1;
            found.conversations.push(("codex", file, t));
        }
    }

    // VS Code / Cursor "recently opened".
    for (app, source) in [("Code", "vscode"), ("Cursor", "cursor")] {
        for path in editor_recents(app) {
            found.add(&path, source, None);
        }
    }

    // One level deep in well-known code folders (never Desktop/Documents/Downloads).
    for root in CODE_ROOTS {
        let Ok(entries) = std::fs::read_dir(home.join(root)) else { continue };
        for e in entries.flatten().take(200) {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(meta) = e.metadata() else { continue };
            if name.starts_with('.') || !meta.is_dir() {
                continue;
            }
            found.add(&e.path().to_string_lossy(), "folder", Some(mtime_ms(&meta)));
        }
    }
    found
}

fn collect_jsonl(dir: &Path, depth: u32, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let path = e.path();
        if meta.is_dir() {
            if depth > 0 {
                collect_jsonl(&path, depth - 1, out);
            }
        } else if path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            out.push((path, mtime_ms(&meta)));
        }
    }
}

/// Folders from an editor's recently-opened list (VS Code or a fork of it).
fn editor_recents(app: &str) -> Vec<String> {
    let dir = platform::app_support_dir()
        .join(app)
        .join("User/globalStorage");
    let mut out = Vec::new();
    let db = dir.join("state.vscdb");
    if db.is_file() {
        // Read-only + immutable: never takes a lock or writes a journal.
        let uri = format!("file:{}?mode=ro&immutable=1", percent_encode_path(&db.to_string_lossy()));
        let argv = [
            "/usr/bin/sqlite3",
            uri.as_str(),
            "SELECT value FROM ItemTable WHERE key = 'history.recentlyOpenedPathsList'",
        ];
        if let Some(json) = local_stdout(&argv, COMMAND_TIMEOUT) {
            out.extend(parse_vscode_recents(&json));
        }
    }
    if let Ok(src) = std::fs::read_to_string(dir.join("storage.json")) {
        for p in parse_vscode_recents(&src) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

fn percent_encode_path(p: &str) -> String {
    let mut out = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Local folder paths from VS Code's `history.recentlyOpenedPathsList` value
/// (`{"entries":[{"folderUri":…}]}`) or an older `storage.json`
/// (`{"openedPathsList":{"workspaces3":[…],"entries":[…]}}`). Workspaces,
/// files and remote folders are skipped.
pub fn parse_vscode_recents(json: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<Value>(json.trim()) else { return vec![] };
    let lists: Vec<&Value> = match root.get("openedPathsList") {
        Some(opened) => ["entries", "workspaces3", "folders2", "folders"]
            .iter()
            .filter_map(|k| opened.get(*k))
            .collect(),
        None => root.get("entries").into_iter().collect(),
    };
    let mut out: Vec<String> = Vec::new();
    for item in lists.into_iter().filter_map(Value::as_array).flatten() {
        let uri = match item {
            Value::String(s) => Some(s.as_str()),
            Value::Object(_) => item.get("folderUri").and_then(Value::as_str),
            _ => None,
        };
        let Some(path) = uri.and_then(|u| u.strip_prefix("file://")).map(percent_decode) else { continue };
        let path = project_list::normalize(&path);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

// ------------------------------------------------------------------ transcripts

#[derive(Debug, Default, PartialEq)]
pub struct TranscriptInfo {
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub title: Option<String>,
}

fn json_lines(r: impl BufRead) -> impl Iterator<Item = Value> {
    r.split(b'\n')
        .take(MAX_LINES)
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_slice::<Value>(&line).ok())
}

/// A prompt the user typed (not a slash command, caveat or injected context).
fn usable_prompt(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.starts_with('<') || t.starts_with("Caveat:") {
        return None;
    }
    Some(title_of(t))
}

/// Whitespace collapsed, at most ~80 characters.
pub fn title_of(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&collapsed, TITLE_CHARS)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Text of a Claude user message; `None` for tool results.
fn claude_user_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            if items.iter().any(|i| i.get("type").and_then(Value::as_str) == Some("tool_result")) {
                return None;
            }
            let text: Vec<&str> = items
                .iter()
                .filter(|i| i.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|i| i.get("text").and_then(Value::as_str))
                .collect();
            (!text.is_empty()).then(|| text.join("\n"))
        }
        _ => None,
    }
}

/// cwd, session id and (if `want_title`) first user prompt of a Claude Code
/// transcript (`~/.claude/projects/*/<session>.jsonl`).
pub fn parse_claude_transcript(r: impl BufRead, want_title: bool) -> TranscriptInfo {
    let mut info = TranscriptInfo::default();
    for v in json_lines(r) {
        if info.cwd.is_none() {
            info.cwd = v.get("cwd").and_then(Value::as_str).map(String::from);
        }
        if info.session_id.is_none() {
            info.session_id = v.get("sessionId").and_then(Value::as_str).map(String::from);
        }
        if want_title
            && info.title.is_none()
            && v.get("type").and_then(Value::as_str) == Some("user")
            && v.get("isMeta").and_then(Value::as_bool) != Some(true)
            && v.get("isSidechain").and_then(Value::as_bool) != Some(true)
        {
            info.title = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(claude_user_text)
                .and_then(|t| usable_prompt(&t));
        }
        if info.cwd.is_some() && info.session_id.is_some() && (!want_title || info.title.is_some()) {
            break;
        }
    }
    info
}

fn env_context_cwd(text: &str) -> Option<String> {
    let start = text.find("<cwd>")? + "<cwd>".len();
    let end = text[start..].find("</cwd>")? + start;
    Some(text[start..end].trim().to_string()).filter(|s| !s.is_empty())
}

/// Text items of a Codex `message` item.
fn codex_message_text(msg: &Value) -> Vec<&str> {
    msg.get("content")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|i| i.get("text").and_then(Value::as_str)).collect())
        .unwrap_or_default()
}

/// Session id, cwd and (if `want_title`) first user prompt of a Codex
/// session log (`~/.codex/sessions/**/rollout-*.jsonl`). Handles the
/// current `session_meta` format and the older bare-header format.
pub fn parse_codex_session(r: impl BufRead, want_title: bool) -> TranscriptInfo {
    let mut info = TranscriptInfo::default();
    for (i, v) in json_lines(r).enumerate() {
        let kind = v.get("type").and_then(Value::as_str);
        let payload = v.get("payload");
        match (kind, payload) {
            (Some("session_meta"), Some(p)) => {
                info.session_id = p
                    .get("id")
                    .or_else(|| p.get("session_id"))
                    .and_then(Value::as_str)
                    .map(String::from);
                info.cwd = p.get("cwd").and_then(Value::as_str).map(String::from).or(info.cwd);
            }
            (Some("event_msg"), Some(p)) if p.get("type").and_then(Value::as_str) == Some("user_message") => {
                if want_title && info.title.is_none() {
                    info.title = p.get("message").and_then(Value::as_str).and_then(usable_prompt);
                }
            }
            _ => {
                // Older logs: header line without "type", then bare message items.
                if i == 0 && kind.is_none() && info.session_id.is_none() {
                    info.session_id = v.get("id").and_then(Value::as_str).map(String::from);
                }
                let msg = match (kind, payload) {
                    (Some("response_item"), Some(p)) => Some(p),
                    (Some("message"), _) => Some(&v),
                    _ => None,
                };
                if let Some(msg) = msg.filter(|m| m.get("role").and_then(Value::as_str) == Some("user")) {
                    for text in codex_message_text(msg) {
                        if info.cwd.is_none() {
                            info.cwd = env_context_cwd(text);
                        }
                        if want_title && info.title.is_none() {
                            info.title = usable_prompt(text);
                        }
                    }
                }
            }
        }
        if info.cwd.is_some() && info.session_id.is_some() && (!want_title || info.title.is_some()) {
            break;
        }
    }
    info
}

// ------------------------------------------------------------------ 3. conversations

fn conversations(candidates: &[(&'static str, PathBuf, u64)], folders: &Folders) -> Vec<Conversation> {
    let mut out: Vec<Conversation> = candidates
        .iter()
        .filter_map(|(kind, file, t)| {
            let r = open_lines(file)?;
            let info = match *kind {
                "claude" => {
                    let mut info = parse_claude_transcript(r, true);
                    // The file name is the session id `claude --resume` takes.
                    info.session_id = file.file_stem().map(|s| s.to_string_lossy().into_owned()).or(info.session_id);
                    info
                }
                _ => parse_codex_session(r, true),
            };
            let cwd = project_list::normalize(&info.cwd?);
            let outside_project = !folders.usable(&cwd);
            if outside_project && !folders.outside(&cwd) {
                return None;
            }
            Some(Conversation {
                kind: kind.to_string(),
                kind_name: if *kind == "claude" { "Claude Code" } else { "Codex" }.into(),
                session_id: info.session_id?,
                project_display: paths::tildify(&cwd),
                project_path: cwd,
                title: info.title?,
                last_used: *t,
                in_pitwall: false,
                outside_project,
                display_project: None,
                running_elsewhere: false,
            })
        })
        .collect();
    out.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    cap_conversations(out)
}

/// Newest first; project conversations and outside ones capped separately so
/// a busy `~` never crowds real projects out (or the other way round).
fn cap_conversations(sorted: Vec<Conversation>) -> Vec<Conversation> {
    let (mut inside, mut outside) = (0, 0);
    sorted
        .into_iter()
        .filter(|c| {
            let (n, max) = if c.outside_project {
                (&mut outside, MAX_OUTSIDE_CONVERSATIONS)
            } else {
                (&mut inside, MAX_CONVERSATIONS)
            };
            *n += 1;
            *n <= max
        })
        .collect()
}

/// Conversations whose session a process outside Pitwall is running now.
fn mark_running_elsewhere(conversations: &mut [Conversation], running: &[RunningAgent]) {
    for c in conversations.iter_mut() {
        c.running_elsewhere = running
            .iter()
            .any(|r| r.kind == c.kind && r.session_id.as_deref() == Some(c.session_id.as_str()));
    }
}

/// Fill in the project the user picked last time for outside-project rows.
pub fn apply_project_choices(result: &mut ScanResult, choices: &std::collections::BTreeMap<String, String>) {
    let usable = |p: &String| Path::new(p).is_dir();
    for c in result.conversations.iter_mut().filter(|c| c.outside_project) {
        c.display_project = choices.get(&c.session_id).filter(|p| usable(p)).cloned();
    }
    for r in result.running.iter_mut().filter(|r| r.outside_project) {
        r.display_project = r
            .session_id
            .as_ref()
            .and_then(|id| choices.get(id))
            .filter(|p| usable(p))
            .cloned();
    }
}

// ------------------------------------------------------------------ 4. running elsewhere

/// Session id (if the args didn't name one) and title from the newest
/// conversation of the same kind in the same folder.
fn fill_running_sessions(running: &mut [RunningAgent], conversations: &[Conversation]) {
    for r in running.iter_mut() {
        let cwd = r.cwd.as_deref().map(project_list::normalize);
        if r.session_id.is_none() {
            // `conversations` is newest first.
            r.session_id = conversations
                .iter()
                .find(|c| c.kind == r.kind && Some(&c.project_path) == cwd.as_ref())
                .map(|c| c.session_id.clone());
        }
        if let Some(id) = &r.session_id {
            r.title = conversations.iter().find(|c| &c.session_id == id).map(|c| c.title.clone());
        }
    }
}

/// Flag conversations / running agents whose session a Pitwall agent already has.
pub fn mark_in_pitwall(result: &mut ScanResult, sessions: &std::collections::HashSet<String>) {
    for c in &mut result.conversations {
        c.in_pitwall = sessions.contains(&c.session_id);
    }
    for r in &mut result.running {
        r.in_pitwall = r.session_id.as_ref().is_some_and(|s| sessions.contains(s));
    }
}

fn running_elsewhere(folders: &Folders, kinds: &KindCatalog) -> Option<Vec<RunningAgent>> {
    let out = platform::process_table()?;
    let matcher = procs::Matcher::new(&kinds.kinds());
    let roots = std::collections::HashSet::from([std::process::id()]);
    let found = procs::agents_outside(&procs::parse_table(&out), &roots, &matcher);
    let cwds: HashMap<u32, String> = if found.is_empty() {
        HashMap::new()
    } else {
        let pids: Vec<u32> = found.iter().map(|p| p.pid).collect();
        platform::process_cwds(&pids, COMMAND_TIMEOUT)
            .map(|cwds| cwds.into_iter().collect())
            .unwrap_or_default()
    };
    Some(
        found
            .into_iter()
            .map(|procs::Found { pid, kind, kind_name, session_id }| {
                let cwd = cwds.get(&pid).cloned();
                let outside_project = cwd
                    .as_deref()
                    .is_some_and(|c| !folders.usable(&project_list::normalize(c)));
                RunningAgent {
                    pid,
                    kind,
                    kind_name,
                    cwd_display: cwd.as_deref().map(paths::tildify),
                    cwd,
                    session_id,
                    title: None,
                    in_pitwall: false,
                    outside_project,
                    display_project: None,
                }
            })
            .collect(),
    )
}

// ------------------------------------------------------------------ helpers

/// The JSON document in a command's output (a login shell may print noise first).
pub fn json_payload(out: &str) -> Option<Value> {
    let start = out.find(['[', '{'])?;
    serde_json::from_str(out[start..].trim()).ok()
}


// ------------------------------------------------------------------ 6. rules

pub fn rules_in(dir: &Path) -> RulesInfo {
    RulesInfo {
        rulesync: dir.join(".rulesync").is_dir(),
        claude_md: dir.join("CLAUDE.md").is_file(),
        agents_md: dir.join("AGENTS.md").is_file(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn reader(s: &str) -> &[u8] {
        s.as_bytes()
    }

    #[test]
    fn claude_transcript_cwd_session_and_title() {
        let src = include_str!("scan_fixtures/claude_transcript.jsonl");
        let info = parse_claude_transcript(reader(src), true);
        assert_eq!(info.cwd.as_deref(), Some("/Users/dev/code/orders-api"));
        assert_eq!(info.session_id.as_deref(), Some("19bd83bc-617c-4e41-8e79-ff6d237cff1c"));
        let title = info.title.unwrap();
        assert!(title.starts_with("Fix the 500 on /v2/orders when the cart is empty. Add a regression"), "{title}");
        assert!(title.chars().count() <= 80);
        assert!(title.ends_with('…'));

        let quick = parse_claude_transcript(reader(src), false);
        assert_eq!(quick.title, None);
        assert_eq!(quick.cwd.as_deref(), Some("/Users/dev/code/orders-api"));
    }

    #[test]
    fn claude_transcript_without_prompt_has_no_title() {
        let info = parse_claude_transcript(reader(include_str!("scan_fixtures/claude_tool_only.jsonl")), true);
        assert_eq!(info.cwd.as_deref(), Some("/Users/dev/code/handbook"));
        assert_eq!(info.session_id.as_deref(), Some("s2"));
        assert_eq!(info.title, None);
        assert_eq!(parse_claude_transcript(reader(""), true), TranscriptInfo::default());
    }

    #[test]
    fn codex_session_meta() {
        let info = parse_codex_session(reader(include_str!("scan_fixtures/codex_session.jsonl")), true);
        assert_eq!(info.cwd.as_deref(), Some("/Users/dev/code/signup"));
        assert_eq!(info.session_id.as_deref(), Some("615b8985-7987-4a23-b961-3eb1b48731ec"));
        assert_eq!(info.title.as_deref(), Some("add input validation to the signup form"));
    }

    #[test]
    fn codex_old_format() {
        let info = parse_codex_session(reader(include_str!("scan_fixtures/codex_session_old.jsonl")), true);
        assert_eq!(info.cwd.as_deref(), Some("/Users/dev/code/legacy"));
        assert_eq!(info.session_id.as_deref(), Some("047921e2-e8c8-43fa-b1fe-14bbd6c814f0"));
        assert_eq!(info.title.as_deref(), Some("rename the config loader"));
    }

    #[test]
    fn vscode_recents_state_db_value() {
        let got = parse_vscode_recents(include_str!("scan_fixtures/vscode_recents.json"));
        assert_eq!(got, vec!["/Users/dev/Desktop/billing-api", "/Users/dev/My Projects/web app"]);
    }

    #[test]
    fn vscode_recents_storage_json() {
        let got = parse_vscode_recents(include_str!("scan_fixtures/vscode_storage.json"));
        assert_eq!(got, vec!["/Users/dev/old/three", "/Users/dev/old/one", "/Users/dev/old/two"]);
        assert!(parse_vscode_recents("").is_empty());
        assert!(parse_vscode_recents("{\"theme\":1}").is_empty());
    }

    fn conv(kind: &str, id: &str, cwd: &str, t: u64) -> Conversation {
        Conversation {
            kind: kind.into(),
            kind_name: kind.into(),
            session_id: id.into(),
            project_path: cwd.into(),
            project_display: cwd.into(),
            title: format!("t-{id}"),
            last_used: t,
            in_pitwall: false,
            outside_project: false,
            display_project: None,
            running_elsewhere: false,
        }
    }

    fn running(kind: &str, cwd: Option<&str>, session: Option<&str>) -> RunningAgent {
        RunningAgent {
            pid: 1,
            kind: kind.into(),
            kind_name: kind.into(),
            cwd: cwd.map(Into::into),
            cwd_display: None,
            session_id: session.map(Into::into),
            title: None,
            in_pitwall: false,
            outside_project: false,
            display_project: None,
        }
    }

    #[test]
    fn home_conversations_are_kept_separately() {
        let home = paths::home().to_string_lossy().into_owned();
        let pw = Paths::new(paths::home().join(".pitwall"));
        let folders = Folders::new(&pw);
        assert!(!is_project_folder(&pw, &home));
        assert!(!is_project_folder(&pw, "/"));
        assert!(folders.outside(&home));
        assert!(!folders.outside(&pw.legacy_worktrees_dir().to_string_lossy()));
        assert!(!folders.outside("/definitely/not/here"));

        let mut list: Vec<Conversation> = (0..50).map(|i| conv("claude", &format!("p{i}"), "/p", 1000 - i)).collect();
        for i in 0..20 {
            let mut c = conv("claude", &format!("h{i}"), &home, 2000 - i);
            c.outside_project = true;
            list.push(c);
        }
        list.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        let capped = cap_conversations(list);
        assert_eq!(capped.iter().filter(|c| c.outside_project).count(), MAX_OUTSIDE_CONVERSATIONS);
        assert_eq!(capped.iter().filter(|c| !c.outside_project).count(), MAX_CONVERSATIONS);
        assert_eq!(capped[0].session_id, "h0");
    }

    #[test]
    fn running_sessions_and_remembered_projects() {
        let home = paths::home().to_string_lossy().into_owned();
        let tmp = std::env::temp_dir().join(format!("pitwall-choice-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let tmp = tmp.to_string_lossy().into_owned();
        let mut h = conv("claude", "h", &home, 5);
        h.outside_project = true;
        let mut gone = conv("claude", "g", &home, 4);
        gone.outside_project = true;
        let mut res = ScanResult {
            conversations: vec![h, gone, conv("claude", "p", "/p", 3)],
            running: vec![running("claude", Some(&home), Some("h")), running("codex", Some("/p"), Some("p"))],
            ..Default::default()
        };
        res.running[0].outside_project = true;
        mark_running_elsewhere(&mut res.conversations, &res.running);
        assert!(res.conversations[0].running_elsewhere);
        assert!(!res.conversations[1].running_elsewhere);
        // A codex process doesn't make a claude session "running elsewhere".
        assert!(!res.conversations[2].running_elsewhere);

        let choices = [("h", tmp.as_str()), ("g", "/definitely/not/here"), ("p", tmp.as_str())]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        apply_project_choices(&mut res, &choices);
        assert_eq!(res.conversations[0].display_project.as_deref(), Some(tmp.as_str()));
        assert_eq!(res.conversations[1].display_project, None);
        assert_eq!(res.conversations[2].display_project, None);
        assert_eq!(res.running[0].display_project.as_deref(), Some(tmp.as_str()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn running_sessions_fall_back_to_newest_transcript() {
        let convs = vec![conv("claude", "new", "/p", 9), conv("codex", "cx", "/p", 8), conv("claude", "old", "/p", 1)];
        let mut r = vec![
            running("claude", Some("/p/"), None),
            running("codex", Some("/p"), Some("given")),
            running("claude", None, None),
            running("claude", Some("/other"), None),
        ];
        fill_running_sessions(&mut r, &convs);
        assert_eq!(r[0].session_id.as_deref(), Some("new"));
        assert_eq!(r[0].title.as_deref(), Some("t-new"));
        assert_eq!(r[1].session_id.as_deref(), Some("given"));
        assert_eq!(r[1].title, None);
        assert_eq!(r[2].session_id, None);
        assert_eq!(r[3].session_id, None);
    }

    #[test]
    fn marks_sessions_pitwall_already_has() {
        let mut res = ScanResult {
            conversations: vec![conv("claude", "a", "/p", 2), conv("claude", "b", "/p", 1)],
            running: vec![running("claude", Some("/p"), Some("a")), running("claude", Some("/p"), None)],
            ..Default::default()
        };
        mark_in_pitwall(&mut res, &["a".to_string()].into_iter().collect());
        assert!(res.conversations[0].in_pitwall && !res.conversations[1].in_pitwall);
        assert!(res.running[0].in_pitwall && !res.running[1].in_pitwall);
    }

    #[test]
    fn versions_and_counts() {
        assert_eq!(version_line("Last login: Mon 6 10:00\n2.1.3 (Claude Code)\n").as_deref(), Some("2.1.3"));
        assert_eq!(version_line("codex-cli 0.142.0").as_deref(), Some("0.142.0"));
        assert_eq!(version_line("agw v0.19.0").as_deref(), Some("0.19.0"));
        assert_eq!(version_line("2025.09.18-7ae6800").as_deref(), Some("2025.09.18-7ae6800"));
        assert_eq!(version_line("no digits 42"), None);
    }

    #[test]
    fn json_after_noise() {
        assert_eq!(json_payload("Last login: Mon\n{\"a\":1}"), Some(serde_json::json!({"a": 1})));
        assert!(json_payload("oops").is_none());
    }

    #[test]
    fn places_are_summarised() {
        let p = ScannedPlace {
            provider: "agw".into(),
            label: "agw".into(),
            version: Some("0.19.0".into()),
            machines: Some(vec![]),
        };
        assert_eq!(place_summary(&p), "agw 0.19.0 (0 machines)");
        assert_eq!(place_summary(&ScannedPlace { machines: None, version: None, ..p }), "agw");
    }

    #[test]
    fn paths_and_titles() {
        assert_eq!(percent_decode("/a%20b/%E2%9C%93"), "/a b/✓");
        assert_eq!(percent_decode("/50%"), "/50%");
        assert_eq!(percent_encode_path("/Library/Application Support/x"), "/Library/Application%20Support/x");
        assert_eq!(title_of("  a\n\n b  "), "a b");
        assert_eq!(env_context_cwd("<cwd> /x </cwd>").as_deref(), Some("/x"));
    }

    #[test]
    fn found_merges_sources() {
        let mut f = Found::default();
        f.add("/a/", "claude", Some(5));
        f.add("/a", "vscode", None);
        f.add("/a", "codex", Some(9));
        let c = &f.candidates["/a"];
        assert_eq!(c.sources, vec!["claude", "vscode", "codex"]);
        assert_eq!(c.last_used, Some(9));
        assert_eq!(f.order, vec!["/a"]);
    }

    /// Read-only scan of this machine: `cargo test smoke_scan -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn smoke_scan() {
        let pw = Paths::new(Paths::default_root());
        let kinds = KindCatalog::new(pw.user_agents_dir());
        let projects = ProjectList::new(pw.projects_file());
        let t = Instant::now();
        let r = scan(&pw, &kinds, &[], &projects, &Vec::new, &|p| eprintln!("{:>13} {:>8} {}", p.step, p.status, p.summary.unwrap_or_default()));
        eprintln!("took {:?}", t.elapsed());
        for p in r.projects.iter().take(12) {
            eprintln!("  {} {:?} {:?}", p.display, p.sources, p.last_used);
        }
        for c in r.conversations.iter().take(8) {
            eprintln!("  [{}] {} — {} ({})", c.kind, c.project_display, c.title, c.session_id);
        }
        for a in &r.running {
            eprintln!("  pid {} {} {:?}", a.pid, a.kind, a.cwd_display);
        }
    }

    #[test]
    fn rules_detection() {
        let dir = std::env::temp_dir().join(format!("pitwall-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".rulesync")).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "x").unwrap();
        assert_eq!(rules_in(&dir), RulesInfo { rulesync: true, claude_md: false, agents_md: true });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
