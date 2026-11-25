//! `pitwall account list`, `pitwall profile …` and `pitwall project
//! accounts|profile|pick`: accounts, profiles and what a project's agents
//! launch with (docs/spec/profiles.md), over `accounts.list`, `profile.*`
//! and `project.accounts|pick|profile`.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use pitwall_client::{Client, Error};
use pitwall_proto::{caps, method, AccountsView, ProfilePick, ProfileRef, ProfileSave, ProfileView, ProjectAccounts, ProjectPath, ProjectPick, ProjectProfile};

use crate::args::{AccountCmd, ProfileCmd, ProjectCmd};
use crate::render::table;
use crate::Output;

/// The value that drops a pick (the project or profile stops deciding).
const INHERIT: &str = "inherit";

fn call<R: DeserializeOwned>(c: &mut Client, m: &str, params: impl Serialize) -> Result<R, Error> {
    c.call_cap(caps::MANAGE, m, params)
}

fn out<T: Serialize>(v: &T, human: String) -> Output {
    Output { json: serde_json::to_value(v).unwrap_or(Value::Null), human, failed: None }
}

fn folder(p: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    crate::absolute(Some(p), Path::new(&cwd))
}

fn value(v: &str) -> Option<String> {
    (!v.trim().eq_ignore_ascii_case(INHERIT)).then(|| v.trim().to_string())
}

/// The `project` subcommands this module answers.
pub fn is_mine(cmd: &ProjectCmd) -> bool {
    matches!(cmd, ProjectCmd::Accounts { .. } | ProjectCmd::Profile { .. } | ProjectCmd::Pick { .. })
}

pub fn project_cmd(cmd: &ProjectCmd, c: &mut Client) -> Result<Output, Error> {
    let v: ProjectAccounts = match cmd {
        ProjectCmd::Accounts { path } => call(c, method::PROJECT_ACCOUNTS, ProjectPath { path: folder(path) })?,
        ProjectCmd::Profile { path, profile } => call(c, method::PROJECT_PROFILE, ProjectProfile { path: folder(path), profile: Some(profile.clone()) })?,
        ProjectCmd::Pick { path, slot, value: v } => call(c, method::PROJECT_PICK, ProjectPick { path: folder(path), slot: slot.clone(), value: value(v) })?,
        _ => unreachable!("is_mine"),
    };
    Ok(out(&v, project_accounts(&v)))
}

pub fn account_cmd(cmd: &AccountCmd, c: &mut Client) -> Result<Output, Error> {
    match cmd {
        AccountCmd::List => {
            let v: AccountsView = call(c, method::ACCOUNTS_LIST, Value::Null)?;
            Ok(out(&v, accounts(&v)))
        }
    }
}

pub fn profile_cmd(cmd: &ProfileCmd, c: &mut Client) -> Result<Output, Error> {
    let one = |p: ProfileView, note: String| out(&p, note + &profile(&p));
    Ok(match cmd {
        ProfileCmd::List => {
            let v: AccountsView = call(c, method::ACCOUNTS_LIST, Value::Null)?;
            out(&v.profiles, profiles(&v.profiles))
        }
        ProfileCmd::Create { name } => one(call(c, method::PROFILE_SAVE, ProfileSave { id: None, name: name.clone() })?, format!("Made the profile {name}\n")),
        ProfileCmd::Rename { profile: p, name } => one(call(c, method::PROFILE_SAVE, ProfileSave { id: Some(p.clone()), name: name.clone() })?, format!("Renamed {p} to {name}\n")),
        ProfileCmd::Set { profile: p, slot, value: v } => one(call(c, method::PROFILE_SET, ProfilePick { profile: p.clone(), slot: slot.clone(), value: value(v) })?, String::new()),
        ProfileCmd::Delete { profile: p } => {
            let v: AccountsView = call(c, method::PROFILE_DELETE, ProfileRef { profile: p.clone() })?;
            out(&v, format!("Deleted the profile {p}\n"))
        }
    })
}

pub fn accounts(v: &AccountsView) -> String {
    let mut h = String::new();
    if v.accounts.is_empty() && v.git.is_empty() {
        h.push_str("No accounts: every agent runs on your own logins.\n");
    } else {
        let mut rows: Vec<Vec<String>> = v.accounts.iter().map(|a| vec![a.slot_label.clone(), a.name.clone(), a.id.clone(), if a.default { "default".into() } else { String::new() }]).collect();
        rows.extend(v.git.iter().map(|g| vec!["Git identity".into(), g.name.clone(), g.id.clone(), g.email.clone()]));
        h.push_str(&table(&rows));
    }
    h.push('\n');
    h.push_str(&profiles(&v.profiles));
    h
}

pub fn profiles(list: &[ProfileView]) -> String {
    if list.is_empty() {
        return "No profiles (make one: profile create <name>).\n".into();
    }
    list.iter().map(profile).collect::<Vec<_>>().join("\n")
}

pub fn profile(p: &ProfileView) -> String {
    let mut h = format!("Profile {} ({})\n", p.name, p.id);
    if p.picks.is_empty() {
        h.push_str("  Picks nothing yet (profile set <profile> <slot> <account>).\n");
    }
    for k in &p.picks {
        let _ = writeln!(h, "  {}: {}", k.slot_label, k.name);
    }
    if !p.projects.is_empty() {
        let _ = writeln!(h, "  Used by {}", p.projects.join(", "));
    }
    h
}

pub fn project_accounts(v: &ProjectAccounts) -> String {
    let mut h = format!("{}: profile {}\n", v.project, v.profile.as_deref().unwrap_or("none"));
    let rows: Vec<Vec<String>> = v
        .slots
        .iter()
        .map(|s| {
            let from = match s.from.as_str() {
                "project" => "its own pick",
                "profile" => "from its profile",
                _ => "default",
            };
            vec![s.slot_label.clone(), s.name.clone(), from.into()]
        })
        .collect();
    if rows.is_empty() {
        h.push_str("Everything at its default (your own logins).\n");
    } else {
        h.push_str(&table(&rows));
    }
    for line in &v.still_running {
        let _ = writeln!(h, "{line}");
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::{PickView, SlotView};

    #[test]
    fn inherit_drops_the_pick() {
        assert_eq!(value("Inherit"), None);
        assert_eq!(value(" Work "), Some("Work".into()));
    }

    #[test]
    fn a_project_says_where_each_answer_came_from() {
        let v = ProjectAccounts {
            project: "/code/api".into(),
            profile: Some("Work".into()),
            slots: vec![
                SlotView { slot: "claude".into(), slot_label: "Claude Code".into(), id: Some("claude-work".into()), name: "Work".into(), from: "profile".into(), own: None },
                SlotView { slot: "ext:inbox".into(), slot_label: "Inbox".into(), id: None, name: "None".into(), from: "project".into(), own: Some("none".into()) },
            ],
            still_running: vec!["api keeps its Inbox variables until it restarts.".into()],
        };
        let h = project_accounts(&v);
        assert!(h.starts_with("/code/api: profile Work\n"), "{h}");
        assert!(h.contains("Claude Code") && h.contains("from its profile") && h.contains("its own pick"), "{h}");
        assert!(h.ends_with("api keeps its Inbox variables until it restarts.\n"), "{h}");
        let p = ProfileView {
            id: "profile-work".into(),
            name: "Work".into(),
            picks: vec![PickView { slot: "web".into(), slot_label: "Browser account".into(), value: "web-work".into(), name: "Work".into() }],
            projects: vec!["/code/api".into()],
        };
        assert_eq!(profile(&p), "Profile Work (profile-work)\n  Browser account: Work\n  Used by /code/api\n");
    }
}
