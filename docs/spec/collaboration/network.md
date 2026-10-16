# Collaboration: networks (Tailscale, Pitwall Cloud)

Part of the [collaboration spec](README.md).

## 15. Networks

### 15.1 The rule
Collaboration never talks to Tailscale, or to any other network, directly.
Everything it needs from a network (who is calling, how to reach a device,
who is around, which groups a person is in) goes through one interface,
`Network`, and every network is a **backend** behind it:

| Backend | What it is | When |
|---|---|---|
| **Tailscale** | Devices on the user's tailnet; identity from `whois` | v1 |
| **Pitwall Cloud** | Pitwall accounts; devices reach each other through a relay; identity from signed tokens | When the Cloud exists. Designed now, so nothing above the interface changes when it lands |
| **Fake** | In-process, scripted people and devices | Tests, from v1 |

The core (teams, shares, conversations, limits, sandboxing, results, the
A2A and MCP fronts, the UI) is written once against `Network`, and the
whole test suite runs on the Fake backend. A Pitwall can use several
backends at once ([§15.7](network.md)). Peers (tailnet-peers.md, whole-machine
access) stay Tailscale-only; this interface is for collaboration.

### 15.2 Words
- **Network** (`NetworkId`): one backend instance, e.g.
  `tailscale:example.ts.net` or `cloud:acme`.
- **Identity** (`Identity { provider, subject, email?, name? }`): who a
  network says someone is: `{tailscale, "bob@example.com"}`,
  `{cloud, "u_8KQ2…", "bob@example.com"}`. `subject` is the provider's
  stable id, never a display name.
- **Person**: one or more identities known to be the same human
  ([§15.7](network.md)). Audiences, team members, conversations and limits are keyed
  by person.
- **Device**: one Pitwall installation, its instance key, plus a
  `DeviceAddr { network, node }` on each network it is on.
- **Group** (`GroupRef { provider, id }`): a set of people the provider
  manages (a Tailscale group via app capabilities, a Cloud or SSO group).
- **Caller**: what a backend hands the core for every inbound request:
  `{ identity, person, device?, network, groups, client }`. Only a backend
  can make one.

### 15.3 The interface
A sketch (Rust, in `pitwall-core::net`):

```rust
pub trait Network: Send + Sync {
    fn id(&self) -> NetworkId;
    fn state(&self) -> NetState;              // off | signed_out | connecting | ready | error(msg)
    fn me(&self) -> Option<(Identity, DeviceAddr)>;

    /// Inbound. The backend authenticates every request *before* calling the
    /// handler; a request without a Caller never reaches collaboration.
    fn serve(&self, handler: Arc<dyn Handler>) -> Result<ServeHandle>;

    /// Outbound HTTP to a device on this network, with this device's
    /// credentials attached and the route chosen by the backend.
    fn client(&self, to: &DeviceAddr) -> Result<Box<dyn HttpClient>>;

    /// What others use to reach this device: base URL, and how a plain
    /// client authenticates (None on Tailscale, OAuth on the Cloud).
    fn endpoint(&self) -> Endpoint;

    /// Devices that may run Pitwall, for team discovery (§3).
    fn discover(&self) -> Result<Vec<PeerHint>>;

    /// Provider-managed groups for an identity, for audiences and team binding.
    fn groups(&self, who: &Identity) -> Result<Vec<GroupRef>>;

    /// Called when the provider says someone was removed (or polled).
    fn revocations(&self) -> Receiver<Revocation>;
}

pub trait Handler: Send + Sync {
    fn handle(&self, req: http::Request<Body>, caller: Caller) -> http::Response<Body>;
}

pub struct Endpoint { pub base_url: Url, pub client_auth: ClientAuth }
pub enum ClientAuth { None, OAuth { issuer: Url, resource: Url, scopes: Vec<String> } }
```

`Endpoint.client_auth` is how the two fronts stay network-neutral: the
A2A Agent Card's `securitySchemes` and the MCP endpoint's protected
resource metadata are generated from it, and the "setup line" for a plain
Claude user is built from `base_url`. On Tailscale both say "no auth"; on
the Cloud both point at the Cloud's OAuth issuer, and Claude Code signs in
through the browser as it does for any remote MCP server that uses OAuth.

### 15.4 What every backend must guarantee
The core's safety arguments ([§9](safety.md)) rest on these, not on any one
network. A backend that can't meet one doesn't ship.

| Guarantee | Tailscale | Pitwall Cloud | Fake |
|---|---|---|---|
| **Caller identity can't be forged** by the caller (no header or body field can change it) | `whois` on the connection, from WireGuard keys | A token signed by the Cloud, short-lived, for this device as audience, checked against the Cloud's keys; the relay channel is itself authenticated | Scripted |
| **Encrypted in transit** | WireGuard | TLS to the relay on both legs; Pitwall-to-Pitwall end to end on top (open question 19) | n/a |
| **Only one way in.** Requests reach collaboration only through `serve` | Listener bound to tailnet IPs only, source checked, `whois` before HTTP | No listening port: the device keeps one outbound tunnel to the relay; nothing else is accepted | In-process |
| **Revocation.** Removing a person or device at the provider stops their requests | Node or user removed in the admin console: `whois` fails at once | Account or device disabled: tokens expire within 5 min and the relay drops open tunnels at once | Scripted |
| **Stable identities** | `UserProfile.LoginName` (and node `StableID`) | Account `subject` | Scripted |
| **Groups** | App capabilities in `CapMap` ([§4.8](teams.md)) | Cloud org groups, or the company's SSO groups | Scripted |
| **Discovery** | Probe tailnet peers ([§3](teams.md)) | Ask the Cloud's directory | Scripted |
| **Plain clients can connect** | `http://<node>.ts.net:47421/mcp`, no sign-in | `https://<device>.<org>.pitwall.cloud/mcp`, OAuth sign-in | — |

### 15.5 Tailscale backend (v1)
- **serve:** the HTTP listener on 47421, bound only to this node's
  Tailscale IPs, source checked against tailnet ranges, LocalAPI `whois`
  before any HTTP handling (tailnet-peers.md §4 rules).
- **identity:** `{tailscale, UserProfile.LoginName}`; device node
  `Node.StableID`; groups from the Pitwall app capability in `CapMap`.
- **client:** direct HTTP to the peer's tailnet name.
- **endpoint:** `http://<name>.<tailnet>.ts.net:47421`, `ClientAuth::None`.
- **discover:** probing as in [§3](teams.md).
- Uses tailnet-peers.md's `Tailnet` trait internally (status, LocalAPI).

### 15.6 Pitwall Cloud backend (designed now, built when the Cloud exists)
- **Sign-in:** Settings → Networks → "Pitwall Cloud" → browser sign-in
  (OAuth with PKCE); the device registers its instance key with the
  account.
- **serve:** one outbound tunnel (HTTP/2 or WebSocket) from the device to
  the Cloud relay. The relay forwards requests for this device through it,
  with a signed caller assertion; the backend verifies it before calling
  the handler. No open ports, works behind any NAT.
- **identity:** `{cloud, subject}` from the assertion; groups from the
  Cloud org (or its SSO).
- **client:** HTTPS to the relay with this device's token; the relay
  routes to the target device's tunnel.
- **Plain clients:** the MCP endpoint publishes protected resource
  metadata pointing at the Cloud's OAuth issuer, so `claude mcp add
  --transport http alice https://alice.acme.pitwall.cloud/mcp` makes
  Claude Code sign in through the browser. A2A clients follow the Agent
  Card's `securitySchemes`.
- **discover:** the Cloud's directory of org members' online devices.
- **What the Cloud sees:** with TLS only, the text of messages; with the
  end-to-end option between two Pitwalls, only who talks to whom and when.
  Plain clients can't do end to end (open question 19).

### 15.7 Several networks at once
- **Serving.** Collaboration calls `serve` on every enabled backend with
  the same handler. A share's audience decides who gets in, whichever
  network they arrive on.
- **Audiences** are made of people, teams, provider groups, and "everyone
  on network X" (`{"networks": ["tailscale:example.ts.net"]}`). Never "the
  tailnet" as a special case.
- **One person, several identities.** Bob may be `{tailscale, bob@…}` and
  `{cloud, u_8KQ2…}`. They count as one person only when linked:
  - a team record lists both for the same member (`add_identity`, signed
    by an admin, or by Bob's own device while it is signed in to both
    networks and each backend confirms one of them), or
  - the owner links them by hand in People.
  Unlinked identities are different callers, with separate conversations
  and limits. Matching by email alone is never automatic.
- **Linked identities share** conversations, limits and blocks: Bob
  asking over Tailscale today and over the Cloud tomorrow continues one
  conversation.
- **No bridging.** A Pitwall on two networks never forwards a request from
  one to the other. Each request goes straight from the caller's device to
  the owner's, on a network both are on.
- **Teams can span networks.** Records list members by person (one or more
  identities). Replication goes over whichever network two members share.

### 15.8 Keeping the core network-neutral
- **Code:** `pitwall-core::collab` depends only on `pitwall-core::net`
  (the trait and its types). A CI grep fails if `collab` imports a
  backend module (`net::tailscale`, `net::cloud`), like the existing
  no-special-cases check.
- **Data:** identities, people, groups and network ids use the shapes in
  [§15.2](network.md) from v1, in team records, share settings, usage and the audit log,
  so nothing is migrated when a second backend arrives.
- **Words:** the UI says "network", "people" and "groups", and names the
  backend only where it matters ("on example.ts.net", "Pitwall Cloud").
- **Tests:** the full suite runs on the Fake backend; a contract test
  suite checks each real backend against [§15.4](network.md).
