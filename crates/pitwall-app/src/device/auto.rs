//! A device's screen opening on its own beside the agent that started it,
//! and the device an agent's `show_device` names (docs/spec/device.md,
//! "Opening on its own" and "Agents: show_device"). Pure: the listings,
//! the agents' signals and the clock come in; what to show goes out. The
//! app's side is `main_screen::device_auto`.
//!
//! **Watching.** A platform's devices are listed only while some agent
//! gives a signal for it ([`pitwall_core::devices`]: a launcher in its
//! process tree, or a line its terminal printed) and for
//! [`KEEP_MS`] after. The first listing of such a period is the baseline;
//! a device that comes online after it is a boot. An iOS Simulator also
//! says when it booted (`lastBootedAt`), so one that booted just before
//! the first listing (a quick `simctl boot`) still counts.
//!
//! **Whose.** A boot is an agent's when exactly one agent had a launcher
//! for that system running around it; with several, the lines they
//! printed decide (one naming the device first); with none, a line alone
//! decides when only one agent printed one. Otherwise it is nobody's, and
//! only offered.
//!
//! **Once.** Each boot is acted on once (`handled`): a tile the user
//! closed doesn't come back for the same boot.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use pitwall_core::devices::{Os, Signal, Source};

use super::{Device, Platform, State};

/// `general.deviceAutoOpen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Open beside the agent that started it (only offer one that is
    /// nobody's).
    Beside,
    /// Only offer.
    Offer,
    Off,
}

impl Mode {
    /// The setting's word (anything else: the default, beside).
    pub fn parse(word: &str) -> Mode {
        match word {
            "offer" => Mode::Offer,
            "off" => Mode::Off,
            _ => Mode::Beside,
        }
    }
}

/// A device in a listing, with when it last booted (ms since 1970) when
/// the listing says (simctl does).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub device: Device,
    pub booted_at: Option<u64>,
}

/// What to do about a boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Show it beside `agent`'s terminal (where it is, when its tile is
    /// open already).
    Open { device: Device, agent: String },
    /// Say it started, with "Open in Pitwall" (beside `agent` when it is
    /// someone's).
    Offer { device: Device, agent: Option<String> },
}

/// A signal remembered: first and last seen (ms since 1970).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Noted {
    agent: String,
    source: Source,
    os: Option<Os>,
    device: Option<String>,
    first: u64,
    last: u64,
}

/// How long a signal keeps its platform watched, and counts.
pub const KEEP_MS: u64 = 180_000;
/// A signal counts for a boot up to this long before it…
pub const BEFORE_MS: u64 = 120_000;
/// …and this long after (a line printed once the device is up).
pub const AFTER_MS: u64 = 15_000;
/// An iOS boot this long before its platform's first listing still counts.
pub const EARLY_MS: u64 = 30_000;

fn os_of(p: Platform) -> Os {
    match p {
        Platform::Ios => Os::Ios,
        Platform::Android => Os::Android,
    }
}

fn for_platform(os: Option<Os>, p: Platform) -> bool {
    os.is_none_or(|o| o == os_of(p))
}

/// The open-on-its-own state: the signals, the devices online at the last
/// listing, the boots handled.
#[derive(Debug, Default)]
pub struct Watch {
    noted: Vec<Noted>,
    /// Per platform being watched: when its period began, and the devices
    /// online at its last listing (key → boot id). No entry: not listed
    /// yet in this period.
    since: HashMap<Platform, u64>,
    online: HashMap<Platform, HashMap<String, String>>,
    /// Boots acted on (key → boot id).
    handled: HashMap<String, String>,
    /// Devices an agent showed itself (`show_device`): their next boot
    /// seen here is already handled.
    shown: HashSet<String>,
}

impl Watch {
    /// The agents' signals as seen at `now` (ms since 1970).
    pub fn note(&mut self, signals: &[Signal], now: u64) {
        for s in signals {
            let at = now.saturating_sub(s.age_ms);
            match self.noted.iter_mut().find(|n| n.agent == s.agent && n.source == s.source && n.os == s.os && n.device == s.device) {
                Some(n) => {
                    n.first = n.first.min(at);
                    n.last = n.last.max(at);
                }
                None => self.noted.push(Noted { agent: s.agent.clone(), source: s.source, os: s.os, device: s.device.clone(), first: at, last: at }),
            }
        }
        self.noted.retain(|n| now.saturating_sub(n.last) <= KEEP_MS);
        for p in [Platform::Ios, Platform::Android] {
            let first = self.noted.iter().filter(|n| for_platform(n.os, p)).map(|n| n.first).min();
            match first {
                Some(f) => {
                    self.since.entry(p).or_insert(f);
                }
                None => {
                    self.since.remove(&p);
                    self.online.remove(&p);
                }
            }
        }
    }

    /// Whether platform `p`'s devices are to be listed now.
    pub fn wants(&self, p: Platform) -> bool {
        self.since.contains_key(&p)
    }

    /// An agent showed device `key` itself: its boot isn't acted on again.
    pub fn shown(&mut self, key: &str) {
        self.shown.insert(key.to_string());
    }

    /// Platform `p`'s devices as listed at `now`: the boots among them,
    /// and what to do about each (`mode`).
    pub fn listed(&mut self, p: Platform, seen: &[Seen], now: u64, mode: Mode) -> Vec<Outcome> {
        let Some(&since) = self.since.get(&p) else { return vec![] };
        let before = self.online.get(&p).cloned();
        let mut online = HashMap::new();
        let mut out = Vec::new();
        for s in seen.iter().filter(|s| s.device.platform == p && s.device.state == State::Ready) {
            let key = s.device.key();
            let was = before.as_ref().and_then(|b| b.get(&key));
            // A boot's id: when it booted (iOS says), else when it was
            // first seen online.
            let id = match (s.booted_at, was) {
                (Some(b), _) => b.to_string(),
                (None, Some(w)) => w.clone(),
                (None, None) => format!("seen {now}"),
            };
            let new = match &before {
                // The baseline: only an iOS boot from around when the
                // period began.
                None => s.booted_at.is_some_and(|b| b + EARLY_MS >= since),
                Some(_) => was != Some(&id),
            };
            online.insert(key.clone(), id.clone());
            if !new || self.handled.get(&key) == Some(&id) {
                continue;
            }
            self.handled.insert(key.clone(), id);
            if self.shown.remove(&key) || mode == Mode::Off {
                continue;
            }
            let boot = s.booted_at.unwrap_or(now);
            let agent = self.whose(&s.device, boot);
            out.push(match (mode, agent) {
                (Mode::Beside, Some(agent)) => Outcome::Open { device: s.device.clone(), agent },
                (_, agent) => Outcome::Offer { device: s.device.clone(), agent },
            });
        }
        self.online.insert(p, online);
        out
    }

    /// The agent device `d`, booted at `boot`, is one's (see the module).
    fn whose(&self, d: &Device, boot: u64) -> Option<String> {
        let near = |n: &&Noted| {
            for_platform(n.os, d.platform) && n.first <= boot + AFTER_MS && n.last + BEFORE_MS >= boot && n.device.as_deref().is_none_or(|h| names(h, d))
        };
        let agents = |f: &dyn Fn(&Noted) -> bool| {
            let mut a: Vec<String> = self.noted.iter().filter(near).filter(|n| f(n)).map(|n| n.agent.clone()).collect();
            a.sort();
            a.dedup();
            a
        };
        let launched = agents(&|n| n.source == Source::Process);
        let named = agents(&|n| n.source == Source::Output && n.device.is_some());
        let printed = agents(&|n| n.source == Source::Output);
        let one = |a: Vec<String>| (a.len() == 1).then(|| a[0].clone());
        match launched.len() {
            1 => one(launched),
            0 => one(named.clone()).or_else(|| if named.is_empty() { one(printed) } else { None }),
            _ => {
                let among = |list: &[String]| launched.iter().filter(|a| list.contains(a)).cloned().collect::<Vec<_>>();
                one(among(&named)).or_else(|| one(among(&printed)))
            }
        }
    }
}

/// A name as people and tools write it: lower case, `_` as a space,
/// without what is in brackets ("Pixel_8(AVD) - 14" → "pixel 8").
fn plain(s: &str) -> String {
    let s = s.split('(').next().unwrap_or(s);
    s.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Whether `hint` (a printed name, UDID or serial) may be device `d`.
fn names(hint: &str, d: &Device) -> bool {
    if hint.eq_ignore_ascii_case(&d.id) {
        return true;
    }
    let (h, n) = (plain(hint), plain(&d.name));
    !h.is_empty() && (h == n || (h.len() >= 4 && (n.contains(&h) || h.contains(&n))))
}

// ── show_device ──────────────────────────────────────────────────────────

/// What a listing says about the device asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    One(Device),
    /// It is there but not up yet (or, none named, none is booted): wait.
    Booting(String),
    /// Nothing by that name yet (an emulator shows up once it starts):
    /// wait a little.
    Missing(String),
    /// Can't be: why, with the candidates.
    No(String),
}

fn listed(devices: &[&Device]) -> String {
    devices.iter().map(|d| format!("{} ({})", d.name, d.key())).collect::<Vec<_>>().join(", ")
}

/// The device `asked` names among `devices` (by key, id, name, or name
/// without its runtime), or the one booted device when none is named.
pub fn resolve(asked: Option<&str>, devices: &[Device]) -> Found {
    let ready: Vec<&Device> = devices.iter().filter(|d| d.state == State::Ready).collect();
    let Some(q) = asked.map(str::trim).filter(|q| !q.is_empty()) else {
        return match ready.len() {
            1 => Found::One(ready[0].clone()),
            0 => Found::Booting("no device is booted".into()),
            _ => Found::No(format!("several devices are booted: {}. Name one.", listed(&ready))),
        };
    };
    let exact: Vec<&Device> = devices
        .iter()
        .filter(|d| d.key().eq_ignore_ascii_case(q) || d.id.eq_ignore_ascii_case(q) || d.name.eq_ignore_ascii_case(q) || plain(&d.name) == plain(q))
        .collect();
    let candidates = if exact.is_empty() { devices.iter().filter(|d| plain(&d.name).contains(&plain(q)) && plain(q).len() >= 3).collect() } else { exact };
    let up: Vec<&Device> = candidates.iter().copied().filter(|d| d.state == State::Ready).collect();
    match (up.len(), candidates.len()) {
        (1, _) => Found::One(up[0].clone()),
        (n, _) if n > 1 => Found::No(format!("\"{q}\" could be {}. Name one.", listed(&up))),
        (_, 0) => {
            let all: Vec<&Device> = devices.iter().collect();
            let there = if all.is_empty() { "there are none".to_string() } else { format!("there are: {}", listed(&all)) };
            Found::Missing(format!("no device \"{q}\" ({there})"))
        }
        _ => {
            let why = match &candidates[0].state {
                State::NotReady(w) => w.clone(),
                State::Ready => "not ready".into(),
            };
            Found::Booting(format!("{} is {why}", candidates[0].name))
        }
    }
}

/// How long `show_device` waits.
#[derive(Debug, Clone, Copy)]
pub struct Patience {
    /// For a device still booting (or, none named, for one to boot).
    pub booting: Duration,
    /// For a name nothing has yet.
    pub missing: Duration,
    pub every: Duration,
}

impl Default for Patience {
    fn default() -> Patience {
        Patience { booting: Duration::from_secs(pitwall_proto::device_open::WAIT_SECS), missing: Duration::from_secs(20), every: Duration::from_secs(1) }
    }
}

/// The device `asked` names, listing (`list`) until it is up or patience
/// runs out (`sleep` between looks). Also whether it waited.
pub fn find_waiting(asked: Option<&str>, mut list: impl FnMut() -> Vec<Device>, mut sleep: impl FnMut(Duration), patience: Patience) -> Result<(Device, bool), String> {
    let mut waited = Duration::ZERO;
    loop {
        let found = resolve(asked, &list());
        let limit = match &found {
            Found::One(d) => return Ok((d.clone(), !waited.is_zero())),
            Found::No(why) => return Err(why.clone()),
            Found::Booting(_) => patience.booting,
            Found::Missing(_) => patience.missing,
        };
        if waited >= limit {
            let why = match found {
                Found::Booting(w) | Found::Missing(w) => w,
                _ => unreachable!("returned above"),
            };
            let secs = waited.as_secs();
            return Err(if secs > 0 { format!("{why}, after waiting {secs} s. Boot or start it first (xcrun simctl boot <name>, an Android emulator…), then ask again.") } else { format!("{why}. Boot or start it first (xcrun simctl boot <name>, an Android emulator…), then ask again.") });
        }
        sleep(patience.every);
        waited += patience.every;
    }
}

#[cfg(test)]
mod tests;
