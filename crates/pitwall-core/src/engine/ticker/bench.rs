//! How much git the ticker runs for many local agents (perf.md, "Git
//! refresh"). Real temp repos, real git, the real ticker on a clock that
//! follows wall time. Not part of the normal test run:
//!
//! `BENCH_MODE=busy BENCH_AGENTS=20 BENCH_SECS=60 cargo test -p pitwall-core
//! --release --lib -- --ignored --exact engine::ticker::bench::git_bench
//! --nocapture` (under `/usr/bin/time -l` for CPU).
//!
//! Modes: `idle` (agents idle, nothing changes), `thinking` (working, no
//! file changes), `busy` (working, each agent edits a file every
//! `BENCH_EDIT_MS`, default 2 s). `BENCH_WATCH=0` polls (remote-style)
//! instead of watching the checkouts.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{tick, TICK};
use crate::engine::gitwatch::tests::CountingExec;
use crate::model::Status;
use crate::testing::{record, Harness};
use crate::vcs::snapshot::TempRepo;

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[test]
#[ignore = "benchmark: run by hand (perf.md)"]
fn git_bench() {
    let mode = env("BENCH_MODE", "busy");
    let n: usize = env("BENCH_AGENTS", "20").parse().unwrap();
    let secs: u64 = env("BENCH_SECS", "30").parse().unwrap();
    let every =
        (env("BENCH_EDIT_MS", "2000").parse::<u64>().unwrap() / TICK.as_millis() as u64).max(1);
    let watch = env("BENCH_WATCH", "1") == "1";
    let repos: Vec<TempRepo> = (0..n)
        .map(|i| {
            let r = TempRepo::new();
            for f in 0..50 {
                r.write(
                    &format!("src/m{f}/file{f}.txt"),
                    &format!("{i} {f}\n").repeat(20),
                );
            }
            r.write(".gitignore", "node_modules/\n");
            r.commit_all("init");
            r
        })
        .collect();
    let exec = Arc::new(CountingExec::default());
    let recs = repos
        .iter()
        .enumerate()
        .map(|(i, r)| record(&format!("a{i}"), r.path()))
        .collect();
    let h = Harness::with_exec(recs, exec.clone());
    let status = if mode == "idle" {
        Status::Idle
    } else {
        Status::Working
    };
    for i in 0..n {
        h.engine
            .with(&format!("a{i}"), |a| {
                a.status = status;
                a.facts.provider.fs_events = watch;
            })
            .unwrap();
    }
    let mut badge = usize::MAX;
    // Settle the start (every agent is refreshed once, and once more when
    // FSEvents restarts its stream for each new watch): 5 s of ticks.
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_secs(5) {
        h.clock.advance(TICK.as_millis() as u64);
        tick(&h.engine, &mut badge);
        std::thread::sleep(TICK);
    }
    let start_runs = exec.runs.load(Ordering::Relaxed);
    let start_micros = exec.micros.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut step: u64 = 0;
    while started.elapsed() < Duration::from_secs(secs) {
        step += 1;
        if mode == "busy" {
            for (i, r) in repos.iter().enumerate() {
                if (step + i as u64).is_multiple_of(every) {
                    r.write("src/m0/file0.txt", &"x\n".repeat(1 + (step % 13) as usize));
                }
            }
        }
        h.clock.advance(TICK.as_millis() as u64);
        tick(&h.engine, &mut badge);
        std::thread::sleep(TICK);
    }
    let elapsed = started.elapsed().as_secs_f64();
    let runs = exec.runs.load(Ordering::Relaxed) - start_runs;
    let ms = (exec.micros.load(Ordering::Relaxed) - start_micros) as f64 / 1000.0;
    let per_min = 60.0 / elapsed;
    println!(
        "BENCH mode={mode} watch={watch} edit_ms={} agents={n} secs={elapsed:.0}: git processes {runs} ({:.0}/min), git time {ms:.0} ms ({:.0} ms/min)",
        every * TICK.as_millis() as u64,
        runs as f64 * per_min,
        ms * per_min
    );
}
