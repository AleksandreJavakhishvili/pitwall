//! Pitwall's socket server (architecture.md §4). It runs inside the app for
//! now (step 5); `pitwalld` (step 6) will host the same [`server::serve`].
//!
//! - [`server`]: listen on `~/.pitwall/run/pitwalld.sock` (folder 0700,
//!   socket 0600), handshake, answer requests.
//! - [`methods`]: the method table with each method's access/risk, and what
//!   each method does in `pitwall-core`.
//! - [`approvals`]: risky requests wait for the user's answer in Pitwall.
//! - [`identity`]: who is calling (peer pid → the agent it runs in).
//! - [`settings`]: `settings.*` over the host's [`SettingsBackend`].
//! - [`manage`]: stop/restart/remove/rename/status/wait, queues, projects,
//!   rules, review.
//! - [`workspace`]: `space.*` over the host's [`WorkspaceBackend`].
//! - [`collab`]: collaboration's owner-only methods (`share.*`, `team.*`,
//!   `people.list`, `ask.*`, `collab.settings.*`) over a [`CollabBackend`].

// The tests build large JSON fixtures (`serde_json::json!`).
#![recursion_limit = "256"]

pub mod approvals;
pub mod collab;
pub mod identity;
pub mod manage;
pub mod methods;
mod platform;
pub mod server;
pub mod settings;
pub mod workspace;

pub use approvals::{Approvals, Decision};

/// This machine's name, for collaboration's `/info` (`studio`).
pub fn device_label() -> String {
    platform::device_label().unwrap_or_else(|| "this computer".into())
}
pub use collab::{CollabBackend, CollabService};
pub use identity::{Caller, Identify, ProcessIdentity};
pub use server::{serve, Config, Handle};
pub use settings::SettingsBackend;
pub use workspace::WorkspaceBackend;

