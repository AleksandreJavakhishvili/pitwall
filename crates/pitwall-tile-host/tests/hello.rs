//! The hello tile (tests/fixtures/hello-tile, committed built as
//! tests/fixtures/hello-tile.wasm) in the spike's host.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use pitwall_tile_host::{
    diff, engine, Configure, Element, EpochTicker, Event, FromWorker, Runtime, Worker,
    HANDLE_DEADLINE, RENDER_DEADLINE,
};

const WASM: &[u8] = include_bytes!("fixtures/hello-tile.wasm");

fn runtime() -> Runtime {
    Runtime::new(engine(None).unwrap()).unwrap()
}

fn text(frame: &pitwall_tile_host::Frame, key: &str) -> String {
    let node = frame.nodes.iter().find(|n| n.key == key).expect(key);
    match &node.element {
        Element::Text(t) => t.clone(),
        other => panic!("{key}: {other:?}"),
    }
}

#[test]
fn renders_and_handles_a_click() {
    let rt = runtime();
    let component = rt.compile(WASM).unwrap();
    let mut tile = rt.instantiate(&component).unwrap();
    let frame = tile.render(RENDER_DEADLINE).unwrap();
    assert_eq!(frame.nodes.len(), 4);
    assert_eq!(text(&frame, "count"), "Clicked 0 times");
    assert!(tile.on_event(Event::Click(1), HANDLE_DEADLINE).unwrap());
    let next = tile.render(RENDER_DEADLINE).unwrap();
    assert_eq!(text(&next, "count"), "Clicked 1 times");

    let (pa, pb) = (diff::paths(&frame).unwrap(), diff::paths(&next).unwrap());
    let d = diff::diff(&frame, &pa, &next, &pb).unwrap();
    assert_eq!((d.added, d.removed, d.changed), (0, 0, 1));

    // An unknown handler is logged through the host's `log` import.
    assert!(!tile.on_event(Event::Click(99), HANDLE_DEADLINE).unwrap());
    let logs = tile.take_logs();
    assert!(logs.iter().any(|(_, l)| l.contains("unknown handler 99")), "{logs:?}");
}

#[test]
fn a_synthetic_tree_has_the_size_asked_for() {
    let rt = runtime();
    let mut tile = rt.instantiate(&rt.compile(WASM).unwrap()).unwrap();
    tile.on_event(Event::Configure(Configure { nodes: 2000, cached: false }), HANDLE_DEADLINE).unwrap();
    let frame = tile.render(RENDER_DEADLINE).unwrap();
    assert_eq!(frame.nodes.len(), 2000);
    diff::paths(&frame).unwrap();
}

#[test]
fn pitwall_cache_round_trips() {
    let rt = runtime();
    let dir = std::env::temp_dir().join(format!("pitwall-tile-host-test-{}", std::process::id()));
    let (_, hit) = rt.load_or_compile(WASM, &dir).unwrap();
    assert!(!hit);
    let (component, hit) = rt.load_or_compile(WASM, &dir).unwrap();
    assert!(hit);
    rt.instantiate(&component).unwrap().render(RENDER_DEADLINE).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_looping_tile_is_stopped_at_its_deadline() {
    let rt = runtime();
    let _ticker = EpochTicker::start(rt.engine()).unwrap();
    let mut tile = rt.instantiate(&rt.compile(WASM).unwrap()).unwrap();
    let t = Instant::now();
    let err = tile.on_event(Event::Spin, Duration::from_millis(50)).unwrap_err();
    let took = t.elapsed();
    assert_eq!(err.downcast_ref::<wasmtime::Trap>(), Some(&wasmtime::Trap::Interrupt), "{err:?}");
    assert!(took >= Duration::from_millis(40) && took < Duration::from_secs(1), "{took:?}");
}

#[test]
fn the_worker_owns_the_store_and_reports_a_hang() {
    let rt = runtime();
    let _ticker = EpochTicker::start(rt.engine()).unwrap();
    let component = rt.compile(WASM).unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = Worker::spawn("hello", rt, component, move |m| {
        let thread = std::thread::current().name().unwrap_or_default().to_string();
        let _ = tx.send((thread, m));
    })
    .unwrap();
    let next = || loop {
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            (_, FromWorker::Log(..)) => continue,
            other => return other,
        }
    };
    let (thread, first) = next();
    assert_eq!(thread, "tile:hello");
    assert!(matches!(first, FromWorker::Frame { .. }), "{first:?}");
    assert!(worker.send(Event::Click(1)));
    let (_, FromWorker::Frame { frame, .. }) = next() else { panic!() };
    assert_eq!(text(&frame, "count"), "Clicked 1 times");
    assert!(worker.send(Event::Spin));
    let (_, stopped) = next();
    assert!(matches!(&stopped, FromWorker::Stopped { reason } if reason.contains("in time")), "{stopped:?}");
}
