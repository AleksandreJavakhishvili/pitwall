//! What the app and `pitwall-simhost` (an iOS Simulator's screen and
//! input, docs/spec/device.md) say to each other:
//!
//! - its stdout: the screen, as [`super::video`];
//! - its stdin: input, as fixed 24-byte [`Input`] records;
//! - its stderr: one JSON [`Status`] per line (other lines are its log).
//!
//! Positions are ratios of the screen (0..1 from the top left, in the
//! screen's own orientation), so the app needs no point size or scale.

use serde::{Deserialize, Serialize};

/// The size of one input record.
pub const RECORD: usize = 24;

/// A touch's phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

/// A hardware button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Home,
    Lock,
    Siri,
}

/// One input event for the simulator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// One finger at (`x`, `y`), ratios of the screen.
    Touch { phase: Phase, x: f64, y: f64 },
    /// A key of the USB HID keyboard page (0x07), e.g. 0x04 for A.
    Key { down: bool, usage: u16 },
    Button { button: Button, down: bool },
}

impl Input {
    pub fn encode(&self) -> [u8; RECORD] {
        let mut b = [0u8; RECORD];
        match *self {
            Input::Touch { phase, x, y } => {
                b[0] = 1;
                b[1] = match phase {
                    Phase::Down => 0,
                    Phase::Move => 1,
                    Phase::Up => 2,
                };
                b[4..12].copy_from_slice(&x.to_be_bytes());
                b[12..20].copy_from_slice(&y.to_be_bytes());
            }
            Input::Key { down, usage } => {
                b[0] = 2;
                b[1] = down as u8;
                b[2..4].copy_from_slice(&usage.to_be_bytes());
            }
            Input::Button { button, down } => {
                b[0] = 3;
                b[1] = down as u8;
                b[2] = match button {
                    Button::Home => 0,
                    Button::Lock => 1,
                    Button::Siri => 2,
                };
            }
        }
        b
    }

    /// A record read back; `None` for one this build doesn't know (skipped).
    pub fn decode(b: &[u8; RECORD]) -> Option<Input> {
        let f = |r: &[u8]| f64::from_be_bytes(r.try_into().unwrap());
        match b[0] {
            1 => {
                let phase = match b[1] {
                    0 => Phase::Down,
                    1 => Phase::Move,
                    2 => Phase::Up,
                    _ => return None,
                };
                let (x, y) = (f(&b[4..12]), f(&b[12..20]));
                if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                    return None;
                }
                Some(Input::Touch { phase, x, y })
            }
            2 => Some(Input::Key { down: b[1] != 0, usage: u16::from_be_bytes([b[2], b[3]]) }),
            3 => {
                let button = match b[2] {
                    0 => Button::Home,
                    1 => Button::Lock,
                    2 => Button::Siri,
                    _ => return None,
                };
                Some(Input::Button { button, down: b[1] != 0 })
            }
            _ => None,
        }
    }
}

/// A line the helper writes on stderr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Status {
    /// Streaming. `input` is false (with `why`) when this Xcode's input
    /// interfaces weren't found: the tile stays view-only.
    #[serde(rename_all = "camelCase")]
    Ready {
        width: u32,
        height: u32,
        input: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
        /// The Xcode the helper loaded the simulator frameworks from.
        #[serde(default)]
        xcode: String,
    },
    /// It stopped: the simulator shut down or something failed. The
    /// helper exits after this.
    Stopped { why: String },
}

impl Status {
    /// The line for stderr.
    pub fn line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// A stderr line read back; `None` for the helper's other log lines.
    pub fn parse(line: &str) -> Option<Status> {
        let line = line.trim();
        if !line.starts_with('{') {
            return None;
        }
        serde_json::from_str(line).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_round_trip_through_their_records() {
        for i in [
            Input::Touch { phase: Phase::Down, x: 0.25, y: 0.75 },
            Input::Touch { phase: Phase::Move, x: 0.0, y: 1.0 },
            Input::Touch { phase: Phase::Up, x: 0.5, y: 0.5 },
            Input::Key { down: true, usage: 0x04 },
            Input::Key { down: false, usage: 0x28 },
            Input::Button { button: Button::Home, down: true },
            Input::Button { button: Button::Lock, down: false },
            Input::Button { button: Button::Siri, down: true },
        ] {
            assert_eq!(Input::decode(&i.encode()), Some(i));
        }
    }

    #[test]
    fn unknown_or_out_of_screen_records_are_skipped() {
        let mut b = [0u8; RECORD];
        b[0] = 9;
        assert_eq!(Input::decode(&b), None);
        let mut b = Input::Touch { phase: Phase::Down, x: 0.5, y: 0.5 }.encode();
        b[4..12].copy_from_slice(&1.5f64.to_be_bytes());
        assert_eq!(Input::decode(&b), None);
        let mut b = Input::Touch { phase: Phase::Down, x: 0.5, y: 0.5 }.encode();
        b[1] = 7;
        assert_eq!(Input::decode(&b), None);
    }

    #[test]
    fn status_lines_parse_and_other_log_lines_dont() {
        let s = Status::Ready { width: 1206, height: 2622, input: false, why: Some("no SimDeviceLegacyHIDClient".into()), xcode: "26.1.1".into() };
        assert_eq!(Status::parse(&s.line()), Some(s));
        assert_eq!(Status::parse(r#"{"event":"stopped","why":"shut down"}"#), Some(Status::Stopped { why: "shut down".into() }));
        assert_eq!(Status::parse("loading SimulatorKit"), None);
        assert_eq!(Status::parse(r#"{"event":"future"}"#), None);
    }
}
