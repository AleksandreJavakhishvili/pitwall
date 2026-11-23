# Writing a Pitwall extension

An extension brings something that isn't Pitwall's own, such as a tool or
a service you run, into Pitwall's UI. Today that means an **inbox**:
threads, messages and questions your agents ask, which the user answers
in Pitwall. Pitwall loads no code from an extension. It draws everything
itself from the data your extension's **adapter** sends: a separate
program that Pitwall starts, which talks to Pitwall over JSON lines on
stdin and stdout.

An extension is a folder:

```
my-inbox/
  extension.json     the manifest: what it is, its settings, what it runs
  adapter.mjs        the adapter (any language; the starter uses Node)
  README.md          what it does and how to set it up
  test/pitwall.json  what `pitwall extension test` checks
```

## Start here

```sh
pitwall extension new ~/code/my-inbox --name "My inbox"   # --protocol 2 reads Pitwall's data, --view draws a view tile, --panel a right-panel section
pitwall extension check ~/code/my-inbox --human
pitwall extension test ~/code/my-inbox --human
```

Then, in Pitwall: Settings → Extensions → **Load from folder…**, pick the
folder and turn it on. Its threads show in the inbox (⌘I). Edit, press
**Reload**, and look at **Log** to see what was said both ways.

## Pages

- [The starter](starter.md): what `pitwall extension new` writes and
  where to go from it.
- [The manifest](manifest.md): `extension.json`, format 1.
- [Manifest format 2](manifest-2.md): the Pitwall it needs, permissions,
  what it contributes, and text, choice, switch and number settings.
- [Protocol 1](protocol.md): what the adapter and Pitwall say to each
  other.
- [Protocol 2](protocol-2.md): calls, replies and events, for an adapter
  that reads Pitwall's data.
- [Views, panels, commands, badges and notifications](views.md): view
  tiles and their components (list, detail, Markdown, status) with JSON
  examples, right-panel sections, web views, palette commands, sidebar
  badges and notes, notifications.
- [Actions](actions.md): showing files and pages, focusing, queueing
  text for an agent and asking to start one, each behind its permission.
- [Permissions](permissions.md): what an extension may read and do, what
  the user sees, and the limits.
- [Methods](methods.md): what an extension may call, with the permission
  each needs (generated from Pitwall's method table).
- [Dev mode](dev-mode.md): loading your folder in place, Reload, the log.
- [Checks](checks.md): `pitwall extension check` and `test`, and
  `test/pitwall.json`.

The full specification, with every limit, is
[`docs/spec/extensions.md`](../spec/extensions.md). Two extensions in
this repository are worth reading: [`extensions/demo-inbox`](../../extensions/demo-inbox/)
(a fake adapter with made-up threads) and
[`extensions/agention`](../../extensions/agention/) (a real service, with
settings, a secret, setup actions and a fake server for its tests).
