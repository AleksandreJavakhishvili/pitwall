Status: phase 0 spike, measured 2026-11-26 (draft PR #107). A prototype;
nothing ships.

# Tile SDK: phase 0 spike results

Part of the [Tile SDK and marketplace](README.md) design ([plan.md](plan.md),
phase 0; [gates](README.md#phase-0-gates)). The question: is running tiles
as WebAssembly components in Wasmtime, with their UI drawn natively by
GPUI, viable? This page gives the measurements, compares each one with
its gate, and ends with a recommendation.

## Summary

| Gate | Pass line | Measured | Result |
|---|---|---|---|
| Release-binary growth | ≤ 10 MB per platform, each macOS architecture | As first built (Cranelift, component model, WASI p2, Wasmtime's cache): macOS arm64 **+8.2**, macOS x86-64 **+11.8**, Linux **+13.1**, Windows **+11.7** MB. After the gate's own fallback ("turn off more features and measure again"), without Wasmtime's cache and with opt-level "s" on Wasmtime's crates: macOS arm64 **+5.8**, macOS x86-64 **+7.3**, Linux **+8.4**, Windows **+7.3** MB | **miss as first built; pass after the fallback on every platform** |
| 2 000-node frame | ABI + decode + diff + apply ≤ 4 ms (M-series) | Canonical ABI + validate + keyed diff: **0.28 ms** median, 0.30 ms p95. Mapping to GPUI elements adds 0.2–0.3 ms. GPUI layout and paint of all 2 000 nodes: 3.5–5.1 ms | **pass** (14× margin on the protocol). See the note on "apply" |
| Cold start | ≤ 300 ms | Hello tile (57 KB): **7.5 ms**. A 1.0 MB tile: 56–108 ms. CI: 17–40 ms for hello | **pass** |
| Warm start | ≤ 50 ms | Hello: **0.16 ms**. 1.0 MB: **1.3–1.9 ms** | **pass** |
| Runaway tile stopped | ≤ 100 ms past the deadline | With the wall-clock check (below): **≈ 7 ms** median, ≤ 12 ms max on the Mac, ≤ 10 ms on Linux and Windows CI. Counting ticks alone, as runtime.md says: 200–430 ms late at 1 s | **pass** with the wall-clock check; **miss** as specified. The macOS CI VM is borderline (up to 100 ms) |
| Threading model | confirmed | The store lives on the worker; the UI thread only sends and receives | **confirmed** |
| Fallbacks (unverified until now) | measured | Winch compiles 3–5× faster than Cranelift and runs the frame 1.9× slower. Fuel costs about 25 % on a frame and isn't time | **measured** |

Recommendation: **continue, with changes** (see [the end](#recommendation)).

## What was built

All of it is in `crates/pitwall-tile-host` (branch `spike/tile-host`, PR
#107):

| Piece | Where | What it does |
|---|---|---|
| WIT world | `wit/tile.wit` | `pitwall:tile-spike@0.0.1`: a slice of [ui.md 3.3](ui.md#33-the-ui-description). A flat node list with parent indices; each node has a key, an element (`box`, `text`, `button` with a handler number) and a list of style variants (tokens, lengths, flex). The tile exports `render() -> frame` and `on-event(event) -> bool`; the host provides `log`. |
| Hello tile | `tests/fixtures/hello-tile/` | Rust, `wit-bindgen` 0.63, `wasm32-wasip2`. A counter with a button. Two spike-only events: `configure(n)` builds a synthetic tree of n nodes, and `spin` loops forever. It is its own workspace, built and committed as `hello-tile.wasm` (57 KB), so the main workspace never needs the wasm target. `build-hello-tile.sh` rebuilds it. `build-hello-tile.sh heavy` builds a 1.0 MB variant (regex, serde_json, chrono), which is not committed. |
| Host | `src/lib.rs` | Wasmtime 48.0.10, default features off. `Runtime` holds the engine and a linker with `log` and WASI 0.2, nothing granted. `TileInstance` gives every call a deadline. `Worker` keeps the store on a thread named `tile:<id>`. Pitwall's own compiled-code cache uses `serialize` and `deserialize_file`. Features `wasmtime-cache` and `winch` exist only for the comparisons. |
| Diff | `src/diff.rs` | Checks shape and limits, hashes each node's key path, and diffs by key: added, removed, changed, and which old node each new node continues. |
| Timings | `examples/tile_bench.rs` | Everything below except GPUI. No window. |
| GPUI mapping | `examples/tile_gpui.rs` | Maps a frame to GPUI `div`s with styles, text, and clickable buttons that only post an event to the worker. Laid out and painted in GPUI's test platform with the system's real text system. No window on screen, no synthetic input. |
| App link | `pitwall-app` features `tile-spike`, `tile-spike-cache` | Off by default. They make the whole runtime reachable from `main` (`PITWALL_TILE_SPIKE=<wasm>` runs it), so a release build shows what Wasmtime really adds. Not wired into the layout. |
| Size CI | `.github/workflows/tile-spike.yml`, `scripts/tile-spike-size.sh`, `scripts/tile-spike-lean.toml` | Runs only on the spike branch or by hand, never in `ci.yml`. It builds the release app in four variants on macOS (both slices), Linux and Windows, then runs `tile_bench`. |

Run them: `cargo test -p pitwall-tile-host`;
`cargo run --release -p pitwall-tile-host --example tile_bench [-- <wasm>] [--features winch]`;
`cargo run --release -p pitwall-tile-host --example tile_gpui`;
`scripts/tile-spike-size.sh [--target x86_64-apple-darwin]`.

### Toolchain: wit-bindgen, not cargo-component

The guest uses `wit_bindgen::generate!` and plain
`cargo build --target wasm32-wasip2`. Since Rust 1.82, rustc's
`wasm32-wasip2` target links a component itself (`wasm-component-ld`),
so one dependency and a rustup target is all it takes.
`cargo-component` is a separate cargo subcommand that authors would have
to install and keep in step with Pitwall, for nothing this needs. The
SDK's guest crate (`pitwall-extension`) is planned on `wit-bindgen`
anyway.

### Wasmtime version

On 2026-11-26 crates.io lists 51.0.0 (20 Nov) as the newest release,
48.0.10 (19 Nov) as the newest on the 48 line, and 36.0.23 as still
patched. So 48 is the current LTS, as [10.3](README.md#103-decisions)
item 5 says, and 60 will be the next. Wasmtime 48 needs Rust 1.95; the
workspace uses 1.98.1.

Features used: `runtime`, `cranelift`, `component-model`,
`parallel-compilation`, `std`, and (opt-in) `cache`. `wasmtime-wasi`
with `p2` also turns on `async`, which brings fibers and, through
wasmtime-wasi, tokio.

## Binary size

Local builds are on macOS 26.5 with an M5 Max; CI uses GitHub's runners.
All builds use the workspace release profile: LTO, one codegen unit,
`panic=abort`, stripped. Each row compares the `pitwall` binary without
the tile runtime and with it.

| Platform | without | + runtime with Wasmtime's cache (run 1) | + runtime (no cache) | lean: no cache, opt-level "s" on Wasmtime's crates |
|---|---|---|---|---|
| macOS arm64 (local) | 71.03 MB | +8.23 MB | +7.60 MB | **+5.75 MB** |
| macOS x86-64 slice (local, cross-built) | 73.85 MB | +11.76 MB | +11.00 MB | **+7.28 MB** |
| macOS arm64 (CI) | 71.03 MB | +8.23 MB | +7.60 MB | **+5.75 MB** |
| macOS x86-64 slice (CI) | — | — | +11.02 MB | **+7.29 MB** |
| Linux x86-64 (CI) | 91.52 MB | +13.08 MB | +12.22 MB | **+8.41 MB** |
| Windows x86-64 (CI) | 76.38 MB | +11.71 MB | +11.10 MB | **+7.30 MB** |

The x86-64 builds grow 40–70 % more than arm64. Cranelift's x86
backend is the larger one (*likely, not broken down*). The macOS
download is universal (release.yml), so it carries both slices: +13 MB
lean, +19 MB as first built.

What each piece costs, from a separate probe binary with the same
profile (macOS arm64, growth over a 0.30 MB hello world):

| Wasmtime features | opt-level 3 | opt-level "s" |
|---|---|---|
| `runtime` + `component-model` only (loads precompiled code; no compiler) | +0.58 MB | +0.50 MB |
| … + WASI p2 | +2.16 MB | +1.60 MB |
| `cranelift` (no WASI) | +6.14 MB | +4.49 MB |
| … + `parallel-compilation` | +6.19 MB | +4.54 MB |
| `cranelift` + WASI p2 | +7.72 MB | +5.59 MB |
| … + `cache` | +8.58 MB | +6.22 MB |

What was tried, in the order the gate lists:

1. **Without Wasmtime's `cache`** (−0.6 to −0.8 MB). Pitwall keeps its
   own `.cwasm` files anyway ([runtime.md 2.6](runtime.md#26-compile-and-cache)),
   and they load 2–3× faster. On its own this is not enough on x86-64.
2. **opt-level "s" for Wasmtime's and Cranelift's crates only**
   (`scripts/tile-spike-lean.toml`, as `[profile.release.package.*]`
   entries; the rest of the app stays at opt-level 3). This saves a
   further 1.9 MB on arm64 and 3.7 MB on x86-64, despite LTO. The cost
   (1.0 MB tile, macOS): cold compile 93 ms instead of 70 ms, and a
   2 000-node frame 0.44 ms instead of 0.37 ms. Warm start is the same.
   Every number stays far inside its gate.
3. **Not needed, still available:** no compiler in the app. Ship only
   the runtime (+0.6 MB, +2.2 MB with WASI) and load Pulley bytecode,
   which is portable, compiled by the index's CI or by
   `pitwall extension pack`. Dev reload then needs a compiler in the
   CLI, and tiles run as interpreted code (several times slower;
   *unverified for tiles*). Winch doesn't help here: it is another
   compiler linked beside Cranelift's code generator. Its size wasn't
   measured.

## The 2 000-node frame

The tile builds rows shaped like a real tile: a box with five styles, a
dot, a title with grow and truncate, a detail text and an "Open"
button. That makes 2 000 nodes, 4 402 style entries and 17 KB of
strings, roughly 150 KB as flat canonical-ABI data, inside the
100–200 KiB estimate in runtime.md 2.7. Each frame changes one row in
50, as a refresh would. Medians over 300 frames:

| Step (thread) | M5 Max | Linux CI | Windows CI | macOS CI |
|---|---|---|---|---|
| `on-event` + `render`: the guest builds the tree, then canonical ABI lower and lift into host Rust types (worker) | 0.222 ms | 0.568 ms | 0.628 ms | 0.375 ms |
| Validate, hash key paths, keyed diff with 16 changed (main) | 0.061 ms | 0.146 ms | 0.152 ms | 0.121 ms |
| **Frame total** (p95) | **0.284 ms** (0.303) | 0.715 ms (0.741) | 0.791 ms (0.857) | 0.498 ms (0.564) |

With a frame the guest keeps, so that mostly the ABI is left, the render
step is 0.19 ms (Mac): building the tree is the smaller part.

GPUI side (`tile_gpui`, main thread, real text system, test platform,
800 × 600, Mac):

| Step | median | p95 |
|---|---|---|
| Frame → GPUI element tree (`map`) | 0.22–0.29 ms | 0.28–0.32 ms |
| `map` + layout + prepaint + paint of all 2 000 nodes | 3.5–5.1 ms | 4.2–5.7 ms |

The protocol is not where the time goes. Lifting, diffing and mapping a
2 000-node frame cost about 0.5 ms together. GPUI's layout and paint of
2 000 elements cost 3.5–5 ms, as they would for a native tile. On
"apply" in the gate: if it means updating the host's retained state,
the gate passes with a 14× margin. If it means GPUI's full layout and
paint of every node, it lands right at the line. That cost is not
specific to tiles, and is the reason the design keeps large collections
in virtual lists. **Full tree + host diff is enough for WIT 0.1**
([10.3](README.md#103-decisions) item 7): patches would save part of
0.28 ms.

## Cold and warm start

| | hello (57 KB) | 1.0 MB tile |
|---|---|---|
| `Engine::new` (once per app) | 0.006 ms | — |
| Compile with Cranelift + write Pitwall's cache file | 7.4 ms (p95 25) | 56–108 ms (p95 ≤ 135) |
| Instantiate | 0.05 ms | 0.10–0.14 ms |
| First render | 0.016 ms | 0.04–0.05 ms |
| **Cold total** | **7.5 ms** | **56–108 ms** |
| `deserialize_file` (Pitwall cache hit) | 0.12 ms | 1.2–1.8 ms |
| **Warm total, Pitwall cache** | **0.16 ms** | **1.3–1.9 ms** |
| Warm total, Wasmtime `Config::cache` hit | 0.33 ms | 3.8–4.9 ms |

CI, hello tile, cold / warm: Linux 33 / 0.34 ms, Windows 40 / 0.38 ms,
macOS 17 / 0.22 ms. Ranges are separate runs on a Mac that was also
doing other work. Compile time grows roughly with code size and uses
every core; the M5 Max has 18. A 4-core machine would take perhaps
0.2–0.4 s for a 1 MB tile (*extrapolated*), so a big tile on a small
laptop can pass 300 ms. Compiling happens once per wasm and Wasmtime
version, off the main thread, while the tile shows "Preparing…", so a
slow compile delays only that tile.

## Stopping a runaway tile

runtime.md 2.5's recipe is a ticker thread calling `increment_epoch`
every 10 ms, and each call setting `set_epoch_deadline(deadline / 10 ms)`
with `epoch_deadline_trap`. **It overshoots**, because a thread's
`sleep(10 ms)` often takes longer: about 12–14 ms per tick on the Mac,
and 50–100 ms on the macOS CI VM. The error grows with the deadline.

**The fix, in the spike:** an `epoch_deadline_callback` checks a
wall-clock `Instant` on every tick. Each call arms
`set_epoch_deadline(1)` and stores its deadline. At each tick the
callback returns `Interrupt` past the deadline and `Continue(1)` before
it. The overshoot is then at most one tick. The callback runs about 100
times a second, and only while the guest runs.

Overshoot past the deadline, median / max:

| Deadline | Mac, ticks only | Mac, ticks + clock | Linux CI, ticks + clock | Windows CI, ticks + clock | macOS CI, ticks + clock |
|---|---|---|---|---|---|
| 100 ms | 17–47 / 50 ms | **5–7 / 12 ms** | 0.7 / 0.8 ms | 2.5 / 4.0 ms | 48 / 100 ms |
| 1 s | 173–430 / 463 ms | **5–8 / 13 ms** | 8.6 / 9.8 ms | 4.5 / 6.7 ms | 34 / 72 ms |
| 1 s, through the worker until `Stopped` reaches the UI thread | — | **1–10 ms** | 5.5 ms | 4.2 ms | 73 ms |

Ticks alone on the macOS CI VM were 0.5 s late at 100 ms and 4.8 s late
at 1 s. Linux's sleeps are precise, so ticks alone pass there. The trap
is `Trap::Interrupt`. The worker then drops the instance and reports
"it didn't answer in time". The macOS CI VM is the one borderline case.
Its sleeps are coarse, so its overshoot is one coarse tick. If real
machines ever behave like it, the ticker should sleep until an absolute
next-tick time, or run at a higher priority. Both are cheap.

## Threading model (confirmed)

- **The store lives on the tile's worker thread.** `Store<HostState>`
  is `Send` because the state is: the WASI context, the resource table,
  the limits and the log buffer. The worker thread creates it, uses it
  and drops it. The test checks that replies come from the thread named
  `tile:<id>`.
- **Calls never run on the UI thread.** The UI thread calls
  `Worker::send(event)`, an `mpsc` send that never blocks. The worker
  loop blocks on the channel, takes every queued event (a burst makes
  one render), calls `on-event` and then `render` with their deadlines,
  and hands `FromWorker::Frame`, `Log` or `Stopped` to a sink. In the
  app, that sink is a channel drained by a foreground task, which
  applies the frame and calls `cx.notify()`.
- **GPUI handlers only post.** In the mapping, a button's `on_click`
  sends `Event::Click(handler)` to the worker, and the new frame
  arrives later: "Clicked 0 times" became "Clicked 1 times". The guest
  is never re-entered, because every call comes from the one worker
  loop.
- From send to frame back on the UI thread: **3 µs** median (hello).
- One epoch ticker thread per engine (`tile-epoch`), shared by every
  tile. One OS thread per running tile.
- No per-OS code was needed. Wasmtime's trap handling worked on macOS,
  Linux and Windows without anything in `src/platform/`.

## The fallbacks, measured

| | Hello (57 KB) | 1.0 MB tile |
|---|---|---|
| Cranelift compile | 7.4 ms | 56–108 ms |
| **Winch compile** | **2.3 ms** (3×) | **10.7 ms** (5×) |
| 2 000-node frame (event + render), Cranelift | 0.225 ms | 0.231 ms |
| … Winch | 0.421 ms (1.9×) | 0.442 ms |
| … fuel instead of epochs (Cranelift) | 0.280 ms (+25 %) | 0.303 ms (+31 %) |
| A spinning tile's run on 10⁹ fuel | 18 ms | 18 ms |

- **Winch** (`--features winch`) compiles 3–5× faster and runs about 2×
  slower. That makes it a good fit for dev reload, or for a first run
  while Cranelift compiles in the background. Not needed for the
  cold-start gate. Not measured: its binary cost.
- **Fuel** stops a tile deterministically, but fuel counts instructions,
  not time: 10⁹ units lasted 18 ms in a tight loop and would last much
  longer in a loop of host calls. It also costs 25–30 % on every frame.
  Epochs with the wall-clock check are the better stop, as runtime.md
  2.5 chose.

## Other findings

- **WASI imports from std.** The hello tile uses no WASI itself, yet its
  component imports `wasi:cli/{stdin,stdout,stderr,environment,exit,terminal-*}`,
  `wasi:io/{poll,error,streams}` and `wasi:clocks/monotonic-clock`
  (`@0.2.9`), from Rust's std and its panic path. So the host links
  WASI p2 (+1.6 MB) even for tiles that never touch files. Stubs that
  deny would also do ([5.2](permissions.md#52-enforcement)), but the SDD
  wants `/data` through WASI anyway.
- `bindgen!` needs `additional_derives: [PartialEq]` for the diff. The
  real model in `pitwall-core` (`tile::model`) has its own types, so
  core stays free of Wasmtime, and converting each frame into them costs
  about as much as the diff (*estimate*: 0.05 ms for 2 000 nodes).
- The spike's key-path hash (FNV-1a over parent hash and key) trusts
  the hash. The real diff must compare keys when two hashes match.
- The spike names cache files by an FNV hash of the wasm; the real cache
  uses SHA-256 in a folder only Pitwall writes, since
  `deserialize_file` is `unsafe` (runtime.md 2.6).
- `wasmtime-wasi` p2 brings tokio and Wasmtime's `async` even though
  the host uses only the synchronous API. Fine in process; worth
  noting for a future `pitwall-tiled` helper.

## Recommendation

**Continue, with changes.** Every gate passes once three changes are
made, and two of them are the gates' own fallbacks:

1. **Size: build Wasmtime lean.** Leave out Wasmtime's `cache` feature
   and use only Pitwall's `.cwasm` files. Set opt-level "s" for
   Wasmtime's and Cranelift's crates in the workspace's release profile.
   That puts every platform under 10 MB: +5.8 MB arm64 and +7.3 MB
   x86-64 on macOS, +8.4 MB on Linux, +7.3 MB on Windows. As built
   at first, x86-64 misses on all three OSes. The universal macOS
   download grows about 13 MB in total. Ask the owner whether "per
   platform" means per architecture, as plan.md says, or per download.
2. **Stopping: deadlines are wall-clock.** Check the deadline against
   the clock on each epoch tick (`epoch_deadline_callback`), not as a
   number of ticks. Update runtime.md 2.5. Without this, the 1 s stop
   misses badly on macOS.
3. **Compiling: plan for small machines.** Keep compiling off the main
   thread with "Preparing…". Consider Winch for dev reload or a first
   run (3–5× faster compile). In phase 1, check that a 2–3 MB tile
   compiles in under 1 s on a 4-core CI runner.

The core bets hold with wide margins. The full-tree protocol costs
0.3 ms for 2 000 nodes against a 4 ms budget. Warm starts take about
1 ms. The worker-thread model works as drawn in runtime.md 2.4. The
real cost is GPUI's own layout and paint of large trees, the same as
for native tiles, which is why the design uses virtual lists.
