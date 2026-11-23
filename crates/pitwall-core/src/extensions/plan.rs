//! What an extension would run and set, worked out without running
//! anything (docs/spec/extensions.md, "Setup actions" and "Settings →
//! Extensions"): an adapter's or a step's program found ([`resolve`]), a
//! setup action's steps as they will run or why they won't ([`plan`]),
//! and the variables it sets on agents ([`env_preview`]). Settings shows
//! these, and `pitwall extension check` says them ([`super::check`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{expand_args, Action, Manifest, SETTING_PLACEHOLDER};

/// The program `program` names: a path inside `dir` when it has a slash,
/// else found on `path` (the login shell's PATH; this process's when
/// `None`).
pub fn resolve(dir: &Path, program: &str, path: Option<&str>) -> Result<PathBuf, String> {
    if program.contains('/') || program.contains('\\') {
        let p = dir.join(program);
        let (Ok(full), Ok(base)) = (p.canonicalize(), dir.canonicalize()) else {
            return Err(format!("{program} isn't in the extension's folder"));
        };
        if !full.starts_with(&base) {
            return Err(format!("{program} isn't inside the extension's folder"));
        }
        return Ok(full);
    }
    let path = path.map(String::from).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    let exts: &[&str] = if std::env::consts::OS == "windows" { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    for d in std::env::split_paths(&path) {
        for e in exts {
            let c = d.join(format!("{program}{e}"));
            if c.is_file() {
                return Ok(c);
            }
        }
    }
    Err(format!("“{program}” isn't installed or isn't on your PATH: install it, then restart the extension"))
}

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
        line(&self.argv)
    }
}

/// `argv` as a line to read.
pub fn line(argv: &[String]) -> String {
    argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
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
            let program = resolve(Path::new("."), &argv[0], path).ok();
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

/// Each variable extension `m` sets, as `NAME=value`: its settings put in
/// (the per-agent placeholders left as they are), or why it isn't set.
pub fn env_preview(m: &Manifest, values: &BTreeMap<String, String>) -> Vec<String> {
    m.agent_env
        .iter()
        .map(|v| {
            let mut value = v.value.clone();
            for s in &m.settings {
                let p = format!("{SETTING_PLACEHOLDER}{}}}", s.key);
                if value.contains(&p) {
                    match values.get(&s.key).filter(|x| !x.trim().is_empty()) {
                        Some(x) => value = value.replace(&p, x),
                        None => return format!("{} (not set until {} is)", v.name, s.label),
                    }
                }
            }
            format!("{}={value}", v.name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_found_and_kept_in_their_folder() {
        let t = crate::testing::TempDir::new("plan-resolve");
        std::fs::write(t.path().join("run.sh"), "").unwrap();
        assert!(resolve(t.path(), "./run.sh", None).is_ok());
        assert!(resolve(t.path(), "../outside.sh", None).is_err());
        std::fs::create_dir_all(t.path().join("bin")).unwrap();
        std::fs::write(t.path().join("bin").join("tool"), "").unwrap();
        let path = t.path().join("bin").to_string_lossy().into_owned();
        assert!(resolve(t.path(), "tool", Some(&path)).is_ok());
        assert!(resolve(t.path(), "no-such-program-here", Some(&path)).unwrap_err().contains("isn't installed"));
    }

    #[test]
    fn lines_are_quoted_for_reading() {
        assert_eq!(line(&["a".into(), "it's".into(), "b c".into(), "--x=1".into()]), "a 'it'\\''s' 'b c' --x=1");
        assert_eq!(kind_name("claude"), "Claude Code");
        assert_eq!(kind_name("other"), "other");
    }
}
