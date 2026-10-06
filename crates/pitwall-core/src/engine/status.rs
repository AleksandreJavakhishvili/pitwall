//! Turning raw signals (hooks, screen rules, output activity) into a Status.

use pitwall_detect::Detected;
use crate::hooks::HookState;
use crate::model::{Source, Status};

/// Output within this window counts as "working" for the activity fallback.
pub const ACTIVITY_WINDOW_MS: u64 = 1200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raw {
    Working,
    Blocked,
    Idle,
    /// Turn finished (hooks only).
    Done,
    Unknown,
}

/// Pick the signal by priority: hooks > screen > activity.
pub fn raw_state(hook: Option<HookState>, screen: Option<Detected>, recent_output: bool) -> (Raw, Source) {
    if let Some(h) = hook {
        let raw = match h {
            HookState::Working => Raw::Working,
            HookState::Blocked => Raw::Blocked,
            HookState::Done => Raw::Done,
            HookState::Idle => Raw::Idle,
        };
        return (raw, Source::Hooks);
    }
    if let Some(s) = screen {
        let raw = match s {
            Detected::Working => Raw::Working,
            Detected::Blocked => Raw::Blocked,
            Detected::Idle => Raw::Idle,
        };
        return (raw, Source::Screen);
    }
    let raw = if recent_output { Raw::Working } else { Raw::Unknown };
    (raw, Source::Activity)
}

/// Fold a raw signal into the displayed status. `turn_active` remembers that
/// work happened since the agent last went quiet, so `idle` after work is
/// reported as `done` until the user looks (`mark_seen`).
pub fn next_status(prev: Status, raw: Raw, turn_active: &mut bool) -> Status {
    match raw {
        Raw::Working => {
            *turn_active = true;
            Status::Working
        }
        Raw::Blocked => {
            *turn_active = true;
            Status::Blocked
        }
        Raw::Done => {
            *turn_active = false;
            Status::Done
        }
        Raw::Idle if *turn_active => {
            *turn_active = false;
            Status::Done
        }
        Raw::Idle | Raw::Unknown if prev == Status::Done => Status::Done,
        Raw::Idle => Status::Idle,
        Raw::Unknown => Status::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_beat_screen_beat_activity() {
        assert_eq!(
            raw_state(Some(HookState::Working), Some(Detected::Idle), false),
            (Raw::Working, Source::Hooks)
        );
        assert_eq!(
            raw_state(None, Some(Detected::Blocked), true),
            (Raw::Blocked, Source::Screen)
        );
        assert_eq!(raw_state(None, None, true), (Raw::Working, Source::Activity));
        assert_eq!(raw_state(None, None, false), (Raw::Unknown, Source::Activity));
    }

    #[test]
    fn idle_after_work_is_done_until_seen() {
        let mut turn = false;
        let s = next_status(Status::Unknown, Raw::Idle, &mut turn);
        assert_eq!(s, Status::Idle);
        let s = next_status(s, Raw::Working, &mut turn);
        assert_eq!(s, Status::Working);
        let s = next_status(s, Raw::Idle, &mut turn);
        assert_eq!(s, Status::Done);
        let s = next_status(s, Raw::Idle, &mut turn);
        assert_eq!(s, Status::Done);
        // mark_seen sets Idle; it stays idle.
        assert_eq!(next_status(Status::Idle, Raw::Idle, &mut turn), Status::Idle);
    }

    #[test]
    fn blocked_then_idle_is_done() {
        let mut turn = false;
        let s = next_status(Status::Idle, Raw::Blocked, &mut turn);
        assert_eq!(next_status(s, Raw::Idle, &mut turn), Status::Done);
    }
}
