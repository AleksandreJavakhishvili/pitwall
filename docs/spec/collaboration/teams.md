# Collaboration: discovery, teams and keys

Part of the [collaboration spec](README.md).

## 3. Discovery

Tailscale does not carry broadcast or multicast between nodes, so mDNS and
LAN broadcast can't find Pitwalls on a tailnet ([References](references.md): Tailscale
forum). Pitwall finds them the way tailnet-peers.md §3 does, by listing the
tailnet and asking.

### Finding Pitwalls
- **List.** `tailscale status --json` (later LocalAPI directly), `Peer[*]`:
  online nodes with their `UserID`, `Tags`, `DNSName`, `TailscaleIPs`.
- **Filter.** Settings → Collaboration → "Look for teams on":
  - **Nodes tagged `tag:pitwall`** (default when the tailnet uses tags at
    all; the tag is configurable). This is how a tailnet admin opts devices
    in.
  - **Every online node of people in my tailnet** (default otherwise),
    capped below.
  - **Only devices I add by name** (for very large tailnets).
- **Probe.** `GET http://<tailnet IP>:47421/pitwall/v1/info` ([§8.4](protocol.md)). No
  answer means no Pitwall, or a Pitwall with collaboration off. A node that
  answers returns only `{ version, login, label, listedTeams, mcp: "/mcp" }`.
- **Limits.** At most 256 nodes per refresh, 16 at a time, 2 s connect
  timeout. A node that didn't answer is not probed again for 24 h, unless it
  changes in `tailscale status` (new IP, came online) or the user presses ↻.
- **When.** When People opens, every 30 min while the app runs and
  collaboration is on, and on ↻. Nothing at all while collaboration is off.
- **Cache.** `collab/discovered.json`: Pitwall nodes seen, with last-seen
  time and the listed teams they reported. The Teams page shows the cache at
  once and refreshes behind it.

### Finding teams
- `GET /pitwall/v1/teams:discover` on each Pitwall found returns the **listed** teams it
  knows: `{ team, name, description, admins: [login], members: n,
  version, join: "request"|"closed" }`. Any member can answer for a listed
  team, so a team is discoverable while its creator is offline. The answer
  is a signed record ([§4](teams.md)), checked before it is shown.
- **Invite-only teams** are never returned by `team.discover`. They are
  reached with an invite link only.
- The same team seen from several members is merged by team id; the highest
  valid version wins.

### Finding shared agents
- The directory is MCP's own `tools/list`: Pitwall calls it on the `/mcp`
  endpoint of each Pitwall that runs a member of the caller's teams, and
  gets the shared agents **that caller may use**, as tools ([§8.7](protocol.md)): name,
  description, capability, what's left today. No paths, no session data.
  People with no common team see nothing, unless a share names them
  directly or is open to the whole tailnet. A plain Claude user sees the
  same list in their client after `claude mcp add`.
- The directory is a cache (`collab/directory.json`), refreshed with
  discovery and when an owner announces a change (`shares.changed` event to
  connected teammates; `notifications/tools/list_changed` to MCP clients).
  An owner who is offline shows their agents dimmed, "alice is offline ·
  2 h", and requests to them fail at once (no offline queue in v1, [§14](plan.md)
  question 5).

---

## 4. Teams and keys

### 4.1 Keys

| Key | Where | Purpose |
|---|---|---|
| **Instance key** (Ed25519) | `peers/identity.key`, as in tailnet-peers.md §5 | Identifies one Pitwall installation. Signs team records (as admin, or for its own device entry). Requests are not signed: callers are identified by `whois` ([§8.2](protocol.md)). A device in a team is this key plus its node |
| **Admin authority** | No separate key: an admin is a person, and each of their devices' instance keys signs as admin | Signs team records |
| **Team id** | Hash (SHA-256) of the genesis record | Names the team forever. Two teams called "Platform" are different ids |

There are no team-wide secret keys to back up or share. Admin power sits on
the devices of the people who are admins, and a team with two or more
admins survives the loss of any one device or person.

### 4.2 Records
A team is a chain of signed records. Each record is the whole team state
after one change, plus what changed:

```json
{
  "team": "t_9f2c…",                  // genesis hash (absent in genesis)
  "version": 7,
  "prev": "sha256:…",                 // hash of record 6
  "ops": [{"op":"add_member","login":"carol@example.com",
           "devices":[{"key":"ed25519:…","node":"nXyz…"}]}],
  "state": {
    "name": "Platform", "description": "API, jobs, infra",
    "listed": true, "join": "request",
    "admins": ["alice@example.com", "dave@example.com"],
    "members": [
      {"login":"alice@example.com","devices":[{"key":"ed25519:A1…","node":"nStudio"},
                                               {"key":"ed25519:A2…","node":"nLaptop"}]},
      {"login":"bob@example.com","devices":[{"key":"ed25519:B1…","node":"nBob"}]},
      …
    ],
    "removed": [{"login":"eve@example.com","at":5}],
    "binding": null                   // §4.8
  },
  "by": {"login":"alice@example.com","key":"ed25519:A1…"},
  "at": "2026-10-16T09:12:00Z",       // informative only, never used for ordering
  "sig": "ed25519:…"                  // over the canonical JSON (RFC 8785 JCS) of all fields above
}
```

**Valid** means:
- **Genesis** (version 1, no `prev`): signed by a device listed in its own
  state, whose login is an admin. The team id is its hash.
- **Later records:** `prev` is the hash of the accepted version − 1, and
  `by.key` is a device of a person who is an **admin in the previous
  state**, the signature checks, and `state` equals the previous state with
  `ops` applied (so every member can recompute it).
- **Ops:** `add_member`, `remove_member`, `add_device`, `remove_device`,
  `rotate_device`, `add_admin`, `remove_admin`, `set_info`,
  `set_join`, `bind`, `unbind`.
- Records are small (members × devices × ~120 B), changes are rare, and the
  whole chain is kept: `collab/teams/<team>/chain.jsonl`.

### 4.3 Replication
- **Who holds it.** Every member device stores the chain of each team it is
  in. Non-members cache the latest listed record from `team.discover`.
- **Pull.** Whenever one member's Pitwall calls another's, the caller sends
  `team.heads [{team, version, hash}]` for teams they share and the answer
  carries the other side's heads. The side behind asks `team.records {team,
  after: version}` and checks each one. (These names map to the endpoints
  in [§8.4](protocol.md).)
- **Push.** After an admin makes a change, their Pitwall sends
  `team.announce {team, version, hash}` to every online member device it
  can reach. Receivers pull. A member that was offline catches up from
  whichever member it talks to next.
- **Highest valid version wins** on one linear chain. Every Pitwall serves
  only records it has validated.

### 4.4 Conflicts: two admins at once
Two admins may both write version n+1 on top of n (offline, or at the same
moment). That forks the chain. Every Pitwall picks the same branch:
1. **Fork point.** Find the last common record n. Compare the two first
   records after it (both n+1).
2. **Revocation wins.** If one of them removes the other's signer (as admin
   or member), or removes the signing device, that branch wins. A removed
   admin can't stay in power by racing.
3. **Removals before additions.** Otherwise, the record whose ops include
   any removal wins.
4. **Lowest hash.** Otherwise, the record with the lower hash wins.

The fork choice is decided at the fork point, never by branch length, so a
longer branch doesn't win by being longer. Records on the losing branch
are dropped. The admin who wrote them sees "Your change to Platform
conflicted with dave@example.com's; re-applying", and their Pitwall
re-applies the dropped ops on top of the winner when they are still valid
(adding a member already added is a no-op; acting as an admin who was just
removed fails and is reported).

**Mutual removal.** Two admins who remove each other at the same version
fall to rule 4, which is arbitrary. Teams with three or more admins can
require **two admin signatures** for `remove_admin` (a `cosig` field on the
record, collected through `team.cosign` requests); open question 1 asks
whether that is the default.

### 4.5 Bootstrapping trust
- **Creating.** Anyone creates a team: name, description, listed or
  invite-only, join by request or closed. Their Pitwall writes and signs the
  genesis record, with them as the only admin and member.
- **First contact (TOFU, pinned).** The first time Bob joins, he trusts the
  genesis record and the admins' keys he received, after checking two
  things Pitwall shows him:
  - **Who served it.** `whois` on the connection: "from alice@example.com
    on studio".
  - **The team fingerprint.** The first 6 characters of the team id, in
    groups (`9F2-C1A`), shown on the Teams page of every member. Read it
    out, compare it in chat.
  From then on the team id is pinned; a record that doesn't chain to it is
  rejected.
- **Invite links** carry the trust: `pitwall://team/join?t=<team id>&v=<version>
  &by=<admin login>&k=<admin key fp>&c=<one-time code>`. Opening one pins
  the team id from the link, so there is nothing to compare by hand.
- **Signed records protect against members who serve stale or forged
  state.** Any member can serve records, but only admins can sign them.

### 4.6 Joining, leaving, devices, rotation
- **Join by request.** Bob → `team.request_join {team, note}` to any online
  member. Members pass it on to admins they can reach (`team.request`
  events). Any admin approves in Requests, which writes `add_member` with
  Bob's device from the request (login from `whois`, key sent in the
  request and bound to that node). Requests expire after 7 days. At most 3 open requests per
  person per team.
- **Join by invite link.** An admin makes a one-time code (valid 7 days,
  one use; or "reusable until revoked", open question 2). Bob's Pitwall
  calls `team.redeem_invite` on the admin's device, or on any member device
  that holds the code's hash (the admin's Pitwall puts code hashes into a
  team-private field that members replicate but never show). The admin's
  Pitwall writes `add_member` when it next comes online, if the redeeming
  member's Pitwall can't sign. Until then Bob is "joining…" and sees only
  the team card.
- **Many teams.** A person may be in any number of teams. Teams are
  independent; nothing is shared between them except the person's devices.
- **New device.** Alice's new laptop asks to join as "another device of
  alice@example.com". Any admin, or Alice from a device already in the team
  (`add_device` is the one op a member may sign for their own login, and
  only when the requesting node's `whois` login is the same), approves it.
- **Key rotation.** Pitwall can rotate its instance key (Settings →
  Collaboration → Rotate key; also after a suspected leak). The device signs
  `{old, new}` with both keys and sends it to each team. Members accept a
  `rotate_device` record signed by the device's own old key, for its own
  entry only. Records signed with the old key are refused once the
  rotation is accepted.
- **Lost or stolen device.** An admin, or the same person from another of
  their devices, writes `remove_device`. The removed key can no longer sign
  records for this team. Access to shares is by login, so a person whose
  laptop was stolen also asks the Tailscale admin to remove that node:
  `whois` then fails and every request from it is refused at once.
  Removing the person from the team takes effect as fast as the record
  spreads; an owner's block list takes effect at once.
- **Leaving.** A member writes `remove_member` for themselves (allowed
  without admin, for their own login only). An admin removes others.
- **Last admin.** The last admin can't leave or step down without naming
  another admin. If every admin device is lost, the team is frozen: members
  keep working, nobody can change membership. Recovery is a new team
  (members can "Re-create team from this one", which copies the member list
  into a new genesis and invites them).

### 4.7 Members without Pitwall
A team member is a **login**. Devices are only needed to sign records and
to see the team in Pitwall, so a person who uses plain Claude Code (or any
MCP client) can be a full member for every share that names the team:
- **Adding.** An admin adds them by login: Team → Add member →
  `erin@example.com` (or `pitwall team add-member platform erin@…`). The
  record holds the login with `devices: []`. No join request is needed,
  since they have no Pitwall to send one from.
- **Setup lines.** Pitwall then shows a short text to send them: one
  `claude mcp add --transport http <owner> http://<node>:47421/mcp` line per
  member who shares agents with the team, and a note that their Claude
  will see the shared agents as tools. Owners can print their own line with
  `pitwall share setup`. When a new owner starts sharing with the team,
  plain members need one more line (Pitwall users get it automatically);
  the team page shows "Setup for people without Pitwall" with the current
  list.
- **Later Pitwall.** If they install Pitwall, their device joins as
  "another device of erin@example.com" ([§4.6](teams.md)).
- **What they can't do:** admin the team, see the member list in a UI, or
  share their own agents. They only call shares.

### 4.8 Binding a team to Tailscale (optional, for orgs with an admin)
Tailscale grants can give an **app capability** to traffic from some
users or devices to others. The receiving app reads it from `whois`
(`CapMap`), the way golink reads `tailscale.com/cap/golink` and tsidp reads
`tailscale.com/cap/tsidp` ([References](references.md)). Pitwall can use the same mechanism:

```json
"grants": [{
  "src": ["group:platform"],
  "dst": ["autogroup:member"],
  "app": { "<pitwall-domain>/cap/pitwall": [
    {"team": "platform", "admin": false},
    {"team": "platform", "admin": true, "logins": ["alice@example.com"]}
  ]}
}]
```

- **What binding means.** A team record with `binding = {capability,
  team: "platform"}`. For a bound team, a caller is a member if their `whois`
  `CapMap` has the capability with that team name. Admin is the same with
  `admin: true`. Pitwall stops accepting signed `add_member`/`remove_member`
  for bound teams; the Tailscale admin console is the source of truth.
  Plain Claude users in the group are members with no further step.
- **Who can bind.** An admin of the team, and only when their own `whois`
  shows the capability with `admin: true` for that team.
- **Ceiling.** The same capability can carry limits an org wants enforced,
  e.g. `{"maxCapability": "read"}` so nobody in the group can share with a
  "Can change code" capability. Pitwall
  applies the lower of the owner's setting and the ceiling.
- The capability name needs a domain the project controls (open question 3).
  The exact grant shape is checked with `tailscale whois` during P5 testing.
