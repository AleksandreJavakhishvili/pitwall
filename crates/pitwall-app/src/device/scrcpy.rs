//! scrcpy-server, the program Android device tiles run on the device
//! (Apache-2.0, github.com/Genymobile/scrcpy), at the version Pitwall
//! pins: its command line, its control messages, and where its file comes
//! from. Layouts checked against scrcpy v5.0.1 (`ControlMessageReader.java`,
//! `control_msg.c`, `server.c`; docs/spec/device.md).
//!
//! Pitwall doesn't ship or download the server yet (docs/spec/device.md,
//! "Getting scrcpy-server"): [`ServerSource`] is the seam. Today it finds a
//! file the user put in place and checks its SHA-256 against the pinned
//! one; nothing else is ever pushed to a device.

use std::path::{Path, PathBuf};

/// The server version Pitwall speaks; the server refuses any other
/// (`Options.java`: the first argument must equal its version).
pub const VERSION: &str = "5.0.1";

/// SHA-256 of `scrcpy-server-v5.0.1`, as scrcpy publishes it
/// (`SHA256SUMS.txt` on the release, `doc/build.md`, `install_release.sh`).
pub const SHA256: &str = "764eb6f79811d5211fe9df341120882ba9994c7a61b897d7bf3fb662e53bc536";

/// Where it goes on the device (scrcpy's own path).
pub const DEVICE_PATH: &str = "/data/local/tmp/scrcpy-server.jar";

/// The abstract socket the server listens on, for a session id.
pub fn socket_name(scid: u32) -> String {
    format!("scrcpy_{:08x}", scid & 0x7fff_ffff)
}

/// What the tile asks of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerOptions {
    /// 31-bit session id: its socket's name, so two tiles never meet.
    pub scid: u32,
    /// The longest side, in pixels (0: the device's own).
    pub max_size: u32,
    pub max_fps: u32,
    pub bit_rate: u32,
    /// Touch and keys go to the device (false: watch only).
    pub control: bool,
}

/// `adb shell`'s command that starts the server (forward tunnel: the
/// server listens, Pitwall connects).
pub fn server_command(o: &ServerOptions) -> String {
    let mut args = vec![
        format!("CLASSPATH={DEVICE_PATH}"),
        "app_process".into(),
        "/".into(),
        "com.genymobile.scrcpy.Server".into(),
        VERSION.into(),
        format!("scid={:08x}", o.scid & 0x7fff_ffff),
        "log_level=warn".into(),
        "tunnel_forward=true".into(),
        "audio=false".into(),
        "video_codec=h264".into(),
        format!("control={}", o.control),
        // No device name: the tile has it from adb.
        "send_device_meta=false".into(),
        format!("max_size={}", o.max_size),
        format!("video_bit_rate={}", o.bit_rate),
        // Leave the device as it was when the tile closes.
        "cleanup=true".into(),
        "power_on=true".into(),
    ];
    if o.max_fps > 0 {
        args.push(format!("max_fps={}", o.max_fps));
    }
    args.join(" ")
}

/// Android's `MotionEvent` actions scrcpy passes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchAction {
    Down = 0,
    Up = 1,
    Move = 2,
}

/// scrcpy's pointer id for a generic finger (`POINTER_ID_GENERIC_FINGER`).
pub const FINGER: u64 = -2i64 as u64;

/// A message to the server's control socket.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// One finger at `pos` in a frame of `size` (the current video size,
    /// or the server drops it).
    Touch { action: TouchAction, pos: (i32, i32), size: (u32, u32) },
    /// An Android key code (`KeyEvent.KEYCODE_*`), down or up.
    Key { down: bool, keycode: u32, meta: u32 },
    /// Text typed as is (at most 300 bytes, cut at a character).
    Text(String),
    /// Back, or the screen on when it is off.
    BackOrScreenOn { down: bool },
}

/// The longest text scrcpy takes in one message.
pub const TEXT_MAX: usize = 300;

impl Control {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Control::Key { down, keycode, meta } => {
                b.push(0);
                b.push(if *down { 0 } else { 1 });
                b.extend_from_slice(&keycode.to_be_bytes());
                b.extend_from_slice(&0u32.to_be_bytes());
                b.extend_from_slice(&meta.to_be_bytes());
            }
            Control::Text(t) => {
                let mut end = t.len().min(TEXT_MAX);
                while !t.is_char_boundary(end) {
                    end -= 1;
                }
                b.push(1);
                b.extend_from_slice(&(end as u32).to_be_bytes());
                b.extend_from_slice(&t.as_bytes()[..end]);
            }
            Control::Touch { action, pos, size } => {
                b.push(2);
                b.push(*action as u8);
                b.extend_from_slice(&FINGER.to_be_bytes());
                b.extend_from_slice(&pos.0.to_be_bytes());
                b.extend_from_slice(&pos.1.to_be_bytes());
                b.extend_from_slice(&(size.0.min(u16::MAX as u32) as u16).to_be_bytes());
                b.extend_from_slice(&(size.1.min(u16::MAX as u32) as u16).to_be_bytes());
                // Pressure 1.0 while down (0xffff), 0 when lifted.
                let pressure: u16 = if *action == TouchAction::Up { 0 } else { 0xffff };
                b.extend_from_slice(&pressure.to_be_bytes());
                b.extend_from_slice(&0u32.to_be_bytes()); // action button
                b.extend_from_slice(&0u32.to_be_bytes()); // buttons
            }
            Control::BackOrScreenOn { down } => {
                b.push(4);
                b.push(if *down { 0 } else { 1 });
            }
        }
        b
    }
}

/// How many bytes of a server-to-client message on the control socket
/// there are to skip (clipboard, its ack, UHID output), given what has
/// arrived; `None` until enough has.
pub fn device_message_len(buf: &[u8]) -> Option<usize> {
    let u32_at = |i: usize| buf.get(i..i + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize);
    match buf.first()? {
        0 => u32_at(1).map(|n| 5 + n),
        1 => Some(9),
        2 => buf.get(3..5).map(|b| 5 + u16::from_be_bytes([b[0], b[1]]) as usize),
        // Unknown: the rest is unreadable; skip everything.
        _ => Some(buf.len()),
    }
}

/// Where scrcpy-server comes from.
pub trait ServerSource: Send + Sync {
    /// The checked server file, or why there is none.
    fn server(&self) -> Result<PathBuf, String>;
}

/// The file the user put in Pitwall's data folder
/// (`<data>/device/scrcpy-server-v5.0.1`), used only if its SHA-256 is the
/// pinned one. Pitwall neither ships nor downloads it (yet): an owner's
/// decision (docs/spec/device.md).
pub struct UserFile {
    pub path: PathBuf,
}

impl UserFile {
    pub fn in_data(data: &Path) -> UserFile {
        UserFile { path: data.join("device").join(format!("scrcpy-server-v{VERSION}")) }
    }
}

/// A file's SHA-256, lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

impl ServerSource for UserFile {
    fn server(&self) -> Result<PathBuf, String> {
        let bytes = std::fs::read(&self.path).map_err(|_| {
            format!("Android screens need scrcpy-server {VERSION}, which Pitwall doesn't include yet. Put it at {}", self.path.display())
        })?;
        if sha256_hex(&bytes) != SHA256 {
            return Err(format!("{} isn't scrcpy-server {VERSION} (its SHA-256 doesn't match)", self.path.display()));
        }
        Ok(self.path.clone())
    }
}
