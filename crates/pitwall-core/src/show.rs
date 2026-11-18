//! Agents showing files to the user (`agents.showFiles`, `viewer.open`):
//! whether it is on, the MCP server the agents Pitwall starts are given,
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

/// What the engine keeps: the setting, and the `pitwall-cli` to run.
pub(crate) struct State {
    on: AtomicBool,
    cli: RwLock<Option<PathBuf>>,
}

impl Default for State {
    fn default() -> State {
        // On by default, as the setting.
        State { on: AtomicBool::new(true), cli: RwLock::new(None) }
    }
}

impl Engine {
    /// `agents.showFiles`, as the host follows it.
    pub fn set_show_files(&self, on: bool) {
        self.show.on.store(on, Ordering::Relaxed);
        self.sync_show_rule();
    }

    /// The built-in rule for agents without an MCP way, in step.
    fn sync_show_rule(&self) {
        let cli = self.show_files_wiring().map(|m| m.command);
        crate::rules::sync_show_files_rule(&crate::rules::Dirs::of(self.paths()), cli.as_deref());
    }

    pub fn show_files_on(&self) -> bool {
        self.show.on.load(Ordering::Relaxed)
    }

    /// The `pitwall-cli` the host ships (`None`: this build has none, and
    /// agents get no MCP server).
    pub fn set_cli_bin(&self, cli: Option<PathBuf>) {
        *self.show.cli.write().unwrap_or_else(|e| e.into_inner()) = cli;
        self.sync_show_rule();
    }

    /// The MCP server for an agent starting now: none when the setting is
    /// off or there is no CLI.
    pub fn show_files_wiring(&self) -> Option<McpWiring> {
        if !self.show_files_on() {
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
    /// The agent's folder.
    pub cwd: &'a str,
    /// Worktrees of the agent's repository: (folder, its name, branch).
    pub worktrees: &'a [(String, String, Option<String>)],
    /// The agent runs on this machine. Elsewhere only its own folder can
    /// be shown (the viewer reads other files from this disk).
    pub local: bool,
}

/// `req`'s files and folder as full, resolved paths that exist on the
/// agent's machine (those that don't are `Skipped`, with why). Errors: the
/// request as a whole can't be done (no such root, nothing asked).
pub fn resolve(place: &Place, agent_id: &str, req: &ViewerOpen) -> Result<(ViewerShow, Vec<Skipped>), String> {
    let x = place.exec;
    let home = x.home().ok();
    let home = home.as_deref();
    let root = match req.root.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        None => None,
        Some(r) => Some(root_of(place, home, req.cwd.as_deref(), r)?),
    };
    let base = root.clone().or_else(|| req.cwd.clone().filter(|c| absolute(c))).unwrap_or_else(|| place.cwd.to_string());
    // Elsewhere: only inside the agent's folder.
    let fence = (!place.local).then(|| x.real_path(place.cwd).unwrap_or_else(|_| place.cwd.to_string()));
    let fenced = |p: &str| fence.as_deref().is_some_and(|root| !crate::explorer::inside(root, p));
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
    let show = ViewerShow { agent_id: agent_id.to_string(), files, folder, root, place: req.place, of: None };
    Ok((show, skipped))
}

fn is_dir(x: &dyn Exec, p: &str) -> bool {
    matches!(x.stat(p), Ok(Some(s)) if s.kind == FileKind::Dir)
}

/// The folder `r` names: a path (from the caller's folder, else the
/// agent's), else a worktree of the agent's repository by folder name or
/// branch.
fn root_of(place: &Place, home: Option<&str>, cwd: Option<&str>, r: &str) -> Result<String, String> {
    let x = place.exec;
    let from = cwd.filter(|c| absolute(c)).unwrap_or(place.cwd);
    if let Ok(real) = x.real_path(&full(home, from, r)) {
        if is_dir(x, &real) {
            return Ok(real);
        }
        return Err(format!("{r} is a file, not a folder"));
    }
    let named: Vec<&String> = place
        .worktrees
        .iter()
        .filter(|(_, name, branch)| name.eq_ignore_ascii_case(r) || branch.as_deref().is_some_and(|b| b.eq_ignore_ascii_case(r)))
        .map(|(p, _, _)| p)
        .collect();
    match named.as_slice() {
        [one] => x.real_path(one).map_err(|_| format!("worktree {r} ({one}) is gone")),
        [] => Err(format!("no folder or worktree \"{r}\" (a path, or a worktree of this agent's repository by folder name or branch)")),
        more => Err(format!(
            "{} worktrees match \"{r}\": give its path ({})",
            more.len(),
            more.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// [`resolve`] for an agent of `engine`: its machine, folder and
/// repository's worktrees. Blocking (asks the machine).
pub fn resolve_for(engine: &Engine, agent_id: &str, req: &ViewerOpen) -> Result<(ViewerShow, Vec<Skipped>), String> {
    crate::exec::assert_off_ui("show::resolve_for");
    let (cwd, can_read, local) = engine.with(agent_id, |a| (a.rec.cwd.clone(), a.facts.provider.exec, a.facts.provider.local_process))?;
    if !can_read {
        return Err("Pitwall can't read files where this agent runs".into());
    }
    let x = engine.exec_for(agent_id);
    // Worktrees only matter for a root that isn't a path.
    let worktrees = match req.root.as_deref().filter(|r| !r.trim().is_empty()) {
        Some(_) => crate::worktrees::list(engine)
            .into_iter()
            .filter(|p| p.agent_ids.iter().any(|a| a == agent_id) || p.worktrees.iter().any(|w| w.path == cwd))
            .flat_map(|p| p.worktrees.into_iter().map(|w| (w.path, w.name, w.branch)))
            .collect(),
        None => vec![],
    };
    resolve(&Place { exec: &*x, cwd: &cwd, worktrees: &worktrees, local }, agent_id, req)
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
        let place = Place { exec: &*x, cwd: "/w/api", worktrees: &wt, local: true };
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
        let place = Place { exec: &*x, cwd: "/w/api", worktrees: &wt, local: true };
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
        let place = Place { exec: &*x, cwd: "/w/api", worktrees: &[], local: true };
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
        let place = Place { exec: &*x, cwd: "/w/api", worktrees: &[], local: false };
        let (show, skipped) = resolve(&place, "a1", &req(&["src/a.rs", "/w/api-fix/b.rs"])).unwrap();
        assert_eq!(show.files.len(), 1);
        assert!(skipped[0].reason.contains("another machine"));
        let r = ViewerOpen { folder: Some("/w/api-fix".into()), ..Default::default() };
        assert!(resolve(&place, "a1", &r).unwrap_err().contains("another machine"));
    }
}
