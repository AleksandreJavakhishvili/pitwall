//! Accounts, extension connections, profiles and what projects pick
//! (docs/spec/profiles.md), over the socket: `accounts.list`,
//! `profile.save|set|delete`, `project.accounts|pick|profile`.
//!
//! A **slot** is a tool kind's id (`claude`), `git`, `web` (the browser
//! account; `browser` is accepted for it) or `ext:<extension id>`. A
//! **value** is an account's (or git identity's, or connection's) name or
//! id, `default`, or `none` (extensions only).

use serde::{Deserialize, Serialize};

/// One account, browser account or extension connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountItem {
    pub id: String,
    pub slot: String,
    /// What the slot is called ("Claude Code", "Browser account", the
    /// extension's name).
    pub slot_label: String,
    pub name: String,
    /// An extension's default connection (applies where nothing picks).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
}

/// One git identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitItem {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// A profile's pick for one slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PickView {
    pub slot: String,
    pub slot_label: String,
    /// As kept: an id, `default` or `none`.
    pub value: String,
    /// In words: the account's name, "Default" or "None".
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub picks: Vec<PickView>,
    /// The project folders that use it.
    pub projects: Vec<String>,
}

/// → `accounts.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountsView {
    pub accounts: Vec<AccountItem>,
    pub git: Vec<GitItem>,
    pub profiles: Vec<ProfileView>,
}

/// `profile.save`: a new profile (no `id`) or a rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSave {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
}

/// `profile.delete`: a profile by id or name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRef {
    pub profile: String,
}

/// `profile.set`: a profile's pick for a slot (`value` absent: it no
/// longer decides that slot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePick {
    pub profile: String,
    pub slot: String,
    #[serde(default)]
    pub value: Option<String>,
}

/// `project.pick`: a project's own pick for a slot (`value` absent: it
/// inherits, from its profile or the Default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPick {
    pub path: String,
    pub slot: String,
    #[serde(default)]
    pub value: Option<String>,
}

/// `project.profile`: the profile a project uses (absent: none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProfile {
    pub path: String,
    #[serde(default)]
    pub profile: Option<String>,
}

/// A project's answer for one slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SlotView {
    pub slot: String,
    pub slot_label: String,
    /// The account (git identity, connection); absent: the user's own, or
    /// no connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// In words ("Work", "Your own", "None").
    pub name: String,
    /// Where it came from: `project`, `profile` or `default`.
    pub from: String,
    /// The project's own pick (an override), as kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<String>,
}

/// → `project.accounts|pick|profile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAccounts {
    pub project: String,
    /// Its profile's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub slots: Vec<SlotView>,
    /// After a change: the project's running agents that still use what
    /// they had (nothing restarts by itself), in words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub still_running: Vec<String>,
}
