//! Print the `ScreenFrame` (JSON) the engine would send for a recording, so
//! it can be compared with what xterm.js shows for the same bytes
//! (docs/spec/perf.md, "Wall tiles from the screen copy").
//!
//!   cargo run -p pitwall-core --example screen_frame -- <bytes-file> <cols> <rows>

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: screen_frame <bytes-file> <cols> <rows>");
    let cols: u16 = args.next().and_then(|v| v.parse().ok()).unwrap_or(100);
    let rows: u16 = args.next().and_then(|v| v.parse().ok()).unwrap_or(30);
    let bytes = std::fs::read(path).expect("read the recording");
    let mut screen = pitwall_detect::Screen::new(rows, cols);
    screen.feed(&bytes);
    let frame = pitwall_core::term::full_frame(&screen.snapshot());
    println!("{}", serde_json::to_string(&frame).unwrap());
}
