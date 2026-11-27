//! The components end to end with the hello extension's `gallery` tile:
//! a 100 000-row list's frame stays small and its rows come a page at a
//! time (the numbers runtime.md §2.7 records), rows follow the list's
//! revision, asks past the end or for no list are ignored, and the image
//! the tile put reaches the app before its first frame.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use pitwall_tile_host::components::{self, ImageFormat};
use pitwall_tile_host::limits::{MAX_FRAME_BYTES, ROWS_PAGE};
use pitwall_tile_host::ui::Element;
use pitwall_tile_host::{FrameUpdate, HostConfig, RowsUpdate, StartOptions, TileHandle, TileHost, TileOutput, UiRequest};

const WAIT: Duration = Duration::from_secs(20);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello")
}

fn start(data: &Path) -> (TileHost, TileHandle, Receiver<TileOutput>) {
    let host = TileHost::new(HostConfig::new(data)).expect("the engine starts");
    let ext = host.load(&fixture()).expect("the fixture loads");
    let (tx, rx) = mpsc::channel();
    let handle = host
        .start(&ext, StartOptions::view("gallery"), move |o| {
            let _ = tx.send(o);
        })
        .expect("starts");
    (host, handle, rx)
}

fn next<T>(rx: &Receiver<TileOutput>, what: &str, mut pick: impl FnMut(TileOutput) -> Option<T>) -> T {
    let until = Instant::now() + WAIT;
    loop {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(TileOutput::Stopped(r)) => panic!("stopped while waiting for {what}: {r}"),
            Ok(o) => {
                if let Some(t) = pick(o) {
                    return t;
                }
            }
            Err(_) => panic!("no {what} within {WAIT:?}"),
        }
    }
}

fn frame(rx: &Receiver<TileOutput>) -> FrameUpdate {
    next(rx, "a frame", |o| match o {
        TileOutput::Frame(f) => Some(f),
        _ => None,
    })
}

fn rows(rx: &Receiver<TileOutput>) -> RowsUpdate {
    next(rx, "rows", |o| match o {
        TileOutput::Rows(r) => Some(r),
        _ => None,
    })
}

#[test]
fn a_hundred_thousand_rows_come_a_page_at_a_time() {
    let t = tempfile::tempdir().unwrap();
    let (_host, handle, rx) = start(t.path());
    // The image it put in `new`, before the frame that shows it.
    let mut put = None;
    let f = next(&rx, "the first frame", |o| match o {
        TileOutput::Ui(UiRequest::PutImage { name, format, bytes }) => {
            put = Some((name, format, bytes.len()));
            None
        }
        TileOutput::Frame(f) => Some(f),
        _ => None,
    });
    let (name, format, _) = put.expect("put-image came first");
    assert_eq!((name.as_str(), format), ("dot", ImageFormat::Png));

    let paths = components::key_paths(&f.frame.nodes);
    let list = paths.iter().position(|p| p.ends_with("rows")).map(|i| paths[i].clone()).expect("the list");
    let l = components::list_at(&f.frame, &list).unwrap();
    assert_eq!(l.count, 100_000);
    // The frame names the count; no row is in it.
    let bytes = pitwall_tile_host::frame::frame_bytes(&f.frame);
    let nodes = f.frame.nodes.len();
    assert!(bytes < MAX_FRAME_BYTES / 20, "{bytes} bytes");
    assert!(!f.frame.nodes.iter().any(|n| matches!(&n.element, Element::Text(t) if t.starts_with("Row "))), "no rows in the frame");

    // Pages anywhere in the list, each checked.
    let mut took = Vec::new();
    let mut row_bytes = 0;
    for start in [0, 50_000, 99_936] {
        assert!(handle.rows("gallery", &list, start, ROWS_PAGE));
        let r = rows(&rx);
        assert_eq!((r.start, r.rows.len(), r.revision), (start, ROWS_PAGE as usize, l.revision));
        assert_eq!(r.rows[0].key, format!("r{start}"));
        row_bytes = r.rows.iter().map(|row| row.nodes.len()).sum::<usize>();
        took.push(r.took);
    }
    // Past the end: only what is there; nothing for no list.
    assert!(handle.rows("gallery", &list, 99_990, ROWS_PAGE));
    assert_eq!(rows(&rx).rows.len(), 10);
    assert!(handle.rows("gallery", "no/such/list", 0, ROWS_PAGE));
    assert!(handle.rows("gallery", &list, 0, 1));
    assert_eq!(rows(&rx).start, 0, "the ask for no list was skipped");
    println!("[components] 100 000-row list: frame of {nodes} nodes, ~{bytes} bytes; a page of {ROWS_PAGE} rows ({row_bytes} nodes) took {took:?} (debug build)");
}

#[test]
fn rows_follow_the_lists_revision() {
    let t = tempfile::tempdir().unwrap();
    let (_host, handle, rx) = start(t.path());
    let f = frame(&rx);
    let paths = components::key_paths(&f.frame.nodes);
    let tree = paths.iter().find(|p| p.ends_with("tree")).cloned().unwrap();
    let rev = components::list_at(&f.frame, &tree).unwrap().revision;
    assert!(handle.rows("gallery", &tree, 0, 64));
    let r = rows(&rx);
    assert_eq!((r.revision, r.rows.len()), (rev, 5));
    assert_eq!(r.rows.iter().map(|r| (r.depth, r.expanded)).collect::<Vec<_>>(), [(0, Some(true)), (1, None), (1, Some(false)), (0, Some(false)), (0, None)]);
    components::check_rows(&r.rows).unwrap();
}
