//! What the conversation core needs from the rest of Pitwall to run a
//! shared session (§6), as narrow traits:
//!
//! - [`SessionHost`]: runs a session's agent as an ordinary engine agent
//!   (`AgentRecord.shared`), types messages into it and reports what the
//!   engine sees (status, hooks, the screen) as [`SessionEvent`]s;
//!   [`super::engine_host::EngineHost`] is the engine's, tests script one.
//! - [`OwnerAgents`]: the ordinary agent being shared (its kind, account).
//! - [`Snapshots`]: makes the session's copy (§6.2);
//!   [`super::sandboxed::GitSnapshots`] in Pitwall.
//! - [`EgressProxies`]: the per-share egress proxy's port (§6.3);
//!   [`super::sandboxed::ProxyPool`] in Pitwall.
//! - [`SessionSkills`]: the agent's assigned rules and skills, generated
//!   into the session folder (§6.2).
//!
//! The OS sandbox itself is [`crate::sandbox::SessionSandbox`]: the core
//! builds the policy and wraps the launch before handing it to the host.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use regex::Regex;

use super::turn::Reply;
use crate::error::Result;
use crate::kind::ShareKind;
use crate::kind::AgentKind;
use crate::sandbox::Wrapped;
use pitwall_proto::collab::{ReplySource, Scope, ShareSettings};

/// What the engine records on a shared session's agent
/// (`AgentRecord.shared`, §6.1).
pub use crate::model::SharedSession;

/// One shared session's agent, ready to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLaunch {
    /// The engine agent id, picked by the core (stable for the conversation).
    pub agent: String,
    /// Shown in the owner's sidebar (`api-expert · bob@example.com`).
    pub name: String,
    /// The kind (its screen rules and status detection apply as usual).
    pub kind: String,
    pub kind_name: String,
    /// The copy (`…/s/<conv>/work`), where it runs.
    pub cwd: PathBuf,
    /// The owner's project it is grouped under.
    pub project: String,
    /// The whole launch, already sandboxed: the host runs exactly this,
    /// with exactly this environment.
    pub wrapped: Wrapped,
    pub shared: SharedSession,
    /// The kind's conversation id this launch uses, when known up front.
    pub session_id: Option<String>,
    /// The launch continues `session_id` (the kind's resume).
    pub resumed: bool,
    /// Tier (b), the kind's non-interactive mode (§7.2): nothing runs
    /// between turns, and each turn is one run of these (`{message}`: the
    /// message as typed), already sandboxed. `None`: an interactive agent.
    pub turns: Option<TurnArgs>,
}

/// A non-interactive kind's turn commands, already sandboxed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnArgs {
    /// The conversation's first turn (a resumed launch has none: `next`).
    pub first: Wrapped,
    /// Every later turn.
    pub next: Wrapped,
}

/// Where a turn's message goes in [`TurnArgs`].
pub const MESSAGE: &str = "{message}";

/// What a host reports about a running session's agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// Up and waiting for input (first idle after a launch).
    Ready,
    /// The kind's conversation id (hooks, `SessionStart` after a fork).
    SessionId(String),
    /// It started working on the message.
    Working,
    /// The turn ended; the reply when one was captured (§7.2).
    TurnEnded(Option<Reply>),
    /// It needs the owner (§7.3): a permission prompt, or it stopped to ask.
    Blocked { detail: Option<String> },
    /// Its vendor's usage limit (§5.8), with the reset time it showed.
    Limited { until: Option<String> },
    /// The limit screen is gone.
    LimitCleared,
    /// The process ended.
    Exited,
}

/// Where a host sends events: (engine agent id, event).
pub type HostSink = Arc<dyn Fn(&str, SessionEvent) + Send + Sync>;

/// Runs shared sessions' agents. Calls may block (starting a process);
/// the core never holds its lock across them.
pub trait SessionHost: Send + Sync {
    /// Where events go; set once by the core.
    fn subscribe(&self, sink: HostSink);
    /// Start (or start again, keeping the record) the agent of `launch`.
    fn launch(&self, launch: &HostLaunch) -> Result<()>;
    /// Deliver a message as ordinary input (pasted, then Enter, §7.1).
    fn send(&self, agent: &str, text: &str) -> Result<()>;
    /// Stop the current turn (as Stop for a turn).
    fn interrupt(&self, agent: &str) -> Result<()>;
    /// End the process; the agent and its copy stay (idle stop, §6.6).
    fn stop(&self, agent: &str) -> Result<()>;
    /// Forget the agent (the conversation was removed).
    fn remove(&self, agent: &str) -> Result<()>;
    /// Whatever reply exists for the current turn right now (a timeout or
    /// cancel returns it, §6.6).
    fn partial_reply(&self, agent: &str) -> Option<Reply>;
}

/// The ordinary agent a share shares.
#[derive(Debug, Clone)]
pub struct SharedAgent {
    pub kind: AgentKind,
    /// The owner's project it is grouped under.
    pub project: String,
    /// Its account (login), unless the share names another.
    pub account: Option<String>,
}

pub trait OwnerAgents: Send + Sync {
    fn agent(&self, id: &str) -> Option<SharedAgent>;
    /// The agent account's config folder and variable, readable and
    /// writable inside the sandbox (§6.3 "The agent account").
    fn account_env(&self, kind: &AgentKind, account: Option<&str>) -> Vec<(String, String)>;
    /// One of `kind`'s accounts by id or name (`accounts.json`); `None`:
    /// no such account. The kind's Default (the owner's own login) is never
    /// one ([`super::live`]).
    fn account(&self, _kind: &AgentKind, _key: &str) -> Option<super::live::AccountInfo> {
        None
    }
    /// The value of the owner's variable `name`, for a share that passes
    /// it explicitly (§5.4 step 5): this process's environment, else the
    /// owner's login shell (the app, started from the Dock, has no shell
    /// variables). Only names a share passes are ever looked up.
    fn env_value(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// The session copy that was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The owner's real commit the copy was made from (`base-commit`).
    pub base_commit: String,
    pub files: u32,
}

/// Makes a session's copy (§6.2): `git init` in `dest` and one commit with
/// only the scope's paths minus its exclusions, no history, no link back.
pub trait Snapshots: Send + Sync {
    fn make(&self, scope: &Scope, dest: &Path) -> Result<Snapshot>;
}

/// The per-share egress proxy (§6.3): the port sessions of `share` reach
/// the network through, allowing `hosts` only; `None`: no network.
pub trait EgressProxies: Send + Sync {
    fn port(&self, share: &ShareSettings, hosts: &[String]) -> Result<Option<u16>>;
    /// The share stopped: its proxy can go.
    fn forget(&self, _share: &str) {}
}

/// Generates the shared agent's assigned rules and skills into a session
/// folder (§6.2). Returns the files written.
pub trait SessionSkills: Send + Sync {
    fn write(&self, owner_agent: &str, kind: &AgentKind, folder: &Path) -> Result<Vec<String>>;
}

// ------------------------------------------------------------------ launch argv

/// The kind's `[share]` data, or the defaults (§6.7).
pub fn share_kind(kind: &AgentKind) -> ShareKind {
    kind.share.clone().unwrap_or_default()
}

/// Whether sessions of `kind` can be resumed after a stop.
pub fn can_resume(kind: &AgentKind) -> bool {
    !kind.resume_args.is_empty()
}

pub fn can_fork(kind: &AgentKind) -> bool {
    !share_kind(kind).fork_args.is_empty()
}

/// How a session starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartAs {
    Fresh,
    /// Continue the kind's conversation `id`.
    Resume(String),
    /// A new conversation forked from the base's (§6.4).
    Fork(String),
}

/// A kind's argv (not a shell line: the sandbox wraps it as is), the
/// conversation id it will use when known, and whether it resumes.
/// `hook`: the hook relay command, when the kind takes hooks per launch.
pub fn kind_argv(kind: &AgentKind, start: &StartAs, hook: Option<&str>) -> (Vec<String>, Option<String>, bool) {
    let mut argv: Vec<String> = kind.command.split_whitespace().map(String::from).collect();
    let fresh_id = || kind.assign_session_id.then(|| uuid::Uuid::new_v4().to_string());
    let (args, session, base, resumed) = match start {
        StartAs::Resume(id) if can_resume(kind) => (kind.resume_args.clone(), Some(id.clone()), None, true),
        StartAs::Fork(base) if can_fork(kind) => (share_kind(kind).fork_args, fresh_id(), Some(base.clone()), false),
        _ => (kind.new_args.clone(), fresh_id(), None, false),
    };
    for arg in args {
        if arg.contains("{session_id}") {
            let Some(id) = &session else { continue };
            argv.push(arg.replace("{session_id}", id));
        } else if arg.contains("{base}") {
            let Some(b) = &base else { continue };
            argv.push(arg.replace("{base}", b));
        } else {
            argv.push(arg);
        }
    }
    // The agent's own sandbox can't nest inside Pitwall's (§6.8): off, by
    // its own arguments, environment (the policy) or settings.
    let sk = share_kind(kind);
    argv.extend(sk.own_sandbox_off_args.iter().cloned());
    if kind.hooks == crate::kind::HookMode::ClaudeSettings {
        let mut settings = match hook {
            Some(cmd) => crate::hooks::claude_settings_value(cmd),
            None => serde_json::json!({}),
        };
        if let (Some(obj), Some(serde_json::Value::Object(extra))) = (settings.as_object_mut(), &sk.own_sandbox_off_settings) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        if settings.as_object().is_some_and(|o| !o.is_empty()) {
            argv.push("--settings".into());
            argv.push(settings.to_string());
        }
    }
    (argv, session, resumed)
}

/// Tier (b): the kind's per-turn argv for its first and later turns
/// (`{message}` left in place, `{session_id}` filled), the conversation
/// id they use, and whether the launch resumes. `None`: the kind has no
/// non-interactive mode in its data, or reads replies another way.
pub fn turn_argv(kind: &AgentKind, start: &StartAs, cwd: &Path) -> Option<(Vec<String>, Vec<String>, String, bool)> {
    let sk = share_kind(kind);
    if sk.reply != crate::kind::ReplyTier::NonInteractive {
        return None;
    }
    let ni = sk.non_interactive.as_ref()?;
    let (id, resumed) = match start {
        StartAs::Resume(id) => (id.clone(), true),
        _ => (uuid::Uuid::new_v4().to_string(), false),
    };
    let build = |args: &[String]| -> Vec<String> {
        let mut argv: Vec<String> = kind.command.split_whitespace().map(String::from).collect();
        argv.extend(args.iter().map(|a| a.replace("{session_id}", &id)));
        argv.extend(sk.own_sandbox_off_args.iter().cloned());
        argv.extend(session_args(kind, cwd));
        argv
    };
    let next = build(&ni.next_args);
    let first = if resumed { next.clone() } else { build(&ni.first_args) };
    Some((first, next, id, resumed))
}

/// Tier (b): the reply in a non-interactive run's output: the last JSON
/// object printed (stdout first; Gemini CLI prints its error object on
/// stderr), its `reply` field. `Err`: the run's error message, when it
/// printed one.
pub fn reply_from_run(ni: &crate::kind::NonInteractive, stdout: &str, stderr: &str) -> std::result::Result<Option<Reply>, String> {
    let objects = |text: &str| -> Vec<serde_json::Value> {
        // Pretty-printed objects start on a line of their own; warnings and
        // stack traces around them are skipped.
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(i) = text[at..].find('{').map(|i| at + i) {
            let line_start = i == 0 || text[..i].ends_with('\n');
            if line_start {
                let mut it = serde_json::Deserializer::from_str(&text[i..]).into_iter::<serde_json::Value>();
                if let Some(Ok(v)) = it.next() {
                    if v.is_object() {
                        out.push(v);
                    }
                    at = i + it.byte_offset().max(1);
                    continue;
                }
            }
            at = i + 1;
        }
        out
    };
    let all: Vec<serde_json::Value> = objects(stdout).into_iter().chain(objects(stderr)).collect();
    for v in all.iter().rev() {
        if let Some(text) = v.pointer(&ni.reply).and_then(|x| x.as_str()).map(str::trim).filter(|t| !t.is_empty()) {
            return Ok(Some(Reply { text: text.to_string(), source: ReplySource::NonInteractive }));
        }
    }
    let error = ni.error.as_deref().and_then(|p| all.iter().rev().find_map(|v| v.pointer(p).and_then(|x| x.as_str()).map(String::from)));
    match error {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

/// The kind's `[share] session_args` for a session in `cwd` (its real
/// path, as the agent records it).
pub fn session_args(kind: &AgentKind, cwd: &Path) -> Vec<String> {
    let real = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let real = real.to_string_lossy();
    share_kind(kind).session_args.iter().map(|a| a.replace("{cwd}", &real)).collect()
}

/// The hook relay command for one shared session: the agent id and the
/// socket are in the command itself, so the session's environment carries
/// no `PITWALL_*` variable (§6.3).
pub fn hook_command(base: &str, agent: &str, socket: &Path) -> String {
    format!(
        "PITWALL_AGENT_ID={} PITWALL_SOCKET={} {base}",
        crate::shell::quote(agent),
        crate::shell::quote(&socket.to_string_lossy())
    )
}

// ------------------------------------------------------------------ capture

/// Tier (a): the reply in a hook payload (Claude Code `Stop`:
/// `last_assistant_message`).
pub fn reply_from_hook(payload: &serde_json::Value) -> Option<Reply> {
    let text = payload.get("last_assistant_message").and_then(|v| v.as_str())?.trim();
    (!text.is_empty()).then(|| Reply { text: text.to_string(), source: ReplySource::Hook })
}

/// Tier (c): the text printed between sending the message and the end of
/// the turn (§7.2): the lines of `after` that weren't on screen in
/// `before`, minus the echo of the message itself and trailing chrome
/// (the input box and status bar under the last blank-separated block are
/// what the kind's screen rules match; lines made of box-drawing only are
/// dropped).
pub fn reply_from_screen(before: &str, after: &str, sent: &str) -> Option<Reply> {
    let old: std::collections::HashSet<&str> = before.lines().map(str::trim_end).collect();
    let sent_lines: std::collections::HashSet<&str> = sent.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let chrome = |l: &str| {
        let t = l.trim();
        t.is_empty() || t.chars().all(|c| matches!(c, '─' | '│' | '╭' | '╮' | '╰' | '╯' | '━' | '-' | '=' | '>' | '›' | ' ' | '▌' | '█'))
    };
    let new: Vec<&str> = after
        .lines()
        .map(str::trim_end)
        .filter(|l| !old.contains(l))
        .filter(|l| !sent_lines.contains(l.trim().trim_start_matches(['>', '›', '│', ' '])))
        .collect();
    let mut lines: Vec<&str> = new.into_iter().skip_while(|l| chrome(l)).collect();
    while lines.last().is_some_and(|l| chrome(l)) {
        lines.pop();
    }
    let text = lines.join("\n").trim().to_string();
    (!text.is_empty()).then_some(Reply { text, source: ReplySource::Terminal })
}

/// The vendor limit on screen (§5.8), with the reset time it shows.
pub fn limit_on_screen(kind: &ShareKind, screen: &str) -> Option<Option<String>> {
    let re = Regex::new(kind.limit_screen.as_deref()?).ok()?;
    let caps = re.captures(screen)?;
    Some(caps.name("until").map(|m| m.as_str().trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> AgentKind {
        toml::from_str(include_str!("../../agents/claude.toml")).unwrap()
    }

    #[test]
    fn claude_argv_fresh_resume_and_fork() {
        let k = claude();
        let (argv, id, resumed) = kind_argv(&k, &StartAs::Fresh, Some("sh /h"));
        let id = id.unwrap();
        assert!(!resumed);
        assert_eq!(&argv[..3], &["claude".to_string(), "--session-id".into(), id.clone()]);
        assert_eq!(argv[3], "--settings");
        assert!(argv[4].contains("\"command\":\"sh /h\""));

        // Its own Bash sandbox is off in the same settings: it can't nest.
        let settings: serde_json::Value = serde_json::from_str(&argv[4]).unwrap();
        assert_eq!(settings["sandbox"], serde_json::json!({"enabled": false}));
        assert!(settings["hooks"]["Stop"].is_array());

        let off = r#"{"sandbox":{"enabled":false}}"#;
        let (argv, id, resumed) = kind_argv(&k, &StartAs::Resume("s-1".into()), None);
        assert_eq!(argv, ["claude", "--resume", "s-1", "--settings", off]);
        assert_eq!(id.as_deref(), Some("s-1"));
        assert!(resumed);

        let (argv, id, _) = kind_argv(&k, &StartAs::Fork("base-1".into()), None);
        let id = id.unwrap();
        assert_eq!(argv, ["claude", "--resume", "base-1", "--fork-session", "--session-id", id.as_str(), "--settings", off]);
    }

    #[test]
    fn codex_sessions_trust_their_copy_for_this_run_only() {
        let k: AgentKind = toml::from_str(include_str!("../../agents/codex.toml")).unwrap();
        let dir = crate::testing::TempDir::new("codex-copy");
        let real = std::fs::canonicalize(dir.path()).unwrap().to_string_lossy().into_owned();
        let trust = format!("projects={{\"{real}\"={{trust_level=\"trusted\"}}}}");
        let (mut argv, id, resumed) = kind_argv(&k, &StartAs::Fresh, None);
        argv.extend(session_args(&k, dir.path()));
        assert_eq!(argv, ["codex", "--sandbox", "danger-full-access", "-c", trust.as_str()]);
        assert!(id.is_none() && !resumed, "Codex names its conversation itself (its session file says which)");
        let (mut argv, _, resumed) = kind_argv(&k, &StartAs::Resume("019a-1".into()), None);
        argv.extend(session_args(&k, dir.path()));
        assert_eq!(argv, ["codex", "resume", "019a-1", "--sandbox", "danger-full-access", "-c", trust.as_str()]);
        assert!(resumed);
        assert_eq!(share_kind(&k).reply, crate::kind::ReplyTier::SessionFile);
    }

    #[test]
    fn gemini_turns_run_its_non_interactive_mode_and_resume_by_id() {
        let k: AgentKind = toml::from_str(include_str!("../../agents/gemini.toml")).unwrap();
        let dir = crate::testing::TempDir::new("gemini-copy");
        let (first, next, id, resumed) = turn_argv(&k, &StartAs::Fresh, dir.path()).unwrap();
        assert!(!resumed);
        assert_eq!(first, ["gemini", "--session-id", id.as_str(), "-p", MESSAGE, "--output-format", "json", "--skip-trust"]);
        assert_eq!(next, ["gemini", "--resume", id.as_str(), "-p", MESSAGE, "--output-format", "json", "--skip-trust"]);
        let (first, next, id2, resumed) = turn_argv(&k, &StartAs::Resume("g-7".into()), dir.path()).unwrap();
        assert!(resumed && id2 == "g-7" && first == next);
        assert!(turn_argv(&claude(), &StartAs::Fresh, dir.path()).is_none(), "an interactive kind");
        assert_eq!(share_kind(&k).own_sandbox_off_env.get("GEMINI_SANDBOX").map(String::as_str), Some("false"));
    }

    /// Recorded from Gemini CLI 0.64.0 (`-p … --output-format json`) against
    /// a local mock of the model API.
    #[test]
    fn gemini_replies_and_errors_from_its_recorded_output() {
        let k: AgentKind = toml::from_str(include_str!("../../agents/gemini.toml")).unwrap();
        let ni = share_kind(&k).non_interactive.unwrap();
        let first = include_str!("fixtures/gemini-headless.json");
        let r = reply_from_run(&ni, first, "Warning: Basic terminal detected (TERM=dumb).\n").unwrap().unwrap();
        assert_eq!((r.text.as_str(), r.source), ("Mock Gemini reply 12: the docs are in docs/.", ReplySource::NonInteractive));
        let resumed = include_str!("fixtures/gemini-headless-resumed.json");
        assert_eq!(reply_from_run(&ni, resumed, "").unwrap().unwrap().text, "Mock Gemini reply 1: the docs are in docs/.");
        // Warnings and a stack trace around it on the same stream.
        let noisy = format!("Ripgrep is not available.\n    at async main (file:///x.js:1:2) {{\n{first}\nmore\n");
        assert!(reply_from_run(&ni, &noisy, "").unwrap().is_some());
        // No login: the error object, on stderr.
        let e = reply_from_run(&ni, "", include_str!("fixtures/gemini-headless-no-login.stderr")).unwrap_err();
        assert!(e.starts_with("Please set an Auth method"), "{e}");
        assert_eq!(reply_from_run(&ni, "", "").unwrap(), None);
    }

    #[test]
    fn hook_commands_carry_the_agent_and_socket() {
        let c = hook_command("sh '/x/hook'", "a-1", Path::new("/tmp/pw/hooks.sock"));
        assert_eq!(c, "PITWALL_AGENT_ID=a-1 PITWALL_SOCKET=/tmp/pw/hooks.sock sh '/x/hook'");
    }

    #[test]
    fn terminal_replies_are_the_new_lines_without_echo_or_chrome() {
        let before = "Welcome\n╭────╮\n│ >  │\n╰────╯";
        let after = "Welcome\n> where are tokens rotated?\nIn auth/refresh.rs, on every use.\nOld ones are revoked.\n╭────╮\n│ >  │\n╰────╯";
        let r = reply_from_screen(before, after, "where are tokens rotated?").unwrap();
        assert_eq!(r.text, "In auth/refresh.rs, on every use.\nOld ones are revoked.");
        assert_eq!(r.source, ReplySource::Terminal);
        assert!(reply_from_screen(before, before, "x").is_none());
    }

    #[test]
    fn hook_replies_need_text() {
        let p = serde_json::json!({"hook_event_name": "Stop", "last_assistant_message": " Done. "});
        assert_eq!(reply_from_hook(&p).unwrap().text, "Done.");
        assert!(reply_from_hook(&serde_json::json!({"hook_event_name": "Stop"})).is_none());
    }

    #[test]
    fn claude_limit_screens_give_the_reset_time() {
        let k = share_kind(&claude());
        assert_eq!(limit_on_screen(&k, "  5-hour limit reached ∙ resets 2pm\n"), Some(Some("2pm".into())));
        assert_eq!(limit_on_screen(&k, "Claude usage limit reached. Your limit will reset at 14:30 (Europe/Berlin)"), Some(Some("14:30".into())));
        assert_eq!(limit_on_screen(&k, "all good"), None);
        assert_eq!(limit_on_screen(&ShareKind::default(), "limit reached"), None);
    }
}
