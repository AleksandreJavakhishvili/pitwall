//! An extension's setup actions (docs/spec/extensions.md, "Setup
//! actions"): [`plan`] says exactly what a button will run and what it
//! will skip, for the confirmation; [`run`] runs it after the user
//! confirms, off the UI thread, each step with a timeout and its output
//! kept short. Nothing runs at any other time.

use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pitwall_core::extensions::{expand_args, Action, Manifest};

/// How long one step may take.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(180);
/// How much of a step's output is kept.
const KEEP: usize = 8 * 1024;
/// The lines of output a failure shows.
const TAIL_LINES: usize = 6;

/// One step as it will run, or why it won't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The program and its arguments, settings put in.
    pub argv: Vec<String>,
    /// Where the program is (found on PATH).
    pub program: Option<PathBuf>,
    /// Why it won't run.
    pub skip: Option<String>,
    pub for_kind: Option<String>,
    pub optional: bool,
}

impl Planned {
    /// The command as a line to read (arguments with spaces or quotes
    /// quoted; nothing is run through a shell).
    pub fn line(&self) -> String {
        self.argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
    }
}

fn quote(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@#=+,%".contains(c)) {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

/// What an agent kind is called (`claude`: "Claude Code").
pub fn kind_name(id: &str) -> String {
    match id {
        "claude" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "gemini" => "Gemini CLI".into(),
        "aider" => "Aider".into(),
        other => other.to_string(),
    }
}

/// Action `a` of extension `m`, with its plain settings `values`, its
/// programs looked up on `path`.
pub fn plan(m: &Manifest, a: &Action, values: &BTreeMap<String, String>, path: Option<&str>) -> Vec<Planned> {
    a.steps
        .iter()
        .map(|s| {
            let (argv, mut skip) = match expand_args(&s.run, m, values) {
                Ok(v) => (v, None),
                Err(label) => (s.run.clone(), Some(format!("set {label} first"))),
            };
            let program = super::adapter::resolve(Path::new("."), &argv[0], path).ok();
            if program.is_none() && skip.is_none() {
                skip = Some(match &s.for_kind {
                    Some(k) => format!("{} isn't installed", kind_name(k)),
                    None => format!("“{}” isn't installed", argv[0]),
                });
            }
            Planned { argv, program, skip, for_kind: s.for_kind.clone(), optional: s.optional }
        })
        .collect()
}

/// How a step went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Failed: why, and the last lines of its output.
    Failed(String),
    Skipped(String),
}

/// Run `plan` in `cwd`, in order (blocking: call it off the UI thread). A
/// failed step that isn't optional skips the later steps for the same
/// kind (all later steps when it is for none).
pub fn run(plan: &[Planned], cwd: &Path, timeout: Duration) -> Vec<Outcome> {
    let mut out = Vec::new();
    let mut failed_kinds: Vec<Option<String>> = Vec::new();
    for p in plan {
        if let Some(why) = &p.skip {
            out.push(Outcome::Skipped(why.clone()));
            continue;
        }
        if failed_kinds.iter().any(|k| k.is_none() || *k == p.for_kind) {
            out.push(Outcome::Skipped(match &p.for_kind {
                Some(k) => format!("an earlier step for {} failed", kind_name(k)),
                None => "an earlier step failed".into(),
            }));
            continue;
        }
        let o = run_one(p, cwd, timeout);
        if matches!(o, Outcome::Failed(_)) && !p.optional {
            failed_kinds.push(p.for_kind.clone());
        }
        out.push(o);
    }
    out
}

fn run_one(p: &Planned, cwd: &Path, timeout: Duration) -> Outcome {
    let Some(program) = &p.program else { return Outcome::Skipped("it isn't installed".into()) };
    let mut cmd = crate::platform::background_command(program);
    cmd.args(&p.argv[1..]).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(path) = pitwall_core::shell::spawn_path() {
        cmd.env("PATH", path);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("it couldn't start: {e}")),
    };
    let kept: Arc<Mutex<VecDeque<u8>>> = Arc::default();
    let readers: Vec<_> = [child.stdout.take().map(|r| Box::new(r) as Box<dyn Read + Send>), child.stderr.take().map(|r| Box::new(r) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
        .map(|mut r| {
            let k = kept.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                while let Ok(n) = r.read(&mut buf) {
                    if n == 0 {
                        return;
                    }
                    let mut q = k.lock().unwrap_or_else(|e| e.into_inner());
                    q.extend(&buf[..n]);
                    while q.len() > KEEP {
                        q.pop_front();
                    }
                }
            })
        })
        .collect();
    let end = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // Its output, once it has ended (a program it started may still hold
    // the pipes after a timeout: those readers are left to finish).
    if status.is_some() {
        for r in readers {
            let _ = r.join();
        }
    }
    let text: Vec<u8> = kept.lock().unwrap_or_else(|e| e.into_inner()).iter().copied().collect();
    let text = String::from_utf8_lossy(&text);
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
    let tail = pitwall_core::extensions::protocol::clean(&tail, 1200, true);
    match status {
        Some(s) if s.success() => Outcome::Done,
        Some(s) => {
            let why = s.code().map_or("it was stopped".to_string(), |c| format!("it exited with code {c}"));
            Outcome::Failed(if tail.is_empty() { why } else { format!("{why}:\n{tail}") })
        }
        None => Outcome::Failed(format!("it took longer than {} s and was stopped", timeout.as_secs())),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        let text = r#"{"id": "x", "name": "X", "settings": [{"key": "server", "type": "url", "label": "Server"}],
            "actions": [{"id": "setup", "label": "Set up", "description": "d", "steps": [
                {"for": "fake-a", "run": ["fake-ok", "add", "--url", "{setting:server}"]},
                {"for": "fake-a", "run": ["fake-fail", "it's"]},
                {"for": "fake-a", "run": ["fake-ok", "after"]},
                {"for": "fake-b", "run": ["fake-missing", "x"]},
                {"for": "fake-c", "run": ["fake-fail"], "optional": true},
                {"for": "fake-c", "run": ["fake-slow"]},
                {"run": ["fake-ok", "last"]}
            ]}]}"#;
        pitwall_core::extensions::parse(text).unwrap()
    }

    fn fakes(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        for (name, body) in [
            ("fake-ok", "#!/bin/sh\necho \"ok $*\"\n"),
            ("fake-fail", "#!/bin/sh\necho line one\necho 'not allowed' >&2\nexit 2\n"),
            ("fake-slow", "#!/bin/sh\nsleep 5\n"),
        ] {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// The confirmation lists exactly what will run and what won't.
    #[test]
    fn the_plan_says_exactly_what_will_run() {
        let t = pitwall_core::testing::TempDir::new("actions-plan");
        fakes(t.path());
        let m = manifest();
        let path = t.path().to_string_lossy().into_owned();
        let none = plan(&m, &m.actions[0], &Default::default(), Some(&path));
        assert_eq!(none[0].skip.as_deref(), Some("set Server first"));
        let values = BTreeMap::from([("server".to_string(), "https://s.example".to_string())]);
        let p = plan(&m, &m.actions[0], &values, Some(&path));
        let lines: Vec<String> = p.iter().map(Planned::line).collect();
        assert_eq!(lines[0], "fake-ok add --url https://s.example");
        assert_eq!(lines[1], "fake-fail 'it'\\''s'");
        assert_eq!(p[3].skip.as_deref(), Some("fake-b isn't installed"));
        assert!(p.iter().enumerate().all(|(i, s)| (i == 3) == s.skip.is_some()));
        assert_eq!(kind_name("claude"), "Claude Code");
    }

    /// Every extension in the repo plans each of its actions, one line per
    /// step; one whose `test/pitwall.json` lists an action's commands plans
    /// exactly those (planned only: nothing runs), with the programs it
    /// names under `installed` as fakes on the PATH.
    #[test]
    fn each_shipped_extension_plans_its_actions() {
        for (dir, m, fixture) in crate::extensions::shipped_all() {
            let t = pitwall_core::testing::TempDir::new("actions-shipped");
            let installed = fixture["actions"]["installed"].as_array().cloned().unwrap_or_default();
            for program in installed.iter().filter_map(|p| p.as_str()) {
                std::fs::write(t.path().join(program), "").unwrap();
            }
            let path = t.path().to_string_lossy().into_owned();
            for a in &m.actions {
                let p = plan(&m, a, &Default::default(), Some(&path));
                assert_eq!(p.len(), a.steps.len(), "{}: {}", dir.display(), a.id);
                let Some(want) = fixture["actions"]["expect"].get(&a.id) else { continue };
                let strings = |v: &serde_json::Value| -> Vec<Option<String>> { v.as_array().unwrap().iter().map(|s| s.as_str().map(str::to_string)).collect() };
                assert_eq!(want["label"].as_str(), Some(a.label.as_str()), "{}", dir.display());
                let lines: Vec<Option<String>> = p.iter().map(|s| Some(s.line())).collect();
                assert_eq!(lines, strings(&want["lines"]), "{}: {}", dir.display(), a.id);
                assert_eq!(p.iter().map(|s| s.skip.clone()).collect::<Vec<_>>(), strings(&want["skips"]), "{}: {}", dir.display(), a.id);
                let optional: Vec<bool> = want["optional"].as_array().unwrap().iter().map(|b| b.as_bool().unwrap()).collect();
                assert_eq!(p.iter().map(|s| s.optional).collect::<Vec<_>>(), optional, "{}: {}", dir.display(), a.id);
            }
            if let Some(expect) = fixture["actions"]["expect"].as_object() {
                assert!(expect.keys().all(|id| m.actions.iter().any(|a| &a.id == id)), "{}: an expected action is missing", dir.display());
            }
        }
    }

    /// Done, failed with its last lines, skipped after a failure for the
    /// same kind, skipped as missing, optional failure carrying on, and a
    /// timeout; only fake programs in a temporary folder run.
    #[test]
    fn a_run_reports_each_step_in_plain_words() {
        let t = pitwall_core::testing::TempDir::new("actions-run");
        fakes(t.path());
        let m = manifest();
        let values = BTreeMap::from([("server".to_string(), "https://s.example".to_string())]);
        let p = plan(&m, &m.actions[0], &values, Some(&t.path().to_string_lossy()));
        let out = run(&p, t.path(), Duration::from_millis(1500));
        assert_eq!(out[0], Outcome::Done);
        match &out[1] {
            Outcome::Failed(why) => assert!(why.starts_with("it exited with code 2") && why.contains("not allowed") && why.contains("line one"), "{why}"),
            o => panic!("{o:?}"),
        }
        assert_eq!(out[2], Outcome::Skipped("an earlier step for fake-a failed".into()));
        assert_eq!(out[3], Outcome::Skipped("fake-b isn't installed".into()));
        assert!(matches!(out[4], Outcome::Failed(_)), "optional: it fails ...");
        assert_eq!(out[5], Outcome::Failed("it took longer than 1 s and was stopped".into()), "... and the next one still runs");
        assert_eq!(out[6], Outcome::Done, "steps for no kind don't stop for another kind's failure");
    }
}
