# Collaboration: protocol, A2A, MCP and API

Part of the [collaboration spec](README.md).

## 8. Protocol and API

### 8.1 One HTTP handler, served by every network
- **One handler.** Collaboration is one HTTP handler with three groups of
  routes: the A2A front (`/a2a/<slug>/…`), the MCP front (`/mcp`) and the
  Pitwall team endpoints (`/pitwall/v1/…`). It is handed to every enabled
  network backend's `serve` ([§15.3](network.md)); it never opens a socket itself.
- **The backend authenticates first.** A request reaches the handler only
  with a `Caller` the backend produced ([§15.4](network.md)): on Tailscale after
  `whois` on the connection, on the Cloud after checking the signed
  assertion. Otherwise the connection is closed without a byte.
- **Transport is the backend's.** Tailscale serves plain HTTP inside
  WireGuard on 47421/tcp (beside peers' 47420), optionally HTTPS with
  `tailscale cert` ([§14](plan.md) question 9); the Cloud serves through its relay
  tunnel ([§15.5](network.md)–15.6).
- **Server.** A small blocking HTTP/1.1 server of Pitwall's own on
  `httparse`, on its own threads, no async runtime (decided in C1a):
  `tiny_http` accepts and parses on its own threads, so it can't run the
  identity check before reading a byte, and it answers dropped requests
  with a 500 instead of closing silently. Streaming responses use chunked
  transfer from a `Read` fed by a channel, so an SSE response is written
  event by event; long-lived streams send heartbeats. Client side (Pitwall
  callers): `ureq` 3 through `Network::client`, reading SSE line by line.
  Recorded in gpui/licensing.md.
- **Limits** per caller, the same on every network: connections, requests
  per second, body size (2 MiB), at most 8 open streams.

### 8.2 Who is calling: whatever the network proves
- **Every request carries a `Caller`** from its backend: identity, person,
  device when it is a Pitwall, network and groups ([§15.2](network.md)). A caller may
  use a share when its audience names the person, includes a team the
  person is a member of, includes one of their provider groups, or is
  everyone on the caller's network. Nothing else is checked, and nothing
  differs between fronts or between Pitwall and plain clients.
- **No credentials in the core.** On Tailscale there are none; on the
  Cloud the bearer token is checked inside the backend. The fronts publish
  how plain clients authenticate from `Network::endpoint` ([§15.3](network.md)): A2A
  `securitySchemes` and MCP protected resource metadata, both empty on
  Tailscale.
- **Pitwall callers may add two labels**, plain headers on either front,
  trusted as much as the person whose device sent them:

  ```
  Pitwall-Agent: client-fix@bob-mbp     (the asking agent, from Bob's process tree)
  Pitwall-Chain: c_17,alice/api-expert  (loop check, §8.6)
  ```

  The label only keys the conversation ([§6.1](sessions.md)) and names it
  for the owner. A label can't widen anything: access is the person's.
- **The instance key** (tailnet-peers.md §5) signs team records ([§4](teams.md)),
  and on the Cloud also registers the device; it never signs requests.

### 8.3 Conversations: one core, two fronts
Everything that matters is in **one core** on the owner's Pitwall:
audience checks, conversations, turns, limits, the sandboxed sessions,
reply capture, held results, patches and the audit log. Two thin fronts
speak to it:

| | **A2A front** | **MCP front** |
|---|---|---|
| For | Pitwall callers (the Ask panel, `pitwall ask`, `pitwall mcp`) and any A2A client | Plain Claude Code and any other MCP client |
| Where | `/a2a/<slug>`, one base URL per share | `/mcp`, one endpoint per owner, one tool per share |
| A conversation is | an A2A `contextId` | keyed by caller; `conversation` in results |
| A turn is | an A2A task | one `ask_<slug>` call (plus `check_turn` if it runs long) |
| Long turns, waiting for the owner, held changes | Native task states and SSE; reconnect with `SubscribeToTask` | `wait_seconds`, then `check_turn`; progress notifications |
| Changes | `changes` artifact | `changes` in the result, or `get_changes` in parts |

**The same conversation on both fronts.** Ids are shared: a conversation
id is the A2A `contextId` and the MCP `conversation`; a turn id is the A2A
task id and the MCP `turn`. A person who asks from Pitwall today and from
plain Claude tomorrow continues the same conversation when the caller key
matches ([§6.1](sessions.md)). Limits count across both fronts.

#### 8.3.1 What the agent receives
The shared agent never sees A2A or MCP. Pitwall turns the message into
ordinary input: the caller's text, as written. By default one factual line
goes in front so the agent knows who is talking:

```
From bob@example.com (agent client-fix on bob-mbp):
Where are refresh tokens rotated?
```

(For a plain client: `From erin@example.com (Claude Code over MCP):`, from
the client name in MCP's `initialize`.) No request kinds, no reply
instructions; the owner can turn the "From" line off per share. When
something changed under the agent since its last turn, Pitwall adds one
factual line after it ("Files updated to main @ 3f2a1c", "Your changes
from the previous turn were not sent"). The owner view shows exactly what
was typed.

#### 8.3.2 The A2A front
**Binding: HTTP+JSON** (A2A's REST binding), recommended over JSON-RPC: one
URL per operation is simpler to route, rate-limit and audit in a small
blocking server, it is `curl`-friendly for owners debugging, and streaming
is the same SSE either way. JSON-RPC can be added later as a second
`supportedInterfaces` entry if clients need it.

Per share, base URL `<Network::endpoint>/a2a/<slug>`, e.g.
`http://studio.example.ts.net:47421/a2a/api-expert` on Tailscale:

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
- **`contextId` = the caller's conversation.** The server generates it. A
  message without `contextId` goes to the caller's current conversation
  (created if none) and the response carries its `contextId`. A message
  with one of the caller's own `contextId`s goes there; anyone else's is
  `404`. A new conversation is a message with `metadata:
  {"pitwall/newConversation": true}`, which any A2A client can send.
- **One task per message.** The message's text parts are the caller's
  text, delivered verbatim ([§8.3.1](protocol.md)). File parts with
  `mediaType: text/x-patch` are the "redo on my code" commits ([§7.5](messages.md)).
  Other part types are refused with an error in v1.
- **Task states:**

  | A2A state | When |
  |---|---|
  | `TASK_STATE_SUBMITTED` | Queued; status message "2nd in line" |
  | `TASK_STATE_WORKING` | The turn runs; also while the agent waits for the owner ("waiting for alice") and while a result is held for the owner's OK ("changes waiting for alice's OK") |
  | `TASK_STATE_COMPLETED` | Reply delivered and the changes sent, discarded or not needed |
  | `TASK_STATE_FAILED` | `no_answer`, `timeout`, the agent crashed |
  | `TASK_STATE_CANCELED` | `CancelTask`, or the owner stopped sharing mid-turn |
  | `TASK_STATE_REJECTED` | Refused by a limit: `quota`, `busy`, `paused`, `share_total`, `agent_limit`, `loop` ([§5.8](sharing.md)) |
  | `TASK_STATE_INPUT_REQUIRED`, `TASK_STATE_AUTH_REQUIRED` | Not used |

  **Why the owner's approval is `working`, not an interrupted state.** Both
  interrupted states hand the next step to the **client**: input-required
  means the client continues with a new message on the same task, and
  auth-required delegates the authorization to the client. Here the caller
  can do nothing; the owner decides on their own machine. So the task
  stays `working` with a status message, and the reply artifact is already
  delivered meanwhile.
- **Artifacts:**
  - `reply`: one text part with what the agent said ([§7.2](messages.md)), plus a
    data part `{ "source": "hook"|"session-file"|"non-interactive"|"terminal" }`.
  - `changes` (only at "Can change code"): a file part with
    `mediaType: text/x-patch`, `filename: "turn-7.patch"`, the series as
    `raw` (base64) up to 2 MiB or a `url` part pointing to
    `/pitwall/v1/results/{task}` above that (same identity check); and a
    data part `{ baseCommit, files, added, removed, patches }`.
  - Streaming sends them as artifact update events (`lastChunk: true`).
- **Errors:** not in the audience, unknown share or context → `404`
  (indistinguishable from "no such share"). Limit refusals are a task in
  `rejected` with the reason and, where there is one, the reset time in the
  status message's data part.
- **Not used in v1:** A2A extensions (`A2A-Extensions`); Pitwall's labels
  are the plain headers of [§8.2](protocol.md).

#### 8.3.3 The MCP front
One endpoint per owner and network: `<Network::endpoint>/mcp`,
MCP over **Streamable HTTP** (JSON-RPC over `POST`, answered as JSON or as
an SSE stream). The front serves both the 2025 protocol versions
(`initialize`) and **2026-07-28**, which Claude Code 2.1.301 speaks
(`server/discover`, `subscriptions/listen`, closing a call's stream
cancels it); `tools/list` must carry `ttlMs` and `cacheScope` or Claude
Code drops every tool (found in C1a interop testing). A plain Claude Code
user adds it once:

```
claude mcp add --transport http alice http://studio.example.ts.net:47421/mcp   # Tailscale
claude mcp add --transport http alice https://studio.acme.pitwall.cloud/mcp    # Pitwall Cloud: signs in via OAuth
```

**Server capabilities:** `tools` with `listChanged: true`. No resources,
prompts or sampling in v1.

**`tools/list` is the plain client's directory.** It returns, for this
caller only, one tool per live share they may use, plus two helpers. A
caller outside every audience gets an empty list. When a share goes live,
pauses, or its audience changes, connected clients get
`notifications/tools/list_changed`.

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
- **Changes are a patch series** (`git format-patch` output). The caller's
  agent applies it with `git am -3` or `git apply`, as it would any patch.
- **Refusals** are tool results with `isError: true` and a reason in
  `structuredContent.refused` (the same reasons as A2A's `rejected`), with
  `resetsAt` or `until` where known. The caller's agent sees a plain
  sentence ("Daily limit reached; resets at 09:00") and decides what to do.
- **Cancel.** `notifications/cancelled` for a running `ask` or `check_turn`
  stops the turn (as Stop) when the turn belongs to that request; a queued
  message is dropped.
- **Not in the audience** and **unknown tool** look the same: the tool
  isn't listed, and calling it is a JSON-RPC error "unknown tool".
- **MCP sessions** (`Mcp-Session-Id`) are transport only. Conversations are
  keyed by the caller ([§6.1](sessions.md)), so a new MCP session or a
  restarted client continues the same conversation.

### 8.4 Pitwall endpoints beside the fronts
Teams, discovery and the directory are small JSON endpoints on the same
handler, under `/pitwall/v1/`. Identity is the `Caller`, as for the
fronts; team records carry their own signatures ([§4](teams.md)). Plain Claude users never need them.

| Endpoint | Caller | What |
|---|---|---|
| `GET /info` | any caller on the network | `{version, person, label, listedTeams, a2a: true, mcp: "/mcp"}`: the discovery probe ([§3](teams.md)) |
| `GET /teams:discover` | any caller on the network | listed teams this Pitwall knows, as signed records |
| `POST /teams/{t}/join`, `POST /teams/{t}/invites:redeem` | anyone with Pitwall | join request, invite ([§4.6](teams.md)) |
| `POST /teams/{t}/heads`, `GET /teams/{t}/records?after=n`, `POST /teams/{t}/announce`, `POST /teams/{t}/cosign` | member devices | replication and conflicts ([§4.3](teams.md)–4.4) |
| `GET /teams/{t}/requests` | admins | pending join requests, forwarded |
| `GET /directory` | anyone in some share's audience | the Agent Card URLs and cards this caller may use (A2A allows catalogs and pre-configured card URLs besides the well-known path; one host here serves many agents, so the root `/.well-known/agent-card.json` is not used) |
| `GET /sessions`, `DELETE /sessions/{conversation}` | audience | the caller's own conversations with this owner: list, forget |
| `GET /results/{task}` | the task's caller | a large patch series |
| `GET /events` (SSE) | member devices | `team.announce`, `directory.changed` |

### 8.5 Local sockets and who may call what

| Caller (how the daemon knows) | Reaches | May call |
|---|---|---|
| **Owner UI** (in-process GPUI app) | — | Everything, including all `share.*` methods |
| **Owner's CLI and agents** (CLI socket; agent vs user from the process tree, as approvals do today) | `pitwalld.sock` | `share.*` reads directly; `share.create` and `share.guest` on drafts directly (nobody else can see a draft); other `share.*` changes through approvals (`share.go_live` and widening always ask, not rememberable; narrowing direct, [§5.5](sharing.md)); `suggest.*`; `ask.*` (the daemon speaks A2A outward) with the "let my agents ask" switch |
| **A shared session** (inside its sandbox; process tree as a second check) | the hook endpoint, as any agent | Hook events for its own agent id (status, the Stop text). Nothing on `pitwalld.sock` |
| **Anyone on an enabled network** | the collaboration handler, through that backend | The A2A and MCP fronts ([§8.3](protocol.md)) and the Pitwall endpoints ([§8.4](protocol.md)) |

Shared sessions are cut off from Pitwall by construction: the sandbox
denies `pitwalld.sock` and the data folder ([§6.3](sessions.md)); their
environment has no `PITWALL_*` variables and no `pitwall` on `PATH`; and the
daemon still rejects any connection whose peer process descends from a
shared session's holder (`Caller::Shared` → `forbidden`). The hook endpoint
already accepts only hook events, keyed to the agent id Pitwall gave the
session.

**Owner-only local methods** (UI, or CLI/agents through approvals; never on
any network, never from shared sessions): `share.get`,
`share.list`, `share.create` (draft), `share.set` (returns the diff it
applied), `share.go_live`, `share.pause`, `share.resume`, `share.stop`,
`share.guest`, `share.sessions`, `share.block`, `share.summary`,
`result.send`, `result.discard`, `team.*` changes (create, approve, invite,
add member, promote, remove, bind), `collab.settings.*`. Typing into or
prompting a shared session uses the ordinary agent methods (`term.*`,
`agent.prompt`), owner only.

### 8.6 CLI (asking is agent-neutral)
Pitwall users and their agents ask through the local daemon, which speaks
A2A to the owner's Pitwall with the `Pitwall-Agent` label. `pitwall ask`
is an ordinary shell command, so **any agent kind** that can run a command
can ask; `pitwall mcp` ([§10.2](protocol.md)) serves agents that prefer MCP tools.

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
  argument. This switch governs Pitwall's own `ask`; a plain MCP or A2A
  client the user configured themselves is the user's choice.
- **Loops.** `Pitwall-Chain` carries the turns and shares a message came
  from. Depth limit 2; a chain containing the target share is refused with
  `loop`. Shared sessions can't ask anyone (no CLI, no network beyond their
  model), so chains only grow through people's ordinary agents. A plain
  client sends no chain and a modified one can lie; limits are the real
  bound ([§5.8](sharing.md)).

### 8.7 Data shapes
A share's Agent Card (A2A objects use the spec's names; field spelling
checked with golden fixtures against the spec's schema at implementation):

```json
{
  "name": "API expert",
  "description": "Questions about the API service: auth, billing, jobs",
  "version": "3",
  "supportedInterfaces": [
    {"url": "http://studio.example.ts.net:47421/a2a/api-expert",
     "protocolBinding": "HTTP+JSON", "protocolVersion": "1.0"}
  ],
  "defaultInputModes": ["text/plain", "text/x-patch"],
  "defaultOutputModes": ["text/plain", "text/x-patch"],
  "skills": [{"id": "ask", "name": "Ask", "tags": ["code", "api"],
              "description": "Send any message; replies, and patches when it changed code"}],
  "capabilities": {"streaming": true, "pushNotifications": false,
    "extensions": [{"uri": "<pitwall-domain>/a2a/ext/share/v1", "required": false,
      "params": {"owner": "alice@example.com", "slug": "api-expert",
                 "capability": "read", "kind": "claude", "messagesPerDay": 20, "left": 17}}]}
}
```

The same share as an MCP tool:

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
  "outputSchema": { "…": "the result shape in §8.3.3" },
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
interface DirectoryEntry { cardUrl: string; card: object /* A2A AgentCard */;
  owner: string; slug: string; capability: "read"|"write"|"write_approval";
  kind: string; left: number|null; online: boolean; lastSeen: string|null;
  via: string[] /* team ids, groups, "direct" or a network id */ }
interface AskView { turn: string; conversation: string; share: string; text: string;
  from: "me"|"owner";
  status: "queued"|"working"|"waiting_for_owner"|"done"|"failed"|"canceled"|"refused";
  statusMessage: string|null; reply: string|null; replySource: string|null;
  changes: { state: "held"|"sent"|"discarded"; files: number; added: number; removed: number;
             baseCommit: string; patches: number; path: string|null } | null;
  at: string; doneAt: string|null }
interface SharedSessionView { conversation: string; share: string;
  caller: { login: string; agent: string|null; client: string|null; front: "a2a"|"mcp";
            guest: boolean; pitwall: boolean };
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
  directory.json             teammates' Agent Cards, cached
  teams/<team>/chain.jsonl   signed records
  teams/<team>/invites.json  admin only (hashes replicate)
  shares/<share>.json        share settings (owner)
  shares/<share>/base/       instructions, approved summary, PRECEDENTS.md
  shares/<share>/s/<conv>/   work/ (snapshot repo), home/, tmp/, sandbox.sb
  usage.jsonl                owner side: per share, per caller, per day
  asks.jsonl                 my messages and replies (caller)
  results/<turn>/            received patch series (caller)
  audit.jsonl                owner side: every turn (with its front), result decision, share change (with diff), join, refusal
  suggestions.json           open and dismissed suggestions
```

The audit log uses tailnet-peers.md §5's format with `kind:"collab"`,
rotates at 1 MiB × 3, and is shown in the owner view and Settings.

---

## 10. Two fronts: A2A for Pitwall, MCP for everyone else

### 10.1 Why both
- **A2A fits agent-to-agent conversations.** Tasks with states (queued,
  working, done), long turns, a turn waiting on the owner, held changes,
  cancel and re-attaching to a stream are all native; a conversation is a
  `contextId`. Pitwall talks to Pitwall over A2A, and any other A2A client
  on the same network can use a share with nothing Pitwall-specific.
- **MCP is what plain Claude Code has.** Claude Code (and most agent tools)
  have no built-in A2A client but connect to MCP over HTTP with one
  command, and a shared agent then shows up as a tool its agent uses like
  any other. Long turns fit a tool call less well, so the MCP front adds
  `wait_seconds` and `check_turn` ([§8.3.3](protocol.md)).
- **One core keeps it one system.** Audience, limits, conversations,
  sandboxing, results and the audit log exist once; the fronts only
  translate. A turn looks the same in the owner's view whichever front it
  came from (with a small "A2A" or "MCP" tag).
- **Identity stays the network's** on both fronts ([§15](network.md)): no
  credentials on Tailscale, the Cloud's OAuth on the Cloud. No OAuth
  server on every owner's laptop.
- **What neither covers stays Pitwall's,** beside them on the same
  listener: teams, discovery, gossip of signed records, the directory
  ([§8.4](protocol.md)).
- **Cost.** An HTTP server and client, two thin protocol layers (A2A
  HTTP+JSON with SSE; MCP Streamable HTTP), and following both specs'
  versions. About 1–1.5 weeks more than one front. The agents never see
  any of it.
- **Peers stay framed TCP** (tailnet-peers.md): terminals and screen frames
  aren't conversations.

### 10.2 `pitwall mcp`: one local server for Pitwall users' agents
A local stdio MCP server on the caller's machine that merges every
teammate's shared agents into one tool list (`ask_alice_api_expert`, …)
plus `list_shared_agents`, and forwards calls through the daemon, which
speaks A2A with the `Pitwall-Agent` label. So the "let my agents ask"
switch, budgets and loop checks apply, and agents don't need one setup per
teammate. It calls owners directly; it never relays for anyone else.
