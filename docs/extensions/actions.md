# Actions: showing, focusing, starting

An extension that only reads stays out of the way. Three
[permissions](permissions.md) let it act in Pitwall:

| Permission | What it may do | Method |
|---|---|---|
| `show` | Open a file (at a line) or a web page beside an agent | `viewer.open`, `web.open` |
| `agents.focus` | Focus an agent's tile, only while Pitwall is in front | `agent.focus` |
| `agents.start` | Ask the user to start an agent, every time | `agent.create` |

Each starts **off** on the permission screen. `agents.start` carries a
risk note there, in plain words. Without the grant, a call is refused
with `denied` ("<name> isn't granted agents.start"). Every call, refused
or not, is in the extension's Activity with what it asked for (which
agent, the project, the prompt).

None of them changes what an agent may do. An agent's own permissions,
its tools and its rules stay as they are. Pitwall checks the grant in one
place, its method table. The adapter's stdio, a program the adapter
started on the socket and a button in a view all go through it. An
extension can never type into an agent, answer an approval, grant
itself anything or change Pitwall's settings.

`agents.queue` (putting text in an agent's Next up, `queue.add`) is
gone with Next up. A manifest that still asks for it installs; the
permission screen and `pitwall extension check` show it as removed, and
it is never granted. `queue.add` is an unknown method.

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
> - Its first prompt, sent once it is ready, as if you typed it:
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
- **The prompt** (at most 4000 characters) is kept with the new agent
  and typed into it once, verbatim, when it is first free: the user saw
  it in full when they allowed the start.

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
   {"id": "rerun", "label": "Re-run", "needs": "agents.read"}
 ]}
```

- `"agent": {"agentId"}` links an item to an agent. The row shows the
  agent's name. With `agents.focus` granted, clicking it focuses the
  agent.
- `"open": {"file", "line", "agentId"}` or `"open": {"url", "agentId"}`
  needs `show`. Without an `agentId` it opens beside the item's agent.
- `"focus": "<agent id>"` needs `agents.focus`.
- `"needs": "<permission>"` marks a button the adapter acts on with a
  permission (here, reading the agents). The adapter is told as usual.

A button whose permission isn't granted is drawn disabled, and says
which permission it needs. Pressed anyway (from a key, say), it is
refused and logged. The adapter isn't told about the buttons Pitwall
runs.

## Checking it

`pitwall extension check` describes each of these permissions and warns
about `agents.start`. A view fixture
([`views`](checks.md#views)) lists the buttons Pitwall runs and the
permission each needs. In `calls`, the fake Pitwall refuses an action
method without its grant, the same as Pitwall does.
