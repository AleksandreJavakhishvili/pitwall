//! Wall-clock time for collaboration, injected. Daily limits reset at the
//! owner's local midnight (§5.8), turns time out (§6.6) and records carry
//! RFC 3339 times; tests drive all of it by hand ([`ManualTime`]).

use std::sync::Mutex;

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveTime, TimeZone};

/// The owner's wall clock, in their local time zone.
pub trait WallClock: Send + Sync {
    /// Now, with the owner's local offset.
    fn now(&self) -> DateTime<FixedOffset>;

    /// The next local midnight after `t` (when daily limits reset).
    fn next_midnight(&self, t: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
        let next_day = t.date_naive().succ_opt().unwrap_or(t.date_naive());
        let naive = next_day.and_time(NaiveTime::MIN);
        t.offset().from_local_datetime(&naive).single().unwrap_or(t + Duration::days(1))
    }
}

/// The machine's clock and time zone.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTime;

impl WallClock for SystemTime {
    fn now(&self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    fn next_midnight(&self, t: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
        // The local zone's own midnight (DST changes the offset).
        let local = t.with_timezone(&Local);
        let next_day = local.date_naive().succ_opt().unwrap_or(local.date_naive());
        match Local.from_local_datetime(&next_day.and_time(NaiveTime::MIN)).earliest() {
            Some(m) => m.fixed_offset(),
            None => t + Duration::days(1),
        }
    }
}

/// A clock that moves only when told to, at a fixed offset.
pub struct ManualTime(Mutex<DateTime<FixedOffset>>);

impl ManualTime {
    /// Starting at `rfc3339` (`2026-10-16T09:00:00+02:00`).
    pub fn at(rfc3339: &str) -> ManualTime {
        ManualTime(Mutex::new(DateTime::parse_from_rfc3339(rfc3339).expect("an RFC 3339 time")))
    }

    pub fn advance(&self, by: std::time::Duration) {
        let mut t = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *t += Duration::from_std(by).unwrap_or_default();
    }

    pub fn set(&self, rfc3339: &str) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = DateTime::parse_from_rfc3339(rfc3339).expect("an RFC 3339 time");
    }
}

impl WallClock for ManualTime {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `2026-10-16T09:12:00+02:00`.
pub fn rfc3339(t: DateTime<FixedOffset>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// `2026-10-16`, the owner's local day of `t`.
pub fn day(t: DateTime<FixedOffset>) -> String {
    t.format("%Y-%m-%d").to_string()
}

/// `00:00`, for sentences ("resets at 00:00").
pub fn clock_time(t: DateTime<FixedOffset>) -> String {
    t.format("%H:%M").to_string()
}

pub fn parse(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midnight_is_the_owners_next_local_midnight() {
        let t = ManualTime::at("2026-10-16T23:59:30+02:00");
        let m = t.next_midnight(t.now());
        assert_eq!(rfc3339(m), "2026-10-17T00:00:00+02:00");
        assert_eq!(day(t.now()), "2026-10-16");
        t.advance(std::time::Duration::from_secs(60));
        assert_eq!(day(t.now()), "2026-10-17");
        assert_eq!(clock_time(m), "00:00");
    }
}
