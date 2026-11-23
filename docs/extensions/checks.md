# Checks

Two commands check an extension's folder. They work offline: no running
Pitwall is needed. Both print JSON by default and plain words with
`--human`. Each exits with status 1 when a check fails, so they fit in
CI. Pitwall's own tests run the same checks on the extensions it ships.

Each line of the `--human` output starts with a mark: `ok`, `note` (worth
a look, not a failure), `FAIL`, or `skip` (not run here, with why).

## `pitwall extension check <folder>`

Reads the folder and runs nothing:

- `extension.json` parses and stays within Pitwall's limits (format 1);
- its version;
- the adapter's program is found, the way Pitwall would start it;
- the files its command names exist inside the folder;
- there is a README and a `test/pitwall.json` (notes when missing);
- the folder's name matches the id (a note when it doesn't).

Then it says, in plain words, what the extension would do: the settings
the user fills in, the command it runs while on, the variables it sets on
agents, how its threads match agents, and each setup action's exact
commands.

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
- `expect`, for each action id: its `label`, and for each step its exact
  command line (`lines`), why it would be skipped (`skips`, or `null`) and
  whether it is `optional`.

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

[`extensions/agention/adapter/test/serve-fake.mjs`](../../extensions/agention/adapter/test/serve-fake.mjs)
is an example.
