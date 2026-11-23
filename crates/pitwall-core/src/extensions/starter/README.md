# __NAME__

A Pitwall extension. It shows one thread in its inbox and takes replies;
make it show your service's data by changing `adapter.mjs`.

## What is here

- `extension.json`: the manifest. Pitwall reads it; nothing in it runs by
  itself.
- `adapter.mjs`: the adapter, a program Pitwall starts while the extension
  is on. It speaks the adapter protocol (version 1) as JSON lines over its
  stdin and stdout. Node 18 or later, no dependencies.
- `test/pitwall.json`: what `pitwall extension test` checks.

## Develop it

1. In Pitwall: Settings → Extensions → **Load from folder…**, and pick this
   folder. It is used where it is, marked "development", and stays off.
2. Turn it on. Its inbox opens with ⌘I.
3. After a change, press **Reload** on its card: the manifest is read
   again and the adapter started again. **Log** shows the lines it sent
   and got, and what it printed on stderr (secrets taken out).

Check it from a terminal, without the app:

```bash
pitwall extension check .    # the manifest, its files, what it runs and sets
pitwall extension test .     # runs test/pitwall.json's expectations
```

## Reference

Pitwall's author guide: the manifest, the protocol, dev mode and checks
(`docs/extensions/` in Pitwall's repository).
