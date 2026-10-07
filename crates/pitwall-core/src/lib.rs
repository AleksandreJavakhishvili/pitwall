//! Pitwall's logic, with no UI framework in it (architecture.md §1, §6 step 2).
//!
//! A host (the Tauri app today, the daemon later) builds an
//! [`Engine`](engine::Engine) from [`Deps`](engine::Deps) — where Pitwall's
//! files live ([`Paths`](paths::Paths)), where events go
//! ([`EventSink`](events::EventSink)), time ([`Clock`](clock::Clock)) and
//! persistence ([`Store`](store::Store)), and
//! the places agents run ([`Provider`](provider::Provider)s, each with an
//! [`Exec`](exec::Exec) for its machines) — and calls the service modules:
//! `engine::{input, lifecycle, changes}`, `review`, `worktrees`, `rules`,
//! `onboarding`.

pub mod clock;
pub mod engine;
pub mod error;
pub mod events;
pub mod exec;
pub mod hooks;
pub mod host;
pub mod kind;
pub mod model;
pub mod onboarding;
pub mod paths;
pub mod permissions;
pub mod procs;
pub mod provider;
pub mod review;
pub mod rules;
pub mod shell;
pub mod store;
pub mod term;
pub mod vcs;
pub mod worktrees;

mod platform;
mod slug;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use engine::{Deps, Engine, Shared};
