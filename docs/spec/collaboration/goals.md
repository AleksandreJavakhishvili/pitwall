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
   per person per day, 2 conversations at once. She reads the plain summary,
   tries it **as a guest** ("show me the .env file" gets nothing), and
   presses **Go live**. Her skills stay on her machine; teammates only get
   answers.
4. **Plain Claude Code.** Erin doesn't use Pitwall. Alice adds
   erin@example.com to team Platform, and Pitwall shows a line to send her:
   `claude mcp add --transport http alice http://studio.tail1234.ts.net:47421/mcp`.
   Erin runs it. Her Claude now has a tool `ask_api_expert`, and while
   fixing a client bug it asks "what does 409 from /jobs mean?" on its own.
   The network (here Tailscale) tells Alice's Pitwall that the call is from
   Erin; nothing else is set up.
5. **Ask from Pitwall.** Bob opens the Ask panel, picks `alice/api-expert`
   and asks "Where are refresh tokens rotated?". His Pitwall sends it over
   A2A; the reply streams into his thread. Tomorrow he asks a follow-up and the same conversation continues.
6. **Ask, change, ask.** `alice/payments-expert` (a Codex agent) is shared
   with team Payments with **Can change code**. Carol asks how refunds are
   booked, then writes "add a dry-run flag to the refund job". That turn
   changed files, so a patch arrives in her Review with the reply. Then she
   asks "why did you change the ledger call?". One conversation, full
   memory, and every message is just a message. She clicks **Apply**; the
   patch lands in a worktree of hers with `git am -3`. Nothing changed in
   Alice's checkout, and nothing went through GitHub. (Erin, in plain
   Claude Code, gets the same patch in the tool result and her Claude
   applies it with `git am`.)
7. **Any agent asks.** Bob's Gemini CLI agent runs
   `pitwall ask alice/api-expert "what does 409 from /jobs mean?"` and gets
   the reply as JSON. Bob allowed his agents to ask, with a budget.
8. **Limits.** Dave has used his 20 messages today. His next call returns
   "Daily limit reached for alice/api-expert; resets at 00:00", and his
   agent carries on without it. Later Alice's Claude subscription hits its
   own usage limit; every caller gets "unavailable until 14:30" instead of
   hanging, and Alice's own work isn't touched.
9. **Needs you.** `alice/product` (Read only) stops and waits on a question
   that needs a decision. Pitwall's usual detection flags it; Alice gets
   "product: bob@example.com's conversation needs you", types two lines into
   the session, and the agent's reply goes back to Bob.
10. **Owner view.** Alice sees `api-expert` in her sidebar with one row per
    caller, how much each one used, and can watch any conversation, take it
    over, change someone's limit, block a person, or stop sharing.
11. **Share and change by asking.** Alice tells her Race Engineer "share
    docs-helper with team Platform, read only, just the docs folder". The
    Engineer runs `pitwall share create …`, reads her the summary and the
    two files Pitwall excluded, runs the guest probes (`pitwall share guest
    --probes`: nothing leaked), then `pitwall share go-live`; Pitwall asks
    Alice with the full summary and she approves. Later: "also share
    payments-expert with Dave" → `pitwall share set …`, an approval with the
    exact diff (`+ people: dave@example.com`). Going live and widening
    always ask.
12. **A suggestion.** The Race Engineer notices people keep asking Alice's
    agents about refunds and suggests sharing `payments-expert` with team
    Payments. A card opens the Share dialog, prefilled. Nothing is shared
    until Alice goes through it.
13. **Org admin.** Carol's company has a Tailscale admin. The admin binds the
    "Platform" team to a Tailscale group with an app capability, so joining
    and leaving follows the company's identity provider, and Pitwall needs no
    signed membership changes for that team.
14. **Later: Pitwall Cloud.** Frank is a contractor outside the company's
    tailnet. Alice also turns on Pitwall Cloud and adds Frank's Cloud
    account to `api-expert`'s audience. Frank runs `claude mcp add
    --transport http alice https://studio.acme.pitwall.cloud/mcp`, signs in
    through his browser, and asks. Bob, on both networks, has linked his
    two identities, so his conversation and limits are the same whichever
    way he comes in. Nothing in shares, sessions or limits changed for
    this.

### Goals
- **No server.** Every request goes from a caller straight to the owner's
  machine over WireGuard. Membership and the directory live on members'
  machines.
- **Works with plain Claude Code.** Anyone on the network with an MCP
  client can use what is shared with them, with one setup command and
  nothing else installed. Pitwall users get more (teams, directory, Ask
  panel, Review) over A2A, which fits agent-to-agent work best. One core
  behind both, so the rules are the same.
- **An open channel.** Any A2A client on the network can use a share too,
  within its audience.
- **Network-neutral, identity from the network.** Every request is tied to
  an identity its network proves (Tailscale `whois`, a Cloud-signed token)
  before anything else, through one `Network` interface ([§15](network.md)). The core
  never knows which network is in use, and several can be on at once.
- **Teams survive any one machine.** Membership records are signed,
  versioned and copied to every member's Pitwall.
- **Expertise stays with its owner.** A shared agent's skills, rules,
  conversation and code never leave the owner's machine; callers get
  replies and patches.
- **Any agent can be shared or ask; nothing is shared automatically.**
  Sharing is a setting on an ordinary agent of any kind. Agents may suggest
  it; the owner always decides.
- **Native behaviour.** No Pitwall-only rules or protocols for agents: the
  shared agent gets messages as input, keeps its own tools, and is read the
  way Pitwall already reads it (hooks, its own session files, its own
  non-interactive mode, terminal output). The caller's agent just sees a
  tool.
- **Enforced, not instructed.** What a shared session can see and write,
  which secrets it has, what network it reaches, its limits, and who can
  change its settings are enforced around the agent (OS sandbox, the copy,
  the environment, Pitwall). Instructions are a clearly labelled soft layer.
- **Off by default, opt-in at every step.** A new share is a draft until the
  owner goes live.
- **A tool, not an agent.** Pitwall carries messages and replies verbatim,
  never sorts messages into kinds, never decides a reply. Whether a turn
  changed code is detected mechanically, from the copy.
- **The caller's code is never touched by someone else.** Changes come back
  as patches over the same channel, and the caller applies them.

### Non-goals (v1)
- **No task lists, jobs or boards.** Asking an agent is a conversation;
  tracking work stays in the tools teams already use (GitHub Issues,
  Linear, chat). Revisit only if real use shows a gap.
- **No sharing of skills, rules or "recipes".** Expertise is reached by
  talking to the owner's agent, never copied to callers.
- No Pitwall server, directory service, relay or account system in v1
  ([§13](plan.md)); the Cloud backend comes later ([§15.6](network.md)).
- No special "shared agent" or "specialist" type or template. A specialist
  is just an ordinary agent with a role and skills, shared ([§5.1](sharing.md)).
- No sharing of a whole Pitwall. Teams never grant tailnet-peers
  permissions (View, Control, …).
- No shared terminals for callers. A caller sees replies and changes, never
  the shared agent's terminal. Only the owner watches and takes over.
- No pushing to remotes or opening pull requests in v1 (a later optional
  result mode, [§7.5](messages.md)).
- No shared sessions on agw machines in v1, on Linux before C3, or on
  Windows before the WSL2 route (C5) ([§9.2](safety.md)). Asking works everywhere.
- No Pitwall Cloud backend in v1: only Tailscale (and Fake for tests). The
  Cloud backend is designed to the same contract and added in C8
  ([§15.6](network.md)).
- No group chat between people. Each conversation is one caller and one
  shared agent.

### What changes in tailnet-peers.md
Collaboration reuses its pieces but relaxes some of its v1 non-goals, on
purpose and only for the narrow collaboration surface:

| tailnet-peers v1 non-goal or rule | With collaboration |
|---|---|
| "No real multi-user collaboration" | Several people use one shared agent, each in their own conversation. Peers (whole-machine access) stay single-owner |
| "Discovery lists same-user nodes only" | Team discovery probes other users' nodes too ([§3](teams.md)), within limits and only while the feature is on |
| "No peer access from agents or the CLI on the viewer" | `pitwall ask` (and `pitwall mcp`) reach other Pitwalls, only shared agents, only with the local user's opt-in and budgets ([§8.6](protocol.md)). All other peer routes stay off the CLI socket |
| Pairing is the only way to be accepted | A share's audience (a person, a team, a group, everyone on a network) is a second way, for the collaboration endpoints only. It never unlocks `peer` methods |
| One framed-TCP listener (47420) | Collaboration has its own HTTP listener (47421 proposed) with the same bind and accept rules ([§8.1](protocol.md)). Peers stay on framed TCP |
| Two listener switches | A third: "Let teammates reach this machine" ([§9.3](safety.md)) |
| "No relaying: one hop only" | Still one hop. A message always goes straight to the owner's machine. Membership records may come from any member, but they are signed ([§4](teams.md)) |

---

## 2. Model

```
 bob-mbp (Pitwall)                                     studio (owner: alice, Pitwall)
┌────────────────────────────────┐                   ┌───────────────────────────────────────┐
│ Ask panel · pitwall ask · mcp  │   A2A over HTTP   │ HTTP handler on each network          │
│   └─ daemon: A2A client        │──  (tasks, SSE) ─▶│  /a2a/<share>   A2A front ┐           │
│ Review: changes, Apply (am -3) │◀─ reply + patch ──│  /mcp           MCP front ┴─▶ one core│
│                                │                   │  /pitwall/v1/…  teams, discovery      │
│ Collab: teams, directory cache │  Network says who │ Collab: shares, conversations, limits │
└────────────────────────────────┘                   │  conversation = ordinary agent, any   │
          ▲   gossip of team records                 │  kind, in an OS sandbox around a      │
          └──── any member ◀───────────────────────▶ │  snapshot copy, clean env             │
                                                     └───────────────────────────────────────┘
 erin-mbp (plain Claude Code)                                     ▲
┌────────────────────────────────┐   MCP over HTTP                │
│ claude mcp add … alice …/mcp   │────────────────────────────────┘
│ tool ask_api_expert            │   Network says: erin@ (a member of Platform)
└────────────────────────────────┘
```

The words used across these files (person, device, team, share, caller,
conversation, message, reply, changes, limits) are defined in the
[README](README.md#glossary).

Collaboration is a new service beside the engine, like `Peers` in
tailnet-peers.md: `Collab` owns teams, the directory, shares,
conversations, limits, the A2A server and client, and the MCP server. Conversations are
ordinary engine agents with an extra `AgentRecord.shared = { share, caller }`.
`Collab` sees networks only through `Network` ([§15](network.md)).
Status, holders, hooks, Review and the Wall work for them as for any agent;
caps decide what the UI shows (the CI no-special-cases grep stays green).
