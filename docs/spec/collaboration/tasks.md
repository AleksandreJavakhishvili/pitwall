# Collaboration: tagged task lists

Part of the [collaboration spec](README.md).

## 15. Tagged task lists

A **task list** is a set of items with tags: "rotate the staging keys"
`#infra #security`, "flaky test in jobs::retry" `#jobs #ci`. People and
agents add items, take them, finish them, and when the one who finished it
changed code, the patch comes with the item. There is **my list** (local)
and **one list per team**, replicated among the team's members like team
records: no server. Pitwall never decides who does what: a person takes an
item, or gives it to an agent, or sets a rule that gives it.

### 15.1 Items

```
t_7k2m9q  Flaky test in jobs::retry                    #jobs #ci
          added by bob@example.com (agent client-fix on bob-mbp) · team Platform
          status: taken by alice/api-expert (given by carol@example.com)
          body:   fails 1 in 20 on CI since 3f2a1c; log attached below …
          result: 2 files +14 −3 · patch series · "made retry wait on the clock fake"
          history: added · tagged #ci · taken · comment · done
```

| Field | What |
|---|---|
| `id` | `t_` + the first 8 base32 characters of the hash of the item's `add` op (longer if two collide in a list) |
| `title`, `body` | Plain text, as written. Title ≤ 200 chars, body ≤ 8 KiB |
| `tags` | Lowercase words (`a-z0-9-`, ≤ 32 chars), up to 10 per item |
| `addedBy` | Who added it, from the op's signature: the signing device's key, mapped to a login by the team records (ops may be relayed by any member, so authorship comes from the signature; `whois` only identifies the member delivering them), and the agent label when an agent added it (from the process tree on the adding machine, [§15.4](tasks.md)). Never from an argument |
| `status` | `open → taken → done`, or `dropped` from `open` or `taken`; `release` returns `taken → open` |
| `takenBy` | A person, one of that person's own agents (`agent:<name>@<device>`), or a shared agent (`owner/slug`, with who gave it) |
| `notes` | Comments, in order |
| `result` | On `done`: a note, and when the one who finished it changed code, the patch series ([§15.6](tasks.md)) |

### 15.2 Lists
- **My list** (`me`): local to this Pitwall, not replicated. Same op format,
  signed with the instance key, so a later sync between one person's own
  devices is just replication ([§15.11](tasks.md) question 2).
- **Team lists** (`team:<team>`): one per team, created with the team. Every
  member device keeps the full log and merges ops from any member, like
  team records ([§4.3](teams.md)). A team can turn its list off (admin).
- **Remote add to someone's own list: not in v1.** A personal list writable
  by others is an inbox: it needs an audience, per-sender limits and spam
  handling, which is a second sharing model. Team lists already cover work
  for a group, and asking a shared agent covers a directed request to one
  person's agent. An explicit per-person **inbox** list with an audience
  like a share's is [§15.11](tasks.md) question 1.

### 15.3 Replication: a log of signed ops
Each list is an **append-only set of ops**. State is never sent, only ops;
every member folds the same ops the same way.

```json
{
  "list": "team:t_9f2c…",
  "op": "take",                        // add | edit | tag | take | release | done | drop | comment | fence | checkpoint
  "item": "t_7k2m9q",                  // absent on add (the item id is derived from it)
  "fields": {"takenBy": {"shared": "alice/api-expert", "givenBy": "carol@example.com"}},
  "by": {"login": "carol@example.com", "device": "ed25519:C1…", "agent": null},
  "seq": 41,                           // per device, 1, 2, 3 … no gaps
  "lamport": 1287,                     // max seen in this list + 1
  "teamVersion": 9,                    // the team record version the signer had
  "at": "2026-10-16T09:40:00Z",        // informative only
  "sig": "ed25519:…"                   // over the canonical JSON (RFC 8785) of the rest
}
```

- **Op id** = SHA-256 of the signed op.
- **Sync** uses a version vector `{device: highest seq}`: on any call
  between two members' Pitwalls (and on `announce`), each side sends its
  vector, the other returns the ops it is missing, per device in `seq`
  order. A gap (seq 40 missing, 41 present) is fetched before 41 is applied.
- **Order.** All valid ops are sorted by `(lamport, op id)`. That order is
  the same on every member; state is the fold of ops in that order.
- **Fold rules** (an op that doesn't apply in the current state is kept in
  the log, marked "lost", and shown in the item's history):
  - `take` on an item that is not `open` is lost. **Two takes at once:**
    the one first in `(lamport, op id)` order wins; the other member's
    Pitwall sees its take marked lost and tells them ("bob took it first").
  - `release` and `done` apply only when the item is `taken` and the op is
    from the taker (or, for `release`, an admin).
  - `edit` and `tag` are last-writer-wins per field in fold order; `tag`
    carries `add`/`remove` sets, so concurrent tag changes merge.
  - `drop` applies from `open` or `taken`; later ops on a dropped item are
    lost except `comment`.
  - `comment` always applies.
- **Validity, checked on arrival:** signature; the device belongs to a
  member in the team state the receiver holds, or in the state at
  `teamVersion` if that is older (ops made before someone's removal stay
  valid); size limits; permissions ([§15.5](tasks.md)).
- **Removed members.** When an admin removes a member, the admin's Pitwall
  also writes a **`fence`** op to the team list: the removed devices and,
  for each, the highest `seq` it has seen from them. Every member then
  rejects that device's ops with a higher `seq`, whatever `teamVersion`
  they claim. Ops at or below the fence that some member hasn't seen yet are
  still accepted when they arrive, so all members end with the same set.
- **Limits.** Op ≤ 16 KiB; ≤ 5,000 items not done or dropped per list;
  per device ≤ 500 ops per day accepted by each receiver (a flood guard at
  the transport, not part of the fold). Patch blobs are separate ([§15.6](tasks.md)).
- **Retention.** Done and dropped items are hidden after 90 days by default.
  The log is kept until a list passes 10 MB or 50,000 ops; then an admin's
  Pitwall writes a signed **`checkpoint`**: the folded state and the version
  vector it covers. Members verify it by folding themselves, keep it, and
  drop the covered ops (and their blobs, unless pinned).

### 15.4 Agents use it with ordinary commands
Any agent kind that can run a command can use tasks; nothing is special to
agents:

```bash
pitwall task list [--list me|team:platform] [--tag jobs --tag ci] [--status open|taken|done|dropped] [--mine]
pitwall task show t_7k2m9q                         # fields, history, result
pitwall task add "Flaky test in jobs::retry" --tag jobs --tag ci --list team:platform [--body - < log.txt]
pitwall task take t_7k2m9q                         # take it for yourself (the caller)
pitwall task release t_7k2m9q
pitwall task done t_7k2m9q [--note "…"] [--attach-changes]   # changes since you took it, as a patch series
pitwall task drop t_7k2m9q [--note "…"]
pitwall task comment t_7k2m9q "seen again on main"
pitwall task tags [--list …]                       # tags in use, with counts
# Owner side
pitwall task give t_7k2m9q --to api                # own agent: into its Next up
pitwall task give t_7k2m9q --to alice/api-expert   # a shared agent: an A2A message
pitwall task watch add --list team:platform --tag ci --to api [--per-day 5]
pitwall task watch list | remove <rule>
```

- **Who is "you".** The caller's identity comes from the process tree, as
  for approvals: the user in their own terminal, or `agent:<name>` inside a
  Pitwall pane. `take` from an agent records that agent as the taker.
- **Agents need the owner's switch.** Settings → Collaboration → "Let my
  agents add and take tasks" (off by default; per project override). With
  it on: per agent, 20 `add` and 10 `take` per hour; `list`, `show` and
  `tags` are always allowed. `give` and `watch` from an agent go through
  the owner's approval, like other settings changes.
- **Shared sessions can't use tasks:** they have no CLI ([§8.5](protocol.md)).
- **MCP:** `pitwall mcp` ([§10.2](protocol.md)) adds tools `task_list`, `task_show`,
  `task_add`, `task_take`, `task_done`, `task_comment` over the same local
  methods and switches.
- `--attach-changes` uses what Review already knows: the agent's changes
  since the `take` (its worktree diff against the commit it was at), packaged
  as a patch series ([§15.6](tasks.md)). For a person it asks which agent's changes.

### 15.5 Who may do what

| Op | Who |
|---|---|
| `add`, `comment` | Any member (and their agents, with the switch) |
| `take` | Any member, for themselves or one of their own agents; a member giving to a shared agent they may use |
| `release` | The taker, or a team admin |
| `done` | The taker (for a shared agent, the member who gave it) |
| `edit`, `tag`, `drop` | The item's author (the person, whichever of their devices or agents), or a team admin |
| `fence`, `checkpoint` | Team admins (written by their Pitwall) |

Agent-made ops carry the agent label; other members see "added by
bob@example.com's agent client-fix" and can filter on it. Members can't
verify Bob's switch; they trust Bob's Pitwall to label honestly, as with
asking ([§6.1](sessions.md)).

### 15.6 Giving an item to an agent, and results
Only a person (or a rule they set, [§15.7](tasks.md)) gives work to an agent:
- **To one of my own agents:** the item goes into that agent's **Next up**
  as an ordinary queued prompt, sent when the agent is idle like any other
  Next-up item. The text is the title and body as written, after one
  factual line: `Task t_7k2m9q from team Platform (added by
  bob@example.com):`. A `take` op records the agent as taker.
- **To a shared agent** (`owner/slug`, one the giver may use): the same text
  goes as a normal A2A message in the giver's conversation with it
  ([§7.1](messages.md)). The `take` op records `{shared, givenBy}`. When the A2A task
  completes, the giver's Pitwall attaches the reply as the result note, and
  any `changes` artifact as the result patch, and asks the giver "Mark
  done?" (one click, or automatic if the giver ticked "done when it
  replies").
- **Results with code.** A patch series in the same format as A2A `changes`
  results (`git format-patch --base=…`, [§7.5](messages.md)). The `done` op carries
  `{note, patch: {sha256, size, files, added, removed, baseCommit}}`; the
  blob itself is fetched by hash from any member who has it
  (`/pitwall/v1/lists/{list}/blobs/{sha256}`), up to 5 MiB, kept while the
  item is visible. Anyone in the team opens it in Review and Apply runs
  `git am -3` into a worktree they pick, as for any result.
- **Nothing runs by itself.** An item is text until someone takes or gives
  it. Done is marked by the taker (or the giver for shared agents), never
  guessed from an agent's words.

### 15.7 Watch rules (opt-in)
- A **watch rule** is the owner's: "send new items tagged `ci` in team
  Platform's list to agent `api`'s Next up". Made in the Tasks screen or
  with `pitwall task watch add` (approval when an agent asks).
- When a new item matches, the owner's Pitwall gives it to that agent
  ([§15.6](tasks.md)), at most `perDay` items per rule (default 5), only items added
  after the rule was made, never items added by that same agent (no loops).
- A rule may target one of the owner's own agents, or a shared agent the
  owner may use. Never anyone else's agent: a teammate's agent only gets
  work through that teammate's share, with its limits.
- **Human watches:** the same tag filter without an agent just raises a
  badge and a quiet notification for new matching items.

### 15.8 Filtering by tag
Tag filters work across all lists at once: `pitwall task list --tag ci`
lists matching items from my list and every team list; the Tasks screen
shows tag chips with counts, combinable (AND), plus list, status and
"added by an agent" filters.

### 15.9 Safety
- Items, bodies and comments are **untrusted text**, possibly written by
  someone else's agent. They are shown, never executed; they reach an agent
  only when a person takes or gives them (or the person's own rule does),
  as an ordinary prompt the agent treats like any other.
- Agent-added items are labelled as such everywhere, and watch rules can
  exclude them ("only items added by people").
- Rate limits on adding and taking (per agent, per device at receivers),
  size limits, and the switch for agents.
- A patch attached to an item is applied only by a person's click, into a
  worktree they pick.

### 15.10 Data shapes and storage

```ts
interface TaskItem { id: string; list: string; title: string; body: string; tags: string[];
  addedBy: { login: string; device: string; agent: string|null };
  status: "open"|"taken"|"done"|"dropped";
  takenBy: { login: string; agent: string|null } | { shared: string; givenBy: string } | null;
  notes: { by: string; text: string; at: string }[];
  result: { note: string|null; patch: { sha256: string; size: number; files: number;
            added: number; removed: number; baseCommit: string; local: string|null } | null } | null;
  history: { op: string; by: string; at: string; lost: boolean }[]; updatedAt: string }
interface TaskListView { list: string /* "me" | "team:<id>" */; name: string; open: number;
  taken: number; tags: { tag: string; count: number }[]; syncedAt: string|null }
interface WatchRule { id: string; list: string; tags: string[]; to: string|null /* agent, owner/slug, or null for a badge */;
  perDay: number; peopleOnly: boolean }
```

```
<PITWALL_HOME>/collab/lists/
  me/ops.jsonl                 my list
  <team>/ops.jsonl             a team list's op log (+ checkpoint.json)
  <team>/blobs/<sha256>.patch  result patches
  watch.json                   watch rules
```

### 15.11 Open questions (tasks)
1. **Inbox.** Add a per-person inbox list that others may add to, with an
   audience and per-sender limits like a share?
2. **My list across my devices.** Replicate `me` between one person's own
   devices (paired peers, or devices of the same login in a team)?
3. **Assignment without taking.** Let someone suggest a taker ("for
   alice") without taking on their behalf? Proposed: only as a tag
   (`for-alice`), so nothing is decided for anyone.
