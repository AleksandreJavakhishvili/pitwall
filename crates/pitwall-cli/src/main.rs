//! `pitwall`: add agents and sessions to Pitwall from a terminal.
//!
//! Output is JSON on stdout (the call's result); `--human` prints text.
//! Errors are `{"error":{"code","message"}}` on stderr. Exit status: 0 ok,
//! 1 error, 2 usage, 3 the user denied (or didn't answer) an approval.

#![recursion_limit = "256"]

mod args;
mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use serde_json::{json, Value};

use args::{AgentCmd, Cli, Command, MachineCmd, SessionCmd};
use pitwall_client::{Client, Error};
use pitwall_proto::{code, AgentCreate, ErrorBody, SessionAdd, SessionFilter};

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
            let view = c.create_agent(&AgentCreate {
                kind: a.kind.clone(),
                project: absolute(a.project.as_deref(), &cwd),
                name: a.name.clone(),
                resume: a.resume.clone(),
                cols: None,
                rows: None,
            })?;
            out(&view, render::agent_new(&view))
        }
        Command::Machine(MachineCmd::List) => {
            let list = c.machines()?;
            out(&list, render::machines(&list))
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
