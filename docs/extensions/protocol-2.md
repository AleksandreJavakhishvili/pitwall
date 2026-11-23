# Protocol 2

Protocol 2 is [protocol 1](protocol.md) with one addition: the adapter
can **ask Pitwall** for data. It makes calls, gets replies, and gets
events it subscribed to. Everything in protocol 1 (the inbox lines,
start and stop, the limits) is unchanged. An adapter that answers
`"protocol": 1` keeps working as before.

## Agreeing on a protocol

Pitwall's `hello` says which protocols it speaks, which permissions the
user granted, what it offers, and which connection this adapter serves:

```json
{"type":"hello","protocol":1,"protocols":[1,2],"pitwall":"0.9.0",
 "settings":{"server":"https://…","limit":20,"branchesOnly":true},"secrets":{"apiKey":"…"},
 "grants":["agents.read"],"offers":["inbox","call","events"],
 "connection":{"id":"ext-my-inbox-work","name":"Work"}}
```

- `protocol` stays 1 so that an adapter written for protocol 1 reads the
  same line it always did. A protocol 2 adapter reads `protocols`.
- `grants`: the permissions the user granted, out of those your manifest
  asks for. Calls that need another are refused.
- `offers`: what this Pitwall can do for an adapter. Check it rather than
  the version: `inbox`, `views`, `commands`, `badges`, `notify`,
  `panels`, `webViews` (what it draws: [views, panels, commands, badges
  and notifications](views.md)), `call` (calls and replies), `events`
  (`events.subscribe`). A later Pitwall adds to it.
- `settings` are typed: a `bool` setting is `true` or `false`, a `number`
  a number, the others strings ([settings](manifest-2.md#setting-types)).
- `connection`: present when the extension has connections.

Answer with the protocol you picked, out of `protocols`, and optionally
what you use:

```json
{"type":"hello","protocol":2,"uses":["call","events"]}
```

A protocol Pitwall didn't offer stops the adapter. Its card then says
"It speaks protocol 3; this Pitwall speaks 1 and 2. Update the extension
or Pitwall."

## Calls

```json
{"type":"call","id":"c1","method":"agent.list"}
{"type":"call","id":"c2","method":"agent.status","params":{"agentId":"a1"}}
```

- `id`: your own, a string of at most 64 characters, given back in the
  reply.
- `method`: one of the [methods](methods.md).
- `params`: an object, at most 64 KiB, when the method takes one.

Each call gets exactly one reply, in any order:

```json
{"type":"reply","id":"c1","result":[{"id":"a1","name":"fix-login","status":"working"}]}
{"type":"reply","id":"c2","error":{"code":"denied","message":"Not allowed: My inbox isn't granted agents.read (Settings → Extensions → My inbox → Permissions)."}}
```

An error's `code` is one of `denied` (not granted, revoked, or the user
said no), `unsupported` (never an extension's, or not in this Pitwall),
`busy` (past a limit: wait), `approval_timeout` (nobody answered an
approval in time), `bad_params`, `not_found`, `too_large` or
`unknown_method`. Its
`message` is plain words you can show. A refusal never stops the adapter.
Persistent abuse of the limits does (see [permissions](permissions.md#limits)).

Four permissions let calls act: `show` (`viewer.open`, `web.open`),
`agents.focus` (`agent.focus`), `agents.queue` (`queue.add`) and
`agents.start` (`agent.create`, which asks the user every time). Their
rules are in [actions](actions.md). Every other method that acts (send
what is queued, stop or remove an agent, answer an approval, change a
setting) is never an extension's: `unsupported` or `denied`.

## Events

Subscribe with a call; `topics` is optional (all you may have):

```json
{"type":"call","id":"s1","method":"events.subscribe","params":{"topics":["agents"]}}
{"type":"reply","id":"s1","result":{"topics":["agents"]}}
```

The reply lists the topics you got: only those your grants allow. Then
events come as they happen:

```json
{"type":"event","event":"agents.changed","data":[…]}
```

There are at most 10 a second. Changes in between are merged, so the
latest state always comes. Subscribing again replaces the topics. Events
stop when the adapter stops, and when the permission behind them is
revoked.

## When the user changes a grant

The adapter isn't restarted. Pitwall sends what it may use now:

```json
{"type":"grants","grants":[]}
```

A call that needed what was revoked is refused from that moment.

## Programs you start

A program your adapter starts can use Pitwall's socket (`pitwall --json
…`). Pitwall sees by its parent chain that it is your extension's, and
it gets the same grants. See [permissions](permissions.md#programs-your-adapter-starts)
for what this does and does not cover.

## Try it

`pitwall extension new <folder> --protocol 2` writes an adapter that
lists the agents in its inbox thread and follows their changes. Its
`test/pitwall.json` has a `calls` part that plays Pitwall's side (see
[checks](checks.md#calls)).
