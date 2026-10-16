# Collaboration: phases, tests, risks, open questions

Part of the [collaboration spec](README.md).

## 12. Phases

After tailnet-peers.md P0 and P1 (`Tailnet` trait, listener rules, instance
key, audit log). Each phase usable on its own, macOS first.

| Phase | Scope | Effort | Done when |
|---|---|---|---|
| **C1a MVP: MCP, discovery, simple teams, Read only, isolation, limits** | HTTP listener (`tiny_http`, `whois` before handling), MCP Streamable HTTP server (initialize, `tools/list` per caller, `tools/call` for `ask_<slug>` / `check_turn` / `get_changes`, progress, cancel, `list_changed`) and the daemon's MCP client. Pitwall endpoints: info, discover, teams with a **single admin** (records already signed and chained, pull replication), join by request and invite link, members without Pitwall and their setup lines. Limits ([§5.8](sharing.md)): per-caller quota, running cap and queue, turn timeout, daily agent time, overrides, vendor-limit pass-through. Share dialog (seven steps, sensitive-file scan, summary, try as guest, draft → live) at **Read only**. Snapshot copies, **Seatbelt sandbox + egress proxy + clean environment** for any kind, per-caller sessions with the kind's resume, approved-summary base + fork where possible. Reply capture tiers (a) Claude Code and Codex, (b) Gemini CLI if verified, (c) everyone else. `[share]` kind data for Claude Code, Codex, Gemini CLI. `pitwall ask`, `people`, `team`, `share` (approvals and diffs). People screen, Ask panel, owner sidebar group and session pane, usage, block, stop sharing, audit | 7–9 wk | Bob joins Alice's team from discovery and asks `alice/api-expert` (Claude Code) and `alice/docs` (Gemini CLI) from the Ask panel and from an agent; Erin, added by login with no Pitwall, runs the setup line and her plain Claude Code gets a reply; Dave over his quota gets a clear refusal; Alice tried as guest first; guest probes for `.env`, `~/.ssh` and the network get nothing; a double-forked process can't reach `pitwalld.sock` |
| **C1b MVP: signed multi-admin teams** | Several admins, fork choice ([§4.4](teams.md)), re-apply of dropped ops, announce/push, devices, rotation, self-leave, last-admin rule, "re-create team", fingerprints, invite code hashes replicated | 1.5–2 wk | The team survives the creator's machine being off for a week; two admins changing at once converge; a removed admin can't win |
| **C2 Can change code** | `write` and `write_approval`, per-turn commits and mechanical change detection, format-patch results (inline and in parts), held results with Send/Discard, caller-side Review, Apply (`git am -3`), "Redo on my code", "let my agents ask" budgets and loop checks | 2–2.5 wk | Carol asks, asks for a change, asks why; she applies the patch in a worktree; at `write_approval` nothing left before Alice's Send |
| **C3 Linux hosting** | Linux backend of `SessionSandbox`: bubblewrap mount and network namespaces, egress proxy via a Unix socket relay, Landlock exec rules where available; detection and guidance for missing `bubblewrap`/`socat` and for Ubuntu 24.04+ AppArmor user-namespace restriction; Linux kind data (install paths); the isolation test suite on Linux CI | 1.5–2 wk | A Linux machine hosts a Read-only and a "Can change code" share; the probes from C1a get nothing; Ubuntu 24.04 without the profile shows the fix instead of failing silently |
| **C4 Owner in the loop, suggestions, then precedent** | Needs-you notifications for shared sessions, owner turns shown in the caller's thread, "Notify me on every reply"; `pitwall suggest share|skill` and cards; then "Save as precedent" | 1–1.5 wk | Alice answers a blocked session from a notification and Bob sees it; the Race Engineer's suggestion opens a prefilled Share dialog |
| **C5 Windows hosting** | (1) WSL2 route: holder starts the session through `wsl.exe` into a chosen distro, Linux backend inside, agent installed there, copy in the distro's filesystem (1–1.5 wk). (2) Native backend (AppContainer or restricted token): evaluation, then build (2–3 wk, may slip to later) | 3–4.5 wk (1–1.5 for WSL2 only) | A Windows machine with WSL2 hosts shares; native backend shipped or a documented decision |
| **C6 Tailscale binding and hardening** | App-capability membership and ceilings, two-signature admin removal option, fuzzing of the HTTP endpoints and records | 1–1.5 wk | A team bound to `group:platform` follows Tailscale changes; fuzzers run in CI |
| **C7 More kinds, local MCP, optional fronts** | `pitwall mcp` (one local server merging teammates' tools), `[share]` data and native controls for more kinds as they're verified, optional HTTPS with `tailscale cert`, optional A2A front, optional push/PR result mode | 1.5–2 wk | Bob's Codex agent uses every teammate's shared agents through `pitwall mcp`; opencode and Aider shares tested |

Total: about 17–25 weeks (about 15–22 with only the WSL2 route on Windows).

### Tests
- **Fake tailnet, three instances** (alice, bob, carol), as tailnet-peers.md
  §9, with `whois` per source port (`CapMap` in the fake file for C6); the
  HTTP listener accepts only what the fake `whois` knows.
- **MCP conformance:** golden fixtures for initialize, `tools/list`,
  `tools/call` results (structured and text content), progress, cancel and
  errors against the spec's schema; the MCP Inspector and real Claude Code
  (`claude mcp add --transport http` then `claude -p` using the tool) as
  black-box clients in an opt-in test, including a turn longer than the
  client's tool timeout.
- **Identity:** the fake `whois` decides everything; headers can't change
  the login; a caller outside every audience gets an empty `tools/list`
  and "unknown tool"; a member added by login with no devices is accepted.
- **Limits:** quota resets at the owner's midnight; running cap and queue
  order; turn timeout returns the partial reply; daily agent time pauses
  the share; overrides and block; a recorded vendor-limit screen per kind
  gives `agent_limit` with the reset time.
- **Team records:** generated op sequences from several admins with random
  delivery and partitions; all members converge; "a removed admin never
  wins"; golden JSON for records and their canonical form.
- **Scope:** the snapshot contains exactly the picked paths minus
  exclusions; no `.git` link back; blob ids equal the owner's; the
  sensitive-file scan against a fixture tree.
- **Isolation (macOS in C1a, Linux in C3, WSL2 in C5; per kind and for a fake kind):** reads of `~/`,
  `~/.ssh`, the owner's project and `~/.pitwall` fail; writes outside the
  copy fail, and inside it fail at Read only; network except the proxy
  fails, and the proxy refuses hosts not allowed; the environment has no
  owner variables; a double-forked child is still sandboxed and can't
  connect to `pitwalld.sock`.
- **Sessions:** same caller resumes (kind's resume), different callers
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
| `sandbox-exec` is deprecated and its profile language undocumented | The kernel mechanism is what Codex and Claude Code use too; keep the profile small; separate user or container as fallbacks ([§14](plan.md) question 7) |
| Linux hosts lack `bubblewrap`/`socat`, or Ubuntu 24.04+ blocks user namespaces | Detect at "Let teammates reach this machine", show the exact install and AppArmor-profile steps, never weaken the system setting ourselves |
| WSL2 adds setup (a distro, the agent installed inside) and file-system latency | Only the share host needs it; asking works natively; native backend later |
| The OS layer can't tell an agent's tools apart, so an agent may try things and fail noisily | Write the kind's native controls too ([§6.8](sessions.md)) so the agent knows its limits |
| MCP spec churn | Negotiate the protocol version in `initialize`, golden fixtures, test against Claude Code on each release |
| Clients give up on long tool calls | `wait_seconds` below the shortest client timeout found, then `check_turn`; progress notifications; measured per client ([§14](plan.md) question 17) |
| An HTTP server in Pitwall widens the attack surface | `whois` before any parsing, tailnet-only bind, size limits, fuzzing (C6) |
| Reply capture from terminal output (tier c) is rough for some kinds | Marked as such to the caller; add kind data for hooks or a non-interactive mode as they're verified |
| Membership conflicts and revocation lag without a server | Fork choice at the fork point, revocation wins, owner block list at once |
| The owner's agent account cost and vendor terms | Quotas, usage view, separate account suggested, [§14](plan.md) question 6 |
| Scope creep toward a hosted product | Non-goals, [§13](plan.md) |

The two risks the first draft rated highest shrink with OS isolation:
**"enforcement depends on each agent's flags"** becomes "one policy Pitwall
owns, the same for every kind, with one backend per OS", with the agents'
native controls as a second layer; and the **socket-escape gap** (a
double-forked process leaving the process tree) closes because both
Seatbelt and bubblewrap namespaces follow every descendant and deny the
socket (a first macOS demo confirmed a detached child stays blocked,
[§6.3](sessions.md)), with the process-tree check kept as a second layer.

---

## 13. Out of scope: a server or B2B version
A hosted directory, accounts, SSO, org-wide policy, billing, audit export,
relay for people outside a tailnet, or shared agents running on shared
infrastructure. Parked. The local design keeps the door open: the channel
is MCP, team records are self-contained and signed, and identity is behind
the `Tailnet` trait.

---

## 14. Open questions
1. **Admin removal.** Require two admin signatures to remove an admin when a
   team has three or more admins (proposed: yes; off for smaller teams)?
2. **Invite links.** One-time only (proposed), or also reusable until
   revoked?
3. **Domain.** The app capability (`<domain>/cap/pitwall`) needs a domain the project controls. Which one?
4. **Full conversation as the base.** Offer forking the owner's whole
   conversation (with an amber warning) besides fresh and summary, or never?
5. **Offline owners.** Refuse messages while the owner is offline
   (proposed), or queue them on the caller's side and send on reconnect?
6. **Agent subscriptions.** Shared sessions use one of the owner's agent
   accounts. Do the vendors' terms allow teammates to use it this way, or
   should Pitwall recommend (or require) an API-key or team-plan account for
   shares? Needs a check per vendor.
7. **Isolation beyond the defaults.** Seatbelt (macOS), bubblewrap (Linux)
   and WSL2 (Windows) are the plan. For Windows native, AppContainer or a
   restricted token? And for owners who want more everywhere: a separate OS
   user per share, a container, or an agw VM?
8. **Shared nodes across tailnets.** Allow callers whose nodes are shared
   into the tailnet (Tailscale node sharing), if `whois` identifies them
   reliably?
9. **HTTPS.** Is plain HTTP inside WireGuard enough for all MCP clients, or
   should Pitwall serve HTTPS with `tailscale cert` by default?
10. **Reply source.** For kinds that have both, prefer the interactive
    terminal with hooks (owner can take over naturally) or the
    non-interactive mode (cleaner replies)?
11. **Session keying for agents.** One session per asking agent (proposed),
    or one per person, shared by all of that person's agents?
12. **Git history in scope.** Offer a filtered clone with history (so "why
    was this changed?" works), and how to keep excluded files out of it?
13. **Saved prompts.** Declared entry points with typed input ("actions")
    were dropped from v1. Add them later as a shortcut, e.g. MCP prompts that
    expand to an owner-written prompt?
14. **Owner notes.** Should the owner be able to type into a shared session
    without the caller seeing it?
15. **One conversation for all of a person's plain clients.** A plain
    client sends no agent label, so all of Erin's Claude sessions share one
    conversation (proposed), unless one passes `new_conversation`. Or key
    by MCP client name as well?
16. **Programs list defaults.** Allowlist by default (safer, may break
    agents that shell out to unexpected tools) or denylist by default
    (friendlier, leakier)? Proposed: allowlist at Read only, denylist at
    "Can change code".
17. **Tool-call timeouts in other clients.** Claude Code's are known (5 min
    idle for HTTP, [§8.3](protocol.md)). Does a progress notification count as
    activity for its idle timer, and what do Codex, Gemini CLI and Cursor
    allow? Could raise the `wait_seconds` cap.
18. **Setup lines for plain members.** One `claude mcp add` per owner
    (proposed), or let one member's Pitwall list the others' endpoints in
    its tool descriptions so a plain user adds one line? Never relaying the
    calls themselves either way.
