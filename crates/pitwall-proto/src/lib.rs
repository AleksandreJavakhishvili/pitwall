//! The Pitwall wire contract (architecture.md §1, §4), shared by the server
//! (in the app today, `pitwalld` later), `pitwall-client`, the `pitwall` CLI
//! and — as generated TypeScript in `src/gen/` — the UI.
//!
//! - [`frame`]: length-prefixed frames, JSON or terminal bytes.
//! - [`msg`]: the versioned handshake, requests, responses, events.
//! - [`api`]: method names, their parameters and results, approvals.
//! - [`create`]: the form a provider describes for new agents on a machine.
//! - [`explorer`]: the read-only code explorer (tree, file, search).
//! - [`manage`]: managing agents, queues, spaces, projects, rules, review.
//! - [`views`]: what clients are shown (`AgentView`, sessions, …).
//! - [`pipe`]: the Windows named pipe for a socket path.
//! - [`settings`]: the settings registry (keys, types, defaults, schema).
//!
//! Serde only: no sockets, no engine.

pub mod api;
pub mod create;
pub mod explorer;
pub mod frame;
pub mod manage;
pub mod msg;
pub mod pipe;
pub mod settings;
pub mod views;

pub use api::*;
pub use create::*;
pub use explorer::*;
pub use manage::*;
pub use msg::*;
pub use settings::{SettingKey, SettingSet, SettingView};
pub use views::*;

/// The protocol version this build speaks.
pub const PROTOCOL: u32 = 1;

/// Environment of every agent and terminal Pitwall starts: the socket the
/// `pitwall` CLI talks to.
pub const SOCKET_ENV: &str = "PITWALL_CLI_SOCKET";
/// Environment of every agent and terminal Pitwall starts: its agent id.
/// Only a convenience for the caller — the server never trusts it.
pub const AGENT_ENV: &str = "PITWALL_AGENT_ID";
/// The socket's file name under Pitwall's `run/` folder.
pub const SOCKET_NAME: &str = "pitwalld.sock";

#[cfg(test)]
mod ts_export {
    use std::path::PathBuf;
    use ts_rs::TS;

    use super::*;

    /// `cargo test` regenerates the UI's types (src/gen/*.ts); the UI build
    /// then catches drift.
    #[test]
    fn export_typescript() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/gen");
        let cfg = ts_rs::Config::default().with_out_dir(&dir);
        AgentView::export_all(&cfg).unwrap();
        ScannedPlace::export_all(&cfg).unwrap();
        ApprovalView::export_all(&cfg).unwrap();
        ApprovalAnswer::export_all(&cfg).unwrap();
        SessionAdded::export_all(&cfg).unwrap();
        SessionAdd::export_all(&cfg).unwrap();
        ProviderMachines::export_all(&cfg).unwrap();
        AgentCreate::export_all(&cfg).unwrap();
        CreateForm::export_all(&cfg).unwrap();
        FormRequest::export_all(&cfg).unwrap();
        ProjectWorktrees::export_all(&cfg).unwrap();
        ScreenFrame::export_all(&cfg).unwrap();
        DirListing::export_all(&cfg).unwrap();
        FileIndex::export_all(&cfg).unwrap();
        FileView::export_all(&cfg).unwrap();
        SearchQuery::export_all(&cfg).unwrap();
        SearchResult::export_all(&cfg).unwrap();
        SettingView::export_all(&cfg).unwrap();
        SettingSet::export_all(&cfg).unwrap();
        SettingKey::export_all(&cfg).unwrap();
        assert!(dir.join("AgentView.ts").is_file());
    }
}
