# Permissions

An extension that reads Pitwall's data says so in its manifest
([format 2](manifest-2.md)): each permission it asks for, with its own
reason. The user grants or refuses each one when turning it on, and can
change their mind at any time.

```json
"permissions": {
  "agents.read": "To show your agents and their statuses in its inbox",
  "worktrees.read": "To link a CI run to the worktree it built"
}
```

A permission bounds what **Pitwall gives** the extension: which
[methods](methods.md) it may call, which events it gets. It is not a
sandbox. The adapter is a program that runs as the user, and can read
what any program of theirs can. Say plainly why you need each one: the
user reads your reason next to Pitwall's own words for it.

## The permissions

| Permission | Gives | In this Pitwall |
|---|---|---|
| `agents.read` | The agents Pitwall shows, their state and their changes as events (`agent.list`, `agent.status`, `events.subscribe`) | yes |
| `projects.read` | The user's projects: names and folders (`project.list`) | yes |
| `worktrees.read` | Each repository's worktrees, branches and the agents in them (`worktree.list`) | yes |
| `profiles.read` | The extension's own connections among the profiles (`accounts.list`, `project.accounts`); never anything else's | yes |
| `show` | Open a file, a folder or a page in Pitwall | not yet |
| `agents.focus` | Bring an agent's terminal to the front | not yet |
| `agents.queue` | Queue a message for an agent | not yet |
| `agents.start` | Start an agent | not yet |
| `notify` | Show notifications while Pitwall isn't in front, at most 3 a minute, labelled with its name ([notifications](views.md#notifications); it needs `contributes.notifications`) | yes |

A permission this Pitwall doesn't serve yet is listed on the permission
screen as "not available in this Pitwall yet" and is never granted. A
name it doesn't know at all shows as "not available in this Pitwall". The
manifest still loads, so a later Pitwall can grant either.

The read permissions start on in the permission screen; the user turns
off what they don't want. Any permission that acts will start off.

## What the user sees

- **Turning it on.** The switch shows the permission screen in the
  extension's card: each permission, your reason, and what it gives.
  Nothing runs until the user presses **Turn on**; **Cancel** leaves it off.
- **In its card**, under Permissions: each permission with "Granted" or
  "Refused" and the date, and a switch. Turning a grant off applies to the
  very next call. A running adapter isn't restarted: a protocol 2 adapter
  gets a `grants` line with what it may use now.
- **After an update** that asks for more: the update's "what changes"
  says so before anything is installed. Once updated, the new permission
  is **not granted** until the user presses Allow in the card (marked
  "new"). The others keep their answers. One the update no longer asks
  for is forgotten.
- **Activity**, next to Log in its card: each call it made (from its
  adapter or a program it started), when, and whether it was refused.
  It keeps the last 500 for 30 days, in
  `~/.pitwall/extensions/.log/<id>.jsonl`.

The answers are kept in `~/.pitwall/ui.json`, under
`extensions.<id>.grants`, each with its date.

## Programs your adapter starts

A program whose parent chain reaches your adapter (a script, a CLI, a
tool your adapter runs) is your extension on Pitwall's socket too. It
gets what your extension is granted, and its calls are in your Activity.
The user's own terminal is unchanged, and so is an agent's. A process
your adapter started inside an agent's terminal is yours, not the
agent's.

What this does **not** stop: a process that leaves the chain (one that
detaches and is re-parented to the system) is no longer seen as your
extension. It is then an ordinary program of the user's. It gets what
the user's own terminal gets, and nothing marks it as yours. A
permission is a promise about what Pitwall gives an extension that plays
along, not a wall against one that doesn't.

## Limits

- 20 calls a second for each extension, its adapter and its programs
  together. Past that a call is refused with `busy`.
- At most 8 calls from an adapter waiting for their answers at once.
- 10 events a second to an adapter. Pitwall merges the changes in
  between, so nothing is lost but the steps.
- A call's params at most 64 KiB, an answer or event at most 512 KiB.

An adapter refused 100 times in a row for its limits is stopped, and its
card says why. A refusal for a missing permission doesn't count: the
user chose that.
