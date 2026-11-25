//! iOS Simulators (macOS): listed with `xcrun simctl`, shown and driven
//! through `pitwall-simhost`, a helper of Pitwall's that loads Xcode's
//! simulator frameworks in its own process (docs/spec/device.md, "iOS
//! Simulator"). The app never loads them.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use pitwall_proto::device::sim::{self, Status};

use super::stream::{Backend, Connection, Input, InputSink, Key, Phase, Stop};
use super::Device;

/// The simulators Xcode knows (none, with why, without Xcode).
pub fn list() -> Result<Vec<Device>, String> {
    let out = Command::new("xcrun")
        .args(["simctl", "list", "devices", "-j"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Xcode isn't installed".to_string())?;
    if !out.status.success() {
        return Err("Xcode's simctl didn't answer".into());
    }
    super::list::parse_simctl(&String::from_utf8_lossy(&out.stdout))
}

/// The HID keyboard usage (page 0x07) and whether Shift is held, for a
/// character on a US layout; `None` for one with no key.
pub fn usage_of(c: char) -> Option<(u16, bool)> {
    let lower = |c: char| (c as u16 - 'a' as u16) + 0x04;
    Some(match c {
        'a'..='z' => (lower(c), false),
        'A'..='Z' => (lower(c.to_ascii_lowercase()), true),
        '1'..='9' => ((c as u16 - '1' as u16) + 0x1e, false),
        '0' => (0x27, false),
        '\n' => (0x28, false),
        '\t' => (0x2b, false),
        ' ' => (0x2c, false),
        '-' => (0x2d, false),
        '=' => (0x2e, false),
        '[' => (0x2f, false),
        ']' => (0x30, false),
        '\\' => (0x31, false),
        ';' => (0x33, false),
        '\'' => (0x34, false),
        '`' => (0x35, false),
        ',' => (0x36, false),
        '.' => (0x37, false),
        '/' => (0x38, false),
        '!' => (0x1e, true),
        '@' => (0x1f, true),
        '#' => (0x20, true),
        '$' => (0x21, true),
        '%' => (0x22, true),
        '^' => (0x23, true),
        '&' => (0x24, true),
        '*' => (0x25, true),
        '(' => (0x26, true),
        ')' => (0x27, true),
        '_' => (0x2d, true),
        '+' => (0x2e, true),
        '{' => (0x2f, true),
        '}' => (0x30, true),
        '|' => (0x31, true),
        ':' => (0x33, true),
        '"' => (0x34, true),
        '~' => (0x35, true),
        '<' => (0x36, true),
        '>' => (0x37, true),
        '?' => (0x38, true),
        _ => return None,
    })
}

/// A key's HID usage.
pub fn usage(k: Key) -> u16 {
    match k {
        Key::Enter => 0x28,
        Key::Escape => 0x29,
        Key::Backspace => 0x2a,
        Key::Tab => 0x2b,
        Key::Space => 0x2c,
        Key::Delete => 0x4c,
        Key::Right => 0x4f,
        Key::Left => 0x50,
        Key::Down => 0x51,
        Key::Up => 0x52,
    }
}

const SHIFT: u16 = 0xe1;

/// The helper's input records for an input.
pub fn records(i: &Input) -> Vec<sim::Input> {
    let key = |down, usage| sim::Input::Key { down, usage };
    match i {
        Input::Touch { phase, at } => {
            let phase = match phase {
                Phase::Down => sim::Phase::Down,
                Phase::Move => sim::Phase::Move,
                Phase::Up => sim::Phase::Up,
            };
            vec![sim::Input::Touch { phase, x: at.0.clamp(0., 1.), y: at.1.clamp(0., 1.) }]
        }
        Input::Key { down, key: k } => vec![key(*down, usage(*k))],
        Input::Text(t) => {
            let mut out = Vec::new();
            for c in t.chars() {
                let Some((u, shift)) = usage_of(c) else { continue };
                if shift {
                    out.push(key(true, SHIFT));
                }
                out.push(key(true, u));
                out.push(key(false, u));
                if shift {
                    out.push(key(false, SHIFT));
                }
            }
            out
        }
        Input::Back => vec![],
        Input::Home => vec![sim::Input::Button { button: sim::Button::Home, down: true }, sim::Input::Button { button: sim::Button::Home, down: false }],
    }
}

/// `pitwall-simhost` next to the app (as `pitwall-hold` is), or the one
/// `cargo build` made while developing.
pub fn helper() -> Option<PathBuf> {
    let name = format!("pitwall-simhost{}", std::env::consts::EXE_SUFFIX);
    if let Some(p) = std::env::var_os("PITWALL_SIMHOST").map(PathBuf::from) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(&name), dir.parent()?.join(&name)].into_iter().find(|p| p.is_file())
}

struct HelperInput(std::process::ChildStdin);

impl InputSink for HelperInput {
    fn send(&mut self, i: &Input, _frame: (u32, u32)) -> std::io::Result<()> {
        for r in records(i) {
            self.0.write_all(&r.encode())?;
        }
        self.0.flush()
    }
}

/// Ends the helper. Holds its stdin when no input goes there: the helper
/// ends when its stdin closes.
struct Kill(std::process::Child, Option<std::process::ChildStdin>);

impl Stop for Kill {
    fn stop(&mut self) {
        self.1 = None;
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// iOS tiles' backend.
pub struct Simulator {
    pub helper: Option<PathBuf>,
    /// A device set other than Xcode's default (`simctl --set`).
    pub set: Option<PathBuf>,
    /// How long to wait for the helper's first word.
    pub patience: Duration,
}

impl Backend for Simulator {
    fn connect(&self, device: &Device) -> Result<Connection, String> {
        let helper = self.helper.clone().ok_or("pitwall-simhost is missing next to the app")?;
        let mut cmd = Command::new(helper);
        cmd.args(["stream", &device.id]);
        if let Some(set) = &self.set {
            cmd.arg("--set").arg(set);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("pitwall-simhost didn't start: {e}"))?;
        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let mut kill = Kill(child, None);
        let (Some(stdin), Some(stdout), Some(stderr)) = (stdin, stdout, stderr) else {
            kill.stop();
            return Err("pitwall-simhost has no pipes".into());
        };
        // Its first status line says whether it streams and takes input;
        // later lines (and its log) are read and dropped.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut tx = Some(tx);
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(s) = Status::parse(&line) {
                    if let Some(t) = tx.take() {
                        let _ = t.send(s);
                    }
                }
            }
        });
        let input = match rx.recv_timeout(self.patience) {
            Ok(Status::Ready { input: true, .. }) => Ok(Box::new(HelperInput(stdin)) as Box<dyn InputSink>),
            Ok(Status::Ready { why, xcode, .. }) => {
                kill.1 = Some(stdin);
                Err(why.unwrap_or_else(|| format!("input isn't available with Xcode {xcode}")))
            }
            Ok(Status::Stopped { why }) => {
                kill.stop();
                return Err(why);
            }
            Err(_) => {
                kill.stop();
                return Err("the simulator didn't start streaming".into());
            }
        };
        Ok(Connection { video: Box::new(stdout), skip: 0, input, stop: Box::new(kill) })
    }
}
