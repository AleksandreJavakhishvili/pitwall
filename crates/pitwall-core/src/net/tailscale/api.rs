//! What the backend asks the local Tailscale: its status (this node, its
//! peers, their users) and `whois` for a connection's source. Behind
//! [`LocalApi`] so tests script a tailnet; [`CliApi`] is the real one.
//!
//! **Why the CLI and not the LocalAPI socket** (tailnet-peers.md §4
//! "Reaching LocalAPI"): the LocalAPI lives somewhere different for every
//! Tailscale flavour (a Unix socket for open-source `tailscaled`, a TCP
//! port plus a token file in a group container for the macOS App Store and
//! standalone apps, a named pipe on Windows), and the `tailscale` CLI
//! already knows each one. A whois costs a process start, so answers are
//! cached (status 5 s, whois 60 s per address, dropped when the address
//! leaves the tailnet); the listener only asks once per new caller address.
//! Talking to the socket directly is a later optimisation behind the same
//! trait.
//!
//! Field names are Tailscale's (`ipnstate.Status`, `apitype.WhoIsResponse`)
//! and were checked against `tailscale` 1.9x output.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{ErrorCode, PwError, Result};
use crate::exec::{Cmd, Exec, LocalExec};
use crate::net::NetState;

/// The local Tailscale, as the backend needs it.
pub trait LocalApi: Send + Sync {
    /// `tailscale status --json`. `Unsupported`: Tailscale isn't installed;
    /// `NotRunning`: it is, but its daemon isn't running.
    fn status(&self) -> Result<Status>;

    /// Who is behind a connection from `addr`. `Ok(None)`: not a node of
    /// this tailnet (or no longer one).
    fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>>;
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
    pub self_: Option<PeerStatus>,
    /// Keyed by node public key.
    #[serde(deserialize_with = "null_default")]
    pub peer: BTreeMap<String, PeerStatus>,
    /// Keyed by user id (as a string).
    #[serde(deserialize_with = "null_default")]
    pub user: BTreeMap<String, UserProfile>,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: String,
    pub current_tailnet: Option<Tailnet>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Tailnet {
    pub name: String,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: String,
}

/// One node in `Status` (`Self` or a `Peer`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PeerStatus {
    /// The node's `StableID`.
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
    /// App capabilities granted to the caller towards this node (§4.8):
    /// capability name → its grants' parameters, as Tailscale passes them.
    #[serde(deserialize_with = "null_default")]
    pub cap_map: BTreeMap<String, Vec<serde_json::Value>>,
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
fn null_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// The login name Tailscale gives the owner of tagged devices.
pub const TAGGED_DEVICES: &str = "tagged-devices";

impl Status {
    /// What Settings → Networks shows for this status.
    pub fn net_state(&self) -> NetState {
        match self.backend_state.as_str() {
            "Running" if self.self_.is_some() => NetState::Ready,
            "Running" | "Starting" => NetState::Connecting,
            "NeedsLogin" | "NeedsMachineAuth" => NetState::SignedOut,
            "Stopped" | "NoState" | "" => NetState::Off,
            other => NetState::Error { message: format!("Tailscale is {other}") },
        }
    }

    /// The tailnet's name in ids (`example.ts.net`): its MagicDNS suffix,
    /// stable for the tailnet's life; the tailnet's own name otherwise.
    pub fn tailnet_name(&self) -> Option<String> {
        let tailnet = self.current_tailnet.as_ref();
        [self.magic_dns_suffix.as_str(), tailnet.map_or("", |t| t.magic_dns_suffix.as_str()), tailnet.map_or("", |t| t.name.as_str())]
            .into_iter()
            .map(|s| s.trim_end_matches('.'))
            .find(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// A peer by its `StableID`.
    pub fn peer(&self, stable_id: &str) -> Option<&PeerStatus> {
        self.peer.values().find(|p| p.id == stable_id)
    }

    pub fn user(&self, id: i64) -> Option<&UserProfile> {
        self.user.get(&id.to_string())
    }
}

impl PeerStatus {
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

// --------------------------------------------------------------- the CLI

/// [`LocalApi`] through the `tailscale` CLI, run on this machine.
pub struct CliApi {
    exec: Arc<dyn Exec>,
    program: Option<String>,
}

/// Where the CLI is when it isn't on PATH: inside the macOS app (App Store
/// and standalone), and the Windows installer's folder.
const CLI_FALLBACKS: &[&str] = &[
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    r"C:\Program Files\Tailscale\tailscale.exe",
];

/// Long enough for a busy daemon, short enough for a listener's accept.
const CLI_TIMEOUT: Duration = Duration::from_secs(5);

impl CliApi {
    /// The `tailscale` CLI on PATH, else in its usual install places.
    pub fn new() -> CliApi {
        let program = crate::platform::which("tailscale")
            .or_else(|| CLI_FALLBACKS.iter().find(|p| Path::new(p).is_file()).map(|p| p.to_string()));
        CliApi { exec: Arc::new(LocalExec), program }
    }

    /// A given program through a given exec (tests of the parsing).
    pub fn with(exec: Arc<dyn Exec>, program: &str) -> CliApi {
        CliApi { exec, program: Some(program.to_string()) }
    }

    fn run(&self, args: &[&str]) -> Result<crate::exec::Out> {
        let program = self.program.as_deref().ok_or_else(|| PwError::unsupported("Tailscale isn't installed"))?;
        let mut argv = vec![program];
        argv.extend_from_slice(args);
        self.exec.run(&Cmd::new(&argv).timeout(CLI_TIMEOUT))
    }
}

impl Default for CliApi {
    fn default() -> Self {
        CliApi::new()
    }
}

impl LocalApi for CliApi {
    fn status(&self) -> Result<Status> {
        let out = self.run(&["status", "--json"])?;
        // `status` exits non-zero when logged out or stopped but still
        // prints JSON; only no JSON at all means no daemon.
        match serde_json::from_slice::<Status>(&out.stdout) {
            Ok(status) => Ok(status),
            Err(_) if !out.ok() => Err(PwError::new(ErrorCode::NotRunning, format!("Tailscale isn't running: {}", out.stderr_text()))),
            Err(e) => Err(PwError::other(format!("tailscale status: {e}"))),
        }
    }

    fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>> {
        let target = addr.to_string();
        let out = self.run(&["whois", "--json", &target])?;
        if !out.ok() {
            // "no match for IP:port": not a tailnet node.
            return Ok(None);
        }
        serde_json::from_slice(&out.stdout).map(Some).map_err(|e| PwError::other(format!("tailscale whois: {e}")))
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
