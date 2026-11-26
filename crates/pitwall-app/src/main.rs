// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Tile SDK phase 0 spike: with `--features tile-spike` the whole tile
    // runtime is reachable, so a release build shows what Wasmtime adds
    // (docs/spec/tile-sdk/spike-results.md). Not a user feature.
    #[cfg(feature = "tile-spike")]
    if let Some(wasm) = std::env::var_os("PITWALL_TILE_SPIKE") {
        std::process::exit(pitwall_tile_host::smoke(std::path::Path::new(&wasm)));
    }
    pitwall_app::run();
}
