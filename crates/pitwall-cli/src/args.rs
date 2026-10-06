//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Add agents and sessions to Pitwall from a terminal. Prints JSON (use
/// --human for text). Requests that change another machine wait for your
/// approval in Pitwall.
#[derive(Debug, Parser, PartialEq)]
#[command(name = "pitwall", bin_name = "pitwall", version)]
pub struct Cli {
    /// Pitwall's socket (default: $PITWALL_CLI_SOCKET, else ~/.pitwall/run/pitwalld.sock).
    #[arg(long, global = true, value_name = "PATH")]
    pub socket: Option<PathBuf>,
    /// Readable text instead of JSON.
    #[arg(long, short = 'H', global = true)]
    pub human: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum Command {
    /// Pitwall's agents and terminals.
    #[command(subcommand)]
    Agent(AgentCmd),
    /// Where agents can run (this Mac, agw VMs, …).
    #[command(subcommand)]
    Machine(MachineCmd),
    /// Sessions on other machines (agw) that can be added to Pitwall.
    #[command(subcommand)]
    Session(SessionCmd),
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum AgentCmd {
    /// List Pitwall's agents.
    List,
    /// Start a new agent (or terminal) on this Mac and add it to Pitwall.
    New(AgentNew),
}

#[derive(Debug, Args, PartialEq)]
pub struct AgentNew {
    /// Agent kind: claude, codex, shell, … (Pitwall's New-agent list).
    #[arg(long)]
    pub kind: String,
    /// Folder it works in (default: the current folder).
    #[arg(long, value_name = "PATH")]
    pub project: Option<String>,
    /// Name shown in Pitwall (default: the kind's name).
    #[arg(long)]
    pub name: Option<String>,
    /// Continue this conversation (session id) instead of starting fresh.
    #[arg(long, value_name = "SESSION_ID")]
    pub resume: Option<String>,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum MachineCmd {
    /// List providers and their machines.
    List,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum SessionCmd {
    /// List sessions that can be added, and whether they already are.
    List {
        /// Only this provider (e.g. agw).
        #[arg(long)]
        provider: Option<String>,
        /// Only this machine (e.g. an agw VM name).
        #[arg(long)]
        machine: Option<String>,
    },
    /// Add an existing session to Pitwall (e.g. `session add agw my-vm work`).
    /// Adding only tracks it; nothing changes on that machine.
    Add {
        /// Where it runs (e.g. agw; see `machine list`).
        provider: String,
        /// The machine there (e.g. an agw VM name).
        machine: String,
        /// The session's name there.
        name: String,
        /// Also start it if it's stopped. This changes that machine, so it
        /// waits for your approval in Pitwall.
        #[arg(long)]
        start: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("pitwall").chain(args.iter().copied()))
    }

    #[test]
    fn parses_every_command() {
        assert_eq!(parse(&["agent", "list"]).unwrap().command, Command::Agent(AgentCmd::List));
        let c = parse(&["agent", "new", "--kind", "claude", "--project", "~/p", "--name", "api", "--resume", "s-1"]).unwrap();
        assert_eq!(
            c.command,
            Command::Agent(AgentCmd::New(AgentNew {
                kind: "claude".into(),
                project: Some("~/p".into()),
                name: Some("api".into()),
                resume: Some("s-1".into())
            }))
        );
        assert_eq!(parse(&["machine", "list"]).unwrap().command, Command::Machine(MachineCmd::List));
        assert_eq!(
            parse(&["session", "list", "--provider", "agw"]).unwrap().command,
            Command::Session(SessionCmd::List { provider: Some("agw".into()), machine: None })
        );
        let c = parse(&["--human", "session", "add", "agw", "vm-1", "work", "--start", "--socket", "/tmp/s"]).unwrap();
        assert!(c.human);
        assert_eq!(c.socket, Some(PathBuf::from("/tmp/s")));
        assert_eq!(
            c.command,
            Command::Session(SessionCmd::Add { provider: "agw".into(), machine: "vm-1".into(), name: "work".into(), start: true })
        );
        assert!(!parse(&["session", "add", "agw", "vm-1", "work"]).unwrap().human);
    }

    #[test]
    fn rejects_what_isnt_a_command() {
        for bad in [&["agent"][..], &["agent", "new"], &["session", "add", "agw", "vm"], &["agent", "remove", "x"], &["nope"]] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse(&["agent", "new"]).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn clap_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
