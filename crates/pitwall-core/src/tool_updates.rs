//! Agent tools kept up to date by Pitwall (`agents.updateTools`): a kind
//! whose definition names an `update_command` (`claude update`) is updated
//! here, one at a time, at startup and every few hours, and everything
//! Pitwall launches gets the kind's `update_off_env` so the tool doesn't
//! also update itself. Many sessions each updating themselves at once race
//! (two `npm install -g` runs) and can leave the tool's command gone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::Serialize;

use crate::engine::Engine;
use crate::exec::{Cmd, Exec, LocalExec};
use crate::kind::AgentKind;
use crate::shell::LoginShell;

/// Between two rounds of updates.
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// After Pitwall starts, before the first round (startup has work enough).
pub const FIRST_AFTER: Duration = Duration::from_secs(60);
/// One update command, at most.
pub const TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The last update of one tool.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    pub kind: String,
    pub kind_name: String,
    /// Unix ms.
    pub at: u64,
    pub ok: bool,
    /// The command's last line of output (or why it failed).
    pub note: String,
}

#[derive(Default)]
pub struct ToolUpdates {
    on: AtomicBool,
    started: AtomicBool,
    last: Mutex<Vec<Checked>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ToolUpdates {
    pub fn on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// The environment that turns off the tools' own updaters (empty when
    /// Pitwall doesn't update them).
    pub fn env(&self, kinds: &[AgentKind]) -> Vec<(String, String)> {
        if !self.on() {
            return vec![];
        }
        off_env(kinds)
    }

    pub fn last(&self) -> Vec<Checked> {
        lock(&self.last).clone()
    }
}

/// Every updated kind's `update_off_env`.
pub fn off_env(kinds: &[AgentKind]) -> Vec<(String, String)> {
    kinds
        .iter()
        .filter(|k| k.update_command.as_deref().is_some_and(|c| !c.trim().is_empty()))
        .flat_map(|k| k.update_off_env.iter().map(|(n, v)| (n.clone(), v.clone())))
        .collect()
}

/// The last non-empty line of `text`.
fn last_line(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).next_back().unwrap_or_default().chars().take(200).collect()
}

impl Engine {
    /// Pitwall updates the agent tools (true) or leaves it to them. The
    /// first time it's turned on, the update loop starts.
    pub fn set_tool_updates(self: &Arc<Self>, on: bool) {
        self.tool_updates.on.store(on, Ordering::Relaxed);
        if on && !self.tool_updates.started.swap(true, Ordering::SeqCst) {
            let weak = Arc::downgrade(self);
            let _ = std::thread::Builder::new().name("tool-updates".into()).spawn(move || run(weak));
        }
    }

    /// The last update of each tool Pitwall updates.
    pub fn tool_update_results(&self) -> Vec<Checked> {
        self.tool_updates.last()
    }

    /// The environment for a process Pitwall launches.
    pub(crate) fn tool_update_env(&self) -> Vec<(String, String)> {
        self.tool_updates.env(&self.kinds().kinds())
    }
}

fn run(engine: Weak<Engine>) {
    std::thread::sleep(FIRST_AFTER);
    loop {
        let Some(e) = engine.upgrade() else { return };
        if e.tool_updates.on() {
            round(&e);
        }
        drop(e);
        std::thread::sleep(EVERY);
    }
}

/// Update every installed tool that has an update command, one by one.
fn round(e: &Engine) {
    let installed: Vec<String> = match e.list_kinds() {
        Ok(v) => v.into_iter().filter(|k| k.installed).map(|k| k.id).collect(),
        Err(_) => return,
    };
    for kind in e.kinds().kinds() {
        let Some(command) = kind.update_command.as_deref().map(str::trim).filter(|c| !c.is_empty()) else { continue };
        if !installed.contains(&kind.id) {
            continue;
        }
        let (program, args) = LoginShell::current().run(command);
        let mut argv = vec![program.as_str()];
        argv.extend(args.iter().map(String::as_str));
        let (ok, note) = match LocalExec.run(&Cmd::new(&argv).timeout(TIMEOUT)) {
            Ok(out) if out.ok() => (true, last_line(&out.stdout_text())),
            Ok(out) => (false, last_line(&format!("{}\n{}", out.stdout_text(), out.stderr_text()))),
            Err(err) => (false, err.to_string()),
        };
        if !ok {
            eprintln!("pitwall: updating {} failed: {note}", kind.name);
        }
        let checked = Checked { kind: kind.id.clone(), kind_name: kind.name.clone(), at: e.now(), ok, note };
        let mut last = lock(&e.tool_updates.last);
        last.retain(|c| c.kind != checked.kind);
        last.push(checked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(update: Option<&str>, off: &[(&str, &str)]) -> AgentKind {
        let mut k = crate::kind::custom_kind("x");
        k.update_command = update.map(String::from);
        k.update_off_env = off.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        k
    }

    #[test]
    fn only_updated_kinds_turn_their_updater_off() {
        let kinds = [kind(Some("x update"), &[("X_NO_UPDATE", "1")]), kind(None, &[("Y_NO_UPDATE", "1")]), kind(Some("  "), &[("Z", "1")])];
        let t = ToolUpdates::default();
        assert!(t.env(&kinds).is_empty(), "off until the app turns it on");
        t.on.store(true, Ordering::Relaxed);
        assert_eq!(t.env(&kinds), [("X_NO_UPDATE".to_string(), "1".to_string())]);
    }

    #[test]
    fn the_bundled_claude_is_updated_by_pitwall() {
        let dir = std::env::temp_dir().join(format!("pw-tool-updates-{}", std::process::id()));
        let kinds = crate::kind::KindCatalog::new(dir).kinds();
        let claude = kinds.iter().find(|k| k.id == "claude").unwrap();
        assert_eq!(claude.update_command.as_deref(), Some("claude update"));
        assert!(off_env(&kinds).contains(&("DISABLE_AUTOUPDATER".to_string(), "1".to_string())));
    }

    #[test]
    fn notes_are_the_last_line() {
        assert_eq!(last_line("Checking…\nUpdated to 2.1.300\n\n"), "Updated to 2.1.300");
        assert_eq!(last_line(""), "");
    }
}
