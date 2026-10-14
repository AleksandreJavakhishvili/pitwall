//! `docs/reference/cli.md`, the CLI reference on the website, written from
//! the command definitions in `args.rs` so it can't drift from them. A test
//! fails when the file is out of date; `PITWALL_WRITE_DOCS=1 cargo test -p
//! pitwall-cli reference` rewrites it.

use std::fmt::Write;

use clap::{Arg, Command, CommandFactory};

use crate::args::Cli;

pub const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/reference/cli.md");

const INTRO: &str = "\
# CLI reference

`pitwall` sets up and manages Pitwall from a terminal: agents, sessions, queued prompts, spaces, projects, rules, review and settings. It does what the UI does, so you can script Pitwall or let an agent drive it. This page is generated from the CLI's own definitions; `pitwall <command> --help` shows the same.

## Install

Settings → *Install command-line tool* links `pitwall` into `~/.local/bin` (or `/usr/local/bin`). Inside a Pitwall terminal it finds the app on its own.

## How it works

- **It talks to the running app** over a local socket: `$PITWALL_CLI_SOCKET` when set, else `~/.pitwall/run/pitwalld.sock` (`--socket` overrides both). Only `settings` also works while Pitwall is closed: it edits `~/.pitwall/ui.json` (safe settings only).
- **Output is JSON** on stdout, the call's result, so scripts and agents can parse it (`jq` works well). `--human` (`-H`) prints readable text instead.
- **Errors** are `{\"error\":{\"code\":\"…\",\"message\":\"…\"}}` on stderr.
- **Agents and spaces** are named by id or by name.
- **Risky requests wait for you.** Stopping, restarting or removing an agent, anything that changes another machine (starting or creating a session on an agw VM), writing rules into a main checkout, importing rules from a file or a git repository, deleting a rule set, and settings that change something outside Pitwall open an approval dialog in Pitwall when anyone but you asks; the command waits for your answer. Pitwall identifies the caller from the connecting process, so an agent can't approve its own request.

## Exit status

| Status | Meaning |
|---|---|
| `0` | Done. |
| `1` | An error (see the JSON on stderr), including `wait` timing out (code `timeout`). |
| `2` | Bad arguments: unknown command or option, a missing value, or a value the server rejects (`bad_params`). |
| `3` | Not approved: you denied the request in Pitwall, or didn't answer. |

## Recipes

Start an agent in a project and give it work:

```bash
id=$(pitwall agent new --kind claude --project ~/code/api --name api | jq -r .id)
pitwall queue add \"$id\" \"run the tests and fix what fails\"
pitwall wait \"$id\" --for done,blocked --fresh
pitwall review changes \"$id\" --human
```

Run it on another login (Settings → Agents → Accounts):

```bash
pitwall agent new --kind codex --project ~/code/api --account work
```

Send the same prompt to every idle agent, then see who needs you:

```bash
pitwall queue add --status idle \"pull main and rebase\"
pitwall agent list --human
```

Arrange a space in its own window:

```bash
pitwall space create Review
pitwall agent move api --space Review
pitwall space move Review --to-window new
```

Change a setting, or read them all:

```bash
pitwall settings set appearance.theme dark
pitwall settings list --human
```
";

/// Examples shown under a command, by its path without `pitwall`.
const EXAMPLES: &[(&str, &str)] = &[
    ("agent new", "pitwall agent new --kind claude --project ~/code/api --name api\npitwall agent new --kind claude --resume 7f3c2a90-…      # continue a conversation\npitwall agent new --kind shell                           # a terminal in this folder\npitwall agent new --machine my-vm --name fix --workspace work --as admin --template claude"),
    ("agent status", "pitwall agent status api --human"),
    ("agent move", "pitwall agent move api --space Review"),
    ("machine form", "pitwall machine form my-vm --human"),
    ("session add", "pitwall session add agw my-vm work           # track it\npitwall session add agw my-vm work --start   # and start it (asks)"),
    ("settings set", "pitwall settings set appearance.theme dark\npitwall settings set appearance.density dense"),
    ("queue add", "pitwall queue add api \"run the tests\"\necho \"summarize the diff\" | pitwall queue add api -\npitwall queue add --status idle \"pull main\""),
    ("space move", "pitwall space move Review --to-window new"),
    ("rules apply", "pitwall rules apply api --set strict"),
    ("rules show", "pitwall rules show ~/code/api --human\npitwall rules show --agent api"),
    ("rules set create", "pitwall rules set create Web --rule style.md --rule testing.md"),
    ("rules set add", "pitwall rules set add Web library:terse.md"),
    ("rules import", "pitwall rules import project ~/code/api          # its own .rulesync/\npitwall rules import file ~/code/api/CLAUDE.md   # asks first\npitwall rules import git https://github.com/acme/ai-rules.git   # asks first"),
    ("review changes", "pitwall review changes api --human"),
    ("wait", "pitwall queue add api \"run the tests\"\npitwall wait api --for done,blocked --fresh --timeout 900"),
];

/// The reference as Markdown.
pub fn markdown() -> String {
    let mut cli = Cli::command();
    cli.build();
    let mut md = String::from(INTRO);
    md.push_str("\n## Global options\n\nThese work with every command.\n\n");
    options(&mut md, &cli, true);
    md.push_str("\n## Commands\n\n");
    for group in cli.get_subcommands().filter(|c| !hidden(c)) {
        let _ = writeln!(md, "- [`{}`](#{}): {}", group.get_name(), anchor(&path(&[group.get_name()])), first_line(group));
    }
    for group in cli.get_subcommands().filter(|c| !hidden(c)) {
        let _ = write!(md, "\n## {}\n\n", group.get_name());
        if group.has_subcommands() {
            let _ = writeln!(md, "{}\n", about(group));
            for c in group.get_subcommands().filter(|c| !hidden(c)) {
                if c.has_subcommands() {
                    // A third level (`rules set create`): one heading each.
                    for sub in c.get_subcommands().filter(|c| !hidden(c)) {
                        let name = format!("{} {} {}", group.get_name(), c.get_name(), sub.get_name());
                        let _ = write!(md, "### {name}\n\n");
                        command(&mut md, &name, sub);
                    }
                    continue;
                }
                let _ = write!(md, "### {} {}\n\n", group.get_name(), c.get_name());
                command(&mut md, &format!("{} {}", group.get_name(), c.get_name()), c);
            }
        } else {
            command(&mut md, group.get_name(), group);
        }
    }
    md
}

fn hidden(c: &Command) -> bool {
    c.is_hide_set() || c.get_name() == "help"
}

fn path(parts: &[&str]) -> String {
    parts.join(" ")
}

/// The heading id the site gives `### agent new` (see website/scripts/markdown.mjs).
fn anchor(heading: &str) -> String {
    let s: String = heading.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

fn about(c: &Command) -> String {
    sentence(&c.get_long_about().or(c.get_about()).map(|s| s.to_string()).unwrap_or_default())
}

fn first_line(c: &Command) -> String {
    c.get_about().map(|s| s.to_string()).unwrap_or_default()
}

fn help(a: &Arg) -> String {
    sentence(&a.get_long_help().or(a.get_help()).map(|s| s.to_string()).unwrap_or_default())
}

/// clap drops the full stop of one-sentence help; prose wants it back.
fn sentence(s: &str) -> String {
    let s = s.trim();
    match s.chars().last() {
        Some(c) if !matches!(c, '.' | '!' | '?' | ':') => format!("{s}."),
        _ => s.to_string(),
    }
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn command(md: &mut String, name: &str, c: &Command) {
    let usage = c.clone().render_usage().to_string();
    let usage = usage.trim_start_matches("Usage: ").trim();
    let _ = write!(md, "{}\n\n```bash\n{usage}\n```\n\n", about(c));
    let positionals: Vec<&Arg> = c.get_positionals().filter(|a| !a.is_hide_set()).collect();
    if !positionals.is_empty() {
        md.push_str("| Argument | |\n|---|---|\n");
        for a in positionals {
            let value = a.get_value_names().map(|v| v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" ")).unwrap_or_else(|| a.get_id().to_string().to_uppercase());
            let optional = if a.is_required_set() { "" } else { " (optional)" };
            let _ = writeln!(md, "| `{}`{optional} | {} |", cell(&value), cell(&help(a)));
        }
        md.push('\n');
    }
    options(md, c, false);
    if let Some((_, ex)) = EXAMPLES.iter().find(|(n, _)| *n == name) {
        let _ = write!(md, "```bash\n{ex}\n```\n\n");
    }
}

/// `c`'s options as a table: its own, or (`global`) the ones every command takes.
fn options(md: &mut String, c: &Command, global: bool) {
    let list: Vec<&Arg> = c
        .get_arguments()
        .filter(|a| !a.is_positional() && !a.is_hide_set() && a.is_global_set() == global)
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        .collect();
    if list.is_empty() {
        return;
    }
    md.push_str("| Option | |\n|---|---|\n");
    for a in list {
        let mut flag = String::new();
        if let Some(s) = a.get_short() {
            let _ = write!(flag, "-{s}, ");
        }
        if let Some(l) = a.get_long() {
            let _ = write!(flag, "--{l}");
        }
        let takes = a.get_num_args().is_some_and(|n| n.takes_values());
        if takes {
            let value = a.get_value_names().and_then(|v| v.first()).map(|v| v.to_string()).unwrap_or_else(|| a.get_id().to_string().to_uppercase());
            let optional = a.get_num_args().is_some_and(|n| n.min_values() == 0);
            let _ = if optional { write!(flag, " [{value}]") } else { write!(flag, " <{value}>") };
        }
        let mut text = help(a);
        if a.is_required_set() {
            text.push_str(" Required.");
        }
        let _ = writeln!(md, "| `{}` | {} |", cell(&flag), cell(text.trim()));
    }
    md.push('\n');
}

#[cfg(test)]
mod tests {
    #[test]
    fn reference_is_current() {
        let md = super::markdown();
        if std::env::var_os("PITWALL_WRITE_DOCS").is_some() {
            std::fs::write(super::PATH, &md).unwrap();
            return;
        }
        let on_disk = std::fs::read_to_string(super::PATH).unwrap_or_default();
        assert!(
            on_disk == md,
            "docs/reference/cli.md is out of date with the CLI's arguments: run `PITWALL_WRITE_DOCS=1 cargo test -p pitwall-cli reference` and commit it"
        );
    }
}
