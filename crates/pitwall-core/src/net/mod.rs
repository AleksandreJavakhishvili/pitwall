//! Networks, as collaboration sees them (docs/spec/collaboration/network.md,
//! §15). Collaboration never talks to Tailscale or any other network
//! directly: who is calling, how to reach a device, who is around and which
//! groups a person is in all come through [`Network`]. Each network is a
//! backend behind it:
//!
//! - `tailscale` (v1): devices on the user's tailnet, identity from `whois`;
//! - `cloud` (later): Pitwall accounts and a relay, identity from signed
//!   tokens;
//! - `fake` (tests, `net::fake`): in-process, scripted people and devices.
//!
//! Backend modules live under `net::` and nothing outside `net` names them;
//! `collab` in particular depends only on this module's traits and types
//! (§15.8, checked by `tests/boundaries.rs`).
//!
//! What every backend must guarantee (§15.4), and what the rest of Pitwall
//! relies on:
//! - a request reaches a [`Handler`] only through [`Network::serve`], and
//!   only with a [`Caller`] the backend proved; nothing in the request
//!   (headers, body) can change it;
//! - removing a person or device at the provider stops their requests;
//! - identities are stable (`subject`, never a display name).

mod key;
mod probe;
mod transport;

#[cfg(any(test, feature = "testing"))]
pub mod fake;
pub mod tailscale;

#[cfg(test)]
mod tests;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub use self::key::{InstanceKey, KEY_PREFIX};
pub use self::probe::{ProbeLimits, Probed, Prober, INFO_PATH};
pub use self::transport::{Body, BodyWriter, HttpClient, Request, Response, Timeouts};
use crate::error::Result;
/// The `http` crate (request and response types), so users of this module
/// needn't depend on it.
pub use ::http;
pub use pitwall_proto::collab::{DeviceAddr, GroupRef, Identity, NetworkId, PersonId};

/// What a backend hands the core with every inbound request (§15.2): who
/// the network proved is calling. Only a backend can make one (the
/// constructor is private to `net`), and it can't be deserialized, so
/// nothing outside a backend can forge one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Caller {
    identity: Identity,
    person: PersonId,
    device: Option<DeviceAddr>,
    network: NetworkId,
    groups: Vec<GroupRef>,
}

impl Caller {
    /// For backends: `person` comes from [`Handler::person_of`].
    #[allow(dead_code)] // used by the backends, and only the Fake one is built everywhere
    pub(in crate::net) fn new(
        identity: Identity,
        person: PersonId,
        device: Option<DeviceAddr>,
        network: NetworkId,
        groups: Vec<GroupRef>,
    ) -> Caller {
        Caller { identity, person, device, network, groups }
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The person the identity belongs to (linked identities share one).
    pub fn person(&self) -> &PersonId {
        &self.person
    }

    /// The device the request came from, when the network names one (a
    /// tailnet node). Whether it runs Pitwall is not the network's business:
    /// team records list Pitwall devices by key and address (§4.2).
    pub fn device(&self) -> Option<&DeviceAddr> {
        self.device.as_ref()
    }

    /// The network the request arrived on.
    pub fn network(&self) -> &NetworkId {
        &self.network
    }

    /// Provider-managed groups the identity is in (§4.8).
    pub fn groups(&self) -> &[GroupRef] {
        &self.groups
    }
}

/// Collaboration's side of [`Network::serve`]: one handler for every
/// network (§8.1). Called on the backend's threads, one request at a time
/// per thread; the response body may stream.
pub trait Handler: Send + Sync {
    fn handle(&self, req: Request, caller: Caller) -> Response;

    /// The person an identity is, for the [`Caller`] the backend builds.
    /// Linking identities is the owner's (and team records') business
    /// (§15.7), so the backend asks; unlinked identities are their own
    /// person.
    fn person_of(&self, identity: &Identity) -> PersonId {
        PersonId::of(identity)
    }
}

impl<F> Handler for F
where
    F: Fn(Request, Caller) -> Response + Send + Sync,
{
    fn handle(&self, req: Request, caller: Caller) -> Response {
        self(req, caller)
    }
}

/// One network backend instance (§15.3). Blocking; implementations are
/// shared across threads.
pub trait Network: Send + Sync {
    fn id(&self) -> NetworkId;

    fn state(&self) -> NetState;

    /// This device's identity and address here, once signed in.
    fn me(&self) -> Option<(Identity, DeviceAddr)>;

    /// Inbound. The backend authenticates every request *before* calling
    /// the handler; a request without a proven [`Caller`] never reaches it
    /// (on Tailscale the connection is closed without a byte). Serving
    /// stops when the handle is dropped or stopped.
    fn serve(&self, handler: Arc<dyn Handler>) -> Result<ServeHandle>;

    /// Outbound HTTP to a device on this network, with this device's
    /// credentials attached and the route chosen by the backend.
    fn client(&self, to: &DeviceAddr) -> Result<Box<dyn HttpClient>>;

    /// What others use to reach this device: the base URL of the handler
    /// (`http://studio.example.ts.net:47421`, no trailing slash) and how a
    /// plain client authenticates. The fronts build the MCP setup line, the
    /// Agent Card's `securitySchemes` and MCP's protected resource metadata
    /// from it.
    fn endpoint(&self) -> Endpoint;

    /// Devices that may run Pitwall, for team discovery (§3). Candidates
    /// only: the caller probes them.
    fn discover(&self) -> Result<Vec<PeerHint>>;

    /// Provider-managed groups of an identity on this network.
    fn groups(&self, who: &Identity) -> Result<Vec<GroupRef>>;

    /// People and devices the provider removed, as it learns of them. Each
    /// call subscribes anew; the backend refuses their requests anyway.
    fn revocations(&self) -> Receiver<Revocation>;
}

/// How a network is doing, for Settings → Networks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NetState {
    /// Not running here (Tailscale not installed or stopped; Cloud off).
    Off,
    SignedOut,
    Connecting,
    Ready,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub base_url: String,
    pub client_auth: ClientAuth,
}

/// How a plain MCP or A2A client authenticates to this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAuth {
    /// The network proves who is calling (Tailscale).
    None,
    /// OAuth through the network's issuer (Pitwall Cloud).
    OAuth { issuer: String, resource: String, scopes: Vec<String> },
}

/// A device that may run Pitwall (§3), from [`Network::discover`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHint {
    pub addr: DeviceAddr,
    /// Who it belongs to, when the network says (tagged devices: nobody).
    pub owner: Option<Identity>,
    /// Its name on the network (`studio`).
    pub label: String,
    pub online: bool,
}

/// Someone the provider removed (§15.4): their requests are refused from
/// now on; collaboration drops what it holds for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "what", rename_all = "snake_case")]
pub enum Revocation {
    Identity { identity: Identity },
    Device { device: DeviceAddr },
}

/// Serving on one network; stops on [`ServeHandle::stop`] or drop.
pub struct ServeHandle {
    stop: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl ServeHandle {
    /// For backends: `stop` closes the listener (or tunnel) and runs once.
    pub fn new(stop: impl FnOnce() + Send + 'static) -> ServeHandle {
        ServeHandle { stop: Mutex::new(Some(Box::new(stop))) }
    }

    pub fn stop(&self) {
        let stop = self.stop.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(stop) = stop {
            stop();
        }
    }
}

impl Drop for ServeHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl std::fmt::Debug for ServeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServeHandle")
    }
}
