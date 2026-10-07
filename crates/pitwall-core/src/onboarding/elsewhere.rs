//! The sidebar's live "Elsewhere" group (docs/spec/terminals.md §2): agent
//! CLIs running in other terminal apps, found the same way as the scan's
//! "Running now". Read-only: Pitwall never adopts or signals these
//! processes; "Bring into Pitwall" resumes the conversation in a new agent.
//!
//! Cheap enough to call every ~10 s: one `ps`; `lsof` and transcript reads
//! only for processes not seen before (cached by pid).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use super::latest;
use super::project_list::normalize;
use super::scan::{self, RunningAgent, ScanResult};
use crate::engine::Engine;
use crate::paths;
use crate::platform;
use crate::procs::{self, Found, Matcher};

const LSOF_TIMEOUT: Duration = Duration::from_secs(3);
/// Look again for the conversation of a process whose transcript wasn't
/// there yet (it appears with the first prompt).
const RELOOK_MS: u64 = 30_000;

#[derive(Debug, Clone, Default, PartialEq)]
struct Known {
    kind: String,
    cwd: Option<String>,
    session_id: Option<String>,
    title: Option<String>,
    /// Unix ms of the last conversation lookup.
    looked_at: u64,
}

/// What earlier calls learned about each process. Owned by the host.
#[derive(Default)]
pub struct Elsewhere {
    known: Mutex<HashMap<u32, Known>>,
}

impl Elsewhere {
    pub fn new() -> Elsewhere {
        Elsewhere::default()
    }

    /// Agents running outside Pitwall right now, minus conversations a
    /// Pitwall agent already has. Blocking (runs `ps`, maybe `lsof`).
    pub fn list(&self, engine: &Engine) -> Result<Vec<RunningAgent>, String> {
        let table = platform::process_table().ok_or("could not read the process list")?;
        let matcher = Matcher::new(&engine.kinds().kinds());
        let found = procs::agents_outside(&procs::parse_table(&table), &engine.owned_pids(), &matcher);
        let now = crate::clock::unix_ms();

        // Which processes need lsof / transcript reads (no lock held for those).
        let todo: Vec<(Found, Option<Known>)> = {
            let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
            let live: HashSet<u32> = found.iter().map(|f| f.pid).collect();
            known.retain(|pid, _| live.contains(pid));
            found
                .iter()
                .filter_map(|f| match known.get(&f.pid) {
                    Some(k) if k.kind == f.kind && (k.session_id.is_some() || now < k.looked_at + RELOOK_MS) => None,
                    Some(k) if k.kind == f.kind => Some((f.clone(), Some(k.clone()))),
                    _ => Some((f.clone(), None)),
                })
                .collect()
        };
        if !todo.is_empty() {
            let need_cwd: Vec<u32> = todo.iter().filter(|(_, k)| k.as_ref().is_none_or(|k| k.cwd.is_none())).map(|(f, _)| f.pid).collect();
            let cwds: HashMap<u32, String> = if need_cwd.is_empty() {
                HashMap::new()
            } else {
                platform::process_cwds(&need_cwd, LSOF_TIMEOUT).map(|v| v.into_iter().collect()).unwrap_or_default()
            };
            let home = paths::home();
            let learned: Vec<(u32, Known)> = todo
                .into_iter()
                .map(|(f, prev)| {
                    let cwd = prev.and_then(|k| k.cwd).or_else(|| cwds.get(&f.pid).cloned());
                    let (session_id, title) = match (&f.session_id, &cwd) {
                        (Some(id), Some(c)) => (Some(id.clone()), latest::title(&home, &f.kind, c, id)),
                        (Some(id), None) => (Some(id.clone()), None),
                        (None, Some(c)) => match latest::newest(&home, &f.kind, c, None) {
                            Some(l) => (Some(l.session_id), l.title),
                            None => (None, None),
                        },
                        (None, None) => (None, None),
                    };
                    (f.pid, Known { kind: f.kind, cwd, session_id, title, looked_at: now })
                })
                .collect();
            let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
            known.extend(learned);
        }

        let known = self.known.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let paths = engine.paths();
        let mut rows = assemble(found, &known, &engine.owned_sessions(), |cwd| scan::is_project_folder(paths, cwd));
        // The project the user chose for that conversation last time.
        let mut shown = ScanResult { running: std::mem::take(&mut rows), ..Default::default() };
        scan::apply_project_choices(&mut shown, &engine.projects().conversation_projects());
        Ok(shown.running)
    }
}

/// Rows for the UI: one per process, without conversations Pitwall already
/// runs (an agent of its own, or one started by hand in a Pitwall terminal).
fn assemble(
    found: Vec<Found>,
    known: &HashMap<u32, Known>,
    owned: &HashSet<String>,
    is_project: impl Fn(&str) -> bool,
) -> Vec<RunningAgent> {
    found
        .into_iter()
        .filter_map(|f| {
            let k = known.get(&f.pid).filter(|k| k.kind == f.kind);
            let session_id = f.session_id.clone().or_else(|| k.and_then(|k| k.session_id.clone()));
            if session_id.as_ref().is_some_and(|s| owned.contains(s)) {
                return None;
            }
            let cwd = k.and_then(|k| k.cwd.clone());
            Some(RunningAgent {
                pid: f.pid,
                kind: f.kind,
                kind_name: f.kind_name,
                cwd_display: cwd.as_deref().map(paths::tildify),
                outside_project: cwd.as_deref().is_some_and(|c| !is_project(&normalize(c))),
                cwd,
                session_id,
                title: k.and_then(|k| k.title.clone()),
                in_pitwall: false,
                display_project: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::load_kinds;

    fn found() -> Vec<Found> {
        let rows = procs::parse_table(include_str!("scan_fixtures/ps.txt"));
        let m = Matcher::new(&load_kinds(std::path::Path::new("/nonexistent/pitwall-agents")));
        procs::agents_outside(&rows, &HashSet::from([500]), &m)
    }

    fn known(kind: &str, cwd: &str, session: Option<&str>) -> Known {
        Known { kind: kind.into(), cwd: Some(cwd.into()), session_id: session.map(String::from), title: Some("t".into()), looked_at: 1 }
    }

    #[test]
    fn conversations_pitwall_runs_are_not_elsewhere() {
        let known = HashMap::from([
            (41207, known("claude", "/w/shop", Some("from-transcript"))),
            (52318, known("claude", "/w/web", None)),
            (36504, known("codex", "/", None)),
        ]);
        let ids = |rows: &[RunningAgent]| rows.iter().map(|r| r.pid).collect::<Vec<_>>();

        let all = assemble(found(), &known, &HashSet::new(), |p| p != "/");
        assert_eq!(ids(&all), vec![41207, 52318, 36504]);
        assert_eq!(all[0].session_id.as_deref(), Some("from-transcript"));
        assert_eq!(all[1].session_id.as_deref(), Some("abc"), "the args win");
        assert_eq!((all[0].cwd.as_deref(), all[0].title.as_deref()), (Some("/w/shop"), Some("t")));
        assert!(all[2].outside_project && !all[0].outside_project);

        // A Pitwall agent (or a Pitwall terminal) has `abc` and `019e`.
        let owned = HashSet::from(["abc".to_string(), "019e".to_string()]);
        assert_eq!(ids(&assemble(found(), &known, &owned, |_| true)), vec![41207]);
    }

    #[test]
    fn stale_cache_entries_are_ignored() {
        // The pid now runs another kind: what was learned about it no longer applies.
        let known = HashMap::from([(41207, known("codex", "/w/x", Some("old")))]);
        let rows = assemble(found(), &known, &HashSet::new(), |_| true);
        assert_eq!((rows[0].session_id.as_deref(), rows[0].cwd.as_deref()), (None, None));
    }
}
