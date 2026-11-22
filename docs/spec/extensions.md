# Extensions

Status: being built. Done: finding extensions, the manifest (with
settings, adapter, agent variables and agent matching), the adapter
protocol (below, fixed for version 1). In progress: the adapter's
lifecycle, settings and secrets, the inbox in the sidebar and its thread
tile, agent variables and badges. The first example is Agention
([`extensions/agention/`](../../extensions/agention/README.md)); a fake
adapter that speaks the protocol is in
[`extensions/demo-inbox/`](../../extensions/demo-inbox/adapter.mjs).

Something that isn't Pitwall's own (a tool, a service the user runs) gets
a place in Pitwall's UI without being part of Pitwall's code. An extension
is a folder with a declarative manifest and, optionally, an **adapter**: a
separate process that fetches the extension's data and sends it to
Pitwall over a small protocol. **No code from an extension is loaded into
Pitwall**: the app draws everything natively from the protocol's data. It
never changes how agents behave; the only thing it can add to an agent is
the environment variables its manifest lists, shown in Settings.

## Where extensions are
- `<data>/extensions/<id>/extension.json`, in Pitwall's data folder (the
  one with `accounts.json` and `ui.json`: `~/.pitwall` on macOS,
  `$XDG_DATA_HOME/pitwall` on Linux, `$PITWALL_HOME` when set;
  `Paths::extensions_dir`).
- One folder per extension, named after its id. Hidden folders and files
  in `extensions/` are skipped.
- Read when Pitwall starts, and again with "Look again" in Settings →
  General → Extensions. Not watched.
- Installing is copying the folder in (the example's README says how).

## The manifest (`extension.json`, format 1)
```json
{
  "v": 1,
  "id": "agention",
  "name": "Agention",
  "icon": "inbox",
  "description": "Questions your agents ask you, from your own Agention server",
  "settings": [
    { "key": "server", "type": "url", "label": "Server address", "placeholder": "https://agention.example.ts.net", "required": true },
    { "key": "apiKey", "type": "secret", "label": "API key", "hint": "…", "required": true }
  ],
  "adapter": { "command": ["node", "adapter/main.mjs"] },
  "agentEnv": [
    { "name": "AGENTION_URL", "value": "{setting:server}" },
    { "name": "AGENTWORKS_AGENT", "value": "{project}" },
    { "name": "AGENTWORKS_SESSION", "value": "{agent_id}" }
  ],
  "matchAgents": { "agent": "{project}", "session": "{agent_id}" }
}
```
- `v`: the manifest format (optional, 1). A higher one is refused: "it
  needs a newer Pitwall".
- `id` (required): 1–40 characters, lower-case ASCII letters, digits and
  `-`, starting with a letter, not ending with `-`. It must be the
  folder's name. It keys the extension's settings and its sidebar group.
- `name` (required): one line, at most 40 characters (trimmed).
- `icon` (optional): the name of one of Pitwall's icons (`kit::icons`;
  `inbox`, `bell` and `chat` were added for extensions). Checked like an
  id; a name this Pitwall doesn't have draws the extension icon (a puzzle
  piece), so a manifest written for a later Pitwall still loads.
- `description` (optional): one line, at most 200 characters.
- `settings` (optional, at most 10): what the user fills in, drawn by
  Pitwall in Settings under the extension. `key` (letters, digits, `_`,
  starting with a letter, at most 32; unique), `type` (`url`: an http(s)
  address kept in `ui.json`; `secret`: kept in the system's secure store),
  `label` (≤ 60), optional `hint` (≤ 300), `placeholder` (≤ 100) and
  `required` (the adapter doesn't start until it is set).
- `adapter` (optional): `command`, 1–16 strings: the program and its
  arguments, run in the extension's folder. A program with a `/` is a path
  in that folder; otherwise it is looked up on the login shell's PATH.
- `agentEnv` (optional, at most 10): variables set on agents Pitwall starts
  while the extension is on. `name`: upper-case letters, digits, `_`; not
  Pitwall's own (`PITWALL_*`) nor ones a program's start depends on
  (`PATH`, `HOME`, `SHELL`, `USER`, `TERM`, `LD_*`, `DYLD_*`, `LC_*`, …).
  `value`: a template (below), which here may also use
  `{setting:KEY}`: the extension's own `url` setting `KEY`, put in as it
  is (not in name form). A variable naming a setting that isn't set is
  left out. Naming a `secret` is refused ("secrets are never given to
  agents"), as is a key the manifest doesn't have.
- `matchAgents` (optional): two templates, `agent` and `session`, whose
  values for a Pitwall agent are the names (`from`) of the threads that
  agent started: Pitwall badges that agent and offers "Go to agent".
- **Templates** use `{project}` (the last part of the agent's project
  folder), `{agent}` (its name when it starts; it can change) and
  `{agent_id}` (Pitwall's id for it, kept across restarts); other text is
  kept. Each value is put in name form: lower case, anything but `a-z`,
  `0-9`, `_`, `-` becomes `-`, starting with a letter or digit, at most 63
  characters (`pitwall_core::extensions::name_form`), so it satisfies the
  usual `^[a-z0-9][a-z0-9_-]{0,62}$` name rule.
- Keys Pitwall doesn't know are ignored, so later versions can add theirs.
- Not JSON, not an object, a field of the wrong type or out of its limits,
  a missing `id` or `name`, or more than 64 KiB: the extension can't be
  used, and Settings says why (`pitwall_core::extensions::parse`).

## Adapter protocol (version 1)
JSON lines over the adapter's stdin (from Pitwall) and stdout (from the
adapter): one JSON object per line, UTF-8, each with a `type`. The
adapter's stderr is kept (its last few KiB) and shown when it stops.
Types and keys a side doesn't know are ignored. Code:
`pitwall_core::extensions::protocol`.

**Start.** Pitwall starts the adapter when the extension is on and its
required settings are set, in the extension's folder, with Pitwall's
environment (the login shell's PATH) and nothing of the extension's in
argv or the environment. Its first line to the adapter is:
```json
{"type":"hello","protocol":1,"pitwall":"0.7.0","settings":{"server":"https://…"},"secrets":{"apiKey":"…"}}
```
Settings and secrets reach the adapter **only** here (a setting not set is
absent). The adapter answers `{"type":"hello","protocol":1}` within 10
seconds, else it is stopped. It is stopped (stdin closed, then killed
after 2 s) when the extension is turned off, its settings change (then
started again), or Pitwall quits; it must also exit when its stdin
closes.

**Adapter → Pitwall**
- `{"type":"status","state":S,"message":"…"}`: S is `connecting`,
  `ready`, `signedOut` (a credential is missing, wrong, expired or lacks
  access) or `error`. `message` is plain words for the user, saying what
  to do (≤ 500).
- `{"type":"threads","threads":[Thread…]}`: the whole list, replacing the
  last one (≤ 500). Send it after start, whenever it changes, and after
  `refresh`.
  - Thread: `{"id","title","from":{"agent","session"},"unread":n,"openAsks":n,"blocking":bool,"lastActivity":"ISO 8601"}`.
    `id` is the adapter's own (1–200 characters; it comes back in
    requests). `from` is optional (both names, ≤ 100 each). `title` ≤ 300.
    `unread` is the number of unread messages (`true`/`false` read as
    1/0). `blocking`: one of its open asks holds its agent up.
- `{"type":"counts","unread":n,"openAsks":n,"blocking":n}` (optional):
  the whole inbox's unread threads, open asks and blocking asks, when the
  service knows more than the list carries. Without it Pitwall counts the
  list.
- `{"type":"thread","thread":ThreadDetail}`: a thread with its messages,
  after `open` and whenever an open thread changes.
  - ThreadDetail: `{"id","title","from","unread","canReply":bool (default true),"earlier":bool,"messages":[Message…]}`.
    At most 200 messages, oldest first: **every message holding an open
    ask, however old**, then the latest. `earlier`: older messages exist
    that weren't sent (Pitwall says so above the first). If more than 200
    arrive, Pitwall keeps the ones with open asks and then the latest.
  - Message: `{"id","author":{"kind":"agent"|"person"|"you","name"},"time","text","detail","asks":[Ask…],"answers":[Answered…],"withdrawals":[{"ask":n,"reason"}]}`.
    `text` (≤ 8000) and `detail` (≤ 40000, shown folded) are Markdown.
    A message may carry no text at all, only answers or withdrawals:
    Pitwall draws those itself rather than the adapter putting them in
    words.
  - Ask: `{"number":n,"question","multiple":bool,"open":bool,"blocking":bool,"suggestions":[{"label","description"}],"answer":{"declined":bool,"picks":[i…],"text"},"withdrawn":"why"}`
    (≤ 10 asks, ≤ 10 suggestions; `answer` and `withdrawn` only when
    so; `picks` index `suggestions`). `blocking`: the agent is held up
    until it is answered.
  - Answered: `{"ask":n,"question","declined":bool,"labels":["…"],"text"}`:
    an answer to ask `n` of this thread (in this or an earlier message),
    with the words of the suggestions picked, so it reads on its own.
- `{"type":"result","req":n,"ok":bool,"message":"…"}`: how request `req`
  went; `message` says why not, in plain words.

**Pitwall → adapter** (`req` is a number Pitwall picks per request; each
gets one `result`)
- `{"type":"open","thread":id}`: send this thread now and keep it current
  while it is open; `{"type":"close","thread":id}`: its view closed.
- `{"type":"answer","req":n,"thread":id,"ask":n,"picks":[i…],"text":"…","declined":bool}`:
  picks and/or the user's own words (`text`, ≤ 4000, absent when none),
  or a decline (no picks, no text).
- `{"type":"message","req":n,"thread":id,"text":"…"}`: a plain message
  (≤ 4000). A service with a shorter limit splits it itself (Agention's
  adapter keeps the first paragraph as the message and the whole as its
  detail).
- `{"type":"markRead","req":n,"thread":id}`, `{"type":"archive","req":n,"thread":id}`.
- `{"type":"refresh"}`: send the list again.

**Untrusted.** Everything from an adapter is data:
- a line over 1 MiB, or more than 200 lines in a second, stops the adapter
  with that reason ("it sent more than 200 lines in a second");
- strings are cut to their limits (with "…"), control characters (but
  newlines in Markdown) and bidirectional overrides are removed, lists are
  cut to their lengths, an item without a usable id is dropped;
- Markdown is drawn by Pitwall's own inert renderer: no HTML, no images
  loaded, no script; a link opens only when the user clicks it, in the
  browser, and only http(s);
- nothing from an adapter is run, followed or used to pick a file.
- The adapter is read and written off the UI thread; an adapter that
  stops, crashes or floods shows why in its sidebar section and in
  Settings, where "Restart" starts it again. Pitwall doesn't restart a
  crashed adapter by itself.

## On and off
- Each extension found is listed in Settings → General → Extensions: its
  icon, name, description and a switch. One that can't be used shows its
  folder and why.
- **Off until the user turns it on.**
- Kept in `ui.json` as `extensions.<id>.enabled` (absent: off). Keys a
  later version adds to an entry are kept. The settings schema
  (`docs/settings.schema.json`) describes it. It isn't a `pitwall
  settings` key (the registry has fixed keys; these are the extensions'
  ids); editing `ui.json` applies live, as for other settings.
- Right-click on its sidebar heading: "Turn off".

## The sidebar section
- Each extension that is on gets a section after the projects (and
  machines), in folder order: a heading shaped like a project's (chevron,
  its icon, its name; its description in the tooltip). Click folds it;
  folded is kept in `ui.json` `collapsed` as `ext:<id>`.
- Not dragged, and not in the narrow icon rail.

## Later (not built)
- In the protocol: an ask's priority and answer-by time, references,
  topics, and replaced messages (an adapter leaves replaced ones out).
- A badge count on the Dock / taskbar from extensions.
- Notifications.
- Installing from a folder or a link (`pitwall extension install`, a
  Settings row), and watching the folder.
- View kinds besides the inbox.
