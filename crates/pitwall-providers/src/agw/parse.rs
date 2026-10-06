//! agw's `--output json` documents (schema_version 1, agw 0.19): an
//! envelope `{"schema_version", "command", "data": {<key>: …}}`. Parsing is
//! lenient: unknown fields are ignored, missing ones are `None`, and a bare
//! array or a top-level key is accepted too.

use serde_json::Value;

/// One VM of `agw vm list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vm {
    pub name: String,
    /// vm-site it lives at ("lima-local", …).
    pub site: Option<String>,
}

/// Live state as agw reports it (`--status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    Stopped,
    /// "unavailable" (no `--status`, or the VM couldn't be asked).
    Unknown,
}

/// One session of `agw session list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub name: String,
    pub vm: String,
    pub workspace: Option<String>,
    /// agw's harness integration ("claude-code", "codex", "shell", …),
    /// else its session template.
    pub harness: Option<String>,
    /// The agent (its own Linux user) it runs as; `None`: the VM's admin user.
    pub agent: Option<String>,
    pub status: Status,
}

/// The JSON document in a command's output (a banner may come first).
pub fn json(out: &str) -> Option<Value> {
    let start = out.find(['[', '{'])?;
    serde_json::from_str(out[start..].trim()).ok()
}

/// `key` array of an agw `--output json` envelope.
fn rows<'a>(v: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    v.as_array()
        .or_else(|| v.get(key).and_then(Value::as_array))
        .or_else(|| v.get("data").and_then(|d| d.get(key)).and_then(Value::as_array))
}

fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

pub fn vms(v: &Value) -> Option<Vec<Vm>> {
    Some(rows(v, "vms")?.iter().filter_map(|r| Some(Vm { name: text(r, "name")?, site: text(r, "site") })).collect())
}

fn session(r: &Value) -> Option<Session> {
    let status = match text(r, "status").map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("running") => Status::Running,
        Some("stopped") => Status::Stopped,
        _ => Status::Unknown,
    };
    // Agent mode names its agent; admin mode has none.
    let agent = text(r, "agent_name").filter(|_| text(r, "mode").as_deref() != Some("admin"));
    Some(Session {
        name: text(r, "name")?,
        vm: text(r, "vm_name").unwrap_or_default(),
        workspace: text(r, "workspace_name"),
        harness: text(r, "harness_integration").or_else(|| text(r, "template")),
        agent,
        status,
    })
}

pub fn sessions(v: &Value) -> Option<Vec<Session>> {
    Some(rows(v, "sessions")?.iter().filter_map(session).collect())
}

/// `agw workspace describe --output json` → the workspace's folder on its VM.
pub fn workspace_path(v: &Value) -> Option<String> {
    let w = v.get("data").and_then(|d| d.get("workspace")).or_else(|| v.get("workspace")).unwrap_or(v);
    text(w, "path")
}

/// The first line of agw's error output, without its "Error: " prefix.
pub fn error_line(stderr: &str) -> String {
    let line = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("agw failed");
    line.strip_prefix("Error:").map(str::trim).unwrap_or(line).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(src: &str) -> Value {
        json(src).expect("fixture is JSON")
    }

    #[test]
    fn session_list() {
        let got = sessions(&fixture(include_str!("fixtures/session_list.json"))).unwrap();
        assert_eq!(got.len(), 2);
        let s = &got[0];
        assert_eq!((s.vm.as_str(), s.name.as_str()), ("shakedown", "gwlive"));
        assert_eq!(s.harness.as_deref(), Some("claude-code"));
        assert_eq!(s.workspace.as_deref(), Some("gwlive"));
        assert_eq!((s.agent.as_deref(), s.status), (None, Status::Running));
        assert_eq!(got[1].status, Status::Stopped);
        // Without `--status` agw reports "unavailable".
        let plain = sessions(&fixture(include_str!("fixtures/session_list_nostatus.json"))).unwrap();
        assert!(plain.iter().all(|s| s.status == Status::Unknown));
        // A banner before the JSON, other harnesses, agent mode, a template only.
        let v = fixture(
            "Deprecated: x\n{\"data\":{\"sessions\":[{\"name\":\"x\",\"vm_name\":\"dev\",\"harness_integration\":\"codex\",\"mode\":\"agent\",\"agent_name\":\"bot\",\"status\":\"running\"},{\"name\":\"y\",\"vm_name\":\"dev\",\"template\":\"login-shell\"},{\"vm_name\":\"nameless\"}]}}",
        );
        let got = sessions(&v).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].harness.as_deref(), got[0].agent.as_deref()), (Some("codex"), Some("bot")));
        assert_eq!((got[1].harness.as_deref(), got[1].status), (Some("login-shell"), Status::Unknown));
        assert!(json("oops").is_none());
        assert!(sessions(&fixture("{\"data\":{}}")).is_none());
    }

    #[test]
    fn vm_list_and_workspaces() {
        let got = vms(&fixture(include_str!("fixtures/vm_list.json"))).unwrap();
        assert_eq!(got, [Vm { name: "shakedown".into(), site: Some("lima-local".into()) }]);
        let ws = fixture(include_str!("fixtures/workspace_describe.json"));
        assert_eq!(workspace_path(&ws).as_deref(), Some("/opt/agentworks/workspaces/gwlive"));
        assert_eq!(workspace_path(&fixture("{}")), None);
    }

    #[test]
    fn errors_read_like_agw() {
        assert_eq!(error_line("Error: unknown VM 'nope'\n  Hint: Run 'agw vm list'"), "unknown VM 'nope'");
        assert_eq!(error_line("\n"), "agw failed");
    }
}
