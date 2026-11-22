# Agention adapter

The process Pitwall starts for the Agention extension (`node adapter/main.mjs`,
from the extension's folder). It speaks Pitwall's adapter protocol, version 1
(`docs/spec/extensions.md`, "Adapter protocol"), on stdin and stdout, and
Agention's operator API on the other side. The server address and API key
arrive only in the first stdin line, `hello`; logs go to stderr, and the key is
never written anywhere.

Node.js 24 or later. No dependencies, no build step, nothing to install.

| File | What it holds |
| --- | --- |
| `main.mjs` | The entry point: stdin and stdout to `lib/adapter.js` |
| `lib/adapter.js` | The protocol: `hello`, `status`, `threads`, `thread`, requests and `result`s |
| `lib/client.js` | The HTTP client: errors, retries, paging, the wait loop |
| `lib/operator-v1.js` | Everything about Agention's API: routes, scopes, fields, bounds |
| `lib/model.js` | Agention's data into the shapes Pitwall draws |
| `lib/errors.js` | Typed errors, in plain words |
| `lib/bounds.js` | Checks for data from the server: types, sizes, counts |
| `test/fake-agention.js` | A fake Agention server for the tests |

Agention's operator API is in a draft pull request
([WayfarerLabs/agention#41](https://github.com/WayfarerLabs/agention/pull/41));
when it changes, `lib/operator-v1.js` (and the fake server) are what change.

The API key needs the scopes `threads:write` and `sessions:read`. On start the
adapter checks the server's operator API versions (without the key), then the
key's scopes, with requests that change nothing.

## Tests

```bash
cd extensions/agention/adapter
node --test test/*.test.js
```

The tests start the fake server on `127.0.0.1` on a port the system picks, and
spawn `main.mjs` for the end-to-end ones. Sleeps in the retry and backoff
tests are injected, so nothing waits long; the whole run takes a few seconds.
No request leaves this computer.
