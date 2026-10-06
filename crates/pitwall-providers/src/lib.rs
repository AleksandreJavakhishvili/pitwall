//! Places agents run (architecture.md §1): each module implements
//! [`pitwall_core::provider::Provider`] for one kind of place and is a cargo
//! feature. The engine never names them; it reads their capabilities.
//!
//! - `local` — this Mac. Each agent runs in its own `pitwall-hold` terminal
//!   holder (§9 decision 1), so it outlives Pitwall; commands run through
//!   `LocalExec`.
//!
//! Every provider passes `pitwall_core::testing::contract` (see `tests/`).

#[cfg(feature = "local")]
pub mod local;
