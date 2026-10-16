# Collaboration: safety

Part of the [collaboration spec](README.md).

## 9. Safety

### 9.1 Enforced and soft
Messages are untrusted input, possibly written by an agent that read
something hostile. So everything that matters is **enforced** around the
agent, the same way for every kind; instructions are only the **soft**
layer, and the UI says so.

| Share answer | Enforced by | Layer |
|---|---|---|
| Who | `whois` before any HTTP handling, team state, the owner's block list | Pitwall |
| Capability (read / write) | OS sandbox: copy not writable at Read only; writes only inside the copy at "Can change code" | OS sandbox, all kinds |
| What it can see | Snapshot copy with only the picked paths, minus exclusions, no history; the OS sandbox denies the rest of the disk | Pitwall + OS sandbox |
| Programs it may run | Exec allowlist/denylist | OS sandbox, all kinds (also written into the kind's per-command rules where they exist) |
| Start from the conversation | Only an owner-approved summary reaches sessions | Pitwall |
| Secrets and logins | Environment built from nothing; session `HOME`; owner's files unreadable | Pitwall + OS sandbox |
| Network | Only Pitwall's egress proxy is reachable; it allows the kind's model hosts and owner-added hosts | OS sandbox + proxy |
| Results leaving | Mechanical diff of the copy; held for Send at `write_approval` | Pitwall |
| Changing sharing settings | Shared sessions can't reach `pitwalld.sock` (sandbox) and are rejected by process tree; widening from CLI/agents always asks | OS sandbox + Pitwall |
| Limits | Per-caller quotas, running caps, queue, turn timeouts, daily agent time, overrides ([§5.8](sharing.md)) | Pitwall |
| Which built-in tools the agent uses (its file, web, shell tools by name) | The agent's own tool settings, where the kind has them ([§6.8](sessions.md)). The OS layer can't see tools by name, only their effects | Second layer (agent-native) |
| Off-limits topics, role, anything in the owner's instructions | Instructions | **Soft** |

### 9.2 Per platform
| Platform | Shared sessions | Isolation |
|---|---|---|
| macOS | Yes (C1a) | Seatbelt profile around the agent ([§6.3](sessions.md)) |
| Linux | C3, right after C2 | bubblewrap (mount namespace with only the copy, own network namespace with only the proxy), Landlock where available; needs `bubblewrap` and `socat` installed and, on Ubuntu 24.04+, an AppArmor profile for `bwrap` (Pitwall detects and explains) |
| Windows | C5 via WSL2 (the Linux backend inside a distro); native AppContainer / restricted token later; until then ask only | WSL2 + bubblewrap, later native |
| Any (asking) | Yes | — |

### 9.3 Defaults and switches
Settings → Collaboration, all off:
- **"Use teams"**: discovery, joining, asking. Outgoing only.
- **"Let teammates reach this machine"**: the HTTP listener ([§8.1](protocol.md)).
- **"Let my agents ask shared agents"**: [§8.6](protocol.md).

Sharing needs the second switch. Every share starts as a **draft**: nobody
can see it until the owner reads the summary and presses Go live ([§5.4](sharing.md)).

### 9.4 Settings are out of reach of shared sessions
Once live, neither a shared session nor any caller can change any sharing
setting, approve anything, or drive Pitwall. Sessions: the sandbox denies
`pitwalld.sock` and Pitwall's data folder, there is no `pitwall` on their
`PATH`, and the daemon rejects their process tree ([§8.5](protocol.md)). Callers: the
tailnet listener has no owner methods at all; the MCP endpoint offers only
the `ask_<slug>`, `check_turn` and `get_changes` tools. The owner's own pane of the
agent is not a shared session and keeps its normal CLI; its changes to
sharing go through approvals with a diff ([§5.5](sharing.md)).

### 9.5 What remains true (said in the docs, and in the summary)
- **Replies leak what the agent can read**: anything in the copy, and the
  agent account's config folder (hence the suggested separate account).
- **Shared sessions run as the owner's OS user** inside the sandbox, with
  the chosen agent account. Use is billed to that account, and vendor terms
  on letting others use a subscription apply ([§14](plan.md) question 6).
- **The sandbox is only as good as its profile.** A rule too wide (a
  `subpath` that includes more than meant, an allowed Mach service that
  leaks) widens every session. One profile template, reviewed and tested
  with canaries on each macOS release, rather than per-agent rules.
- **Callers are their Tailscale login.** Anyone who controls a node
  logged in as Bob is Bob. Teammates' Pitwalls are trusted to label their
  agents, and the label never grants anything.
- **Plain clients' agents decide when to ask.** The owner's limits bound
  what that costs; the caller's own client settings decide whether their
  agent may call the tool without asking them.

### 9.6 Threats

| Threat | Mitigation |
|---|---|
| A message tells a Read-only session to edit files or run commands | It can run commands (its own tools), but writes to the copy and everything outside fail: the sandbox, not the agent, says no |
| A message tells a session to exfiltrate secrets | Secrets not in the copy, no history, clean environment, the owner's files unreadable, network only to model hosts via the proxy; at `write_approval` the owner sees every diff before it leaves |
| A session (or a process it double-forked) tries to reach Pitwall or widen its share | Sandbox inherited by every descendant denies the socket and data folder; process-tree rejection as a second check |
| An owner's agent is tricked into widening a share | Widening always asks, shows the diff, can't be remembered |
| The owner's history leaks into sessions | Fresh by default; only an approved summary |
| A removed member keeps asking | Every request checked against the current team state; owner block list at once |
| A removed admin races with forged changes | Revocation wins at the fork point ([§4.4](teams.md)) |
| Someone on the tailnet impersonates a member | `whois` identifies the node's login from WireGuard keys; a header can't change it. A stolen node is removed in the Tailscale admin console |
| A member serves forged team records | Admin-signed records chained to the pinned team id |
| Agent↔agent loops burn the owner's quota | Chain depth and cycle check for Pitwall callers; per-caller quotas and daily agent time for everyone ([§5.8](sharing.md)) |
| Flood of requests or probes | Rate limits per caller, queue cap, probe limits |
| A result carries a malicious change to the caller | Shown in Review; applied only by the caller's click, into a worktree they pick |
| A teammate's agent floods a team list | Owner switch and per-agent budgets on the adding machine; per-device flood limits at receivers; admins can drop and fence |
| Tailscale ACLs block the port | Discovery shows "can't reach"; docs give an ACL grant example for `tcp:47421` |
