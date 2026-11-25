//! The helper on macOS: a booted simulator's main screen, polled from its
//! framebuffer IOSurface and encoded to H.264 on stdout; input records
//! from stdin sent to it through SimulatorKit's HID client.

mod encode;
mod ffi;
mod sim;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pitwall_proto::device::sim::{Input, Phase, Status, RECORD};
use pitwall_proto::device::video::{codec, session, Packet, CODEC_H264};

use encode::{Encoder, Sink};

pub struct Options {
    pub udid: String,
    pub set: Option<PathBuf>,
    pub fps: u32,
    /// Plain Annex B on stdout (no framing), for `ffmpeg -f h264`.
    pub raw: bool,
    /// Stop after this long (tests).
    pub seconds: Option<u64>,
    /// Input records from this file instead of stdin (tests).
    pub input: Option<PathBuf>,
}

fn status(s: &Status) {
    eprintln!("{}", s.line());
}

/// Stops with a reason (on stderr) and exits.
fn stop(why: impl Into<String>) -> ! {
    status(&Status::Stopped { why: why.into() });
    std::process::exit(1);
}

struct Stdout {
    raw: bool,
    /// The codec id and a session packet, written before the next packet
    /// (a session starts with the encoder's first output).
    header: Vec<u8>,
}

impl Sink for Stdout {
    fn packet(&mut self, p: Packet) {
        let mut out = std::io::stdout().lock();
        let bytes = if self.raw { p.data } else { p.encode() };
        let header = std::mem::take(&mut self.header);
        let ok = self.raw || out.write_all(&header).is_ok();
        if !ok || out.write_all(&bytes).and_then(|_| out.flush()).is_err() {
            // The app went away: nothing left to show to.
            std::process::exit(0);
        }
    }
}

pub fn probe() -> i32 {
    match sim::Frameworks::load() {
        Ok(fw) => {
            println!("xcode {} at {}", fw.xcode, fw.dev.display());
            match &fw.kit_error {
                None => println!("SimulatorKit: loaded"),
                Some(e) => println!("SimulatorKit: {e}"),
            }
            0
        }
        Err(e) => {
            println!("{e}");
            1
        }
    }
}

pub fn stream(o: Options) -> i32 {
    let fw = sim::Frameworks::load().unwrap_or_else(|e| stop(e));
    let device = sim::Device::find(&fw, &o.udid, o.set.as_deref()).unwrap_or_else(|e| stop(e));
    if !device.booted() {
        stop(format!("{} isn't booted", device.name));
    }
    let screen = device.screen().unwrap_or_else(|e| stop(e));
    let (hid, why) = match sim::Hid::new(&fw, &device) {
        Ok(h) => (Some(h), None),
        Err(e) => (None, Some(e)),
    };
    let input = hid.is_some();
    let sink = Arc::new(Mutex::new(Stdout { raw: o.raw, header: Vec::new() }));
    let started = Instant::now();

    // Input, on a thread of its own: records until stdin closes, then the
    // helper ends (the tile was closed).
    if let Some(hid) = hid {
        let file = o.input.clone();
        std::thread::spawn(move || {
            let mut src: Box<dyn Read> = match file {
                Some(f) => match std::fs::File::open(f) {
                    Ok(f) => Box::new(f),
                    Err(_) => return,
                },
                None => Box::new(std::io::stdin()),
            };
            let mut rec = [0u8; RECORD];
            while src.read_exact(&mut rec).is_ok() {
                match Input::decode(&rec) {
                    Some(Input::Touch { phase, x, y }) => hid.touch(phase != Phase::Up, x, y),
                    Some(Input::Key { down, usage }) => hid.key(down, usage),
                    Some(Input::Button { button, down }) => hid.button(sim::button_source(button), down),
                    None => {}
                }
            }
            if o.input.is_none() {
                std::process::exit(0);
            }
        });
    } else if o.input.is_none() {
        std::thread::spawn(|| {
            let mut sink = [0u8; 256];
            while std::io::stdin().read(&mut sink).is_ok_and(|n| n > 0) {}
            std::process::exit(0);
        });
    }

    let tick = Duration::from_micros(1_000_000 / o.fps.max(1) as u64);
    let mut encoder: Option<Encoder> = None;
    let mut seed = None;
    let mut last_check = Instant::now();
    let mut last_frame = Instant::now();
    loop {
        if o.seconds.is_some_and(|s| started.elapsed() >= Duration::from_secs(s)) {
            drop(encoder);
            return 0;
        }
        if last_check.elapsed() >= Duration::from_secs(1) {
            last_check = Instant::now();
            if !device.booted() {
                drop(encoder);
                stop(format!("{} shut down", device.name));
            }
        }
        objc2::rc::autoreleasepool(|_| {
            let Some(surface) = screen.surface() else { return };
            let (w, h) = unsafe { (ffi::IOSurfaceGetWidth(surface) as u32, ffi::IOSurfaceGetHeight(surface) as u32) };
            if encoder.as_ref().is_none_or(|e| (e.width, e.height) != (w, h)) {
                let first = encoder.is_none();
                encoder = None;
                {
                    let mut s = sink.lock().unwrap();
                    if first {
                        s.header.extend(codec(CODEC_H264));
                        status(&Status::Ready { width: w, height: h, input, why: why.clone(), xcode: fw.xcode.clone() });
                    }
                    s.header.extend(session(w, h));
                }
                match Encoder::new(w, h, o.fps, sink.clone()) {
                    Ok(e) => encoder = Some(e),
                    Err(e) => stop(e),
                }
                seed = None;
            }
            let s = unsafe { ffi::IOSurfaceGetSeed(surface) };
            // A frame when the screen changed, and one a second anyway
            // (the seed isn't promised to move for every GPU write).
            if seed != Some(s) || last_frame.elapsed() >= Duration::from_secs(1) {
                seed = Some(s);
                last_frame = Instant::now();
                if let Some(e) = encoder.as_mut() {
                    if let Err(e) = e.encode(surface, started.elapsed().as_micros() as u64) {
                        eprintln!("{e}");
                    }
                }
            }
        });
        std::thread::sleep(tick);
    }
}
