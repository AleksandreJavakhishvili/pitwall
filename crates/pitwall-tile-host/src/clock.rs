//! A tile's clock: the time its timers, its rate limits and its WASI
//! clocks (`Instant`, `SystemTime` in the guest) read. The app's is the
//! wall clock; a test's can be virtual ([`Clock::manual`]), moved only by
//! [`TileHandle::advance`](crate::TileHandle::advance), so a test of a
//! timer doesn't sleep. Deadlines of calls are always the wall clock.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wasmtime_wasi::{HostMonotonicClock, HostWallClock};

/// The time a tile lives by.
#[derive(Debug, Clone, Default)]
pub enum Clock {
    /// The computer's clock (the app).
    #[default]
    Wall,
    /// A virtual clock that stands still until the test moves it.
    Manual(ManualClock),
}

/// A virtual clock: starts at the moment it was made, moves only when
/// told.
#[derive(Debug, Clone)]
pub struct ManualClock {
    inner: Arc<Mutex<Manual>>,
}

#[derive(Debug)]
struct Manual {
    start: Instant,
    wall: SystemTime,
    passed: Duration,
}

impl ManualClock {
    fn get(&self) -> (Instant, SystemTime, Duration) {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        (m.start, m.wall, m.passed)
    }

    /// The time passed since it was made.
    pub fn passed(&self) -> Duration {
        self.get().2
    }

    /// Move it on to `at` (never back).
    pub(crate) fn advance_to(&self, at: Instant) {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let passed = at.saturating_duration_since(m.start);
        m.passed = m.passed.max(passed);
    }
}

impl Clock {
    /// A virtual clock, at the wall clock's time now.
    pub fn manual() -> Clock {
        Clock::Manual(ManualClock { inner: Arc::new(Mutex::new(Manual { start: Instant::now(), wall: SystemTime::now(), passed: Duration::ZERO })) })
    }

    /// Whether it is virtual.
    pub fn is_manual(&self) -> bool {
        matches!(self, Clock::Manual(_))
    }

    /// Now, as an `Instant` (a virtual clock's start plus the time it was
    /// moved).
    pub fn now(&self) -> Instant {
        match self {
            Clock::Wall => Instant::now(),
            Clock::Manual(m) => {
                let (start, _, passed) = m.get();
                start + passed
            }
        }
    }

    /// The wall time now.
    pub fn system_now(&self) -> SystemTime {
        match self {
            Clock::Wall => SystemTime::now(),
            Clock::Manual(m) => {
                let (_, wall, passed) = m.get();
                wall + passed
            }
        }
    }

    /// The guest's WASI clocks for a virtual clock (`None`: WASI's own).
    pub(crate) fn wasi(&self) -> Option<(GuestWall, GuestMonotonic)> {
        match self {
            Clock::Wall => None,
            Clock::Manual(m) => Some((GuestWall(m.clone()), GuestMonotonic(m.clone()))),
        }
    }
}

/// `wasi:clocks/wall-clock` from a virtual clock.
pub(crate) struct GuestWall(ManualClock);

impl HostWallClock for GuestWall {
    fn resolution(&self) -> Duration {
        Duration::from_nanos(1)
    }

    fn now(&self) -> Duration {
        let (_, wall, passed) = self.0.get();
        (wall + passed).duration_since(UNIX_EPOCH).unwrap_or_default()
    }
}

/// `wasi:clocks/monotonic-clock` from a virtual clock, in nanoseconds.
pub(crate) struct GuestMonotonic(ManualClock);

impl HostMonotonicClock for GuestMonotonic {
    fn resolution(&self) -> u64 {
        1
    }

    fn now(&self) -> u64 {
        // Not 0 at start: some code takes 0 as "never".
        (Duration::from_secs(1) + self.0.passed()).as_nanos() as u64
    }
}
