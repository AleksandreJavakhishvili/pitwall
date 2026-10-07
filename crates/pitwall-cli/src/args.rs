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
    /// Start a new agent (or terminal) on this Mac, or create a session on
    /// another machine (--machine, an agw VM), and add it to Pitwall.
    New(Box<AgentNew>),
}

#[derive(Debug, Args, PartialEq, Default)]
pub struct AgentNew {
    /// Agent kind: claude, codex, shell, … (Pitwall's New-agent list). Not
    /// used with --machine: the session template decides what runs there.
    #[arg(long, required_unless_present = "machine")]
    pub kind: Option<String>,
    /// Folder it works in (default: the current folder). This Mac only.
    #[arg(long, value_name = "PATH", conflicts_with = "machine")]
    pub project: Option<String>,
    /// Name shown in Pitwall (default: the kind's name). With --machine:
    /// the session's name there (required).
    #[arg(long)]
    pub name: Option<String>,
    /// Continue this conversation (session id) instead of starting fresh.
    #[arg(long, value_name = "SESSION_ID", conflicts_with = "machine")]
    pub resume: Option<String>,
    /// Create it on this machine (an agw VM; see `machine list` and
    /// `machine form <machine>`). It changes that machine, so it waits for
    /// your approval in Pitwall.
    #[arg(long, value_name = "MACHINE", requires = "name")]
    pub machine: Option<String>,
    /// The provider of --machine, when two have a machine by that name.
    #[arg(long, requires = "machine")]
    pub provider: Option<String>,
    /// --machine: an existing workspace there.
    #[arg(long, value_name = "NAME", requires = "machine", conflicts_with = "new_workspace")]
    pub workspace: Option<String>,
    /// --machine: a new workspace (named NAME; default: the session's name).
    #[arg(long, value_name = "NAME", requires = "machine", num_args = 0..=1, default_missing_value = "")]
    pub new_workspace: Option<String>,
    /// --machine: the new workspace's template.
    #[arg(long, value_name = "TEMPLATE", requires = "new_workspace")]
    pub workspace_template: Option<String>,
    /// --machine: who it runs as: `admin`, `agent:<name>` (an existing agent
    /// user) or `new-agent[:<name>]` (a new one).
    #[arg(long = "as", value_name = "WHO", requires = "machine")]
    pub run_as: Option<String>,
    /// --machine with `--as new-agent`: the new agent user's template.
    #[arg(long, value_name = "TEMPLATE", requires = "run_as")]
    pub agent_template: Option<String>,
    /// --machine: the session template (what runs in it).
    #[arg(long, value_name = "TEMPLATE", requires = "machine")]
    pub template: Option<String>,
    /// --machine: any field of the machine's form, as FIELD=VALUE
    /// (`machine form <machine>` lists them). Repeatable.
    #[arg(long = "option", value_name = "FIELD=VALUE", requires = "machine")]
    pub options: Vec<String>,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum MachineCmd {
    /// List providers and their machines.
    List,
    /// What `agent new --machine` takes there: the machine's form, with the
    /// choices it has now (read-only).
    Form {
        /// The machine (e.g. an agw VM name).
        machine: String,
        /// Its provider, when two have a machine by that name.
        #[arg(long)]
        provider: Option<String>,
    },
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
            Command::Agent(AgentCmd::New(Box::new(AgentNew {
                kind: Some("claude".into()),
                project: Some("~/p".into()),
                name: Some("api".into()),
                resume: Some("s-1".into()),
                ..Default::default()
            })))
        );
        let c = parse(&["agent", "new", "--machine", "my-vm", "--name", "api-fix", "--workspace", "work", "--as", "admin", "--template", "claude"]).unwrap();
        assert_eq!(
            c.command,
            Command::Agent(AgentCmd::New(Box::new(AgentNew {
                machine: Some("my-vm".into()),
                name: Some("api-fix".into()),
                workspace: Some("work".into()),
                run_as: Some("admin".into()),
                template: Some("claude".into()),
                ..Default::default()
            })))
        );
        let Command::Agent(AgentCmd::New(n)) = parse(&["agent", "new", "--machine", "vm", "--name", "s", "--new-workspace", "--as", "new-agent:bot"]).unwrap().command else {
            panic!()
        };
        assert_eq!((n.new_workspace.as_deref(), n.run_as.as_deref()), (Some(""), Some("new-agent:bot")));
        assert_eq!(
            parse(&["machine", "form", "my-vm"]).unwrap().command,
            Command::Machine(MachineCmd::Form { machine: "my-vm".into(), provider: None })
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
        for bad in [
            &["agent"][..],
            &["agent", "new"],
            &["session", "add", "agw", "vm"],
            &["agent", "remove", "x"],
            &["nope"],
            // Platform options need a machine; a session there needs a name; no folder there.
            &["agent", "new", "--kind", "shell", "--workspace", "w"],
            &["agent", "new", "--machine", "vm"],
            &["agent", "new", "--machine", "vm", "--name", "s", "--project", "/p"],
            &["agent", "new", "--machine", "vm", "--name", "s", "--workspace", "w", "--new-workspace"],
            &["agent", "new", "--machine", "vm", "--name", "s", "--workspace-template", "t"],
        ] {
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
