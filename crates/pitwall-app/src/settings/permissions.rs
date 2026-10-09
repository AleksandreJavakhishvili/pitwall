//! macOS folder access for the welcome screen's first step and Settings →
//! Folder access (a port of `src/lib/permissions.ts`): per-folder prompts by
//! default, Full Disk Access as the advanced alternative. The
//! status comes from `pitwall_core::permissions::status`, which is read-only
//! and never makes macOS ask.

use std::time::Duration;

use pitwall_core::permissions::{Access, PermissionsStatus};

/// How often the status is re-read while someone waits for the switch.
pub const POLL: Duration = Duration::from_secs(2);

/// The welcome screen's folder-access step:
/// checking → (already granted / nothing to ask) done
///          → ask → waiting (System Settings opened) → granted → done.
/// "Continue" (macOS's per-folder prompts, the recommended path) is
/// [`AccessEvent::Skip`]: done from anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPhase {
    Checking,
    Ask,
    Waiting,
    Granted,
    Done,
}

#[derive(Debug, Clone)]
pub enum AccessEvent {
    Status(PermissionsStatus),
    Failed,
    Open,
    Continue,
    Skip,
}

impl AccessPhase {
    pub fn step(self, e: &AccessEvent) -> AccessPhase {
        use AccessPhase::*;
        if self == Done {
            return self;
        }
        match e {
            AccessEvent::Skip => Done,
            // A failed check never blocks the welcome screen.
            AccessEvent::Failed => {
                if self == Checking {
                    Done
                } else {
                    self
                }
            }
            AccessEvent::Status(s) => {
                let granted = s.full_disk_access == Access::Granted;
                match self {
                    Checking if !s.applies || s.full_disk_access != Access::Denied => Done,
                    Checking => Ask,
                    Ask | Waiting if granted => Granted,
                    other => other,
                }
            }
            AccessEvent::Open => match self {
                Ask | Waiting => Waiting,
                other => other,
            },
            AccessEvent::Continue => {
                if self == Granted {
                    Done
                } else {
                    self
                }
            }
        }
    }

    /// The status is re-read every [`POLL`] in these phases.
    pub fn polls(self) -> bool {
        matches!(self, AccessPhase::Ask | AccessPhase::Waiting)
    }
}

pub use crate::kit::Tone;

/// Settings → Folder access → Full Disk Access chip (`fdaBadge`).
pub fn badge(s: Option<&PermissionsStatus>) -> (&'static str, Tone) {
    match s {
        None => ("checking…", crate::kit::Tone::Subtle),
        Some(s) if !s.applies => ("not needed", crate::kit::Tone::Subtle),
        Some(s) => match s.full_disk_access {
            Access::Granted => ("granted", crate::kit::Tone::Ok),
            Access::Denied => ("not granted", crate::kit::Tone::Warn),
            Access::Unknown => ("unknown", crate::kit::Tone::Subtle),
        },
    }
}

/// Open System Settings at Privacy → Full Disk Access (shows the pane only).
/// `open` runs in the background: never a program on the UI thread.
pub fn open_full_disk_access() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("privacy settings are a macOS feature".into());
    }
    std::thread::spawn(|| {
        if let Err(e) = pitwall_core::permissions::open_settings("fullDiskAccess") {
            eprintln!("pitwall: {e}");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::permissions::status_from;
    use AccessPhase::*;

    fn st(applies: bool, fda: Access) -> AccessEvent {
        AccessEvent::Status(status_from(applies, fda))
    }

    #[test]
    fn the_step_is_skipped_unless_access_is_denied() {
        assert_eq!(Checking.step(&st(false, Access::Unknown)), Done);
        assert_eq!(Checking.step(&st(true, Access::Granted)), Done);
        assert_eq!(Checking.step(&st(true, Access::Unknown)), Done);
        assert_eq!(Checking.step(&st(true, Access::Denied)), Ask);
        assert_eq!(Checking.step(&AccessEvent::Failed), Done);
    }

    #[test]
    fn waiting_for_the_switch_then_continue() {
        let p = Ask.step(&AccessEvent::Open);
        assert_eq!(p, Waiting);
        assert!(p.polls());
        assert_eq!(p.step(&st(true, Access::Denied)), Waiting);
        let g = p.step(&st(true, Access::Granted));
        assert_eq!(g, Granted);
        assert!(!g.polls());
        assert_eq!(g.step(&AccessEvent::Open), Granted);
        assert_eq!(g.step(&AccessEvent::Continue), Done);
        assert_eq!(Ask.step(&AccessEvent::Skip), Done);
        assert_eq!(Done.step(&st(true, Access::Denied)), Done);
        assert_eq!(Ask.step(&AccessEvent::Failed), Ask);
    }

    #[test]
    fn badges() {
        assert_eq!(badge(None), ("checking…", crate::kit::Tone::Subtle));
        assert_eq!(
            badge(Some(&status_from(false, Access::Unknown))).0,
            "not needed"
        );
        assert_eq!(
            badge(Some(&status_from(true, Access::Granted))),
            ("granted", crate::kit::Tone::Ok)
        );
        assert_eq!(
            badge(Some(&status_from(true, Access::Denied))),
            ("not granted", crate::kit::Tone::Warn)
        );
        assert_eq!(
            badge(Some(&status_from(true, Access::Unknown))).0,
            "unknown"
        );
    }
}
