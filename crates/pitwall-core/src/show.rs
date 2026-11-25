//! Agents showing files to the user (`agents.showFiles`, `viewer.open`),
//! web pages (`agents.showWeb`, `web.open`) and Pitwall's Settings
//! (`agents.showSettings`, `settings.open`) and device screens
//! (`agents.showDevice`, `device.open`): whether each is on, the
//! MCP server the agents Pitwall starts are given (while either is),
//! and turning what an agent asked for (`src/a.rs:42`, a folder, a
//! worktree's name) into full paths on the agent's machine that exist.
//! Showing them is the host's (the app's viewer). Writes only the
//! built-in rule and, when the user asks, Pitwall's entry in a tool's MCP
//! file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use pitwall_proto::{OpenedFile, Skipped, ViewerOpen, ViewerShow};

use crate::engine::Engine;
use crate::exec::{self, Exec, FileKind};
use crate::paths::expand_tilde_in;
use crate::kind::{mcp_file, ShowFilesRoute};
use crate::provider::McpWiring;

/// What the engine keeps: the settings, and the `pitwall-cli` to run.
pub(crate) struct State {
    on: AtomicBool,
    /// `agents.showWeb`.
    web: AtomicBool,
    /// `general.webPages`.
    web_pages: AtomicBool,
    /// `agents.showSettings`.
    settings: AtomicBool,
    /// `agents.showDevice`.
    device: AtomicBool,
    /// `general.devicePreviews`.
    device_previews: AtomicBool,
    cli: RwLock<Option<PathBuf>>,
}

impl Default for State {
    fn default() -> State {
        // On by default, as the settings.
        State { on: AtomicBool::new(true), web: AtomicBool::new(true), web_pages: AtomicBool::new(true), settings: AtomicBool::new(true), device: AtomicBool::new(true), device_previews: AtomicBool::new(true), cli: RwLock::new(None) }
    }
}

impl Engine {
    /// `agents.showFiles`, as the host follows it.
    pub fn set_show_files(&self, on: bool) {
        self.show.on.store(on, Ordering::Relaxed);
        self.sync_show_rule();
    }

    /// `agents.showWeb` and `general.webPages`, as the host follows them:
    /// agents may show web pages while both are on.
    pub fn set_show_web(&self, tool: bool, pages: bool) {
        let was = self.show_web_on();
        self.show.web.store(tool, Ordering::Relaxed);
        self.show.web_pages.store(pages, Ordering::Relaxed);
        // (The rule says only whether pages can be shown.)
        if was != self.show_web_on() {
            self.sync_show_rule();
        }
    }

    /// `agents.showSettings`, as the host follows it.
    pub fn set_show_settings(&self, on: bool) {
        let was = self.show_settings_on();
        self.show.settings.store(on, Ordering::Relaxed);
        if was != on {
            self.sync_show_rule();
        }
    }

    /// `agents.showDevice` and `general.devicePreviews`, as the host
    /// follows them: agents may show devices while both are on.
    pub fn set_show_device(&self, tool: bool, previews: bool) {
        let was = self.show_device_on();
        self.show.device.store(tool, Ordering::Relaxed);
        self.show.device_previews.store(previews, Ordering::Relaxed);
        if was != self.show_device_on() {
            self.sync_show_rule();
        }
    }

    /// Agents may show the user a device's screen (`agents.showDevice` and
    /// `general.devicePreviews`).
    pub fn show_device_on(&self) -> bool {
        self.show.device.load(Ordering::Relaxed) && self.show.device_previews.load(Ordering::Relaxed)
    }

    /// Why agents can't show devices now, in the user's words (`None`:
    /// they can).
    pub fn show_device_off(&self) -> Option<&'static str> {
        if !self.show.device_previews.load(Ordering::Relaxed) {
            return Some("Device screens are turned off in Pitwall (general.devicePreviews, or `pitwall settings set general.devicePreviews true`). Nothing was opened.");
        }
        if !self.show.device.load(Ordering::Relaxed) {
            return Some("Showing devices is turned off in Pitwall (Settings → Agents → “Agents can show you a device”, agents.showDevice). Nothing was opened.");
        }
        None
    }

    /// Agents may show the user Pitwall's Settings (`agents.showSettings`).
    pub fn show_settings_on(&self) -> bool {
        self.show.settings.load(Ordering::Relaxed)
    }

    /// The built-in rule for agents without an MCP way, in step.
    fn sync_show_rule(&self) {
        let cli = self.show_files_wiring().map(|m| m.command);
        let what = crate::rules::ShowWhat { files: self.show_files_on(), web: self.show_web_on(), settings: self.show_settings_on(), device: self.show_device_on() };
        crate::rules::sync_show_files_rule(&crate::rules::Dirs::of(self.paths()), cli.as_deref(), what);
    }

    pub fn show_files_on(&self) -> bool {
        self.show.on.load(Ordering::Relaxed)
    }

    /// Agents may show web pages (`agents.showWeb` and `general.webPages`).
    pub fn show_web_on(&self) -> bool {
        self.show.web.load(Ordering::Relaxed) && self.show.web_pages.load(Ordering::Relaxed)
    }

    /// Why agents can't show web pages now, in the user's words (`None`:
    /// they can).
    pub fn show_web_off(&self) -> Option<&'static str> {
        if !self.show.web_pages.load(Ordering::Relaxed) {
            return Some("Web pages are turned off in Pitwall (Settings → General → “Show web pages in Pitwall”, general.webPages). Nothing was opened.");
        }
        if !self.show.web.load(Ordering::Relaxed) {
            return Some("Showing web pages is turned off in Pitwall (Settings → Agents → “Agents can show you web pages”, agents.showWeb). Nothing was opened.");
        }
        None
    }

    /// The `pitwall-cli` the host ships (`None`: this build has none, and
    /// agents get no MCP server).
    pub fn set_cli_bin(&self, cli: Option<PathBuf>) {
        *self.show.cli.write().unwrap_or_else(|e| e.into_inner()) = cli;
        self.sync_show_rule();
    }

    /// The MCP server for an agent starting now: none when showing files
    /// and web pages are both off, or there is no CLI.
    pub fn show_files_wiring(&self) -> Option<McpWiring> {
        if !self.show_files_on() && !self.show_web_on() && !self.show_device_on() {
            return None;
        }
        self.show_files_server()
    }

    /// The MCP server, setting or not (what goes in a tool's MCP file).
    pub fn show_files_server(&self) -> Option<McpWiring> {
        let cli = self.show.cli.read().unwrap_or_else(|e| e.into_inner()).clone()?;
        Some(McpWiring { command: cli.to_string_lossy().into_owned(), args: vec!["mcp".into()] })
    }

    /// How each agent kind gets the ability (Settings → Agents).
    pub fn show_files_routes(&self) -> Vec<KindRoute> {
        let server = self.show_files_server();
        self.kinds()
            .kinds()
            .into_iter()
            .map(|k| {
                let file = k.mcp_file.as_ref();
                KindRoute {
                    route: k.show_files_route(),
                    file: file.map(|f| f.path.clone()),
                    entry: match (file, &server) {
                        (Some(f), Some(m)) => Some(mcp_file::status_at(&mcp_file::location(f), f, m)),
                        _ => None,
                    },
                    id: k.id,
                    name: k.name,
                }
            })
            .collect()
    }

    /// Add Pitwall's server to `kind`'s own MCP file, or take it out
    /// (only when the user asks). Returns the file, as shown (`~/…`).
    pub fn set_show_files_file(&self, kind: &str, add: bool) -> Result<String, String> {
        let k = self.kinds().find(kind).ok_or_else(|| format!("no agent kind \"{kind}\""))?;
        let f = k.mcp_file.as_ref().ok_or_else(|| format!("{} doesn't read Pitwall's server from a file", k.name))?;
        let at = mcp_file::location(f);
        if add {
            let m = self.show_files_server().ok_or("this build has no pitwall-cli to run")?;
            mcp_file::add_at(&at, f, &m)?;
        } else {
            mcp_file::remove_at(&at)?;
        }
        Ok(f.path.clone())
    }
}

/// How one agent kind gets `agents.showFiles`.
#[derive(Debug, Clone)]
pub struct KindRoute {
    pub id: String,
    pub name: String,
    pub route: ShowFilesRoute,
    /// [`ShowFilesRoute::ConfigFile`]: the file (`~/…`).
    pub file: Option<String>,
    /// Pitwall's entry in it (None: no CLI to compare with). Err: the
    /// file can't be read.
    pub entry: Option<Result<mcp_file::Entry, String>>,
}

/// `path`, `path:line` or `path:line:col` (1-based). A path that ends in
/// digits after a colon is taken as a position; [`resolve`] tries the whole
/// text as a name first.
pub fn split_position(spec: &str) -> (String, Option<u32>, Option<u32>) {
    let spec = spec.trim();
    let num = |s: &str| (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<u32>().ok()).flatten();
    if let Some((rest, last)) = spec.rsplit_once(':') {
        if let Some(n) = num(last) {
            if let Some((path, line)) = rest.rsplit_once(':') {
                if let Some(l) = num(line).filter(|_| !path.is_empty()) {
                    return (path.to_string(), Some(l), Some(n));
                }
            }
            if !rest.is_empty() {
                return (rest.to_string(), Some(n), None);
            }
        }
    }
    (spec.to_string(), None, None)
}

/// A full path on a Unix machine or Windows.
fn absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with(['/', '\\']) || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// `p` (`~` expanded) as a full path, relative ones from `base`.
fn full(home: Option<&str>, base: &str, p: &str) -> String {
    let p = expand_tilde_in(home, p.trim());
    if absolute(&p) {
        p
    } else {
        exec::join(base, &p)
    }
}

/// What a resolve needs about the agent and its machine.
pub struct Place<'a> {
    pub exec: &'a dyn Exec,
    /// The folder the agent started in (its record's).
    pub cwd: &'a str,
    /// The folder it works in now, when Pitwall knows (its latest hook,
    /// the folder it said it works in, its process's): a worktree it
    /// entered mid-session, a folder it moved to.
    pub now: Option<&'a str>,
    /// Worktrees a root may name ([`worktrees_near`]): those of the
    /// agent's repository, of the one it works in now and of the caller's
    /// folder (`pitwall open`'s shell).
    pub worktrees: &'a [Worktree],
    /// Worktrees of the repositories directly inside the agent's folder,
    /// when that isn't in a repository itself (an agent started in `~` or
    /// in a projects folder): looked in only when none of `worktrees` is
    /// the one named.
    pub inside: &'a [Worktree],
    /// The agent runs on this machine. Elsewhere only its own folder can
    /// be shown (the viewer reads other files from this disk).
    pub local: bool,
}

/// A git worktree a root may name: its folder, the folder's name and its
/// branch (`None`: detached).
pub type Worktree = (String, String, Option<String>);

/// The worktrees of the repositories holding each of `dirs` (each
/// repository once, its main checkout first; bare ones left out).
pub fn worktrees_near(x: &dyn Exec, dirs: &[&str]) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = vec![];
    for d in dirs.iter().filter(|d| !d.trim().is_empty()) {
        let git = crate::vcs::git::Git::new(x, d);
        let Some(top) = git.repo_root().filter(|t| !t.is_empty()) else { continue };
        let top = x.real_path(&top).unwrap_or(top);
        if out.iter().any(|(p, _, _)| *p == top) {
            continue;
        }
        add_worktrees(x, &top, &mut out);
    }
    out
}

/// The worktrees of the repositories directly inside `dir` (a folder that
/// isn't one itself: `~`, a projects folder). Hidden folders are passed
/// over, and in the home folder those an OS guards (looking into Desktop
/// or Documents is what makes macOS ask for access: never done for a
/// name); only a folder with a `.git` of its own counts.
pub fn worktrees_inside(x: &dyn Exec, dir: &str) -> Vec<Worktree> {
    const GUARDED: [&str; 9] = ["Desktop", "Documents", "Downloads", "Library", "Pictures", "Movies", "Music", "Public", "Applications"];
    let mut out: Vec<Worktree> = vec![];
    let Ok(entries) = x.list_dir(dir) else { return out };
    let home = x.home().ok().map(|h| x.real_path(&h).unwrap_or(h));
    let in_home = home.is_some_and(|h| x.real_path(dir).map_or(h.as_str() == dir, |d| d == h));
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.kind == FileKind::Dir && !e.name.starts_with('.') && !(in_home && GUARDED.contains(&e.name.as_str())))
        .map(|e| e.name)
        .collect();
    names.sort();
    for n in names {
        let child = exec::join(dir, &n);
        if !matches!(x.stat(&exec::join(&child, ".git")), Ok(Some(_))) {
            continue;
        }
        let top = x.real_path(&child).unwrap_or(child);
        if !out.iter().any(|(p, _, _)| *p == top) {
            add_worktrees(x, &top, &mut out);
        }
    }
    out
}

/// `git worktree list` of the repository at `top`, added to `out` (once
/// each).
fn add_worktrees(x: &dyn Exec, top: &str, out: &mut Vec<Worktree>) {
    let Ok(list) = crate::vcs::git::Git::new(x, top).worktree_list() else { return };
    for e in list.into_iter().filter(|e| !e.bare) {
        let path = x.real_path(&e.path).unwrap_or(e.path);
        if out.iter().any(|(p, _, _)| *p == path) {
            continue;
        }
        let name = path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
        out.push((path, name, e.branch));
    }
}

/// How a worktree is listed in an answer: its folder's name and branch.
fn listed(w: &Worktree) -> String {
    match &w.2 {
        Some(b) if *b != w.1 => format!("{} ({b})", w.1),
        _ => w.1.clone(),
    }
}

/// `req`'s files and folder as full, resolved paths that exist on the
/// agent's machine (those that don't are `Skipped`, with why). Errors: the
/// request as a whole can't be done (no such root, nothing asked).
pub fn resolve(place: &Place, agent_id: &str, req: &ViewerOpen) -> Result<(ViewerShow, Vec<Skipped>), String> {
    let x = place.exec;
    let home = x.home().ok();
    let home = home.as_deref();
    // Elsewhere: only inside the agent's folder.
    let fence = (!place.local).then(|| x.real_path(place.cwd).unwrap_or_else(|_| place.cwd.to_string()));
    let fenced = |p: &str| fence.as_deref().is_some_and(|root| !crate::explorer::inside(root, p));
    // Where the agent works now (a removed worktree: said, not guessed past).
    let now = match place.now.map(str::trim).filter(|n| !n.is_empty() && *n != place.cwd) {
        None => None,
        Some(n) => match x.real_path(n) {
            Ok(r) if fenced(&r) => None,
            Ok(r) => Some(r),
            Err(_) => {
                return Err(format!(
                    "the folder this agent works in now ({n}) is gone (a removed worktree?): give root to show another folder"
                ))
            }
        },
    };
    let asked = req.root.as_deref().map(str::trim).filter(|r| !r.is_empty());
    let from = req.cwd.as_deref().filter(|c| absolute(c)).or(now.as_deref()).unwrap_or(place.cwd);
    // What the answer says besides (how a name was read, why a root).
    let mut notes: Vec<String> = vec![];
    let root = match asked {
        None => None,
        Some(r) => {
            let (path, note) = root_of(place, home, from, r)?;
            notes.extend(note);
            Some(path)
        }
    };
    // Relative paths: from the root asked for, else the caller's folder,
    // else where the agent works now.
    let base = root.clone().unwrap_or_else(|| from.to_string());
    let mut skipped = vec![];
    let mut files = vec![];
    for spec in req.files.iter().filter(|s| !s.trim().is_empty()) {
        let (path, line, column) = split_position(spec);
        // A name with a colon and digits in it: the whole text first.
        let whole = (line.is_some()).then(|| full(home, &base, spec)).and_then(|p| x.real_path(&p).ok());
        let (real, line, column) = match whole {
            Some(r) if !is_dir(x, &r) => (Ok(r), None, None),
            _ => (x.real_path(&full(home, &base, &path)).map_err(|_| "not found".to_string()), line, column),
        };
        match real {
            Ok(r) if is_dir(x, &r) => skipped.push(Skipped { path: spec.clone(), reason: "is a folder (show it as a folder)".into() }),
            Ok(r) if fenced(&r) => skipped.push(Skipped { path: spec.clone(), reason: "outside the agent's folder, on another machine".into() }),
            Ok(r) => files.push(OpenedFile { path: r, line, column }),
            Err(why) => skipped.push(Skipped { path: spec.clone(), reason: why }),
        }
    }
    let wants_folder = req.folder.as_deref().map(str::trim).filter(|f| !f.is_empty());
    let folder = match (wants_folder, &root) {
        (Some(f), _) => {
            let p = full(home, &base, f);
            match x.real_path(&p) {
                Ok(r) if is_dir(x, &r) => Some(r),
                Ok(_) => return Err(format!("{f} is a file, not a folder (show it as a file)")),
                Err(_) => return Err(format!("{f}: not found")),
            }
        }
        // Only a root: that folder (the source of worktree X).
        (None, Some(r)) if req.files.is_empty() => Some(r.clone()),
        _ => None,
    };
    // No root asked: the worktree it works in now, when that isn't the
    // one it started in (its tree and git state are that worktree's).
    let root = match root {
        Some(r) => Some(r),
        None => {
            let moved = moved_to(x, place.cwd, &base).filter(|r| !fenced(r));
            // A folder asked for by its path, in another git worktree than
            // the one shown: rooted at that worktree (and revealed in it).
            let shown = moved.as_deref().unwrap_or(place.cwd);
            let by_folder = folder.as_deref().filter(|_| wants_folder.is_some()).and_then(|f| other_worktree(x, shown, f).map(|t| (f, t))).filter(|(_, t)| !fenced(t));
            match by_folder {
                Some((f, t)) => {
                    if f != t {
                        notes.push(format!("{f} is in the git worktree {t}: the viewer is rooted there, with that folder open."));
                    }
                    Some(t)
                }
                // A folder outside the worktree it works in: as asked.
                None => moved.filter(|m| folder.as_deref().is_none_or(|f| crate::explorer::inside(m, f))),
            }
        }
    };
    if let Some(f) = folder.as_deref().filter(|f| fenced(f)) {
        return Err(format!("{f} is outside the agent's folder, on another machine: only its own folder can be shown"));
    }
    if files.is_empty() && folder.is_none() {
        return Err(match skipped.as_slice() {
            [] => "nothing to show: give files or a folder".into(),
            s => format!(
                "nothing was opened: {}",
                s.iter().map(|s| format!("{} ({})", s.path, s.reason)).collect::<Vec<_>>().join(", ")
            ),
        });
    }
    let note = (!notes.is_empty()).then(|| notes.join(" "));
    let show = ViewerShow { agent_id: agent_id.to_string(), files, folder, root, place: req.place, of: None, note };
    Ok((show, skipped))
}

fn is_dir(x: &dyn Exec, p: &str) -> bool {
    matches!(x.stat(p), Ok(Some(s)) if s.kind == FileKind::Dir)
}

/// The top of the git worktree holding `dir` (resolved), else `dir`.
fn worktree_top(x: &dyn Exec, dir: &str) -> String {
    let real = x.real_path(dir).unwrap_or_else(|_| dir.trim_end_matches('/').to_string());
    crate::vcs::git::Git::new(x, &real)
        .repo_root()
        .filter(|t| !t.is_empty())
        .map_or(real, |t| x.real_path(&t).unwrap_or(t))
}

/// The top of the git worktree holding `dir` (resolved); `None` outside
/// git.
fn git_top(x: &dyn Exec, dir: &str) -> Option<String> {
    crate::vcs::git::Git::new(x, dir).repo_root().filter(|t| !t.is_empty()).map(|t| x.real_path(&t).unwrap_or(t))
}

/// The git worktree `here` is in, when the viewer showing `shown` doesn't
/// show it as its own: `None` outside git, and for a folder inside
/// `shown` when `shown` is in a repository (it is that worktree's, or a
/// repository nested in it, revealed there). Inside a folder that isn't a
/// repository (`~`, a projects folder): the worktree.
fn other_worktree(x: &dyn Exec, shown: &str, here: &str) -> Option<String> {
    let here_real = x.real_path(here).ok()?;
    let top = git_top(x, &here_real)?;
    let shown_real = x.real_path(shown).unwrap_or_else(|_| shown.trim_end_matches('/').to_string());
    let shown_top = git_top(x, &shown_real);
    if shown_top.is_some() && crate::explorer::inside(&shown_real, &here_real) {
        return None;
    }
    (shown_top.as_deref() != Some(top.as_str())).then_some(top)
}

/// The folder to show for an agent that started in `start` and works in
/// `here` now: the top of `here`'s worktree when that isn't `start`'s
/// (also when it lies inside `start`, a folder that isn't a repository: an
/// agent started in `~` that works in a repository below it); `None` while
/// it works in `start`'s worktree or a plain folder inside `start`;
/// `here` itself outside git and outside `start`.
fn moved_to(x: &dyn Exec, start: &str, here: &str) -> Option<String> {
    if let Some(top) = other_worktree(x, start, here) {
        return Some(top);
    }
    let start_real = x.real_path(start).unwrap_or_else(|_| start.trim_end_matches('/').to_string());
    let here_real = x.real_path(here).ok()?;
    if crate::explorer::inside(&start_real, &here_real) {
        return None;
    }
    let top = worktree_top(x, &here_real);
    (top != worktree_top(x, &start_real)).then_some(top)
}

/// The worktrees in `list` named `r`: by folder name or branch, any case.
fn named<'w>(list: &'w [Worktree], r: &str) -> Vec<&'w Worktree> {
    list.iter().filter(|(_, name, branch)| name.eq_ignore_ascii_case(r) || branch.as_deref().is_some_and(|b| b.eq_ignore_ascii_case(r))).collect()
}

/// The folder `r` names: a path (from `from`: the caller's folder, else
/// where the agent works now), else a worktree by folder name or branch:
/// of the agent's repository (and the one it works in, and the caller's),
/// else of a repository directly inside the agent's folder when that
/// isn't one ([`Place::inside`]). With a note saying which worktree a
/// name was read as.
fn root_of(place: &Place, home: Option<&str>, from: &str, r: &str) -> Result<(String, Option<String>), String> {
    let x = place.exec;
    if let Ok(real) = x.real_path(&full(home, from, r)) {
        if is_dir(x, &real) {
            return Ok((real, None));
        }
        return Err(format!("{r} is a file, not a folder"));
    }
    // A full path that isn't there: no name to look for.
    if absolute(&expand_tilde_in(home, r)) {
        return Err(format!("{r}: no such folder (a removed worktree?)"));
    }
    let near = named(place.worktrees, r);
    let (hits, among) = if near.is_empty() { (named(place.inside, r), true) } else { (near, false) };
    match hits.as_slice() {
        [one] => {
            let path = x.real_path(&one.0).map_err(|_| format!("worktree {r} ({}) is gone", one.0))?;
            let whose = if among { format!(", a repository in {}", place.cwd) } else { String::new() };
            let branch = one.2.as_deref().map_or_else(|| " (detached)".to_string(), |b| format!(" on branch {b}"));
            Ok((path.clone(), Some(format!("\"{r}\" is the worktree {path}{branch}{whose}."))))
        }
        [] => {
            let all: Vec<String> = place.worktrees.iter().chain(place.inside.iter()).map(listed).collect();
            Err(if all.is_empty() {
                format!("no folder or worktree \"{r}\": it isn't a path from {from}, and no git repository holds the agent's folder ({}) or lies directly inside it; give the worktree's path as root", place.cwd)
            } else {
                format!("no folder or worktree \"{r}\" (a path, or a worktree by folder name or branch). Worktrees: {}", all.join(", "))
            })
        }
        more => Err(format!(
            "{} worktrees match \"{r}\": give the path of the one meant ({})",
            more.len(),
            more.iter().map(|w| format!("{}{}", w.0, w.2.as_deref().map(|b| format!(" on {b}")).unwrap_or_default())).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// [`resolve`] for an agent of `engine`: its machine, folder and where it
/// works now, and its repository's worktrees. Blocking (asks the machine).
pub fn resolve_for(engine: &Engine, agent_id: &str, req: &ViewerOpen) -> Result<(ViewerShow, Vec<Skipped>), String> {
    crate::exec::assert_off_ui("show::resolve_for");
    let (cwd, can_read, local) = engine.with(agent_id, |a| (a.rec.cwd.clone(), a.facts.provider.exec, a.facts.provider.local_process))?;
    if !can_read {
        return Err("Pitwall can't read files where this agent runs".into());
    }
    let x = engine.exec_for(agent_id);
    let now = working_folder(engine, agent_id);
    let (worktrees, inside) = names_for(&*x, &cwd, now.as_deref(), req);
    resolve(&Place { exec: &*x, cwd: &cwd, now: now.as_deref(), worktrees: &worktrees, inside: &inside, local }, agent_id, req)
}

/// The worktrees a root that is a name may mean, for an agent started in
/// `cwd` working in `now` ([`Place::worktrees`], [`Place::inside`]):
/// asked of git only for a root that isn't a path there, and the
/// repositories inside the agent's folder only when the nearer ones have
/// none of that name. Blocking.
pub fn names_for(x: &dyn Exec, cwd: &str, now: Option<&str>, req: &ViewerOpen) -> (Vec<Worktree>, Vec<Worktree>) {
    let Some(r) = req.root.as_deref().map(str::trim).filter(|r| !r.is_empty()) else { return (vec![], vec![]) };
    let home = x.home().ok();
    let from = req.cwd.as_deref().filter(|c| absolute(c)).or(now).unwrap_or(cwd);
    if absolute(&expand_tilde_in(home.as_deref(), r)) || x.real_path(&full(home.as_deref(), from, r)).is_ok() {
        return (vec![], vec![]);
    }
    let dirs: Vec<&str> = [Some(cwd), now, req.cwd.as_deref().filter(|c| absolute(c))].into_iter().flatten().collect();
    let near = worktrees_near(x, &dirs);
    let inside = if named(&near, r).is_empty() && git_top(x, cwd).is_none() { worktrees_inside(x, cwd) } else { vec![] };
    (near, inside)
}

/// Where agent `id` works now, when Pitwall knows: the folder its latest
/// hook reported, else the newest one it said it works in (`pitwall agent
/// track`), else its process's (where the provider can tell: a shell the
/// user `cd`s in). `None`: only the folder it started in is known.
/// Blocking (may ask the machine).
pub fn working_folder(engine: &Engine, id: &str) -> Option<String> {
    let (hook, told, ask) = engine
        .with(id, |a| {
            let pid = a.inner.as_ref().map(|i| i.pid).filter(|_| a.facts.provider.local_process).or_else(|| a.local_pid());
            let ask = (a.facts.provider.process_cwd && a.running()).then(|| (a.rec.locator(), pid));
            (a.hook_cwd.clone(), a.rec.tracked.first().cloned(), ask)
        })
        .ok()?;
    hook.or(told).or_else(|| {
        let (loc, pid) = ask?;
        engine.providers().for_locator(&loc).ok()?.process_cwd(&loc, pid).ok().flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeExec;

    #[test]
    fn positions_come_off_the_end() {
        assert_eq!(split_position("src/a.rs"), ("src/a.rs".into(), None, None));
        assert_eq!(split_position("src/a.rs:42"), ("src/a.rs".into(), Some(42), None));
        assert_eq!(split_position(" src/a.rs:42:7 "), ("src/a.rs".into(), Some(42), Some(7)));
        assert_eq!(split_position("C:\\x\\a.rs:3"), ("C:\\x\\a.rs".into(), Some(3), None));
        assert_eq!(split_position("a.rs:x"), ("a.rs:x".into(), None, None));
        assert_eq!(split_position(":12"), (":12".into(), None, None));
        assert_eq!(split_position("12:5"), ("12".into(), Some(5), None));
    }

    fn fake() -> std::sync::Arc<FakeExec> {
        let x = FakeExec::new();
        x.dir("/w/api")
            .file("/w/api/src/a.rs", b"fn a() {}\n")
            .file("/w/api/README.md", b"# api\n")
            .dir("/w/api/src")
            .dir("/w/api-fix")
            .file("/w/api-fix/b.rs", b"b\n")
            .file("/w/api/odd:12", b"odd\n");
        x
    }

    fn req(files: &[&str]) -> ViewerOpen {
        ViewerOpen { files: files.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn files_are_found_from_the_agents_folder() {
        let x = fake();
        let wt = vec![("/w/api-fix".to_string(), "api-fix".to_string(), Some("fix/login".to_string()))];
        let place = Place { exec: &*x, cwd: "/w/api", now: None, worktrees: &wt, inside: &[], local: true };
        let (show, skipped) = resolve(&place, "a1", &req(&["src/a.rs:3:2", "/w/api/README.md", "nope.rs", "src", "odd:12"])).unwrap();
        let paths: Vec<_> = show.files.iter().map(|f| (f.path.as_str(), f.line, f.column)).collect();
        assert_eq!(paths, vec![("/w/api/src/a.rs", Some(3), Some(2)), ("/w/api/README.md", None, None), ("/w/api/odd:12", None, None)]);
        assert_eq!(skipped.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), vec!["nope.rs", "src"]);
        assert!(skipped[1].reason.contains("folder"));
        assert_eq!(show.folder, None);
    }

    #[test]
    fn a_root_is_a_path_or_a_worktree_name() {
        let x = fake();
        let wt = vec![("/w/api-fix".to_string(), "api-fix".to_string(), Some("fix/login".to_string()))];
        let place = Place { exec: &*x, cwd: "/w/api", now: None, worktrees: &wt, inside: &[], local: true };
        // "Show me the source of worktree fix/login": its folder as a tree.
        let r = ViewerOpen { root: Some("fix/login".into()), ..Default::default() };
        assert_eq!(resolve(&place, "a1", &r).unwrap().0.folder.as_deref(), Some("/w/api-fix"));
        let r = ViewerOpen { root: Some("api-fix".into()), files: vec!["b.rs".into()], ..Default::default() };
        let (show, _) = resolve(&place, "a1", &r).unwrap();
        assert_eq!((show.files[0].path.as_str(), show.folder), ("/w/api-fix/b.rs", None));
        let r = ViewerOpen { root: Some("/w/api-fix".into()), ..Default::default() };
        assert_eq!(resolve(&place, "a1", &r).unwrap().0.folder.as_deref(), Some("/w/api-fix"));
        let r = ViewerOpen { root: Some("nowhere".into()), ..Default::default() };
        assert!(resolve(&place, "a1", &r).unwrap_err().contains("no folder or worktree"));
    }

    #[test]
    fn folders_and_nothing_at_all() {
        let x = fake();
        let place = Place { exec: &*x, cwd: "/w/api", now: None, worktrees: &[], inside: &[], local: true };
        let r = ViewerOpen { folder: Some("src".into()), place: Some(pitwall_proto::ViewerPlace::Window), ..Default::default() };
        let (show, _) = resolve(&place, "a1", &r).unwrap();
        assert_eq!((show.folder.as_deref(), show.place), (Some("/w/api/src"), Some(pitwall_proto::ViewerPlace::Window)));
        let r = ViewerOpen { folder: Some("README.md".into()), ..Default::default() };
        assert!(resolve(&place, "a1", &r).unwrap_err().contains("is a file"));
        assert!(resolve(&place, "a1", &ViewerOpen::default()).unwrap_err().contains("nothing to show"));
        let e = resolve(&place, "a1", &req(&["gone.md"])).unwrap_err();
        assert!(e.contains("gone.md (not found)"), "{e}");
        // A shell's folder for relative paths.
        let r = ViewerOpen { files: vec!["a.rs".into()], cwd: Some("/w/api/src".into()), ..Default::default() };
        assert_eq!(resolve(&place, "a1", &r).unwrap().0.files[0].path, "/w/api/src/a.rs");
    }

    #[test]
    fn elsewhere_only_the_agents_own_folder() {
        let x = fake();
        let place = Place { exec: &*x, cwd: "/w/api", now: None, worktrees: &[], inside: &[], local: false };
        let (show, skipped) = resolve(&place, "a1", &req(&["src/a.rs", "/w/api-fix/b.rs"])).unwrap();
        assert_eq!(show.files.len(), 1);
        assert!(skipped[0].reason.contains("another machine"));
        let r = ViewerOpen { folder: Some("/w/api-fix".into()), ..Default::default() };
        assert!(resolve(&place, "a1", &r).unwrap_err().contains("another machine"));
    }

    /// `fake()` with git: /w/api is the main checkout, /w/api-fix a linked
    /// worktree, /plain no repository at all.
    fn with_git() -> std::sync::Arc<FakeExec> {
        let x = fake();
        x.dir("/w/api-fix/sub").file("/w/api-fix/sub/c.rs", b"c\n").dir("/plain").file("/plain/notes.md", b"n\n");
        for (dir, top) in [("/w/api", "/w/api"), ("/w/api/src", "/w/api"), ("/w/api-fix", "/w/api-fix"), ("/w/api-fix/sub", "/w/api-fix")] {
            x.on(&["git", "-C", dir, "rev-parse", "--show-toplevel"], &format!("{top}\n"));
        }
        x.on_exit(&["git", "-C", "/plain", "rev-parse", "--show-toplevel"], 128, "", "fatal: not a git repository");
        x
    }

    #[test]
    fn the_worktree_it_works_in_now_is_shown() {
        let x = with_git();
        // Started in /w/api, now in a folder of worktree /w/api-fix.
        let place = Place { exec: &*x, cwd: "/w/api", now: Some("/w/api-fix/sub"), worktrees: &[], inside: &[], local: true };
        let (show, _) = resolve(&place, "a1", &req(&["c.rs"])).unwrap();
        // Relative paths from that folder itself; the tree is its worktree's.
        assert_eq!(show.files[0].path, "/w/api-fix/sub/c.rs");
        assert_eq!(show.root.as_deref(), Some("/w/api-fix"));
        let r = ViewerOpen { folder: Some("sub".into()), ..Default::default() };
        let (show, _) = resolve(&Place { now: Some("/w/api-fix"), ..place }, "a1", &r).unwrap();
        assert_eq!((show.folder.as_deref(), show.root.as_deref()), (Some("/w/api-fix/sub"), Some("/w/api-fix")));
        // Still in the worktree it started in: as before.
        let place = Place { now: Some("/w/api/src"), ..place };
        let (show, _) = resolve(&place, "a1", &req(&["a.rs"])).unwrap();
        assert_eq!((show.files[0].path.as_str(), show.root), ("/w/api/src/a.rs", None));
        let place = Place { now: None, ..place };
        assert_eq!(resolve(&place, "a1", &req(&["src/a.rs"])).unwrap().0.root, None);
        // A folder outside git: that folder.
        let place = Place { now: Some("/plain"), ..place };
        let (show, _) = resolve(&place, "a1", &req(&["notes.md"])).unwrap();
        assert_eq!((show.files[0].path.as_str(), show.root.as_deref()), ("/plain/notes.md", Some("/plain")));
        // A shell's folder (`pitwall open`) is where it is.
        let place = Place { now: None, ..place };
        let r = ViewerOpen { files: vec!["c.rs".into()], cwd: Some("/w/api-fix/sub".into()), ..Default::default() };
        assert_eq!(resolve(&place, "a1", &r).unwrap().0.root.as_deref(), Some("/w/api-fix"));
    }

    /// Started in a folder that isn't a repository (`~`, a projects
    /// folder), working in a repository below it: that worktree is shown;
    /// a folder asked for in another worktree roots the viewer there.
    #[test]
    fn a_repository_below_a_plain_folder_is_its_own_root() {
        let x = with_git();
        x.on_exit(&["git", "-C", "/w", "rev-parse", "--show-toplevel"], 128, "", "fatal: not a git repository");
        let place = Place { exec: &*x, cwd: "/w", now: Some("/w/api/src"), worktrees: &[], inside: &[], local: true };
        let (show, _) = resolve(&place, "a1", &req(&["a.rs"])).unwrap();
        assert_eq!((show.files[0].path.as_str(), show.root.as_deref()), ("/w/api/src/a.rs", Some("/w/api")));
        // A folder by its path in another worktree: rooted there.
        let place = Place { now: None, ..place };
        let r = ViewerOpen { folder: Some("/w/api-fix/sub".into()), ..Default::default() };
        let (show, _) = resolve(&place, "a1", &r).unwrap();
        assert_eq!((show.folder.as_deref(), show.root.as_deref()), (Some("/w/api-fix/sub"), Some("/w/api-fix")));
        assert!(show.note.unwrap_or_default().contains("is in the git worktree /w/api-fix"));
        // Started in the repository: a folder of it is revealed, no root.
        let place = Place { cwd: "/w/api", ..place };
        let r = ViewerOpen { folder: Some("/w/api/src".into()), ..Default::default() };
        assert_eq!(resolve(&place, "a1", &r).unwrap().0.root, None);
        // Names: the nearer worktrees first, then those inside the folder.
        let near = vec![("/w/api".to_string(), "api".to_string(), Some("main".to_string()))];
        let inside = vec![("/w/api-fix".to_string(), "api-fix".to_string(), Some("fix/login".to_string()))];
        let place = Place { cwd: "/w", worktrees: &near, inside: &inside, ..place };
        let r = ViewerOpen { root: Some("main".into()), ..Default::default() };
        assert_eq!(resolve(&place, "a1", &r).unwrap().0.root.as_deref(), Some("/w/api"));
        // (A name that is a path from the folder is that path.)
        let r = ViewerOpen { root: Some("fix/login".into()), ..Default::default() };
        let (show, _) = resolve(&place, "a1", &r).unwrap();
        assert_eq!(show.root.as_deref(), Some("/w/api-fix"));
        assert!(show.note.unwrap_or_default().contains("a repository in /w"));
        let r = ViewerOpen { root: Some("nope".into()), ..Default::default() };
        let e = resolve(&place, "a1", &r).unwrap_err();
        assert!(e.contains("Worktrees: api (main), api-fix (fix/login)"), "{e}");
    }

    #[test]
    fn a_root_asked_for_beats_where_it_works_now() {
        let x = with_git();
        let wt = vec![
            ("/w/api".to_string(), "api".to_string(), Some("main".to_string())),
            ("/w/api-fix".to_string(), "api-fix".to_string(), Some("fix/login".to_string())),
        ];
        let place = Place { exec: &*x, cwd: "/w/api", now: Some("/w/api-fix"), worktrees: &wt, inside: &[], local: true };
        // By path, by folder name, by branch.
        for root in ["/w/api", "api", "main"] {
            let r = ViewerOpen { root: Some(root.into()), files: vec!["README.md".into()], ..Default::default() };
            let (show, _) = resolve(&place, "a1", &r).unwrap();
            assert_eq!((show.files[0].path.as_str(), show.root.as_deref()), ("/w/api/README.md", Some("/w/api")), "{root}");
        }
        // A relative root: from where it works now.
        let r = ViewerOpen { root: Some("sub".into()), files: vec!["c.rs".into()], ..Default::default() };
        assert_eq!(resolve(&place, "a1", &r).unwrap().0.files[0].path, "/w/api-fix/sub/c.rs");
    }

    #[test]
    fn a_removed_worktree_is_said_plainly() {
        let x = with_git();
        let wt = vec![("/w/old".to_string(), "old".to_string(), Some("old-branch".to_string()))];
        let place = Place { exec: &*x, cwd: "/w/api", now: Some("/w/old"), worktrees: &wt, inside: &[], local: true };
        let e = resolve(&place, "a1", &req(&["a.rs"])).unwrap_err();
        assert!(e.contains("(/w/old) is gone") && e.contains("root"), "{e}");
        let place = Place { now: None, ..place };
        let r = ViewerOpen { root: Some("/w/old".into()), ..Default::default() };
        assert!(resolve(&place, "a1", &r).unwrap_err().contains("no such folder"));
        let r = ViewerOpen { root: Some("old-branch".into()), ..Default::default() };
        assert!(resolve(&place, "a1", &r).unwrap_err().contains("worktree old-branch (/w/old) is gone"));
    }
}
