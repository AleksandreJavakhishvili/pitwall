# Extensions

Status: built, unreleased. Finding extensions, the manifest (settings,
adapter, agent variables, agent matching, setup actions), the adapter
protocol (version 1), the adapter's lifecycle, settings and secrets, the
inbox tile, agents' ask badges and notifications. The first example is Agention
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
  folder's name. It keys the extension's settings and its inbox tile.
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
- `actions` (optional, at most 5): setup actions, buttons in Settings
  ("Setup actions" below). Each: `id` (like an icon name), `label` (the
  button, ≤ 40), `description` (one line, ≤ 200) and `steps` (1–12).
  A step: `run`, the program and its arguments as a list of 1–32 strings
  (≤ 500 each; never one shell line: a string is refused); the program
  is a name looked up on the login shell's PATH, not a path or a
  placeholder; arguments may use `{setting:KEY}` of a plain setting,
  never a secret. Optional `for`: the agent kind it is for (`claude`,
  `codex`, …). Optional `optional: true`: its failure doesn't stop the
  steps after it.
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
  stops, crashes or floods shows why in its inbox tile and in
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
- It is turned off in Settings only. Extensions have no place in the
  sidebar.

## The inbox tile
An extension with an adapter has an inbox: a tile of the space's layout,
a peer of the files and web tiles. Code: `main_screen/inbox_tile.rs`.
- **Opening it:** ⌘I (Ctrl+Shift+I elsewhere) opens the first
  extension's inbox that is on; the command palette has "Open <name>
  inbox" for each. It goes where "Open web page…" puts a page (right of
  the focused tile, in it when it is empty). One tile per extension:
  opening it again focuses it. With none on, a toast says where to turn
  one on.
- **Kept:** the pane's `inbox` (the extension's id) in `ui.json`, so it
  is back after a restart, with the thread it showed
  (`extensions.<id>.selected`).
- **Layout:** the threads on the left (300 px), the thread on the right,
  its text at most 760 px wide and lined up with its title. A wide tile
  always shows a thread: the first listed, when none is chosen.
  Narrower than 760 px, the list, and the thread over it with a back
  button (Esc).
- **The list:** "Needs you" (threads with an open ask; its count in the
  segment) or "All". Blocking ones first, then those that ask, then the
  latest. Unread: bold with a dot. A count pill of open asks, amber with
  "waiting" when one blocks the agent. Long titles are cut, the whole in
  the tooltip. Relative times, the exact one in the tooltip. Archived
  threads aren't listed (the protocol has no list of them).
- **The thread:** the agent's messages under its name chip in its
  Pitwall look (its kind's logo and name, through `matchAgents`), yours
  on the right. Markdown text, selectable, its headings a step above the
  text; a message's detail folds under "Show details". A message that
  only answers or withdraws asks shown above isn't drawn again (each ask
  shows how it went). An open ask is a card: its question large, its
  suggestions as buttons (with their descriptions as rows when they have
  any; a number key each), a field for an answer of your own, Send and
  Decline; a blocking one in amber with "Waiting on you". Answered asks
  are one quiet line; withdrawn, closed and declined ones are dimmed. A
  reply box at the bottom when the adapter allows replies (⌘↵ sends):
  one row, growing with its text up to six, then scrolling.
  Opening a thread goes to its first open ask and marks it read.
- **At once, and back:** an answer, a reply or an archive shows as done
  as soon as it is sent; if the adapter says it failed it comes back (the
  ask open again, the reply in the box, the thread in the list) with
  "Not sent: why" in the thread.
- **Keys**, only while the tile itself has the keyboard (never while
  typing in its fields, never in a terminal): ↑↓ or J K go through the
  threads, Enter opens one, 1–9 pick a suggestion of the first open ask
  (sent at once; toggled when the ask takes several), E archives, R goes
  to the reply box, Esc goes back to the list (narrow).
- **States**, in the tile: off ("Turn it on", Open Settings); a required
  setting not set ("Connect <name>", what to set, Open Settings); loading
  (grey rows); signed out or unreachable with no threads (the adapter's
  message; Open Settings or Restart); stopped (why, Restart, and its
  last lines of output in a box when it printed any); "Nothing needs
  you". A connection problem while there are threads is a line over
  the list.
- **Bench commands** (`PITWALL_BENCH`, for screenshots): `inbox open
  [ext]`, `inbox show <thread>`, `inbox filter all|needs`, `inbox back`,
  `inbox details <message>`, `inbox pick <n>`, `inbox reply <text>`,
  `inbox close`, and
  `ext-action <ext> <action>` (Settings → Extensions with that action's
  confirmation shown).
- **Shipped extensions' checks:** Pitwall's tests read every folder
  under the repo's `extensions/`: each manifest parses and plans its
  actions, and its `test/pitwall.json`, when there is one, says what
  else to expect: an action's exact command lines (with the programs
  taken as installed), the variables Settings shows before and after
  its settings are set, and a fake server its real adapter must agree
  with Pitwall against (skipped without Node).

## Agents' ask badges and notifications
- An agent whose threads (matched through `matchAgents`) have open asks
  gets a pill on its own sidebar row: an inbox icon and the count, amber
  when one blocks it. A click opens the inbox at the thread with the
  oldest open ask. (One call in `agent_row`, `agent_ask_badge`.)
- A thread with more open asks than in the last list notifies as an
  agent that needs you does (`platform::notify_attention`: not while
  Pitwall's window has your attention, never from a bench instance; the
  first list after a start never notifies).

## Setup actions
What "setting up" means for an extension is the manifest's to say, such
as installing a tool's plugin in the user's agents. Code:
`pitwall_core::extensions::Action`, `crate::extensions::actions`.
- Settings shows each action of an extension that is on as a button
  with its label, its description beside it.
- **Pressing it runs nothing.** It opens a confirmation listing each
  command exactly as it will run, one per line (arguments with spaces
  quoted for reading; no shell is involved), and each step that will be
  skipped with why: its program isn't installed ("Codex isn't
  installed", for a step `for` a kind), or a setting it uses isn't set
  ("set Server address first"). Run and Cancel.
- After Run: the steps run in order, off the UI thread, in the user's
  home folder, each with a 3-minute timeout and its output kept short.
  Each step then says **Done**, **Failed** (why: the exit code and the
  last lines of output, or that it took too long and was stopped), or
  **Skipped** (why). A failed step that isn't `optional` skips the later
  steps for the same kind (or every later step when it is for none).
- Nothing runs when an extension is found, turned on, at startup, or at
  an adapter's request: only from the button, after the confirmation.

## Later (not built)
- Pitwall registering an extension's MCP server in agents itself, rather
  than a setup action running the agent tools' own commands.
- In the protocol: an ask's priority and answer-by time, references,
  topics, and replaced messages (an adapter leaves replaced ones out).
- A badge count on the Dock / taskbar from extensions.
- Marking a thread unread (the protocol has only "mark read"), and a
  list of archived threads.
- Dragging the inbox tile to another place (it is closed and opened
  again instead).
- Installing from a folder or a link (`pitwall extension install`, a
  Settings row), and watching the folder.
- View kinds besides the inbox.
