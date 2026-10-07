//! Places agents run (architecture.md §1): each module implements
//! [`pitwall_core::provider::Provider`] for one kind of place and is a cargo
//! feature. The engine never names them; it reads their capabilities.
//!
//! - `local` — this Mac. Each agent runs in its own `pitwall-hold` terminal
//!   holder (§9 decision 1), so it outlives Pitwall; commands run through
//!   `LocalExec`.
//! - `agw` — sessions on agw VMs (§2.7), adopted rather than created (step
//!   8a): the terminal is `agw session attach` running in a holder, status
//!   comes from the screen, stop/start are agw's own commands; diffs and
//!   Review run through agw's `vm exec` / `agent exec` (`agw::AgwExec`).
//!
//! Every provider passes `pitwall_core::testing::contract` (see `tests/`).

#[cfg(any(feature = "local", feature = "agw"))]
pub mod hold;

#[cfg(feature = "agw")]
pub mod agw;
#[cfg(feature = "local")]
pub mod local;
