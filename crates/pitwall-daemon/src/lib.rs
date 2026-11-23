//! Pitwall's socket server (architecture.md §4). It runs inside the app for
//! now (step 5); `pitwalld` (step 6) will host the same [`server::serve`].
//!
//! - [`server`]: listen on `~/.pitwall/run/pitwalld.sock` (folder 0700,
//!   socket 0600), handshake, answer requests.
//! - [`methods`]: the method table with each method's access/risk, and what
//!   each method does in `pitwall-core`.
//! - [`approvals`]: risky requests wait for the user's answer in Pitwall.
//! - [`events`]: the engine's events to subscribed connections
//!   (`events.subscribe`), never blocking the engine.
//! - [`identity`]: who is calling (peer pid → the agent it runs in, or the
//!   extension whose adapter started it).
//! - [`extensions`]: extensions as callers: their grants, adapters, call
//!   budget and activity.
//! - [`settings`]: `settings.*` over the host's [`SettingsBackend`].
//! - [`manage`]: stop/restart/remove/rename/status/wait, queues, projects,
//!   rules, review.
//! - [`profiles`]: accounts, profiles and projects' picks (`accounts.json`).
//! - [`workspace`]: `space.*` over the host's [`WorkspaceBackend`].
//! - [`viewer`]: `viewer.open` over the host's [`ViewerBackend`].

pub mod approvals;
pub mod events;
pub mod extensions;
pub mod identity;
pub mod manage;
pub mod methods;
mod platform;
pub mod profiles;
pub mod server;
pub mod settings;
pub mod viewer;
pub mod workspace;

pub use approvals::{Approvals, Decision};
pub use events::Hub;
pub use identity::{Caller, Identify, ProcessIdentity};
pub use extensions::Registry;
pub use server::{build, listen, serve, Config, Handle, Server};
pub use settings::SettingsBackend;
pub use viewer::ViewerBackend;
pub use workspace::WorkspaceBackend;

