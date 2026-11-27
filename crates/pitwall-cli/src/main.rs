//! `pitwall`: set up and manage Pitwall from a terminal — agents, sessions,
//! queues, spaces, projects, rules, review, settings and peers — and show
//! the user files (`open`, and the `mcp` server agents get).
//!
//! Output is JSON on stdout (the call's result); `--human` prints text.
//! Errors are `{"error":{"code","message"}}` on stderr. Exit status: 0 ok,
//! 1 error, 2 bad arguments (usage, or `bad_params`), 3 the user denied (or
//! didn't answer) an approval.

#![recursion_limit = "256"]

mod args;
mod events;
mod extension;
mod manage;
mod mcp;
mod open;
mod peers;
mod profiles;
mod render;
#[cfg(test)]
mod reference;
mod settings;
mod tile;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use serde_json::{json, Value};

use args::{AgentCmd, Cli, Command, MachineCmd, SessionCmd};
use pitwall_client::{Client, Error};
use pitwall_proto::{code, AgentCreate, AgentCreated, ErrorBody, FormRequest, ProviderMachines, SessionAdd, SessionFilter, CREATE_NEW};

/// What a command produced: its JSON result and the same as text.
pub struct Output {
    pub json: Value,
    pub human: String,
    /// Done in part, then failed: printed as an error after the output,
    /// and the exit status is its.
    pub failed: Option<ErrorBody>,
}

fn out<T: serde::Serialize>(v: &T, human: String) -> Output {
    Output { json: serde_json::to_value(v).unwrap_or(Value::Null), human, failed: None }
}

/// `agent new --prompt`/`--prompt-file`: the text (`-`: stdin), trailing
/// newlines dropped; blank is refused before anything is made.
fn first_instruction(a: &args::AgentNew) -> Result<Option<String>, Error> {
    let text = match (&a.prompt, &a.prompt_file) {
        (Some(p), _) => manage::prompt_text(p)?,
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("--prompt-file {path}: {e}"))))?
            .trim_end_matches(['\n', '\r'])
            .to_string(),
        (None, None) => return Ok(None),
    };
    if text.trim().is_empty() {
        return Err(Error::Server(ErrorBody::new(code::BAD_PARAMS, "the first instruction is empty: nothing was created")));
    }
    Ok(Some(text))
}

/// Error code: the agent was made, but its first instruction wasn't queued.
pub const QUEUE_FAILED: &str = "queue_failed";

/// What `agent new` printed, and its first instruction's fate.
fn agent_new_output(made: &AgentCreated) -> Output {
    let a = &made.agent;
    let mut human = render::agent_new(a);
    let mut failed = None;
    if made.queued.is_some() {
        human.push_str(&render::first_queued(a));
    } else if let Some(e) = &made.queue_error {
        failed = Some(ErrorBody::new(
            QUEUE_FAILED,
            format!("{} was created (id {}), but its first instruction couldn't be queued: {e}. Queue it with `pitwall queue add {} -`.", a.name, a.id, a.id),
        ));
    }
    Output { failed, ..out(made, human) }
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
    if let Command::Settings(s) = cmd {
        // Works without Pitwall running too (edits ui.json).
        return settings::run(s, socket, &pitwall_client::data_dir());
    }
    if let Command::Peers(p) = cmd {
        // Works on the data folder, without the app.
        return peers::run(p, &pitwall_client::data_dir(), peers::in_an_agent());
    }
    let mut c = Client::connect_as(socket, &format!("pitwall-cli/{}", env!("CARGO_PKG_VERSION")), pitwall_proto::Role::Cli)?;
    Ok(match cmd {
        Command::Agent(AgentCmd::List) => {
            let list = c.agents()?;
            out(&list, render::agents(&list))
        }
        Command::Agent(AgentCmd::New(a)) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            let options = machine_options(a).map_err(|e| Error::Server(ErrorBody::new(code::BAD_PARAMS, e)))?;
            let prompt = first_instruction(a)?;
            let made = c.create_agent_queued(&AgentCreate {
                kind: a.kind.clone().unwrap_or_default(),
                project: if a.machine.is_some() { String::new() } else { absolute(a.project.as_deref(), &cwd) },
                name: a.name.clone(),
                resume: a.resume.clone(),
                account: a.account.clone(),
                provider: a.provider.clone(),
                machine: a.machine.clone(),
                options,
                cols: None,
                rows: None,
                prompt,
            })?;
            agent_new_output(&made)
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
        Command::Settings(_) | Command::Peers(_) => unreachable!("handled above"),
        Command::Extension(e) => extension::setup(e, &mut c)?,
        Command::Events(_) => unreachable!("streamed by main (events::follow)"),
        Command::Mcp => unreachable!("served by main (mcp::serve)"),
        Command::Open(a) if a.device.is_some() => {
            let req = open::device_request(a).expect("a device");
            let (opened, text) = open::open_device(&mut c, &req)?;
            out(&opened, text)
        }
        Command::Open(a) if a.settings.is_some() => {
            let req = open::settings_request(a).expect("Settings");
            let (opened, text) = open::open_settings(&mut c, &req)?;
            out(&opened, text)
        }
        Command::Open(a) if a.web.is_some() => {
            let req = open::web_request(a).expect("a web page");
            let (opened, text) = open::open_web(&mut c, &req)?;
            out(&opened, text)
        }
        Command::Open(a) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            let (opened, text) = open::open(&mut c, &open::request(a, &cwd))?;
            out(&opened, text)
        }
        Command::Agent(a) => manage::agent_cmd(a, &mut c)?,
        Command::Queue(q) => manage::queue_cmd(q, &mut c)?,
        Command::Space(sp) => manage::space_cmd(sp, &mut c)?,
        Command::Project(p) => manage::project_cmd(p, &mut c)?,
        Command::Account(a) => profiles::account_cmd(a, &mut c)?,
        Command::Profile(p) => profiles::profile_cmd(p, &mut c)?,
        Command::Rules(r) => manage::rules_cmd(r, &mut c)?,
        Command::Review(r) => manage::review_cmd(r, &mut c)?,
        Command::Wait(w) => manage::wait_cmd(w, &mut c)?,
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
    (json!({ "error": body }), exit_status(&body.code))
}

/// 3: not approved; 2: bad arguments; 1: anything else.
pub fn exit_status(code: &str) -> u8 {
    match code {
        code::DENIED | code::APPROVAL_TIMEOUT => 3,
        code::BAD_PARAMS | BAD_ARGS => 2,
        _ => 1,
    }
}

/// Error code of a command line that doesn't parse.
pub const BAD_ARGS: &str = "bad_args";

/// Parse the command line; a usage error is JSON on stderr (text with
/// `--human`) and exit 2, help and version print as usual.
fn parse() -> Result<Cli, ExitCode> {
    let raw: Vec<std::ffi::OsString> = std::env::args_os().collect();
    match Cli::try_parse_from(&raw) {
        Ok(cli) => Ok(cli),
        Err(e) if !e.use_stderr() => {
            let _ = e.print();
            Err(ExitCode::SUCCESS)
        }
        Err(e) => {
            if raw.iter().any(|a| a == "--human" || a == "-H") {
                let _ = e.print();
            } else {
                eprintln!("{}", usage_error(&e.render().to_string()));
            }
            Err(ExitCode::from(2))
        }
    }
}

/// Clap's message as the CLI's error JSON (first line, without "error: ").
pub fn usage_error(rendered: &str) -> Value {
    let first = rendered.lines().next().unwrap_or("bad arguments").trim();
    let message = first.strip_prefix("error: ").unwrap_or(first);
    json!({ "error": { "code": BAD_ARGS, "message": format!("{message} (see `pitwall --help`)") } })
}

fn main() -> ExitCode {
    let cli = match parse() {
        Ok(cli) => cli,
        Err(status) => return status,
    };
    let socket = cli.socket.clone().unwrap_or_else(pitwall_client::socket_path);
    if let Command::Peers(args::PeersCmd::Watch { name, count }) = &cli.command {
        return match peers::watch(name, *count, &pitwall_client::data_dir(), cli.human, &mut std::io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => report(&e, cli.human),
        };
    }
    if let Command::Mcp = &cli.command {
        let up = mcp::SocketUpstream { socket: socket.clone() };
        let proxy = mcp::Proxy::new(&up, open::env_agent());
        return match mcp::serve(&proxy, std::io::stdin().lock(), &mut std::io::stdout(), Some(mcp::WATCH_EVERY)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("pitwall mcp: {e}");
                ExitCode::from(1)
            }
        };
    }
    if let Command::Extension(args::ExtensionCmd::Dev { folder, once }) = &cli.command {
        let folder = folder.clone().unwrap_or_else(|| PathBuf::from("."));
        return match tile::dev(&folder, &socket, *once, cli.human, &mut std::io::stdout().lock()) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(e) => report(&Error::Server(ErrorBody::new("failed", e)), cli.human),
        };
    }
    if let Some(e) = cli.command.offline_extension() {
        // On a folder, offline: no socket.
        return match extension::run(e) {
            Ok((o, passed)) => {
                if cli.human {
                    print!("{}", o.human);
                } else {
                    println!("{}", serde_json::to_string_pretty(&o.json).unwrap_or_default());
                }
                if passed {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => report(&e, cli.human),
        };
    }
    if let Command::Events(a) = &cli.command {
        return match events::follow(a, &socket, cli.human, &mut std::io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => report(&e, cli.human),
        };
    }
    match run(&cli.command, &socket) {
        Ok(o) => {
            if cli.human {
                print!("{}", o.human);
            } else {
                println!("{}", serde_json::to_string_pretty(&o.json).unwrap_or_default());
            }
            match o.failed {
                // What was done is on stdout; what failed after it, as any error.
                Some(body) => report(&Error::Server(body), cli.human),
                None => ExitCode::SUCCESS,
            }
        }
        Err(e) => report(&e, cli.human),
    }
}

/// Print the error on stderr; its exit status.
fn report(e: &Error, human: bool) -> ExitCode {
    let (body, status) = failure(e);
    if human {
        eprintln!("pitwall: {e}");
    } else {
        eprintln!("{body}");
    }
    ExitCode::from(status)
}

#[cfg(test)]
mod tests;
