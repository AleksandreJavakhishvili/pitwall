//! Android devices (emulators and phones on a cable): listed with adb, and
//! shown with scrcpy-server (docs/spec/device.md, "Android").
//!
//! Connecting: push the checked server file, forward a local port to its
//! socket (`adb forward tcp:<port> localabstract:scrcpy_<scid>`), start it
//! with `adb shell`, connect the video socket (reading the dummy byte that
//! says the server is there) and the control socket. Stopping: end the
//! server, remove the forward.
//!
//! adb is reached through [`Adb`], so the sequence is tested without a
//! device ([`super::tests`]).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::scrcpy::{self, Control, ServerOptions, ServerSource, TouchAction};
use super::stream::{Backend, Connection, Input, InputSink, Key, Phase, Stop};
use super::{geometry, Device};

/// A process adb started (the server's `adb shell`).
pub trait Child: Send {
    fn kill(&mut self);
}

/// A connected socket's two halves.
pub type Socket = (Box<dyn Read + Send>, Box<dyn Write + Send>);

/// What Android tiles need from adb and the network.
pub trait Adb: Send + Sync {
    /// `adb [-s serial] <args>`; its stdout, or why it failed.
    fn run(&self, serial: Option<&str>, args: &[&str]) -> Result<String, String>;
    /// `adb -s serial shell <command>`, left running.
    fn shell(&self, serial: &str, command: &str) -> Result<Box<dyn Child>, String>;
    /// A free local port.
    fn free_port(&self) -> Result<u16, String>;
    /// A connection to localhost:`port`.
    fn connect(&self, port: u16) -> std::io::Result<Socket>;
}

/// The devices adb knows (none, with why, if there is no adb).
pub fn list(adb: &dyn Adb) -> Result<Vec<Device>, String> {
    adb.run(None, &["devices", "-l"]).map(|out| super::list::parse_adb(&out))
}

/// Android's `KeyEvent.KEYCODE_*` for a key.
pub fn keycode(k: Key) -> u32 {
    match k {
        Key::Enter => 66,
        Key::Backspace => 67,
        Key::Delete => 112,
        Key::Tab => 61,
        Key::Escape => 111,
        Key::Left => 21,
        Key::Right => 22,
        Key::Up => 19,
        Key::Down => 20,
        Key::Space => 62,
    }
}

/// `KEYCODE_HOME`.
const HOME: u32 = 3;

/// scrcpy's control messages for an input.
pub fn controls(i: &Input, frame: (u32, u32)) -> Vec<Control> {
    match i {
        Input::Touch { phase, at } => {
            let action = match phase {
                Phase::Down => TouchAction::Down,
                Phase::Move => TouchAction::Move,
                Phase::Up => TouchAction::Up,
            };
            vec![Control::Touch { action, pos: geometry::to_pixel(*at, frame), size: frame }]
        }
        Input::Key { down, key } => vec![Control::Key { down: *down, keycode: keycode(*key), meta: 0 }],
        Input::Text(t) => vec![Control::Text(t.clone())],
        Input::Back => vec![Control::BackOrScreenOn { down: true }, Control::BackOrScreenOn { down: false }],
        Input::Home => vec![Control::Key { down: true, keycode: HOME, meta: 0 }, Control::Key { down: false, keycode: HOME, meta: 0 }],
    }
}

struct ControlSocket(Box<dyn Write + Send>);

impl InputSink for ControlSocket {
    fn send(&mut self, i: &Input, frame: (u32, u32)) -> std::io::Result<()> {
        for c in controls(i, frame) {
            self.0.write_all(&c.encode())?;
        }
        self.0.flush()
    }
}

/// Undoes a connection: the server ends, the forward goes.
struct Undo {
    adb: Arc<dyn Adb>,
    serial: String,
    port: u16,
    server: Option<Box<dyn Child>>,
}

impl Stop for Undo {
    fn stop(&mut self) {
        if let Some(mut s) = self.server.take() {
            s.kill();
        }
        let _ = self.adb.run(Some(&self.serial), &["forward", "--remove", &format!("tcp:{}", self.port)]);
    }
}

/// Android tiles' backend.
pub struct Android {
    pub adb: Arc<dyn Adb>,
    pub server: Arc<dyn ServerSource>,
    pub options: ServerOptions,
    /// How long to wait for the server to listen.
    pub patience: Duration,
}

impl Backend for Android {
    fn connect(&self, device: &Device) -> Result<Connection, String> {
        let file = self.server.server()?;
        let serial = device.id.as_str();
        let file = file.to_string_lossy();
        self.adb.run(Some(serial), &["push", &file, scrcpy::DEVICE_PATH]).map_err(|e| format!("scrcpy-server couldn't be copied to {}: {e}", device.name))?;
        let port = self.adb.free_port()?;
        let socket = format!("localabstract:{}", scrcpy::socket_name(self.options.scid));
        self.adb.run(Some(serial), &["forward", &format!("tcp:{port}"), &socket]).map_err(|e| format!("adb couldn't forward a port: {e}"))?;
        let mut undo = Undo { adb: self.adb.clone(), serial: serial.to_string(), port, server: None };
        match self.adb.shell(serial, &scrcpy::server_command(&self.options)) {
            Ok(c) => undo.server = Some(c),
            Err(e) => {
                undo.stop();
                return Err(format!("scrcpy-server didn't start: {e}"));
            }
        }
        // adb accepts the connection before anything listens behind it:
        // the server's dummy byte says it is there (scrcpy's own retry:
        // every 100 ms).
        let started = std::time::Instant::now();
        let (video_in, _video_out) = loop {
            match self.adb.connect(port) {
                Ok((mut r, w)) => {
                    let mut b = [0u8; 1];
                    if r.read_exact(&mut b).is_ok() {
                        break (r, w);
                    }
                }
                Err(_) if started.elapsed() < self.patience => {}
                Err(e) => {
                    undo.stop();
                    return Err(format!("scrcpy-server didn't answer: {e}"));
                }
            }
            if started.elapsed() >= self.patience {
                undo.stop();
                return Err("scrcpy-server didn't answer".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let input = if self.options.control {
            match self.adb.connect(port) {
                Ok((r, w)) => {
                    // Messages from the device (its clipboard) are read and
                    // dropped, so the socket never fills.
                    std::thread::spawn(move || drain(r));
                    Ok(Box::new(ControlSocket(w)) as Box<dyn InputSink>)
                }
                Err(e) => Err(format!("no control connection: {e}")),
            }
        } else {
            Err("watch only".into())
        };
        Ok(Connection { video: video_in, skip: 0, input, stop: Box::new(undo) })
    }
}

fn drain(mut r: Box<dyn Read + Send>) {
    let mut buf = [0u8; 4096];
    while r.read(&mut buf).is_ok_and(|n| n > 0) {}
}

/// adb on this computer: `ANDROID_HOME`/`ANDROID_SDK_ROOT`, the SDK's
/// usual place, or `PATH`.
pub fn find_adb() -> Option<PathBuf> {
    let name = format!("adb{}", std::env::consts::EXE_SUFFIX);
    let mut dirs: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) {
        dirs.push(home.join("Library/Android/sdk"));
        dirs.push(home.join("Android/Sdk"));
        dirs.push(home.join("AppData/Local/Android/Sdk"));
    }
    dirs.iter()
        .map(|d| d.join("platform-tools").join(&name))
        .chain(std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).map(|d| d.join(&name)).collect::<Vec<_>>()))
        .find(|p| p.is_file())
}

/// adb as a program on this computer.
pub struct LocalAdb {
    pub path: PathBuf,
}

struct Process(std::process::Child);

impl Child for Process {
    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl LocalAdb {
    pub fn new() -> Result<LocalAdb, String> {
        find_adb().map(|path| LocalAdb { path }).ok_or_else(|| "adb isn't installed (Android SDK platform-tools)".into())
    }

    fn command(&self, serial: Option<&str>) -> std::process::Command {
        let mut c = std::process::Command::new(&self.path);
        if let Some(s) = serial {
            c.args(["-s", s]);
        }
        c.stdin(std::process::Stdio::null());
        c
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Adb for LocalAdb {
    fn run(&self, serial: Option<&str>, args: &[&str]) -> Result<String, String> {
        let out = self.command(serial).args(args).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    fn shell(&self, serial: &str, command: &str) -> Result<Box<dyn Child>, String> {
        let child = self
            .command(Some(serial))
            .args(["shell", command])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(Box::new(Process(child)))
    }

    fn free_port(&self) -> Result<u16, String> {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
    }

    fn connect(&self, port: u16) -> std::io::Result<Socket> {
        let s = std::net::TcpStream::connect(("127.0.0.1", port))?;
        s.set_nodelay(true)?;
        let w = s.try_clone()?;
        Ok((Box::new(s), Box::new(w)))
    }
}
