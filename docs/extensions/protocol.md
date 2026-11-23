# Protocol 1

An adapter that also reads Pitwall's data speaks [protocol 2](protocol-2.md):
everything here, plus calls and events.

Your adapter and Pitwall talk in JSON lines: one JSON object per line,
UTF-8, each object with a `type`. Pitwall writes to the adapter's stdin.
The adapter writes to its stdout. Its stderr is for you: it shows in the
[log](dev-mode.md#log), and its last lines show when it stops. Types and
keys a side doesn't know are ignored.

## Start and stop

Pitwall starts the adapter in the extension's folder while the extension
is on. Its first line is:

```json
{"type":"hello","protocol":1,"pitwall":"0.8.0","settings":{"server":"https://…"},"secrets":{"apiKey":"…"}}
```

Answer within 10 seconds, or Pitwall stops the adapter:

```json
{"type":"hello","protocol":1}
```

Then send a `status` and the `threads`.

Pitwall stops the adapter when the extension is turned off, when its
settings change (it starts again after), on Reload and when Pitwall
quits. To stop it, Pitwall closes the adapter's stdin, waits 2 seconds,
then kills its whole process group. **Exit when stdin closes.**

## From the adapter

```json
{"type":"status","state":"ready"}
{"type":"status","state":"signedOut","message":"The API key was refused: set a new one in Settings."}
```

`state` is one of `connecting`, `ready`, `signedOut` (a credential is
missing, wrong or expired) or `error`. `message` tells the user what to
do.

```json
{"type":"threads","threads":[{"id":"t1","title":"Which database?","from":{"agent":"api","session":"a1b2"},"unread":1,"openAsks":1,"blocking":true,"lastActivity":"2026-11-23T10:00:00Z"}]}
```

Each `threads` message is the whole list (at most 500), replacing the
last one. Send it after you start, whenever the list changes, and after
`refresh`.

- `id` is yours (1–200 characters). It comes back in Pitwall's requests.
- `from` is optional. With
  [`matchAgents`](manifest.md#matching-threads-to-agents), it ties the
  thread to an agent.
- `blocking` means one of the thread's open asks holds an agent up.

```json
{"type":"counts","unread":3,"openAsks":2,"blocking":1}
```

Optional: the totals, when your service knows more than the list holds.

```json
{"type":"thread","thread":{"id":"t1","title":"Which database?","unread":0,"canReply":true,"earlier":false,"messages":[…]}}
```

Send one thread with its messages after `open`, and again whenever an
open thread changes. It holds at most 200 messages, oldest first, and
every message with an open ask is included however old it is. `earlier:
true` means there are older messages you didn't send.

- A message is
  `{"id","author":{"kind":"agent"|"person"|"you","name"},"time","text","detail","asks":[…],"answers":[…],"withdrawals":[…]}`.
- `text` and `detail` are Markdown. `detail` is shown folded.
- An ask is
  `{"number":1,"question":"…","multiple":false,"open":true,"blocking":true,"suggestions":[{"label":"SQLite","description":"…"}]}`.
  Once it is answered, it also carries `"answer":{"declined":false,"picks":[0],"text":"…"}`.

```json
{"type":"result","req":7,"ok":false,"message":"That thread was archived."}
```

Send one `result` for each request. Its `message` says why it failed, in
plain words.

## From Pitwall

`req` is a number Pitwall picks, and each request with one gets exactly
one `result`.

| Line | What to do |
| --- | --- |
| `{"type":"open","thread":"t1"}` | Send that thread now, and keep it current while it is open. |
| `{"type":"close","thread":"t1"}` | Its view closed. |
| `{"type":"answer","req":7,"thread":"t1","ask":1,"picks":[0],"text":"…","declined":false}` | The user answered an ask with picks, their own words, or both. A decline has neither. |
| `{"type":"message","req":8,"thread":"t1","text":"…"}` | A plain reply (at most 4000 characters). |
| `{"type":"markRead","req":9,"thread":"t1"}` | The user read the thread. |
| `{"type":"archive","req":10,"thread":"t1"}` | The user archived the thread. |
| `{"type":"refresh"}` | Send the list again. |

If your service can't do a request, answer `ok: false` with why.

## What Pitwall does with your data

Everything from an adapter is treated as untrusted data:

- Strings and lists are cut to their limits.
- Control characters are removed.
- Markdown is drawn without HTML, images or scripts. Links open in the
  browser only when the user clicks them, and only `http(s)` links.
- Nothing you send is run or used to pick a file.

Pitwall stops an adapter that sends a line over 1 MiB, or more than 200
lines in a second. Pitwall doesn't restart a crashed adapter by itself:
the user restarts it.

Every limit is in [the specification](../spec/extensions.md#adapter-protocol-version-1).
