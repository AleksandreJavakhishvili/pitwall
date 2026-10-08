# Pitwall peers over Tailscale

One Pitwall sees and drives the agents of another Pitwall running on another
machine on the same Tailscale tailnet: the laptop shows the desktop's agents
next to its own, opens their terminals, reviews their changes, gets their
"needs you" notifications and starts new agents there.

Status: **planned, not built.** It comes after the GPUI switch (gpui/README.md
phase 9): the Tauri app is frozen and gets none of it. The core, protocol and
daemon parts are UI-agnostic. They also finish work that architecture.md §4
and §6 step 5 already plan (events and terminal streams over the socket), and
the local CLI needs that work anyway.

Hostnames and users below are made up: `laptop` and `studio` are two
machines, and `alice@example.com` is their owner.

---

## 1. Goals, non-goals, user stories

### User stories
1. **One list.** On `laptop` I see my agents and, under a `studio` heading,
   the agents running on `studio`, with live status, diffstat and Next up.
   This also covers `studio`'s agw VMs, since they are `studio`'s agents.
2. **Take over a terminal.** I open `studio`'s `api-fix` in a pane and type
   into it, as if it were local. On `studio`, its pane says "Typing from
   laptop" until someone there takes it back.
3. **Review.** Review, Changes and the explorer work for `studio`'s agents.
   That includes diff, per-task changes, commit and merge (merge with the
   confirmation it always has).
4. **Needs you.** When an agent on `studio` turns blocked or done, `laptop`
   notifies me ("api-fix on studio needs you"), and the Dock or taskbar
   badge counts it.
5. **Start an agent there.** In New agent, "Runs on" offers `studio`, and
   `studio`'s agw VMs if `studio` can create there. The agent then runs on
   `studio`.
6. **Decide how much.** Per paired machine I choose **View**, **Control**,
   **Start agents** or **Full**, I can change it at any time, and I can
   revoke a machine.

### Goals
- The peer stays **authoritative**. Agents keep living in the peer's
  holders, and the peer's engine keeps folding status, sending Next up,
  snapshotting tasks and asking approvals. The viewer is a remote UI.
- **Off by default, opt-in twice.** A machine accepts connections only after
  its owner turns that on. A peer sees anything only after it has been paired
  and given a permission on the machine it connects to.
- **Never listen outside the tailnet.**
- **No special cases in the UI.** Peer agents are `AgentView`s whose `caps`
  already say what works (architecture.md §3). The UI keeps deciding by
  caps, so the CI grep stays green.

### Non-goals (v1)
- No relaying: one hop only. `laptop` sees `studio`'s own agents, never the
  agents `studio` mirrors from a third Pitwall.
- No syncing of settings, rule libraries, project lists, spaces or kinds
  between machines.
- No moving an agent from one machine to another.
- No reaching peers outside a tailnet: no public internet, no other VPNs,
  and no mDNS or LAN broadcast.
- No web or mobile client.
- No real multi-user collaboration. Two people typing into one terminal is
  possible but is just last-writer input, as with tmux.
- No peer access from agents or the CLI on the viewer. `pitwall` on `laptop`
  does not reach `studio` in v1 (open question 5).

---

## 2. Model

### A peer is neither a provider nor only a machine group
A `Provider` (architecture.md §2.2) is a "dumb" place to run. Pitwall's own
engine is authoritative for an agent there: it folds status, keeps the queue,
auto-sends, snapshots tasks and runs git through `Exec`. A peer Pitwall is a
**smart** place that already does all of that. Modelling it as a `Provider`
would mean:
- **Two engines driving one agent.** Both would auto-send Next up, both would
  record tasks and refs, and both would decide its status.
- **`Exec` over the network**, which is a remote shell.
  `Provider::exec` → `Cmd { argv }` would let the viewer run any command on
  the peer, far beyond what the user granted.
- **Re-attaching through two layers of holders**, so terminal history,
  resize and "who controls" would be wrong twice.

So a peer is a **new layer beside the engine, not under it**:

```
            laptop (viewer)                              studio (peer)
┌──────────────────────────────────┐       ┌─────────────────────────────────┐
│ GPUI app                         │       │ Engine (authoritative)          │
│  AgentStore = local ∪ mirrors    │       │  providers: local, agw          │
│  Route(agent) → Local | Peer     │       │  holders, status, queue, tasks  │
├──────────────────────────────────┤       ├─────────────────────────────────┤
│ Engine (local agents only)       │       │ daemon: method table + approvals│
│ Peers: PeerLink per paired peer  │──TCP─▶│ peer listener (tailnet IP only) │
│   mirror of studio's AgentViews  │ over  │   Caller::Peer{peer, perms}     │
│   forwards calls, streams        │ WG    │   audit log                     │
└──────────────────────────────────┘       └─────────────────────────────────┘
```

It looks like both of the options in the question:
- **In the UI** it reads like a provider. "Runs on" lists `studio` and
  `studio › vm-1`, and the pane header's machine chip says `studio`.
- **In the sidebar and Wall** it is a machine group, using the machine
  heading pattern that already exists (`machineHeading` in `groups.ts`, and
  its port in `pitwall-app/src/agents.rs`).
- **In core** it is neither. The engine never sees peer agents. A `Peers`
  service holds one `PeerLink` per paired peer, and the app merges the
  mirrors into what it shows.

### Identity of peer agents on the viewer
- **Id.** The viewer namespaces the peer's id as `peer:<peerId>:<agentId>`.
  `peerId` is the viewer's local name for the pairing, a short slug, so ids
  in `ui.json` spaces stay stable across renames and a route is found from
  the id alone.
- **Machine.** `AgentView.machine` is rewritten on the viewer:
  - provider `peer:<peerId>`
  - id = the peer's machine id
  - label `studio`, or `studio › vm-1` for the peer's agw machines
  - `canCreate` from the permission (§5)
- **Link state.** A new optional `MachineView.link`:
  `{ peer, state: "online"|"connecting"|"offline", lastSeen }`. It is
  additive: older UIs and local agents leave it out.
- **Effective caps.** The viewer computes them as
  `peer's AgentCaps ∧ permission ∧ link online ∧ peer's welcome.caps`.
  For example, View makes `input`, `stop`, `restart`, `merge` and `rules`
  false, and an offline link makes everything except showing the row false.
  The UI keeps reading only `caps`.

### Loops and duplicates
- **No loops.** A peer serves only its engine's own records; peer-served
  `agent.list` never includes mirrors. So A showing B while B shows A is two
  independent one-hop views, with no echo and no growth.
- **Duplicates.** If `laptop` and `studio` both adopted the same agw session
  (same `Locator` `agw:<vm>/<session>`), the viewer shows it once, as its
  own local agent, with a "also on studio" tooltip. The peer's copy is
  hidden, not merged: the two Pitwalls are independent attachments. To
  match them, `AgentView` gains an optional `locator` string. It is
  additive, and only adopted platform sessions need it.

### When the peer sleeps or goes offline
- **Agents are unaffected.** Its agents keep running (or pause with the
  machine), because holders don't depend on any viewer.
- **Detecting it.** The link sends an app-level ping every 15 s and calls
  the peer offline after 45 s without traffic. `tailscale status` showing
  the node as `Online: false` makes it offline immediately.
- **Reconnecting.** Backoff goes 1 s, 2 s, 5 s, 10 s, 30 s. It reconnects at
  once when the node comes back in `tailscale status`, on the viewer's wake
  from sleep, and on a network change.
- **What the viewer shows.**
  - The last known rows stay, dimmed, with "studio offline · 12 min".
  - Statuses are shown as last known, never as live. No attention is raised
    from stale data, and the badge drops them.
  - Open terminals show an overlay: "studio is offline, reconnecting…". On
    reconnect the terminal is re-attached with replay, like a local
    re-attach.
- **No writes while offline.** Nothing is buffered for later: typing, Next-up
  edits and creates are refused with "studio is offline". Queuing for later
  would surprise people (an open question if wanted).
- **Restarts.** A peer restart changes `welcome.instance`, so the viewer
  re-subscribes and re-attaches.

---

## 3. Discovery

Three ways to find peers, in this order.

1. **Tailscale status (default).**
   - **Source.** Pitwall asks the local Tailscale for its peers through the
     `tailscale` CLI (`status --json`). From the result it uses:
     - `Self` (its `UserID` and `TailscaleIPs`)
     - `Peer[*]`: `HostName`, `DNSName`, `TailscaleIPs`, `Online`, `OS`,
       `UserID`, `Tags`, and `CurAddr`/`Relay` for the path
     - `User[*]`, for `LoginName` and `DisplayName`
     - `MagicDNSSuffix`

     Field names are from Tailscale's public docs and are checked again at
     implementation time.
   - **Candidates.** Online peers owned by the **same Tailscale user** as
     this node. Peers carrying a configurable ACL tag (default
     `tag:pitwall`) are candidates too: on a shared tailnet that is the
     admin's way to opt devices in.
   - **Probing.** Pitwall connects to the candidate's tailnet IP on Pitwall's
     port and sends a `hello`. A machine that isn't listening simply doesn't
     answer.
   - **When.** It probes only while Settings → Peers is open, or every
     10 min for paired peers that are offline. Nothing is probed in the
     background otherwise, and nothing at all while the feature is off.
2. **Manual add by name.** The user types a MagicDNS name (`studio`, or
   `studio.example-tailnet.ts.net`) or a tailnet IP. Pitwall resolves it,
   refuses anything that isn't a tailnet address (§4), then probes it. This
   is the way to reach a machine owned by another user who isn't tagged.
3. **Remembered peers.** A paired peer is stored with its Tailscale
   `StableID`, so its current IP comes from `tailscale status` and keeps
   working after IP or name changes.

**Pitwall's port.** One fixed default is proposed: 47420/tcp, to be checked
against well-known registries before release, and configurable. Tailscale
has no general "advertise a service on a port" feature for ordinary nodes,
which is why a fixed port plus a probe is used. Tailscale Services (VIP
services that admins define) are an option for P5, not a requirement.

**No mDNS, no broadcast.** Tailnets don't carry multicast, and broadcasting
on the LAN would advertise Pitwall to networks the user didn't choose.

**Unpaired peers learn little.** A peer that answers a probe returns only
`{ pitwall version, machine label, pairing: "open"|"closed" }`. It returns
nothing about agents.

---

## 4. Transport

### Options

| Option | How | Complexity | macOS / Windows / Linux | Without Tailscale |
|---|---|---|---|---|
| **A. TCP bound to the tailnet IP** (recommended) | `std::net::TcpListener` on this node's 100.x / fd7a:115c:a1e0:: addresses, carrying the existing pitwall-proto frames | Low: the framing, handshake and dispatch already exist | All three (Tailscale's interface is a normal interface: utun, Wintun, tun) | No address to bind, so the feature is shown as unavailable |
| B. HTTP/2 or WebSocket on the tailnet IP | hyper or tungstenite with the same frames inside | Medium: new deps, async runtime in the daemon | All three | Same as A |
| C. Embedded tsnet (Go) | Pitwall becomes its own tailnet node through `libtailscale` (C API, cgo) | High: Go runtime in the binary (tens of MB), cross-compiling cgo for three OSes, a **second device** per Pitwall in the admin console with its own login or auth key | Doable, painful on Windows | Works without the Tailscale app but needs a login flow inside Pitwall |
| D. A Rust Tailscale library | None mature enough to ship today | — | — | — |
| E. `tailscale serve` to localhost | Tailscale forwards a tailnet port to `127.0.0.1` | Low | All three | — |

**Recommendation: A.** It reuses everything Pitwall already has, needs no new
runtime and no new tailnet device, and identity comes from the system's
Tailscale (`whois`).
- **Why not B.** It adds nothing a single framed TCP stream lacks. Pitwall
  isn't served to browsers.
- **Why not E.** It hides the real peer address behind localhost, so `whois`
  can't identify the caller, and any local process could pretend to be a
  peer.
- **Why not C.** It is kept as a fallback idea, only if userspace-only
  setups matter later.

**Rules for the listener (A):**
- **Addresses.** It binds only to this node's Tailscale IPs, taken from
  `Self.TailscaleIPs`, one socket per address. Never `0.0.0.0`, `::` or a
  LAN address.
- **Rebinding.** If Tailscale starts after Pitwall, or the IP changes, the
  listener rebinds when the status changes (polled every 30 s while enabled,
  and on network-change notifications).
- **Checks on accept.**
  - The peer address must be in `100.64.0.0/10` or `fd7a:115c:a1e0::/48`.
  - `whois` must succeed for it. Otherwise the connection is closed without
    a byte.

  This holds even though the bind already limits who can reach the socket.
- **Linux in userspace-networking mode** (no tun device): the tailnet IP
  isn't on any interface, so binding fails, and ordinary outgoing
  connections to 100.x don't route either. Settings says so ("Tailscale
  runs in userspace mode; Pitwall can't use it here"). Dialing out through
  Tailscale's SOCKS5/HTTP proxy is a P5 option (open question 7).
- **Firewalls.**
  - macOS's application firewall may ask once for "incoming connections"
    (signed builds keep the answer).
  - Windows Defender Firewall asks per network profile. The Tailscale
    adapter is usually its own network, so Pitwall asks only for that one
    and documents it.
  - Linux has nothing extra.
- **Reaching LocalAPI.** Through the `tailscale` CLI: `status --json`,
  `whois --json <ip>`, `ip`.
  - The CLI already knows each platform's LocalAPI path: the macOS app
    variants (App Store, standalone, open-source `tailscaled`), the Windows
    named pipe and the Linux socket.
  - Pitwall finds the binary on PATH, then in the macOS app bundle
    (`/Applications/Tailscale.app/Contents/MacOS/Tailscale`), then in the
    Windows install folder.
  - Results are cached: status for 5 s, whois per peer address for 60 s
    and dropped on disconnect.
  - Talking to LocalAPI directly is a later optimisation behind the same
    trait (§8).
- **Without Tailscale.** Settings → Peers shows "Tailscale isn't installed",
  "…isn't running" or "…needs login" (from `BackendState`), with a link to
  Tailscale. Pitwall itself is unchanged.

**Encryption.** WireGuard already encrypts and authenticates every packet
between the two nodes, end to end, DERP relays included. Pitwall adds
**application-level authentication** (§5: a pairing key per Pitwall
instance) but not a second encryption layer in v1. The transport sits behind
a small trait, so TLS (rustls with keys pinned at pairing) can be added if a
non-Tailscale transport is ever allowed.

---

## 5. Auth and security

### Default posture
- **Two separate switches, both off:**
  - "Connect to other Pitwalls" lets this machine pair with others and
    show their agents (outgoing only).
  - "Let paired Pitwalls connect to this machine" opens the listener.
- `laptop` typically turns on only the first, and `studio` both. Turning
  the listener on shows the address it listens on, and the warning in
  "What each permission really means" below.

### Who is connecting: three facts, all required
1. **Tailscale node and user** from `whois` on the connection's source
   address: `Node.StableID`, `Node.Name`, `Node.Tags`,
   `UserProfile.LoginName`. Tagged devices have no user (Tailscale reports
   them as tagged devices), so they are named by node and tags.
2. **Pitwall instance key.** Each Pitwall data folder has an Ed25519 key pair
   in `peers/identity.key` (0600; on Windows, readable by the user only, as
   for the named pipes). In the handshake, each side signs the other's
   random nonce together with both tailnet addresses and the protocol
   version. This binds the connection to that Pitwall installation, not
   merely to "some process on that node".
3. **The pairing record:**
   `peers.json[peer] = { name, stableId, login, publicKey, perms, notify, addedAt }`.
   All three facts must match. If the node, the user or the key changes,
   the peer is refused with "studio's identity changed; pair again". Node
   key rotation doesn't matter, because `StableID` survives it.

### Pairing (first contact)
- **Starting it.** On `laptop`, Settings → Peers → `studio` → **Pair…**.
  `laptop` connects and sends `peer.pair { publicKey, label }`.
- **What `studio` shows.** A dialog in every window, from the same component
  as approvals:

  > **Allow alice@example.com on laptop to use Pitwall here?**
  > Code **4F7-K2Q**, also shown on laptop.
  > Access: View ▾  (View · Control · Start agents · Full)
  > [Don't allow] [Allow]

- **What `laptop` shows.** The same code and "Waiting for studio…". Matching
  codes prove that the dialog belongs to this request.
- **Warnings.** Requests from another Tailscale user, or from a tagged
  device, show an amber line: "This device belongs to bob@example.com", or
  "This is a tagged device (tag:ci)".
- **Not being at the other machine.** The user may be at `laptop` only.
  `studio` can pre-authorise: Settings → Peers → **Add a device…** shows a
  one-time code, valid for 10 minutes, that the user types into `laptop`'s
  Pair dialog. With a valid code no dialog appears on `studio`, and the
  access level is the one chosen when the code was made. This is the only
  way to pair without someone at the listening machine.
- **Direction.** Pairing goes one way: `laptop` sees `studio`. Showing
  `laptop` on `studio` is a separate pairing in the other direction, with its
  own permission.
- **Limits.** At most 3 pairing requests per node per 10 minutes. An
  unanswered request expires after 120 s, the approvals timeout.

### Permissions (per paired peer, set on the machine being accessed)

| Level | Allows | Does not allow |
|---|---|---|
| **View** | Agent list and status, events, attention, view-only terminal streams and screen frames (Wall), Changes, diffs, Review reads, explorer reads, worktree lists, Next-up list | Any input, any change |
| **Control** | View, plus typing and resizing (take-over), send prompt, Next-up add/remove/send-now/auto-send, mark seen, stop and restart, Review comments (a prompt), refresh | Creating or removing agents, Low/High-risk methods |
| **Start agents** | Control, plus `agent.create` on its creatable machines (folder form: kinds, recent projects; platform form: agw), `session.add` | High-risk methods |
| **Full** | Start agents, plus the risky methods (commit, merge, discard, remove with worktree, rules apply to main), and **answering this machine's approvals** from the viewer | `hooks.install`, `daemon.stop`, Settings, pairing others, CLI install: never over the network |

**What each permission really means, said plainly in Settings:** Control is
effectively full use of this computer as you. Typing into an agent or a
shell terminal runs commands. View still shows code and whatever terminals
print, which may include secrets. Only pair machines you own or fully trust.

**Approvals for peer requests.**
- The peer's method table (`methods.rs` `METHODS`) gains one column: the
  minimum permission a peer needs. A missing entry means "never over the
  network".
- `dispatch` runs as today with `Caller::Peer { peer, login, node, perms }`.
- Risky methods follow the existing approval path:
  - **Below Full**, a risky method asks on the peer machine (amber dialog,
    "alice@example.com on laptop wants to merge api-fix") and waits up to
    120 s. Nobody there means a timeout, which is a denial.
  - **With Full**, the confirmation the user already gave on the viewer
    counts. "The dialog the user sees is the approval" (architecture.md §4)
    holds across the link. The viewer's merge confirmation is the approval,
    and the peer's audit log records it as "approved on laptop".
- **Approvals raised on the peer** (an agent on `studio` asking through
  `studio`'s CLI) are sent to viewers with Full that enabled "Answer approvals
  from studio". They appear in `laptop`'s approval dialog marked "on studio".
  The first answer wins, from either side.

**Viewer-side rule (critical).** On `laptop`, every call that goes to a peer
must come from `laptop`'s own verified UI. In the GPUI app the UI is
in-process, so this holds by construction. An agent on `laptop` can never
drive `studio`: peer routes are not exposed on `laptop`'s CLI socket in v1,
and if they are later, they are `Access::Ui`, or answered with an approval
on `laptop`.

### Limits, revocation, audit
- **Rate limits per peer.**
  - Connections: at most 4.
  - Requests: 100 per second sustained, bursts of 300.
  - Terminal input: not rate limited (typing), but its frames are capped
    at 64 KiB.
  - Screen watches: at most 64.
  - Search: one at a time (as now: a newer search cancels).
  - Failed signatures close the connection and back off that node
    exponentially, up to 10 min.
- **Frame size.** `MAX_FRAME` drops to 4 MiB for peer connections. Large
  reads (`read_file` up to 10 MiB) are chunked (§6).
- **Revocation.** Settings → Peers → Remove deletes the pairing and closes
  its connections at once. Lowering a permission takes effect on the next
  request: open terminal streams are downgraded to view-only and lose
  control. Turning the listener off closes everything.
- **Audit log.** `peers/audit.jsonl` on the accessed machine records one line
  per mutating call, per pairing event and per refused connection:
  `{ at, peer, login, node, method, agent, outcome, approvedOn }`.
  - Reads are only counted per minute: the stream would be too noisy.
  - The log rotates at 1 MiB, keeping 3 files.
  - It is shown in Settings → Peers → `<peer>` → Activity.

### Threats

| Threat | Mitigation |
|---|---|
| Someone else on a shared tailnet (family, company) | Discovery lists same-user nodes only. Other users need a manual add or a tag, plus pairing on the target with an amber "belongs to" warning. Admins can also block the port with ACLs (below) |
| A process on the peer node other than Pitwall connects | It lacks the Pitwall instance key, so the signature fails and the connection is refused |
| Another OS user on a multi-user peer machine | Same as above: the key file is readable by its owner only |
| A compromised viewer (malware running as alice on `laptop`) | It can do what the permission allows. That is inherent and is why the default level is View, Full is spelled out, and the audit log is on the accessed side |
| An agent on the viewer tries to drive the peer | No peer routes on the viewer's CLI socket in v1, and later only for the verified UI |
| An agent on the peer tries to approve its own request through a viewer | Viewers can answer only approvals the peer forwarded to Full viewers, from their verified UI. Agents never see approvals |
| Replay or relay of a handshake | Nonces are signed together with both tailnet addresses and the version, and `whois` must match the stored node |
| Listener exposed beyond the tailnet | It binds only to Tailscale IPs, checks the source range and requires `whois` |
| Tailnet ACLs | Pitwall works under any ACL that lets the viewer reach the port. Docs suggest a grant that limits the port to the user's own devices, e.g. `{"src":["autogroup:member"],"dst":["autogroup:self"],"ip":["tcp:47420"]}`. P5: honour an optional Tailscale app capability from grants (seen in `whois` `CapMap`) as a ceiling on the permission level an admin allows |
| Denial of service from a paired peer | Rate limits and caps on connections and watches, peer work on its own threads, and the local UI never waits on peer calls |

---

## 6. Protocol

### Reuse pitwall-proto, as is, over TCP
- **Same framing and messages.** Peers use `len:u32be type:u8 body`, with
  JSON type 0 and terminal type 1, the same `Request`/`Response`/`Event`,
  and the same method names (architecture.md §4).
- **Handshake.** It gains a role and a signature step:

```json
→ {"hello":{"protocol":{"min":1,"max":2},"client":"pitwall-app/0.4.0","role":"peer",
            "peer":{"key":"ed25519:…","nonce":"…","label":"laptop"}}}
← {"welcome":{"protocol":2,"daemon":"0.4.0","caps":["agents","events","term","screen","review",
            "explorer","worktrees","queue","create","approvals","peer"],"instance":"…",
            "peer":{"key":"ed25519:…","nonce":"…","sig":"…","label":"studio","perms":"control"}}}
→ {"id":1,"method":"peer.auth","params":{"sig":"…"}}
← {"id":1,"result":{"perms":"control"}}          // or {"error":{"code":"denied"}}
```

Until `peer.auth` succeeds, only `peer.info` and `peer.pair` answer. Every
other method returns `denied`, so an unpaired caller learns nothing about
agents.

### Methods a peer can call

Most already exist, or are planned as the api.md → method mapping
(architecture.md §4: `list_agents` → `agent.list`). The column says the
lowest permission that allows it.

| Area | Methods | Perm |
|---|---|---|
| Agents | `agent.list`, `agent.refresh`, `machine.list`, `kind.list`, `project.recent` | View |
| Events | `events.subscribe { topics: ["agents","attention","blocked","approvals"] }` → `agents.changed`, `attention`, `blocked.changed`, `approvals.changed` (the last only for Full with "answer approvals") | View |
| Terminal | `term.attach { agent, replay, control }` → stream id, then type-1 frames; `term.detach`; `term.resize` (control only); input = type-1 frames from the viewer on a control stream | View for `control:false`; Control for `control:true` |
| Screen (Wall) | `screen.watch { agent, gapMs }` → stream id, frames as type-2 (below); `screen.unwatch` | View |
| Input | `agent.prompt`, `agent.mark_seen`, `queue.add`, `queue.remove`, `queue.send_now`, `queue.auto_send` | Control |
| Lifecycle | `agent.stop`, `agent.restart` | Control |
| Lifecycle | `agent.create`, `machine.form`, `session.list`, `session.add` | Start agents |
| Lifecycle | `agent.remove` (keeps worktree) | Start agents |
| Changes | `changes.get`, `changes.refresh`, `changes.file_diff` | View |
| Review | `review.tasks`, `review.task_changes`, `review.file_versions`, `review.merge_status` | View |
| Review | `review.commit`, `review.merge`, `review.discard`, `agent.remove{deleteWorktree}` | Full (else asks on the peer) |
| Worktrees | `worktree.list`, `worktree.refresh`, `worktree.changes`, `worktree.file_versions`, `worktree.merge_status` | View |
| Worktrees | `worktree.commit`, `worktree.merge`, `worktree.remove` | Full (else asks) |
| Explorer | `explorer.list`, `explorer.all`, `explorer.read`, `explorer.search`, `explorer.cancel` | View |
| Rules | `rules.agent` (which set an agent uses, read-only) | View |
| Rules | `rules.apply` to the agent's folder | Control |
| Rules | `rules.apply` to the main checkout | Full |
| Approvals | `approval.list`, `approval.answer` (forwarded ones only) | Full + opt-in |
| Pairing | `peer.info`, `peer.pair`, `peer.auth` | Unauthenticated |

Never over the network:
- `hooks.install`, `daemon.stop`, `ui.get` / `ui.set`
- Settings, `cli.install`, the onboarding scan
- `project.add` / `project.remove`
- the rule library and its sources

The paths that explorer, Review and worktree methods accept stay as they are
today. They are relative to the agent's folder, or come from
`worktree.list`, and the existing `clean`/`inside` checks apply. A peer can
never name an arbitrary file.

### Terminal streams
- **Bytes, not frames, for panes.** `term.attach` replays the ring (as
  `attach_output` does) and then streams raw PTY bytes, so the viewer's
  `pitwall-term-view` renders them as for a local agent. Input goes back as
  type-1 frames on the same stream id.
- **Control lease.** Today, "the last controlling client to resize wins".
  With two machines that would make the sizes of `laptop`'s pane and
  `studio`'s pane fight. `TermHost` gains a **controller**: `Local` or
  `Peer(id)`.
  - A `control:true` attach from a peer takes control. The peer's own pane
    turns view-only, shows "Typing from laptop · Take back", and stops
    resizing.
  - "Take back" (or the peer detaching or going offline) returns control.
  - Only the controller's resizes count. Input from a non-controller is
    refused with `conflict`.
  - The rule is one controller per agent, the same "an agent is shown in at
    most one pane" idea extended across machines.
- **Wall tiles use `ScreenFrame`s**, never bytes. A tile watch is
  `screen.watch`, and its frames travel in a new **frame type 2**
  (`stream:u32be` + JSON `ScreenFrame`). This keeps them off the JSON
  request lane and lets the writer drop them under pressure. Old peers
  ignore unknown frame types, which the framing already allows.

### Bandwidth and latency
- **Two lanes per peer, two TCP connections.**
  - The **interactive** connection carries JSON calls, events and
    control-terminal streams.
  - The **bulk** connection carries screen frames, view-only terminal
    streams, file reads and searches.

  This way a burst of Wall frames or a 10 MiB file read never delays a
  keystroke. Both connections are authenticated the same way.
- **Back-pressure.**
  - Each connection's writer has a bounded queue.
  - Screen watchers already coalesce: one pending wake per watcher, and a
    diff against what was last sent. When the bulk queue is above its
    watermark, a watcher skips the frame, and the next one carries every
    change since.
  - Terminal byte streams are never dropped. A view-only stream that falls
    more than 1 MiB behind is cut and re-attached with replay.
- **Rates.**
  - Wall tiles over a peer link: at most 4 frames/s each, against 10
    locally, and only tiles on screen.
  - The viewer lowers the rate to 1/s when the link is relayed (DERP) or
    its RTT is over 80 ms.
  - Pane terminals are unthrottled.
- **Compression.** A `zstd` welcome cap turns on per-frame compression
  (frame type 3 wraps a compressed inner frame) for frames of 1 KiB and up.
  Rough numbers, to be measured in P2:
  - A full 120×40 `ScreenFrame` is about 20–40 KB of JSON and about
    3–8 KB compressed.
  - 20 busy tiles change about 5 rows per frame. At 4 fps that is roughly
    100–150 KB/s compressed.

  PTY bytes compress well too (TUI redraws are repetitive).
- **Big reads.** `explorer.read` and `review.file_versions` above 1 MiB come
  in 256 KiB chunks on the bulk connection.
- **Latency.** On a direct tailnet path the RTT is typically single-digit to
  tens of ms. Over DERP it can be 50–200 ms. Typing is echoed by the remote
  program, as with ssh: no local echo, no prediction. Settings → Peers
  shows each link's path (direct or relayed, from `tailscale status`) and
  its RTT (from the ping).
- **Polling stays on the peer.** Git refreshes, file watches and agw polling
  run on the peer as they do now. The viewer receives `agents.changed`
  events, throttled to at most 4 per second as locally. It never polls git
  over the link: Changes and Review call `changes.refresh` only when opened
  or on ↻.

### Versioning and compatibility
- **Additive changes.** Within a protocol version, fields and methods are
  only added, and unknown fields are ignored. Features are gated by
  `welcome.caps`, never by version numbers (as in msg.rs today).
- **Version 2.** Peers need things version 1 doesn't define (frame types 2
  and 3, `role: peer`, `peer.*` methods), so peers bump `PROTOCOL` to 2. A
  v2 server keeps answering v1 hellos for the local CLI.
- **Mixed Pitwall versions.**
  - The viewer shows a peer's features only when its caps include them. A
    peer without `review` gets `caps.review = false` on its agents.
  - A peer whose protocol range doesn't overlap shows as "studio runs an
    incompatible Pitwall (0.3); update one of them".
  - `AgentView` from a newer peer may carry unknown fields, which are
    ignored. One from an older peer lacks some fields, which take their
    defaults (they are already `#[serde(default)]` or `Option`).
- **Golden JSON fixtures**, as for the local protocol, for every peer
  message and for the two lanes.

---

## 7. UI (GPUI app)

Compact, in the existing visual language: machine headings, chips, the amber
approval dialog, and the Settings row style.

- **Sidebar.**
  - A peer is a machine heading, `studio` (and `studio › vm-1` for its agw
    machines), placed after this machine's groups and sorted with the other
    machines by label (`sortGroups`).
  - The heading carries a small link dot: online (no dot), connecting
    (pulse), offline (hollow, plus "offline · 12 min"). It is collapsible
    like the others and shows the ▲ blocked count when collapsed.
  - The project "+" menu appears for a peer's projects only when
    `machine.canCreate`, which is Start agents.
  - Peer rows look exactly like local rows. With View, the context menu
    shows only "Open", "Open in Review" and "Copy name".
- **Panes.** The pane header's machine chip reads `studio`, and its tooltip
  "Pitwall on studio · Control · direct, 8 ms".
  - A view-only peer pane (View permission) shows a thin "View only" strip
    and no prompt box.
  - The peer machine's own pane shows "Typing from laptop · Take back" while
    `laptop` controls.
- **Wall.** It has the same sections per machine. Peer tiles are drawn from
  remote `ScreenFrame`s at the reduced rate, and offline tiles show their
  last frame dimmed.
- **Right panel** (Next up, Changes, Files, Last sent). Unchanged, driven by
  caps.
- **New agent → "Runs on".**
  - It lists `studio` and `studio › vm-1` when the peer granted Start
    agents and is online.
  - "Reading what studio has…" covers the form load from `machine.form`,
    `kind.list` and `project.recent`.
  - The project picker shows `studio`'s recent projects, and a typed path is
    taken as a path on `studio`. There is no native folder picker for a
    remote machine.
- **Settings → Peers** (new page):
  - The two switches (§5), with "Listening on 100.x.y.z:47420", or why not.
  - Paired, one row per peer: name, user, link state and path, and the
    permission select (View · Control · Start agents · Full). Toggles:
    "Notify me", "Count in badge" and (Full) "Answer its approvals".
    Buttons: "Activity…" and "Remove".
  - Who can use this machine: peers paired in the other direction, with
    their permission and Remove. Buttons: "Add a device…" (one-time code)
    and "Activity…".
  - On your tailnet: discovered candidates with "Pair…" and "Add by name…".
  - A status line when Tailscale is missing or stopped, or needs login.
- **Notifications and badge.**
  - `attention` events from online peers with "Notify me" raise the same
    native notification, with " on studio" appended:
    "api-fix on studio needs you".
  - The Dock badge, taskbar overlay and tray tooltip count local blocked
    agents plus blocked agents of peers with "Count in badge" (on by
    default when Notify is on).
  - Clicking the notification focuses that agent, as for local ones.
- **Approvals dialog.** The requester line gains the place: "alice@example.com
  on laptop" for a peer's request on this machine, and "on studio" for a
  forwarded approval.

---

## 8. Where the code goes

| Piece | Where | Notes |
|---|---|---|
| Frame types 2 and 3, `role: peer`, `peer.*` shapes, `MachineView.link`, `PROTOCOL` 2 | `pitwall-proto` | Serde only, TS generated as today (the React demo ignores the new fields) |
| Events and terminal streams over the socket, remaining methods | `pitwall-daemon` (`server.rs`, `methods.rs`) | Already planned (architecture.md §6 step 5 "Not done"). Shared by the local CLI and peers |
| Per-method peer permission column; `Caller::Peer` | `pitwall-daemon::methods`, `identity` | One table, as now |
| `Tailnet` trait (`status`, `whois`, `self_addrs`) with `TailscaleCli` and `FakeTailnet` | new crate `pitwall-peer` | No Tailscale code anywhere else |
| Listener (bind, accept checks, auth, rate limits, audit) | `pitwall-peer::server`, calling `pitwall_daemon::methods::dispatch` | Threads, like the local server |
| `PeerLink` (two connections, mirror of views, call forwarding, stream mux, reconnect) and `Peers` | `pitwall-peer::link` | Blocking and threaded, like `pitwall-client` (reuses its framing code) |
| Pairing store, keys, `peers.json`, `audit.jsonl` | `pitwall-peer::store`, under `<PITWALL_HOME>/peers/` | `Paths`-injected, tempdir-testable |
| `TermHost` controller lease | `pitwall-core::term` | Small. Local-only behaviour is unchanged when no peer controls |
| Routing: `Route::Local | Route::Peer` behind the app's existing `Source` traits (explorer `Source`, wall `ScreenSource`, `remote::Source`) plus a `ReviewSource` and an ops router for queue and lifecycle | `crates/pitwall-app` | Views stay unaware of peers |
| `AgentStore` merges the local engine's agents with the peers' mirrors; `PeersChanged` events; badge sum | `crates/pitwall-app/src/agents.rs` | |
| Terminal view fed by a `TermFeed` (local `TermHost` or remote stream) | `pitwall-term-view` / `terminal.rs` | |

New dependencies, each recorded in gpui/licensing.md:
- an Ed25519 implementation (`ed25519-dalek`, BSD-3-Clause, or `ring`)
- `zstd` (MIT/BSD)

No async runtime is added.

---

## 9. Phases

Efforts are rough, for one engineer working with agents, after the GPUI
switch. Each phase ends usable on its own, with tests green on all three
OSes.

| Phase | Scope | Effort | Done when |
|---|---|---|---|
| **P0 prerequisites** | Socket events (`events.subscribe`), `term.attach`/`detach`/`resize` and input over the local socket. The remaining methods as daemon methods: queue, prompt, stop/restart, changes, review, worktrees, explorer. Golden fixtures. Useful to the CLI by itself | 1–1.5 wk | `pitwall` CLI can follow events and attach a terminal locally |
| **P1 read-only list and status** (macOS) | `pitwall-peer`: `Tailnet` (CLI) and `FakeTailnet`, keys, pairing (dialog and one-time code), listener with all accept checks, `PeerLink` interactive lane, mirror, `agents.changed`/`attention`, View permission only. UI: sidebar machine heading with link state, Settings → Peers, notifications and badge. Audit log | 2 wk | `laptop` shows `studio`'s agents live, notifies on blocked, survives `studio` sleeping |
| **P2 terminal view and take-over** | `term.attach` over the link (view-only, then control), `TermHost` controller lease, "Typing from laptop · Take back", Control permission, prompt and Next up, stop and restart, the bulk lane, frame type 2, Wall tiles, zstd, back-pressure. Measure bandwidth | 2 wk | Typing into `studio`'s agent from `laptop` feels like ssh; 20 remote Wall tiles stay within budget |
| **P3 Review, explorer and git** | Changes, Review (reads, then commit, merge and discard with Full, or asked on the peer), worktrees view, explorer (chunked reads, search), `ReviewSource` routing in the app | 1–1.5 wk | Review and merge a `studio` worktree agent from `laptop` end to end |
| **P4 start agents, queue, approvals** | Start agents permission, "Runs on" with peer machines, `machine.form`/`kind.list`/`project.recent`, remote project picker, `session.add`, forwarded approvals for Full ("answer its approvals"), rules apply | 1–1.5 wk | Start a Claude agent on `studio` and an agw session through `studio` from `laptop`; answer `studio`'s approval on `laptop` |
| **P5 Windows, Linux, hardening** | Windows (firewall prompt, Tailscale CLI lookup, key file ACL), Linux (tun vs userspace mode message, optional SOCKS5 dialing), rate limits tuned, app-capability ceiling from ACL grants, direct LocalAPI client (optional), fuzzing of the peer handshake and frames, Headscale-based e2e job | 1.5–2 wk | Every pair of OSes works; fuzzers run in CI; security review done |

Total: about 9–11 weeks.

### Test strategy
- **Fake tailnet, two instances, one machine.** Two Pitwalls with separate
  `PITWALL_HOME`s, each built with the `fake-tailnet` cargo feature. That
  feature is never enabled in release builds, and a CI check fails if it is.
  - The `FakeTailnet` reads a JSON file shaped like `status --json`, with
    made-up nodes `laptop` and `studio` and users `alice@example.com` and
    `bob@example.com`.
  - Each instance listens on `127.0.0.1:<its port>` and dials **from** a
    fixed source port declared in the file.
  - `whois` maps `(127.0.0.1, source port)` to the fake node.

  This is deterministic on macOS, Windows and Linux. It needs no loopback
  aliases (macOS has only 127.0.0.1) and no root.
- **In-process tests.** The two engines run in one test process with
  `FakeProvider`, `MemStore` and a manual clock. The link and the listener
  are wired through `FakeTailnet` sockets, the same pattern as the existing
  client↔daemon socketpair tests. The tests cover:
  - pairing (allow, deny, code mismatch, one-time code, expiry)
  - identity changes (node, user, key) being refused
  - each permission level against every method in the table, generated from
    `METHODS` so a new method without a peer column fails the build
  - approvals: asked on the peer below Full, accepted with Full, forwarded
    approvals with first answer wins
  - the control lease and take-back
  - offline → stale → reconnect with replay
  - no loops (A↔B), and duplicate agw locators
  - back-pressure: screen frames dropped, terminal bytes kept
  - rate limits, oversized frames, garbage before `hello`
- **Security tests.**
  - Connections from outside the tailnet range, or failing `whois`, are
    closed without a byte.
  - The listener never binds `0.0.0.0` or `::`: a test enumerates the
    sockets.
  - Unauthenticated calls get `denied`.
  - The audit log has a line for every mutating call.
  - Fuzz targets cover `frame::read`, the hello, `peer.auth` and the stream
    mux.
- **UI tests.**
  - The mock store gains peer agents with link states.
  - Component tests check that the caps rule holds: no `peer:` string
    checks in views, enforced by the existing no-special-cases grep,
    extended.
  - Settings → Peers and the pairing dialog get their own tests.
- **Opt-in real runs.** `#[ignore]` tests use a real tailnet named in the
  environment (two machines the developer owns). They are read-only except
  for one throwaway shell agent. P5 adds a CI job with Headscale and two
  containers (Linux) for real `whois` and binding.

### Risks

| Risk | Mitigation |
|---|---|
| Control means remote code execution on the peer | Off by default, View default, plain wording, audit log, revocation, per-peer levels |
| `tailscale` CLI output changes or differs per platform/variant | One parser with fixtures per variant (from public docs examples), `--json` only, version shown in Settings, direct LocalAPI later behind the same trait |
| Binding to the tailnet IP fails (userspace mode, IP changes, Tailscale started later) | Rebind on status change, clear message, outgoing-only still works |
| Keystroke latency over DERP | Show the path and RTT, keep the interactive lane separate, never queue input behind bulk data |
| Bandwidth with many remote tiles | Frames instead of bytes, 4→1 fps adaptive, visible tiles only, zstd, drop-under-pressure |
| Two controllers fight over a terminal's size | Controller lease in `TermHost` |
| Version skew between machines | Caps-gated features, protocol range, clear "incompatible" state |
| Scope creep (sync settings, relay, multi-user) | Non-goals above |
| Windows firewall or macOS prompts confuse users | Ask only when the listener is turned on, with a sentence in Settings explaining why |

---

## 10. Open questions

1. **Default permission** offered in the pairing dialog for the same
   Tailscale user: View (proposed) or Control?
2. **Pairing remotely.** Is the one-time code from the listening machine
   enough, or should pairing also be allowed with no one at that machine
   when both nodes belong to the same Tailscale user (View only)?
3. **Full permission and approvals.** Is it right that with Full, the
   viewer's own confirmation counts as the peer's approval for merge,
   commit and discard? Or should those always ask on the machine where the
   code lives?
4. **Offline writes.** Refuse (proposed), or let Next-up additions queue on
   the viewer and deliver them on reconnect?
5. **CLI on the viewer.** Should `pitwall --peer studio agent list` exist,
   read-only and for the user's own terminal only, or stay out of v1?
6. **Port.** Is a fixed default port (47420 proposed) acceptable, or should
   discovery require an ACL tag on shared tailnets?
7. **Linux userspace-mode Tailscale.** Support outgoing links through
   Tailscale's SOCKS5 proxy in P5, or document it as unsupported?
