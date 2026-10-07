//! agw's `--output json` documents (schema_version 1, agw 0.19): an
//! envelope `{"schema_version", "command", "data": {<key>: …}}`. Parsing is
//! lenient: unknown fields are ignored, missing ones are `None`, and a bare
//! array or a top-level key is accepted too.

use serde_json::Value;

/// One VM of `agw vm list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vm {
    pub name: String,
    /// vm-site it lives at ("local-site", …).
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

/// `agw session describe <name> --output json` → `data.session`.
pub fn described(v: &Value) -> Option<Session> {
    let s = v.get("data").and_then(|d| d.get("session")).or_else(|| v.get("session")).unwrap_or(v);
    session(s)
}

pub fn sessions(v: &Value) -> Option<Vec<Session>> {
    Some(rows(v, "sessions")?.iter().filter_map(session).collect())
}

/// `agw workspace describe --output json` → the workspace's folder on its VM.
pub fn workspace_path(v: &Value) -> Option<String> {
    let w = v.get("data").and_then(|d| d.get("workspace")).or_else(|| v.get("workspace")).unwrap_or(v);
    text(w, "path")
}

/// agw's error message, without its "Error: " prefix: the `Error:` line
/// (agw prints notices and warnings before it), else the first line.
pub fn error_line(stderr: &str) -> String {
    let mut lines = stderr.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = stderr.lines().map(str::trim).find(|l| l.starts_with("Error:")).or_else(|| lines.next()).unwrap_or("agw failed");
    line.strip_prefix("Error:").map(str::trim).unwrap_or(line).to_string()
}

/// The `Hint:` agw prints after an error, if any.
pub fn hint_line(stderr: &str) -> Option<String> {
    let after = stderr.lines().map(str::trim).skip_while(|l| !l.starts_with("Error:"));
    after.filter_map(|l| l.strip_prefix("Hint:")).map(|h| h.trim().to_string()).find(|h| !h.is_empty())
}

/// One workspace of `agw workspace list` (or an agent of `agw agent list`):
/// its name and the template it was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub name: String,
    pub template: Option<String>,
}

fn named(v: &Value, key: &str) -> Option<Vec<Named>> {
    Some(rows(v, key)?.iter().filter_map(|r| Some(Named { name: text(r, "name")?, template: text(r, "template") })).collect())
}

/// `agw workspace list --vm <vm> --output json` → `data.workspaces`.
pub fn workspaces(v: &Value) -> Option<Vec<Named>> {
    named(v, "workspaces")
}

/// `agw agent list --vm <vm> --output json` → `data.agents`.
pub fn agents(v: &Value) -> Option<Vec<Named>> {
    named(v, "agents")
}

/// One entry of agw's resource registry (`agw resource list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// "session-template", "workspace-template", "agent-template", …
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    /// How many live things use it now.
    pub used_by: u64,
}

/// `agw resource list --kind … --output json` → `data.resources`, without
/// the disabled ones and those agw says aren't ready.
pub fn resources(v: &Value) -> Option<Vec<Resource>> {
    Some(
        rows(v, "resources")?
            .iter()
            .filter(|r| r.get("disabled").and_then(Value::as_bool) != Some(true))
            .filter(|r| r.get("not_ready_reason").is_none_or(Value::is_null))
            .filter_map(|r| {
                Some(Resource {
                    kind: text(r, "kind")?,
                    name: text(r, "name")?,
                    description: text(r, "description"),
                    used_by: r.get("used_by_count").and_then(Value::as_u64).unwrap_or(0),
                })
            })
            .collect(),
    )
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
        assert_eq!((s.vm.as_str(), s.name.as_str()), ("my-vm", "api-session"));
        assert_eq!(s.harness.as_deref(), Some("claude-code"));
        assert_eq!(s.workspace.as_deref(), Some("api-session"));
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
        assert_eq!(got, [Vm { name: "my-vm".into(), site: Some("local-site".into()) }]);
        let ws = fixture(include_str!("fixtures/workspace_describe.json"));
        assert_eq!(workspace_path(&ws).as_deref(), Some("/opt/agentworks/workspaces/api-session"));
        assert_eq!(workspace_path(&fixture("{}")), None);
    }

    #[test]
    fn errors_read_like_agw() {
        assert_eq!(error_line("Error: unknown VM 'nope'\n  Hint: Run 'agw vm list'"), "unknown VM 'nope'");
        assert_eq!(hint_line("Error: unknown VM 'nope'\n  Hint: Run 'agw vm list'").as_deref(), Some("Run 'agw vm list'"));
        assert_eq!(error_line("\n"), "agw failed");
        assert_eq!(hint_line("Error: x"), None);
        // Notices and warnings come first; the error is what counts.
        let err = "Notice: refreshing the VM\nWarning: slow\nError: session 'api' already exists\n";
        assert_eq!(error_line(err), "session 'api' already exists");
        assert_eq!(error_line("plain failure\nmore"), "plain failure");
    }

    #[test]
    fn workspaces_agents_and_templates() {
        let ws = workspaces(&fixture(include_str!("fixtures/workspace_list.json"))).unwrap();
        assert_eq!(ws, [Named { name: "api-session".into(), template: Some("api-session".into()) }, Named { name: "work".into(), template: Some("agentworks".into()) }]);
        assert!(agents(&fixture(include_str!("fixtures/agent_list_empty.json"))).unwrap().is_empty());
        let a = agents(&fixture(include_str!("fixtures/agent_list.json"))).unwrap();
        assert_eq!(a[0], Named { name: "bot".into(), template: Some("claude".into()) });
        let r = resources(&fixture(include_str!("fixtures/resource_list.json"))).unwrap();
        let of = |k: &str| r.iter().filter(|x| x.kind == k).map(|x| x.name.as_str()).collect::<Vec<_>>();
        assert!(of("session-template").contains(&"claude") && of("session-template").contains(&"default"));
        assert!(of("workspace-template").contains(&"agentworks"));
        assert!(of("agent-template").contains(&"default"));
        let filtered = resources(&fixture(
            r#"{"data":{"resources":[{"kind":"session-template","name":"ok","not_ready_reason":null},{"kind":"session-template","name":"off","disabled":true},{"kind":"session-template","name":"broken","not_ready_reason":"missing secret"}]}}"#,
        ))
        .unwrap();
        assert_eq!(filtered.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["ok"], "disabled and not-ready ones are left out");
        let claude = r.iter().find(|x| x.kind == "session-template" && x.name == "claude").unwrap();
        assert_eq!((claude.description.as_deref(), claude.used_by), (Some("Claude Code interactive session"), 2));
    }
}
