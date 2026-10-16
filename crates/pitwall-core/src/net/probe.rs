//! Probing discovery candidates (collaboration teams.md §3 "Probe" and
//! "Limits"): `GET /pitwall/v1/info` through [`Network::client`] on each
//! [`PeerHint`] a backend's [`Network::discover`] offered. Network-neutral:
//! the same for every backend.
//!
//! - at most 256 devices per refresh, 16 at a time, 2 s to connect;
//! - offline devices aren't probed;
//! - a device that didn't answer isn't probed again for 24 h, unless it
//!   went offline and came back, its name changed, or the user pressed ↻
//!   (`force`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use super::{http, Body, DeviceAddr, Network, PeerHint, Timeouts};

/// The discovery probe's path (protocol.md §8.4).
pub const INFO_PATH: &str = "/pitwall/v1/info";

/// How much one refresh may probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeLimits {
    pub per_refresh: usize,
    pub concurrent: usize,
    pub connect: Duration,
    /// Silence on an open probe before giving up.
    pub idle: Duration,
    /// How long a device that didn't answer is left alone.
    pub retry_after: Duration,
    /// The largest answer read.
    pub max_answer: usize,
}

impl Default for ProbeLimits {
    fn default() -> ProbeLimits {
        ProbeLimits {
            per_refresh: 256,
            concurrent: 16,
            connect: Duration::from_secs(2),
            idle: Duration::from_secs(5),
            retry_after: Duration::from_secs(24 * 60 * 60),
            max_answer: 64 * 1024,
        }
    }
}

/// A device that answered, with its answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Probed<T> {
    pub hint: PeerHint,
    pub info: T,
}

/// Probes candidates and remembers who didn't answer. One per network
/// (or one for all: addresses name their network).
pub struct Prober {
    limits: ProbeLimits,
    silent: Mutex<HashMap<DeviceAddr, Silent>>,
}

/// A device that didn't answer.
struct Silent {
    at: Instant,
    label: String,
    /// It was seen offline since: probe it once it is back.
    went_offline: bool,
}

impl Default for Prober {
    fn default() -> Prober {
        Prober::new(ProbeLimits::default())
    }
}

impl Prober {
    pub fn new(limits: ProbeLimits) -> Prober {
        Prober { limits, silent: Mutex::new(HashMap::new()) }
    }

    /// Forget who didn't answer (↻).
    pub fn forget(&self) {
        self.silent.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Probe `hints` on `net` and return those that answered with a `T`
    /// (`collab::team::PitwallInfo`). `force` probes devices that recently
    /// didn't answer, too. Blocks until every probe is done.
    pub fn probe<T: DeserializeOwned + Send>(&self, net: &dyn Network, hints: &[PeerHint], force: bool) -> Vec<Probed<T>> {
        let due = self.due(hints, force);
        let queue = Mutex::new(due.into_iter());
        let found = Mutex::new(Vec::new());
        let workers = self.limits.concurrent.max(1);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                    let Some(hint) = next else { return };
                    match self.ask::<T>(net, &hint) {
                        Some(info) => {
                            self.silent.lock().unwrap_or_else(|e| e.into_inner()).remove(&hint.addr);
                            found.lock().unwrap_or_else(|e| e.into_inner()).push(Probed { hint, info });
                        }
                        None => {
                            let silent = Silent { at: Instant::now(), label: hint.label.clone(), went_offline: false };
                            self.silent.lock().unwrap_or_else(|e| e.into_inner()).insert(hint.addr.clone(), silent);
                        }
                    }
                });
            }
        });
        let mut found = found.into_inner().unwrap_or_else(|e| e.into_inner());
        found.sort_by(|a, b| a.hint.label.cmp(&b.hint.label));
        found
    }

    /// The hints to probe now, at most `per_refresh`.
    fn due(&self, hints: &[PeerHint], force: bool) -> Vec<PeerHint> {
        let mut silent = self.silent.lock().unwrap_or_else(|e| e.into_inner());
        let mut due = Vec::new();
        for hint in hints {
            if !hint.online {
                if let Some(s) = silent.get_mut(&hint.addr) {
                    s.went_offline = true;
                }
                continue;
            }
            let skip = !force
                && silent.get(&hint.addr).is_some_and(|s| {
                    s.at.elapsed() < self.limits.retry_after && !s.went_offline && s.label == hint.label
                });
            if !skip && due.len() < self.limits.per_refresh {
                due.push(hint.clone());
            }
        }
        due
    }

    fn ask<T: DeserializeOwned>(&self, net: &dyn Network, hint: &PeerHint) -> Option<T> {
        let mut req = http::Request::get(INFO_PATH).header(http::header::ACCEPT, "application/json").body(Body::Empty).ok()?;
        req.extensions_mut().insert(Timeouts { connect: self.limits.connect, idle: Some(self.limits.idle) });
        let res = net.client(&hint.addr).ok()?.send(req).ok()?;
        if !res.status().is_success() {
            return None;
        }
        let bytes = res.into_body().read_all(self.limits.max_answer).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::net::fake::FakeNet;
    use crate::net::{Caller, Request, Response};

    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Info {
        label: String,
    }

    fn info(label: &'static str, hits: Arc<AtomicUsize>) -> Arc<dyn crate::net::Handler> {
        Arc::new(move |req: Request, _caller: Caller| {
            hits.fetch_add(1, Ordering::SeqCst);
            assert_eq!(req.uri().path(), INFO_PATH);
            assert_eq!(req.extensions().get::<Timeouts>().map(|t| t.connect), Some(Duration::from_secs(2)));
            Response::new(Body::json(&serde_json::json!({ "label": label })))
        })
    }

    #[test]
    fn probes_online_devices_and_leaves_silent_ones_alone() {
        let net = FakeNet::new("example");
        let me = net.device("alice@example.com", "studio");
        let hits = Arc::new(AtomicUsize::new(0));
        let _bob = net.device("bob@example.com", "bob-mbp").serve(info("bob-mbp", hits.clone())).unwrap();
        net.device("carol@example.com", "carol-mbp"); // no Pitwall serving
        net.device("erin@example.com", "erin-mbp");
        net.set_online("erin-mbp", false);

        let prober = Prober::default();
        let hints = me.discover().unwrap();
        let found: Vec<Probed<Info>> = prober.probe(&me, &hints, false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].info, Info { label: "bob-mbp".into() });
        assert_eq!(found[0].hint.addr, net.addr("bob-mbp"));

        // Carol didn't answer: not asked again for a day…
        let _carol = net.device("carol@example.com", "carol-mbp").serve(info("carol-mbp", hits.clone())).unwrap();
        let found: Vec<Probed<Info>> = prober.probe(&me, &me.discover().unwrap(), false);
        assert_eq!(found.len(), 1);
        // …unless forced (↻), or she went offline and came back.
        let found: Vec<Probed<Info>> = prober.probe(&me, &me.discover().unwrap(), true);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn a_device_that_comes_back_online_is_probed_again() {
        let net = FakeNet::new("example");
        let me = net.device("alice@example.com", "studio");
        net.device("carol@example.com", "carol-mbp");
        let prober = Prober::default();
        assert!(prober.probe::<Info>(&me, &me.discover().unwrap(), false).is_empty());

        net.set_online("carol-mbp", false);
        assert!(prober.probe::<Info>(&me, &me.discover().unwrap(), false).is_empty());
        let _carol = net.device("carol@example.com", "carol-mbp").serve(info("carol-mbp", Arc::default())).unwrap();
        assert_eq!(prober.probe::<Info>(&me, &me.discover().unwrap(), false).len(), 1);
    }

    #[test]
    fn at_most_the_limit_per_refresh() {
        let net = FakeNet::new("example");
        let me = net.device("alice@example.com", "studio");
        let hits = Arc::new(AtomicUsize::new(0));
        let mut serving = Vec::new();
        for n in 0..10 {
            let node = format!("node-{n}");
            serving.push(net.device(&format!("u{n}@example.com"), &node).serve(info("x", hits.clone())).unwrap());
        }
        let prober = Prober::new(ProbeLimits { per_refresh: 4, concurrent: 3, ..ProbeLimits::default() });
        assert_eq!(prober.probe::<Info>(&me, &me.discover().unwrap(), false).len(), 4);
        assert_eq!(hits.load(Ordering::SeqCst), 4);
    }
}
