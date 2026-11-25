//! `pitwall-simhost`: an iOS Simulator's screen and input for Pitwall's
//! device tiles (docs/spec/device.md).
//!
//! ```text
//! pitwall-simhost stream <udid> [--set <device set folder>] [--fps <n>]
//! pitwall-simhost probe
//! ```
//!
//! `stream` writes the simulator's main screen to stdout as H.264 in
//! scrcpy's framing (`pitwall_proto::device::video`), reads input records
//! from stdin (`pitwall_proto::device::sim`) and says how it is going on
//! stderr, one JSON line each. It ends when stdin closes or the simulator
//! shuts down. `probe` says which Xcode it would use and whether input is
//! possible with it.
//!
//! Its own process because it loads Xcode's private frameworks: neither
//! the app nor pitwall-core links them, and if they misbehave (an Xcode
//! that changed them) only this helper ends; the tile says so.

#[cfg(target_os = "macos")]
mod mac;

fn usage() -> i32 {
    eprintln!("usage: pitwall-simhost stream <udid> [--set <folder>] [--fps <n>] [--raw] [--seconds <n>] [--input <file>]\n       pitwall-simhost probe");
    2
}

#[cfg(target_os = "macos")]
fn run(args: Vec<String>) -> i32 {
    let mut it = args.into_iter();
    match it.next().as_deref() {
        Some("probe") => mac::probe(),
        Some("stream") => {
            let Some(udid) = it.next() else { return usage() };
            let mut o = mac::Options { udid, set: None, fps: 30, raw: false, seconds: None, input: None };
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--set" => o.set = it.next().map(Into::into),
                    "--fps" => o.fps = it.next().and_then(|n| n.parse().ok()).unwrap_or(30).clamp(1, 60),
                    "--raw" => o.raw = true,
                    "--seconds" => o.seconds = it.next().and_then(|n| n.parse().ok()),
                    "--input" => o.input = it.next().map(Into::into),
                    _ => return usage(),
                }
            }
            mac::stream(o)
        }
        _ => usage(),
    }
}

#[cfg(not(target_os = "macos"))]
fn run(_args: Vec<String>) -> i32 {
    eprintln!("pitwall-simhost: iOS Simulators run on macOS only");
    let _ = usage;
    2
}

fn main() {
    std::process::exit(run(std::env::args().skip(1).collect()));
}
