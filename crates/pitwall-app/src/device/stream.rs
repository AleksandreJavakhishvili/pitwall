//! One device's stream, from start to stop.
//!
//! A [`Backend`] connects to the device (Android: adb and scrcpy-server;
//! iOS: `pitwall-simhost`) and hands back a [`Connection`]: the video
//! bytes, where input goes, and how to undo everything it set up. A
//! [`Session`] reads the video on a thread of its own, decodes it and
//! reports [`Event`]s; dropping the session (the tile closed) or the video
//! ending (the device went away) runs the undo exactly once.

use std::io::Read;
use std::sync::{Arc, Mutex};

use pitwall_proto::device::video::{Event as Video, Reader};

use super::Device;

/// A touch's phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

/// A key the tile sends (what each device calls it is its backend's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Space,
}

/// Input for the device.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// One finger at a ratio of the frame (0..1 from its top left).
    Touch { phase: Phase, at: (f64, f64) },
    Key { down: bool, key: Key },
    /// Typed text.
    Text(String),
    /// Android's Back (iOS has none).
    Back,
    /// The Home button / gesture.
    Home,
}

/// Where input goes. `frame` is the current video size (scrcpy needs it).
pub trait InputSink: Send {
    fn send(&mut self, input: &Input, frame: (u32, u32)) -> std::io::Result<()>;
}

/// Undoes what a backend set up (a port forward, a process). Called once.
pub trait Stop: Send {
    fn stop(&mut self);
}

/// A connected device.
pub struct Connection {
    pub video: Box<dyn Read + Send>,
    /// Bytes before the stream proper (scrcpy's device name, if asked).
    pub skip: usize,
    /// `None`: watch only, with why.
    pub input: Result<Box<dyn InputSink>, String>,
    pub stop: Box<dyn Stop>,
}

/// Connects to a device. Runs on the session's thread (it may wait).
pub trait Backend: Send + Sync {
    fn connect(&self, device: &Device) -> Result<Connection, String>;
}

/// Decodes the stream (VideoToolbox in the app; a fake in tests).
pub trait Decode: Send + 'static {
    type Picture: Send + 'static;
    fn config(&mut self, data: &[u8]) -> Result<(), String>;
    fn decode(&mut self, data: &[u8], pts: u64) -> Result<Option<Self::Picture>, String>;
}

impl Decode for crate::platform::video::Decoder {
    type Picture = crate::platform::video::Frame;
    fn config(&mut self, data: &[u8]) -> Result<(), String> {
        crate::platform::video::Decoder::config(self, data)
    }
    fn decode(&mut self, data: &[u8], pts: u64) -> Result<Option<Self::Picture>, String> {
        crate::platform::video::Decoder::decode(self, data, pts)
    }
}

/// What a session reports, in order: `Connected`, then sizes and
/// pictures, then `Ended` once.
pub enum Event<P> {
    /// Streaming; whether input reaches the device (and why not).
    Connected { input: Result<(), String> },
    /// The frames from now on are this size (a rotation changes it).
    Size(u32, u32),
    Picture(P),
    /// It stopped: why, in the user's words. Not sent after the session
    /// was dropped.
    Ended(String),
}

type Shared = Arc<Mutex<Option<Box<dyn Stop>>>>;

fn stop_once(s: &Shared) {
    let taken = s.lock().ok().and_then(|mut g| g.take());
    if let Some(mut stop) = taken {
        stop.stop();
    }
}

/// A running stream. Dropping it stops it.
pub struct Session {
    stop: Shared,
    input: Arc<Mutex<Option<Box<dyn InputSink>>>>,
    frame: Arc<Mutex<(u32, u32)>>,
    /// Set when dropped: the thread reports nothing more.
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl Session {
    /// Starts streaming `device` with `backend`; `events` is called on the
    /// session's thread.
    pub fn start<D: Decode>(backend: Arc<dyn Backend>, device: Device, mut decoder: D, events: impl Fn(Event<D::Picture>) + Send + 'static) -> Session {
        let stop: Shared = Arc::new(Mutex::new(None));
        let input: Arc<Mutex<Option<Box<dyn InputSink>>>> = Arc::new(Mutex::new(None));
        let frame = Arc::new(Mutex::new((0, 0)));
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s = Session { stop: stop.clone(), input: input.clone(), frame: frame.clone(), closed: closed.clone() };
        let live = move || !closed.load(std::sync::atomic::Ordering::SeqCst);
        std::thread::Builder::new()
            .name("device-stream".into())
            .spawn(move || {
                let c = match backend.connect(&device) {
                    Ok(c) => c,
                    Err(why) => {
                        if live() {
                            events(Event::Ended(why));
                        }
                        return;
                    }
                };
                *stop.lock().unwrap() = Some(c.stop);
                if !live() {
                    // Closed while connecting.
                    stop_once(&stop);
                    return;
                }
                let can = c.input.as_ref().map(|_| ()).map_err(|e| e.clone());
                if let Ok(sink) = c.input {
                    *input.lock().unwrap() = Some(sink);
                }
                events(Event::Connected { input: can });
                let why = pump(c.video, c.skip, &mut decoder, &frame, &live, &events);
                stop_once(&stop);
                if live() {
                    events(Event::Ended(why));
                }
            })
            .ok();
        s
    }

    /// Sends input (dropped when the device takes none, or before the
    /// first frame).
    pub fn send(&self, i: &Input) -> Result<(), String> {
        let frame = *self.frame.lock().unwrap();
        let mut g = self.input.lock().unwrap();
        let Some(sink) = g.as_mut() else { return Ok(()) };
        sink.send(i, frame).map_err(|e| format!("the device didn't take that: {e}"))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        *self.input.lock().unwrap() = None;
        stop_once(&self.stop);
    }
}

/// Reads and decodes until the stream ends; why it ended.
fn pump<D: Decode>(mut video: Box<dyn Read + Send>, skip: usize, decoder: &mut D, frame: &Mutex<(u32, u32)>, live: &dyn Fn() -> bool, events: &dyn Fn(Event<D::Picture>)) -> String {
    let mut reader = Reader::skipping(skip);
    let mut buf = vec![0u8; 256 * 1024];
    let mut configured = false;
    loop {
        let n = match video.read(&mut buf) {
            Ok(0) => return "The device went away".into(),
            Ok(n) => n,
            Err(e) => return format!("The device's stream broke: {e}"),
        };
        if !live() {
            return "closed".into();
        }
        let evs = match reader.push(&buf[..n]) {
            Ok(e) => e,
            Err(e) => return e.to_string(),
        };
        for ev in evs {
            match ev {
                Video::Session { width, height } => {
                    *frame.lock().unwrap() = (width, height);
                    events(Event::Size(width, height));
                }
                Video::Packet(p) if p.config => match decoder.config(&p.data) {
                    Ok(()) => configured = true,
                    Err(e) => return e,
                },
                Video::Packet(p) => {
                    if !configured {
                        continue;
                    }
                    match decoder.decode(&p.data, p.pts) {
                        Ok(Some(pic)) => events(Event::Picture(pic)),
                        Ok(None) => {}
                        // One bad frame: wait for the next key frame.
                        Err(_) if !p.key => {}
                        Err(e) => return e,
                    }
                }
            }
        }
    }
}
