//! `pitwall`: add agents and sessions to Pitwall from a terminal.
//!
//! Output is JSON on stdout (the call's result); `--human` prints text.
//! Errors are `{"error":{"code","message"}}` on stderr. Exit status: 0 ok,
//! 1 error, 2 usage, 3 the user denied (or didn't answer) an approval.

#![recursion_limit = "256"]

mod args;
mod render;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use serde_json::{json, Value};

use args::{AgentCmd, Cli, Command, MachineCmd, SessionCmd};
use pitwall_client::{Client, Error};
use pitwall_proto::{code, AgentCreate, ErrorBody, FormRequest, ProviderMachines, SessionAdd, SessionFilter, CREATE_NEW};

/// What a command produced: its JSON result and the same as text.
pub struct Output {
    pub json: Value,
    pub human: String,
}

fn out<T: serde::Serialize>(v: &T, human: String) -> Output {
    Output { json: serde_json::to_value(v).unwrap_or(Value::Null), human }
}

/// A folder as the server needs it: absolute (`~` is left for the server,
/// which knows the target machine's home).
pub fn absolute(project: Option<&str>, cwd: &Path) -> String {
    match project.map(str::trim).filter(|p| !p.is_empty()) {
        None => cwd.to_string_lossy().into_owned(),
        Some(p) if p == "~" || p.starts_with("~/") => p.to_string(),
        Some(p) => {
            let path = cwd.join(p);
            std::fs::canonicalize(&path).unwrap_or(path).to_string_lossy().into_owned()
        }
    }
}

/// `agent new --machine`'s flags as the machine's form fields. The flags
/// are shorthands for the usual field ids (`machine form` lists a
/// machine's own); `--option FIELD=VALUE` sets any field.
pub fn machine_options(a: &args::AgentNew) -> Result<BTreeMap<String, String>, String> {
    let mut o = BTreeMap::new();
    let mut set = |k: &str, v: &str| {
        o.insert(k.to_string(), v.to_string());
    };
    if let Some(w) = &a.workspace {
        set("workspace", w);
    }
    if let Some(n) = &a.new_workspace {
        set("workspace", CREATE_NEW);
        if !n.is_empty() {
            set("workspaceName", n);
        }
    }
    if let Some(t) = &a.workspace_template {
        set("workspaceTemplate", t);
    }
    match a.run_as.as_deref() {
        None => {}
        Some(w) if w == "new-agent" || w.starts_with("new-agent:") => {
            set("runAs", CREATE_NEW);
            if let Some(n) = w.strip_prefix("new-agent:").filter(|n| !n.is_empty()) {
                set("agentName", n);
            }
        }
        Some(w) if w == "admin" || w.starts_with("agent:") => set("runAs", w),
        Some(w) => return Err(format!("--as {w}: expected admin, agent:<name> or new-agent[:<name>]")),
    }
    if let Some(t) = &a.agent_template {
        set("agentTemplate", t);
    }
    if let Some(t) = &a.template {
        set("template", t);
    }
    for kv in &a.options {
        let (k, v) = kv.split_once('=').ok_or_else(|| format!("--option {kv}: expected FIELD=VALUE"))?;
        set(k.trim(), v.trim());
    }
    Ok(o)
}

/// The provider that has `machine`.
fn provider_of(list: &[ProviderMachines], machine: &str) -> Result<String, String> {
    let found: Vec<_> = list.iter().filter(|p| p.machines.iter().any(|m| m.id == machine)).map(|p| p.provider.clone()).collect();
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("no machine \"{machine}\" (see `pitwall machine list`)")),
        _ => Err(format!("\"{machine}\" is a machine of {}: pass --provider", found.join(" and "))),
    }
}

/// Run `cmd` against the Pitwall listening at `socket`.
pub fn run(cmd: &Command, socket: &Path) -> Result<Output, Error> {
    let mut c = Client::connect_as(socket, &format!("pitwall-cli/{}", env!("CARGO_PKG_VERSION")), pitwall_proto::Role::Cli)?;
    Ok(match cmd {
        Command::Agent(AgentCmd::List) => {
            let list = c.agents()?;
            out(&list, render::agents(&list))
        }
        Command::Agent(AgentCmd::New(a)) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            let options = machine_options(a).map_err(|e| Error::Server(ErrorBody::new(code::BAD_PARAMS, e)))?;
            let view = c.create_agent(&AgentCreate {
                kind: a.kind.clone().unwrap_or_default(),
                project: if a.machine.is_some() { String::new() } else { absolute(a.project.as_deref(), &cwd) },
                name: a.name.clone(),
                resume: a.resume.clone(),
                provider: a.provider.clone(),
                machine: a.machine.clone(),
                options,
                cols: None,
                rows: None,
            })?;
            out(&view, render::agent_new(&view))
        }
        Command::Machine(MachineCmd::List) => {
            let list = c.machines()?;
            out(&list, render::machines(&list))
        }
        Command::Machine(MachineCmd::Form { machine, provider }) => {
            let provider = match provider {
                Some(p) => p.clone(),
                None => provider_of(&c.machines()?, machine).map_err(|e| Error::Server(ErrorBody::new(code::NOT_FOUND, e)))?,
            };
            let form = c.create_form(&FormRequest { provider, machine: machine.clone() })?;
            out(&form, render::form(&form))
        }
        Command::Session(SessionCmd::List { provider, machine }) => {
            let list = c.sessions(&SessionFilter { provider: provider.clone(), machine: machine.clone() })?;
            out(&list, render::sessions(&list))
        }
        Command::Session(SessionCmd::Add { provider, machine, name, start }) => {
            let added = c.add_session(&SessionAdd {
                provider: provider.clone(),
                machine: machine.clone(),
                native: name.clone(),
                start: *start,
                cols: None,
                rows: None,
            })?;
            out(&added, render::session_added(&added))
        }
    })
}

/// The error as JSON, and the exit status for it.
pub fn failure(e: &Error) -> (Value, u8) {
    let body = match e {
        Error::Server(b) => b.clone(),
        Error::Connect { .. } => ErrorBody::new("not_connected", e.to_string()),
        Error::Rejected(_) => ErrorBody::new(code::INCOMPATIBLE, e.to_string()),
        Error::Io(_) | Error::Protocol(_) => ErrorBody::new("connection", e.to_string()),
    };
    let status = if body.code == code::DENIED || body.code == code::APPROVAL_TIMEOUT { 3 } else { 1 };
    (json!({ "error": body }), status)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let socket = cli.socket.clone().unwrap_or_else(pitwall_client::socket_path);
    match run(&cli.command, &socket) {
        Ok(o) if cli.human => {
            print!("{}", o.human);
            ExitCode::SUCCESS
        }
        Ok(o) => {
            println!("{}", serde_json::to_string_pretty(&o.json).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => {
            let (body, status) = failure(&e);
            if cli.human {
                eprintln!("pitwall: {e}");
            } else {
                eprintln!("{body}");
            }
            ExitCode::from(status)
        }
    }
}

#[cfg(test)]
mod tests;
