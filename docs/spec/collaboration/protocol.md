# Collaboration: protocol, MCP and API

Part of the [collaboration spec](README.md).

## 8. Protocol and API

### 8.1 One HTTP listener on the tailnet
- **Port.** Its own port, 47421/tcp proposed (beside peers' framed-TCP
  47420), configurable. One listener serves the MCP endpoint (`/mcp`) and
  the Pitwall team endpoints (`/pitwall/v1/…`).
- **Same rules as the peer listener** (tailnet-peers.md §4): bound only to
  this node's Tailscale IPs, source address checked against the tailnet
  ranges, and `whois` must succeed **before any HTTP handling**; otherwise
  the connection is closed without a byte. The `whois` result (login, node,
  tags, `CapMap`) is the request's identity.
- **Plain HTTP inside WireGuard.** Traffic is already encrypted and
  authenticated end to end by the tailnet, so `http://` URLs work in
  `claude mcp add`. HTTPS with a `tailscale cert` certificate is an option
  for clients that insist ([§14](plan.md) question 9).
- **Server.** A small blocking HTTP/1.1 server on its own threads, no async
  runtime: `tiny_http` is the candidate (a request per worker thread;
  streaming responses use chunked transfer from a `Read` fed by a channel),
  so an SSE response is written event by event. Client side (Pitwall
  callers): `ureq`, reading SSE line by line. Both to be confirmed in a C1a
  spike and recorded in gpui/licensing.md.
- **Limits** as peers: per caller connections, requests per second, body
  size (2 MiB), at most 8 open streams per caller.

### 8.2 Who is calling: Tailscale, nothing else
- **Every caller is its `whois` login and node.** That is the whole check
  for the MCP endpoint: a caller may use a share when the share's audience
  names the login, includes a team the login is a member of (team records
  list logins, [§4](teams.md)), or is the whole tailnet. There are no
  accounts, tokens, OAuth or pairing for callers, and no difference between
  a Pitwall caller and a plain Claude Code one. (MCP's HTTP authorization
  is optional; it is not used.)
- **Pitwall callers may add two labels**, plain headers, trusted as much as
  the person whose node sent them:

  ```
  Pitwall-Agent: client-fix@bob-mbp     (the asking agent, from Bob's process tree)
  Pitwall-Chain: c_17,alice/api-expert  (loop check, §8.6)
  ```

  The label only keys the conversation ([§6.1](sessions.md)) and names it
  for the owner. A label can't widen anything: access is the login's.
- **The instance key** (tailnet-peers.md §5) is used only to sign team
  records ([§4](teams.md)), never per request.

### 8.3 The MCP endpoint
One endpoint per owner: `http://<owner's tailnet name or IP>:47421/mcp`,
MCP over **Streamable HTTP** (JSON-RPC over `POST`, answered as JSON or as
an SSE stream). A plain Claude Code user adds it once:

```
claude mcp add --transport http alice http://studio.tail1234.ts.net:47421/mcp
```

**Server capabilities:** `tools` with `listChanged: true`. No resources,
prompts or sampling in v1.

**`tools/list` is the directory.** It returns, for this caller only, one
tool per live share they may use, plus two helpers. A caller outside every
audience gets an empty list. When a share goes live, pauses, or its
audience changes, connected clients get `notifications/tools/list_changed`.

| Tool | Input | What it does |
|---|---|---|
| `ask_<slug>` (e.g. `ask_payments_expert`) | `message` (string, required), `new_conversation` (bool), `wait_seconds` (int, default 120, max 240), `my_changes` (string, a patch series: "redo on my code", [§7.5](messages.md)) | Sends one message into the caller's conversation and waits for the turn, up to `wait_seconds` |
| `check_turn` | `turn` (string), `wait_seconds` | The state of a turn that was still working, waiting for the owner, or holding changes; waits like `ask` |
| `get_changes` | `turn`, `part` (int, default 1) | A turn's patch series in parts of 40,000 characters, for series too big to inline |

The `ask_<slug>` description is built from the share: its name and
description, the capability in plain words ("read only" / "can change
code; changes come back as a patch"), the agent kind ("runs on Claude
Code"), and "N messages left today". The agent on the caller's side reads
that like any tool description.

**Results.** Every result has the MCP `structuredContent` described by the
tool's `outputSchema`, and the same information as text content (the
reply first, then a short status line, then the patch when inlined), so
clients that only show text still work:

```json
{
  "conversation": "c_8f2k",
  "turn": "c_8f2k/7",
  "status": "done",                     // done | working | waiting_for_owner | failed
  "reply": "Refunds older than 30 days go through …",
  "replySource": "hook",                // hook | session-file | non-interactive | terminal
  "changes": {                          // only when the turn changed the copy
    "state": "sent",                    // sent | held (waiting for the owner's OK) | discarded
    "baseCommit": "3f2a1c9…", "files": 2, "added": 14, "removed": 3, "patches": 1,
    "patch": "From 7c1e… Mon Sep 17 00:00:00 2001\n…",   // inline up to 40,000 characters, else get_changes
    "parts": 1
  },
  "left": {"messagesToday": 17, "resetsAt": "2026-10-17T00:00:00+02:00"}
}
```

- **Waiting.** `ask` returns when the turn ends, or after `wait_seconds`
  with `status: "working"` and the `turn` id; the caller's agent then calls
  `check_turn` when it wants. When the client sent a `progressToken`,
  Pitwall sends `notifications/progress` while waiting ("2nd in line",
  "working", "waiting for alice"). Claude Code's hard limit per tool call
  is about 28 hours by default, but it also gives up on an HTTP tool call
  that is idle for 5 minutes (`CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`), so
  `wait_seconds` is capped at 240. Other clients: **unverified**
  ([References](references.md)).
- **Size.** Claude Code warns above 10,000 tokens of tool output, cuts at
  25,000 (`MAX_MCP_OUTPUT_TOKENS`), and saves text results over 50,000
  characters to a file. So a result inlines at most 40,000 characters of
  patch; larger series come through `get_changes` in parts of that size.
- **Changes are a patch series** (`git format-patch` output). A Pitwall
  caller shows it in Review; a plain caller's agent applies it with
  `git am -3` or `git apply`, as it would any patch.
- **Refusals** are tool results with `isError: true` and a reason in
  `structuredContent.refused`: `quota` (with `resetsAt`), `busy` (queue
  full), `paused`, `share_total` (the share's daily total is used up),
  `agent_limit` (the agent's own vendor limit, with `until` when the agent
  reported one), `loop`. The caller's agent sees a plain sentence ("Daily
  limit reached; resets at 09:00") and decides what to do.
- **Cancel.** `notifications/cancelled` for a running `ask` or `check_turn`
  stops the turn (as Stop) when the turn belongs to that request; a queued
  message is dropped.
- **Not in the audience** and **unknown tool** look the same: the tool
  isn't listed, and calling it is a JSON-RPC error "unknown tool".
- **MCP sessions** (`Mcp-Session-Id`) are transport only. Conversations are
  keyed by the caller ([§6.1](sessions.md)), so a new MCP session or a
  restarted client continues the same conversation.

#### 8.3.1 What the agent receives
The shared agent never sees MCP. Pitwall turns the message into ordinary
input: the caller's text, as written. By default one factual line goes in
front so the agent knows who is talking:

```
From bob@example.com (agent client-fix on bob-mbp):
Where are refresh tokens rotated?
```

(For a plain client: `From bob@example.com (Claude Code over MCP):`, from
the client name in MCP's `initialize`.) No request kinds, no reply
instructions; the owner can turn the "From" line off per share. When
something changed under the agent since its last turn, Pitwall adds one
factual line after it ("Files updated to main @ 3f2a1c", "Your changes
from the previous turn were not sent"). The owner view shows exactly what
was typed.

### 8.4 Pitwall endpoints beside MCP
Teams and discovery are small JSON endpoints on the same listener, under
`/pitwall/v1/`. Identity is `whois`, as for MCP; team records carry their
own signatures ([§4](teams.md)). Plain Claude users never need them.

| Endpoint | Caller | What |
|---|---|---|
| `GET /info` | any tailnet caller | `{version, login, label, listedTeams, mcp: "/mcp"}`: the discovery probe ([§3](teams.md)) |
| `GET /teams:discover` | any tailnet caller | listed teams this Pitwall knows, as signed records |
| `POST /teams/{t}/join`, `POST /teams/{t}/invites:redeem` | anyone with Pitwall | join request, invite ([§4.6](teams.md)) |
| `POST /teams/{t}/heads`, `GET /teams/{t}/records?after=n`, `POST /teams/{t}/announce`, `POST /teams/{t}/cosign` | member devices | replication and conflicts ([§4.3](teams.md)–4.4) |
| `GET /teams/{t}/requests` | admins | pending join requests, forwarded |
| `GET /events` (SSE) | member devices | `team.announce`, `shares.changed` (refresh `tools/list`) |

### 8.5 Local sockets and who may call what

| Caller (how the daemon knows) | Reaches | May call |
|---|---|---|
| **Owner UI** (in-process GPUI app) | — | Everything, including all `share.*` methods |
| **Owner's CLI and agents** (CLI socket; agent vs user from the process tree, as approvals do today) | `pitwalld.sock` | `share.*` reads directly; `share.create` and `share.guest` on drafts directly (nobody else can see a draft); other `share.*` changes through approvals (`share.go_live` and widening always ask, not rememberable; narrowing direct, [§5.5](sharing.md)); `suggest.*`; `ask.*` (the daemon calls the owner's MCP endpoint) with the "let my agents ask" switch |
| **A shared session** (inside its sandbox; process tree as a second check) | the hook endpoint, as any agent | Hook events for its own agent id (status, the Stop text). Nothing on `pitwalld.sock` |
| **Anyone on the tailnet** | the HTTP listener | MCP ([§8.3](protocol.md)) and team endpoints ([§8.4](protocol.md)) |

Shared sessions are cut off from Pitwall by construction: the sandbox
denies `pitwalld.sock` and the data folder ([§6.3](sessions.md)); their
environment has no `PITWALL_*` variables and no `pitwall` on `PATH`; and the
daemon still rejects any connection whose peer process descends from a
shared session's holder (`Caller::Shared` → `forbidden`). The hook endpoint
already accepts only hook events, keyed to the agent id Pitwall gave the
session.

**Owner-only local methods** (UI, or CLI/agents through approvals; never on
the tailnet listener, never from shared sessions): `share.get`,
`share.list`, `share.create` (draft), `share.set` (returns the diff it
applied), `share.go_live`, `share.pause`, `share.resume`, `share.stop`,
`share.guest`, `share.sessions`, `share.block`, `share.summary`,
`result.send`, `result.discard`, `team.*` changes (create, approve, invite,
add member, promote, remove, bind), `collab.settings.*`. Typing into or
prompting a shared session uses the ordinary agent methods (`term.*`,
`agent.prompt`), owner only.

### 8.6 CLI (asking is agent-neutral)
Pitwall users and their agents ask through the local daemon, which calls
the owner's MCP endpoint with the `Pitwall-Agent` label. `pitwall ask` is
an ordinary shell command, so **any agent kind** that can run a command can
ask; `pitwall mcp` ([§10.2](protocol.md)) serves agents that prefer MCP
tools.

```bash
# Caller side
pitwall people list                                   # people in my teams and their shared agents
pitwall team list | show <team> | join <team|link> | leave <team>
pitwall ask alice/api-expert "where are refresh tokens rotated?"   # waits; prints {reply, conversation, turn, changes}
pitwall ask alice/api-expert "add a dry-run flag to the jobs CLI"  # same command, same conversation
pitwall ask alice/api-expert - < message.md --new --timeout 900
pitwall ask list [--owner alice]                      # my messages, replies, changes
pitwall result apply <turn> [--worktree new|<agent>]  # asks the user; git am -3
pitwall result redo <turn>                            # asks the user: sends my commits since base

# Owner side (CLI or the owner's own agents; changes go through approvals)
pitwall share list | show <agent>
pitwall share create <agent> --team platform --capability read \
    --paths src/,docs/ [--exclude …] [--start fresh|summary] [--programs allow:git,rg,cargo] \
    [--messages-per-day 20] [--instructions -]       # a draft; prints the summary and excluded files
pitwall share guest <agent> --probes                 # runs the guest probes on the draft; prints results
pitwall share set <agent> --add-team payments --capability write_approval   # prints the diff; widening asks
pitwall share set <agent> --remove-person bob@example.com                   # narrowing: direct
pitwall share go-live <agent> | pause <agent> | stop <agent>
pitwall share sessions <agent>                        # per caller: state, usage
pitwall share setup                                   # the `claude mcp add` line for this machine
pitwall suggest share <agent> --reason "…"            # a card for the owner, nothing more
pitwall suggest skill <skill> --agent <agent> --reason "…"

# Teams (admins)
pitwall team add-member <team> bob@example.com       # works for people without Pitwall; prints their setup lines
```

- Output, errors and exit codes as today (JSON, exit 3 for a denied
  approval). New error codes: `offline`, `busy`, `quota`, `share_total`,
  `agent_limit`, `not_allowed` (my agents asking without the switch),
  `no_answer`, `loop`, `forbidden`.
- **Agents asking:** refused with `not_allowed` unless Settings →
  Collaboration → "Let my agents ask shared agents" is on (off by default;
  per project override). With it on: 20 messages per hour per asking agent,
  `result apply` and `result redo` still ask the user, and the request is
  labelled with the agent's id from the process tree, never from an
  argument. This switch governs Pitwall's own `ask`; a plain MCP client the
  user configured themselves is the user's choice.
- **Loops.** `Pitwall-Chain` carries the turns and shares a message came
  from. Depth limit 2; a chain containing the target share is refused with
  `loop`. Shared sessions can't ask anyone (no CLI, no network beyond their
  model), so chains only grow through people's ordinary agents. A plain
  client sends no chain and a modified one can lie; limits are the real
  bound ([§5.8](sharing.md)).

### 8.7 Data shapes
A tool as `tools/list` returns it:

```json
{
  "name": "ask_api_expert",
  "title": "alice/api-expert",
  "description": "API expert (alice@example.com): questions about the API service — auth, billing, jobs. Read only. Runs on Claude Code. One conversation per caller; it remembers earlier messages. 17 of 20 messages left today.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "message": {"type": "string"},
      "new_conversation": {"type": "boolean"},
      "wait_seconds": {"type": "integer", "minimum": 0, "maximum": 240},
      "my_changes": {"type": "string", "description": "a git format-patch series to apply to the agent's copy first"}
    },
    "required": ["message"]
  },
  "outputSchema": { "…": "the result shape in §8.3" },
  "annotations": {"readOnlyHint": true, "openWorldHint": false},
  "_meta": {"pitwall/share": {"owner": "alice@example.com", "slug": "api-expert",
            "capability": "read", "kind": "claude", "messagesPerDay": 20, "left": 17}}
}
```

Pitwall-side views (TS, generated from `pitwall-proto` as today):

```ts
interface TeamCard { team: string; name: string; description: string;
  admins: string[]; members: number; version: number; listed: boolean;
  join: "request"|"closed"; fingerprint: string; bound: boolean;
  mine: "member"|"admin"|"requested"|null }
interface DirectoryEntry { endpoint: string; tool: string; owner: string; slug: string;
  name: string; description: string; capability: "read"|"write"|"write_approval";
  kind: string; left: number|null; online: boolean; lastSeen: string|null;
  via: string[] /* team ids, "direct" or "tailnet" */ }
interface AskView { turn: string; conversation: string; share: string; text: string;
  from: "me"|"owner";
  status: "queued"|"working"|"waiting_for_owner"|"done"|"failed"|"refused";
  statusMessage: string|null; reply: string|null; replySource: string|null;
  changes: { state: "held"|"sent"|"discarded"; files: number; added: number; removed: number;
             baseCommit: string; patches: number; path: string|null } | null;
  at: string; doneAt: string|null }
interface SharedSessionView { conversation: string; share: string;
  caller: { login: string; agent: string|null; client: string|null; guest: boolean; pitwall: boolean };
  state: "new"|"starting"|"ready"|"working"|"blocked"|"stopped"|"closed";
  agentId: string; turns: number; results: number; held: number;
  usage: { minutesToday: number; messagesToday: number; tokens: number|null }; lastAt: string|null }
type ShareDiff = { field: string; from: unknown; to: unknown; widens: boolean }[]
```

### 8.8 Storage

```
<PITWALL_HOME>/collab/
  (settings in ui.json, collab.*, like every setting)
  discovered.json            Pitwall nodes and listed teams seen
  directory.json             teammates' tools, cached from tools/list
  teams/<team>/chain.jsonl   signed records
  teams/<team>/invites.json  admin only (hashes replicate)
  shares/<share>.json        share settings (owner)
  shares/<share>/base/       instructions, approved summary, PRECEDENTS.md
  shares/<share>/s/<conv>/   work/ (snapshot repo), home/, tmp/, sandbox.sb
  usage.jsonl                owner side: per share, per caller, per day
  asks.jsonl                 my messages and replies (caller)
  results/<turn>/            received patch series (caller)
  audit.jsonl                owner side: every turn, result decision, share change (with diff), join, refusal
  suggestions.json           open and dismissed suggestions
```

The audit log uses tailnet-peers.md §5's format with `kind:"collab"`,
rotates at 1 MiB × 3, and is shown in the owner view and Settings.

---

## 10. MCP as the channel

### 10.1 Why MCP
Earlier drafts used Pitwall's own framed protocol, then A2A. MCP replaces
both:
- **Plain Claude Code users can connect with one command.** Claude Code
  (and most agent tools) already speak MCP over HTTP; nobody needs Pitwall
  to ask. A2A would need a client most people don't have.
- **It is the native interface for agents.** A shared agent shows up as a
  tool in the caller's agent, which decides when to use it, like any other
  tool. Nothing is added to the caller's agent beyond a tool.
- **Pitwall callers use the same endpoint.** One server, one set of checks,
  one audit log; Pitwall is just a richer client (directory, Ask panel,
  Review).
- **Identity stays Tailscale's.** No OAuth server on every owner's laptop.
- **What MCP doesn't cover stays Pitwall's,** beside it on the same
  listener: teams, discovery and gossip of signed records ([§8.4](protocol.md)).
- **Cost.** Long turns don't fit a single tool call well: `wait_seconds`
  plus `check_turn` covers it, and progress notifications where clients
  support them ([§8.3](protocol.md)). Large patches come in parts.
- **Peers stay framed TCP** (tailnet-peers.md): terminals and screen frames
  aren't conversations.

### 10.2 `pitwall mcp`: one local server for Pitwall users' agents
A local stdio MCP server on the caller's machine that merges every
teammate's tools into one (`ask_alice_api_expert`, …) plus
`list_shared_agents`, and forwards calls through the daemon with the
`Pitwall-Agent` label. So the "let my agents ask" switch, budgets and loop
checks apply, and agents don't need one `claude mcp add` per teammate. It
calls owners directly; it never relays for anyone else.

### 10.3 A2A later
An A2A front for the same shares (Agent Card per share, `contextId` = the
conversation) can be added beside `/mcp` if A2A clients show up. Not in
v1.
