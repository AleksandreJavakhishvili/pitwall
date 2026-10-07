//! Time, injected. The engine's timing (status changes, auto-send settling,
//! echo/activity windows, git refresh) reads a [`Clock`] instead of a global,
//! so tests can drive it by hand (`testing::ManualClock`).

use std::time::Instant;

/// Monotonic milliseconds. Values only mean something relative to each other
/// and must never be 0 (0 is "never" in the engine's bookkeeping).
pub trait Clock: Send + Sync {
    fn mono_ms(&self) -> u64;
}

/// The real clock: milliseconds since this value was made, plus one.
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> SystemClock {
        SystemClock { start: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn mono_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64 + 1
    }
}

/// Wall-clock milliseconds since the Unix epoch (timestamps stored in records).
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic_and_never_zero() {
        let c = SystemClock::new();
        let a = c.mono_ms();
        assert!(a >= 1);
        assert!(c.mono_ms() >= a);
        assert!(unix_ms() > 1_600_000_000_000);
    }
}
