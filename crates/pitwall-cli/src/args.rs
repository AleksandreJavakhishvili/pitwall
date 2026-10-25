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
        /// The agent (id or name).
        agent: String,
        /// Its place in `queue list` (1 = first).
        n: usize,
    },
    /// Send the Nth queued prompt (default: the first) to the agent now.
    Send {
        /// The agent (id or name).
        agent: String,
        /// Its place in `queue list` (default: 1, the first).
        n: Option<usize>,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum SpaceCmd {
    /// Every space: its window, the agents shown in it and its members.
    List,
    /// A new space (in the main window).
    Create {
        /// The space's name.
        name: String,
    },
    /// Rename a space (id or name).
    Rename {
        /// The space (id or name).
        space: String,
        /// The new name.
        name: String,
    },
    /// Move a space into another window: `new`, `main` or a window's label.
    Move {
        /// The space (id or name).
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
        /// The folder.
        path: String,
    },
    /// Take a folder off the list (never deletes anything).
    Remove {
        /// The folder.
        path: String,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum RulesCmd {
    /// The library's skills and rules (with their type), and each agent's
    /// rules state.
    List,
    /// Write a new rule into the library: from scratch, a file, or a
    /// library entry. A rule is in every session's context.
    New(NewRule),
    /// Skills: know-how an agent loads only when it's relevant.
    #[command(subcommand)]
    Skill(SkillCmd),
    /// Rule sets, and each project's default set.
    Sets,
    /// What a project's agents get: its default set, each agent's own set,
    /// and the rules those add up to for the agent's kind (or one agent's,
    /// with --agent).
    Show {
        /// The project folder (default: the current folder).
        #[arg(conflicts_with = "agent")]
        project: Option<String>,
        /// One agent (id or name) instead of a whole project.
        #[arg(long, value_name = "AGENT")]
        agent: Option<String>,
    },
    /// Generate an agent's rule files now (used from its next session), or
    /// every agent's whose rules changed (--stale).
    Apply {
        /// The agent (id or name).
        #[arg(required_unless_present = "stale", conflicts_with = "stale")]
        agent: Option<String>,
        /// Give the agent this set of its own first (id or name; `none`
        /// clears). With --stale: only agents that use this set.
        #[arg(long, value_name = "SET")]
        set: Option<String>,
        /// The agent works in the main checkout: write there anyway (asks
        /// you in Pitwall).
        #[arg(long, conflicts_with = "stale")]
        main_checkout: bool,
        /// Every agent whose rules changed or failed since its session
        /// started (in its own worktree, or a main checkout you confirmed).
        #[arg(long)]
        stale: bool,
        /// With --stale: only this project's agents.
        #[arg(long, value_name = "PATH", conflicts_with = "agent")]
        project: Option<String>,
        /// Then restart each re-applied agent that is running, so the rules
        /// take effect now (each restart asks you in Pitwall).
        #[arg(long)]
        restart: bool,
    },
    /// A project's default rule set (`none` clears it).
    Default {
        /// The project folder.
        project: String,
        /// A set id or name, or `none`.
        set: String,
    },
    /// Make and change rule sets: create, add, remove, rename, delete.
    #[command(subcommand)]
    Set(RuleSetCmd),
    /// A project's own rules: library rules copied into its
    /// `.rulesync/rules/` and the files rulesync generates from them, all
    /// committed with the project. Writing there asks you in Pitwall.
    #[command(subcommand)]
    Project(ProjectRulesCmd),
    /// Add rules and skills to the library: a project's own `.rulesync/`
    /// (read in place) and its `.claude/skills` (copied), a CLAUDE.md /
    /// AGENTS.md / GEMINI.md (needs rulesync), or a git repository (cloned).
    /// A file or a repository asks you in Pitwall.
    Import {
        /// Where from: project, file or git.
        #[arg(value_parser = ["project", "file", "git"])]
        kind: String,
        /// The project folder, the file, or the repository's URL.
        source: String,
    },
}

#[derive(Debug, Args, PartialEq, Default)]
pub struct NewRule {
    /// The rule's name (its file name in the library, without .md).
    pub name: String,
    /// Start from a library entry (id), or a file.
    #[arg(long, value_name = "ID|PATH")]
    pub from: Option<String>,
    /// What it's about (shown in lists).
    #[arg(long)]
    pub description: Option<String>,
    /// rulesync targets it's for, comma-separated (default: every tool).
    #[arg(long, value_name = "TARGETS", value_delimiter = ',')]
    pub targets: Vec<String>,
    /// --from is a skill: make a rule of it (its other files are left out,
    /// and it will be in every session's context).
    #[arg(long)]
    pub as_rule: bool,
    /// Replace a rule of that name (asks you in Pitwall).
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum SkillCmd {
    /// Write a new skill into the library (a folder with SKILL.md): from
    /// scratch, a skill's folder (its other files come along), a file, or a
    /// library entry.
    New(NewSkill),
}

#[derive(Debug, Args, PartialEq, Default)]
pub struct NewSkill {
    /// The skill's name (its folder in the library).
    pub name: String,
    /// Start from a library entry (id), a skill's folder, or a file.
    #[arg(long, value_name = "ID|PATH")]
    pub from: Option<String>,
    /// When to use it: agents read this to decide (required unless --from
    /// has one).
    #[arg(long)]
    pub description: Option<String>,
    /// rulesync targets it's for, comma-separated (default: every tool).
    #[arg(long, value_name = "TARGETS", value_delimiter = ',')]
    pub targets: Vec<String>,
    /// --from is a rule: make a skill of it.
    #[arg(long)]
    pub as_skill: bool,
    /// Replace a skill of that name (asks you in Pitwall).
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum ProjectRulesCmd {
    /// The project's rules (copied from the library or its own), updates
    /// waiting, whether its generated files are out of date, and what to
    /// do first.
    Status {
        /// The project folder (default: the current folder).
        project: Option<String>,
    },
    /// Copy a rule set and/or single library rules into the project
    /// (asks). A different file already there is left alone unless named
    /// with --overwrite.
    Assign {
        /// The project folder.
        project: String,
        /// A rule set (id or name): all its rules.
        #[arg(long, value_name = "SET")]
        set: Option<String>,
        /// A library rule (id, or file name when only one rule has it).
        /// Repeatable.
        #[arg(long = "rule", value_name = "RULE", required_unless_present_any = ["set", "skills"])]
        rules: Vec<String>,
        /// A library skill (its name or id). Repeatable.
        #[arg(long = "skill", value_name = "SKILL")]
        skills: Vec<String>,
        /// Replace this file in .rulesync/rules/ although it differs.
        /// Repeatable.
        #[arg(long, value_name = "NAME")]
        overwrite: Vec<String>,
    },
    /// Remove copied rules from the project (asks). Rules changed there
    /// are kept unless --force.
    Unassign {
        /// The project folder.
        project: String,
        /// Rules: names under .rulesync/rules/ or library ids.
        #[arg(required = true, value_name = "RULE")]
        rules: Vec<String>,
        /// Remove them even if they were changed in the project.
        #[arg(long)]
        force: bool,
    },
    /// Rules whose library version changed since they were copied, or that
    /// were changed in the project, with a diff.
    Updates {
        /// The project folder (default: the current folder).
        project: Option<String>,
    },
    /// Take library updates into the project (asks): the named rules, or
    /// every rule with an update. Rules changed in both places need --take.
    Update {
        /// The project folder.
        project: String,
        /// Rules (names or library ids); none: every rule with an update.
        #[arg(value_name = "RULE")]
        rules: Vec<String>,
        /// For rules changed in both places: `library` (replace the
        /// project's copy) or `project` (keep it).
        #[arg(long, value_parser = ["library", "project"])]
        take: Option<String>,
    },
    /// Import the project's hand-written CLAUDE.md / AGENTS.md / GEMINI.md
    /// into .rulesync/rules/ with rulesync (asks), so generating keeps
    /// their text. Do this before the first generate.
    ImportExisting {
        /// The project folder (default: the current folder).
        project: Option<String>,
    },
    /// Generate the project's instruction files (CLAUDE.md, AGENTS.md, …)
    /// from its .rulesync/ with rulesync (asks). Only files Pitwall
    /// generated before are replaced, unless named with --overwrite.
    Generate {
        /// The project folder.
        project: String,
        /// rulesync targets, comma-separated (default: the ones used last,
        /// else those of the project's agents).
        #[arg(long, value_name = "TARGETS", value_delimiter = ',')]
        targets: Vec<String>,
        /// Replace this file although Pitwall didn't write it. Repeatable.
        #[arg(long, value_name = "PATH")]
        overwrite: Vec<String>,
    },
    /// Copy the project's default rule set (kept in Pitwall) into the
    /// project, and drop the default (asks).
    MoveDefault {
        /// The project folder (default: the current folder).
        project: Option<String>,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum RuleSetCmd {
    /// A new rule set (rules by id from `rules list`, or by file name when
    /// only one rule has it).
    Create {
        /// The set's name.
        name: String,
        /// A rule to put in it. Repeatable.
        #[arg(long = "rule", value_name = "RULE")]
        rules: Vec<String>,
    },
    /// Add rules to a set.
    Add {
        /// The set (id or name).
        set: String,
        /// Rules to add.
        #[arg(required = true, value_name = "RULE")]
        rules: Vec<String>,
    },
    /// Take rules out of a set.
    Remove {
        /// The set (id or name).
        set: String,
        /// Rules to take out.
        #[arg(required = true, value_name = "RULE")]
        rules: Vec<String>,
    },
    /// Rename a set.
    Rename {
        /// The set (id or name).
        set: String,
        /// The new name.
        name: String,
    },
    /// Delete a set (asks you in Pitwall). Projects and agents that use it
    /// lose it; agents keep their files until rules are re-applied.
    Delete {
        /// The set (id or name).
        set: String,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum ReviewCmd {
    /// Files an agent changed since it started (or in one task), with
    /// lines added and removed.
    Changes {
        /// The agent (id or name).
        agent: String,
        /// One task's changes (ids in the Review screen).
        #[arg(long, value_name = "TASK_ID")]
        task: Option<String>,
        /// Compare with this branch (`develop`, `origin/main`) instead of
        /// the one Review compares with.
        #[arg(long, value_name = "BRANCH", conflicts_with = "task")]
        base: Option<String>,
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
        /// The agent (id or name).
        agent: String,
    },
    /// Remove an agent from Pitwall (asks you in Pitwall).
    Remove {
        /// The agent (id or name).
        agent: String,
        /// Also delete its own worktree.
        #[arg(long)]
        worktree: bool,
    },
    /// Rename an agent. A name you give is kept; agents otherwise take
    /// their conversation's title (Claude Code, Codex).
    Rename {
        /// The agent (id or name).
        agent: String,
        /// The new name.
        #[arg(required_unless_present = "auto", conflicts_with = "auto")]
        name: Option<String>,
        /// Follow its conversation's title again instead of a name.
        #[arg(long)]
        auto: bool,
    },
    /// Why an agent has its status (hooks, screen rules, activity).
    Status {
        /// The agent (id or name).
        agent: String,
    },
    /// Show an agent in a space (as dragging it onto the space's tab).
    Move {
        /// The agent (id or name).
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
    /// The login it runs with: an account's name or id (Settings → Agents
    /// → Accounts), or `default` for your own (default: the project's).
    #[arg(long, value_name = "ACCOUNT", conflicts_with = "machine")]
    pub account: Option<String>,
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
        assert_eq!(parse(&["agent", "rename", "a1", "api"]).unwrap().command, Command::Agent(AgentCmd::Rename { agent: "a1".into(), name: Some("api".into()), auto: false }));
        assert_eq!(parse(&["agent", "rename", "a1", "--auto"]).unwrap().command, Command::Agent(AgentCmd::Rename { agent: "a1".into(), name: None, auto: true }));
        assert!(parse(&["agent", "rename", "a1"]).is_err(), "a name or --auto");
        assert!(parse(&["agent", "rename", "a1", "api", "--auto"]).is_err(), "not both");
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
            Command::Rules(RulesCmd::Apply { agent: Some("api".into()), set: Some("strict".into()), main_checkout: true, stale: false, project: None, restart: false })
        );
        assert_eq!(
            parse(&["rules", "apply", "--stale", "--project", "~/p", "--restart"]).unwrap().command,
            Command::Rules(RulesCmd::Apply { agent: None, set: None, main_checkout: false, stale: true, project: Some("~/p".into()), restart: true })
        );
        assert_eq!(parse(&["rules", "show", "~/p"]).unwrap().command, Command::Rules(RulesCmd::Show { project: Some("~/p".into()), agent: None }));
        assert_eq!(
            parse(&["rules", "set", "create", "Web", "--rule", "style.md", "--rule", "library:t.md"]).unwrap().command,
            Command::Rules(RulesCmd::Set(RuleSetCmd::Create { name: "Web".into(), rules: vec!["style.md".into(), "library:t.md".into()] }))
        );
        assert_eq!(
            parse(&["rules", "set", "add", "Web", "a.md", "b.md"]).unwrap().command,
            Command::Rules(RulesCmd::Set(RuleSetCmd::Add { set: "Web".into(), rules: vec!["a.md".into(), "b.md".into()] }))
        );
        assert_eq!(
            parse(&["rules", "set", "rename", "Web", "Strict"]).unwrap().command,
            Command::Rules(RulesCmd::Set(RuleSetCmd::Rename { set: "Web".into(), name: "Strict".into() }))
        );
        assert_eq!(parse(&["rules", "set", "delete", "Web"]).unwrap().command, Command::Rules(RulesCmd::Set(RuleSetCmd::Delete { set: "Web".into() })));
        assert_eq!(
            parse(&["rules", "import", "git", "https://x/r.git"]).unwrap().command,
            Command::Rules(RulesCmd::Import { kind: "git".into(), source: "https://x/r.git".into() })
        );
        assert_eq!(
            parse(&["rules", "project", "assign", "~/p", "--set", "Web", "--rule", "a.md", "--overwrite", "a.md"]).unwrap().command,
            Command::Rules(RulesCmd::Project(ProjectRulesCmd::Assign {
                project: "~/p".into(),
                set: Some("Web".into()),
                rules: vec!["a.md".into()],
                skills: vec![],
                overwrite: vec!["a.md".into()]
            }))
        );
        assert_eq!(
            parse(&["rules", "project", "update", "~/p", "a.md", "--take", "project"]).unwrap().command,
            Command::Rules(RulesCmd::Project(ProjectRulesCmd::Update { project: "~/p".into(), rules: vec!["a.md".into()], take: Some("project".into()) }))
        );
        assert_eq!(
            parse(&["rules", "project", "generate", "~/p", "--targets", "claudecode,codexcli"]).unwrap().command,
            Command::Rules(RulesCmd::Project(ProjectRulesCmd::Generate { project: "~/p".into(), targets: vec!["claudecode".into(), "codexcli".into()], overwrite: vec![] }))
        );
        assert_eq!(
            parse(&["rules", "project", "assign", "~/p", "--skill", "review"]).unwrap().command,
            Command::Rules(RulesCmd::Project(ProjectRulesCmd::Assign { project: "~/p".into(), set: None, rules: vec![], skills: vec!["review".into()], overwrite: vec![] }))
        );
        assert_eq!(
            parse(&["rules", "skill", "new", "review", "--from", "./r", "--description", "Review a diff", "--targets", "claudecode,codexcli"]).unwrap().command,
            Command::Rules(RulesCmd::Skill(SkillCmd::New(NewSkill {
                name: "review".into(),
                from: Some("./r".into()),
                description: Some("Review a diff".into()),
                targets: vec!["claudecode".into(), "codexcli".into()],
                ..Default::default()
            })))
        );
        assert_eq!(
            parse(&["rules", "new", "tabs", "--from", "library:skills/x", "--as-rule", "--overwrite"]).unwrap().command,
            Command::Rules(RulesCmd::New(NewRule { name: "tabs".into(), from: Some("library:skills/x".into()), as_rule: true, overwrite: true, ..Default::default() }))
        );
        assert_eq!(parse(&["rules", "project", "status"]).unwrap().command, Command::Rules(RulesCmd::Project(ProjectRulesCmd::Status { project: None })));
        assert_eq!(
            parse(&["rules", "project", "import-existing", "."]).unwrap().command,
            Command::Rules(RulesCmd::Project(ProjectRulesCmd::ImportExisting { project: Some(".".into()) }))
        );
        assert_eq!(parse(&["rules", "show", "--agent", "api"]).unwrap().command, Command::Rules(RulesCmd::Show { project: None, agent: Some("api".into()) }));
        assert_eq!(parse(&["review", "changes", "api", "--json"]).unwrap().command, Command::Review(ReviewCmd::Changes { agent: "api".into(), task: None, base: None }));
        assert_eq!(
            parse(&["review", "changes", "api", "--base", "develop"]).unwrap().command,
            Command::Review(ReviewCmd::Changes { agent: "api".into(), task: None, base: Some("develop".into()) })
        );
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
            &["rules", "show", "/p", "--agent", "api"],
            &["rules", "set", "add", "Web"],
            &["rules", "apply"],
            &["rules", "project", "assign", "/p"],
            &["rules", "skill", "new"],
            &["rules", "new"],
            &["rules", "project", "unassign", "/p"],
            &["rules", "project", "update", "/p", "--take", "both"],
            &["rules", "apply", "api", "--stale"],
            &["rules", "apply", "api", "--project", "/p"],
            &["rules", "apply", "--project", "/p"],
            &["rules", "import", "zip", "x"],
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
