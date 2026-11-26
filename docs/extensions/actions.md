# Actions: showing, focusing, queueing, starting

An extension that only reads stays out of the way. Four
[permissions](permissions.md) let it act in Pitwall:

| Permission | What it may do | Method |
|---|---|---|
| `show` | Open a file (at a line) or a web page beside an agent | `viewer.open`, `web.open` |
| `agents.focus` | Focus an agent's tile, only while Pitwall is in front | `agent.focus` |
| `agents.queue` | Put text in an agent's Next up, marked with its name | `queue.add` |
| `agents.start` | Ask the user to start an agent, every time | `agent.create` |

Each starts **off** on the permission screen. The two that reach an
agent carry a risk note there, in plain words. Without the grant, a call
is refused with `denied` ("<name> isn't granted agents.queue"). Every
call, refused or not, is in the extension's Activity with what it asked
for (which agent, the text, the project).

None of them changes what an agent may do. An agent's own permissions,
its tools and its rules stay as they are. Pitwall checks the grant in one
place, its method table. The adapter's stdio, a program the adapter
started on the socket and a button in a view all go through it. An
extension can never send what it queued, answer an approval, grant
itself anything or change Pitwall's settings.

## `show`

```json
{"type":"call","id":"c1","method":"viewer.open","params":{"agentId":"a1","files":["src/main.rs:42"],"proto":2}}
{"type":"call","id":"c2","method":"web.open","params":{"agentId":"a1","url":"http://localhost:3000"}}
```

The same paths as an agent's own `show_files` and `show_web`: the file
opens in that agent's file viewer, the page in its web tile. `agentId`
is required: an extension has no agent of its own. Pitwall never takes
the keyboard and never brings itself forward for it. The user's
settings decide as for an agent: with "Agents can show you files" off,
nothing opens and the call says why. An extension can't put it in a
window of its own or replace the user's layout (`place` `window` or
`replace` is refused).

## `agents.focus`

```json
{"type":"call","id":"c3","method":"agent.focus","params":{"agentId":"a1"}}
```

Selects the agent's tile in the window that shows it, only while
Pitwall is the app in front. Otherwise nothing changes and the answer
says why:

```json
{"focused": false, "note": "Pitwall isn't the app in front: nothing was focused (an extension never brings it forward)."}
```

## `agents.queue`

```json
{"type":"call","id":"c4","method":"queue.add","params":{"agentId":"a1","text":"Re-run the failing test and fix it"}}
```

The text goes to the end of the agent's **Next up** (the "N queued" chip
on its tile opens it), drawn in its own colour with "From <name>". It
waits there until the user presses Send.

- **Auto-send skips it.** The user can turn on "Send what <name> queues
  without asking" in its card, under the permission. It starts off and
  is shown with a warning. Then Auto-send sends it like the user's own.
- **Bounded**: at most 4000 characters (trimmed; empty is refused), one
  item per agent every 10 seconds, three of its items waiting per agent.
  Past those the call is refused with `busy` or `bad_params`.
- **Only Pitwall's own agents**, by id (`agent.list`).
- The user removes it like any other item. Turning the extension off, or
  its `agents.queue` grant, removes its items that are still waiting.
- **Logged**: Activity records which agent and the text. The agent's
  "last sent" line says "from <name>" once it is sent.

`queue.send` and `queue.remove` are never an extension's.

## `agents.start`

```json
{"type":"call","id":"c5","method":"agent.create","params":{
  "kind":"claude","project":"/Users/me/code/api","name":"fix-ci",
  "prompt":"The CI run #812 failed in tests/auth.rs. Find out why and fix it."}}
```

Every request is an approval the user answers. It is never remembered.
It shows:

> **<name>** wants to start a Claude agent in api.
>
> - Asked by the extension “<name>”, not by you.
> - Project: /Users/me/code/api
> - Tool: Claude
> - Name: fix-ci
> - Its first prompt, put in its Next up marked “<name>” (sent only when
>   you press Send, unless you let <name> send without asking):
>   *the prompt in full*
> - It runs as you, in this project's folder, with the project's
>   accounts and its own permissions there.

- **Denied** (`denied`) when the user says no; **`approval_timeout`** when
  nobody answers in time. Nothing starts.
- **On this computer, in one of Pitwall's projects** (one in the
  project list, or a folder one of Pitwall's agents works in). `provider`,
  `machine`, `options`, `account` and `resume` are refused. Any other
  folder is `not_found`.
- **One request at a time, three in ten minutes** (a timed-out one
  counts). Past that, `busy`.
- **The prompt** (at most 4000 characters) isn't typed into the agent. It
  is put in its Next up as the extension's item and follows the rules
  above.

The answer is the new agent, as `agent.list` gives it.

## In a view

A [view](views.md)'s buttons and its items can use the same actions
without the adapter calling anything. Pitwall runs them itself when they
are pressed, as the extension, through the same check:

```json
{"id": "r812", "title": "api: tests failed", "agent": {"agentId": "a1"},
 "actions": [
   {"id": "src", "label": "Open test", "open": {"file": "tests/auth.rs", "line": 88}},
   {"id": "ci", "label": "Open run", "open": {"url": "https://ci.example.com/runs/812"}},
   {"id": "jump", "label": "Go to agent", "focus": "a1"},
   {"id": "fix", "label": "Ask it to fix", "needs": "agents.queue"}
 ]}
```

- `"agent": {"agentId"}` links an item to an agent. The row shows the
  agent's name. With `agents.focus` granted, clicking it focuses the
  agent.
- `"open": {"file", "line", "agentId"}` or `"open": {"url", "agentId"}`
  needs `show`. Without an `agentId` it opens beside the item's agent.
- `"focus": "<agent id>"` needs `agents.focus`.
- `"needs": "<permission>"` marks a button the adapter acts on with a
  permission (here, queueing). The adapter is told as usual.

A button whose permission isn't granted is drawn disabled, and says
which permission it needs. Pressed anyway (from a key, say), it is
refused and logged. The adapter isn't told about the buttons Pitwall
runs.

## Checking it

`pitwall extension check` describes each of these permissions and warns
about `agents.queue` and `agents.start`. A view fixture
([`views`](checks.md#views)) lists the buttons Pitwall runs and the
permission each needs. In `calls`, the fake Pitwall refuses an action
method without its grant, the same as Pitwall does.
