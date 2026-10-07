//! Optional status hooks: the relay script, the local-socket listener it
//! posts to, how payloads map to statuses, and the per-agent hook
//! configuration.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use crate::model::CodexHooksStatus;
use crate::paths::{self, Paths};
use crate::platform::{LocalListener, LocalStream};
use crate::shell;

pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
];

const SCRIPT: &str = r#"#!/bin/sh
# Pitwall hook relay. Forwards the hook payload on stdin to the Pitwall app.
# Outside Pitwall (PITWALL_AGENT_ID / PITWALL_SOCKET unset) it does nothing.
# Never prints to stdout and always exits 0.
if [ -z "$PITWALL_AGENT_ID" ] || [ -z "$PITWALL_SOCKET" ] || [ ! -S "$PITWALL_SOCKET" ]; then
  cat >/dev/null 2>&1
  exit 0
fi
curl -s -m 2 --unix-socket "$PITWALL_SOCKET" \
  -H 'Content-Type: application/json' \
  -H 'Expect:' \
  --data-binary @- \
  "http://pitwall/hook/$PITWALL_AGENT_ID" >/dev/null 2>&1
exit 0
"#;

/// Put the hook relay at `paths.hook_script()`: the sh script above on Unix;
/// on Windows a copy of the `pitwall-hook` binary shipped next to Pitwall.
pub fn install_script(paths: &Paths) -> Result<(), String> {
    let path = paths.hook_script();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::platform::install_hook_relay(&path, SCRIPT)
}

/// The command hook configs run: `sh '<script>'` (Windows: the relay
/// binary's path, which bash, cmd and PowerShell all run).
pub fn hook_command(paths: &Paths) -> String {
    crate::platform::hook_relay_command(&paths.hook_script())
}

/// What follows `claude --settings`: the JSON itself, or — where the login
/// shell can't hand JSON to a program intact (PowerShell) — the path of a
/// file holding it (Claude Code takes either).
pub fn claude_settings_arg(command: &str) -> String {
    let json = claude_settings_json(command);
    if shell::LoginShell::current().passes_json() {
        return json;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let path = paths::Paths::new(paths::Paths::default_root()).root().join("run").join(format!("claude-hooks-{h:016x}.json"));
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
            std::fs::write(&path, &json)?;
        }
        Ok(())
    };
    match write() {
        Ok(()) => path.to_string_lossy().into_owned(),
        Err(_) => json,
    }
}

/// `claude --settings` value wiring every event to the relay script.
pub fn claude_settings_json(command: &str) -> String {
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        hooks.insert(
            (*event).to_string(),
            json!([{ "hooks": [{ "type": "command", "command": command }] }]),
        );
    }
    json!({ "hooks": hooks }).to_string()
}

// ---------------------------------------------------------------- mapping

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookState {
    Working,
    Blocked,
    Done,
    Idle,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookEffect {
    pub session_id: Option<String>,
    pub state: Option<HookState>,
    pub detail: Option<String>,
    /// SessionStart: only sets `Idle` if no hook status is known yet.
    pub session_start: bool,
    pub prompt_submitted: bool,
}

pub fn map_hook(payload: &Value) -> HookEffect {
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool = payload.get("tool_name").and_then(Value::as_str);
    let mut effect = HookEffect {
        session_id: payload
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
        ..Default::default()
    };
    match event {
        "SessionStart" => {
            effect.session_start = true;
            effect.state = Some(HookState::Idle);
        }
        "UserPromptSubmit" => {
            effect.prompt_submitted = true;
            effect.state = Some(HookState::Working);
        }
        "PostToolUse" => effect.state = Some(HookState::Working),
        "PreToolUse" if tool == Some("AskUserQuestion") => {
            effect.state = Some(HookState::Blocked);
            effect.detail = Some("Asking a question".into());
        }
        "PreToolUse" => effect.state = Some(HookState::Working),
        "PermissionRequest" => {
            effect.state = Some(HookState::Blocked);
            effect.detail = Some(match tool {
                Some(t) => format!("Permission: {t}"),
                None => "Permission needed".into(),
            });
        }
        "Stop" | "Interrupt" => effect.state = Some(HookState::Done),
        _ => {}
    }
    effect
}

// ---------------------------------------------------------------- HTTP

#[derive(Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 4 * 1024 * 1024;

/// Parse a complete request out of `buf`. `Ok(None)` = need more bytes.
pub fn parse_request(buf: &[u8]) -> Result<Option<Request>, String> {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if buf.len() > MAX_HEADER {
            Err("header too large".into())
        } else {
            Ok(None)
        };
    };
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| "header not utf-8")?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (Some(method), Some(path)) = (first.next(), first.next()) else {
        return Err("bad request line".into());
    };
    let mut length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().map_err(|_| "bad content-length")?;
            }
        }
    }
    if length > MAX_BODY {
        return Err("body too large".into());
    }
    let body_start = end + 4;
    if buf.len() < body_start + length {
        return Ok(None);
    }
    Ok(Some(Request {
        method: method.to_string(),
        path: path.to_string(),
        body: buf[body_start..body_start + length].to_vec(),
    }))
}

pub fn agent_id_from_path(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/hook/")?;
    let id = id.split(['?', '#']).next().unwrap_or("");
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// Bind the hook socket at `path` and serve forever on a background thread.
pub fn serve(path: &Path, on_hook: impl Fn(&str, Value) + Send + Sync + 'static) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let listener = LocalListener::bind(path).map_err(|e| format!("hook socket: {e}"))?;
    let on_hook = std::sync::Arc::new(on_hook);
    std::thread::Builder::new()
        .name("hook-socket".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let on_hook = on_hook.clone();
                std::thread::spawn(move || {
                    if let Some((id, payload)) = handle(stream) {
                        on_hook(&id, payload);
                    }
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn handle(mut stream: LocalStream) -> Option<(String, Value)> {
    stream.set_timeouts(Duration::from_secs(3));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let request = loop {
        match parse_request(&buf) {
            Ok(Some(req)) => break Some(req),
            Ok(None) => {}
            Err(_) => break None,
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let reply: &[u8] = match &request {
        Some(_) => b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n",
        None => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    };
    let _ = stream.write_all(reply);
    let request = request?;
    if request.method != "POST" {
        return None;
    }
    let id = agent_id_from_path(&request.path)?.to_string();
    let payload = serde_json::from_slice(&request.body).ok()?;
    Some((id, payload))
}

// ---------------------------------------------------------------- Codex

pub fn codex_hooks_path() -> std::path::PathBuf {
    paths::home().join(".codex").join("hooks.json")
}

fn has_command(entries: &Value, command: &str) -> bool {
    entries.as_array().is_some_and(|groups| {
        groups.iter().any(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hs| {
                    hs.iter()
                        .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
                })
        })
    })
}

/// Whether every Pitwall event already runs `command` in this hooks.json text.
pub fn codex_installed_in(existing: Option<&str>, command: &str) -> bool {
    let Some(src) = existing else { return false };
    let Ok(root) = serde_json::from_str::<Value>(src) else { return false };
    let Some(hooks) = root.get("hooks") else { return false };
    EVENTS
        .iter()
        .all(|e| hooks.get(*e).is_some_and(|v| has_command(v, command)))
}

/// Add Pitwall's command to every event, keeping all existing entries.
/// Returns `None` if nothing needs to change.
pub fn merge_codex_hooks(existing: Option<&str>, command: &str) -> Result<Option<String>, String> {
    let mut root: Value = match existing.map(str::trim) {
        None | Some("") => json!({}),
        Some(src) => serde_json::from_str(src).map_err(|e| format!("hooks.json is not valid JSON: {e}"))?,
    };
    let obj = root
        .as_object_mut()
        .ok_or("hooks.json is not a JSON object")?;
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("\"hooks\" in hooks.json is not an object")?;
    let mut changed = false;
    for event in EVENTS {
        let list = hooks.entry(*event).or_insert_with(|| json!([]));
        if has_command(list, command) {
            continue;
        }
        let arr = list
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} in hooks.json is not an array"))?;
        arr.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": 5 }] }));
        changed = true;
    }
    if !changed {
        return Ok(None);
    }
    serde_json::to_string_pretty(&root)
        .map(|s| Some(s + "\n"))
        .map_err(|e| e.to_string())
}

/// Back up then rewrite `path` with Pitwall's hooks merged in.
pub fn install_codex_hooks_at(path: &Path, command: &str) -> Result<(), String> {
    let existing = match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let Some(merged) = merge_codex_hooks(existing.as_deref(), command)? else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if let Some(src) = &existing {
        let backup = path.with_file_name("hooks.json.pitwall-backup");
        std::fs::write(&backup, src).map_err(|e| format!("could not back up hooks.json: {e}"))?;
    }
    let tmp = path.with_file_name("hooks.json.pitwall-tmp");
    std::fs::write(&tmp, merged).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn codex_status_at(path: &Path, command: &str) -> bool {
    codex_installed_in(std::fs::read_to_string(path).ok().as_deref(), command)
}

/// Whether `~/.codex/hooks.json` runs Pitwall's relay for every event.
pub fn codex_status(paths: &Paths) -> CodexHooksStatus {
    let path = codex_hooks_path();
    CodexHooksStatus {
        installed: codex_status_at(&path, &hook_command(paths)),
        path: path.to_string_lossy().into_owned(),
    }
}

/// Install the relay script and merge it into `~/.codex/hooks.json` (only on
/// the user's explicit request).
pub fn install_codex(paths: &Paths) -> Result<CodexHooksStatus, String> {
    install_script(paths)?;
    install_codex_hooks_at(&codex_hooks_path(), &hook_command(paths))?;
    Ok(codex_status(paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_request() {
        let raw = b"POST /hook/abc HTTP/1.1\r\nHost: pitwall\r\ncontent-length: 7\r\n\r\n{\"a\":1}";
        let req = parse_request(raw).unwrap().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/hook/abc");
        assert_eq!(req.body, b"{\"a\":1}");
    }

    #[test]
    fn waits_for_partial_request() {
        assert_eq!(parse_request(b"POST /hook/a HTTP/1.1\r\nContent-Le").unwrap(), None);
        assert_eq!(
            parse_request(b"POST /hook/a HTTP/1.1\r\nContent-Length: 10\r\n\r\n{}").unwrap(),
            None
        );
    }

    #[test]
    fn rejects_bad_requests() {
        assert!(parse_request(b"\r\n\r\n").is_err());
        assert!(parse_request(b"POST / HTTP/1.1\r\nContent-Length: x\r\n\r\n").is_err());
        assert!(parse_request(b"POST / HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n").is_err());
        assert!(parse_request(&vec![b'a'; MAX_HEADER + 1]).is_err());
    }

    #[test]
    fn no_body_without_length() {
        let req = parse_request(b"POST /hook/x HTTP/1.1\r\n\r\n").unwrap().unwrap();
        assert!(req.body.is_empty());
    }

    #[test]
    fn serves_a_hook_over_a_socket() {
        let (mut client, server) = LocalStream::pair().unwrap();
        let body = r#"{"hook_event_name":"Stop"}"#;
        let req = format!("POST /hook/a1 HTTP/1.1\r\nHost: pitwall\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        client.write_all(req.as_bytes()).unwrap();
        let (id, payload) = handle(server).unwrap();
        assert_eq!(id, "a1");
        assert_eq!(map_hook(&payload).state, Some(HookState::Done));
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 204"));
    }

    #[test]
    fn script_is_valid_sh_and_silent_outside_pitwall() {
        let dir = std::env::temp_dir().join(format!("pitwall-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("hook");
        std::fs::write(&script, SCRIPT).unwrap();
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env_remove("PITWALL_AGENT_ID")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(out.stdout.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn agent_ids_from_paths() {
        assert_eq!(agent_id_from_path("/hook/abc-1"), Some("abc-1"));
        assert_eq!(agent_id_from_path("/hook/abc?x=1"), Some("abc"));
        assert_eq!(agent_id_from_path("/hook/"), None);
        assert_eq!(agent_id_from_path("/hook/a/b"), None);
        assert_eq!(agent_id_from_path("/other/a"), None);
    }

    #[test]
    fn maps_hook_events() {
        let m = |v: Value| map_hook(&v);
        assert_eq!(m(json!({"hook_event_name":"UserPromptSubmit"})).state, Some(HookState::Working));
        assert!(m(json!({"hook_event_name":"UserPromptSubmit"})).prompt_submitted);
        assert_eq!(m(json!({"hook_event_name":"PostToolUse"})).state, Some(HookState::Working));
        assert_eq!(
            m(json!({"hook_event_name":"PreToolUse","tool_name":"Bash"})).state,
            Some(HookState::Working)
        );
        assert_eq!(
            m(json!({"hook_event_name":"PreToolUse","tool_name":"AskUserQuestion"})).state,
            Some(HookState::Blocked)
        );
        let perm = m(json!({"hook_event_name":"PermissionRequest","tool_name":"Edit"}));
        assert_eq!(perm.state, Some(HookState::Blocked));
        assert_eq!(perm.detail.as_deref(), Some("Permission: Edit"));
        assert_eq!(m(json!({"hook_event_name":"Stop"})).state, Some(HookState::Done));
        assert_eq!(m(json!({"hook_event_name":"Interrupt"})).state, Some(HookState::Done));
        let start = m(json!({"hook_event_name":"SessionStart","session_id":"s-1"}));
        assert!(start.session_start);
        assert_eq!(start.session_id.as_deref(), Some("s-1"));
        assert_eq!(m(json!({"hook_event_name":"Nope"})).state, None);
        assert_eq!(m(json!("garbage")), HookEffect::default());
    }

    #[test]
    fn claude_settings_cover_all_events() {
        let v: Value = serde_json::from_str(&claude_settings_json("sh '/x y/hook'")).unwrap();
        for e in EVENTS {
            assert_eq!(v["hooks"][e][0]["hooks"][0]["type"], "command");
            assert_eq!(v["hooks"][e][0]["hooks"][0]["command"], "sh '/x y/hook'");
        }
    }

    #[test]
    fn codex_merge_keeps_existing_and_is_idempotent() {
        let existing = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"other"}]}]},"x":1}"#;
        assert!(!codex_installed_in(Some(existing), "pw"));
        let merged = merge_codex_hooks(Some(existing), "pw").unwrap().unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["x"], 1);
        assert_eq!(v["hooks"]["SessionStart"][0]["hooks"][0]["command"], "other");
        assert_eq!(v["hooks"]["SessionStart"][1]["hooks"][0]["command"], "pw");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "pw");
        assert!(codex_installed_in(Some(&merged), "pw"));
        assert_eq!(merge_codex_hooks(Some(&merged), "pw").unwrap(), None);
    }

    #[test]
    fn codex_merge_handles_empty_and_invalid() {
        assert!(merge_codex_hooks(None, "pw").unwrap().is_some());
        assert!(merge_codex_hooks(Some("  "), "pw").unwrap().is_some());
        assert!(merge_codex_hooks(Some("{nope"), "pw").is_err());
        assert!(merge_codex_hooks(Some("[]"), "pw").is_err());
        assert!(merge_codex_hooks(Some(r#"{"hooks":{"Stop":{}}}"#), "pw").is_err());
    }

    #[test]
    fn codex_install_backs_up_on_temp_files() {
        let dir = std::env::temp_dir().join(format!("pitwall-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hooks.json");
        std::fs::write(&path, r#"{"hooks":{}}"#).unwrap();
        install_codex_hooks_at(&path, "pw").unwrap();
        assert!(codex_status_at(&path, "pw"));
        let backup = std::fs::read_to_string(dir.join("hooks.json.pitwall-backup")).unwrap();
        assert_eq!(backup, r#"{"hooks":{}}"#);
        let before = std::fs::read_to_string(&path).unwrap();
        install_codex_hooks_at(&path, "pw").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
