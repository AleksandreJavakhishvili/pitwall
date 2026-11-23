//! Updating an extension safely (docs/extensions/updates.md): one ordered
//! step, so its three moving parts (the extension's files, the plugin its
//! setup action installs in the user's agents, the server it keeps
//! running) never drift apart.
//!
//! A format 2 manifest may say how its updates go:
//!
//! ```json
//! "update": { "order": "serviceFirst", "agents": "compatible" }
//! ```
//!
//! `order`: `serviceFirst` (the default: a new server must accept old
//! clients, so the agents' setup runs last) or `agentsFirst`. `agents`:
//! `compatible` (the default: agents started before the update keep
//! working with the setup they started with) or `restart` (they stop
//! working until restarted).
//!
//! This module reads that block ([`Policy`]), lays out the steps an update
//! takes in plain words for the review ([`plan`]), names, makes and prunes
//! the backup folders ([`make_backup_folder`], [`prune_backups`]), copies
//! and puts back a service's data ([`copy_data`], [`restore_copy`],
//! [`restore_files`]) and says which setup an agent started under
//! ([`generation`]). Running the steps is the app's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::plan::{self, ToolAccount};
use super::{opt_str, Action, Facts, Manifest, SETTING_PLACEHOLDER};

/// The folder of Pitwall's data folder an update's backups go in.
pub const BACKUPS_DIR: &str = "extension-backups";
/// How many backups of one extension are kept (the newest).
pub const KEEP_BACKUPS: usize = 3;
/// The most Pitwall copies of a service's data folder itself.
pub const COPY_CAP: u64 = 2 * 1024 * 1024 * 1024;
/// How long a started service has to be up after an update.
pub const READY_TIMEOUT_SECS: u64 = 60;
/// How long the adapter has to be back as it was after an update.
pub const CONNECT_TIMEOUT_SECS: u64 = 30;
/// A backup folder's note of what it holds.
pub const BACKUP_NOTE: &str = "pitwall-backup.json";
/// In a backup folder made by copying: the copy of the data folder.
pub const COPY_DIR: &str = "data";
/// In a backup folder: what putting it back moved out of the data folder.
pub const REPLACED_DIR: &str = "replaced";

/// Which goes first: the server or the agents' setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// The service, then the agents' setup (a new server accepts old
    /// clients).
    #[default]
    ServiceFirst,
    /// The agents' setup, then the service (new clients accept an old
    /// server).
    AgentsFirst,
}

/// What agents started before an update can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Agents {
    /// They keep working with what they started with.
    #[default]
    Compatible,
    /// They stop working with it until restarted.
    Restart,
}

/// A manifest's `update` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Policy {
    pub order: Order,
    pub agents: Agents,
}

impl Policy {
    /// In plain words, for `check`.
    pub fn words(&self) -> Vec<String> {
        let order = match self.order {
            Order::ServiceFirst => "An update starts its new service first and updates its agents' setup last (a new server accepts old clients).",
            Order::AgentsFirst => "An update updates its agents' setup first, then starts its new service (new clients accept an old server).",
        };
        let agents = match self.agents {
            Agents::Compatible => "Agents started before an update keep working with what they started with until they are restarted.",
            Agents::Restart => "Agents started before an update stop working with it until they are restarted.",
        };
        vec![order.into(), agents.into()]
    }
}

/// Read `update` (format 2).
pub(super) fn parse(v: Option<&Value>) -> Result<Policy, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(Policy::default()),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("\"update\" must be an object: \"order\" and \"agents\"".into()),
    };
    if let Some(k) = o.keys().find(|k| !["order", "agents"].contains(&k.as_str())) {
        return Err(format!("\"update\" has “{k}”, which this Pitwall doesn't know (it reads order and agents)"));
    }
    let order = match opt_str(o, "order")? {
        None | Some("serviceFirst") => Order::ServiceFirst,
        Some("agentsFirst") => Order::AgentsFirst,
        Some(x) => return Err(format!("\"update.order\" is “{x}”: it must be serviceFirst or agentsFirst")),
    };
    let agents = match opt_str(o, "agents")? {
        None | Some("compatible") => Agents::Compatible,
        Some("restart") => Agents::Restart,
        Some(x) => return Err(format!("\"update.agents\" is “{x}”: it must be compatible or restart")),
    };
    Ok(Policy { order, agents })
}

/// What Pitwall does with running agents that started under an earlier
/// setup of an extension (`extensions.restartAfterUpdate`, and per
/// extension on its card).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Restart {
    /// Nothing by itself: their rows say so, with Restart.
    #[default]
    Ask,
    /// Each is restarted once it has been idle a while, one at a time.
    WhenIdle,
    /// Nothing; their rows say so.
    Never,
}

impl Restart {
    pub const ALL: [Restart; 3] = [Restart::Ask, Restart::WhenIdle, Restart::Never];

    /// Its id in `ui.json`.
    pub fn id(self) -> &'static str {
        match self {
            Restart::Ask => "ask",
            Restart::WhenIdle => "whenIdle",
            Restart::Never => "never",
        }
    }

    pub fn parse(id: &str) -> Option<Restart> {
        Restart::ALL.into_iter().find(|r| r.id() == id)
    }

    /// Its label in Settings.
    pub fn label(self) -> &'static str {
        match self {
            Restart::Ask => "Ask",
            Restart::WhenIdle => "When idle",
            Restart::Never => "Never",
        }
    }

    /// What it does, in a line.
    pub fn words(self) -> &'static str {
        match self {
            Restart::Ask => "Pitwall restarts nothing by itself: their rows and this card say so, with Restart.",
            Restart::WhenIdle => "Pitwall restarts each once it has waited for you for 30 seconds, one at a time, continuing its conversation; never one that is working or waiting on an approval, nor one that can't continue its conversation.",
            Restart::Never => "Pitwall restarts none: their rows say so.",
        }
    }
}

// ---------------------------------------------------------------- the plan

/// A step of an update, in the order it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The service stopped and its data backed up.
    Backup,
    /// The new files put in place (the old ones kept for Go back).
    Install,
    /// The service's program updated by its install action, when its
    /// version is below the one needed.
    ServiceInstall,
    /// The service started and waited for.
    ServiceStart,
    /// The service up and the adapter back as it was.
    Health,
    /// A setup action run again for every account (its id).
    Setup(String),
}

/// A step as the review says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub step: Step,
    pub title: String,
    /// Its exact commands, or what it does.
    pub lines: Vec<String>,
}

/// What the plan depends on beyond the manifests.
#[derive(Debug, Clone, Default)]
pub struct PlanInput<'a> {
    /// The extension is on (else only its files change).
    pub on: bool,
    /// Pitwall runs its service here now: confirmed, not stopped by the
    /// user, not someone else's on its address.
    pub service_here: bool,
    /// Its data folder has something in it.
    pub has_data: bool,
    /// The size of its data folder, when known (a copy over
    /// [`COPY_CAP`] isn't made).
    pub data_size: Option<u64>,
    /// Its settings (the default connection's).
    pub values: BTreeMap<String, String>,
    /// Its data folder and its backup folder, as shown.
    pub data: &'a str,
    pub backup: &'a str,
    /// Where programs are found, and the tools' accounts.
    pub path: Option<&'a str>,
    pub accounts: &'a [ToolAccount],
    /// The setup actions set up before the update (their ids).
    pub set_up: &'a [String],
    /// Running agents that use its setup now.
    pub agents: usize,
    pub restart: Restart,
}

/// The setup actions of `new` an update runs again: set up before, and
/// their steps changed.
pub fn changed_setup<'a>(old: &Manifest, new: &'a Manifest, set_up: &[String]) -> Vec<&'a Action> {
    new.actions
        .iter()
        .filter(|a| a.setup && set_up.contains(&a.id))
        .filter(|a| old.actions.iter().find(|o| o.id == a.id).is_none_or(|o| super::steps_digest(o) != super::steps_digest(a)))
        .collect()
}

/// The backup command an update runs: `new`'s, else `old`'s (`None`:
/// Pitwall copies the data folder).
pub fn backup_of<'a>(old: &'a Manifest, new: &'a Manifest) -> Option<(&'a Manifest, &'a super::service::Backup)> {
    let of = |m: &'a Manifest| m.service.as_ref().and_then(|s| s.backup.as_ref()).map(|b| (m, b));
    of(new).or_else(|| of(old))
}

/// Whether an update backs up `old`'s service data: it is on, Pitwall runs
/// its service here and its data folder has something.
pub fn backs_up(old: &Manifest, i: &PlanInput) -> bool {
    i.on && old.service.is_some() && i.service_here && i.has_data
}

/// Whether an update starts the service and checks it: it is on and
/// Pitwall ran it here before.
pub fn runs_service(old: &Manifest, new: &Manifest, i: &PlanInput) -> bool {
    i.on && old.service.is_some() && new.service.is_some() && i.service_here
}

/// The steps updating `old` to `new` takes, in order, with their exact
/// commands: what the review lists and the run follows.
pub fn plan(old: &Manifest, new: &Manifest, i: &PlanInput) -> Vec<Planned> {
    let mut out = Vec::new();
    let label = |m: &Manifest| m.service.as_ref().map_or_else(|| "its service".to_string(), |s| s.label.clone());
    if backs_up(old, i) {
        let (title, lines) = match backup_of(old, new) {
            Some((m, b)) => {
                let cmd = super::service::backup_command(m, b, &i.values, i.data, i.backup).unwrap_or_else(|_| b.run.clone());
                (format!("Stops {} and backs up its data first:", label(old)), vec![plan::line(&cmd)])
            }
            None => {
                let mut lines = vec![format!("{} → {}/{COPY_DIR}", i.data, i.backup)];
                if let Some(n) = i.data_size.filter(|n| *n > COPY_CAP) {
                    lines.push(format!("Its data is {}, more than Pitwall copies ({}): the update goes on without a backup only if you say so.", size_words(n), size_words(COPY_CAP)));
                }
                (format!("Stops {} and backs up its data first (Pitwall copies its data folder, up to {}):", label(old), size_words(COPY_CAP)), lines)
            }
        };
        out.push(Planned { step: Step::Backup, title, lines });
    } else if i.on && old.service.is_some() && i.service_here {
        out.push(Planned { step: Step::Backup, title: format!("Stops {} (its data folder is empty: nothing to back up).", label(old)), lines: vec![] });
    }
    let v = |m: &Manifest| m.version.clone().unwrap_or_else(|| "this copy".into());
    out.push(Planned { step: Step::Install, title: format!("Installs {} {} (the installed {} is kept for Go back).", new.name, v(new), v(old)), lines: vec![] });
    let service = runs_service(old, new, i);
    let mut service_steps = Vec::new();
    if service {
        let s = new.service.as_ref().expect("runs_service");
        if let Some(a) = s.install.as_ref().and_then(|id| new.actions.iter().find(|a| a.id == *id)) {
            let need = s.version.as_ref().and_then(|v| v.requires.clone());
            let when = match need {
                Some(n) => format!("If {} is older than {n} or isn't installed", s.label),
                None => format!("If {} isn't installed", s.label),
            };
            let lines = plan::plan(new, a, &i.values, i.path).iter().map(|p| p.line()).collect();
            service_steps.push(Planned { step: Step::ServiceInstall, title: format!("{when}, runs “{}”:", a.label), lines });
        }
        service_steps.push(Planned { step: Step::ServiceStart, title: format!("Starts {} and waits until it is up (at most {READY_TIMEOUT_SECS} s).", s.label), lines: vec![] });
    }
    let health = if service || (i.on && new.adapter.is_some()) {
        let what = match (service, new.adapter.is_some()) {
            (true, true) => format!("{} is up and {} connects as before", label(new), new.name),
            (true, false) => format!("{} is up", label(new)),
            _ => format!("{} connects as before", new.name),
        };
        let back = if service { ", its data and its service" } else { "" };
        Some(Planned {
            step: Step::Health,
            title: format!("Checks that {what} (within {CONNECT_TIMEOUT_SECS} s). If not, it puts back {} {}{back}, and says what failed.", new.name, v(old)),
            lines: vec![],
        })
    } else {
        None
    };
    let mut setup = Vec::new();
    if i.on {
        for a in changed_setup(old, new, i.set_up) {
            let planned = plan::plan_for(new, a, &i.values, i.path, i.accounts, true);
            let lines = planned
                .iter()
                .map(|p| {
                    let who = p.who().map(|w| format!("{w}: ")).unwrap_or_default();
                    let skip = p.skip.as_ref().map(|s| format!(" (skipped: {s})")).unwrap_or_default();
                    format!("{who}{}{skip}", p.line())
                })
                .collect();
            setup.push(Planned { step: Step::Setup(a.id.clone()), title: format!("Runs “{}” again for every account (its commands changed); a failure there doesn't undo the update, and shows Retry:", a.label), lines });
        }
    }
    match new.update.order {
        Order::ServiceFirst => {
            out.extend(service_steps);
            out.extend(health);
            out.extend(setup);
        }
        Order::AgentsFirst => {
            out.extend(setup);
            out.extend(service_steps);
            out.extend(health);
        }
    }
    if !i.on {
        out.push(Planned { step: Step::Install, title: "It is off: only its files change. Its service and its setup run when you turn it on.".into(), lines: vec![] });
    }
    out
}

/// What the review says about running agents (`n` of them) that use the
/// setup from before the update, and what `restart` will do; `None` when
/// none runs.
pub fn agents_lines(new: &Manifest, n: usize, restart: Restart) -> Vec<String> {
    if n == 0 {
        return vec![];
    }
    let who = if n == 1 { "1 running agent".to_string() } else { format!("{n} running agents") };
    let first = match new.update.agents {
        Agents::Compatible => format!("{who} keep{} the previous setup until restarted.", if n == 1 { "s" } else { "" }),
        Agents::Restart => format!("Risk: {who} will stop working with {} until restarted.", new.name),
    };
    vec![first, format!("After the update: {}", restart.words())]
}

/// `n` bytes in a few words ("3.1 GiB", "12 MiB").
pub fn size_words(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f >= K * K * K {
        format!("{:.1} GiB", f / (K * K * K))
    } else if f >= K * K {
        format!("{:.0} MiB", f / (K * K))
    } else if f >= K {
        format!("{:.0} KiB", f / K)
    } else {
        format!("{n} bytes")
    }
}

// ---------------------------------------------------------------- backups

/// A time as a backup folder's name: `YYYYMMDD-HHMMSS` (UTC), so names
/// sort as times do.
pub fn stamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs = unix_secs % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
}

/// Make a new, empty backup folder for extension `id` in `backups` (its
/// `extension-backups/<id>` folder) named after `unix_secs`, private to
/// the user (mode 700, the folders above it too). A name taken gets `-2`,
/// `-3`, ….
pub fn make_backup_folder(backups: &Path, unix_secs: u64) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(backups)?;
    if let Some(root) = backups.parent() {
        crate::platform::make_private(root)?;
    }
    crate::platform::make_private(backups)?;
    let base = stamp(unix_secs);
    for n in 1..1000 {
        let name = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let dir = backups.join(name);
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                crate::platform::make_private(&dir)?;
                return Ok(dir);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many backups at the same second"))
}

/// The backup folders in `backups`, oldest first.
pub fn backups(backups: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(backups).into_iter().flatten().flatten().filter(|e| e.path().is_dir() && e.file_name().to_string_lossy().starts_with(|c: char| c.is_ascii_digit())).map(|e| e.path()).collect();
    list.sort();
    list
}

/// Delete all but the newest `keep` backup folders in `backups`; the ones
/// deleted.
pub fn prune_backups(backups_dir: &Path, keep: usize) -> Vec<PathBuf> {
    let list = backups(backups_dir);
    let gone: Vec<PathBuf> = list[..list.len().saturating_sub(keep)].to_vec();
    for d in &gone {
        let _ = std::fs::remove_dir_all(d);
    }
    gone
}

/// What a backup folder holds, noted in it ([`BACKUP_NOTE`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// The extension's version it was taken before, and the one updated
    /// to.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Unix seconds.
    pub at: u64,
    /// Made by copying the data folder (in [`COPY_DIR`]), else by the
    /// service's own backup command.
    pub copy: bool,
    /// For the service's own backup: the files in the data folder it
    /// stands for (empty: Pitwall can't put it back by itself).
    pub replaces: Vec<String>,
}

impl Note {
    pub fn write(&self, folder: &Path) -> std::io::Result<()> {
        let v = serde_json::json!({ "from": self.from, "to": self.to, "at": self.at, "copy": self.copy, "replaces": self.replaces });
        std::fs::write(folder.join(BACKUP_NOTE), serde_json::to_string_pretty(&v).unwrap_or_default())
    }

    pub fn read(folder: &Path) -> Option<Note> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(folder.join(BACKUP_NOTE)).ok()?).ok()?;
        Some(Note {
            from: v["from"].as_str().map(str::to_string),
            to: v["to"].as_str().map(str::to_string),
            at: v["at"].as_u64().unwrap_or(0),
            copy: v["copy"].as_bool().unwrap_or(false),
            replaces: v["replaces"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        })
    }

    /// Whether Pitwall can put it back by itself.
    pub fn restorable(&self) -> bool {
        self.copy || !self.replaces.is_empty()
    }
}

/// How big folder `dir` is: its files' sizes (links not followed).
pub fn dir_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let t = e.file_type()?;
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() {
                total = total.saturating_add(e.metadata()?.len());
            }
        }
    }
    Ok(total)
}

/// Copy folder `from` to `to` (made, mode 700): files, folders and links
/// as links.
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    crate::platform::make_private(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let t = e.file_type()?;
        if t.is_symlink() {
            crate::platform::copy_symlink(&src, &dst)?;
        } else if t.is_dir() {
            copy_tree(&src, &dst)?;
        } else if t.is_file() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Why a copy wasn't made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyRefused {
    /// The data folder is bigger than the cap (its size).
    TooBig(u64),
    Failed(String),
}

/// Copy service data folder `data` into backup folder `backup` (its
/// [`COPY_DIR`]), unless it is bigger than `cap`: its size.
pub fn copy_data(data: &Path, backup: &Path, cap: u64) -> Result<u64, CopyRefused> {
    let size = dir_size(data).map_err(|e| CopyRefused::Failed(format!("Its data folder can't be read: {e}")))?;
    if size > cap {
        return Err(CopyRefused::TooBig(size));
    }
    copy_tree(data, &backup.join(COPY_DIR)).map_err(|e| CopyRefused::Failed(format!("Its data folder couldn't be copied: {e}")))?;
    Ok(size)
}

/// Put a copied backup (`backup`, made by [`copy_data`]) back as data
/// folder `data`: what is there now is moved into the backup's
/// [`REPLACED_DIR`] (never deleted), then the copy is copied in. On a
/// failure, what was there is put back.
pub fn restore_copy(backup: &Path, data: &Path) -> Result<(), String> {
    let copy = backup.join(COPY_DIR);
    if !copy.is_dir() {
        return Err(format!("The backup at {} has no copy of the data folder.", backup.display()));
    }
    let aside = fresh_replaced(backup)?;
    if data.exists() {
        std::fs::rename(data, &aside).map_err(|e| format!("Its data folder couldn't be moved aside: {e}"))?;
    }
    if let Err(e) = copy_tree(&copy, data) {
        let _ = std::fs::remove_dir_all(data);
        let _ = std::fs::rename(&aside, data);
        return Err(format!("The backup couldn't be copied back: {e}"));
    }
    Ok(())
}

/// Put a service's own backup (`backup`, holding the files its command
/// wrote) back into data folder `data`: each of `replaces` there is moved
/// into the backup's [`REPLACED_DIR`] (never deleted), then each file the
/// backup holds is copied in, mode 600. On a failure, what was moved is put
/// back.
pub fn restore_files(backup: &Path, data: &Path, replaces: &[String]) -> Result<(), String> {
    if replaces.is_empty() {
        return Err(format!("Pitwall can't put this backup back by itself: it is at {}.", backup.display()));
    }
    let files: Vec<PathBuf> = std::fs::read_dir(backup)
        .map_err(|e| format!("The backup can't be read: {e}"))?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && e.file_name() != BACKUP_NOTE)
        .map(|e| e.path())
        .collect();
    if files.is_empty() {
        return Err(format!("The backup at {} holds no files.", backup.display()));
    }
    let aside = fresh_replaced(backup)?;
    std::fs::create_dir_all(&aside).map_err(|e| format!("Can't make {}: {e}", aside.display()))?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let undo = |moved: &[(PathBuf, PathBuf)]| {
        for (from, to) in moved.iter().rev() {
            let _ = std::fs::rename(to, from);
        }
    };
    for name in replaces {
        let from = data.join(name);
        if from.exists() {
            let to = aside.join(name);
            if let Err(e) = std::fs::rename(&from, &to) {
                undo(&moved);
                return Err(format!("{name} couldn't be moved aside: {e}"));
            }
            moved.push((from, to));
        }
    }
    std::fs::create_dir_all(data).map_err(|e| format!("Can't make {}: {e}", data.display()))?;
    for f in &files {
        let to = data.join(f.file_name().unwrap_or_default());
        if let Err(e) = std::fs::copy(f, &to).and_then(|_| crate::platform::make_private_file(&to)) {
            for g in &files {
                let _ = std::fs::remove_file(data.join(g.file_name().unwrap_or_default()));
            }
            undo(&moved);
            return Err(format!("The backup couldn't be copied back: {e}"));
        }
    }
    Ok(())
}

/// A new place in `backup` for what putting it back moves aside.
fn fresh_replaced(backup: &Path) -> Result<PathBuf, String> {
    for n in 1..1000 {
        let p = backup.join(if n == 1 { REPLACED_DIR.to_string() } else { format!("{REPLACED_DIR}-{n}") });
        if !p.exists() {
            return Ok(p);
        }
    }
    Err("The backup folder is full of earlier restores.".into())
}

// ---------------------------------------------------------------- agents

/// What a run of setup action `a` is kept under for connection `conn`:
/// `<action>@<connection>` when its steps use a setting, else its id (as
/// the app keeps them in `ui.json`).
pub fn run_key(a: &Action, conn: Option<&str>) -> String {
    let uses = a.steps.iter().any(|s| s.run.iter().any(|x| x.contains(SETTING_PLACEHOLDER)));
    match conn.filter(|_| uses) {
        Some(c) => format!("{}@{c}", a.id),
        None => a.id.clone(),
    }
}

/// An extension that is on, as agents take it: its manifest and the
/// digest of the steps each of its setup actions last ran with
/// (`extensions.<id>.actionSteps`: what is installed in the agents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub manifest: Manifest,
    pub steps: BTreeMap<String, String>,
}

/// The setup an agent with `facts` (its connection `conn`) starts under
/// with extension `s`: a digest of its setup actions' steps that ran, the
/// variables it sets on that agent and the connection. When it changes (an
/// update, a setup run with other steps, a setting its variables use, a
/// connection switched), agents started before use the previous setup.
pub fn generation(s: &Setup, conn: Option<&str>, facts: &Facts) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    for a in s.manifest.actions.iter().filter(|a| a.setup) {
        let key = run_key(a, conn);
        eat(key.as_bytes());
        eat(s.steps.get(&key).map_or(&b"-"[..], |d| d.as_bytes()));
    }
    let mut env = super::agent_env(&s.manifest, facts);
    env.sort();
    for (k, v) in env {
        eat(k.as_bytes());
        eat(v.as_bytes());
    }
    eat(conn.unwrap_or("").as_bytes());
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(extra: &str) -> Manifest {
        super::super::parse(&format!(
            r#"{{"v": 2, "id": "kit", "name": "Kit", "version": "1.0.0",
              "settings": [{{"key": "server", "type": "url", "label": "Server"}}],
              "agentEnv": [{{"name": "KIT_URL", "value": "{{setting:server}}"}}],
              "actions": [{{"id": "setup", "setup": true, "label": "Set up Kit", "description": "d", "steps": [{{"for": "claude", "run": ["claude", "plugin", "add", "kit#v1"]}}]}},
                          {{"id": "install", "label": "Install the server", "description": "d", "steps": [{{"run": ["npm", "i", "-g", "kit-srv@1"]}}]}}],
              "service": {{"label": "Kit's server", "run": ["kit-srv", "serve", "--data", "{{data}}"], "install": "install",
                 "version": {{"run": ["kit-srv", "version"], "match": "(\\d+\\.\\d+\\.\\d+)", "requires": ">=1.0.0"}}}}
              {extra}}}"#
        ))
        .unwrap()
    }

    fn bumped(m: &Manifest, pin: &str, order: Order) -> Manifest {
        let mut n = m.clone();
        n.version = Some("1.1.0".into());
        n.actions[0].steps[0].run[3] = format!("kit#{pin}");
        n.update.order = order;
        n
    }

    fn input<'a>(set_up: &'a [String]) -> PlanInput<'a> {
        PlanInput { on: true, service_here: true, has_data: true, data: "/pw/extension-data/kit", backup: "/pw/extension-backups/kit/<time>", set_up, path: Some("/nonexistent-pitwall-bin"), ..Default::default() }
    }

    #[test]
    fn the_policy_is_read_and_checked() {
        assert_eq!(manifest("").update, Policy::default());
        let m = manifest(r#", "update": {"order": "agentsFirst", "agents": "restart"}"#);
        assert_eq!(m.update, Policy { order: Order::AgentsFirst, agents: Agents::Restart });
        let bad = |u: &str| super::super::parse(&format!(r#"{{"v": 2, "id": "x", "name": "X", "update": {u}}}"#)).unwrap_err();
        assert!(bad(r#"{"order": "whenever"}"#).contains("serviceFirst or agentsFirst"));
        assert!(bad(r#"{"agents": "maybe"}"#).contains("compatible or restart"));
        assert!(bad(r#"{"rollback": true}"#).contains("doesn't know"));
        assert!(bad(r#""serviceFirst""#).contains("must be an object"));
        // Format 1 ignores it, as every format 2 key.
        assert_eq!(super::super::parse(r#"{"id": "x", "name": "X", "update": "nonsense"}"#).unwrap().update, Policy::default());
    }

    #[test]
    fn the_plan_runs_the_service_first_and_the_agents_last() {
        let old = manifest("");
        let new = bumped(&old, "v2", Order::ServiceFirst);
        let set_up = vec!["setup".to_string()];
        let steps: Vec<Step> = plan(&old, &new, &input(&set_up)).into_iter().map(|p| p.step).collect();
        assert_eq!(steps, [Step::Backup, Step::Install, Step::ServiceInstall, Step::ServiceStart, Step::Health, Step::Setup("setup".into())]);
        let p = plan(&old, &new, &input(&set_up));
        assert_eq!(p[0].lines, ["/pw/extension-data/kit → /pw/extension-backups/kit/<time>/data"], "no backup declared: Pitwall copies");
        assert_eq!(p[2].lines, ["npm i -g kit-srv@1"]);
        assert!(p[2].title.starts_with("If Kit's server is older than 1.0.0 or isn't installed"), "{}", p[2].title);
        assert_eq!(p[5].lines, ["Claude Code, default: claude plugin add kit#v2 (skipped: Claude Code isn't installed)"]);
    }

    #[test]
    fn agents_first_runs_the_setup_before_the_service() {
        let old = manifest("");
        let new = bumped(&old, "v2", Order::AgentsFirst);
        let set_up = vec!["setup".to_string()];
        let steps: Vec<Step> = plan(&old, &new, &input(&set_up)).into_iter().map(|p| p.step).collect();
        assert_eq!(steps, [Step::Backup, Step::Install, Step::Setup("setup".into()), Step::ServiceInstall, Step::ServiceStart, Step::Health]);
    }

    #[test]
    fn only_what_applies_is_planned() {
        let old = manifest("");
        // The same setup steps: not run again; never set up: not run.
        let same = bumped(&old, "v1", Order::ServiceFirst);
        let set_up = vec!["setup".to_string()];
        assert!(!plan(&old, &same, &input(&set_up)).iter().any(|p| matches!(p.step, Step::Setup(_))));
        let changed = bumped(&old, "v2", Order::ServiceFirst);
        assert!(!plan(&old, &changed, &input(&[])).iter().any(|p| matches!(p.step, Step::Setup(_))));
        // Off: only its files.
        let off = PlanInput { on: false, ..input(&set_up) };
        let p = plan(&old, &changed, &off);
        assert_eq!(p.iter().map(|p| &p.step).collect::<Vec<_>>(), [&Step::Install, &Step::Install]);
        assert!(p[1].title.contains("It is off"));
        // Its service runs elsewhere: nothing of it is touched.
        let elsewhere = PlanInput { service_here: false, ..input(&set_up) };
        assert!(!plan(&old, &changed, &elsewhere).iter().any(|p| matches!(p.step, Step::Backup | Step::ServiceStart)));
        // A declared backup is the one that runs, with its folders put in.
        let declared = manifest(r#", "update": {}"#);
        let mut new = changed.clone();
        new.service.as_mut().unwrap().backup = Some(crate::extensions::service::Backup { run: vec!["kit-srv".into(), "backup".into(), "--data".into(), "{data}".into(), "{backup}/kit.db".into()], replaces: vec!["kit.db".into()] });
        let p = plan(&declared, &new, &input(&set_up));
        assert_eq!(p[0].lines, ["kit-srv backup --data /pw/extension-data/kit '/pw/extension-backups/kit/<time>/kit.db'"]);
        // Too much to copy: said.
        let big = PlanInput { data_size: Some(3 * COPY_CAP), ..input(&set_up) };
        assert!(plan(&old, &changed, &big)[0].lines[1].contains("more than Pitwall copies (2.0 GiB)"));
    }

    #[test]
    fn the_review_says_what_running_agents_keep() {
        let old = manifest("");
        assert!(agents_lines(&old, 0, Restart::Ask).is_empty());
        let l = agents_lines(&old, 3, Restart::Ask);
        assert_eq!(l[0], "3 running agents keep the previous setup until restarted.");
        assert!(l[1].contains("restarts nothing by itself"));
        let mut r = old.clone();
        r.update.agents = Agents::Restart;
        assert_eq!(agents_lines(&r, 1, Restart::WhenIdle)[0], "Risk: 1 running agent will stop working with Kit until restarted.");
    }

    #[test]
    fn backup_folders_are_named_by_time_private_and_the_last_three_kept() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_795_392_000 + 3661), "20261123-010101");
        let t = crate::testing::TempDir::new("ext-backups");
        let dir = t.path().join(BACKUPS_DIR).join("kit");
        let a = make_backup_folder(&dir, 1_795_392_000).unwrap();
        let b = make_backup_folder(&dir, 1_795_392_000).unwrap();
        assert_eq!(a.file_name().unwrap(), "20261123-000000");
        assert_eq!(b.file_name().unwrap(), "20261123-000000-2", "the same second: another name");
        if let Some(mode) = crate::platform::mode(&a) {
            assert_eq!(mode & 0o777, 0o700);
            assert_eq!(crate::platform::mode(&dir).unwrap() & 0o777, 0o700);
        }
        for s in 1..=3 {
            make_backup_folder(&dir, 1_795_392_000 + s * 60).unwrap();
        }
        let gone = prune_backups(&dir, KEEP_BACKUPS);
        assert_eq!(gone, [a, b]);
        let left: Vec<String> = backups(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["20261123-000100", "20261123-000200", "20261123-000300"]);
    }

    #[test]
    fn a_copied_backup_is_put_back_and_what_was_there_kept() {
        let t = crate::testing::TempDir::new("ext-backup-copy");
        let data = t.path().join("extension-data/kit");
        std::fs::create_dir_all(data.join("db")).unwrap();
        std::fs::write(data.join("db/kit.db"), "before").unwrap();
        let backup = make_backup_folder(&t.path().join(BACKUPS_DIR).join("kit"), 1).unwrap();
        assert_eq!(copy_data(&data, &backup, COPY_CAP), Ok(6));
        assert_eq!(copy_data(&data, &backup, 3), Err(CopyRefused::TooBig(6)), "over the cap: not copied");
        std::fs::write(data.join("db/kit.db"), "after a migration").unwrap();
        restore_copy(&backup, &data).unwrap();
        assert_eq!(std::fs::read_to_string(data.join("db/kit.db")).unwrap(), "before");
        assert_eq!(std::fs::read_to_string(backup.join(REPLACED_DIR).join("db/kit.db")).unwrap(), "after a migration", "kept, never deleted");
    }

    #[test]
    fn a_services_own_backup_replaces_only_its_files() {
        let t = crate::testing::TempDir::new("ext-backup-files");
        let data = t.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        for (f, v) in [("kit.db", "new"), ("kit.db-wal", "w"), ("keep.txt", "k")] {
            std::fs::write(data.join(f), v).unwrap();
        }
        let backup = t.path().join("b");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("kit.db"), "old").unwrap();
        Note { from: Some("1.0.0".into()), to: Some("1.1.0".into()), at: 9, copy: false, replaces: vec!["kit.db".into(), "kit.db-wal".into()] }.write(&backup).unwrap();
        let note = Note::read(&backup).unwrap();
        assert!(note.restorable());
        restore_files(&backup, &data, &note.replaces).unwrap();
        assert_eq!(std::fs::read_to_string(data.join("kit.db")).unwrap(), "old");
        assert!(!data.join("kit.db-wal").exists(), "the files it stands for moved out");
        assert!(!data.join(BACKUP_NOTE).exists(), "the note isn't data");
        assert_eq!(std::fs::read_to_string(data.join("keep.txt")).unwrap(), "k", "the rest stays");
        assert_eq!(std::fs::read_to_string(backup.join(REPLACED_DIR).join("kit.db")).unwrap(), "new");
        if let Some(mode) = crate::platform::mode(&data.join("kit.db")) {
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(restore_files(&backup, &data, &[]).unwrap_err().contains("can't put this backup back"));
    }

    #[test]
    fn the_generation_follows_the_setup_variables_and_connection() {
        let m = manifest("");
        let mut steps = BTreeMap::from([("setup".to_string(), "aaaa".to_string())]);
        let settings = BTreeMap::from([("server".to_string(), "https://a.example".to_string())]);
        let facts = Facts { project: "/w/p", agent: "a", agent_id: "id1", settings: Some(&settings) };
        let s = Setup { manifest: m.clone(), steps: steps.clone() };
        let g = generation(&s, Some("c1"), &facts);
        assert_eq!(g, generation(&s, Some("c1"), &facts), "stable");
        steps.insert("setup".into(), "bbbb".into());
        assert_ne!(g, generation(&Setup { manifest: m.clone(), steps }, Some("c1"), &facts), "the setup ran with other steps");
        let other = BTreeMap::from([("server".to_string(), "https://b.example".to_string())]);
        assert_ne!(g, generation(&s, Some("c1"), &Facts { settings: Some(&other), ..facts }), "a setting its variables use");
        assert_ne!(g, generation(&s, Some("c2"), &facts), "another connection");
        assert_eq!(run_key(&m.actions[0], Some("c1")), "setup", "steps without a setting: one key");
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(size_words(COPY_CAP), "2.0 GiB");
        assert_eq!(size_words(12 * 1024 * 1024), "12 MiB");
        assert_eq!(size_words(512), "512 bytes");
    }
}
