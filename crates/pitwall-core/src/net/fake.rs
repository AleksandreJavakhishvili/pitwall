//! The Fake network (§15.4 "Fake"): in-process, scripted people, devices and
//! groups, for every collaboration test. A [`FakeNet`] is one network (a
//! made-up tailnet); [`FakeNet::device`] signs a device in as a person and
//! returns its [`FakeNetwork`], the [`Network`] that device's Pitwall uses.
//! A request from one device's [`Network::client`] is handed, on the
//! calling thread, to the handler another device serves, with the
//! [`Caller`] this network proves: the sender's scripted identity, device
//! and groups, whatever the request says.
//!
//! Built for this crate's tests and, with the `testing` feature, for other
//! crates' tests; never in a release build.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    Caller, ClientAuth, DeviceAddr, Endpoint, GroupRef, Handler, HttpClient, Identity, NetState, Network, NetworkId,
    PeerHint, PersonId, Request, Response, Revocation, ServeHandle,
};
use crate::error::{ErrorCode, PwError, Result};

/// The provider name of Fake identities and network ids (`fake:<name>`).
pub const PROVIDER: &str = "fake";

/// One fake network, shared by every device on it. Cheap to clone.
#[derive(Clone)]
pub struct FakeNet {
    id: NetworkId,
    world: Arc<Mutex<World>>,
}

#[derive(Default)]
struct World {
    people: BTreeMap<String, Person>,
    devices: BTreeMap<String, Device>,
    subscribers: Vec<Sender<Revocation>>,
    next_serve: u64,
}

struct Person {
    identity: Identity,
    groups: BTreeSet<String>,
    revoked: bool,
}

struct Device {
    owner: String,
    online: bool,
    revoked: bool,
    state: NetState,
    client_auth: ClientAuth,
    serving: Option<(u64, Arc<dyn Handler>)>,
}

impl FakeNet {
    /// A network with id `fake:<name>`.
    pub fn new(name: &str) -> FakeNet {
        FakeNet { id: NetworkId::new(PROVIDER, name), world: Arc::default() }
    }

    pub fn id(&self) -> NetworkId {
        self.id.clone()
    }

    fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A person on this network (added on first use), by login. Plain MCP
    /// users are people without a device of their own that serves.
    pub fn person(&self, login: &str) -> Identity {
        let mut world = self.world();
        world
            .people
            .entry(login.to_string())
            .or_insert_with(|| Person {
                identity: Identity {
                    email: login.contains('@').then(|| login.to_string()),
                    ..Identity::new(PROVIDER, login)
                },
                groups: BTreeSet::new(),
                revoked: false,
            })
            .identity
            .clone()
    }

    /// Sign device `node` in as `login` (both added on first use; a device
    /// signed in again changes owner). Online and ready.
    pub fn device(&self, login: &str, node: &str) -> FakeNetwork {
        self.person(login);
        self.world().devices.insert(
            node.to_string(),
            Device {
                owner: login.to_string(),
                online: true,
                revoked: false,
                state: NetState::Ready,
                client_auth: ClientAuth::None,
                serving: None,
            },
        );
        FakeNetwork { net: self.clone(), node: node.to_string() }
    }

    /// A device's address on this network.
    pub fn addr(&self, node: &str) -> DeviceAddr {
        DeviceAddr { network: self.id.clone(), node: node.to_string() }
    }

    /// A provider group on this network.
    pub fn group(&self, id: &str) -> GroupRef {
        GroupRef { network: self.id.clone(), id: id.to_string() }
    }

    pub fn join_group(&self, login: &str, group: &str) {
        self.person(login);
        if let Some(p) = self.world().people.get_mut(login) {
            p.groups.insert(group.to_string());
        }
    }

    pub fn leave_group(&self, login: &str, group: &str) {
        if let Some(p) = self.world().people.get_mut(login) {
            p.groups.remove(group);
        }
    }

    /// The provider removes a person: every request from their devices is
    /// refused from now on, and subscribers hear of it.
    pub fn revoke_person(&self, login: &str) {
        let mut world = self.world();
        let Some(p) = world.people.get_mut(login) else { return };
        p.revoked = true;
        let identity = p.identity.clone();
        world.announce(Revocation::Identity { identity });
    }

    /// The provider removes a device (lost or stolen).
    pub fn revoke_device(&self, node: &str) {
        let mut world = self.world();
        let Some(d) = world.devices.get_mut(node) else { return };
        d.revoked = true;
        let device = self.addr(node);
        world.announce(Revocation::Device { device });
    }

    /// A device goes offline (or back): requests to and from it fail.
    pub fn set_online(&self, node: &str, online: bool) {
        if let Some(d) = self.world().devices.get_mut(node) {
            d.online = online;
        }
    }

    /// What this network would prove for a request from `node` right now,
    /// for tests that call the core without HTTP. The person is the
    /// unlinked default ([`PersonId::of`]).
    pub fn caller(&self, node: &str) -> Result<Caller> {
        let world = self.world();
        let (identity, groups) = world.sender(node)?;
        let person = PersonId::of(&identity);
        Ok(Caller::new(identity, person, Some(self.addr(node)), self.id.clone(), self.groups_of(&groups)))
    }

    fn groups_of(&self, ids: &BTreeSet<String>) -> Vec<GroupRef> {
        ids.iter().map(|g| self.group(g)).collect()
    }
}

impl World {
    fn announce(&mut self, revocation: Revocation) {
        self.subscribers.retain(|tx| tx.send(revocation.clone()).is_ok());
    }

    /// The identity and groups behind a request from `node`, or the refusal
    /// a real backend gives: the connection is closed.
    fn sender(&self, node: &str) -> Result<(Identity, BTreeSet<String>)> {
        let closed = || PwError::unreachable("connection closed");
        let device = self.devices.get(node).ok_or_else(closed)?;
        if device.revoked || !device.online || device.state != NetState::Ready {
            return Err(closed());
        }
        let person = self.people.get(&device.owner).filter(|p| !p.revoked).ok_or_else(closed)?;
        Ok((person.identity.clone(), person.groups.clone()))
    }
}

/// One device's view of a [`FakeNet`]: the [`Network`] its Pitwall uses.
#[derive(Clone)]
pub struct FakeNetwork {
    net: FakeNet,
    node: String,
}

impl FakeNetwork {
    pub fn net(&self) -> &FakeNet {
        &self.net
    }

    pub fn node(&self) -> &str {
        &self.node
    }

    pub fn addr(&self) -> DeviceAddr {
        self.net.addr(&self.node)
    }

    /// Script the backend's state (signed out, error, …). Anything but
    /// `Ready` refuses this device's requests and requests to it.
    pub fn set_state(&self, state: NetState) {
        if let Some(d) = self.net.world().devices.get_mut(&self.node) {
            d.state = state;
        }
    }

    /// How plain clients authenticate to this device (default: none).
    pub fn set_client_auth(&self, auth: ClientAuth) {
        if let Some(d) = self.net.world().devices.get_mut(&self.node) {
            d.client_auth = auth;
        }
    }

    /// Whether a handler is being served here.
    pub fn serving(&self) -> bool {
        self.net.world().devices.get(&self.node).is_some_and(|d| d.serving.is_some())
    }
}

impl Network for FakeNetwork {
    fn id(&self) -> NetworkId {
        self.net.id()
    }

    fn state(&self) -> NetState {
        self.net.world().devices.get(&self.node).map_or(NetState::SignedOut, |d| d.state.clone())
    }

    fn me(&self) -> Option<(Identity, DeviceAddr)> {
        let world = self.net.world();
        let device = world.devices.get(&self.node).filter(|d| d.state == NetState::Ready)?;
        let person = world.people.get(&device.owner)?;
        Some((person.identity.clone(), self.addr()))
    }

    fn serve(&self, handler: Arc<dyn Handler>) -> Result<ServeHandle> {
        let mut world = self.net.world();
        world.next_serve += 1;
        let token = world.next_serve;
        let device = world.devices.get_mut(&self.node).ok_or_else(|| PwError::not_found("no such device"))?;
        if device.serving.is_some() {
            return Err(PwError::new(ErrorCode::Conflict, format!("{} is already serving", self.node)));
        }
        device.serving = Some((token, handler));
        let (net, node) = (self.net.clone(), self.node.clone());
        Ok(ServeHandle::new(move || {
            if let Some(d) = net.world().devices.get_mut(&node) {
                if d.serving.as_ref().is_some_and(|(t, _)| *t == token) {
                    d.serving = None;
                }
            }
        }))
    }

    fn client(&self, to: &DeviceAddr) -> Result<Box<dyn HttpClient>> {
        if to.network != self.net.id {
            return Err(PwError::unreachable(format!("{} is not on {}", to.node, self.net.id)));
        }
        Ok(Box::new(FakeClient { net: self.net.clone(), from: self.node.clone(), to: to.node.clone() }))
    }

    fn endpoint(&self) -> Endpoint {
        let name = self.net.id.as_str().split_once(':').map_or("", |(_, n)| n);
        let client_auth =
            self.net.world().devices.get(&self.node).map_or(ClientAuth::None, |d| d.client_auth.clone());
        Endpoint { base_url: format!("http://{}.{name}.fake", self.node), client_auth }
    }

    fn discover(&self) -> Result<Vec<PeerHint>> {
        let world = self.net.world();
        Ok(world
            .devices
            .iter()
            .filter(|(node, d)| **node != self.node && !d.revoked)
            .map(|(node, d)| PeerHint {
                addr: self.net.addr(node),
                owner: world.people.get(&d.owner).map(|p| p.identity.clone()),
                label: node.clone(),
                online: d.online && d.state == NetState::Ready,
            })
            .collect())
    }

    fn groups(&self, who: &Identity) -> Result<Vec<GroupRef>> {
        let world = self.net.world();
        let groups = world
            .people
            .values()
            .find(|p| p.identity.is(who) && !p.revoked)
            .map(|p| self.net.groups_of(&p.groups))
            .unwrap_or_default();
        Ok(groups)
    }

    fn revocations(&self) -> Receiver<Revocation> {
        let (tx, rx) = mpsc::channel();
        self.net.world().subscribers.push(tx);
        rx
    }
}

struct FakeClient {
    net: FakeNet,
    from: String,
    to: String,
}

impl HttpClient for FakeClient {
    fn send(&self, req: Request) -> Result<Response> {
        let (identity, groups, handler) = {
            let world = self.net.world();
            let (identity, groups) = world.sender(&self.from)?;
            let target = world
                .devices
                .get(&self.to)
                .filter(|d| d.online && !d.revoked && d.state == NetState::Ready)
                .ok_or_else(|| PwError::unreachable(format!("{} can't be reached", self.to)))?;
            let (_, handler) =
                target.serving.as_ref().ok_or_else(|| PwError::unreachable(format!("{} refused the connection", self.to)))?;
            (identity, groups, handler.clone())
        };
        let person = handler.person_of(&identity);
        let caller =
            Caller::new(identity, person, Some(self.net.addr(&self.from)), self.net.id(), self.net.groups_of(&groups));
        Ok(handler.handle(req, caller))
    }
}
