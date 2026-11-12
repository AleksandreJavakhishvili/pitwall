//! What Pitwall sees on an agent's screen: feed a recording of a terminal's
//! raw output to the headless screen, then print its text and what the
//! rules of an agent kind make of it (state, detail, answer keys, excerpt).
//! For capturing rule fixtures and checking a new version of an agent.
//!
//!   cargo run -p pitwall-detect --example screen_text -- claude out.bin [rows cols]
//!
//! Prints `#title: <title>` first, like the fixtures under
//! `src/detect/fixtures/`; the verdict goes to stderr.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(kind), Some(file)) = (args.first(), args.get(1)) else {
        eprintln!("usage: screen_text <kind> <recording> [rows cols]");
        std::process::exit(2);
    };
    let rows = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);
    let cols = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(110);
    let bytes = std::fs::read(file).expect("read the recording");
    let mut screen = pitwall_detect::Screen::new(rows, cols);
    screen.feed(&bytes);
    let (text, title) = (screen.text(), screen.title());
    println!("#title: {}", title.as_deref().unwrap_or(""));
    println!("{text}");
    let d = pitwall_detect::detect(kind, &text, title.as_deref());
    eprintln!("state: {:?}", d.state);
    eprintln!("detail: {:?}", d.detail);
    eprintln!("answer: {:?}", d.answer);
    eprintln!("excerpt:\n{}", d.excerpt.unwrap_or_default());
}
