//! Phase 0 measurements (docs/spec/tile-sdk/spike-results.md), no GPUI and
//! no window. Run in release:
//!
//!   cargo run --release -p pitwall-tile-host --example tile_bench [-- <tile.wasm>]
//!
//! Without an argument it uses the committed hello tile. Prints Markdown.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pitwall_tile_host::{
    diff, engine, engine_with, Configure, EpochTicker, Event, FromWorker, Runtime, Worker, HANDLE_DEADLINE,
    RENDER_DEADLINE,
};

const HELLO: &[u8] = include_bytes!("../tests/fixtures/hello-tile.wasm");
const NODES: u32 = 2000;
const FRAMES: usize = 300;

type R<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> R<()> {
    let wasm = match std::env::args_os().nth(1) {
        Some(p) => std::fs::read(p)?,
        None => HELLO.to_vec(),
    };
    let scratch = std::env::temp_dir().join(format!("pitwall-tile-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    println!("## tile_bench ({} {}, {} KiB component)\n", std::env::consts::OS, std::env::consts::ARCH, wasm.len() / 1024);
    println!("| What | median | p95 | min | max | n |\n|---|---|---|---|---|---|");

    // Engine creation (once per app).
    let mut s = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let e = engine(None)?;
        s.push(t.elapsed());
        drop(e);
    }
    row("engine: Engine::new", &s);

    // Cold: a fresh engine each time (no in-memory state), empty caches.
    let (mut compile, mut inst, mut first) = (Vec::new(), Vec::new(), Vec::new());
    let mut total = Vec::new();
    for i in 0..10 {
        let rt = Runtime::new(engine(Some(&scratch.join(format!("cold-wt-{i}"))))?)?;
        let t0 = Instant::now();
        let (component, hit) = rt.load_or_compile(&wasm, &scratch.join(format!("cold-pw-{i}")))?;
        assert!(!hit);
        let t1 = Instant::now();
        let mut tile = rt.instantiate(&component)?;
        let t2 = Instant::now();
        tile.render(RENDER_DEADLINE)?;
        let t3 = Instant::now();
        compile.push(t1 - t0);
        inst.push(t2 - t1);
        first.push(t3 - t2);
        total.push(t3 - t0);
    }
    row("cold: compile (Cranelift) + write cache", &compile);
    row("cold: instantiate", &inst);
    row("cold: first render", &first);
    row("**cold start total** (compile + instantiate + first render)", &total);

    // Warm, Pitwall's own cache (runtime.md §2.6): deserialize_file.
    let rt = Runtime::new(engine(None)?)?;
    let pw = scratch.join("warm-pw");
    rt.load_or_compile(&wasm, &pw)?;
    let (mut load, mut total) = (Vec::new(), Vec::new());
    for _ in 0..20 {
        let rt = Runtime::new(engine(None)?)?;
        let t0 = Instant::now();
        let (component, hit) = rt.load_or_compile(&wasm, &pw)?;
        assert!(hit);
        let t1 = Instant::now();
        rt.instantiate(&component)?.render(RENDER_DEADLINE)?;
        load.push(t1 - t0);
        total.push(t1.elapsed() + (t1 - t0));
    }
    row("warm (Pitwall cache): deserialize_file", &load);
    row("**warm start total** (Pitwall cache: load + instantiate + first render)", &total);

    // Warm, Wasmtime's Config::cache (`--features wasmtime-cache`).
    if cfg!(feature = "wasmtime-cache") {
        let wt = scratch.join("warm-wt");
        Runtime::new(engine(Some(&wt))?)?.compile(&wasm)?;
        let mut total = Vec::new();
        for _ in 0..20 {
            let rt = Runtime::new(engine(Some(&wt))?)?;
            let t0 = Instant::now();
            let component = rt.compile(&wasm)?;
            rt.instantiate(&component)?.render(RENDER_DEADLINE)?;
            total.push(t0.elapsed());
        }
        row("warm (Wasmtime cache hit) total", &total);
    }

    // A 2 000-node frame: render (guest builds + canonical ABI lower/lift),
    // the same with a frame the guest keeps (mostly ABI), and the host diff.
    let rt = Runtime::new(engine(None)?)?;
    let component = rt.compile(&wasm)?;
    for cached in [false, true] {
        let mut tile = rt.instantiate(&component)?;
        tile.on_event(Event::Configure(Configure { nodes: NODES, cached }), HANDLE_DEADLINE)?;
        let mut prev = tile.render(RENDER_DEADLINE)?;
        let mut prev_paths = diff::paths(&prev)?;
        let (mut render, mut dif, mut frame) = (Vec::new(), Vec::new(), Vec::new());
        let mut changed = 0;
        for _ in 0..FRAMES {
            let t0 = Instant::now();
            tile.on_event(Event::Click(1), HANDLE_DEADLINE)?;
            let next = tile.render(RENDER_DEADLINE)?;
            let t1 = Instant::now();
            let paths = diff::paths(&next)?;
            let d = diff::diff(&prev, &prev_paths, &next, &paths)?;
            let t2 = Instant::now();
            changed = d.changed;
            // Dropping the old frame is part of the host's cost.
            prev = next;
            prev_paths = paths;
            render.push(t1 - t0);
            dif.push(t2 - t1);
            frame.push(t2 - t0);
        }
        let what = if cached { "guest-cached frame" } else { "guest builds it" };
        row(&format!("{NODES} nodes, {what}: event + render (canonical ABI)"), &render);
        row(&format!("{NODES} nodes, {what}: host validate + keyed diff ({changed} changed)"), &dif);
        row(&format!("**{NODES}-node frame total**, {what}"), &frame);
    }
    let mut tile = rt.instantiate(&component)?;
    tile.on_event(Event::Configure(Configure { nodes: NODES, cached: true }), HANDLE_DEADLINE)?;
    let f = tile.render(RENDER_DEADLINE)?;
    let strings: usize = f.nodes.iter().map(|n| n.key.len() + match &n.element {
        pitwall_tile_host::Element::Text(t) => t.len(),
        pitwall_tile_host::Element::Button(b) => b.label.len(),
        _ => 0,
    }).sum();
    let styles: usize = f.nodes.iter().map(|n| n.style.len()).sum();

    // Epoch interruption: deadline → trap, with the 10 ms ticker.
    let ticker = EpochTicker::start(rt.engine())?;
    for (ticks_only, deadline_ms) in [(true, 100u64), (true, 1000), (false, 100), (false, 1000)] {
        let deadline = Duration::from_millis(deadline_ms);
        let mut over = Vec::new();
        for _ in 0..if deadline_ms > 500 { 3 } else { 10 } {
            let mut tile = rt.instantiate(&component)?;
            if ticks_only {
                tile.use_tick_deadlines();
            }
            let t = Instant::now();
            let err = tile.on_event(Event::Spin, deadline).expect_err("must trap");
            let took = t.elapsed();
            assert_eq!(err.downcast_ref::<wasmtime::Trap>(), Some(&wasmtime::Trap::Interrupt));
            over.push(took.saturating_sub(deadline));
        }
        let how = if ticks_only { "ticks only" } else { "ticks + wall clock" };
        row(&format!("epoch kill ({how}): overshoot past a {deadline_ms} ms deadline"), &over);
    }

    // The worker: round trip from the UI thread's send to the frame back.
    let (tx, rx) = mpsc::channel();
    let worker = Worker::spawn("bench", rt.clone(), component.clone(), move |m| {
        let _ = tx.send((Instant::now(), m));
    })?;
    let mut trips = Vec::new();
    let mut kill = Vec::new();
    wait_frame(&rx)?;
    for _ in 0..200 {
        let t = Instant::now();
        worker.send(Event::Click(1));
        let at = wait_frame(&rx)?;
        trips.push(at - t);
    }
    row("worker: click → frame back on the UI thread (4 nodes)", &trips);
    let t = Instant::now();
    worker.send(Event::Spin);
    loop {
        match rx.recv_timeout(Duration::from_secs(5))? {
            (at, FromWorker::Stopped { .. }) => {
                kill.push((at - t).saturating_sub(pitwall_tile_host::HANDLE_DEADLINE));
                break;
            }
            _ => continue,
        }
    }
    row("worker: hang → Stopped on the UI thread, past the 1 s deadline", &kill);
    drop(worker);
    drop(ticker);

    fallbacks(&wasm)?;

    println!("\nFrame shape: {} nodes, {} style entries, {} bytes of strings.", f.nodes.len(), styles, strings);
    let imports: Vec<String> = component.component_type().imports(rt.engine()).map(|(n, _)| n.to_string()).collect();
    println!("Component imports: {}", imports.join(", "));
    let _ = std::fs::remove_dir_all(&scratch);
    let _: PathBuf = scratch;
    Ok(())
}

/// The fallbacks plan.md lists for phase 0: fuel in place of epochs (its
/// cost on a frame, and how fuel maps to time), and Winch in place of
/// Cranelift (with `--features winch`).
fn fallbacks(wasm: &[u8]) -> R<()> {
    // Fuel only (no epochs), so the frame pays for fuel checks alone.
    let rt = Runtime::new(engine_with(None, |c| {
        c.epoch_interruption(false);
        c.consume_fuel(true);
    })?)?
    .with_fuel(u64::MAX / 4);
    let component = rt.compile(wasm)?;
    frame_rows(&rt, &component, "fuel instead of epochs")?;
    // How long a spinning tile runs on a given amount of fuel.
    let mut per_ms = Vec::new();
    for _ in 0..5 {
        let mut tile = rt.instantiate(&component)?;
        let fuel = 1_000_000_000u64;
        tile.set_fuel(fuel)?;
        let t = Instant::now();
        let err = tile.on_event(Event::Spin, HANDLE_DEADLINE).expect_err("out of fuel");
        let took = t.elapsed();
        assert_eq!(err.downcast_ref::<wasmtime::Trap>(), Some(&wasmtime::Trap::OutOfFuel), "{err:?}");
        per_ms.push(took);
    }
    row("fuel: a spinning tile's run on 10^9 fuel (fuel is not time)", &per_ms);

    #[cfg(feature = "winch")]
    {
        let mut compile = Vec::new();
        let mut component = None;
        for _ in 0..10 {
            let rt = Runtime::new(engine_with(None, |c| {
                c.strategy(wasmtime::Strategy::Winch);
            })?)?;
            let t = Instant::now();
            match rt.compile(wasm) {
                Ok(c) => component = Some((rt, c)),
                Err(e) => {
                    println!("| Winch: compile failed: {e:#} | | | | | |");
                    return Ok(());
                }
            }
            compile.push(t.elapsed());
        }
        row("Winch: compile (fresh engine)", &compile);
        if let Some((rt, c)) = component {
            frame_rows(&rt, &c, "Winch")?;
        }
    }
    Ok(())
}

fn frame_rows(rt: &Runtime, component: &wasmtime::component::Component, what: &str) -> R<()> {
    let mut tile = rt.instantiate(component)?;
    tile.on_event(Event::Configure(Configure { nodes: NODES, cached: false }), HANDLE_DEADLINE)?;
    let mut s = Vec::new();
    for _ in 0..FRAMES {
        let t = Instant::now();
        tile.on_event(Event::Click(1), HANDLE_DEADLINE)?;
        tile.render(RENDER_DEADLINE)?;
        s.push(t.elapsed());
    }
    row(&format!("{what}: {NODES} nodes, event + render"), &s);
    Ok(())
}

fn wait_frame(rx: &mpsc::Receiver<(Instant, FromWorker)>) -> R<Instant> {
    loop {
        match rx.recv_timeout(Duration::from_secs(5))? {
            (at, FromWorker::Frame { .. }) => return Ok(at),
            (_, FromWorker::Stopped { reason }) => return Err(reason.into()),
            _ => continue,
        }
    }
}

fn row(what: &str, samples: &[Duration]) {
    let mut s = samples.to_vec();
    s.sort();
    let pct = |p: f64| s[((s.len() as f64 - 1.0) * p).round() as usize];
    println!(
        "| {what} | {} | {} | {} | {} | {} |",
        ms(pct(0.5)),
        ms(pct(0.95)),
        ms(s[0]),
        ms(s[s.len() - 1]),
        s.len()
    );
}

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}
