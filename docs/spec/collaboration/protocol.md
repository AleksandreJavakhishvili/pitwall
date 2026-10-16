# Collaboration: protocol, A2A and API

Part of the [collaboration spec](README.md).

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
  certificate is an option for clients that insist ([§14](plan.md) question 9).
- **Server.** A small blocking HTTP/1.1 server on its own threads, no async
  runtime: `tiny_http` is the candidate (a request per worker thread;
  streaming responses use chunked transfer from a `Read` fed by a channel),
  so SSE is a response whose body is written event by event. Client side:
  `ureq` (blocking, reads the SSE body line by line). Both to be confirmed in
  a C1a spike and recorded in gpui/licensing.md.
- **Limits** as peers: per caller connections, requests per second, body
  size (2 MiB for messages; larger patch artifacts are fetched by URL,
  [§8.3](protocol.md)), streams at most 8 per caller.

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
  Pitwall-Chain:  r_17,x_api-expert        (optional: loop check, [§8.6](protocol.md))
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
- The same headers sign the Pitwall endpoints ([§8.4](protocol.md)), which require them
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
  the owner or they `DELETE /pitwall/v1/sessions/{contextId}` ([§8.4](protocol.md)).
- **One task per message.** The message's text parts are the asker's text,
  delivered verbatim ([§8.3.1](protocol.md)). File parts with `mediaType: text/x-patch`
  are the "redo on my code" commits ([§7.5](messages.md)). Other part types are refused
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
  - `reply`: one text part with what the agent said ([§7.2](messages.md)), plus a data part
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
headers ([§8.2](protocol.md)). They keep the signed-records design of [§4](teams.md).

| Endpoint | Caller | What |
|---|---|---|
| `GET /info` | any tailnet caller (`whois` only) | `{version, login, label, listedTeams, a2a: true}`: the discovery probe ([§3](teams.md)) |
| `GET /teams:discover` | any tailnet caller | listed teams this Pitwall knows, as signed records |
| `POST /teams/{t}/join`, `POST /teams/{t}/invites:redeem` | Pitwall callers | join request, invite ([§4.6](teams.md)) |
| `POST /teams/{t}/heads`, `GET /teams/{t}/records?after=n`, `POST /teams/{t}/announce`, `POST /teams/{t}/cosign` | members | replication and conflicts ([§4.3](teams.md)–4.4) |
| `GET /teams/{t}/requests` | admins | pending join requests, forwarded |
| `GET /directory` | anyone in some share's audience | the Agent Card URLs and cards this caller may use (A2A allows catalogs and pre-configured card URLs besides the well-known path; one host here serves many agents, so the root `/.well-known/agent-card.json` is not used) |
| `GET /sessions`, `DELETE /sessions/{contextId}` | audience | the caller's own sessions with this owner: list, forget |
| `GET /results/{task}` | the task's asker | a large patch series |
| `POST /lists/{list}/sync` | members of the list's team | exchange version vectors, return missing ops ([§15.3](tasks.md)) |
| `POST /lists/{list}/announce` | members | "I have new ops": the receiver syncs |
| `GET /lists/{list}/blobs/{sha256}` | members | a result patch attached to an item ([§15.6](tasks.md)) |
| `GET /events` (SSE) | members | `directory.changed`, `team.announce`, `list.changed` |

### 8.5 Local sockets and who may call what

| Caller (how the daemon knows) | Reaches | May call |
|---|---|---|
| **Owner UI** (in-process GPUI app) | — | Everything, including all `share.*` methods |
| **Owner's CLI and agents** (CLI socket; agent vs user from the process tree, as approvals do today) | `pitwalld.sock` | `share.*` reads directly; `share.create` and `share.guest` on drafts directly (nobody else can see a draft); other `share.*` changes through approvals (`share.go_live` and widening always ask, not rememberable; narrowing direct, [§5.5](sharing.md)); `suggest.*`; `ask.*` (the daemon sends it out over A2A) with the "let my agents ask" switch |
| **A shared session** (inside its sandbox; process tree as a second check) | the hook endpoint only, as any agent | Hook events for its own agent id (status, the Stop text). Nothing on `pitwalld.sock` |
| **Teammates and A2A clients** | the tailnet HTTP listener | A2A ([§8.3](protocol.md)) and Pitwall endpoints ([§8.4](protocol.md)) |

Shared sessions are cut off from Pitwall by construction: the sandbox
denies `pitwalld.sock` and the data folder ([§6.3](sessions.md)); their environment has no
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
run a command can ask, and `pitwall mcp` ([§10.2](protocol.md)) serves agents that prefer
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
pitwall share create <agent> --team platform --capability read \
    --paths src/,docs/ [--exclude …] [--start fresh|summary] [--programs allow:git,rg,cargo] \
    [--messages-per-day 20] [--instructions -]       # a draft; prints the summary and excluded files
pitwall share guest <agent> --probes                 # runs the guest probes on the draft; prints results
pitwall share set <agent> --add-team payments --capability write_approval   # prints the diff; widening asks
pitwall share set <agent> --remove-person bob@example.com                   # narrowing: direct
pitwall share go-live <agent> | pause <agent> | stop <agent>
pitwall share sessions <agent>                        # per asker: state, usage
pitwall suggest share <agent> --reason "…"            # a card for the owner, nothing more
pitwall suggest skill <skill> --agent <agent> --reason "…"

# Tasks (§15): pitwall task add|list|show|take|release|done|drop|comment|tags|give|watch
```

The task commands are listed in [§15.4](tasks.md).

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

Task list shapes (`TaskItem`, `TaskListView`, `WatchRule`) are in
[§15.10](tasks.md).

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
  lists/                     task lists: op logs, result blobs, watch rules (§15.10)
```

The audit log uses tailnet-peers.md §5's format with `kind:"collab"`,
rotates at 1 MiB × 3, and is shown in the owner view and Settings.

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
  turn ([§8.3](protocol.md)).
- **What A2A doesn't cover stays Pitwall's,** beside it on the same
  listener: tailnet identity before HTTP, the device-key extension, teams,
  discovery, gossip of signed records ([§8.4](protocol.md)).
- **Cost.** An HTTP server and client in Pitwall (blocking, small, [§8.1](protocol.md)),
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
