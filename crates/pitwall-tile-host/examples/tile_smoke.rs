//! A tile started as the app starts one, for CI to run under macOS's
//! hardened runtime with the app's entitlements
//! (`scripts/check-hardened-tiles.sh`): Wasmtime compiles the tile's
//! component to machine code in memory and makes it executable, which the
//! hardened runtime kills ("Code Signature Invalid") unless the app is
//! signed with `com.apple.security.cs.allow-unsigned-executable-memory`.
//! A kill there took the whole app down on every launch, so this runs the
//! same steps in a binary signed the same way.
//!
//! It starts the tile in folder `DIR` (default: the hello fixture) twice,
//! compiled the first time and from the compile cache the second, and
//! waits each time for its first frame. Exit 0: it drew both times.
//!
//!     cargo run -p pitwall-tile-host --example tile_smoke [DIR]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pitwall_tile_host::{HostConfig, StartOptions, TileHost, TileOutput};

const WAIT: Duration = Duration::from_secs(120);

fn main() -> ExitCode {
    let dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello"));
    let data = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => return fail(&format!("no temporary folder: {e}")),
    };
    let host = match TileHost::new(HostConfig::new(data.path())) {
        Ok(h) => h,
        Err(e) => return fail(&format!("the engine didn't start: {e}")),
    };
    for (round, what) in ["compiled", "from the cache"].iter().enumerate() {
        let ext = match host.load(&dir) {
            Ok(e) => e,
            Err(e) => return fail(&format!("{} didn't load: {e}", dir.display())),
        };
        let Some(tile) = ext.manifest().contributes.tiles.first().map(|t| t.id.clone()) else {
            return fail(&format!("{} has no tile", dir.display()));
        };
        let (tx, rx) = mpsc::channel();
        let handle = match host.start(&ext, StartOptions::view(&tile), move |o| {
            let _ = tx.send(o);
        }) {
            Ok(h) => h,
            Err(e) => return fail(&format!("it didn't start: {e}")),
        };
        let until = Instant::now() + WAIT;
        let mut cached = None;
        loop {
            match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok(TileOutput::Started { cached: c, .. }) => cached = Some(c),
                Ok(TileOutput::Frame(_)) => break,
                Ok(TileOutput::Stopped(why)) => return fail(&format!("it stopped: {why}")),
                Ok(_) => {}
                Err(_) => return fail(&format!("no frame within {WAIT:?}")),
            }
        }
        if round == 1 && cached == Some(false) {
            return fail("the second start compiled again instead of using the cache");
        }
        println!("tile smoke: {} drew ({what})", ext.manifest().id);
        drop(handle);
    }
    println!("tile smoke: ok");
    ExitCode::SUCCESS
}

fn fail(why: &str) -> ExitCode {
    eprintln!("tile smoke: {why}");
    ExitCode::FAILURE
}
