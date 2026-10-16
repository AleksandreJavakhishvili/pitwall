# Collaboration: goals, stories and model

Part of the [collaboration spec](README.md).

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
9. **Share and change by asking.** Alice tells her Race Engineer "share
   docs-helper with team Platform, read only, just the docs folder". The
   Engineer runs `pitwall share create …`, reads her the summary and the
   two files Pitwall excluded, runs the guest probes (`pitwall share guest
   --probes`: nothing leaked), then `pitwall share go-live`; Pitwall asks
   Alice with the full summary and she approves. Later: "also share
   payments-expert with Dave" → `pitwall share set …`, an approval with the
   exact diff (`+ people: dave@example.com`). Going live and widening
   always ask.
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
- No Pitwall server, directory service, relay or account system ([§13](plan.md)).
- No special "shared agent" or "specialist" type or template. A specialist
  is just an ordinary agent with a role and skills, shared ([§5.1](sharing.md)).
- No sharing of a whole Pitwall. Teams never grant tailnet-peers
  permissions (View, Control, …).
- No shared terminals for askers. An asker sees replies and results, never
  the shared agent's terminal. Only the owner watches and takes over.
- No pushing to remotes or opening pull requests in v1 (a later optional
  result mode, [§7.5](messages.md)).
- No shared sessions on agw machines in v1, on Linux before C3, or on
  Windows before the WSL2 route (C5) ([§9.2](safety.md)). Asking works everywhere.
- No reaching people outside the tailnet.
- No group chat between people. Each conversation is one asker and one
  shared agent.

### What changes in tailnet-peers.md
Collaboration reuses its pieces but relaxes some of its v1 non-goals, on
purpose and only for the narrow collaboration surface:

| tailnet-peers v1 non-goal or rule | With collaboration |
|---|---|
| "No real multi-user collaboration" | Several people use one shared agent, each in their own session. Peers (whole-machine access) stay single-owner |
| "Discovery lists same-user nodes only" | Team discovery probes other users' nodes too ([§3](teams.md)), within limits and only while the feature is on |
| "No peer access from agents or the CLI on the viewer" | `pitwall ask` (and `pitwall mcp`) reach other Pitwalls, only shared agents, only with the local user's opt-in and budgets ([§8.6](protocol.md)). All other peer routes stay off the CLI socket |
| Pairing is the only way to be accepted | Team membership or a share's audience is a second way, for the collaboration endpoints only. It never unlocks `peer` methods |
| One framed-TCP listener (47420) | Collaboration has its own HTTP listener (47421 proposed) with the same bind and accept rules ([§8.1](protocol.md)). Peers stay on framed TCP |
| Two listener switches | A third: "Let teammates reach this machine" ([§9.3](safety.md)) |
| "No relaying: one hop only" | Still one hop. A message always goes straight to the owner's machine. Membership records may come from any member, but they are signed ([§4](teams.md)) |

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

The words used across these files (person, device, team, share, session,
message, reply, result) are defined in the [README](README.md#glossary).

Collaboration is a new service beside the engine, like `Peers` in
tailnet-peers.md: `Collab` owns teams, the directory, shares, sessions and
the A2A server and client. Sessions are ordinary engine agents with an extra
`AgentRecord.shared = { share, asker }`. Status, holders, hooks, Review and
the Wall work for them as for any agent; caps decide what the UI shows (the
CI no-special-cases grep stays green).
