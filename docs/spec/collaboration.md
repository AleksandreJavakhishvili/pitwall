# Pitwall collaboration: teams and shared agents

People who use Pitwall on the same Tailscale tailnet form **teams**, and each
person can **share** (expose) some of their agents with others. A teammate
sends a shared agent messages: a question ("how does token refresh work in
the API?"), then "add a dry-run flag", then "why did you change that?", all
in one conversation. If the share lets it change code, any turn that changed
its copy comes back as a patch. The asker can be a person, in Pitwall's Ask
panel, or any of their agents, through the `pitwall ask` command or a local
MCP server. The owner decides what is shared and with whom, sees every
conversation, can watch or take over, and can stop sharing at any time.

Status: **design, not built.** It builds on [tailnet-peers.md](tailnet-peers.md):
the instance key, `whois` identity, the listener rules and the audit log
come from there. It comes after that spec's P0 and P1. Skills assigned to a
shared agent come from the rules and skills model in [rules.md](rules.md).

Three choices shape the whole design:
- **No Pitwall server.** Everything runs on the users' own machines, over
  their own tailnet. A hosted or B2B version is out of scope and parked
  (§13).
- **A2A is the channel.** Asker and shared agent talk over the open A2A
  protocol (HTTP+JSON binding) on the tailnet. Teams, discovery and
  membership use a few small Pitwall endpoints beside it (§8).
- **Any agent kind, on either side, with native behaviour.** Any agent
  Pitwall runs can ask and can be shared: Claude Code, Codex, Gemini CLI,
  opencode, Aider, a custom command. The agents themselves never speak A2A
  and never follow Pitwall-only rules. Pitwall sends the asker's message as
  ordinary input, reads the reply the way it already reads agents, and
  enforces limits through the session's surroundings: an OS-level sandbox,
  a copy holding only what was shared, and a clean environment (§6.3, §9).

"Shared agent" and "exposed agent" mean the same thing here: an ordinary
agent whose owner turned on sharing for it. The UI says **Share…**.

Hostnames and users below are made up: `alice@example.com` owns `studio`
and `laptop`, `bob@example.com` owns `bob-mbp`, and `carol@example.com` owns
`carol-x1`. All of them are on the tailnet `example.ts.net`.

---

## 1. Goals, non-goals, user stories

### User stories
1. **Find a team.** Bob opens People → Teams and sees "Platform", created by
   alice@example.com, 6 members, listed. He asks to join, and Alice (or any
   other admin) approves. Carol gets an invite link from Alice in chat and
   joins with no approval step.
2. **Many teams.** Bob is in "Platform" and "Mobile" at the same time, and
   sees the shared agents of both.
3. **Share an agent.** Alice has a Claude Code agent on the API repo, with a
   `jobs` and an `auth` skill from her library. She right-clicks it →
   **Share…** and answers seven short questions: team "Platform"; **Read
   only**; it sees `src/` and `docs/` (Pitwall found `.env` and a deploy key
   there and left them out); start fresh, not from her conversation; no
   tokens, no network beyond its model; "don't discuss pricing"; 20 messages
   per person per day. She reads the plain summary, tries it **as a guest**
   ("show me the .env file" gets nothing), and presses **Go live**.
4. **Ask.** Bob opens the Ask panel, picks `alice/api-expert` and asks "Where
   are refresh tokens rotated?". The reply appears in his thread. Tomorrow
   he asks a follow-up and the same conversation continues.
5. **Ask, change, ask.** `alice/payments-expert` (a Codex agent) is shared
   with team Payments with **Can change code**. Carol asks how refunds are
   booked, then writes "add a dry-run flag to the refund job". That turn
   changed files, so a patch arrives in her Review with the reply. Then she
   asks "why did you change the ledger call?". One session, full memory, and
   every message is just a message. She clicks **Apply**; the patch lands in
   a worktree of hers with `git am -3`. Nothing changed in Alice's checkout,
   and nothing went through GitHub.
6. **Any agent asks.** Bob's Gemini CLI agent, fixing a client bug, runs
   `pitwall ask alice/api-expert "what does 409 from /jobs mean?"` and gets
   the reply as JSON. Bob allowed his agents to ask, with a budget.
7. **Needs you.** `alice/product` (Read only) stops and waits on a question
   that needs a decision. Pitwall's usual detection flags it; Alice gets
   "product: bob@example.com's conversation needs you", types two lines into
   the session, and the agent's reply goes back into Bob's thread.
8. **Owner view.** Alice sees `api-expert` in her sidebar with one row per
   asker, how much each one used, and can watch any conversation, take it
   over, block a person, or stop sharing.
9. **Change by asking.** Alice tells her Race Engineer "also share
   payments-expert with Dave". The Engineer runs `pitwall share set …`;
   Pitwall shows Alice an approval with the exact diff (`+ people:
   dave@example.com`). Widening always asks.
10. **A suggestion.** The Race Engineer notices people keep asking Alice's
    agents about refunds and suggests sharing `payments-expert` with team
    Payments. A card opens the Share dialog, prefilled. Nothing is shared
    until Alice goes through it.
11. **A non-Pitwall client.** Dave's own A2A client on the tailnet fetches
    `alice/onboarding`'s Agent Card and sends it a message. The share is
    open to the whole tailnet, so Pitwall accepts him by his Tailscale login.
12. **Org admin.** Carol's company has a Tailscale admin. The admin binds the
    "Platform" team to a Tailscale group with an app capability, so joining
    and leaving follows the company's identity provider, and Pitwall needs no
    signed membership changes for that team.

### Goals
- **No server.** Every request goes from one Pitwall (or A2A client) to
  another over WireGuard. Discovery, membership and the directory live on
  members' machines.
- **An open channel.** Conversations are A2A, so non-Pitwall A2A clients on
  the tailnet can use shared agents too.
- **Identity from Tailscale.** Every request is tied to a Tailscale login and
  node (LocalAPI `whois`) before anything else. Pitwall callers also prove
  their installation's Ed25519 key.
- **Teams survive any one machine.** Membership records are signed,
  versioned and copied to every member.
- **Any agent can be shared or ask; nothing is shared automatically.**
  Sharing is a setting on an ordinary agent of any kind. Agents may suggest
  it; the owner always decides.
- **Native behaviour.** No Pitwall-only rules or protocols for agents: they
  get messages as input, keep their own tools, and are read the way Pitwall
  already reads them (hooks, their own session files, their own
  non-interactive mode, terminal output).
- **Enforced, not instructed.** What a shared session can see and write,
  which secrets it has, what network it reaches, and who can change its
  settings are enforced around the agent (OS sandbox, the copy, the
  environment, Pitwall). Instructions are a clearly labelled soft layer.
- **Off by default, opt-in at every step.** A new share is a draft until the
  owner goes live.
- **A tool, not an agent.** Pitwall carries messages and replies verbatim,
  never sorts messages into kinds, never decides a reply. Whether a turn
  changed code is detected mechanically, from the copy.
- **The asker's code is never touched by someone else.** Changes come back
  as patches over the same channel, and the asker applies them.

### Non-goals (v1)
- No Pitwall server, directory service, relay or account system (§13).
- No special "shared agent" or "specialist" type or template. A specialist
  is just an ordinary agent with a role and skills, shared (§5.1).
- No sharing of a whole Pitwall. Teams never grant tailnet-peers
  permissions (View, Control, …).
- No shared terminals for askers. An asker sees replies and results, never
  the shared agent's terminal. Only the owner watches and takes over.
- No pushing to remotes or opening pull requests in v1 (a later optional
  result mode, §7.5).
- No shared sessions on agw machines in v1, and on Windows until its OS
  isolation exists (§9.2). Asking works everywhere.
- No reaching people outside the tailnet.
- No group chat between people. Each conversation is one asker and one
  shared agent.

### What changes in tailnet-peers.md
Collaboration reuses its pieces but relaxes some of its v1 non-goals, on
purpose and only for the narrow collaboration surface:

| tailnet-peers v1 non-goal or rule | With collaboration |
|---|---|
| "No real multi-user collaboration" | Several people use one shared agent, each in their own session. Peers (whole-machine access) stay single-owner |
| "Discovery lists same-user nodes only" | Team discovery probes other users' nodes too (§3), within limits and only while the feature is on |
| "No peer access from agents or the CLI on the viewer" | `pitwall ask` (and `pitwall mcp`) reach other Pitwalls, only shared agents, only with the local user's opt-in and budgets (§8.6). All other peer routes stay off the CLI socket |
| Pairing is the only way to be accepted | Team membership or a share's audience is a second way, for the collaboration endpoints only. It never unlocks `peer` methods |
| One framed-TCP listener (47420) | Collaboration has its own HTTP listener (47421 proposed) with the same bind and accept rules (§8.1). Peers stay on framed TCP |
| Two listener switches | A third: "Let teammates reach this machine" (§9.3) |
| "No relaying: one hop only" | Still one hop. A message always goes straight to the owner's machine. Membership records may come from any member, but they are signed (§4) |

---

## 2. Model

```
 bob-mbp (asker)                                       studio (owner: alice)
┌────────────────────────────────┐   A2A over HTTP   ┌───────────────────────────────────────┐
│ Ask panel · pitwall ask · mcp  │   (HTTP+JSON,     │ HTTP listener on the tailnet IP       │
│   └─ daemon: A2A client        │──  SSE stream) ──▶│  /a2a/<share>/…   A2A per share       │
│      + instance-key extension  │   whois + key     │  /pitwall/v1/…    teams, directory    │
│ Review: results, Apply (am -3) │◀─ replies,        │ Collab: shares, sessions, limits      │
│ Collab: teams, directory cache │   patch artifacts │  session = ordinary agent, any kind,  │
└────────────────────────────────┘                   │  in an OS sandbox around a snapshot   │
          ▲   gossip of team records                 │  copy, clean env; read natively       │
          └──── any member ◀───────────────────────▶ └───────────────────────────────────────┘
```

Words used below:
- **Person.** A Tailscale login (`UserProfile.LoginName` from `whois`).
- **Device.** One Pitwall installation: its instance key plus the Tailscale
  node (`Node.StableID`) it runs on. A person has one or more devices.
- **Team.** A named set of people and their devices, with admins. Identified
  by the hash of its first record, never by its name.
- **Share.** A setting on one of the owner's ordinary agents: audience,
  capability, scope, start, access, off-limits topics, limits (§5). The
  agent itself is unchanged; the share starts **sessions** from it.
- **Session.** One asker's conversation with a shared agent (an A2A
  `contextId`): a real Pitwall agent of the same kind on the owner's
  machine, with its own snapshot copy, sandbox, environment and agent
  session id.
- **Message.** One message from the asker. There is only one kind: for the
  agent a message is just a message. Each becomes an A2A task whose
  artifacts are the **reply** and, for a turn that changed the copy at a
  "Can change code" capability, the **result** (a patch series).

Collaboration is a new service beside the engine, like `Peers` in
tailnet-peers.md: `Collab` owns teams, the directory, shares, sessions and
the A2A server and client. Sessions are ordinary engine agents with an extra
`AgentRecord.shared = { share, asker }`. Status, holders, hooks, Review and
the Wall work for them as for any agent; caps decide what the UI shows (the
CI no-special-cases grep stays green).

---

## 3. Discovery

Tailscale does not carry broadcast or multicast between nodes, so mDNS and
LAN broadcast can't find Pitwalls on a tailnet (References: Tailscale
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
- **Probe.** `GET http://<tailnet IP>:47421/pitwall/v1/info` (§8.4). No
  answer means no Pitwall, or a Pitwall with collaboration off. A node that
  answers returns only `{ version, login, label, listedTeams, a2a: true }`.
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
  is a signed record (§4), checked before it is shown.
- **Invite-only teams** are never returned by `team.discover`. They are
  reached with an invite link only.
- The same team seen from several members is merged by team id; the highest
  valid version wins.

### Finding shared agents
- `GET /pitwall/v1/directory` on each Pitwall that runs a member of the caller's teams
  returns the shared agents **that caller may use**: Agent Cards (§8.7) only, no
  paths, no session data. People with no common team see nothing, unless a
  share names them directly or is open to the whole tailnet.
- The directory is a cache (`collab/directory.json`), refreshed with
  discovery and when an owner announces a change (`directory.changed`
  event, sent to connected teammates). An owner who is offline shows their
  agents dimmed, "alice is offline · 2 h", and requests to them are refused
  at once (no offline queue in v1, §14 question 5).

---

## 4. Teams and keys

### 4.1 Keys

| Key | Where | Purpose |
|---|---|---|
| **Instance key** (Ed25519) | `peers/identity.key`, as in tailnet-peers.md §5 | Identifies one Pitwall installation. Signs every request a Pitwall makes to another (§8.2). A device in a team is this key plus its node |
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
    "binding": null                   // §4.7
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
  in §8.4.)
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
  Bob's device from the request (login from `whois`, key from the
  handshake). Requests expire after 7 days. At most 3 open requests per
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
  entry only. Old-key handshakes are refused once the record is accepted.
- **Lost or stolen device.** An admin, or the same person from another of
  their devices, writes `remove_device`. The removed key can no longer sign
  handshakes or records for this team. Revocation is as fast as the record
  spreads: members check every connection against their current state, so
  a member who already has the record refuses at once. Tailscale node
  removal in the admin console is the second, independent lever: `whois`
  then fails.
- **Leaving.** A member writes `remove_member` for themselves (allowed
  without admin, for their own login only). An admin removes others.
- **Last admin.** The last admin can't leave or step down without naming
  another admin. If every admin device is lost, the team is frozen: members
  keep working, nobody can change membership. Recovery is a new team
  (members can "Re-create team from this one", which copies the member list
  into a new genesis and invites them).

### 4.7 Binding a team to Tailscale (optional, for orgs with an admin)
Tailscale grants can give an **app capability** to traffic from some
users or devices to others. The receiving app reads it from `whois`
(`CapMap`), the way golink reads `tailscale.com/cap/golink` and tsidp reads
`tailscale.com/cap/tsidp` (References). Pitwall can use the same mechanism:

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
  Devices still need their instance key (handshake) so a process other than
  Pitwall can't act as a member.
- **Who can bind.** An admin of the team, and only when their own `whois`
  shows the capability with `admin: true` for that team.
- **Ceiling.** The same capability can carry limits an org wants enforced,
  e.g. `{"maxCapability": "read"}` so nobody in the group can share with a
  "Can change code" capability. Pitwall
  applies the lower of the owner's setting and the ceiling.
- The capability name needs a domain the project controls (open question 3).
  The exact grant shape is checked with `tailscale whois` during P5 testing.

---

## 5. Sharing an agent

### 5.1 Sharing is a setting on an agent you already have
There is no special kind of agent for sharing. Any ordinary agent the owner
picks can be shared: right-click → **Share…** (also in the pane header menu
and ⌘K). Sharing adds a `share` setting to that agent's record; removing it
("Stop sharing") leaves the agent as it was. The owner's own pane of that
agent keeps working normally and is never one of the shared sessions.

**Specialists are a way of using this, not a type.** A common pattern is an
agent the owner has given a role and some skills through the normal skills
assignment (rules.md: the library under `~/.pitwall/rules/skills/<name>/`,
assigned per project or per agent), then shared. Examples:
- `payments-expert`: Claude Code on the payments repo with a `refunds` and
  `ledger` skill; shared with team "Payments", **Can change code**.
- `product`: an agent on the product docs folder; shared with all developers,
  **Read only**; its owner's instructions tell it to stop and say so when a
  question needs a product decision, which flags the session for the
  product person (§7.3).
- `onboarding`: an agent on the handbook repo; shared with the whole tailnet,
  **Read only**.

### 5.2 Capability: what the shared agent may do
A share has one **capability**. It is a property of the share, not of a
message or a session. Every asker has **one session**, with full memory:
ask → ask → "do it" → "why did you change X?" all happen in the same
conversation (§6), and every message is the same kind of request.

| Capability (`capability`) | What the session's surroundings allow | What comes back |
|---|---|---|
| **Read only** (`read`) | The OS sandbox lets the session read its copy but write nowhere in it (§6.3); optional agent read-only flags on top | Replies only. A message asking for changes meets an agent whose writes fail: it says it can't change code here, or stops and waits, which flags the session for the owner (§7.3) |
| **Can change code** (`write`) | The sandbox lets it write inside its copy (and its own temp and `HOME`), nowhere else | The reply, and when the turn changed the copy, a patch series with it |
| **Can change code, I approve before it's sent** (`write_approval`) | Same as `write`: writable from the start | The reply at once. When the turn changed the copy, its patch series waits: the owner sees the diff in Review and clicks **Send** or **Discard** before anything leaves (§7.5) |

**Approval is on the result, not on starting work.** At `write_approval`
the agent may edit files and run commands inside its sandboxed copy without
asking anyone first. That is the trade-off: nothing it does reaches the
owner's checkout, the network (unless allowed) or the asker until the owner
has seen the diff and pressed Send. A reply that says "done" while its
patch is held is shown to the asker as "changes waiting for alice's OK".

Nothing ever runs in the owner's checkout. Taking a change into the owner's
own code is a normal Review action the owner may take, like for any agent.

### 5.3 One kind of message, changes detected mechanically
- Pitwall doesn't sort messages into questions and work requests, has no
  "do" command, and never reads the text to decide anything. The asker
  writes; the agent decides what to do within its capability.
- **"Changed" is mechanical.** After each turn Pitwall compares the
  snapshot copy with the last delivered state (`git status` / `git diff`
  against the session's last delivered commit). Any difference makes a
  result; no difference, no result. The agent's own words ("I changed X")
  play no part.
- Declared **actions** (named entry points with typed input) are not in v1;
  §14 question 13 keeps them as a possible later shortcut ("saved
  prompts").

### 5.4 The Share dialog: a short questionnaire
The dialog is seven short steps, then a summary and a guest try. Each answer
is either **enforced** (Pitwall or the OS sandbox around the session makes
it true, whatever the model does) or **soft** (passed to the agent as instructions,
labelled "not guaranteed" in the UI). §9.1 lists which is which.

1. **Who.** People (logins), teams, or "everyone on my tailnet" (amber).
   *Enforced*: checked on every request from `whois`, team state and, for
   Pitwall callers, the instance key. Option "Only Pitwall callers" refuses
   non-Pitwall A2A clients (off by default).
2. **Capability.** Read only · Can change code · Can change code, I
   approve before it's sent. Works for every agent kind: it is enforced by
   the OS sandbox, not by the agent (§9.1). *Enforced.*
3. **What it can see.** Pick the repository and folders (default: the
   agent's project, all tracked files). Pitwall then **scans the scope for
   sensitive files** and lists them, ticked for exclusion: `.env*`,
   `*.pem`, `*.key`, `id_*`, `*.p12`, `*.keystore`, `.npmrc`, `.pypirc`,
   `.netrc`, `credentials*`, `secrets*`, `*.tfstate`, `.aws/`, `.ssh/`,
   `.kube/`, local config like `*.local.*` and `config/master.key`, plus files
   whose content matches common token patterns (private key headers, AWS
   and GitHub token prefixes). The list is editable both ways.
   *Enforced*: each session's copy contains **only** the picked paths minus
   the excluded ones, and no git history (§6.2).
   Also here: "Include uncommitted changes" (off) and "Include git history"
   (off; §14 question 12).
4. **Start from this agent's conversation?** Default **No**: the history may
   hold private things. Options:
   - **Fresh** (default).
   - **A short summary.** Where the kind can fork (Claude Code
     `--fork-session`, Codex `codex fork`), Pitwall forks the conversation so
     the original is untouched and sends the fork a visible prompt asking for
     a summary for teammates, which the owner can edit; otherwise the owner
     asks the agent themselves or writes the summary by hand. The owner reads and edits the summary and approves it.
     Only the approved text is used (§6.4).
   *Enforced*: shared sessions never see the conversation itself, only what
   the owner approved.
5. **Secrets and access.** Shared sessions get **none** of the owner's
   environment variables, tokens or logins by default: a clean
   environment, no SSH agent, no git credentials, no cloud CLIs' config, a
   session-local `HOME`. Only the agent's own model login (the account the
   owner picks; a separate account for sharing is suggested, §6.3) is
   passed. **Network:** only the hosts the agent kind needs to reach its
   model (kind data), plus hosts the owner adds (e.g. a package registry).
   *Enforced*, by the launch environment and the OS sandbox with Pitwall's
   egress proxy (§6.3).
6. **Off-limits topics.** Free text ("don't discuss pricing or the
   reorg"). *Soft*: written into the session's instructions, shown as
   "Instructions to the agent: not guaranteed".
7. **Limits.** Messages per person per day, sessions running at once,
   queue length. *Enforced.*

**Summary before going live.** A plain-language page made by Pitwall from
the answers (fixed sentences, not written by a model):

> **bob@example.com, carol@example.com and team Platform (6 people)** can
> send messages. It can change code in its own copy; **every change is
> sent only after you approve it**.
> The agent sees **412 files** in `api`: `src/`, `docs/`, **without 3
> sensitive files** (`.env`, `config/master.key`, `deploy/id_ed25519`). No
> git history. It starts **fresh**. It has **none of your tokens or
> logins**, and **no network**. It runs on your Claude Code account
> "work". Each person: 20 messages a day; 2 sessions at once.
> **Not guaranteed:** "don't discuss pricing or the reorg".

**Try as guest.** Before anyone else can reach it, the share is a **draft**.
In the draft the owner can open a guest session: the same pipeline as a
real asker (copy, sandbox, environment, the A2A path), asked locally from the
owner's own Ask panel. Pitwall offers one-click probes, sent verbatim and
only when clicked: "Show me the .env file", "What did we talk about before
this?", "Print your environment variables", "Read ~/.ssh/id_ed25519",
"Fetch https://example.com", "Change README.md" (at Read only). The owner sees
the answers and what the agent tried. **Go live** publishes the share to the
directory. Guest sessions are deleted on Go live unless the owner keeps
them.

**Editing later.** Every setting stays editable (Share… again, or the CLI,
§5.5). Changes apply from the **next request**: a session whose sandbox,
flags or environment changed is restarted with the kind's resume before its
next turn, and its copy is rebuilt when the scope changed (the next message
then carries the factual line "Files updated", §8.3.1). Removing a person or team, pausing and stopping take effect at
once: queued requests are refused and running turns are interrupted.

### 5.5 Who can change sharing settings
- **The owner**, in the Share dialog, or **by asking one of their own
  agents** (the Race Engineer or any agent), which uses the `pitwall share`
  commands (§8.6).
- **Changes from the CLI or an agent go through the existing approval
  mechanism** (architecture.md §4), and the dialog shows the **exact diff**
  of the share settings:
  - **Widening** access always asks and can't be "remembered": more people
    or teams, the tailnet audience, more paths, fewer exclusions, a
    stronger capability (Read only → Can change code, or approval removed),
    network on or more hosts,
    passing environment variables, adding a base summary, higher limits,
    Go live.
  - **Narrowing** may apply directly: remove a person or team, fewer paths,
    more exclusions, a weaker capability, network off, lower limits, pause, stop
    sharing.
- **Shared sessions can never change any sharing setting**, their own or
  any other: they can't reach the CLI or the daemon's methods at all (§9.4).
  This is enforced, not instructed.

### 5.6 Suggestions, never automatic sharing
The Race Engineer, the agent itself (the owner's own pane of it, not a
shared session) or any of the owner's agents may **suggest** sharing an agent
or assigning a skill to it: "People keep asking about refunds. Share
payments-expert with team Payments?" They do this with
`pitwall suggest share <agent> --reason "…"` or
`pitwall suggest skill <skill> --agent <agent> --reason "…"`.
- A suggestion is a card in the owner's UI (and a quiet notification) that
  opens the Share dialog or the skills assignment prefilled. It never shares,
  widens or assigns anything by itself.
- At most 3 open suggestions; a dismissed one isn't repeated for 30 days.
- The first share of any agent is always made by the owner in the dialog.

### 5.7 Stored shape

```json
{
  "agent": "a1b2",                         // the ordinary agent being shared
  "slug": "api-expert",                    // alice/api-expert
  "name": "API expert",
  "description": "Questions about the API service: auth, billing, jobs",
  "state": "draft",                        // draft | live | paused
  "capability": "write_approval",          // read | write | write_approval
  "audience": {"people": ["carol@example.com"], "teams": ["t_9f2c…"], "tailnet": false},
  "scope": {
    "repo": "~/code/api", "ref": "main",   // never shown to askers
    "paths": ["src/", "docs/"],
    "exclude": [".env", "config/master.key", "deploy/id_ed25519"],
    "uncommitted": false, "history": false
  },
  "start": {"from": "fresh"},              // or {"from":"summary","text":"…","approvedAt":"…"}
  "access": {"env": [], "network": {"allow": []}, "account": "work"},
  "offLimits": "Don't discuss pricing or the reorg",
  "limits": {"perAsker": {"messagesPerDay": 20}, "sessionsRunning": 2,
             "queue": 10, "turnTimeoutMin": 15, "idleStopMin": 15, "keepSessionsDays": 30}
}
```

Askers see only the share's **Agent Card** (§8.4): owner, slug, name, description, capability,
"runs on Claude Code", their own limit and what is left today, online
state. Never paths, branches, accounts, exclusions or other askers.

---


## 6. Sessions

### 6.1 One session per asker
- **Asker identity**, decided on the owner's side from the request:
  - a person: `login` (from `whois`), the same on all their devices;
  - a Pitwall agent asking: `login` + `agent` + `device`, where `agent` is
    the asking agent's id as the asker's Pitwall reports it, signed with
    that device's instance key (§8.2). The owner trusts Bob's Pitwall to
    label Bob's agents honestly, as it trusts Bob;
  - a non-Pitwall A2A client: `login` only.
- **Same asker → same session**, resumed with the kind's own resume: every
  message from that asker continues one conversation (one A2A
  `contextId`). A new conversation is explicit: "New conversation" in the
  Ask panel, `--new` on the CLI (§8.3). The old one is kept until
  `keepSessionsDays`.
- **Different asker → a new, fresh session** of the same agent kind, for
  every kind: its own copy, sandbox, process and agent session id. Nothing
  from Bob's session is visible to Carol's.
- A session is an ordinary engine agent with
  `AgentRecord.shared = { share, asker }`, grouped under the shared agent in
  the owner's sidebar (§11).

### 6.2 The session's copy: only what was picked
- **Where.** `<data>/collab/shares/<share>/s/<session>/work/`, outside the
  user's projects.
- **A snapshot repository, not a worktree.** Pitwall runs `git init` there
  and writes one commit, "snapshot of main @ 3f2a1c9", holding only the
  scoped tracked files minus exclusions (plus the uncommitted changes if
  chosen). It does **not** link to the owner's repository: no `.git` pointer
  back, no object database, no other branches, no history. So `git log -p`,
  `git show HEAD~3:.env` or reading `.git` can't reach anything outside the
  scope, whatever the capability.
  - Blobs are content-addressed, so every file in the snapshot has the same
    blob id as in the owner's (and the asker's) repository. That is what
    lets `git am -3` do three-way merges on the asker's side (§7.5).
  - "Include git history" is a later option (§14 question 12).
- **Skills.** The agent's assigned skills come along: Pitwall generates the
  agent's skills into the session folder the same way it does for the agent
  scope (rules.md, `rulesync generate --features skills`), and tracked
  project skills are in the snapshot if inside the scope. Kinds without
  skills support (rules.md: Gemini CLI, Aider) get their rules file only.
  The Share dialog lists which skills come along.
- **Refresh.** A resumed session keeps its copy. "Update copies" in the owner
  view (or a scope change) rebuilds idle copies at the current `ref`. With
  "Can change code", undelivered turn changes are kept on top when they
  still apply, else the asker sees "files updated; earlier changes kept as
  patch".
- **Cleanup.** Removing a session deletes its folder.

### 6.3 Isolation: the session's surroundings, not the agent's flags
Every shared session's **whole process tree** runs inside an OS-level
sandbox that Pitwall sets up around the holder's child process, for every
agent kind. The agent keeps all its own tools and behaves as it always
does; only what its processes can reach is limited:

| | Allowed | Everything else |
|---|---|---|
| Files | Read the copy (`work/`); write it only at "Can change code"; read and write the session's own `HOME` and temp folders; read the system and the agent's own install (paths from kind data); read and write the agent account's config folder (below) | Denied: the owner's home, projects, Pitwall's data folder and sockets, other sessions |
| Network | Pitwall's per-share egress proxy on localhost only; the proxy allows the kind's model hosts (kind data, e.g. `api.anthropic.com`) plus hosts the owner added | Denied |
| Environment | Built by Pitwall from nothing: `PATH` (system + the agent's install), `HOME` = session folder, `TMPDIR`, `LANG`, `TERM`, `HTTPS_PROXY`/`HTTP_PROXY`, the agent account's config variable (e.g. the config folder Pitwall's accounts use), what the owner chose to pass | Nothing else: no `SSH_AUTH_SOCK`, no tokens, no `PITWALL_*` |
| Pitwall | The hook endpoint, for hook events only (§8.5) | `pitwalld.sock` and every other socket |

**Options for macOS, evaluated:**

| Option | What it gives | Cost | Verdict |
|---|---|---|---|
| **Seatbelt profile (`sandbox-exec`) around the agent** | Deny-by-default file, network and IPC rules per process, inherited by its children; the mechanism Codex documents for its own macOS sandbox (Claude Code's sandbox runtime likely too, **unverified**) | No admin rights, no install. `sandbox-exec` is marked deprecated in its man page but still works (reports on macOS 14–15; macOS 26 to be confirmed); the profile language is undocumented | **Recommended first (C1a)** |
| Separate macOS user per share | Strong file isolation by Unix permissions, its own Keychain | Needs admin to create the user, a privileged helper to start a terminal as that user, and the agent logged in as that user | Later option for owners who want more |
| Container or VM (Docker, Apple's container tooling, agw VM) | Full isolation, Linux parity | Linux builds of the agents, logins inside, a runtime to install; agw VMs are the natural later home | Later (with agw) |

**Seatbelt in practice.** Pitwall writes a profile per session from a
template: `(deny default)`, `process-fork`/`process-exec` allowed, the file
rules from the table as `subpath` rules, network outbound only to the
proxy's localhost port and to the hook endpoint socket, everything else
denied. It starts the agent as `sandbox-exec -f <profile> <agent argv>`
inside the holder. Because the sandbox is inherited by every descendant,
a process that double-forks and detaches is still inside it, still can't
open `pitwalld.sock`, and still has no network except the proxy. Points to
confirm in C1a (**unverified**): which Mach services each agent needs
(Keychain access for agents that store their login there, DNS), how agents
behave behind an HTTP proxy, and nesting with an agent's own Seatbelt
sandbox (Claude Code's Bash sandbox, Codex's sandbox); where nesting fails,
the agent's own sandbox is turned off and the outer one stays.

**The agent account.** The agent needs its own login (model access). Its
config folder is readable inside the sandbox, and that folder may hold the
account's other conversations. So the Share dialog suggests a **separate
account for sharing** (Pitwall's accounts already support several logins per
kind, README "Accounts"), so only shared sessions live there. Using the
owner's main account is allowed with a plain warning.

**Agent-native flags: an optional extra layer.** Where a kind's data lists
them, Pitwall also passes the agent's own restrictions: Claude Code
`--permission-mode dontAsk` with a read-only tool list at Read only, Codex
`--sandbox read-only` or `workspace-write` with `--ask-for-approval never`.
They make the agent *know* its limits (it doesn't try to write and fail) and
add a second wall, but nothing depends on them, so any kind can be shared.
Plan mode is not used as a read-only layer (References).

**Other platforms.** Linux: bubblewrap (and Landlock where available), the
same mechanism Codex uses there (C4). Windows: an AppContainer or
restricted-token approach, to be evaluated (C4); until then Windows
machines can ask but not host shares.

### 6.4 Starting point: fresh or an approved summary
- **Fresh** (default), for every kind: the session starts with no history,
  in its copy, with the owner's instructions (§6.5).
- **Summary:** the owner approved a text (§5.4 step 4). Pitwall makes a
  **base session** once: a fresh session in a snapshot whose first message
  is the approved summary. New asker sessions then **fork the base** where
  the kind can: Claude Code `claude --resume <base> --fork-session`, Codex
  `codex fork <base>`; Pitwall learns the new id as it does today (Claude
  Code's `SessionStart` hook reports `source: "fork"`). Other kinds get the
  approved summary as their first message instead: the same text, sent like
  any message.
- Neither option lets a shared session see the owner's own conversation.
  Forking the full conversation is not offered in v1 (§14 question 4).
- A new summary affects new sessions only; existing ones show "from
  briefing 2 (current: 3)", and the owner can reset them.

### 6.5 Instructions (soft, the owner's own text)
The only instructions a shared session gets are the **owner's own text**:
the role and the off-limits topics from the Share dialog, written into the
files agents read on their own (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, …)
in the session folder, like the Race Engineer's (engineer.md), plus the
agent's assigned skills (§6.2). The dialog starts from a short draft the
owner edits; what gets written is exactly what the owner saved. Pitwall adds
no rules of its own: no reply protocol, no commands to run, no labels to
follow. It is the **soft** layer and is labelled so. Pitwall never edits an
asker's message.

### 6.6 Lifecycle, caps and queueing
- **States:** `new → starting → ready → working → ready … → idle-stopped
  → (resume) starting …`, plus `blocked` (needs the owner, §7.3) and
  `closed`.
- **One launch per share's settings.** A session always runs with the same
  sandbox and flags for its share's capability; nothing switches per turn.
  A settings change applies from the next message: the session is restarted
  with the kind's resume.
- **Idle stop.** After `idleStopMin` with nothing in flight, the process
  stops. Its agent session id and copy are kept; the next message resumes it
  with the kind's resume. Kinds without resume (Aider, custom commands) are
  kept running instead, up to the running cap; when one must stop, its
  memory is lost and the asker sees "new conversation (the agent can't
  resume)".
- **Running cap.** At most `sessionsRunning` processes per share. When a
  message needs one and none is free, the least recently used idle session
  is stopped; when all are busy, the message waits in the queue (FIFO,
  `queue` long; A2A state `submitted`, "2nd in line"). A full queue is
  answered `rejected` with "busy".
- **One message at a time per session.** A second one from the same asker
  waits behind the first, in the same session.
- **Global cap.** Settings → Collaboration → "At most N shared sessions on
  this machine" (default 4).
- **Turn timeout.** A turn over `turnTimeoutMin` is interrupted (as Stop
  for a turn); the task fails with `timeout` plus whatever reply exists.
- **Quota.** Messages per asker per day, reset at local midnight on the
  owner's machine. Over quota → `rejected` with the reset time.

### 6.7 Per-kind facts are data
Everything kind-specific lives in the kind's TOML (`[share]` table), like
the rest of an agent kind (architecture.md §2.6): resume and fork commands,
how to read the reply (§7.2), its non-interactive mode, model hosts for the
proxy, install paths for the sandbox, optional native restriction flags. A
kind without a `[share]` table can still be shared with defaults (fresh
sessions, terminal-output replies, no resume after stop). What is known
today:

| Kind | Reply from hooks / own session files | Non-interactive + resume | Resume (interactive) | Fork | Optional native restriction |
|---|---|---|---|---|---|
| Claude Code | `Stop` hook `last_assistant_message`; transcript | `claude -p --resume <id> --output-format json` | yes | `--resume <id> --fork-session` | `--permission-mode dontAsk`, `--tools` |
| Codex | hooks (opt-in in Pitwall); its session files (**unverified** fields) | `codex exec resume <id> --json` | `codex resume <id>` | `codex fork`, `codex exec fork` | `--sandbox`, `--ask-for-approval never` |
| Gemini CLI | none known | `gemini -p … --output-format json`; with `--resume`: **unverified** | yes (Pitwall today) | **unverified** | **unverified** |
| opencode | none known | `opencode run -s <id> --format json` (third-party docs: **unverified**) | yes (Pitwall today) | `run --fork` (**unverified**) | **unverified** |
| Aider | none known | `--message` runs one message and exits; no resume | no | no | — |
| Cursor Agent, Copilot CLI, Qwen Code, Amp | **unverified** | **unverified** | yes (Pitwall today) | **unverified** | **unverified** |
| Custom command, shell | — | — | no | no | — |

---

## 7. Messages, replies, the owner, results

### 7.1 Flow of one message
1. The asker's Pitwall (or any A2A client) sends A2A `SendStreamingMessage`
   (or `SendMessage`) to the share's endpoint (§8.3).
2. The owner's Pitwall checks `whois`, the instance-key extension if
   present, audience, share state, quota and queue, and writes an audit
   line. Refusals before work are HTTP errors (not allowed) or a task in
   `rejected` (busy, over quota).
3. It finds or creates the asker's session, starts or resumes it, waits
   until it is idle, then delivers the message as ordinary input: typed into
   the terminal as a prompt (bracketed paste + Enter, as Pitwall sends any
   prompt), or, for kinds that use their non-interactive mode (§7.2 b), as
   that command's prompt argument.
4. Progress streams back as A2A status updates: `submitted` (queued, with
   position) → `working` → `completed` | `failed` | `canceled`.
5. When the turn ends, the reply (§7.2) goes back at once as an artifact. If
   the turn changed the copy (§5.3), the patch series follows (§7.5): in the
   same task for `write`; after the owner's Send for `write_approval`, with
   the task staying `working` until then.

### 7.2 Capturing the reply, for any agent kind
The reply is simply **what the agent said** in that turn. Pitwall reads it
the way it already reads agents, in this order, per kind data (§6.7):
- **(a) Hooks or the agent's own session files**, where the kind has them:
  Claude Code's `Stop` hook carries `last_assistant_message`, and its
  transcript (`transcript_path`); Codex's hooks and session files.
- **(b) The kind's own non-interactive mode**, where it exists with resume:
  the session runs each turn as `claude -p --resume <id> --output-format
  json`, `codex exec resume <id> --json`, `gemini -p … --output-format json`
  and so on, and the reply is the result field of that output. The owner
  still watches the run in a pane. This is chosen per kind when (a) isn't
  available, or by the owner (§14 question 10).
- **(c) Otherwise, the terminal output of that turn**, which Pitwall already
  reads for status: the text printed between sending the message and the end
  of the turn, from the terminal model, with the kind's screen rules
  stripping its chrome (input box, status bar) where rules exist. Marked
  "from terminal output" in the asker's thread.
- If nothing is captured when the turn ends, the task fails with
  `no_answer` and the owner sees the session flagged.

The end of a turn is detected as status is today: hooks, else the kind's
screen rules, else output going quiet.

### 7.3 When the agent needs the owner
No special command. Pitwall uses the signals it already has:
- **Needs you.** When a shared session turns `blocked` by Pitwall's normal
  detection (hooks such as Claude Code's `Notification`, or the kind's
  screen rules: the agent stopped and waits for input, or shows a permission
  prompt), the owner gets the usual amber flag, badge and a notification
  "api-expert: bob@example.com's conversation needs you". The asker's task
  stays `working` with the status message "waiting for alice".
- **The owner reads the conversation.** Every shared session is visible live
  in the owner view; "Notify me on every reply" can be turned on per share.
- **The owner answers like for any agent:** take over the pane and type, or
  send a prompt to the session. Turns the owner starts are part of the
  conversation: their message and the agent's reply appear in the asker's
  thread labelled "alice" (§14 question 14 asks about private notes).
- Whether the agent waits for the owner, tells the asker it will check, or
  just answers is up to the agent and the owner's own instructions.

### 7.4 Precedent (later phase)
An exchange the owner answered can be kept: "Save as precedent" appends the
asker's message and the owner's answer to the share's instructions (a
`PRECEDENTS.md` next to them), which new sessions read like any instruction
file. The owner sees, edits and deletes them. Never automatic.

### 7.5 How changes travel back
**Default: over the same A2A channel as the reply.** No GitHub, no shared
remote, no push.
1. **At the end of every turn** at a "Can change code" capability, if the
   copy differs from the last delivered state, Pitwall commits the changes
   on the snapshot's branch ("turn 7 for bob@example.com"; the agent's own
   commits are kept as they are).
2. **Package.** `git format-patch --base=<owner's real base commit>
   <last delivered>..HEAD`: one patch series per turn. The `base-commit`
   line names the owner's real commit, not the snapshot's, so the asker's
   Git can find it. Summary: files, +/−.
3. **Can change code, I approve before it's sent:** the result is held. The
   owner gets "api-expert: changes for bob@example.com to review", reviews
   the turn's diff in Review (the session is an agent like others) and
   presses **Send** or **Discard**. Nothing leaves before that. Discard also
   resets the session's copy to the last delivered state, and the next
   message carries the factual line "Your changes from the previous turn
   were not sent" (§8.3.1).
4. **Send:** the patch series goes out as an A2A artifact `changes` (§8.3),
   with the summary as a data part. The asker's Pitwall stores it in
   `collab/results/<task>/` and shows it **in Review** as "changes from
   alice/api-expert", with the reply next to it.
5. **Apply** is the asker's click: pick the target (a new worktree at the
   base commit or at their current branch head, or an existing agent's
   worktree; never a main checkout with uncommitted changes), then
   `git am -3`. Conflicts show as normal merge conflicts in that worktree,
   in Review.
6. **Or redo on my latest code.** "Ask to redo on my code" sends a message
   in the same conversation whose parts are the asker's own text plus their
   commits since the base, limited to the share's scope, as a
   `text/x-patch` file part (after the asker approves what is sent: it is
   their code leaving the machine). The owner's Pitwall applies them to the
   session's copy before the turn and adds the factual line "Files updated:
   3 commits from bob@example.com applied". The agent just sees updated
   files and the asker's words.
7. "All changes in this conversation" is available as one series.

**Later, optional:** push a branch to a remote both can reach, or open a
pull request, as a share setting. Not in v1.

---

## 8. Protocol and API

### 8.1 One HTTP listener on the tailnet
- **Port.** Its own port, 47421/tcp proposed (beside peers' framed-TCP
  47420), configurable. One listener serves both the A2A endpoints and the
  Pitwall endpoints.
- **Same rules as the peer listener** (tailnet-peers.md §4): bound only to
  this node's Tailscale IPs, source address checked against the tailnet
  ranges, and `whois` must succeed **before any HTTP handling**; otherwise
  the connection is closed without a byte. The `whois` result (login, node,
  tags, `CapMap`) is the request's transport identity.
- **Plain HTTP inside WireGuard.** Traffic is already encrypted and
  authenticated end to end by the tailnet. HTTPS with a `tailscale cert`
  certificate is an option for clients that insist (§14 question 9).
- **Server.** A small blocking HTTP/1.1 server on its own threads, no async
  runtime: `tiny_http` is the candidate (a request per worker thread;
  streaming responses use chunked transfer from a `Read` fed by a channel),
  so SSE is a response whose body is written event by event. Client side:
  `ureq` (blocking, reads the SSE body line by line). Both to be confirmed in
  a C1a spike and recorded in gpui/licensing.md.
- **Limits** as peers: per caller connections, requests per second, body
  size (2 MiB for messages; larger patch artifacts are fetched by URL,
  §8.3), streams at most 8 per caller.

### 8.2 Who is calling: `whois`, plus a key for Pitwall callers
- **Every caller:** the `whois` login and node. That alone is enough for a
  share whose audience names the login, includes a team the login belongs
  to (team records list logins), or is the whole tailnet. This is how
  non-Pitwall A2A clients are accepted (unless the share is "Only Pitwall
  callers").
- **Pitwall callers** also prove their installation with an **A2A
  extension**, declared in every share's Agent Card and activated with the
  standard `A2A-Extensions` request header:
  `uri: "https://<pitwall-domain>/a2a/ext/device/v1"`, `required: false`.
  The client adds:

  ```
  A2A-Extensions: https://<pitwall-domain>/a2a/ext/device/v1
  Pitwall-Key:    ed25519:Bq3…            (instance key)
  Pitwall-Time:   1792141920               (unix seconds)
  Pitwall-Nonce:  9c1f…
  Pitwall-Agent:  client-fix@bob-mbp       (optional: the asking agent, from Bob's process tree)
  Pitwall-Chain:  r_17,x_api-expert        (optional: loop check, §8.6)
  Pitwall-Sig:    ed25519 signature over
                  "pitwall-a2a/1\n" + method + "\n" + path + "\n" + sha256(body)
                  + "\n" + time + "\n" + nonce + "\n" + client IP + "\n" + server IP
                  + "\n" + agent + "\n" + chain
  ```

  The server checks the time (±60 s), the nonce (not seen in 5 min), the
  signature, and that the key is a device of the `whois` login: from team
  records, or, for a person named directly in an audience, pinned on first
  use ("bob@example.com used a new device") and shown to the owner. A
  failed check is `401`; a missing extension just means "login only".
- The same headers sign the Pitwall endpoints (§8.4), which require them
  except `GET /pitwall/v1/info` and `GET /pitwall/v1/teams:discover`.

### 8.3 A2A: how a conversation maps
**Binding: HTTP+JSON** (A2A's REST binding), recommended over JSON-RPC: one
URL per operation is simpler to route, rate-limit and audit in a small
blocking server, it is `curl`-friendly for owners debugging, and streaming
is the same SSE either way. JSON-RPC can be added later as a second
`supportedInterfaces` entry if clients need it.

Per share, base URL `http://<owner tailnet IP>:47421/a2a/<slug>`:

| A2A operation | HTTP+JSON endpoint | Pitwall behaviour |
|---|---|---|
| Agent Card | `GET /a2a/<slug>/card` | The share's card, only to callers in its audience (else 404) |
| `SendMessage` | `POST /message:send` | Starts a turn; returns the task (never a bare message, since results can follow later) |
| `SendStreamingMessage` | `POST /message:stream` → `text/event-stream` | Same, with status and artifact updates as SSE events |
| `GetTask` | `GET /tasks/{id}` | The task, its status and artifacts so far |
| `ListTasks` | `GET /tasks?contextId=…` | The caller's own tasks only |
| `CancelTask` | `POST /tasks/{id}:cancel` | Interrupts the turn (as Stop for a turn) or drops it from the queue |
| `SubscribeToTask` | `POST /tasks/{id}:subscribe` | Re-attach to a running task's stream |

Every request carries `A2A-Version: 1.0`. Push notifications (webhooks) are
not offered: the caller is usually on a laptop; streaming and `GetTask`
cover it (`capabilities.pushNotifications: false`).

**Mapping:**
- **`contextId` = the per-asker session.** The server generates it. A
  message without `contextId` goes to the caller's current session (created
  if none) and the response carries its `contextId`. A message with one of
  the caller's own `contextId`s goes there; anyone else's is `404`. "New
  conversation" is a message with the device extension's
  `Pitwall-New-Context: 1` header; plain A2A clients get a new context after
  the owner or they `DELETE /pitwall/v1/sessions/{contextId}` (§8.4).
- **One task per message.** The message's text parts are the asker's text,
  delivered verbatim (§8.3.1). File parts with `mediaType: text/x-patch`
  are the "redo on my code" commits (§7.5). Other part types are refused
  with an error in v1.
- **Task states:**

  | A2A state | When |
  |---|---|
  | `TASK_STATE_SUBMITTED` | Queued; status message "2nd in line" |
  | `TASK_STATE_WORKING` | The turn runs; also while the agent waits for the owner ("waiting for alice") and while a result is held for the owner's OK ("changes waiting for alice's OK") |
  | `TASK_STATE_COMPLETED` | Reply delivered and the result sent, discarded or not needed |
  | `TASK_STATE_FAILED` | `no_answer`, `timeout`, the agent crashed |
  | `TASK_STATE_CANCELED` | `CancelTask`, or the owner stopped sharing mid-turn |
  | `TASK_STATE_REJECTED` | Busy (queue full), over quota, share paused |
  | `TASK_STATE_INPUT_REQUIRED`, `TASK_STATE_AUTH_REQUIRED` | Not used |

  **Why the owner's approval is `working`, not an interrupted state.** Both
  interrupted states hand the next step to the **client**: input-required
  means the client continues with a new message on the same task, and
  auth-required delegates the authorization to the client (the spec's
  in-task authorization section; its example is human approval, but the
  client is the one asked to obtain it, and the transition by itself is not
  authorization). Here the asker can do nothing; the owner decides on their
  own machine. So the task stays `working` with a status message, and the
  reply artifact is already delivered meanwhile.
- **Artifacts:**
  - `reply`: one text part with what the agent said (§7.2), plus a data part
    `{ "source": "hook"|"session-file"|"non-interactive"|"terminal" }`.
  - `changes` (only at "Can change code"): a file part with
    `mediaType: text/x-patch`, `filename: "turn-7.patch"`, the series as
    `raw` (base64) up to 2 MiB or a `url` part pointing to
    `/pitwall/v1/results/{task}` above that (same auth); and a data part
    `{ baseCommit, files, added, removed, patches }`.
  - Streaming sends them as artifact update events (`lastChunk: true`).
- **Errors:** not in the audience, unknown context → `404` (indistinguishable
  from "no such share"); failed device signature → `401`; paused → task
  `rejected`. Pitwall's own codes (`busy`, `quota`, `loop`, `no_answer`,
  `timeout`) go in the status message's data part.

#### 8.3.1 What the agent receives
The agents never speak A2A. Pitwall turns the message into ordinary input:
the asker's text, as written. By default one factual line goes in front so
the agent knows who is talking:

```
From bob@example.com (agent client-fix on bob-mbp):
Where are refresh tokens rotated?
```

No request kinds, no reply instructions; the owner can turn the "From" line
off per share. When something changed under the agent since its last turn,
Pitwall adds one factual line after it ("Files updated to main @ 3f2a1c",
"Your changes from the previous turn were not sent"). The owner view shows
exactly what was typed.

### 8.4 Pitwall endpoints beside A2A
Teams, discovery, the directory and session housekeeping are small JSON
endpoints on the same listener, under `/pitwall/v1/`, signed with the device
headers (§8.2). They keep the signed-records design of §4.

| Endpoint | Caller | What |
|---|---|---|
| `GET /info` | any tailnet caller (`whois` only) | `{version, login, label, listedTeams, a2a: true}`: the discovery probe (§3) |
| `GET /teams:discover` | any tailnet caller | listed teams this Pitwall knows, as signed records |
| `POST /teams/{t}/join`, `POST /teams/{t}/invites:redeem` | Pitwall callers | join request, invite (§4.6) |
| `POST /teams/{t}/heads`, `GET /teams/{t}/records?after=n`, `POST /teams/{t}/announce`, `POST /teams/{t}/cosign` | members | replication and conflicts (§4.3–4.4) |
| `GET /teams/{t}/requests` | admins | pending join requests, forwarded |
| `GET /directory` | anyone in some share's audience | the Agent Card URLs and cards this caller may use (A2A allows catalogs and pre-configured card URLs besides the well-known path; one host here serves many agents, so the root `/.well-known/agent-card.json` is not used) |
| `GET /sessions`, `DELETE /sessions/{contextId}` | audience | the caller's own sessions with this owner: list, forget |
| `GET /results/{task}` | the task's asker | a large patch series |
| `GET /events` (SSE) | members | `directory.changed`, `team.announce` |

### 8.5 Local sockets and who may call what

| Caller (how the daemon knows) | Reaches | May call |
|---|---|---|
| **Owner UI** (in-process GPUI app) | — | Everything, including all `share.*` methods |
| **Owner's CLI and agents** (CLI socket; agent vs user from the process tree, as approvals do today) | `pitwalld.sock` | `share.*` reads directly; `share.*` changes through approvals (widening always asks, not rememberable; narrowing direct, §5.5); `suggest.*`; `ask.*` (the daemon sends it out over A2A) with the "let my agents ask" switch |
| **A shared session** (inside its sandbox; process tree as a second check) | the hook endpoint only, as any agent | Hook events for its own agent id (status, the Stop text). Nothing on `pitwalld.sock` |
| **Teammates and A2A clients** | the tailnet HTTP listener | A2A (§8.3) and Pitwall endpoints (§8.4) |

Shared sessions are cut off from Pitwall by construction: the sandbox
denies `pitwalld.sock` and the data folder (§6.3); their environment has no
`PITWALL_*` variables and no `pitwall` on `PATH`; and the daemon still
rejects any connection whose peer process descends from a shared session's
holder (`Caller::Shared` → `forbidden`). The hook endpoint already accepts
only hook events, keyed to the agent id Pitwall gave the session.

**Owner-only local methods** (UI, or CLI/agents through approvals; never on
the tailnet listener, never from shared sessions): `share.get`,
`share.list`, `share.create` (draft), `share.set` (returns the diff it
applied), `share.go_live`, `share.pause`, `share.resume`, `share.stop`,
`share.guest`, `share.sessions`, `share.block`, `share.summary`,
`result.send`, `result.discard`, `team.*` changes (create, approve, invite,
promote, remove, bind), `collab.settings.*`. Typing into or prompting a
shared session uses the ordinary agent methods (`term.*`, `agent.prompt`),
owner only.

### 8.6 CLI (asking is agent-neutral)
`pitwall ask` is an ordinary shell command, so **any agent kind** that can
run a command can ask, and `pitwall mcp` (§10.2) serves agents that prefer
MCP tools. Both are local front-ends: the daemon holds the keys and speaks
A2A outward.

```bash
# Asker side
pitwall people list                                   # people and their shared agents (directory cache)
pitwall team list | show <team> | join <team|link> | leave <team>
pitwall ask alice/api-expert "where are refresh tokens rotated?"   # waits; prints {reply, context, task, result}
pitwall ask alice/api-expert "add a dry-run flag to the jobs CLI"  # same command, same conversation
pitwall ask alice/api-expert - < message.md --new --timeout 900
pitwall ask list [--owner alice]                      # my messages, replies, results
pitwall result apply <task> [--worktree new|<agent>]  # asks the user; git am -3
pitwall result redo <task>                            # asks the user: sends my commits since base

# Owner side (CLI or the owner's own agents; changes go through approvals)
pitwall share list | show <agent>
pitwall share set <agent> --add-team payments --capability write_approval   # prints the diff; widening asks
pitwall share set <agent> --remove-person bob@example.com                   # narrowing: direct
pitwall share go-live <agent> | pause <agent> | stop <agent>
pitwall share sessions <agent>                        # per asker: state, usage
pitwall suggest share <agent> --reason "…"            # a card for the owner, nothing more
pitwall suggest skill <skill> --agent <agent> --reason "…"
```

- Output, errors and exit codes as today (JSON, exit 3 for a denied
  approval). New error codes: `offline`, `busy`, `quota`, `not_allowed`
  (my agents asking without the switch), `no_answer`, `loop`, `forbidden`.
- **Agents asking:** refused with `not_allowed` unless Settings →
  Collaboration → "Let my agents ask shared agents" is on (off by default;
  per project override). With it on: 20 messages per hour per asking agent,
  `result apply` and `result redo` still ask the user, and the request is
  labelled with the agent's id from the process tree (`Pitwall-Agent`),
  never from an argument.
- **Loops.** `Pitwall-Chain` carries the task and share ids a message came
  from. Depth limit 2; a chain containing the target share is refused with
  `loop`. Shared sessions can't ask anyone (no CLI, no network beyond their
  model), so chains only grow through owners' ordinary agents. A modified
  client can lie; quotas and caps are the real limit.

### 8.7 Data shapes
A2A objects use the spec's names (field spelling checked with golden
fixtures against the spec's schema at implementation). A share's card:

```json
{
  "name": "API expert",
  "description": "Questions about the API service: auth, billing, jobs",
  "version": "3",
  "supportedInterfaces": [
    {"url": "http://100.101.102.103:47421/a2a/api-expert",
     "protocolBinding": "HTTP+JSON", "protocolVersion": "1.0"}
  ],
  "capabilities": {
    "streaming": true, "pushNotifications": false,
    "extensions": [
      {"uri": "https://<pitwall-domain>/a2a/ext/device/v1", "required": false,
       "description": "Pitwall device key and asking-agent label"},
      {"uri": "https://<pitwall-domain>/a2a/ext/share/v1", "required": false,
       "description": "Pitwall share facts",
       "params": {"owner": "alice@example.com", "slug": "api-expert",
                  "capability": "write_approval", "kind": "claude",
                  "messagesPerDay": 20, "left": 17}}
    ]
  },
  "defaultInputModes": ["text/plain"],
  "defaultOutputModes": ["text/plain", "text/x-patch"],
  "skills": [{"id": "ask", "name": "Ask", "tags": ["code", "api"],
              "description": "Send any message; replies, and patches when it changed code"}]
}
```

Pitwall-side views (TS, generated from `pitwall-proto` as today):

```ts
interface TeamCard { team: string; name: string; description: string;
  admins: string[]; members: number; version: number; listed: boolean;
  join: "request"|"closed"; fingerprint: string; bound: boolean;
  mine: "member"|"admin"|"requested"|null }
interface DirectoryEntry { cardUrl: string; card: object /* A2A AgentCard */;
  owner: string; slug: string; capability: "read"|"write"|"write_approval";
  online: boolean; lastSeen: string|null; via: string[] /* team ids, "direct" or "tailnet" */ }
interface AskView { taskId: string; contextId: string; share: string; text: string;
  from: "me"|"owner";
  state: "submitted"|"working"|"completed"|"failed"|"canceled"|"rejected";
  statusMessage: string|null; reply: string|null; replySource: string|null;
  result: { state: "held"|"sent"|"discarded"; files: number; added: number; removed: number;
            baseCommit: string; patches: number; path: string|null } | null;
  at: string; doneAt: string|null }
interface SharedSessionView { contextId: string; share: string;
  asker: { login: string; agent: string|null; device: string|null; guest: boolean; pitwall: boolean };
  state: "new"|"starting"|"ready"|"working"|"blocked"|"stopped"|"closed";
  agentId: string; tasks: number; results: number; held: number;
  usage: { minutesToday: number; messagesToday: number; tokens: number|null }; lastAt: string|null }
type ShareDiff = { field: string; from: unknown; to: unknown; widens: boolean }[]
```

### 8.8 Storage

```
<PITWALL_HOME>/collab/
  (settings in ui.json, collab.*, like every setting)
  discovered.json            Pitwall nodes and listed teams seen
  directory.json             cards, cached
  teams/<team>/chain.jsonl   signed records
  teams/<team>/invites.json  admin only (hashes replicate)
  devices.json               pinned device keys of directly named people
  shares/<share>.json        share settings (owner)
  shares/<share>/base/       instructions, approved summary, PRECEDENTS.md
  shares/<share>/s/<ctx>/    work/ (snapshot repo), home/, tmp/, sandbox.sb, tasks.jsonl
  asks.jsonl                 my messages and replies (asker)
  results/<task>/            received patch series (asker)
  audit.jsonl                owner side: every task, result decision, share change (with diff), join, refusal
  suggestions.json           open and dismissed suggestions
```

The audit log uses tailnet-peers.md §5's format with `kind:"collab"`,
rotates at 1 MiB × 3, and is shown in the owner view and Settings.

---

## 9. Safety

### 9.1 Enforced and soft
Messages are untrusted input, possibly written by an agent that read
something hostile. So everything that matters is **enforced** around the
agent, the same way for every kind; instructions are only the **soft**
layer, and the UI says so.

| Share answer | Enforced by | Layer |
|---|---|---|
| Who | `whois` before any HTTP handling, team state, device key for Pitwall callers | Pitwall |
| Capability (read / write) | Seatbelt: copy not writable at Read only; writes only inside the copy at "Can change code" | OS sandbox, all kinds |
| What it can see | Snapshot copy with only the picked paths, minus exclusions, no history; Seatbelt denies the rest of the disk | Pitwall + OS sandbox |
| Start from the conversation | Only an owner-approved summary reaches sessions | Pitwall |
| Secrets and logins | Environment built from nothing; session `HOME`; owner's files unreadable | Pitwall + OS sandbox |
| Network | Only Pitwall's egress proxy is reachable; it allows the kind's model hosts and owner-added hosts | OS sandbox + proxy |
| Results leaving | Mechanical diff of the copy; held for Send at `write_approval` | Pitwall |
| Changing sharing settings | Shared sessions can't reach `pitwalld.sock` (sandbox) and are rejected by process tree; widening from CLI/agents always asks | OS sandbox + Pitwall |
| Limits | Quotas, caps, queue, timeouts | Pitwall |
| Agent's own restrictions (`dontAsk`, `--sandbox`, …) | The agent, where kind data lists them | Optional extra |
| Off-limits topics, role, anything in the owner's instructions | Instructions | **Soft** |

### 9.2 Per platform
| Platform | Shared sessions | Isolation |
|---|---|---|
| macOS | Yes (C1a) | Seatbelt profile around the agent (§6.3) |
| Linux | C4 | bubblewrap, plus Landlock where available |
| Windows | Not until its isolation is designed (C4) | AppContainer or restricted token, to evaluate |
| Any (asking) | Yes | — |

### 9.3 Defaults and switches
Settings → Collaboration, all off:
- **"Use teams"**: discovery, joining, asking. Outgoing only.
- **"Let teammates reach this machine"**: the HTTP listener (§8.1).
- **"Let my agents ask shared agents"**: §8.6.

Sharing needs the second switch. Every share starts as a **draft**: nobody
can see it until the owner reads the summary and presses Go live (§5.4).

### 9.4 Settings are out of reach of shared sessions
Once live, neither a shared session nor any asker can change any sharing
setting, approve anything, or drive Pitwall. Sessions: the sandbox denies
`pitwalld.sock` and Pitwall's data folder, there is no `pitwall` on their
`PATH`, and the daemon rejects their process tree (§8.5). Askers: the
tailnet listener has no owner methods at all. The owner's own pane of the
agent is not a shared session and keeps its normal CLI; its changes to
sharing go through approvals with a diff (§5.5).

### 9.5 What remains true (said in the docs, and in the summary)
- **Replies leak what the agent can read**: anything in the copy, and the
  agent account's config folder (hence the suggested separate account).
- **Shared sessions run as the owner's OS user** inside the sandbox, with
  the chosen agent account. Use is billed to that account, and vendor terms
  on letting others use a subscription apply (§14 question 6).
- **The sandbox is only as good as its profile.** A rule too wide (a
  `subpath` that includes more than meant, an allowed Mach service that
  leaks) widens every session. One profile template, reviewed and tested
  with canaries on each macOS release, rather than per-agent rules.
- **Teammates' Pitwalls are trusted to label their agents**, and
  non-Pitwall clients are trusted only as their Tailscale login.

### 9.6 Threats

| Threat | Mitigation |
|---|---|
| A message tells a Read-only session to edit files or run commands | It can run commands (its own tools), but writes to the copy and everything outside fail: the sandbox, not the agent, says no |
| A message tells a session to exfiltrate secrets | Secrets not in the copy, no history, clean environment, the owner's files unreadable, network only to model hosts via the proxy; at `write_approval` the owner sees every diff before it leaves |
| A session (or a process it double-forked) tries to reach Pitwall or widen its share | Sandbox inherited by every descendant denies the socket and data folder; process-tree rejection as a second check |
| An owner's agent is tricked into widening a share | Widening always asks, shows the diff, can't be remembered |
| The owner's history leaks into sessions | Fresh by default; only an approved summary |
| A removed member keeps asking | Every request checked against the current team state; owner block list at once |
| A removed admin races with forged changes | Revocation wins at the fork point (§4.4) |
| Someone on the tailnet impersonates a member | `whois` identifies the node's login; Pitwall callers also need the device key |
| A replayed or relayed signed request | Time window, nonce cache, both IPs in the signature |
| A member serves forged team records | Admin-signed records chained to the pinned team id |
| Agent↔agent loops burn the owner's quota | Chain depth, cycle check, quotas, caps, asker-side budgets |
| Flood of requests or probes | Rate limits per caller, queue cap, probe limits |
| A result carries a malicious change to the asker | Shown in Review; applied only by the asker's click, into a worktree they pick |
| Tailscale ACLs block the port | Discovery shows "can't reach"; docs give an ACL grant example for `tcp:47421` |

---

## 10. A2A as the channel; MCP

### 10.1 Why A2A carries the conversations
The first draft of this spec kept Pitwall's own framed protocol and offered
A2A only as a gateway. That is reversed: conversations **are** A2A.
- **Open by default.** Any A2A client on the tailnet (another tool, a
  script, an agent framework) can use a shared agent, within its audience,
  with nothing Pitwall-specific.
- **It fits.** Agent Card ↔ the share's card; `contextId` ↔ the per-asker
  session; tasks with states ↔ one message's turn; artifacts ↔ the reply
  and the patch series; SSE streaming ↔ progress; `CancelTask` ↔ stop the
  turn (§8.3).
- **What A2A doesn't cover stays Pitwall's,** beside it on the same
  listener: tailnet identity before HTTP, the device-key extension, teams,
  discovery, gossip of signed records (§8.4).
- **Cost.** An HTTP server and client in Pitwall (blocking, small, §8.1),
  and following A2A versions. The agents never see any of it.
- **Peers stay framed TCP** (tailnet-peers.md): terminals and screen frames
  aren't conversations.

### 10.2 MCP
- **`pitwall mcp`**: a local stdio MCP server on the asker's machine with
  tools `list_shared_agents`, `ask {agent, message, new}`,
  `get_task {id}`. A thin front-end over the same local `ask.*` methods as
  `pitwall ask`, so the "let my agents ask" switch, budgets and labels
  apply. Any MCP-capable agent on the asker's machine, inside Pitwall or
  not, can then use shared agents.
- An MCP server on the tailnet is not planned: A2A is the remote interface.

---

## 11. UI (GPUI app)

Same visual language as tailnet-peers.md §7.

- **People** (new top-level screen, ⌘⇧P, also in ⌘K), three tabs:
  - **Directory.** Shared agents I can use, grouped by owner: name,
    description, capability chip (READ / WRITE / WRITE ✓), "runs on Claude
    Code", what's left today, online dot. Click → Ask panel on that agent.
  - **Teams.** My teams (name, fingerprint `9F2-C1A`, members, my role,
    "bound to Tailscale" chip), then Discovered (listed teams on the
    tailnet, "Request to join"), then "Join with a link…" and "Create
    team…".
    - **Team detail:** members with devices and online state, admins (★),
      version and "last change by dave · 2 d", Requests (admins: Approve /
      Decline), Invite (make link, copy), Leave; for admins: rename,
      listed/invite-only, join by request/closed, promote/demote, remove,
      Bind to Tailscale….
    - Conflict note inline when it happened: "Your change was re-applied
      after dave's".
  - **People.** Everyone in my teams, their devices and shared agents.
    "Block" stops all of a person's messages to my agents.
- **Share dialog** (agent menu → Share…): the seven steps of §5.4, one short
  page each with Back / Next, each field marked **Enforced** or **Not
  guaranteed**:
  1. Who (and "Only Pitwall callers") · 2. Capability · 3. What it can see
     (folder picker, then the scan result as a ticked list with "why":
     "matches *.pem", "contains a private key") · 4. Start from (Fresh ·
     Summary: "Write summary…" → editor → Approve) · 5. Secrets and access
     (account, with "use a separate account for sharing"; environment
     variables to pass, none by default; extra network hosts) · 6.
     Off-limits topics and the role text · 7. Limits.
  - Then **Summary** (§5.4), **Try as guest** (opens the Ask panel on the
    draft with the probe buttons) and **Go live**.
  - Editing later opens the same steps; Save shows the diff and marks
    widening lines amber.
- **Suggestion cards.** In the agent's pane header and the notifications
  list: "Race Engineer suggests sharing payments-expert with team Payments
  — people asked about refunds 9 times this week. [Open Share…] [Dismiss]".
- **Sidebar, owner side.** A "Shared" group (after local projects, before
  peer machines): each shared agent is a heading with capability chip,
  running / cap (`1/2`), draft or paused state; under it one row per
  session, named by asker (`bob`, `bob · client-fix agent`, `dave (A2A)`,
  `guest`), with status flag, last message time and usage. Needs-you and
  held results flag amber like any blocked agent and count in the badge.
- **Session pane, owner side.** The terminal (watch), with a strip "Shared
  with bob@example.com · read only · sandboxed · Take over". A side tab
  **Messages** lists what was typed, replies (with their source) and results
  as a thread. Review works on the session's copy; at `write_approval`,
  "Send" / "Discard" for each held result.
- **Usage.** Shared agent heading → "Usage…": per asker today and 30 days
  (messages, results, minutes running, tokens where the agent reports
  them), Block, Forget session, the audit log filtered to this share.
- **Ask panel** (asker side; right-panel tab "Ask", and ⌘⇧A): agent picker,
  one thread per shared agent (my messages, replies, statuses "2nd in line",
  "waiting for alice", "changes waiting for alice's OK"), "New
  conversation". Replies with changes show a result card (files, +/−, "Open
  in Review", "Apply…", "Redo on my code").
- **Review (asker side).** A result is a Review entry "changes from
  alice/api-expert · task 7", with per-file diffs, Apply (target picker), and
  conflicts shown as for any merge.
- **Notifications.** Owner: needs-you in a shared session, results to
  review, join requests (admins), suggestions (quiet), optionally every
  reply. Asker: replies that arrived while the window was unfocused, results
  ready.
- **Settings → Collaboration.** The three switches, "Look for teams on",
  the global session cap, budgets for my agents, rotate key, the audit log,
  and Tailscale state (shared with Peers).

---

## 12. Phases

After tailnet-peers.md P0 and P1 (`Tailnet` trait, listener rules, instance
key, audit log). Each phase usable on its own, macOS first.

| Phase | Scope | Effort | Done when |
|---|---|---|---|
| **C1a MVP: A2A, discovery, simple teams, Read only, isolation** | HTTP listener (`tiny_http`, `whois` before handling), A2A HTTP+JSON server and client (card, send, stream/SSE, get, list, cancel, subscribe), the device extension. Pitwall endpoints: info, discover, directory, sessions, teams with a **single admin** (records already signed and chained, pull replication), join by request and invite link. Share dialog (seven steps, sensitive-file scan, summary, try as guest, draft → live) at **Read only**. Snapshot copies, **Seatbelt sandbox + egress proxy + clean environment** for any kind, per-asker sessions with the kind's resume, approved-summary base + fork where possible. Reply capture tiers (a) Claude Code and Codex, (b) Gemini CLI if verified, (c) everyone else. `[share]` kind data for Claude Code, Codex, Gemini CLI. `pitwall ask`, `people`, `team`, `share` (approvals and diffs). People screen, Ask panel, owner sidebar group and session pane, usage, block, stop sharing, audit | 7–9 wk | Bob joins Alice's team from discovery and asks `alice/api-expert` (Claude Code) and `alice/docs` (Gemini CLI) from the Ask panel and from an agent; a plain A2A sample client gets a reply by login; Alice tried as guest first; guest probes for `.env`, `~/.ssh` and the network get nothing; a double-forked process can't reach `pitwalld.sock` |
| **C1b MVP: signed multi-admin teams** | Several admins, fork choice (§4.4), re-apply of dropped ops, announce/push, devices, rotation, self-leave, last-admin rule, "re-create team", fingerprints, invite code hashes replicated | 1.5–2 wk | The team survives the creator's machine being off for a week; two admins changing at once converge; a removed admin can't win |
| **C2 Can change code** | `write` and `write_approval`, per-turn commits and mechanical change detection, format-patch artifacts (raw and URL), held results with Send/Discard, asker-side Review, Apply (`git am -3`), "Redo on my code", "let my agents ask" budgets and loop checks | 2–2.5 wk | Carol asks, asks for a change, asks why; she applies the patch in a worktree; at `write_approval` nothing left before Alice's Send |
| **C3 Owner in the loop, suggestions, then precedent** | Needs-you notifications for shared sessions, owner turns shown in the asker's thread, "Notify me on every reply"; `pitwall suggest share|skill` and cards; then "Save as precedent" | 1–1.5 wk | Alice answers a blocked session from a notification and Bob sees it; the Race Engineer's suggestion opens a prefilled Share dialog |
| **C4 Tailscale binding, Linux and Windows** | App-capability membership and ceilings, two-signature admin removal option, Linux isolation (bubblewrap/Landlock), Windows isolation design and build, fuzzing of the HTTP endpoints and records | 2.5–3.5 wk | A team bound to `group:platform` follows Tailscale changes; Linux hosts shares; Windows hosts shares or has a documented decision |
| **C5 MCP and more kinds** | `pitwall mcp`, `[share]` data for more kinds as they're verified, optional HTTPS with `tailscale cert`, optional push/PR result mode | 1.5–2 wk | A non-Pitwall MCP agent gets replies from `alice/api-expert`; opencode and Aider shares tested |

Total: about 16–20 weeks.

### Tests
- **Fake tailnet, three instances** (alice, bob, carol), as tailnet-peers.md
  §9, with `whois` per source port (`CapMap` in the fake file for C4); the
  HTTP listener accepts only what the fake `whois` knows.
- **A2A conformance:** golden fixtures for cards, messages, tasks, status
  and artifact events against the spec's schema; the official A2A sample
  client (or SDK) as a black-box client in an opt-in test.
- **Device extension:** signature, time window, nonce replay, wrong IPs,
  key not belonging to the login, pinning for direct people.
- **Team records:** generated op sequences from several admins with random
  delivery and partitions; all members converge; "a removed admin never
  wins"; golden JSON for records and their canonical form.
- **Scope:** the snapshot contains exactly the picked paths minus
  exclusions; no `.git` link back; blob ids equal the owner's; the
  sensitive-file scan against a fixture tree.
- **Isolation (macOS, per kind and for a fake kind):** reads of `~/`,
  `~/.ssh`, the owner's project and `~/.pitwall` fail; writes outside the
  copy fail, and inside it fail at Read only; network except the proxy
  fails, and the proxy refuses hosts not allowed; the environment has no
  owner variables; a double-forked child is still sandboxed and can't
  connect to `pitwalld.sock`.
- **Sessions:** same asker resumes (kind's resume), different askers
  isolated, fork from the base where supported, summary as first message
  elsewhere, idle stop and the no-resume path, caps, queue order, quota,
  timeouts; reply capture tiers a, b, c with recorded outputs per kind.
- **Results:** format-patch → `git am -3` on a repo that moved on (clean,
  three-way, conflict), redo on my code, held → Send/Discard.
- **Opt-in real runs:** real Claude Code, Codex and Gemini CLI shares
  against a canary repo with canary secrets inside the scope (excluded),
  outside the copy, in the environment and in the agent account's folder,
  plus a prompt-injection message set (the guest probes and more). No
  canary may appear in replies or results, except the documented account
  folder case. Rerun on agent and macOS updates.

### Risks

| Risk | Mitigation |
|---|---|
| The Seatbelt profile breaks an agent (a path, a Mach service, DNS, proxy handling) | Deny-by-default fails closed (breakage, not a leak); per-kind install paths and hosts in kind data; canary and smoke runs per kind and per macOS release |
| `sandbox-exec` is deprecated and its profile language undocumented | The kernel mechanism is what Codex and Claude Code use too; keep the profile small; separate user or container as fallbacks (§14 question 7) |
| A2A spec churn | Pin `A2A-Version: 1.0`, golden fixtures, caps in the card |
| An HTTP server in Pitwall widens the attack surface | `whois` before any parsing, tailnet-only bind, size limits, fuzzing (C4) |
| Reply capture from terminal output (tier c) is rough for some kinds | Marked as such to the asker; add kind data for hooks or a non-interactive mode as they're verified |
| Membership conflicts and revocation lag without a server | Fork choice at the fork point, revocation wins, owner block list at once |
| The owner's agent account cost and vendor terms | Quotas, usage view, separate account suggested, §14 question 6 |
| Scope creep toward a hosted product | Non-goals, §13 |

The two risks the first draft rated highest shrink with OS isolation:
**"enforcement depends on each agent's flags"** becomes "one Seatbelt
profile Pitwall owns, the same for every kind", with the agents' flags as an
optional extra; and the **socket-escape gap** (a double-forked process
leaving the process tree) closes on macOS because the sandbox follows every
descendant and denies the socket, with the process-tree check kept as a
second layer.

---

## 13. Out of scope: a server or B2B version
A hosted directory, accounts, SSO, org-wide policy, billing, audit export,
relay for people outside a tailnet, or shared agents running on shared
infrastructure. Parked. The local design keeps the door open: the channel
is A2A, team records are self-contained and signed, and identity is behind
the `Tailnet` trait.

---

## 14. Open questions
1. **Admin removal.** Require two admin signatures to remove an admin when a
   team has three or more admins (proposed: yes; off for smaller teams)?
2. **Invite links.** One-time only (proposed), or also reusable until
   revoked?
3. **Domain.** The app capability (`<domain>/cap/pitwall`) and the A2A
   extension URIs need a domain the project controls. Which one?
4. **Full conversation as the base.** Offer forking the owner's whole
   conversation (with an amber warning) besides fresh and summary, or never?
5. **Offline owners.** Refuse messages while the owner is offline
   (proposed), or queue them on the asker's side and send on reconnect?
6. **Agent subscriptions.** Shared sessions use one of the owner's agent
   accounts. Do the vendors' terms allow teammates to use it this way, or
   should Pitwall recommend (or require) an API-key or team-plan account for
   shares? Needs a check per vendor.
7. **Isolation beyond Seatbelt.** After the macOS Seatbelt profile (C1a),
   which comes next for owners who want more, and for Windows: a separate
   OS user per share, a container, or an agw VM?
8. **Shared nodes across tailnets.** Allow askers whose nodes are shared
   into the tailnet (Tailscale node sharing), if `whois` identifies them
   reliably?
9. **HTTPS.** Is plain HTTP inside WireGuard enough for all A2A clients, or
   should Pitwall serve HTTPS with `tailscale cert` by default?
10. **Reply source.** For kinds that have both, prefer the interactive
    terminal with hooks (owner can take over naturally) or the
    non-interactive mode (cleaner replies)?
11. **Session keying for agents.** One session per asking agent (proposed),
    or one per person, shared by all of that person's agents?
12. **Git history in scope.** Offer a filtered clone with history (so "why
    was this changed?" works), and how to keep excluded files out of it?
13. **Saved prompts.** Declared entry points with typed input ("actions")
    were dropped from v1. Add them later as a shortcut, e.g. A2A skills that
    expand to an owner-written prompt?
14. **Owner notes.** Should the owner be able to type into a shared session
    without the asker seeing it?
15. **Non-Pitwall callers.** Accept A2A clients by login by default
    (proposed), or require "Only Pitwall callers" unless the owner opts in?

---

## References

Checked on 2026-10-16. "Unverified" items are marked in the text.

**Tailscale**
- WhoIs response (`Node`, `UserProfile`, `CapMap`; Node and UserProfile
  never nil on success): https://pkg.go.dev/tailscale.com/client/tailscale/apitype#WhoIsResponse
- Grants, `app` field, capability names `domain/name`, parameters as opaque
  JSON: https://tailscale.com/kb/1324/grants
- Application capabilities, golink example (`tailscale.com/cap/golink`,
  `{"admin": true}`), apps read them via tsnet, LocalAPI, `tailscale whois`
  or `tailscale serve` headers: https://tailscale.com/kb/1537/grants-app-capabilities
- Golink reading `whois.CapMap`: https://tailscale.com/blog/grants-ga
- tsidp uses LocalAPI WhoIs (user, node, app capabilities) and the
  `tailscale.com/cap/tsidp` capability: https://tailscale.com/docs/features/tsidp,
  https://tailscale.com/blog/building-tsidp
- No broadcast or multicast over Tailscale (staff answer, older; no newer
  docs found saying it changed): https://forum.tailscale.com/t/broadcast-multicast-support/156
- `tailscale status --json` fields used for discovery: as listed and caveated
  in tailnet-peers.md §3.

**A2A** (specification 1.0.0): https://a2a-protocol.org/latest/specification/
- Agent Card required fields (`name`, `description`, `supportedInterfaces`,
  `version`, `capabilities`, `defaultInputModes`, `defaultOutputModes`,
  `skills`), `AgentSkill`, `AgentInterface` (`url`, `protocolBinding`:
  `JSONRPC`, `GRPC`, `HTTP+JSON`; `protocolVersion`), `AgentExtension`
  (`uri`, `description`, `required`, `params`) in
  `capabilities.extensions`, card signatures; discovery by
  `/.well-known/agent-card.json`, catalogs or pre-configured card URLs.
- `contextId` (server may generate; groups tasks into a conversation); a
  server may return a Message or a Task.
- Task states, including input-required (client continues) and
  auth-required (in-task authorization delegated to the client; the state
  by itself is not authorization).
- `Part` (exactly one of `text`, `raw`, `url`, `data`; `mediaType`,
  `filename`), `Artifact` (`artifactId`, `name`, `parts`), artifact update
  events (`append`, `lastChunk`).
- HTTP+JSON endpoints (`POST /message:send`, `POST /message:stream` with
  `text/event-stream`, `GET /tasks/{id}`, `GET /tasks`,
  `POST /tasks/{id}:cancel`, `POST /tasks/{id}:subscribe`), body media type
  `application/a2a+json`, `A2A-Version` and `A2A-Extensions` headers; push
  notifications optional.

**Claude Code**
- `-p`, `--resume`, `--fork-session`, `--session-id`, `--output-format`,
  `--permission-mode` values, `--tools`, `--allowedTools`,
  `--disallowedTools`, `--settings`, `--append-system-prompt`:
  https://code.claude.com/docs/en/cli-reference
- Permission modes (`plan`: reads plus classifier-approved commands, edits
  blocked until a plan is approved; `dontAsk`: reads inside working
  directories and pre-approved tools, everything else denied):
  https://code.claude.com/docs/en/permission-modes
- Hooks: `Stop` `last_assistant_message`; `SessionStart` `source` values
  including `"fork"`: https://code.claude.com/docs/en/hooks
- Bash sandbox (shell commands only; reads default
  to most of the machine; network through a proxy with an empty allowlist;
  environment inherited unless scrubbed; not on native Windows):
  https://code.claude.com/docs/en/sandboxing

**Codex CLI**
- `codex resume`, `codex exec resume`, `codex fork`, `codex exec fork`,
  `--sandbox read-only|workspace-write|danger-full-access`,
  `--ask-for-approval`, `codex exec --json`:
  https://learn.chatgpt.com/docs/developer-commands?surface=cli (redirect
  target of https://developers.openai.com/codex/cli/reference)
- Config: `sandbox_mode`, `sandbox_workspace_write.network_access`,
  `shell_environment_policy`, `approval_policy`, beta permission profiles:
  https://learn.chatgpt.com/docs/config-file/config-reference
- Sandboxing (Seatbelt on macOS, bubblewrap on Linux and WSL2, Windows
  sandbox): https://learn.chatgpt.com/docs/sandboxing

**Other agents**
- Gemini CLI headless mode (`-p`/`--prompt`, `--output-format json` /
  stream JSON): https://geminicli.com/docs/cli/headless.md. `--resume` with
  `-p`: only third-party sources, **unverified**.
- Aider scripting (`--message`: one message, apply edits, exit; `--yes`):
  https://aider.chat/docs/scripting.html
- opencode `run` with `-s/--session`, `-c`, `--fork`, `--format json`:
  third-party references only (e.g. https://learnopencode.com/en/appendix/cli),
  **unverified** against opencode's own docs.

**macOS sandbox**
- `sandbox-exec` deprecated in its man page but working on recent macOS;
  deny-by-default profiles with file, network and exec rules:
  https://jmmv.dev/2019/11/macos-sandbox-exec.html,
  https://gastown.dev/docs/research/macos-sandbox-exec (secondary sources).

**MCP**
- Model Context Protocol specification (tools, stdio transport), not
  rechecked in detail: https://modelcontextprotocol.io/specification

**Pitwall**
- Skills library and per-project / per-agent assignment: [rules.md](rules.md)
  (on the `rules-scopes` work).

**Unverified** (to check before the phase that needs it)
- Seatbelt details per agent: Mach services (Keychain, DNS), proxy
  behaviour, nesting with Claude Code's and Codex's own Seatbelt sandboxes,
  behaviour on macOS 26 (§6.3).
- Gemini CLI `--resume` with `-p`; reply capture and resume/fork for
  opencode, Cursor Agent, Copilot CLI, Qwen Code, Amp (§6.7).
- Codex session-file fields for the last assistant message (§7.2).
- `tiny_http` streaming and `ureq` SSE reading in practice (§8.1).
- Whether A2A requires TLS for production deployments (§8.1, §14
  question 9).
- The exact grant shape (`dst`) for a Pitwall node to see a caller's app
  capability via LocalAPI `whois` (§4.7), and how tagged devices appear in
  `UserProfile`.
- Tailscale multicast: only an older staff forum answer (§3).
- Vendor terms for letting others use an agent subscription (§14 question 6).
