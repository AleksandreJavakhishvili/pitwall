//! New agw sessions from Pitwall (step 8 slice c): the New-agent form for a
//! VM, from agw's own read-only listings, and the `agw session create`
//! command its answers become.
//!
//! agw 0.19 (`agw session create --help`, agentworks' `sessions/manager/
//! _create_plan.py`): a session is created *and started* in one workspace —
//! an existing one (`--workspace`) or a new one (`--new-workspace
//! [--workspace-name] [--workspace-template]`) — as the VM's admin user
//! (`--admin`), an existing agent user (`--agent`) or a new one
//! (`--new-agent [--agent-name] [--agent-template]`), from a session
//! template (`--template`, agw's `default` when omitted). A new workspace or
//! agent is named after the session unless named. `--vm` is always passed,
//! and `--non-interactive` (every agw call) means agw never prompts: a
//! missing choice is an error, not a question.
//!
//! Names follow agw's one rule (agentworks `naming.validate_name`):
//! lowercase letters, digits, `-` and `_`, starting and ending with a letter
//! or digit, no `--`; at most 34 characters for a session (its tmux socket
//! path), 29 for a workspace (Linux group `ws-<name>`), 28 for an agent
//! (Linux user `agt-<name>`), 64 for anything else (templates).

use std::collections::BTreeMap;
use std::time::Duration;

use pitwall_core::provider::{CreateChoice, CreateField, CreateForm, NameRule, PwError, Result, CREATE_NEW};

use super::parse::{Named, Resource};

/// Each read-only listing behind the form.
pub const OPTIONS_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a VM's form is reused (the dialog opens, the user picks).
pub const FORM_TTL: Duration = Duration::from_secs(30);
/// `agw session create`: a new workspace clones its repositories and a new
/// agent user gets its tools installed, so this can take minutes.
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub const MAX_SESSION: usize = 34;
pub const MAX_WORKSPACE: usize = 29;
pub const MAX_AGENT: usize = 28;
pub const MAX_OTHER: usize = 64;

/// The form's field ids (the keys of `CreateSpec::options`).
pub mod field {
    pub const WORKSPACE: &str = "workspace";
    pub const WORKSPACE_NAME: &str = "workspaceName";
    pub const WORKSPACE_TEMPLATE: &str = "workspaceTemplate";
    pub const RUN_AS: &str = "runAs";
    pub const AGENT_NAME: &str = "agentName";
    pub const AGENT_TEMPLATE: &str = "agentTemplate";
    pub const TEMPLATE: &str = "template";
}

/// `runAs` values: the admin user, `agent:<name>`, or [`CREATE_NEW`].
pub const ADMIN: &str = "admin";
pub const AGENT_PREFIX: &str = "agent:";

/// agw's name rule, `max` long at most.
pub fn name_rule(max: usize) -> NameRule {
    NameRule {
        pattern: "^(?!.*--)[a-z0-9]([a-z0-9_-]*[a-z0-9])?$".into(),
        max_len: max as u32,
        hint: format!("Lowercase letters, digits, - or _; starts and ends with a letter or digit; no --; max {max}"),
    }
}

/// Whether `name` follows agw's rule (`double_hyphen`: an existing,
/// older name may contain `--`).
pub fn check_name(what: &str, name: &str, max: usize, double_hyphen: bool) -> Result<()> {
    let b = name.as_bytes();
    let edge = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    let ok = !b.is_empty()
        && edge(b[0])
        && edge(b[b.len() - 1])
        && b.iter().all(|&c| edge(c) || c == b'-' || c == b'_')
        && (double_hyphen || !name.contains("--"));
    if name.len() > max {
        return Err(PwError::other(format!("{what} \"{name}\" is too long ({} characters, agw allows {max})", name.len())));
    }
    if !ok {
        return Err(PwError::other(format!(
            "{what} \"{name}\" isn't a valid agw name: lowercase letters, digits, - or _, starting and ending with a letter or digit, no --"
        )));
    }
    Ok(())
}

/// What the VM's listings said (each may have failed on its own).
pub struct Listings {
    pub workspaces: std::result::Result<Vec<Named>, String>,
    pub agents: std::result::Result<Vec<Named>, String>,
    /// Session, workspace and agent templates.
    pub templates: std::result::Result<Vec<Resource>, String>,
}

fn choice(value: &str, label: &str, detail: Option<String>, phrase: &str) -> CreateChoice {
    CreateChoice { value: value.into(), label: label.into(), detail, phrase: Some(phrase.into()), creates: None }
}

/// Templates of `kind` as choices, the most used first after agw's
/// `default`; when they couldn't be listed, just `default` (agw always has
/// one).
fn template_choices(templates: &std::result::Result<Vec<Resource>, String>, kind: &str, phrase: impl Fn(&str) -> String) -> Vec<CreateChoice> {
    let mut list: Vec<&Resource> = templates.as_ref().map(|t| t.iter().filter(|r| r.kind == kind).collect()).unwrap_or_default();
    list.sort_by(|a, b| b.used_by.cmp(&a.used_by).then(a.name.cmp(&b.name)));
    let mut out: Vec<CreateChoice> = list.iter().map(|r| choice(&r.name, &r.name, r.description.clone(), &phrase(&r.name))).collect();
    if out.is_empty() {
        out.push(choice("default", "default", None, &phrase("default")));
    }
    out
}

/// A template field's default: the one in use most (what this VM's
/// sessions already run), else agw's `default`.
fn template_default(choices: &[CreateChoice], templates: &std::result::Result<Vec<Resource>, String>, kind: &str) -> Option<String> {
    let used = templates.as_ref().ok().and_then(|t| t.iter().filter(|r| r.kind == kind && r.used_by > 0).max_by_key(|r| r.used_by));
    used.map(|r| r.name.clone())
        .or_else(|| choices.iter().find(|c| c.value == "default").map(|c| c.value.clone()))
        .or_else(|| choices.first().map(|c| c.value.clone()))
}

/// The New-agent form for `vm`.
pub fn form(vm: &str, l: &Listings) -> CreateForm {
    use field::*;
    let mut errors = Vec::new();
    for (what, r) in [("workspaces", l.workspaces.as_ref().err()), ("agents", l.agents.as_ref().err()), ("templates", l.templates.as_ref().err())] {
        if let Some(e) = r {
            errors.push(format!("couldn't list {what}: {e}"));
        }
    }

    let mut ws: Vec<CreateChoice> = l
        .workspaces
        .as_ref()
        .map(|w| w.iter().map(|w| choice(&w.name, &w.name, w.template.as_ref().map(|t| format!("template {t}")), &format!("in workspace {}", w.name))).collect())
        .unwrap_or_default();
    let ws_default = ws.first().map(|c| c.value.clone()).unwrap_or_else(|| CREATE_NEW.into());
    ws.push(CreateChoice {
        value: CREATE_NEW.into(),
        label: "New workspace…".into(),
        detail: None,
        phrase: Some("in a new workspace {workspaceName}".into()),
        creates: Some("workspace {workspaceName} (template {workspaceTemplate})".into()),
    });
    let mut workspace = CreateField::select(WORKSPACE, "Workspace", ws, Some(ws_default));
    workspace.hint = Some("Where it works on the VM: its repositories and environment".into());
    let mut ws_name = CreateField::text(WORKSPACE_NAME, "Workspace name", name_rule(MAX_WORKSPACE)).when(WORKSPACE, CREATE_NEW);
    ws_name.defaults_to_name = true;
    ws_name.placeholder = Some("same as the session".into());
    let ws_templates = template_choices(&l.templates, "workspace-template", |t| t.to_string());
    let ws_template_default = template_default(&ws_templates, &Ok(vec![]), "workspace-template");
    let ws_template = CreateField::select(WORKSPACE_TEMPLATE, "Workspace template", ws_templates, ws_template_default).when(WORKSPACE, CREATE_NEW);

    let mut run_as = vec![choice(ADMIN, "Admin user", Some("the VM's own user".into()), "as the admin user")];
    if let Ok(agents) = &l.agents {
        for a in agents {
            run_as.push(choice(
                &format!("{AGENT_PREFIX}{}", a.name),
                &format!("Agent {}", a.name),
                Some(format!("Linux user agt-{}", a.name)),
                &format!("as agent {}", a.name),
            ));
        }
    }
    run_as.push(CreateChoice {
        value: CREATE_NEW.into(),
        label: "New agent user…".into(),
        detail: Some("its own Linux user, isolated from the admin".into()),
        phrase: Some("as a new agent {agentName}".into()),
        creates: Some("agent user agt-{agentName} (template {agentTemplate})".into()),
    });
    let mut run_as = CreateField::select(RUN_AS, "Runs as", run_as, Some(ADMIN.into()));
    run_as.hint = Some("The Linux user on the VM it runs as (agw's admin or agent users)".into());
    let mut agent_name = CreateField::text(AGENT_NAME, "Agent name", name_rule(MAX_AGENT)).when(RUN_AS, CREATE_NEW);
    agent_name.defaults_to_name = true;
    agent_name.placeholder = Some("same as the session".into());
    let agent_templates = template_choices(&l.templates, "agent-template", |t| t.to_string());
    let agent_template_default = template_default(&agent_templates, &Ok(vec![]), "agent-template");
    let agent_template = CreateField::select(AGENT_TEMPLATE, "Agent template", agent_templates, agent_template_default).when(RUN_AS, CREATE_NEW);

    let templates = template_choices(&l.templates, "session-template", |t| format!("with session template {t}"));
    let template_def = template_default(&templates, &l.templates, "session-template");
    let mut template = CreateField::select(TEMPLATE, "Session template", templates, template_def);
    template.hint = Some("What runs in the session (Claude Code, Codex, a shell, …)".into());

    CreateForm {
        provider: String::new(),
        machine: vm.into(),
        machine_label: vm.into(),
        folder: false,
        name: name_rule(MAX_SESSION),
        fields: vec![workspace, ws_name, ws_template, run_as, agent_name, agent_template, template],
        summary: Some(format!("Creates and starts session {{name}} on {vm} {{workspace}} {{runAs}}, {{template}}.")),
        submit: "Create".into(),
        error: (!errors.is_empty()).then(|| errors.join("; ")),
    }
}

/// `agw session create` for session `name` on `vm` with the form's values
/// (after `agw --non-interactive`; options before the name).
pub fn args(name: &str, vm: &str, options: &BTreeMap<String, String>) -> Result<Vec<String>> {
    use field::*;
    if let Some(k) = options.keys().find(|k| ![WORKSPACE, WORKSPACE_NAME, WORKSPACE_TEMPLATE, RUN_AS, AGENT_NAME, AGENT_TEMPLATE, TEMPLATE].contains(&k.as_str())) {
        return Err(PwError::other(format!("agw sessions have no option \"{k}\"")));
    }
    check_name("session name", name, MAX_SESSION, false)?;
    let get = |k: &str| options.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let only_with = |k: &str, cond: bool, needs: &str| {
        if get(k).is_some() && !cond {
            return Err(PwError::other(format!("{k} only applies to {needs}")));
        }
        Ok(())
    };
    let mut a: Vec<String> = ["session", "create", "--vm", vm].map(String::from).to_vec();
    let mut push = |flag: &str, v: &str| {
        a.push(flag.into());
        a.push(v.into());
    };

    let new_ws = get(WORKSPACE) == Some(CREATE_NEW);
    only_with(WORKSPACE_NAME, new_ws, "a new workspace")?;
    only_with(WORKSPACE_TEMPLATE, new_ws, "a new workspace")?;
    let mut flags: Vec<String> = Vec::new();
    match get(WORKSPACE) {
        None => return Err(PwError::other("choose a workspace (an existing one, or a new one)")),
        Some(CREATE_NEW) => {
            flags.push("--new-workspace".into());
            if let Some(n) = get(WORKSPACE_NAME) {
                check_name("workspace name", n, MAX_WORKSPACE, false)?;
                push("--workspace-name", n);
            }
            if let Some(t) = get(WORKSPACE_TEMPLATE) {
                check_name("workspace template", t, MAX_OTHER, false)?;
                push("--workspace-template", t);
            }
        }
        Some(ws) => {
            check_name("workspace", ws, MAX_OTHER, true)?;
            push("--workspace", ws);
        }
    }

    let new_agent = get(RUN_AS) == Some(CREATE_NEW);
    only_with(AGENT_NAME, new_agent, "a new agent user")?;
    only_with(AGENT_TEMPLATE, new_agent, "a new agent user")?;
    match get(RUN_AS) {
        None => return Err(PwError::other("choose who it runs as (the admin user or an agent)")),
        Some(ADMIN) => flags.push("--admin".into()),
        Some(CREATE_NEW) => {
            flags.push("--new-agent".into());
            if let Some(n) = get(AGENT_NAME) {
                check_name("agent name", n, MAX_AGENT, false)?;
                push("--agent-name", n);
            }
            if let Some(t) = get(AGENT_TEMPLATE) {
                check_name("agent template", t, MAX_OTHER, false)?;
                push("--agent-template", t);
            }
        }
        Some(other) => match other.strip_prefix(AGENT_PREFIX) {
            Some(agent) => {
                check_name("agent", agent, MAX_OTHER, true)?;
                push("--agent", agent);
            }
            None => return Err(PwError::other(format!("runAs \"{other}\": expected admin, agent:<name> or {CREATE_NEW}"))),
        },
    }

    if let Some(t) = get(TEMPLATE) {
        check_name("session template", t, MAX_OTHER, false)?;
        push("--template", t);
    }
    a.extend(flags);
    a.push(name.into());
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agw::parse;

    fn opts(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn argv(name: &str, kv: &[(&str, &str)]) -> Vec<String> {
        args(name, "my-vm", &opts(kv)).unwrap()
    }

    fn err(name: &str, kv: &[(&str, &str)]) -> String {
        args(name, "my-vm", &opts(kv)).unwrap_err().message
    }

    #[test]
    fn every_combination_maps_to_agws_flags() {
        let base = ["session", "create", "--vm", "my-vm"];
        let with = |rest: &[&str]| base.iter().chain(rest).map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(argv("api-fix", &[("workspace", "work"), ("runAs", "admin")]), with(&["--workspace", "work", "--admin", "api-fix"]));
        assert_eq!(
            argv("api-fix", &[("workspace", "work"), ("runAs", "admin"), ("template", "claude")]),
            with(&["--workspace", "work", "--template", "claude", "--admin", "api-fix"])
        );
        assert_eq!(argv("s", &[("workspace", "work"), ("runAs", "agent:bot")]), with(&["--workspace", "work", "--agent", "bot", "s"]));
        assert_eq!(argv("s", &[("workspace", "+new"), ("runAs", "admin")]), with(&["--new-workspace", "--admin", "s"]));
        assert_eq!(
            argv("s", &[("workspace", "+new"), ("workspaceName", "scratch"), ("workspaceTemplate", "api-session"), ("runAs", "admin")]),
            with(&["--workspace-name", "scratch", "--workspace-template", "api-session", "--new-workspace", "--admin", "s"])
        );
        assert_eq!(argv("s", &[("workspace", "work"), ("runAs", "+new")]), with(&["--workspace", "work", "--new-agent", "s"]));
        assert_eq!(
            argv("s", &[("workspace", "+new"), ("runAs", "+new"), ("agentName", "helper"), ("agentTemplate", "claude_light"), ("template", "codex")]),
            with(&["--agent-name", "helper", "--agent-template", "claude_light", "--template", "codex", "--new-workspace", "--new-agent", "s"])
        );
        // Empty text is agw's default (the session's name).
        assert_eq!(argv("s", &[("workspace", "+new"), ("workspaceName", " "), ("runAs", "admin")]), with(&["--new-workspace", "--admin", "s"]));
        // An older workspace or agent name with `--` can still be used.
        assert_eq!(argv("s", &[("workspace", "old--ws"), ("runAs", "agent:a--b")]), with(&["--workspace", "old--ws", "--agent", "a--b", "s"]));
    }

    #[test]
    fn bad_input_never_reaches_agw() {
        let ok = [("workspace", "work"), ("runAs", "admin")];
        assert!(err("Api", &ok).contains("isn't a valid agw name"));
        assert!(err("-rf", &ok).contains("isn't a valid agw name"));
        assert!(err("a--b", &ok).contains("no --"));
        assert!(err("api-", &ok).contains("isn't a valid agw name"));
        assert!(err(&"a".repeat(35), &ok).contains("too long (35 characters, agw allows 34)"));
        assert!(args(&"a".repeat(34), "vm", &opts(&ok)).is_ok());
        assert!(err("s", &[("runAs", "admin")]).contains("choose a workspace"));
        assert!(err("s", &[("workspace", "work")]).contains("choose who it runs as"));
        assert!(err("s", &[("workspace", "work"), ("runAs", "root")]).contains("expected admin, agent:<name>"));
        assert!(err("s", &[("workspace", "--admin"), ("runAs", "admin")]).contains("isn't a valid agw name"));
        assert!(err("s", &[("workspace", "work"), ("runAs", "admin"), ("template", "--spec")]).contains("isn't a valid agw name"));
        assert!(err("s", &[("workspace", "work"), ("workspaceName", "x"), ("runAs", "admin")]).contains("only applies to a new workspace"));
        assert!(err("s", &[("workspace", "work"), ("runAs", "admin"), ("agentName", "x")]).contains("only applies to a new agent user"));
        assert!(err("s", &[("workspace", "+new"), ("workspaceName", &"w".repeat(30)), ("runAs", "admin")]).contains("agw allows 29"));
        assert!(err("s", &[("workspace", "work"), ("runAs", "+new"), ("agentName", &"w".repeat(29))]).contains("agw allows 28"));
        assert!(err("s", &[("workspace", "work"), ("runAs", "admin"), ("spec", "{}")]).contains("no option \"spec\""));
    }

    fn listings() -> Listings {
        let j = |s: &str| parse::json(s).unwrap();
        Listings {
            workspaces: Ok(parse::workspaces(&j(include_str!("fixtures/workspace_list.json"))).unwrap()),
            agents: Ok(parse::agents(&j(include_str!("fixtures/agent_list.json"))).unwrap()),
            templates: Ok(parse::resources(&j(include_str!("fixtures/resource_list.json"))).unwrap()),
        }
    }

    #[test]
    fn the_form_lists_what_the_vm_has() {
        let f = form("my-vm", &listings());
        assert!(!f.folder && f.error.is_none());
        assert_eq!((f.name.max_len, f.submit.as_str()), (34, "Create"));
        let values = |id: &str| f.field(id).unwrap().choices.iter().map(|c| c.value.clone()).collect::<Vec<_>>();
        assert_eq!(values("workspace"), ["api-session", "work", "+new"]);
        assert_eq!(values("runAs"), ["admin", "agent:bot", "+new"]);
        assert!(values("template").contains(&"claude".to_string()) && values("template").contains(&"default".to_string()));
        assert!(values("workspaceTemplate").contains(&"agentworks".to_string()));
        assert!(values("agentTemplate").contains(&"claude_light".to_string()));
        // Defaults: the first workspace, the admin, the session template in use.
        let v = f.values(&BTreeMap::new()).unwrap();
        assert_eq!(v, opts(&[("workspace", "api-session"), ("runAs", "admin"), ("template", "claude")]));
        let s = f.summarize("api-fix", &f.values(&opts(&[("workspace", "work")])).unwrap());
        assert_eq!(s.text, "Creates and starts session api-fix on my-vm in workspace work as the admin user, with session template claude.");
        assert!(s.creates.is_empty());
        // Extra things made on the VM are spelled out.
        let v = f.values(&opts(&[("workspace", "+new"), ("runAs", "+new"), ("agentName", "helper")])).unwrap();
        assert_eq!(v.get("workspaceTemplate").map(String::as_str), Some("default"));
        let s = f.summarize("api-fix", &v);
        assert_eq!(s.text, "Creates and starts session api-fix on my-vm in a new workspace api-fix as a new agent helper, with session template claude.");
        assert_eq!(s.creates, ["workspace api-fix (template default)", "agent user agt-helper (template default)"]);
        // Every default and every choice turns into a valid command.
        assert!(args("api-fix", "my-vm", &v).is_ok());
    }

    #[test]
    fn a_failed_listing_leaves_a_usable_form() {
        let l = Listings { workspaces: Err("VM is stopped".into()), agents: Err("VM is stopped".into()), templates: Err("timed out".into()) };
        let f = form("vm", &l);
        assert_eq!(f.error.as_deref(), Some("couldn't list workspaces: VM is stopped; couldn't list agents: VM is stopped; couldn't list templates: timed out"));
        let v = f.values(&BTreeMap::new()).unwrap();
        assert_eq!(v, opts(&[("workspace", "+new"), ("workspaceTemplate", "default"), ("runAs", "admin"), ("template", "default")]));
    }
}
