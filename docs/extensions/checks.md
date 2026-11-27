# Checks

Two commands check an extension's folder. They work offline: no running
Pitwall is needed. Both print JSON by default and plain words with
`--human`. Each exits with status 1 when a check fails, so they fit in
CI. Pitwall's own tests run the same checks on the extensions it ships.

Each line of the `--human` output starts with a mark: `ok`, `note` (worth
a look, not a failure), `FAIL`, or `skip` (not run here, with why).

## `pitwall extension check <folder>`

Reads the folder and runs nothing:

- `extension.json` parses and stays within Pitwall's limits (format 1 or
  [2](manifest-2.md)), with warnings for format-2 keys in format 1, a
  missing `pitwall`, permissions this Pitwall doesn't serve, and an adapter
  that contributes nothing;
- its version;
- its `source`, when it has one: which tag (git) or package version
  (npm) this version's release is. For an npm package, also that
  `package.json` names the package at the same version, with notes for
  dependencies and install scripts (Pitwall installs and runs neither)
  and for a `files` list that leaves out `extension.json`
  ([publishing on npm](manifest.md#publishing-on-npm));
- the adapter's program is found, the way Pitwall would start it;
- the files its command names exist inside the folder;
- a [service](service.md)'s program is found, or its `install` names an
  action (a warning otherwise), and the files its command names exist;
- there is a README and a `test/pitwall.json` (notes when missing);
- the folder's name matches the id (a note when it doesn't).

Then it says, in plain words, what the extension would do: the settings
the user fills in (with their types, options, ranges and defaults), the
permissions it asks for with its reasons (and a warning for
`agents.queue` and `agents.start`, saying what the user sees and the
limits: [actions](actions.md)), what it contributes (and a warning for
`panels`, which are never shown now), the
command it runs while on, the variables it sets on agents, how its
threads match agents, and each setup action's exact commands. A
service is described with its command, its version check, its first run,
its variables and its data folder, and a note that it runs continuously
while the extension is on.

```
$ pitwall extension check extensions/demo-inbox --human
Demo inbox (demo-inbox)
  ok    extension.json is valid and within Pitwall's limits (format 1)
  ok    version 1.0.0
  ok    its adapter runs node, found at /opt/homebrew/bin/node
  ok    adapter.mjs is there
  note  there is no README.md: say what it does and how to set it up
  ok    test/pitwall.json is there: `pitwall extension test` runs it

The user sets (they reach the adapter only in its hello):
  Any address (greeting): a web address, optional

While it is on, it runs this in its folder:
  node adapter.mjs

A thread is an agent's when its names are:
  agent {project}, session {agent_id}

Passed.
```

## `pitwall extension test <folder>`

Runs what `test/pitwall.json` expects. Every part is optional. If a
program a part needs isn't installed (Node, say), that part is skipped,
not failed. In every run, the adapter must exit when its stdin closes.

```
$ pitwall extension test my-inbox --human
My inbox (my-inbox)
  ok    it answers hello with protocol 1
  ok    it says status “ready”
  ok    it lists 1 thread(s)
  ok    opened, thread “welcome” comes with 1 message(s)
  ok    a reply is answered ok
  ok    it exits when its stdin closes

Passed.
```

### `run`

Starts the adapter, the way Pitwall does.

```json
"run": {
  "settings": { "server": "https://example.com" },
  "secrets": { "apiKey": "test-key" },
  "status": "ready",
  "threads": 1,
  "open": "welcome",
  "reply": "Hello back"
}
```

- `settings` and `secrets` go in its `hello`.
- It must answer `hello` with protocol 1.
- It must then send `status` (`ready` unless `status` says otherwise) and
  list at least `threads` threads.
- `open` (a thread id) must come back as a `thread` with its messages.
- `reply` is sent to that thread as a `message`, and must be answered
  with `result` ok.

### `actions`

Plans setup actions and runs nothing:

```json
"actions": {
  "installed": ["claude", "npm"],
  "expect": {
    "setup": {
      "label": "Set up … in my agents",
      "lines": ["claude plugin install …"],
      "skips": [null],
      "optional": [false]
    }
  }
}
```

- `installed`: the programs taken as installed.
- `accounts` (optional): per tool, the names of Pitwall accounts of it to
  plan with, such as `{"claude": ["Work", "Home"]}`. A step `for` that
  tool is planned for its default folder and then once for each, as
  Settings would run it, so `lines` lists every command.
- `expect`, for each action id: its `label`, and for each step its exact
  command line (`lines`), why it would be skipped (`skips`, or `null`),
  whether it is `optional`, and optionally where it runs (`for`: "Claude
  Code, default", "Claude Code, account Work", or `null` for a step for
  no tool).

### `env`

The variables Settings shows:

```json
"env": {
  "unset": ["MY_URL (not set until Server address is)", "MY_AGENT={project}"],
  "values": { "server": "https://example.com" },
  "set": ["MY_URL=https://example.com", "MY_AGENT={project}"]
}
```

- `unset`: what Settings shows with no settings.
- `set`: what it shows with `values` filled in.

### `service`

What Pitwall would run for the extension's [service](service.md), and
what it would do in each case. Nothing is started.

```json
"service": {
  "values": { "server": "https://example.com" },
  "expect": {
    "run": "inbox serve --data {data} --origin https://example.com --listen 127.0.0.1:8787",
    "version": "inbox version",
    "firstRun": "inbox link --data {data}",
    "backup": "inbox backup --data {data} {backup}/inbox.db"
  },
  "cases": [
    { "version": "inbox 0.6.0", "verdict": "starts (The inbox server 0.6.0)" },
    { "version": "inbox 0.5.0", "verdict": "The inbox server 0.5.0; this extension needs 0.6.0 or later: its install action is offered" },
    { "installed": false, "verdict": "“inbox” isn't installed: its install action is offered" },
    { "portTaken": true, "verdict": "something else is already listening on 127.0.0.1:8787: that one is used, nothing is started" },
    { "values": { "server": "" }, "verdict": "waits for Server address to be set" }
  ]
}
```

- `values`: its plain settings (defaults fill the rest).
- `expect`: each command as Pitwall would run it, `{data}` left as is.
- `cases`: what Pitwall finds before it starts the service. `installed`
  (default `true`): its program is on the PATH; `version`: what its
  version command prints; `portTaken`: something else listens on its
  address; `values`: settings changed for this case only. `verdict` is
  what Pitwall then does, in the words `test` prints.

### `update`

The steps an [update](updates.md) to this manifest takes, in order, with
their commands, as the review lists them. Nothing runs.

```json
"update": {
  "from": "test/previous-extension.json",
  "values": { "server": "https://example.com" },
  "setUp": ["setup"],
  "agents": 2,
  "expect": [
    ["Stops The inbox server and backs up its data first:", "inbox backup --data {data} {backup}/inbox.db"],
    ["Installs Inbox 1.3.0 (the installed 1.2.0 is kept for Go back)."],
    ["Starts The inbox server and waits until it is up (at most 60 s)."],
    ["Checks that The inbox server is up and Inbox connects as before (within 30 s). If not, it puts back Inbox 1.2.0, its data and its service, and says what failed."]
  ],
  "agentsSay": [
    "2 running agents keep the previous setup until restarted.",
    "After the update: Pitwall restarts nothing by itself: their rows and this card say so, with Restart."
  ]
}
```

- `from`: the manifest updated from, a file in the folder (default: this
  one). The update is planned with the extension on, its service run by
  Pitwall and its data folder not empty.
- `setUp`: the setup actions set up before; one whose steps changed runs
  again for every account.
- `agents`: how many running agents use its setup now.
- `installed`: programs taken as installed, as fakes on an empty PATH,
  as for `actions`. Give it when a setup action runs again, so the steps
  don't depend on what the computer running the check has installed.
- `expect`: each step, its title then its lines (`{data}` and `{backup}`
  left as is). `agentsSay`: what the review says about those agents.

### `agreement`

Checks your real adapter against a fake of your service, which you
write and keep in the folder. Use it when the adapter talks to a server.

```json
"agreement": {
  "fake": ["node", "adapter/test/serve-fake.mjs"],
  "settings": { "server": "{url}" },
  "secrets": { "apiKey": "{key}" },
  "answer": { "pick": 1, "label": "SQLite", "text": "in the data dir" }
}
```

The fake is a command run in the folder:

- Its first stdout line is a JSON object with the id of the `thread` it
  serves, plus the values that `{field}` in `settings` and `secrets` stand
  for (here `url` and `key`).
- It serves one thread with one blocking ask.
- It stops when its stdin closes.

The adapter, given those settings and secrets, must:

- connect and say ready;
- list that thread, unread, with its blocking ask;
- open the thread with the ask and its suggestions;
- carry `answer` (the suggestion picked and the user's words) back, so
  that the answer shows in the thread.

### `calls`

For a [protocol 2](protocol-2.md) adapter: `test` plays Pitwall's side,
answering its calls from the fixture, and checks what it does with them.

```json
"calls": {
  "grants": ["agents.read"],
  "settings": {}, "secrets": {},
  "answers": {
    "agent.list": [{ "id": "a1", "name": "fix-login", "status": "working" }],
    "project.list": { "error": { "code": "denied", "message": "Not allowed" } }
  },
  "events": [{ "event": "agents.changed", "data": [{ "id": "a2", "name": "docs-pass" }] }],
  "expect": {
    "called": ["agent.list", "events.subscribe"],
    "subscribes": ["agents"],
    "threads": 1,
    "open": "agents",
    "text": "fix-login",
    "afterEvents": "docs-pass"
  }
}
```

- `grants` go in its hello. Each must be one its manifest asks for:
  Pitwall never grants anything else.
- `answers`: what each method returns, or `{"error": {code, message}}` to
  refuse it. `events.subscribe` is answered with the topics asked for. A
  method that isn't listed gets `unknown_method`.
- `expect`: the methods it calls and the topics it subscribes to; at
  least `threads` threads listed; thread `open`, once opened, shows
  `text`. After the `events` are sent, the thread shows `afterEvents`.

An adapter that answers hello with protocol 1 fails this part.

### `views`

For an extension that [contributes](views.md) views, commands or badges:
`test` plays Pitwall's side (answering calls as `calls` does) and checks
what the adapter draws.

```json
"views": {
  "grants": ["agents.read"],
  "answers": { "agent.list": [{ "id": "a1", "name": "fix-login" }] },
  "show": "runs",
  "kind": "list",
  "items": 2,
  "select": "r812",
  "detail": "detail",
  "action": { "item": "r812", "action": "rerun" },
  "command": "refresh",
  "badges": 1
}
```

- After `show`, view `show` arrives as a `kind` root (`list`, `detail`,
  `markdown` or `status`) with at least `items` items; one Pitwall would
  refuse (too big, a form) fails with its reason.
- After `select`, the view comes again with a `detail` of that kind
  beside the list.
- `action` (on `item`, of view `show`) and `command` are answered ok.
- At least `badges` badges or notes are sent.
- `grants`, `answers`, `settings` and `secrets` are as in `calls`.
- Each button Pitwall runs itself (`open`, `focus`) is listed with the
  permission it needs. One whose permission the manifest doesn't ask for
  is a warning: it is always drawn disabled.

In `calls` and `views`, the fake Pitwall refuses `queue.add`,
`agent.create`, `agent.focus`, `viewer.open` and `web.open` with
`denied` unless `grants` has their permission, as Pitwall does. Each
action called is reported.

Each key is optional; give the ones your extension does.
