# Pitwall collaboration: teams and shared agents

People who use Pitwall on the same Tailscale tailnet form **teams**, and each
person can **share** (expose) some of their agents with others. A teammate
sends a shared agent messages: a question, then "add a dry-run flag", then
"why did you change that?", all in one conversation. If the share lets it
change code, any turn that changed its copy comes back as a patch. The asker
can be a person, in Pitwall's Ask panel, or any of their agents, through
`pitwall ask` or a local MCP server. The owner decides what is shared and
with whom, sees every conversation, can watch or take over, and can stop
sharing at any time.

**Status: design, not built.** It builds on
[tailnet-peers.md](../tailnet-peers.md) (instance key, `whois` identity,
listener rules, audit log) and comes after that spec's P0 and P1. Skills
assigned to a shared agent come from [rules.md](../rules.md).

## Three choices that shape it
- **No Pitwall server.** Everything runs on the users' own machines, over
  their own tailnet. A hosted or B2B version is parked
  ([plan.md §13](plan.md)).
- **A2A is the channel.** Asker and shared agent talk over the open A2A
  protocol (HTTP+JSON binding, SSE streaming) on the tailnet. Teams,
  discovery and membership use a few small Pitwall endpoints beside it
  ([protocol.md](protocol.md)).
- **Any agent kind, on either side, with native behaviour.** Any agent
  Pitwall runs can ask and can be shared. The agents never speak A2A and
  never follow Pitwall-only rules: Pitwall sends the message as ordinary
  input, reads the reply the way it already reads agents, and enforces
  limits through the session's surroundings: an OS sandbox (Seatbelt on
  macOS, bubblewrap on Linux, WSL2 on Windows), a copy holding only what was
  shared, and a clean environment ([sessions.md](sessions.md)).

## Glossary
- **Person.** A Tailscale login (`UserProfile.LoginName` from `whois`).
- **Device.** One Pitwall installation: its instance key plus the Tailscale
  node (`Node.StableID`) it runs on. A person has one or more devices.
- **Team.** A named set of people and their devices, with admins, identified
  by the hash of its first record, never by its name.
- **Share** ("shared agent", "exposed agent"). A setting on one of the
  owner's ordinary agents: audience, capability (`read`, `write`,
  `write_approval`), scope, start, access, off-limits topics, limits. The UI
  says **Share…**.
- **Session.** One asker's conversation with a shared agent (an A2A
  `contextId`): a real Pitwall agent of the same kind on the owner's
  machine, with its own snapshot copy, sandbox, environment and agent
  session id.
- **Message.** One message from the asker; there is only one kind. Each
  becomes an A2A task whose artifacts are the **reply** (what the agent
  said) and, when the turn changed the copy at a "Can change code"
  capability, the **result** (a patch series).
- **Task list.** My list (local) or a team's list (replicated among its
  members as a log of signed ops). Its **items** have tags, a status
  (proposed, open, taken, done, dropped), an **assignee** (who it is for)
  separate from the taker (who works on it), and a result patch when the
  one who finished it changed code. Shared agents may only *propose* items,
  if their share allows ([§15](tasks.md)).

## Parts

| File | Sections | What |
|---|---|---|
| [goals.md](goals.md) | §1–2 | User stories, goals, non-goals, what changes in tailnet-peers, the model |
| [teams.md](teams.md) | §3–4 | Discovery on the tailnet; teams, keys, signed records, gossip, conflicts, joining, Tailscale binding |
| [sharing.md](sharing.md) | §5 | Share as a setting, capability, the Share questionnaire, who can change settings, suggestions, stored shape |
| [sessions.md](sessions.md) | §6 | Per-asker sessions, the snapshot copy, OS isolation per platform, native controls per agent kind, starting point, instructions, lifecycle |
| [messages.md](messages.md) | §7 | Message flow, reply capture, the owner in the loop, precedent, how patches travel back |
| [protocol.md](protocol.md) | §8, §10 | HTTP listener, identity and the device extension, A2A mapping, Pitwall endpoints, local sockets, CLI, data shapes, storage; why A2A; MCP |
| [safety.md](safety.md) | §9 | Enforced vs soft, per platform, switches, settings out of reach, what remains, threats |
| [ui.md](ui.md) | §11 | People, Share dialog, owner sidebar and session pane, Ask panel, Review |
| [tasks.md](tasks.md) | §15 | Tagged task lists: my list and team lists, signed op log, permissions, agents via `pitwall task`, giving items to agents, watch rules, results |
| [plan.md](plan.md) | §12–14 | Phases, tests, risks, out of scope, open questions |
| [references.md](references.md) | — | Sources checked, and what is still unverified |

Section numbers (§1–§15) are global across the files, so a reference like
§6.3 always means the same place.

## Phases at a glance
C1a MVP on macOS (A2A, discovery, single-admin teams, Read only shares,
Seatbelt isolation) → C1b signed multi-admin teams → C2 "Can change code"
and patches → C2b tagged task lists → C3 Linux hosting → C4 owner in the loop, suggestions,
precedent → C5 Windows (WSL2 route, native later) → C6 Tailscale binding
and hardening → C7 MCP, more agent kinds. About 20–28 weeks; details in
[plan.md](plan.md).
