//! What peers ask the local Tailscale (tailnet-peers.md §3, §4): its status
//! (this node, its peers, their users) and `whois` for a connection's
//! source. Behind [`Tailnet`] so tests script one ([`crate::fake`]);
//! [`TailscaleCli`] is the real one.
//!
//! Through the `tailscale` CLI, not the LocalAPI socket: the socket lives
//! somewhere different for every Tailscale flavour (open-source
//! `tailscaled`, the macOS App Store and standalone apps, Windows), and the
//! CLI knows each. A call costs a process start, so answers are cached
//! (status 5 s, whois 60 s per address).
//!
//! Field names are Tailscale's (`ipnstate.Status`, `apitype.WhoIsResponse`),
//! checked against `tailscale` 1.9x output. Ported from the parked
//! collaboration branch's `net::tailscale::api`.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use pitwall_core::exec::{Cmd, Exec, LocalExec};
use pitwall_proto::{code, ErrorBody};

use crate::lock;

/// The local Tailscale, as peers need it.
pub trait Tailnet: Send + Sync {
    /// `tailscale status --json`. `unsupported`: Tailscale isn't installed;
    /// `not_running`: it is, but its daemon isn't running.
    fn status(&self) -> Result<Status, ErrorBody>;

    /// Ask again next time, not a cached status (a user's Refresh).
    fn forget(&self) {}

    /// Who is behind a connection from `addr`. `Ok(None)`: not a node of
    /// this tailnet (or no longer one).
    fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>, ErrorBody>;

    /// Where the listener listens: this node's Tailscale addresses, never
    /// `0.0.0.0`, `::` or a LAN address.
    fn bind_ips(&self, me: &Node) -> Vec<IpAddr> {
        me.tailscale_ips.iter().copied().filter(|ip| is_tailnet_ip(*ip)).collect()
    }

    /// Whether a connection from `ip` may come from the tailnet.
    fn address_ok(&self, ip: IpAddr) -> bool {
        is_tailnet_ip(ip)
    }

    /// Where `node` listens on `port` (its IPv4 address first).
    fn dial_addr(&self, node: &Node, port: u16) -> Option<SocketAddr> {
        let ips = node.tailscale_ips.iter().filter(|ip| is_tailnet_ip(**ip));
        let ip = ips.clone().find(|ip| ip.is_ipv4()).or_else(|| ips.clone().next())?;
        Some(SocketAddr::new(*ip, port))
    }

    /// The listener now listens on `addr` (the fake learns ports this way).
    fn listening(&self, _addr: SocketAddr) {}

    /// A connection to a peer was opened from `local`, before a byte was
    /// sent (the fake's `whois` learns callers this way).
    fn dialed(&self, _local: SocketAddr) {}
}

// ------------------------------------------------------------------ shapes

/// `ipnstate.Status`, the parts used.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Status {
    /// `Running`, `NeedsLogin`, `NeedsMachineAuth`, `Starting`, `Stopped`,
    /// `NoState`.
    pub backend_state: String,
    #[serde(rename = "Self")]
    pub self_: Option<Node>,
    /// Keyed by node public key.
    #[serde(deserialize_with = "null_default")]
    pub peer: BTreeMap<String, Node>,
    /// Keyed by user id (as a string).
    #[serde(deserialize_with = "null_default")]
    pub user: BTreeMap<String, UserProfile>,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: String,
    /// False in userspace-networking mode (no tun device): nothing can
    /// bind to or reach tailnet addresses.
    #[serde(rename = "TUN")]
    pub tun: Option<bool>,
}

/// One node in `Status` (`Self` or a `Peer`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Node {
    /// The node's `StableID` (survives key rotation and renames).
    #[serde(rename = "ID")]
    pub id: String,
    pub host_name: String,
    /// `studio.example.ts.net.` (trailing dot).
    #[serde(rename = "DNSName")]
    pub dns_name: String,
    #[serde(rename = "OS")]
    pub os: String,
    #[serde(rename = "UserID")]
    pub user_id: i64,
    #[serde(rename = "TailscaleIPs", deserialize_with = "null_default")]
    pub tailscale_ips: Vec<IpAddr>,
    #[serde(deserialize_with = "null_default")]
    pub tags: Vec<String>,
    pub online: bool,
}

/// `tailcfg.UserProfile`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserProfile {
    #[serde(rename = "ID")]
    pub id: i64,
    pub login_name: String,
    pub display_name: String,
}

/// `apitype.WhoIsResponse`, the parts used.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct WhoIs {
    pub node: WhoIsNode,
    pub user_profile: UserProfile,
}

/// `tailcfg.Node`, the parts used.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct WhoIsNode {
    #[serde(rename = "StableID")]
    pub stable_id: String,
    /// `studio.example.ts.net.`
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub tags: Vec<String>,
}

/// Tailscale writes `null` for empty maps and lists.
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// The login name Tailscale gives the owner of tagged devices.
pub const TAGGED_DEVICES: &str = "tagged-devices";

/// What Settings → Peers says about Tailscale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TailnetState {
    Ready,
    NotInstalled,
    NotRunning,
    NeedsLogin,
    /// Userspace networking (no tun device): Pitwall can't use it.
    Userspace,
}

impl Status {
    pub fn state(&self) -> TailnetState {
        match self.backend_state.as_str() {
            "Running" if self.tun == Some(false) => TailnetState::Userspace,
            "Running" if self.self_.is_some() => TailnetState::Ready,
            "NeedsLogin" | "NeedsMachineAuth" => TailnetState::NeedsLogin,
            _ => TailnetState::NotRunning,
        }
    }

    /// This node, when Tailscale is up.
    pub fn me(&self) -> Result<&Node, ErrorBody> {
        match (self.state(), &self.self_) {
            (TailnetState::Ready, Some(me)) => Ok(me),
            (TailnetState::Userspace, _) => Err(ErrorBody::new(code::UNSUPPORTED, "Tailscale runs in userspace mode; Pitwall can't use it here")),
            (TailnetState::NeedsLogin, _) => Err(ErrorBody::new(code::NOT_RUNNING, "Tailscale needs you to log in")),
            _ => Err(ErrorBody::new(code::NOT_RUNNING, "Tailscale isn't running")),
        }
    }

    /// A peer by its `StableID`.
    pub fn node(&self, stable_id: &str) -> Option<&Node> {
        self.peer.values().find(|p| p.id == stable_id)
    }

    pub fn user(&self, id: i64) -> Option<&UserProfile> {
        self.user.get(&id.to_string())
    }

    /// The login of a node's owner (empty when unknown).
    pub fn login_of(&self, node: &Node) -> String {
        self.user(node.user_id).map(|u| u.login_name.clone()).unwrap_or_default()
    }

    /// The user's own other machines: peers of this node's user, not
    /// tagged (online or not).
    pub fn own_machines(&self) -> Vec<&Node> {
        let Some(me) = &self.self_ else { return Vec::new() };
        self.peer.values().filter(|p| p.user_id == me.user_id && !p.tagged()).collect()
    }

    /// A peer by `StableID`, MagicDNS name (`studio`, `studio.x.ts.net`),
    /// host name or tailnet IP.
    pub fn find(&self, name: &str) -> Option<&Node> {
        let name = name.trim().trim_end_matches('.');
        let ip = name.parse::<IpAddr>().ok();
        self.peer.values().find(|p| {
            p.id == name || p.dns() == name || p.label() == name || p.host_name.eq_ignore_ascii_case(name) || ip.is_some_and(|ip| p.tailscale_ips.contains(&ip))
        })
    }
}

impl Node {
    /// The short name (`studio`), from MagicDNS, else the host name.
    pub fn label(&self) -> String {
        self.dns_name.split('.').next().filter(|s| !s.is_empty()).unwrap_or(&self.host_name).to_string()
    }

    /// The MagicDNS name without the trailing dot.
    pub fn dns(&self) -> &str {
        self.dns_name.trim_end_matches('.')
    }

    pub fn tagged(&self) -> bool {
        !self.tags.is_empty()
    }
}

impl WhoIs {
    pub fn tagged(&self) -> bool {
        !self.node.tags.is_empty() || self.user_profile.login_name == TAGGED_DEVICES
    }

    /// A machine of the same Tailscale user as `me` (in `status`): same
    /// user id and login, not a tagged device.
    pub fn same_user(&self, status: &Status, me: &Node) -> bool {
        let login = status.login_of(me);
        !self.tagged() && self.user_profile.id == me.user_id && !login.is_empty() && self.user_profile.login_name == login
    }

    /// The short name from the node's MagicDNS name.
    pub fn label(&self) -> String {
        self.node.name.split('.').next().unwrap_or_default().to_string()
    }
}

/// Whether `ip` is in Tailscale's address ranges: `100.64.0.0/10` (CGNAT)
/// or `fd7a:115c:a1e0::/48`.
pub fn is_tailnet_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (o[1] & 0xC0) == 64
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_tailnet_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

// --------------------------------------------------------------- the CLI

/// [`Tailnet`] through the `tailscale` CLI on this machine.
pub struct TailscaleCli {
    exec: Arc<dyn Exec>,
    program: Option<String>,
    status: Mutex<Option<(Instant, Result<Status, ErrorBody>)>>,
    whois: Mutex<HashMap<SocketAddr, (Instant, Option<WhoIs>)>>,
}

/// Where the CLI is when it isn't on PATH: inside the macOS app (App Store
/// and standalone), and the Windows installer's folder.
const CLI_FALLBACKS: &[&str] = &["/Applications/Tailscale.app/Contents/MacOS/Tailscale", r"C:\Program Files\Tailscale\tailscale.exe"];

/// Long enough for a busy daemon, short enough for a listener's accept.
const CLI_TIMEOUT: Duration = Duration::from_secs(5);
const STATUS_TTL: Duration = Duration::from_secs(5);
const WHOIS_TTL: Duration = Duration::from_secs(60);

/// The CLI's environment. The standalone macOS app's binary tells the CLI
/// from its GUI by `TERM` (or `SHLVL`): a Pitwall opened from the Dock has
/// neither, and `status --json` then tries to start the GUI and prints an
/// error instead of JSON. A `TERM` already set is kept.
fn cli_env(term_set: bool) -> &'static [(&'static str, &'static str)] {
    if term_set {
        &[]
    } else {
        &[("TERM", "dumb")]
    }
}

/// `program` on PATH.
fn on_path(program: &str) -> Option<PathBuf> {
    let name = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&name)).find(|p| p.is_file())
}

impl TailscaleCli {
    /// The `tailscale` CLI on PATH, else in its usual install places.
    pub fn new() -> TailscaleCli {
        let program = on_path("tailscale")
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| CLI_FALLBACKS.iter().find(|p| Path::new(p).is_file()).map(|p| p.to_string()));
        TailscaleCli::with(Arc::new(LocalExec), program)
    }

    /// A given program through a given exec (tests of the parsing).
    pub fn with(exec: Arc<dyn Exec>, program: Option<String>) -> TailscaleCli {
        TailscaleCli { exec, program, status: Mutex::new(None), whois: Mutex::new(HashMap::new()) }
    }

    fn run(&self, args: &[&str]) -> Result<pitwall_core::exec::Out, ErrorBody> {
        let program = self.program.as_deref().ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "Tailscale isn't installed"))?;
        let mut argv = vec![program];
        argv.extend_from_slice(args);
        let env = cli_env(std::env::var_os("TERM").is_some());
        self.exec.run(&Cmd::new(&argv).env(env).timeout(CLI_TIMEOUT)).map_err(|e| ErrorBody::new(code::OTHER, format!("tailscale: {e}")))
    }


    fn fetch_status(&self) -> Result<Status, ErrorBody> {
        let out = self.run(&["status", "--json"])?;
        // `status` exits non-zero when logged out or stopped but still
        // prints JSON; only no JSON at all means no daemon.
        match serde_json::from_slice::<Status>(&out.stdout) {
            Ok(status) => Ok(status),
            Err(_) if !out.ok() => Err(ErrorBody::new(code::NOT_RUNNING, format!("Tailscale isn't running: {}", out.stderr_text()))),
            Err(e) => Err(ErrorBody::new(code::OTHER, format!("tailscale status: {e}"))),
        }
    }
}

impl Default for TailscaleCli {
    fn default() -> Self {
        TailscaleCli::new()
    }
}

impl Tailnet for TailscaleCli {
    fn status(&self) -> Result<Status, ErrorBody> {
        let mut cached = lock(&self.status);
        if let Some((at, s)) = cached.as_ref() {
            if at.elapsed() < STATUS_TTL {
                return s.clone();
            }
        }
        let s = self.fetch_status();
        *cached = Some((Instant::now(), s.clone()));
        s
    }

    fn forget(&self) {
        *lock(&self.status) = None;
    }

    fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>, ErrorBody> {
        if let Some((at, w)) = lock(&self.whois).get(&addr) {
            if at.elapsed() < WHOIS_TTL {
                return Ok(w.clone());
            }
        }
        let target = addr.to_string();
        let out = self.run(&["whois", "--json", &target])?;
        // "peer not found": not a tailnet node.
        let w = if out.ok() { Some(serde_json::from_slice(&out.stdout).map_err(|e| ErrorBody::new(code::OTHER, format!("tailscale whois: {e}")))?) } else { None };
        let mut cache = lock(&self.whois);
        cache.retain(|_, (at, _)| at.elapsed() < WHOIS_TTL);
        cache.insert(addr, (Instant::now(), w.clone()));
        Ok(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::testing::FakeExec;

    /// `tailscale status --json` as 1.9x prints it (trimmed; made-up nodes).
    const STATUS: &str = r#"{
      "Version": "1.90.0", "TUN": true, "BackendState": "Running", "MagicDNSSuffix": "example.ts.net",
      "Self": {"ID": "nLAPTOP", "HostName": "laptop", "DNSName": "laptop.example.ts.net.", "OS": "macOS",
               "UserID": 1, "TailscaleIPs": ["100.64.0.1", "fd7a:115c:a1e0::1"], "Tags": null, "Online": true},
      "Peer": {
        "nodekey:a": {"ID": "nSTUDIO", "HostName": "Studio", "DNSName": "studio.example.ts.net.", "OS": "macOS",
                      "UserID": 1, "TailscaleIPs": ["fd7a:115c:a1e0::2", "100.64.0.2"], "Online": true},
        "nodekey:b": {"ID": "nCI", "HostName": "ci", "DNSName": "ci.example.ts.net.", "OS": "linux",
                      "UserID": 9, "TailscaleIPs": ["100.64.0.3"], "Tags": ["tag:ci"], "Online": true},
        "nodekey:c": {"ID": "nBOB", "HostName": "bobs", "DNSName": "bobs.example.ts.net.", "OS": "windows",
                      "UserID": 2, "TailscaleIPs": ["100.64.0.4"], "Online": false}
      },
      "User": {"1": {"ID": 1, "LoginName": "alice@example.com", "DisplayName": "Alice"},
               "2": {"ID": 2, "LoginName": "bob@example.com", "DisplayName": "Bob"},
               "9": {"ID": 9, "LoginName": "tagged-devices", "DisplayName": "Tagged Devices"}}
    }"#;

    #[test]
    fn parses_status_and_finds_own_machines() {
        let s: Status = serde_json::from_str(STATUS).unwrap();
        assert_eq!(s.state(), TailnetState::Ready);
        let me = s.me().unwrap();
        assert_eq!((me.label().as_str(), s.login_of(me).as_str()), ("laptop", "alice@example.com"));
        let own: Vec<_> = s.own_machines().iter().map(|n| n.id.clone()).collect();
        assert_eq!(own, ["nSTUDIO"], "tagged devices and other users are left out");
        let studio = s.find("studio").unwrap();
        assert_eq!(s.find("studio.example.ts.net").unwrap().id, studio.id);
        assert_eq!(s.find("100.64.0.2").unwrap().id, studio.id);
        assert_eq!(s.find("nSTUDIO").unwrap().id, studio.id);
        assert!(s.find("nowhere").is_none());
        let cli = TailscaleCli::with(FakeExec::new(), None);
        assert_eq!(cli.dial_addr(studio, 47420), Some("100.64.0.2:47420".parse().unwrap()), "IPv4 first");
    }

    #[test]
    fn states_from_the_backend() {
        let state = |json: &str| serde_json::from_str::<Status>(json).unwrap().state();
        assert_eq!(state(r#"{"BackendState":"NeedsLogin","Self":null,"Peer":null,"User":null}"#), TailnetState::NeedsLogin);
        assert_eq!(state(r#"{"BackendState":"Stopped"}"#), TailnetState::NotRunning);
        assert_eq!(state(r#"{"BackendState":"Running","TUN":false,"Self":{"ID":"n"}}"#), TailnetState::Userspace);
    }

    #[test]
    fn whois_tells_the_same_user_from_others_and_tags() {
        let s: Status = serde_json::from_str(STATUS).unwrap();
        let me = s.me().unwrap();
        let w = |id: i64, login: &str, tags: &[&str]| WhoIs {
            node: WhoIsNode { stable_id: "n".into(), name: "x.example.ts.net.".into(), tags: tags.iter().map(|t| t.to_string()).collect() },
            user_profile: UserProfile { id, login_name: login.into(), display_name: String::new() },
        };
        assert!(w(1, "alice@example.com", &[]).same_user(&s, me));
        assert!(!w(2, "bob@example.com", &[]).same_user(&s, me));
        assert!(!w(1, "bob@example.com", &[]).same_user(&s, me), "id and login both");
        assert!(!w(1, "alice@example.com", &["tag:ci"]).same_user(&s, me));
        assert!(!w(9, TAGGED_DEVICES, &[]).same_user(&s, me));
        let parsed: WhoIs = serde_json::from_str(
            r#"{"Node":{"ID":5,"StableID":"nSTUDIO","Name":"studio.example.ts.net.","Tags":null},"UserProfile":{"ID":1,"LoginName":"alice@example.com"},"CapMap":null}"#,
        )
        .unwrap();
        assert_eq!((parsed.node.stable_id.as_str(), parsed.label().as_str()), ("nSTUDIO", "studio"));
    }

    #[test]
    fn tailnet_ranges_only() {
        for ip in ["100.64.0.1", "100.127.255.254", "fd7a:115c:a1e0::5", "::ffff:100.100.1.1"] {
            assert!(is_tailnet_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["100.63.0.1", "100.128.0.1", "192.168.1.2", "0.0.0.0", "::", "127.0.0.1", "fd7a:115c:a1e1::1"] {
            assert!(!is_tailnet_ip(ip.parse().unwrap()), "{ip}");
        }
        // The listener binds only those (never 0.0.0.0, :: or a LAN address).
        let me = Node { tailscale_ips: ["0.0.0.0", "::", "192.168.1.2", "100.64.0.1", "fd7a:115c:a1e0::1"].iter().map(|s| s.parse().unwrap()).collect(), ..Node::default() };
        let cli = TailscaleCli::with(FakeExec::new(), None);
        assert_eq!(cli.bind_ips(&me), ["100.64.0.1".parse::<IpAddr>().unwrap(), "fd7a:115c:a1e0::1".parse().unwrap()]);
    }

    #[test]
    fn the_cli_is_run_and_cached() {
        let exec = FakeExec::new();
        exec.on(&["tailscale", "status", "--json"], STATUS);
        exec.on_exit(&["tailscale", "whois", "--json", "100.64.0.9:5000"], 1, "", "peer not found");
        let cli = TailscaleCli::with(exec.clone(), Some("tailscale".into()));
        assert_eq!(cli.status().unwrap().self_.unwrap().id, "nLAPTOP");
        cli.status().unwrap();
        assert_eq!(cli.whois("100.64.0.9:5000".parse().unwrap()).unwrap(), None);
        cli.whois("100.64.0.9:5000".parse().unwrap()).unwrap();
        assert_eq!(exec.calls().len(), 2, "both answers cached");
        // Opened from the Dock (no TERM): both get one, so the standalone
        // macOS binary answers as the CLI; a TERM already set is kept.
        let term = cli_env(std::env::var_os("TERM").is_some());
        assert!(exec.calls().iter().all(|c| c.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).eq(term.iter().copied())));
        assert_eq!(cli_env(false), [("TERM", "dumb")]);
        assert!(cli_env(true).is_empty());
        let missing = TailscaleCli::with(exec, None);
        assert_eq!(missing.status().unwrap_err().code, code::UNSUPPORTED);
    }

    #[test]
    fn no_json_and_a_failure_is_not_running_with_why() {
        let exec = FakeExec::new();
        let gui = "The Tailscale GUI failed to start (Tailscale.CLIError error 3.)";
        exec.on_exit(&["tailscale", "status", "--json"], 1, "", gui);
        let cli = TailscaleCli::with(exec.clone(), Some("tailscale".into()));
        let e = cli.status().unwrap_err();
        assert_eq!(e.code, code::NOT_RUNNING);
        assert!(e.message.contains(gui), "{}", e.message);
        // Cached until forgotten (Settings → Peers' Refresh).
        let _ = cli.status();
        assert_eq!(exec.calls().len(), 1);
        cli.forget();
        let _ = cli.status();
        assert_eq!(exec.calls().len(), 2);
    }
}
