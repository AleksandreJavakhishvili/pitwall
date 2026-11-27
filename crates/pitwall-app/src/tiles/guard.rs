//! Crash-loop protection for tiles (runtime.md §2.6). A tile's component
//! runs inside Pitwall's process: when starting one kills the process
//! (macOS once killed it for running Wasmtime's compiled code), Pitwall
//! would start it again on the next launch, and crash again, every time.
//!
//! So before a component starts, its extension is written down in
//! `<data>/tiles-running.json` ([`FILE`]); it is crossed out once the
//! component has run for [`HEALTHY_AFTER`], stopped on its own, or
//! Pitwall quits cleanly. Whatever is still written down when Pitwall
//! launches was starting (or had just started) when the last launch
//! ended without quitting: that launch crashed.
//!
//! - The first time ([`Verdict::paused`]): its components aren't started
//!   in this launch. A toast says "Pitwall closed unexpectedly last time
//!   while <extension> was running. Its tiles are paused." with Resume,
//!   Keep paused and Turn <extension> off; its tiles and its card in
//!   Settings say so too, the card with Go back when an update kept the
//!   version before.
//! - The second launch in a row ([`Verdict::off`]): it is turned off,
//!   with a note on its card, until the user turns it on again.
//!
//! A launch where it ran fine (or stopped without taking Pitwall down)
//! starts the count again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

/// The file, in Pitwall's data folder.
pub const FILE: &str = "tiles-running.json";

/// How long a component runs before a crash is no longer put down to it.
pub const HEALTHY_AFTER: Duration = Duration::from_secs(30);

/// Launches in a row that ended while an extension's component was
/// starting, after which it is turned off.
pub const OFF_AFTER: u32 = 2;

/// What the file holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// Extensions whose component started in this launch and hasn't
    /// proved itself yet, with the version that started.
    #[serde(default)]
    pub starting: BTreeMap<String, Option<String>>,
    /// Launches in a row that ended while each was starting.
    #[serde(default)]
    pub crashes: BTreeMap<String, u32>,
    /// Extensions turned off for it, with the version that crashed (the
    /// note on its card, until it is turned on again).
    #[serde(default)]
    pub turned_off: BTreeMap<String, Option<String>>,
}

/// What a launch does about the last one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Paused for this launch, with the version that was starting.
    pub paused: BTreeMap<String, Option<String>>,
    /// Turned off now.
    pub off: BTreeMap<String, Option<String>>,
}

/// A launch: each extension still written down as starting crashed the
/// last one; once more than before, or [`OFF_AFTER`] times in a row and
/// it is turned off. The record as it is now, for this launch.
pub fn launch(rec: &mut Record) -> Verdict {
    let mut v = Verdict::default();
    for (id, version) in std::mem::take(&mut rec.starting) {
        let n = rec.crashes.entry(id.clone()).or_default();
        *n += 1;
        if *n >= OFF_AFTER {
            rec.crashes.remove(&id);
            rec.turned_off.insert(id.clone(), version.clone());
            v.off.insert(id, version);
        } else {
            v.paused.insert(id, version);
        }
    }
    v
}

/// Read the record in `dir` (none or unreadable: empty).
pub fn read(dir: &Path) -> Record {
    std::fs::read(dir.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write the record to `dir` (an empty one removes the file). Written in
/// place and synced: it has to be on disk before the component starts.
pub fn write(dir: &Path, rec: &Record) -> std::io::Result<()> {
    let path = dir.join(FILE);
    if *rec == Record::default() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec_pretty(rec).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!(".{FILE}.tmp"));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, path)
}

/// What an extension's card and tiles say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// Its components aren't started in this launch.
    Paused { version: Option<String> },
    /// Turned off after crashing twice in a row.
    TurnedOff { version: Option<String> },
}

/// The guard for this launch.
pub struct Guard {
    dir: Option<PathBuf>,
    rec: Record,
    paused: BTreeMap<String, Option<String>>,
    /// Turned off at this launch (its toast says so once).
    just_off: Vec<String>,
    /// The toasts were shown ([`offer`]).
    offered: bool,
    pub healthy_after: Duration,
}

impl Global for Guard {}

impl Guard {
    fn save(&self) {
        if let Some(dir) = &self.dir {
            if let Err(e) = write(dir, &self.rec) {
                eprintln!("pitwall: couldn't write {}: {e}", dir.join(FILE).display());
            }
        }
    }
}

fn guard_mut(cx: &mut App) -> Option<&mut Guard> {
    cx.has_global::<Guard>().then(|| cx.global_mut::<Guard>())
}

/// At launch, before any component starts: what the last launch left in
/// data folder `root` (`None`: nothing is kept), paused and turned off.
/// Shown later by [`offer`].
pub fn init(root: Option<&Path>, cx: &mut App) {
    let dir = root.map(Path::to_path_buf);
    let mut rec = dir.as_deref().map(read).unwrap_or_default();
    let v = launch(&mut rec);
    for id in v.off.keys() {
        crate::ui_state::update(cx, |s| *s = std::mem::take(s).set_extension(id, false));
    }
    let g = Guard { dir, rec, paused: v.paused, just_off: v.off.into_keys().collect(), offered: false, healthy_after: HEALTHY_AFTER };
    g.save();
    cx.set_global(g);
    // Turned off by the user: not paused any more; turned on again: the
    // note goes, and it gets a fresh count.
    let ui = crate::ui_state::store(cx);
    cx.observe(&ui, |_, cx| {
        let ui = crate::ui_state::get(cx);
        let Some(g) = cx.try_global::<Guard>() else { return };
        let unpause: Vec<String> = g.paused.keys().filter(|id| !ui.extension_on(id)).cloned().collect();
        let back_on: Vec<String> = g.rec.turned_off.keys().filter(|id| ui.extension_on(id)).cloned().collect();
        if unpause.is_empty() && back_on.is_empty() {
            return;
        }
        let g = cx.global_mut::<Guard>();
        for id in unpause {
            g.paused.remove(&id);
        }
        for id in back_on {
            g.rec.turned_off.remove(&id);
        }
        g.save();
    })
    .detach();
}

/// Extension `id`'s components aren't started in this launch.
pub fn paused(id: &str, cx: &App) -> bool {
    cx.try_global::<Guard>().is_some_and(|g| g.paused.contains_key(id))
}

/// What extension `id`'s card and tiles say about the guard.
pub fn note(id: &str, cx: &App) -> Option<Note> {
    let g = cx.try_global::<Guard>()?;
    if let Some(v) = g.paused.get(id) {
        return Some(Note::Paused { version: v.clone() });
    }
    g.rec.turned_off.get(id).map(|v| Note::TurnedOff { version: v.clone() })
}

/// How long a component runs before it is crossed out.
pub fn healthy_after(cx: &App) -> Duration {
    cx.try_global::<Guard>().map_or(HEALTHY_AFTER, |g| g.healthy_after)
}

/// Extension `id`'s component (version `version`) is about to start:
/// written down, on disk before it starts.
pub fn starting(id: &str, version: Option<String>, cx: &mut App) {
    let Some(g) = guard_mut(cx) else { return };
    if g.rec.starting.get(id) != Some(&version) {
        g.rec.starting.insert(id.to_string(), version);
        g.save();
    }
}

/// Extension `id`'s component ran for [`HEALTHY_AFTER`], or stopped
/// without taking Pitwall down: crossed out, and its count starts again.
pub fn settled(id: &str, cx: &mut App) {
    let Some(g) = guard_mut(cx) else { return };
    let was = g.rec.starting.remove(id).is_some();
    if g.rec.crashes.remove(id).is_some() || was {
        g.save();
    }
}

/// Pitwall quits cleanly: nothing that was starting crashed it.
pub fn quit(cx: &mut App) {
    let Some(g) = guard_mut(cx) else { return };
    let ids: Vec<String> = g.rec.starting.keys().cloned().collect();
    if ids.is_empty() {
        return;
    }
    for id in ids {
        g.rec.starting.remove(&id);
        g.rec.crashes.remove(&id);
    }
    g.save();
}

/// Resume: extension `id`'s components start again (its open tiles,
/// its toolbar badge's).
pub fn resume(id: &str, cx: &mut App) {
    if let Some(g) = guard_mut(cx) {
        g.paused.remove(id);
    }
    let id = id.to_string();
    super::tiles(cx).update(cx, |t, cx| t.restart(&id, cx));
}

/// Turn extension `id` off (its tiles stay paused until it is on again).
pub fn turn_off(id: &str, cx: &mut App) {
    crate::ui_state::update(cx, |s| *s = std::mem::take(s).set_extension(id, false));
}

/// Tests: as if the app had run [`init`] with `rec` left by the last
/// launch, in data folder `dir`; components are crossed out after
/// `healthy_after`.
#[cfg(test)]
pub fn init_for_test(dir: &Path, rec: &Record, healthy_after: Duration, cx: &mut App) {
    write(dir, rec).unwrap();
    init(Some(dir), cx);
    cx.global_mut::<Guard>().healthy_after = healthy_after;
}

/// The words of the paused toast and the turned-off one, for extension
/// `name`.
pub fn paused_words(name: &str) -> (String, String) {
    (format!("Pitwall closed unexpectedly last time while {name} was running."), "Its tiles are paused.".to_string())
}

pub fn off_words(name: &str) -> (String, String) {
    (format!("Pitwall closed unexpectedly twice in a row while {name} was running."), format!("{name} is turned off. Turn it on again in Settings → Extensions."))
}

/// Once the main window is there: a toast for each extension paused or
/// turned off at this launch.
pub fn offer(cx: &mut App) {
    let Some(g) = guard_mut(cx) else { return };
    if std::mem::replace(&mut g.offered, true) {
        return;
    }
    let paused: Vec<String> = g.paused.keys().cloned().collect();
    let off = g.just_off.clone();
    let name = |id: &str, cx: &App| crate::extensions::manifest_of(id, cx).map(|m| m.name).unwrap_or_else(|| id.to_string());
    for id in paused {
        let n = name(&id, cx);
        crate::windows::paused_tiles_offer(&id, &n, cx);
    }
    for id in off {
        let n = name(&id, cx);
        let (title, detail) = off_words(&n);
        crate::windows::extension_toast(&id, title, detail, None, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(starting: &[&str], crashes: &[(&str, u32)]) -> Record {
        Record {
            starting: starting.iter().map(|s| (s.to_string(), Some("2.0.0".to_string()))).collect(),
            crashes: crashes.iter().map(|(s, n)| (s.to_string(), *n)).collect(),
            turned_off: BTreeMap::new(),
        }
    }

    /// Nothing written down: nothing happens. One crash: paused; the
    /// second in a row: turned off, with its version noted.
    #[test]
    fn a_crash_pauses_and_two_in_a_row_turn_off() {
        let mut r = Record::default();
        assert_eq!(launch(&mut r), Verdict::default());
        let mut r = rec(&["inbox"], &[]);
        let v = launch(&mut r);
        assert_eq!(v.paused.keys().collect::<Vec<_>>(), ["inbox"]);
        assert!(v.off.is_empty());
        assert_eq!(r.crashes.get("inbox"), Some(&1));
        assert!(r.starting.is_empty(), "this launch starts with nothing written down");
        // Resumed, and it took Pitwall down again.
        r.starting.insert("inbox".into(), Some("2.0.0".into()));
        let v = launch(&mut r);
        assert!(v.paused.is_empty());
        assert_eq!(v.off.get("inbox"), Some(&Some("2.0.0".to_string())));
        assert_eq!(r.turned_off.get("inbox"), Some(&Some("2.0.0".to_string())));
        assert!(!r.crashes.contains_key("inbox"), "turned on again, it gets a fresh count");
    }

    /// Only the ones that were starting count.
    #[test]
    fn others_are_left_alone() {
        let mut r = rec(&["a"], &[("b", 1)]);
        let v = launch(&mut r);
        assert_eq!(v.paused.keys().collect::<Vec<_>>(), ["a"]);
        assert_eq!(r.crashes.get("b"), Some(&1));
    }

    /// Written and read back; an empty record leaves no file; a broken
    /// file reads as empty.
    #[test]
    fn the_file() {
        let t = pitwall_core::testing::TempDir::new("tiles-guard-file");
        let r = rec(&["inbox"], &[("c4", 1)]);
        write(t.path(), &r).unwrap();
        assert_eq!(read(t.path()), r);
        write(t.path(), &Record::default()).unwrap();
        assert!(!t.path().join(FILE).exists());
        std::fs::write(t.path().join(FILE), "{").unwrap();
        assert_eq!(read(t.path()), Record::default());
    }
}
