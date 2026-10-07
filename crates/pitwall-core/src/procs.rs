//! Recognising agent CLIs in the process table (docs/spec/terminals.md):
//! which process is which agent kind, which session it runs, and whether it
//! belongs to Pitwall. Pure parsing over `ps` output; the platform layer runs
//! `ps` (architecture.md §9 decision 7). Used by the onboarding scan's
//! "Running now", the sidebar's "Elsewhere" group and the agent-in-a-terminal
//! detection.

use std::collections::{HashMap, HashSet};

use crate::kind::{catalog::program, AgentKind};

/// Programs that run an agent's script: the agent is their first non-flag argument.
const WRAPPERS: &[&str] = &["node", "bun", "deno", "python", "python3"];
/// Background helpers agent CLIs start; never an interactive agent.
const SERVERS: &[&str] = &["app-server", "mcp-server"];

/// One `ps -o pid=,ppid=,args=` row.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcRow {
    pub pid: u32,
    pub ppid: u32,
    pub args: String,
}

/// Rows of `ps -axww -o pid=,ppid=,args=`.
pub fn parse_table(out: &str) -> Vec<ProcRow> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let rest = l.trim_start();
            let rest = rest[rest.find(char::is_whitespace)?..].trim_start();
            let args = rest[rest.find(char::is_whitespace)?..].trim_start();
            Some(ProcRow { pid, ppid, args: args.to_string() })
        })
        .collect()
}

/// `pid → number` pairs from `ps -o pid=,<field>=` (e.g. `tpgid`).
pub fn parse_pid_numbers(out: &str) -> HashMap<u32, i64> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect()
}

/// `pid → args` from `ps -ww -o pid=,args=`.
pub fn parse_pid_args(out: &str) -> HashMap<u32, String> {
    out.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let cut = l.find(char::is_whitespace).unwrap_or(l.len());
            Some((l[..cut].parse().ok()?, l[cut..].trim().to_string()))
        })
        .collect()
}

fn base(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// An agent kind as the process table shows it.
#[derive(Debug, Clone)]
struct Entry {
    names: Vec<String>,
    kind: AgentKind,
}

/// Which agent a command line runs, from the agent definitions: their
/// `process_names`, else the program of their `command`. The shell and
/// custom commands are never matched.
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    entries: Vec<Entry>,
}

/// A command line recognised as an agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Recognised {
    pub kind: String,
    pub kind_name: String,
    /// The conversation it resumes or uses, when its arguments say so.
    pub session_id: Option<String>,
}

impl Matcher {
    pub fn new(kinds: &[AgentKind]) -> Matcher {
        let entries = kinds
            .iter()
            .filter(|k| k.id != "shell" && k.id != "custom")
            .filter_map(|k| {
                let names: Vec<String> = if k.process_names.is_empty() {
                    let p = program(&k.command);
                    vec![base(&p).to_string()]
                } else {
                    k.process_names.clone()
                };
                let names: Vec<String> = names.into_iter().filter(|n| !n.is_empty()).collect();
                (!names.is_empty()).then(|| Entry { names, kind: k.clone() })
            })
            .collect();
        Matcher { entries }
    }

    /// The agent `args` (a full command line) runs, if any.
    pub fn recognise(&self, args: &str) -> Option<Recognised> {
        let mut words = args.split_whitespace();
        let mut prog = base(words.next()?);
        if WRAPPERS.contains(&prog) {
            prog = base(words.by_ref().find(|w| !w.starts_with('-'))?);
        }
        let rest: Vec<&str> = words.collect();
        if rest.iter().any(|w| SERVERS.contains(w)) {
            return None;
        }
        let e = self.entries.iter().find(|e| e.names.iter().any(|n| n == prog))?;
        Some(Recognised {
            kind: e.kind.id.clone(),
            kind_name: e.kind.name.clone(),
            session_id: session_from_templates(&e.kind, &rest).or_else(|| session_from_args(&e.kind.id, &rest)),
        })
    }
}

/// The session id in `args` where the kind's own launch templates put one
/// (`new_args` / `resume_args` entries with `{session_id}`): `--resume X`,
/// `--resume=X`, `resume X`.
pub fn session_from_templates(kind: &AgentKind, args: &[&str]) -> Option<String> {
    let id = |s: &str| (!s.is_empty() && !s.starts_with('-')).then(|| s.to_string());
    for tpl in [&kind.new_args, &kind.resume_args] {
        let Some(at) = tpl.iter().position(|a| a.contains("{session_id}")) else { continue };
        let slot = &tpl[at];
        if slot != "{session_id}" {
            // "--resume={session_id}": a prefix glued to the id.
            let prefix = slot.split("{session_id}").next().unwrap_or_default();
            if let Some(v) = args.iter().find_map(|a| a.strip_prefix(prefix)) {
                return id(v);
            }
            continue;
        }
        let Some(flag) = at.checked_sub(1).map(|i| tpl[i].as_str()) else { continue };
        if let Some(v) = args.iter().find_map(|a| a.strip_prefix(flag).and_then(|r| r.strip_prefix('='))) {
            return id(v);
        }
        if let Some(i) = args.iter().position(|a| *a == flag) {
            if let Some(v) = args.get(i + 1).and_then(|v| id(v)) {
                return Some(v);
            }
        }
    }
    None
}

/// Shorthands the templates don't spell out (`claude -r <id>`).
pub fn session_from_args(kind: &str, args: &[&str]) -> Option<String> {
    let id = |s: &str| (!s.is_empty() && !s.starts_with('-')).then(|| s.to_string());
    match kind {
        "claude" => args.iter().enumerate().find_map(|(i, a)| {
            for flag in ["--resume", "--session-id"] {
                if let Some(v) = a.strip_prefix(flag).and_then(|r| r.strip_prefix('=')) {
                    return id(v);
                }
            }
            if matches!(*a, "--resume" | "-r" | "--session-id") {
                return args.get(i + 1).and_then(|v| id(v));
            }
            None
        }),
        "codex" => {
            let i = args.iter().position(|a| *a == "resume")?;
            args.get(i + 1).and_then(|v| id(v))
        }
        _ => None,
    }
}

/// An agent process found in the process table.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub pid: u32,
    pub kind: String,
    pub kind_name: String,
    pub session_id: Option<String>,
}

/// Agent processes in `rows` that are not descendants of any of `roots`
/// (Pitwall itself and its terminals). An agent's own children (helpers,
/// a re-exec of the same CLI) are not listed twice.
pub fn agents_outside(rows: &[ProcRow], roots: &HashSet<u32>, matcher: &Matcher) -> Vec<Found> {
    let parent: HashMap<u32, u32> = rows.iter().map(|r| (r.pid, r.ppid)).collect();
    let ancestors = |pid: u32| {
        let mut out = Vec::new();
        let mut p = pid;
        for _ in 0..64 {
            match parent.get(&p) {
                Some(&pp) if pp != p && pp != 0 => {
                    out.push(pp);
                    p = pp;
                }
                _ => break,
            }
        }
        out
    };
    let found: Vec<(u32, Recognised)> = rows
        .iter()
        .filter(|r| !roots.contains(&r.pid))
        .filter_map(|r| Some((r.pid, matcher.recognise(&r.args)?)))
        .collect();
    let agent_pids: HashSet<u32> = found.iter().map(|(p, _)| *p).collect();
    found
        .into_iter()
        .filter(|(pid, _)| {
            let up = ancestors(*pid);
            !up.iter().any(|a| roots.contains(a) || agent_pids.contains(a))
        })
        .map(|(pid, r)| Found { pid, kind: r.kind, kind_name: r.kind_name, session_id: r.session_id })
        .collect()
}

/// What runs in the foreground of a terminal whose shell is `shell_pid`:
/// the leader of the terminal's foreground process group (`tpgid`), or the
/// shell itself when it `exec`ed into something. `tpgids`: `pid → tpgid`;
/// `args`: `pid → command line`.
pub fn foreground_agent(
    shell_pid: u32,
    tpgids: &HashMap<u32, i64>,
    args: &HashMap<u32, String>,
    matcher: &Matcher,
) -> Option<(u32, Recognised)> {
    let leader = match tpgids.get(&shell_pid) {
        Some(&g) if g > 0 => g as u32,
        _ => shell_pid,
    };
    let line = args.get(&leader)?;
    matcher.recognise(line).map(|r| (leader, r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::load_kinds;

    fn matcher() -> Matcher {
        Matcher::new(&load_kinds(std::path::Path::new("/nonexistent/pitwall-agents")))
    }

    fn roots(pids: &[u32]) -> HashSet<u32> {
        pids.iter().copied().collect()
    }

    fn short(found: Vec<Found>) -> Vec<(u32, String, Option<String>)> {
        found.into_iter().map(|f| (f.pid, f.kind, f.session_id)).collect()
    }

    #[test]
    fn ps_output_skips_own_children_and_servers() {
        let rows = parse_table(include_str!("onboarding/scan_fixtures/ps.txt"));
        let got = short(agents_outside(&rows, &roots(&[500]), &matcher()));
        assert_eq!(
            got,
            vec![
                (41207, "claude".into(), None),
                (52318, "claude".into(), Some("abc".into())),
                (36504, "codex".into(), Some("019e".into()))
            ]
        );
        // Not running under Pitwall: its child agent counts too.
        let got = short(agents_outside(&rows, &roots(&[4242]), &matcher()));
        assert!(got.contains(&(502, "claude".into(), Some("x".into()))));
    }

    #[test]
    fn agents_in_pitwall_terminals_are_not_elsewhere() {
        // A terminal holder is detached from the app: its shell (700) is a root.
        let rows = parse_table(include_str!("onboarding/scan_fixtures/ps.txt"));
        let got = short(agents_outside(&rows, &roots(&[500, 700]), &matcher()));
        assert_eq!(got, vec![(52318, "claude".into(), Some("abc".into()))]);
    }

    #[test]
    fn session_ids_from_args() {
        assert_eq!(session_from_args("claude", &["--resume", "s1"]).as_deref(), Some("s1"));
        assert_eq!(session_from_args("claude", &["-r", "s2", "--verbose"]).as_deref(), Some("s2"));
        assert_eq!(session_from_args("claude", &["--session-id=s3"]).as_deref(), Some("s3"));
        assert_eq!(session_from_args("claude", &["--resume"]), None);
        assert_eq!(session_from_args("claude", &["--continue"]), None);
        assert_eq!(session_from_args("codex", &["resume", "019e"]).as_deref(), Some("019e"));
        assert_eq!(session_from_args("codex", &["resume", "--last"]), None);
        assert_eq!(session_from_args("codex", &[]), None);
    }

    #[test]
    fn session_ids_from_kind_templates() {
        let m = matcher();
        let r = |line: &str| m.recognise(line).and_then(|r| r.session_id);
        assert_eq!(r("gemini --resume g1").as_deref(), Some("g1"));
        assert_eq!(r("copilot --resume=c1").as_deref(), Some("c1"));
        assert_eq!(r("opencode --session ses_1").as_deref(), Some("ses_1"));
        assert_eq!(r("claude --session-id u1").as_deref(), Some("u1"));
        assert_eq!(r("claude -r u2").as_deref(), Some("u2"), "shorthand");
        assert_eq!(r("gemini"), None);
    }

    #[test]
    fn recognises_kinds_by_program_and_wrapper() {
        let m = matcher();
        let kind = |line: &str| m.recognise(line).map(|r| r.kind);
        assert_eq!(kind("/opt/homebrew/bin/claude").as_deref(), Some("claude"));
        assert_eq!(kind("node --no-warnings /usr/local/bin/gemini -m pro").as_deref(), Some("gemini"));
        assert_eq!(kind("cursor-agent").as_deref(), Some("cursor"));
        assert_eq!(kind("aider --model sonnet").as_deref(), Some("aider"));
        assert_eq!(m.recognise("claude").unwrap().kind_name, "Claude Code");
        // Not agents: the shell, look-alikes, servers, plain node.
        for line in ["-zsh", "/bin/zsh -l", "claude-trace foo", "codex app-server --listen", "node", "vim claude.md", ""] {
            assert_eq!(kind(line), None, "{line}");
        }
    }

    #[test]
    fn process_names_override_the_command() {
        let mut k = load_kinds(std::path::Path::new("/nonexistent/pitwall-agents"))
            .into_iter()
            .find(|k| k.id == "gemini")
            .unwrap();
        k.process_names = vec!["gem".into(), "gemini-cli".into()];
        let m = Matcher::new(&[k]);
        assert_eq!(m.recognise("gemini-cli").map(|r| r.kind).as_deref(), Some("gemini"));
        assert_eq!(m.recognise("gemini"), None);
    }

    #[test]
    fn foreground_process_group_of_a_terminal() {
        let m = matcher();
        let tpgids = parse_pid_numbers("  700   900\n  800   800\n  810    -1\n 820 0\n");
        let args = parse_pid_args(
            "  900 node /opt/homebrew/bin/codex resume 019e\n  800 -zsh\n  810 claude --resume s9\n 820 bash\n",
        );
        // A job in the foreground: its group leader.
        let (pid, r) = foreground_agent(700, &tpgids, &args, &m).unwrap();
        assert_eq!((pid, r.kind.as_str(), r.session_id.as_deref()), (900, "codex", Some("019e")));
        // The shell itself is in the foreground: nothing.
        assert_eq!(foreground_agent(800, &tpgids, &args, &m), None);
        // No terminal group known (exec'd agent): the process itself.
        let (pid, r) = foreground_agent(810, &tpgids, &args, &m).unwrap();
        assert_eq!((pid, r.kind.as_str()), (810, "claude"));
        assert_eq!(foreground_agent(820, &tpgids, &args, &m), None);
        // Gone from the table.
        assert_eq!(foreground_agent(999, &tpgids, &args, &m), None);
    }

    #[test]
    fn ps_field_parsing_tolerates_noise() {
        assert_eq!(parse_pid_numbers("x y\n 12 34\n\n"), HashMap::from([(12, 34)]));
        assert_eq!(parse_pid_args("  7 a b  c\nnope\n"), HashMap::from([(7, "a b  c".to_string())]));
        assert_eq!(parse_table("  1 0 /sbin/launchd\nbad\n"), vec![ProcRow { pid: 1, ppid: 0, args: "/sbin/launchd".into() }]);
    }
}
