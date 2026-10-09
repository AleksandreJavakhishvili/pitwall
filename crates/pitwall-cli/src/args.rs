//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Set up and manage Pitwall from a terminal: agents, sessions, queues,
/// spaces, projects, rules, review and settings. Prints JSON (use --human
/// for text). Risky requests (stopping or removing agents, changing another
/// machine, settings outside Pitwall's own look) wait for your approval in
/// Pitwall. Exit status: 0 ok, 1 error, 2 bad arguments, 3 not approved.
#[derive(Debug, Parser, PartialEq)]
#[command(name = "pitwall", bin_name = "pitwall", version)]
pub struct Cli {
    /// Pitwall's socket (default: $PITWALL_CLI_SOCKET, else ~/.pitwall/run/pitwalld.sock).
    #[arg(long, global = true, value_name = "PATH")]
    pub socket: Option<PathBuf>,
    /// Readable text instead of JSON.
    #[arg(long, short = 'H', global = true, conflicts_with = "json")]
    pub human: bool,
    /// JSON output (the default; for scripts and agents that want to say so).
    #[arg(long, global = true)]
    pub json: bool,
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
    /// Pitwall's settings (theme, look, density, hooks, …): list, get, set,
    /// reset. Applied live when Pitwall runs; otherwise `ui.json` is edited
    /// (safe settings only).
    #[command(subcommand)]
    Settings(SettingsCmd),
    /// Prompts queued for agents (sent when they are free).
    #[command(subcommand)]
    Queue(QueueCmd),
    /// Spaces (tabs of agents) and the windows they are in.
    #[command(subcommand)]
    Space(SpaceCmd),
    /// Pitwall's project list (folders in the sidebar).
    #[command(subcommand)]
    Project(ProjectCmd),
    /// Rule sets (rulesync) for agents and projects.
    #[command(subcommand)]
    Rules(RulesCmd),
    /// What agents changed (read-only).
    #[command(subcommand)]
    Review(ReviewCmd),
    /// Wait until an agent is idle, blocked or done (for coordinating agents).
    Wait(WaitArgs),
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum QueueCmd {
    /// Queue a prompt for an agent (`queue add api "run the tests"`), or for
    /// every agent with a status (`queue add --status idle "run the tests"`).
    /// `-` reads the prompt from stdin.
    Add {
        /// [AGENT] TEXT: the agent (id or name; not with --status) and the prompt.
        #[arg(required = true, num_args = 1..=2, value_name = "AGENT TEXT")]
        args: Vec<String>,
        /// Queue it for every agent that has this status now (idle, done, …).
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
    },
    /// What is queued: one agent's, or every agent with a queue.
    List {
        /// An agent (id or name).
        agent: Option<String>,
    },
    /// Take the Nth prompt (1 = first) out of an agent's queue.
    Remove {
        agent: String,
        /// Its place in `queue list` (1 = first).
        n: usize,
    },
    /// Send the Nth queued prompt (default: the first) to the agent now.
    Send {
        agent: String,
        n: Option<usize>,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum SpaceCmd {
    /// Every space: its window, the agents shown in it and its members.
    List,
    /// A new space (in the main window).
    Create {
        name: String,
    },
    /// Rename a space (id or name).
    Rename {
        space: String,
        name: String,
    },
    /// Move a space into another window: `new`, `main` or a window's label.
    Move {
        space: String,
        #[arg(long = "to-window", value_name = "new|main|LABEL")]
        to_window: String,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum ProjectCmd {
    /// Pitwall's projects.
    List,
    /// Add a folder to Pitwall's projects.
    Add {
        path: String,
    },
    /// Take a folder off the list (never deletes anything).
    Remove {
        path: String,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum RulesCmd {
    /// Rule files in the library, and each agent's rules state.
    List,
    /// Rule sets, and each project's default set.
    Sets,
    /// Generate an agent's rule files now (used from its next session).
    Apply {
        agent: String,
        /// Give the agent this set of its own first (id or name; `none` clears).
        #[arg(long, value_name = "SET")]
        set: Option<String>,
        /// The agent works in the main checkout: write there anyway (asks
        /// you in Pitwall).
        #[arg(long)]
        main_checkout: bool,
    },
    /// A project's default rule set (`none` clears it).
    Default {
        /// The project folder.
        project: String,
        /// A set id or name, or `none`.
        set: String,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum ReviewCmd {
    /// Files an agent changed since it started (or in one task), with
    /// lines added and removed.
    Changes {
        agent: String,
        /// One task's changes (ids in the Review screen).
        #[arg(long, value_name = "TASK_ID")]
        task: Option<String>,
    },
}

#[derive(Debug, Args, PartialEq)]
pub struct WaitArgs {
    /// The agent (id or name).
    pub agent: String,
    /// Any of these statuses ends the wait: idle, blocked, done, working,
    /// stopped, exited (comma-separated).
    #[arg(long = "for", value_name = "STATUS", value_delimiter = ',', required = true)]
    pub until: Vec<String>,
    /// Give up after this many seconds (default 300): exit 1, code `timeout`.
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u64>,
    /// Ignore the status it has now: wait for it to change first (use after
    /// sending a prompt).
    #[arg(long)]
    pub fresh: bool,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum SettingsCmd {
    /// Every setting: key, current value, allowed values, default, whether
    /// it needs approval, and what it does.
    List,
    /// One setting (e.g. `settings get appearance.theme`).
    Get {
        /// A key from `settings list` (e.g. appearance.theme).
        key: String,
    },
    /// Change a setting (e.g. `settings set appearance.theme dark`).
    /// Settings marked `approval` wait for your OK in Pitwall.
    Set {
        /// A key from `settings list`.
        key: String,
        /// The new value: true/false for switches, one of the listed
        /// choices, or a number.
        value: String,
    },
    /// Put a setting back to its default.
    Reset {
        /// A key from `settings list`.
        key: String,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum AgentCmd {
    /// List Pitwall's agents.
    List,
    /// Start a new agent (or terminal) on this Mac, or create a session on
    /// another machine (--machine, an agw VM), and add it to Pitwall.
    New(Box<AgentNew>),
    /// Stop an agent (asks you in Pitwall).
    Stop {
        /// The agent (id or name).
        agent: String,
    },
    /// Start an agent again (asks you in Pitwall).
    Restart {
        agent: String,
    },
    /// Remove an agent from Pitwall (asks you in Pitwall).
    Remove {
        agent: String,
        /// Also delete its own worktree.
        #[arg(long)]
        worktree: bool,
    },
    /// Rename an agent.
    Rename {
        agent: String,
        name: String,
    },
    /// Why an agent has its status (hooks, screen rules, activity).
    Status {
        agent: String,
    },
    /// Show an agent in a space (as dragging it onto the space's tab).
    Move {
        agent: String,
        /// The space (id or name).
        #[arg(long, value_name = "SPACE")]
        space: String,
    },
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
        assert_eq!(parse(&["settings", "list", "--json"]).unwrap().command, Command::Settings(SettingsCmd::List));
        assert!(parse(&["settings", "list", "--json"]).unwrap().json);
        assert_eq!(
            parse(&["settings", "set", "appearance.theme", "dark"]).unwrap().command,
            Command::Settings(SettingsCmd::Set { key: "appearance.theme".into(), value: "dark".into() })
        );
        assert_eq!(parse(&["settings", "reset", "appearance.look"]).unwrap().command, Command::Settings(SettingsCmd::Reset { key: "appearance.look".into() }));
        assert_eq!(parse(&["settings", "get", "x"]).unwrap().command, Command::Settings(SettingsCmd::Get { key: "x".into() }));
        assert_eq!(parse(&["agent", "remove", "a1", "--worktree"]).unwrap().command, Command::Agent(AgentCmd::Remove { agent: "a1".into(), worktree: true }));
        assert_eq!(parse(&["agent", "stop", "api"]).unwrap().command, Command::Agent(AgentCmd::Stop { agent: "api".into() }));
        assert_eq!(parse(&["agent", "rename", "a1", "api"]).unwrap().command, Command::Agent(AgentCmd::Rename { agent: "a1".into(), name: "api".into() }));
        assert_eq!(
            parse(&["agent", "move", "a1", "--space", "Work"]).unwrap().command,
            Command::Agent(AgentCmd::Move { agent: "a1".into(), space: "Work".into() })
        );
        assert_eq!(
            parse(&["queue", "add", "api", "run tests"]).unwrap().command,
            Command::Queue(QueueCmd::Add { args: vec!["api".into(), "run tests".into()], status: None })
        );
        assert_eq!(
            parse(&["queue", "add", "--status", "idle", "run tests"]).unwrap().command,
            Command::Queue(QueueCmd::Add { args: vec!["run tests".into()], status: Some("idle".into()) })
        );
        assert_eq!(parse(&["queue", "send", "api"]).unwrap().command, Command::Queue(QueueCmd::Send { agent: "api".into(), n: None }));
        assert_eq!(parse(&["queue", "remove", "api", "2"]).unwrap().command, Command::Queue(QueueCmd::Remove { agent: "api".into(), n: 2 }));
        assert_eq!(
            parse(&["space", "move", "Work", "--to-window", "new"]).unwrap().command,
            Command::Space(SpaceCmd::Move { space: "Work".into(), to_window: "new".into() })
        );
        assert_eq!(parse(&["project", "add", "."]).unwrap().command, Command::Project(ProjectCmd::Add { path: ".".into() }));
        assert_eq!(
            parse(&["rules", "apply", "api", "--set", "strict", "--main-checkout"]).unwrap().command,
            Command::Rules(RulesCmd::Apply { agent: "api".into(), set: Some("strict".into()), main_checkout: true })
        );
        assert_eq!(parse(&["review", "changes", "api", "--json"]).unwrap().command, Command::Review(ReviewCmd::Changes { agent: "api".into(), task: None }));
        assert_eq!(
            parse(&["wait", "api", "--for", "idle,done", "--timeout", "60", "--fresh"]).unwrap().command,
            Command::Wait(WaitArgs { agent: "api".into(), until: vec!["idle".into(), "done".into()], timeout: Some(60), fresh: true })
        );
    }

    #[test]
    fn rejects_what_isnt_a_command() {
        for bad in [
            &["agent"][..],
            &["agent", "new"],
            &["session", "add", "agw", "vm"],
            &["agent", "remove"],
            &["agent", "move", "a"],
            &["queue", "add"],
            &["queue", "add", "a", "b", "c"],
            &["space", "move", "s"],
            &["wait", "a"],
            &["queue", "remove", "a", "x"],
            &["nope"],
            // Platform options need a machine; a session there needs a name; no folder there.
            &["agent", "new", "--kind", "shell", "--workspace", "w"],
            &["agent", "new", "--machine", "vm"],
            &["agent", "new", "--machine", "vm", "--name", "s", "--project", "/p"],
            &["agent", "new", "--machine", "vm", "--name", "s", "--workspace", "w", "--new-workspace"],
            &["agent", "new", "--machine", "vm", "--name", "s", "--workspace-template", "t"],
            &["settings", "set", "appearance.theme"],
            &["settings", "get"],
            &["settings", "list", "--json", "--human"],
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
