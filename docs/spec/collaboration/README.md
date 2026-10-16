# Pitwall collaboration: teams and shared agents

People on the same Tailscale tailnet can **share** (expose) some of their
agents with others. Someone sends a shared agent messages: a question, then
"add a dry-run flag", then "why did you change that?", all in one
conversation. If the share lets it change code, any turn that changed its
copy comes back as a patch. The owner decides what is shared and with whom,
sees every conversation, can watch or take over, and can stop sharing at
any time.

It works for two kinds of people:
- **Pitwall users.** They join **teams**, see every teammate's shared
  agents without adding endpoints, talk to them from an Ask panel or let
  their own agents ask, and get patches in Review. Pitwall talks to Pitwall
  over **A2A**.
- **Plain Claude Code users** (or any MCP client). They add the owner's
  endpoint once with `claude mcp add --transport http …`, and the shared
  agents show up as tools over **MCP**. Nothing else to install.

Both fronts lead to the same conversations, limits and checks. Only Pitwall
users can share their own agents.

**Status: design, not built.** It builds on
[tailnet-peers.md](../tailnet-peers.md) (instance key, `whois` identity,
listener rules, audit log) and comes after that spec's P0 and P1. Skills
assigned to a shared agent come from [rules.md](../rules.md); they stay on
the owner's machine.

## Three choices that shape it
- **No Pitwall server.** Everything runs on the users' own machines, over
  their own tailnet. A hosted or B2B version is parked
  ([plan.md §13](plan.md)).
- **One core, two fronts; Tailscale is the identity.** Each owner's
  Pitwall serves an **A2A** front (Pitwall callers and any A2A client: long
  tasks, waiting for the owner, re-attaching are native) and an **MCP**
  front (plain Claude Code: each shared agent is a tool) on its tailnet
  address, over one core. Who is calling comes from Tailscale (`whois` on
  the connection): no accounts, keys or pairing for callers. Teams use a
  few small Pitwall endpoints beside them ([protocol.md](protocol.md)). The network and
  identity sit behind one seam, so a Pitwall Cloud could later stand in
  for Tailscale ([§13](plan.md)).
- **Any agent kind, on either side, with native behaviour.** Any agent
  Pitwall runs can be shared, and any agent that can call an MCP tool or
  run a command can ask. The shared agent never speaks A2A or MCP itself and
  follows no Pitwall-only rules: Pitwall sends the message as ordinary
  input, reads the reply the way it already reads agents, and enforces
  limits through the session's surroundings: an OS sandbox (Seatbelt on
  macOS, bubblewrap on Linux, WSL2 on Windows), a copy holding only what
  was shared, and a clean environment ([sessions.md](sessions.md)).

## Glossary
- **Person.** Stored as `{provider, id}`; in v1 always a Tailscale login
  (`UserProfile.LoginName` from `whois`). A Pitwall Cloud account could be a
  second provider later ([§13](plan.md)).
- **Device.** One Pitwall installation: its instance key plus the Tailscale
  node (`Node.StableID`) it runs on. A person has zero or more devices;
  plain Claude users have none.
- **Team.** A named list of people, with admins, kept as signed records on
  members' Pitwalls and identified by the hash of its first record, never
  by its name. Used as an audience for shares and, in Pitwall, as a
  directory. Members may be plain Claude users.
- **Share** ("shared agent", "exposed agent"). A setting on one of the
  owner's ordinary agents: audience, capability (`read`, `write`,
  `write_approval`), scope, start, access, off-limits topics, limits. The
  UI says **Share…**.
- **Caller.** Whoever sends a message: a person, or one of their agents.
- **Conversation** (session). One caller's conversation with a shared
  agent (an A2A `contextId`; `conversation` on MCP): a real Pitwall agent of the same kind on the owner's machine, with
  its own snapshot copy, sandbox, environment and agent session id.
- **Front.** How a caller reaches the core: A2A (Pitwall, A2A clients) or
  MCP (plain Claude Code, MCP clients).
- **Message.** One message from the caller; there is only one kind. It is
  one **turn** (an A2A task; one MCP tool call). Its
  **reply** is what the agent said, and its **changes**, when the turn
  changed the copy at a "Can change code" capability, are a patch series.
- **Limits.** What the owner allows per share (messages per person per day,
  conversations at once, time per turn, total use per day), plus the
  agent's own vendor limits, which Pitwall passes on ([§5.8](sharing.md)).

## Parts

| File | Sections | What |
|---|---|---|
| [goals.md](goals.md) | §1–2 | User stories, goals, non-goals, what changes in tailnet-peers, the model |
| [teams.md](teams.md) | §3–4 | Discovery on the tailnet; teams, keys, signed records, gossip, conflicts, joining, members without Pitwall, Tailscale binding |
| [sharing.md](sharing.md) | §5 | Share as a setting, capability, the Share questionnaire, limits, who can change settings, suggestions, stored shape |
| [sessions.md](sessions.md) | §6 | Per-caller conversations, the snapshot copy, OS isolation per platform, native controls per agent kind, starting point, instructions, lifecycle |
| [messages.md](messages.md) | §7 | Message flow, reply capture, the owner in the loop, precedent, how patches travel back |
| [protocol.md](protocol.md) | §8, §10 | HTTP listener, identity, one core with an A2A front and an MCP front, Pitwall endpoints, local sockets, CLI, data shapes, storage; why both |
| [safety.md](safety.md) | §9 | Enforced vs soft, per platform, switches, settings out of reach, what remains, threats |
| [ui.md](ui.md) | §11 | People, Share dialog, owner sidebar and conversation pane, Ask panel, Review |
| [plan.md](plan.md) | §12–14 | Phases, tests, risks, out of scope, open questions |
| [references.md](references.md) | — | Sources checked, and what is still unverified |

Section numbers (§1–§14) are global across the files, so a reference like
§6.3 always means the same place.

Task lists, jobs and boards are **not** part of this design: people
already track work in GitHub Issues, Linear or chat, and every flow here
works with conversations alone ([goals.md](goals.md#non-goals-v1)).

## Phases at a glance
C1a MVP on macOS (A2A and MCP fronts, discovery, single-admin teams, Read
only shares, Seatbelt isolation, limits) → C1b signed multi-admin teams →
C2 "Can change code" and patches → C3 Linux hosting → C4 owner in the loop,
suggestions, precedent → C5 Windows (WSL2 route, native later) → C6
Tailscale binding and hardening → C7 more agent kinds, `pitwall mcp`.
About 18–27 weeks; details in [plan.md](plan.md).
