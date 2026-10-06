//! Pitwall's logic, with no UI framework in it (architecture.md §1, §6 step 2).
//!
//! A host (the Tauri app today, the daemon later) builds an
//! [`Engine`](engine::Engine) from [`Deps`](engine::Deps) — where Pitwall's
//! files live ([`Paths`](paths::Paths)), where events go
//! ([`EventSink`](events::EventSink)), time ([`Clock`](clock::Clock)) and
//! persistence ([`Store`](store::Store)), and how to run programs and reach
//! files where agents work ([`Exec`](exec::Exec)) — and calls the service modules:
//! `engine::{input, lifecycle, changes}`, `review`, `rules`, `onboarding`.

pub mod clock;
pub mod engine;
pub mod events;
pub mod exec;
pub mod hooks;
pub mod kind;
pub mod model;
pub mod onboarding;
pub mod paths;
pub mod procs;
pub mod review;
pub mod rules;
pub mod session;
pub mod store;
pub mod vcs;

mod platform;
mod shell;
mod slug;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use engine::{Deps, Engine, Shared};
