//! Git in the agent's folder, run through the [`Exec`](crate::exec::Exec) of
//! the machine where the agent works (architecture.md §2.4), so it works the
//! same wherever that is.

pub mod git;
pub(crate) mod review;
pub mod snapshot;
