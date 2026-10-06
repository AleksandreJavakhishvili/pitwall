//! Git, run as a subprocess in the agent's folder (step 3 routes these
//! through an `Exec`, so they also work where the agent runs remotely).

pub mod git;
pub(crate) mod review;
pub(crate) mod snapshot;
