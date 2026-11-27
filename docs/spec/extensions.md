# Extensions

> **Superseded in part (owner, 2026-11-28): self-contained extensions.**
> Everything an extension shows now comes from its WebAssembly component,
> and Pitwall gives only generic, permission-gated pieces
> ([tile-sdk/extension-model.md](tile-sdk/extension-model.md); [15.5](tile-sdk/extension-model.md#155-removed) lists what is removed and
> what replaces it, [15.7](tile-sdk/extension-model.md#157-migration) is the migration). Sections marked
> **Superseded** below describe the adapter era: they stay true of the
> code until plan item 8c removes it, and are history after. Still
> current: where extensions are, installing, updates from git and npm,
> on and off, format 2's permissions and activity log, the generic parts
> of updating as one step, and dev mode for components.

Status: built, unreleased. Finding extensions, the manifest (settings,
adapter, agent variables, agent matching, setup actions, source),
updates from a git source or an npm package, the adapter
protocol (version 1), manifest format 2 and protocol 2 (permissions,
calls and events, the activity log; "Format 2 and protocol 2" below),
the adapter's lifecycle, settings and secrets, the
inbox tile, agents' ask badges and notifications, questions in the
floating control, and a service Pitwall keeps running ("A service" below);
for authors, dev mode (a folder loaded in place), the
adapter's log, process groups and `pitwall extension check|test|new`
("For authors" below; the author's guide is
[`docs/extensions/`](../extensions/README.md)). The first example is Agention
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
  Extensions (and after Install, Update or Remove there). Not watched.
- Installed from Settings → Extensions (below), or by copying a folder
  in by hand: a folder put there by hand works the same.
- **Development folders** ("For authors" below): a folder anywhere,
  recorded in `ui.json` as `extensions.<id>.devPath` and read where it
  is, never copied. It is listed with the installed ones and wins over an
  installed copy with the same id.

## Installing (the extensions Pitwall ships)
Code: `pitwall_core::extensions::install`, `crate::extensions::catalog`.
- **Shipped with the app**, like the skills: the repo's `extensions/`
  folders that have an `extension.json`, staged by
  `scripts/stage-extensions.sh` without their tests (`test/`), any
  `node_modules` and `.DS_Store`. macOS: `Contents/Resources/extensions`;
  Linux and Windows packages: an `extensions` folder beside the program.
  Found as the skills are: `$PITWALL_BUNDLED_EXTENSIONS`, then
  `../Resources/extensions`, `../lib/pitwall/extensions` and
  `extensions` next to the program, then (development builds) the
  repo's `extensions/`.
- **Available:** each shipped one whose id isn't installed, demos
  (`"demo": true`) last and marked "demo". Install copies it into
  `<data>/extensions/<id>/` atomically: into a hidden folder beside it
  first, then renamed into place, so a failed copy leaves nothing. It
  stays **off**; installing runs nothing.
- **The marker:** an installed copy has `.pitwall-install.json` with the
  version installed and a digest of its files (all but `node_modules`,
  `.git`, `.DS_Store` and the marker). A folder whose files no longer
  match, or that has no marker (put there by hand), is "changed
  locally".
- **Update** is offered when the shipped copy's `version` is newer than
  the installed one's (numeric parts compared in order; a pre-release
  such as `1.2.0-beta` sorts before `1.2.0`; an installed one without a
  version counts as older). Its settings and secrets stay. An extension
  that is on is turned off for the swap (its adapter stops) and on again
  after. The new folder is staged, the old one moved aside, the new one
  moved in (the old one back if that fails), the old one kept for Go
  back ("Updates from a git source" below). A folder changed locally
  asks first: "Replace", and those changes are lost. Review… shows what
  the update changes on this computer first.
- **Remove** asks first. It turns the extension off (its adapter stops),
  forgets its settings and recorded setup actions in `ui.json`, deletes
  its secrets from the system's secure store and deletes its folder (and
  the copy kept for Go back).
- From the command line, by an agent: `pitwall extension install|update|
  remove` ("Setup by an agent").

## Updates from a git source
Status: built, unreleased. An extension updates from its own releases,
without waiting for a Pitwall release. Code:
`pitwall_core::extensions::source` (listing and fetching),
`pitwall_core::extensions::install` (origins, the marker, Go back, what
changes), `crate::extensions::catalog` (checks, offers, Add from Git),
`settings/extensions/updates.rs` (the card).
- **A source** is the manifest's `source` ("The manifest" below): a git
  repository (`https://` only), the folder in it holding
  `extension.json`, and a tag pattern with `{version}`
  (`agention-v{version}`). A tag that matches gives a version; a
  pre-release (`1.2.0-beta`) is offered only to an installed pre-release.
- **Checking** lists the repository's tags (`git ls-remote --tags`: no
  clone, nothing written), at start, once a day while Pitwall runs, and
  with "Check for updates" on the card; off the UI thread. Git runs with
  no terminal prompt, no askpass program and no credential manager
  window (`GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=`, `GCM_INTERACTIVE=never`):
  a private repository works through the user's own credential helper,
  or the check says it can't be read. Without git, or offline, the check
  fails quietly with a plain line on the card ("Couldn't check …: this
  computer seems to be offline"), and the card says when it last checked.
- **Offered:** the newest of the shipped copy and the source's newest
  release whose version is above the installed one; never a downgrade or
  the same version. Between two of the same version, the one from where
  it was installed, else the shipped copy.
- **Looked at before it is installed.** "Review…" on the update line
  fetches the release (below) and lists what changes on this computer
  compared to the installed copy: where it comes from (repository,
  folder, tag, commit), the program it runs while on, the variables it
  sets on agents (added and removed), each setup action whose commands
  changed (its new command lines; a `setup` one says its step will be
  unticked), actions added or gone, settings it asks for or no longer
  asks for, and where its updates come from when that moved. With
  nothing else, "Nothing else changes on this computer: only its files."
  Only "Update to 1.1.0" installs it; Cancel deletes what was fetched.
  The shipped copy is looked at the same way (nothing to fetch).
- **Fetching** a tag: a shallow clone of exactly that tag into a hidden
  folder of `<data>/extensions/` (git's hooks pointed at a folder that
  doesn't exist, no large-file download), its commit checked to be the
  one the listing named, then only the extension's folder copied out
  (without `.git`, `node_modules`, `.DS_Store`) and the clone deleted.
  Refused, leaving nothing: another commit than listed, no such folder,
  a link pointing outside the folder or to a folder (a link to a file
  inside is copied as that file), more than 50 MiB or 5 000 files, a
  manifest that can't be used, of another extension, or of another
  version than its tag. Nothing from the fetched files runs; Pitwall
  never runs a package manager or a build step (a release holds what
  runs).
- **The marker** of a copy installed from a source records it:
  `"source": {"git": …, "dir": …}`, `"tag"` and `"commit"` (a shipped
  copy: `"source": "bundled"`; a marker from before sources counts as
  shipped). The card shows it: "From github.com/… (extensions/agention),
  tag agention-v1.1.0, commit 1a2b3c4 · checked 5 min ago".
- **A moved tag is refused:** a tag that now points at another commit
  than the one the installed copy came from, or than an earlier check
  saw since Pitwall started ("The agention-v1.1.0 tag moved: it pointed
  at 1a2b3c4 and now points at 9f8e7d6. Not installed."), isn't offered.
- **Updating** works as for a shipped copy (an extension that is on is
  stopped for the swap and started again; settings and secrets stay; a
  folder changed locally asks first). It runs nothing by itself.
- **Go back:** an update keeps the installed copy as
  `<data>/extensions/.<id>.previous` (hidden, one step back; an older one
  goes). "Go back to 1.0.0" in the card's footer puts it back in place
  (stopped and started again when on; a folder changed locally asks
  first); there is then nothing more to go back to, and the newer one is
  offered again.
- **Add from Git or npm…** under Available, on "Git repository": a
  repository's https address and, optionally, the folder in it. Pitwall lists its tags and takes the
  newest release by the usual patterns (`<folder>-v1.0.0`,
  `<folder>-1.0.0`, `v1.0.0`, `1.0.0`; `<folder>` is the folder's last
  part), fetches it as above, and shows the extension (icon, name,
  version), "Comes from … Not reviewed by Pitwall" and everything it
  does on this computer, then Install (Replace when the same id is
  installed: the installed copy is kept for Go back). It stays off.
- **A development folder** ("For authors" below) is the author's own:
  its source is never checked, no update is offered, its card has no
  source line and no Go back, and neither an update nor Add from Git
  replaces it ("“x” is loaded from a development folder, which Pitwall
  never replaces: remove it from the list first."). An installed copy
  with its id is hidden while the folder is listed, and comes back with
  its updates and Go back once the folder is removed from the list.
- Folders a fetch left behind (Pitwall quit midway) are deleted at the
  next start.
- Not yet: a per-extension switch to install by itself when nothing in
  "what changes" changed, permissions, signed release feeds, the command
  line.

## Updates from an npm package
Status: built, unreleased. The same updates as from a git source, from
an npm package's versions. Code: `pitwall_core::extensions::npm`
(metadata, versions, download, integrity, unpacking),
`pitwall_core::extensions::install` (the npm origin and its marker),
`crate::extensions::catalog` (checks, offers, Add from npm), `crate::extensions::https` (the https client),
`settings/extensions/updates.rs`.
- **A source** naming a package: `"source": {"npm": "@me/x"}`, optionally
  with `"registry"` (`https://` only, no user or password; default
  `https://registry.npmjs.org`). Exactly one of `git` and `npm`; `dir` and
  `tags` are refused beside `npm`. The name follows npm's rules for a new
  package, strictly (`pitwall_core::extensions::check_npm_package`): at
  most 214 characters; lower-case ASCII letters, digits and `-._`, each
  part starting with a letter or digit; optionally `@scope/name`; not
  `node_modules` or `favicon.ico`. So no paths, addresses or version
  ranges.
- **Nothing runs.** Pitwall never runs npm, node or a package script
  (`preinstall`, `install`, `postinstall`, `prepare`), and never installs
  dependencies: a package holds what it runs, its `extension.json` at the
  package's root. `node_modules`, `.git` and `.DS_Store` in a tarball are
  left out, as from a git release.
- **Checking** reads the registry's metadata of the package
  (`<registry>/<package>`, a scoped name's `/` as `%2f`, asking for npm's
  abbreviated install format), Pitwall's own request over https (`npm::Http`; the app's is ureq,
  rustls with Mozilla's roots): no credentials (no `.npmrc`, token or
  cookie), no proxy, no prompt; 15 s to connect, 30 s for the metadata, at
  most 5 MiB of it. At start, once a day and with "Check for updates",
  off the UI thread. Offline or refused, the card says so in a line
  ("Couldn't check npm: @me/x: …").
- **Versions:** each key of the metadata's `versions` that reads as a
  version and has a tarball. Offered: the newest above the installed one,
  never a downgrade; a pre-release only to an installed pre-release; a
  deprecated version only when no candidate isn't deprecated. A version
  without a sha512 `dist.integrity` (only a sha1 `shasum`) is refused.
- **Integrity:** the tarball is downloaded (https; at most 50 MiB, 180 s;
  a redirect only to https) and its sha512 checked against
  `dist.integrity` before anything is unpacked. The metadata is read
  again at fetch time: a version whose integrity isn't the one offered
  is refused ("Version 1.1.0 of @me/x changed on the registry: its
  integrity was sha512 1a2b… and is now sha512 9f8e…. Not installed."),
  as is one whose integrity differs from the installed copy's or from
  what a check saw since Pitwall started: the npm form of a moved tag.
- **Unpacking** is Pitwall's own (gzip, then tar read by hand), into a
  hidden folder of `<data>/extensions/`: only regular files and folders
  under `package/`, which becomes the extension's folder. Refused,
  leaving nothing: a symbolic or hard link, a device, a pipe or any other
  kind of entry; an absolute path, a `..` part, a name outside
  `package/` or with `\` or `:`; the same file twice; a damaged header;
  more than 5 000 files or 50 MiB unpacked. Files are created new, under
  plain path parts of the hidden folder, and no link is ever made, so
  nothing is written outside it. Modes aren't kept, except that a file
  packed with an executable bit is made executable (as git checks one
  out). Then the manifest: at the package's root, the same id on an
  update, and the same `version` as the package's.
- **The marker** records `"source": {"npm": …, "registry": …}`,
  `"version"` and `"integrity"`. The card shows "From npm: @me/x, version
  1.1.0, sha512 1a2b3c4d5e6f…" (the registry in brackets when it isn't
  npm's own); an update offered is "available from npm: @me/x".
- **Review, Update, Go back, development folders** and leftovers work as
  for a git source (above).
- **Add from Git or npm…** under Available: "npm package" picks a
  package's name and, optionally, its registry. Pitwall takes its newest
  version (stable, not deprecated; a pre-release when there is nothing
  else), fetches and checks it as above and shows what it changes, then
  Install.

## The manifest (`extension.json`, format 1)
*Superseded in part: `adapter`, `agentEnv`, `matchAgents`, `actions`, `contributes.inbox` and the settings' `label`, `hint`, `placeholder` and `required` are removed ([15.5](tile-sdk/extension-model.md#155-removed)); `settings` stays as a schema ([14.6](tile-sdk/platform-services.md#146-settings-secrets-and-the-setup-view)).*

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
  folder's name (a development folder may be named otherwise). It keys the extension's settings and its inbox tile.
- `name` (required): one line, at most 40 characters (trimmed).
- `version` (optional): at most 32 characters, starting with a digit;
  letters, digits and `.-+`. Shown beside the name; what Update compares.
- `demo` (optional, `true`/`false`): a demonstration, listed last under
  Available and marked "demo".
- `docs` (optional): an `https://` address of at most 300 characters,
  linked as "Docs" in the extension's card.
- `icon` (optional): the name of one of Pitwall's icons (`kit::icons`;
  `inbox`, `bell` and `chat` were added for extensions). Checked like an
  id; a name this Pitwall doesn't have draws the extension icon (a puzzle
  piece), so a manifest written for a later Pitwall still loads.
- `description` (optional): one line, at most 200 characters.
- `settings` (optional, at most 10): what the user fills in, drawn by
  Pitwall in Settings under the extension. `key` (letters, digits, `_`,
  starting with a letter, at most 32; unique), `type` (`url`: an http(s)
  address kept in the connection; `secret`: kept in the system's secure
  store), each set per connection ("Connections" below),
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
  button, ≤ 40), `description` (one line, ≤ 200), optional `setup`
  (`true`: a step of the extension's setup checklist) and `steps` (1–12).
  A step: `run`, the program and its arguments as a list of 1–32 strings
  (≤ 500 each; never one shell line: a string is refused); the program
  is a name looked up on the login shell's PATH, not a path or a
  placeholder; arguments may use `{setting:KEY}` of a plain setting,
  never a secret. Optional `for`: the agent kind it is for (`claude`,
  `codex`, …): it runs for that tool's default folder and for each of
  Pitwall's accounts of it ("Setup actions" below). Optional
  `optional: true`: its failure doesn't stop the steps after it.
- `source` (optional): where its updates come from ("Updates from a git
  source" above): `git`, an `https://` address of at most 300
  characters without a user or password; `dir` (optional), the folder in
  the repository holding `extension.json` (relative, no `.` or `..`
  parts); `tags`, a pattern holding `{version}` once, with letters,
  digits and `._/-` around it. An older Pitwall ignores it. Or an npm
  package ("Updates from an npm package" above): `npm`, its name, and
  optionally `registry`, an `https://` address (a Pitwall from before
  npm sources refuses such a manifest).
  ```json
  "source": { "git": "https://github.com/pitwalls/pitwall", "dir": "extensions/agention", "tags": "agention-v{version}" }
  "source": { "npm": "@me/my-ext" }
  ```
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
*Superseded: adapters and the protocol are removed ([15.5](tile-sdk/extension-model.md#155-removed)).*

JSON lines over the adapter's stdin (from Pitwall) and stdout (from the
adapter): one JSON object per line, UTF-8, each with a `type`. The
adapter's stderr is kept (its last few KiB) and shown when it stops, and
goes in its log ("For authors").
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
seconds, else it is stopped. It runs in a process group of its own (a
job object on Windows), and stopping it closes its stdin, gives it 2 s
to exit, then kills the whole group, so whatever it started stops with
it. It is stopped when the extension is turned off, its settings change (then
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

## Format 2 and protocol 2
*Superseded in part: protocol 2 goes with adapters; format 2's permissions, grants and activity log stay, used by components.*

The first part of the extension API (docs/spec/extension-api.md, phase
3): an extension may **read** Pitwall's data with the user's grant.
Author docs: [`manifest-2.md`](../extensions/manifest-2.md),
[`protocol-2.md`](../extensions/protocol-2.md),
[`permissions.md`](../extensions/permissions.md),
[`methods.md`](../extensions/methods.md).

**Manifest format 2** (`v: 2`; `pitwall_core::extensions::parse_for`).
Format 1 reads as before.
- `pitwall`: `>=X.Y.Z`, checked first: an older Pitwall says "it needs
  Pitwall X.Y.Z or later (this is …)".
- `permissions`: `{name: reason}`, at most 16; names are lower-case words
  with dots, reasons one line of at most 200 characters. Names are kept
  even when this Pitwall doesn't serve them (`permissions::ALL`, with
  `available`).
- `contributes`: `inbox`, `views` (≤ 10), `commands` (≤ 20),
  `badges` and `notifications` are drawn (each needs an adapter;
  `notifications` the `notify` permission; see "Views, commands, badges
  and notifications" below); `panels`, `cards`, `webViews` are refused
  as needing a newer Pitwall, others as unknown. In format 2 the
  inbox needs `contributes.inbox` (`Manifest::has_inbox`); in format 1,
  an adapter is an inbox.
- Setting types `text` (≤ 500 characters), `choice` (2–20 options,
  strings or `{value, label}`), `bool`, `number` (`min`, `max`), each
  with an optional `default` (not for secrets). Values are kept as
  strings in accounts.json and sent typed in `hello`
  (`SettingDef::json`); a `required` one with a default, and any bool,
  counts as set. Settings checks a value before keeping it
  (`SettingDef::check_value`).

**Negotiation** (`protocol.rs`). Pitwall's `hello` keeps `protocol: 1`
and adds `protocols: [1, 2]`, `grants` (what the user granted, of what
the manifest asks), `offers` (`inbox`, `call`, `events`) and
`connection` (`{id, name}`). The adapter answers with any offered
protocol (`negotiate`) and optionally `uses`; another stops it ("It
speaks protocol 3; this Pitwall speaks 1 and 2. Update the extension or
Pitwall."). A protocol 1 adapter is unchanged and its calls are refused
with `unsupported`. The inbox lines (`FromInbox`, `ToInbox`) are kept
apart from the general ones (`hello`, `call`, `reply`, `event`,
`grants`).

**Calls** (`extensions/calls.rs`, `hub.rs`). `{"type":"call","id","method","params"}`
(id ≤ 64 characters, params an object ≤ 64 KiB) goes, off the UI
thread, to the in-process daemon (`Server::call_as`) with a caller whose
`extension` is the extension (`RequesterKind::Extension`, key
`ext:<id>`), and comes back as `reply` with `result` or `error {code,
message}` (an answer over 512 KiB is `too_large`). `METHODS` has a third
column (`methods::Ext`): `Grant(permission)` for the reads,
`Later(permission)` for actions a later Pitwall will serve (refused
`unsupported`, naming it), `Never` for the rest. `dispatch` checks it
(`ext_allowed`) for every extension caller; `accounts.list` and
`project.accounts` keep only the extension's own `ext:<id>` connections
and drop git identities. `docs/extensions/methods.md` is generated from
the table (`PITWALL_WRITE_DOCS=1 cargo test -p pitwall-daemon
methods_doc`).

**Events.** `events.subscribe` through a call subscribes the adapter
(`Server::subscribe_as`): only the topics its grants allow
(`agents`, `attention`, `blocked` behind `agents.read`), sent as
`{"type":"event","event","data"}` lines by a thread of its own, at most
10 a second (the daemon's queue merges what comes between), each checked
against the grants as it goes (`may_get`) and against the size limit.
Subscribing again replaces the topics; stopping the adapter closes the
queue.

**Grants** live in `ui.json` `extensions.<id>.grants`:
`{name: {granted, at}}`, `at` in seconds. What may be used now is what
the manifest asks for, is available, and is granted
(`permissions::effective`); one without an answer (added by an update)
is not granted (`undecided`). The daemon's `Registry` holds each
extension that is on with its effective grants; it follows `ui.json`
(`calls::sync`) so a revoke applies to the next call and event. A
protocol 2 adapter is sent `{"type":"grants","grants":[…]}` when they
change; it isn't restarted. An update's "what changes" lists permissions
asked for and no longer asked for; those no longer asked for are
forgotten when it is next turned on.

**The socket** (decision 5; `pitwall_daemon::extensions`). The hub
registers each adapter's pid. A connection whose process's parent chain
(`identity::attribute`, the same process table that tells agents)
reaches an adapter is that extension: the nearest of an agent's holder
and an adapter wins, so a program an adapter started inside an agent is
the extension's. The user's own terminal is unchanged. It doesn't stop a
process that leaves the chain (detached, re-parented to init/launchd):
that one is an ordinary local program of the user's again. The parent
chain comes from `ps` on macOS, `/proc` on Linux and Toolhelp on
Windows.

**Limits.** 20 calls a second per extension, socket and stdio together
(`Registry::admit`, `busy`); 8 calls in flight per adapter; 100 refusals
in a row for limits stop the adapter ("It was stopped: it kept calling
past its limits …"). A refusal for a missing grant doesn't count.

**The activity log** (`pitwall_core::extensions::activity`): each call
from the adapter or the socket, with when, from where, and ok / refused
/ failed, in `<data>/extensions/.log/<id>.jsonl` (500 entries, 30 days;
`discover` skips dot folders). Settings shows it under **Activity** in
the card, next to Log.

**Settings.** Turning on an extension that asks for permissions shows
the permission screen in its card: each permission's title, name, the
extension's reason and what it gives, with a switch (reads start on);
unavailable ones are listed as not available. Nothing is on until
**Turn on**; Cancel leaves it off. A card that is on lists Permissions
with "Granted 14 Nov 2026" / "Refused …" and a switch, and one an update
added as "new" with Allow / Don't allow.

## Settings → Extensions
*Superseded in part: the checklist, the fields drawn from the manifest, More and "What this extension does on your computer" are replaced by the extension's own Setup view and the host's generic page ([3.12](tile-sdk/ui.md#312-the-setup-view-and-host-drawn-fields)).*

A tab of its own (after Accounts). Code: `settings/extensions.rs`.
- **Installed:** one card per folder found. Its top row: icon, name,
  version (a "demo" mark), description, a status pill of fixed width
  (Off, Needs setup, Connecting, Connected, Signed out, Stopped, Error;
  On for one without an adapter), the switch and a fold chevron. Folded
  while off, unfolded while on; the user can fold or unfold it. One
  that can't be used shows its folder and why. With none installed: a
  dashed box pointing at Available.
- **The checklist** (an extension that is on), built from the manifest,
  numbered, each step a tick once done, its field or button on the step:
  each required setting (an optional one counts as done), then each
  `setup` action, then "Connected" (for one with an adapter: the
  adapter's own words when it isn't, and Try again). For Agention:
  1 Server address, 2 API key, 3 Set up Agention in my agents,
  4 Connected. A setup action whose last run had a step done and none
  failed or cancelled is recorded (`extensions.<id>.actionsDone`), so
  its step stays ticked; a later failed run unticks it.
- **Fields:** a `url` field saves when it loses focus or on Enter; one
  that isn't a web address says "That isn't a web address (like
  https://example.com)." under it and isn't saved. Hints sit under their
  field. A secret that is set shows "Saved in your keychain" (Credential
  Manager, keyring) with Replace and Remove, never its value; unset,
  "Paste from clipboard".
- **More:** actions that aren't `setup` (Agention's "Remove Agention from
  my agents"), under the checklist.
- **The footer:** "What this extension does on your computer", folded:
  the command its adapter runs while on, the variables it sets on agents
  (with its settings put in), each action's command lines, its folder.
  Then Docs (the manifest's `docs`) and Remove.
- **Under the top row:** where it came from ("Comes with Pitwall", or
  its source's repository, tag and commit) and, for one with a source,
  when it was last checked, with "Check for updates"; a failed check or
  a moved tag in amber; an update offered ("Version 1.1.0 is available
  from …") with Review…, then what it changes and Update ("Updates from
  a git source"); changed locally; failures. Remove, Replace and Go back
  of a changed folder confirm in a card there. The footer has "Go back
  to 1.0.0" after an update.
- **Available:** compact cards (icon, name, version, description,
  Install), wrapping to the width; "Add from Git or npm…" beside the
  heading (a choice of "Git repository" or "npm package", then its
  fields).

## Connections
*Superseded: connections are removed; one set of settings per extension, the default connection's carried over ([15.7](tile-sdk/extension-model.md#157-migration)).*

Status: built below the screens (docs/spec/profiles.md has the design).
- An extension with `settings` has **connections**: each one set of its
  settings and secrets (one per server or key: "Work", "Personal"). A
  connection is an account of kind `ext:<id>` in `accounts.json`, its
  plain settings in `values`, which secrets are set in `secretsSet`; the
  secrets are in the secure store under `<connection id>.<key>` (the
  extension's id for the one carried over from before connections, so its
  keychain items keep their names). The first one made is the extension's
  default connection.
- Each project gets one connection per extension, or none, through its
  profile or its own pick; where nothing picks, the default connection.
  An agent's `agentEnv` comes from its project's connection
  (`{setting:KEY}` from that connection); with none, nothing is set.
- **One adapter per configured connection** of an extension that is on
  (at most 8; `hub::MAX_RUNNING`), its `hello` carrying that connection's
  settings and secrets. **One inbox per extension**: their threads merged,
  ids `<connection id>:<adapter's id>` so requests reach the right
  adapter, each labelled with its connection; counts added up; a
  connection that stops or can't connect is named over the list without
  hiding the others' threads. `matchAgents`: a thread is an agent's only
  if it is from that agent's connection.
- An extension without settings has no connections: one adapter, its
  variables on every agent, as before.
- **Carried over:** an extension whose `ui.json` entry has `values` or
  `secretsSet` becomes, on start, one connection named after it, the
  default; `selected` gets the connection's prefix. Nothing is
  re-entered. Those keys stay in `ui.json` as a copy of that connection,
  kept up to date, so an older Pitwall still finds it set up; a copy an
  older build changed is taken back on the next start
  (docs/spec/profiles.md, "Migration").
- **The card owns its connections.** With one, the card is as before
  plus "Add connection". With two or more: a tab per connection (status
  dot, the default marked) over the selected one's checklist and setup
  actions, with Rename, Make default and Remove; the pill says "N of M
  connected". A setup action that uses `{setting:KEY}` runs for, and is
  ticked on, the selected connection (`accounts.json` `actionsDone`);
  others are the extension's (`ui.json` `actionsDone`). Removing the
  default asks which connection becomes the default, or none.

## On and off
- **Off until the user turns it on.**
- Kept in `ui.json` as `extensions.<id>.enabled` (absent: off), with
  `selected` (the thread its inbox shows). Keys a
  later version adds to an entry are kept. The settings schema
  (`docs/settings.schema.json`) describes it. It isn't a `pitwall
  settings` key (the registry has fixed keys; these are the extensions'
  ids); editing `ui.json` applies live, as for other settings.
- It is turned off in Settings only. Extensions have no place in the
  sidebar. Turning it off uninstalls nothing its setup actions installed
  (a "remove" action does, when pressed).

## The inbox tile
*Superseded: the built-in inbox is removed; an inbox is an extension's own tile ([15.5](tile-sdk/extension-model.md#155-removed)).*

An extension with an adapter has an inbox: a tile of the space's layout,
a peer of the files and web tiles, or a window of its own. Code:
`main_screen/inbox_tile.rs` (the inbox), `main_screen/inbox_window.rs`
(its window, and where it goes).
- **One per extension:** one inbox view per extension, wherever it is
  drawn (`inbox_tile::view`). Moving it never makes another: its thread,
  its drafts, its picks and the adapter's connection stay as they were.
- **Opening it:** ⌘I (Ctrl+Shift+I elsewhere) opens the first
  extension's inbox that is on; the command palette has "Open <name>
  inbox" for each. It goes where "Open web page…" puts a page (right of
  the focused tile, in it when it is empty). Opening it again focuses it
  where it is: its tile (the space's focused one), or its window, brought
  forward. With none on, a toast says where to turn one on.
- **Its bar**, as a web tile's: a grip, its icon and name, the filter,
  "Move…" (a tile) or "Dock…" (a window), then for a tile Maximise /
  Restore (⌘⏎), "Open in a window of its own" and close; for a window,
  keep on top (macOS).
- **Moving it**, as a web page moves (`web::place`, `explorer::drag`):
  - a tile dragged by its bar to another tile's side, onto an empty tile
    (it takes it), or onto a terminal (they swap), in its window;
  - "Move…": a column on the right or a row at the bottom of the space,
    on its own, in place of a terminal (it keeps running), or a window
    of its own (an inbox has no agent, so there is no "next to its
    agent's terminal");
  - a tile dragged out of every Pitwall window: a window of its own,
    where it was let go (macOS);
  - from its window: "Dock…" (the same places), or its grip dragged over
    a Pitwall window, which shows where it would land (macOS).
  Closing its window closes the inbox; quitting doesn't.
- **A tile like the others:** a press anywhere in it makes it the space's
  focused tile, shown by its border, so ⌘⏎ maximises and restores it
  (after ⌘I too). In its fields ⌘⏎ sends instead (the reply, your own
  answer).
- **Kept:** the pane's `inbox` (the extension's id) in `ui.json`, so it
  is back after a restart, with the thread it showed
  (`extensions.<id>.selected`). In a window: `extensions.<id>.window`,
  and it opens in one again at the next start (behind the rest, without
  the keyboard); the window's place and size are `inboxWindow`.
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
  to the reply box, Esc goes back to the list (narrow). A press in a
  field keeps the keyboard there: everything typed goes in it, after the
  list refreshes and after an answer is sent too. Esc in a field, or its
  ask answered, gives the keyboard back to the tile.
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
  `inbox close`; through GPUI's own event dispatch, in the window the
  inbox is in (nothing is clicked or typed on the screen): `inbox click
  reply|other` (a press in that field), `inbox press` (on its bar),
  `inbox type <text>`, `inbox key <keystroke>…` (`cmd-enter`,
  `escape`…), `inbox drag <x> <y>` (its bar dragged there in its
  window); `inbox place right|left|above|below|replace <agent>`,
  `inbox place edge-right|edge-bottom|own|window` (Move… / Dock…);
  `inbox state` (its fields, which has the keys, where it is), and
  `ext-action <ext> <action>` (Settings → Extensions with that action's
  confirmation shown), `ext-run <ext> <action> <prog args|prog args…>`
  (that action run with these harmless steps in its place),
  `ext-disclose <ext>`, `ext-secret <ext> <key> <value>` (a bench
  instance keeps secrets in memory, never in the keychain), `ext-dev
  <folder>` (that folder loaded as from "Load from folder…", without the
  picker), `ext-log <ext> [off]` (its card unfolded with its log shown or
  hidden) and `settings-end` (the Settings page scrolled to its end).
- **Shipped extensions' checks:** Pitwall's tests run
  `pitwall_core::extensions::check` (the same as `pitwall extension
  check` and `test`, "For authors") on every folder under the repo's
  `extensions/`, and on each staged copy.

## Agents' ask badges and notifications
*Superseded: these came from inbox threads matched by `matchAgents`; a component sets `badges` and `notify` itself.*

- An agent whose threads (matched through `matchAgents`) have open asks
  gets a pill on its own sidebar row: an inbox icon and the count, amber
  when one blocks it. A click opens the inbox at the thread with the
  oldest open ask. (One call in `agent_row`, `agent_ask_badge`.)
- A thread with more open asks than in the last list notifies as an
  agent that needs you does (`platform::notify_attention`: not while
  Pitwall's window has your attention, never from a bench instance; the
  first list after a start never notifies).

## Views, commands, badges and notifications
*Superseded: views and commands fed by an adapter are removed; tiles, commands, `badges` and `notify` come from the component.*

What an extension's adapter draws beyond its inbox (author's guide:
`docs/extensions/views.md`; proposal and differences:
`docs/spec/extension-api.md`, phase 4). Pitwall draws all of it; what
acts (a button Pitwall runs, phase 5) is in "Actions, right-panel
sections and web views".

- **Model** (`pitwall_core::extensions::view`): a view's `root` is a
  `list` (sections of items: id, title, subtitle, tone, icon, ≤ 3
  badges, time, ≤ 5 actions) with an optional pane beside it, or a pane
  alone: `detail` (title, ≤ 30 fields with http(s) links, a Markdown
  body ≤ 40 000 characters, actions), `markdown`, `status` (empty,
  loading or error; ≤ 2 actions). `form` and `table` are refused, said
  in the tile. A line ≤ 256 KiB, ≤ 500 items, ≤ 50 sections; text is cut
  and cleaned as threads are.
- **Protocol** (either version, gated by the manifest): in `view`,
  `badges`, `notify`; out `show`, `hide`, `select`, `action {req, view,
  item?, action}`, `command {req, command, agentId?}`, answered by
  `result`. `hello`'s `offers` lists `views`, `commands`, `badges`,
  `notify`. A view the manifest doesn't declare is refused and logged.
- **Hub** (`extensions/hub.rs`, `hub/views.rs`): a view is kept only
  while a tile shows it, each adapter is told `show` after its hello;
  several connections' lists are merged section by section, item ids
  carrying their connection (`<connection>:<id>`) as threads do. An
  action's failure is a line in the tile until the next.
- **The tile** (`main_screen/view_tile.rs`): one per view, opened by
  "Open <title> (<extension>)" in the palette or a badge, note or
  toast that names it; placed as the inbox is, moved, maximised, in a
  window of its own (`inbox_window.rs` takes a surface key: the
  extension's id for its inbox, `<extension>/<view>` for a view). A pane
  keeps it as `view` beside `inbox` (an older Pitwall shows it empty); a
  view in a window is kept in `extensions.<id>.viewWindows` and opened
  again at start. ↑↓/J K select (sent as `select`), Enter presses the
  selected row's primary action. Bench: `view open|select|action|close|
  state <ext>/<view> …`.
- **Commands**: "<Extension>: <title>" in the palette while it runs;
  `agent` ones only with an agent focused (`agentId` only with
  `agents.read`); sent to the first running connection; the answer's
  message or failure is a toast. No keys.
- **Badges and notes**: the whole set each time, applied at most four
  times a second; an agent by id needs `agents.read`, by `matchAgents`
  names nothing, a project by folder `projects.read` (refusals logged
  once). Drawn beside the ask badge on agents' rows and before the count
  on projects' headers; a click opens what `open` names. Cleared when
  the adapter stops.
- **Notifications**: need `contributes.notifications` and the `notify`
  grant; ≤ 3 a minute per extension, the same id and words not again for
  10 minutes; the extension's name before the title; through
  `platform::notify_attention` (not while Pitwall is in front, never
  from a bench instance) and a toast with "Open" in the main window.
- **Settings**: the permission screen and an extension's card list what
  it adds ("Adds to Pitwall"); the install and update review lists it,
  and what an update adds and drops.

## Actions, right-panel sections and web views
*Superseded: adapter actions, right-panel sections and declared web views are removed; a tile opens a web tile with `show.web`.*

What an extension may do in Pitwall beyond drawing (author's guides:
`docs/extensions/actions.md`, `docs/extensions/views.md`; proposal and
differences: `docs/spec/extension-api.md`, phase 5). Nothing here
changes what an agent itself may do: its permissions, tools and rules
(project and agent scoped) are as before.

- **Permissions** (`pitwall_core::extensions::permissions`): `show`,
  `agents.focus`, `agents.queue`, `agents.start` are served, each off on
  the permission screen until the user turns it on (`on_by_default`).
  `agents.queue` and `agents.start` carry a risk note, drawn in an amber
  box under the reason (`settings/extensions/grants.rs`) and added to
  their line in an update's "what changes" (`install.rs`).
- **One check** (`pitwall-daemon/src/methods.rs`): the method table
  names each action's permission (`queue.add` agents.queue,
  `agent.create` agents.start, `agent.focus` agents.focus,
  `viewer.open`/`web.open` show; `ACTION_METHODS` in core says the same
  and a test keeps them equal). `dispatch` checks it for the adapter's
  stdio, a program it started on the socket and a view's button alike
  (`Via::Adapter`, `Via::Socket`, `Via::View`); every call is in the
  activity log with a short `detail` (which agent, the text, the
  project). `queue.send`, `queue.remove`, approvals, settings, stop,
  restart, remove and `session.add` are never an extension's.
  `agent.focus` (new, `Access::UiOrExtension`) is the UI's and
  extensions' only.
- **`show`**: `viewer.open` / `web.open` as an agent's show tools, with
  `agentId` required and `place` `window`/`replace` refused; the user's
  `agents.showFiles` / `showWeb` settings apply. Never takes the
  keyboard or activates Pitwall.
- **`agents.focus`** (`windows::focus_in_front`): selects the agent's
  tile in the window showing it only while Pitwall is the app in front;
  otherwise `{"focused": false, "note": …}`. Never activates.
- **`agents.queue`** (`pitwall-daemon/src/actions.rs`): text trimmed,
  1–4000 characters; only Pitwall's own agents; one item per agent per
  extension every 10 s (`Registry::may_queue`), at most 3 of its items
  waiting per agent (`busy`). The item carries `origin` (`QueueOrigin`:
  extension id and name, `pitwall-core` `engine`); Next up draws it in
  the focus colour with "From <name> · waits for you: press Send" (or,
  amber, "· sends without asking"); Last sent says "from <name>". Auto-
  send (`engine/ticker.rs`) skips it unless the extension is in the
  engine's `auto_senders`, set from `extensions.<id>.sendWithoutAsking`
  (a dated decision, off by default, shown under the grant with a
  warning and logged when changed). Items of an extension that is off
  or no longer granted are removed (`calls::sync_queue`). The user
  removes one like any other.
- **`agents.start`**: on this computer only (no provider, machine,
  options, account or resume), in one of Pitwall's projects (listed, or
  the folder of one of its local agents), a kind
  Pitwall knows, a prompt ≤ 4000 characters. Each request is an
  approval (`Risk::High`, never remembered) with the project, the tool,
  the name and the prompt in full (the details box scrolls); denied →
  `denied`, unanswered → `approval_timeout`. One waiting at a time,
  three in ten minutes per extension (`Registry::start_asking`). The
  prompt is put in the new agent's Next up as the extension's item,
  never typed in.
- **In views** (`view.rs`): an item's `agent` (`{"agentId"}`) shows the
  agent's name (a click focuses it with `agents.focus`); an action's
  `open` (`{file, line, agentId}` / `{url, agentId}`) or `focus` is run
  by Pitwall (`Hub::press`) as the extension through the method table,
  off the UI thread; `needs` names a permission for one the adapter acts
  on. A button whose permission isn't granted is drawn disabled with a
  tooltip naming it; pressed anyway it is refused and logged.
- **Right-panel sections**: removed with the right panel (2026-11-26).
  `contributes.panels` is still read (`Contributes::retired_panels`), so
  a manifest that has it installs, and never drawn; `check` warns to
  make them views. Adapters aren't offered `panels`, `show`/`hide`/
  `action` carry no `agentId`, and a `view` line with one is refused.
- **Web views** (`contributes.webViews`, ≤ 5): opened by "Open <title>
  (<extension>)" in the palette (`MainScreen::open_ext_web`) as a web
  tile with `ext` set. The address is the manifest's (`{setting:KEY}`
  filled from its settings) or the adapter's `webView` line, https or
  http on localhost / 127.0.0.1 / [::1] only, checked again when opened.
  No script bridge. Its cookies and storage are its own,
  `<data>/webview/profile-ext-<id>` (`own_store`); where stores can't be
  separate it isn't opened. The bar says "from <name>" instead of a
  profile. Bench: `ext-web <ext> <view>`.
- **Floating-control cards**: none. The control shows an extension's
  blocking inbox asks (below); `contributes.cards` stays refused.
- **Authors**: `check` warns about `agents.queue`, `agents.start` and
  `panels`; the `calls`/`views` fixtures' fake Pitwall refuses action
  methods without their grant and lists a view's buttons Pitwall runs.
- **Debug bench**: `ext-call <ext> <method> <json>` / `ext-call-view …`
  call as the extension (debug builds).

## In the floating control
*Superseded in part: inbox questions go with the inbox; the control shows components' `asks` ([14.2](tile-sdk/platform-services.md#142-the-v0x-minimum-built-in-phases-12)).*

The floating "needs you" control (Settings → General,
`general.floatingPrompts`; code: `float/`) shows the open questions of
every extension that is installed, on, has an inbox and whose adapter
runs, after the agents' cards. Without such an extension it looks and
works as before.
- **Which:** each open ask (not answered, withdrawn or closed) of the
  threads with open asks, the first 6 of each inbox in its "Needs you"
  order (`float::model::ASK_THREADS`); blocking ones first, then in
  the inboxes' order. The control shows one card at a time, the agents'
  first (oldest first) and then the questions, with "1 of N" and arrows
  (←/→) to go between them; the pill shows the first and "+N".
- **Asked for only while it could show them:** while the setting is on
  and Pitwall isn't in front, the control `open`s those threads in their
  adapters (and `close`s them when Pitwall comes forward, the setting is
  turned off or a thread has no open ask any more). The hub counts who
  asked, so an inbox tile showing the same thread keeps it. Nothing is
  marked read.
- **A card:** the extension's icon and name (and the connection's, when
  it has several), who asks (the Pitwall agent the thread is matched to
  through `matchAgents`, else the name the thread gives) and the
  thread's title, "Waiting on you" for a blocking one; the question;
  its suggestions as buttons (rows with their descriptions when they
  have any); a field for an answer of your own (Enter sends it with what
  is picked), Send, Decline and Open in inbox. A click on a suggestion
  sends it at once, or toggles it when the ask takes several (Send then
  sends them). The card's height is what it holds; the window is sized
  to it.
- **One answer path, one state:** an answer goes as the inbox tile's
  does (`inbox_tile::answer_ask`: through the extension's inbox view when
  there is one, so the tile shows it as sent too, else straight to the
  hub). The card says "Sending…" until the thread says the ask is
  answered, and goes; answered in the tile, it goes from the control
  too. A refused answer shows "Not sent: why" on the card (the thread's
  note, as in the tile) and can be sent again.
- **Open in inbox** brings Pitwall forward on that thread in the
  extension's inbox (`windows::show_inbox`, as ⌘I opens it).
- **The count** on the pill ("+N") and the header's "1 of N" include
  the questions. On the pill a question shows who asks, the question and Open.
- **Keys:** a press in a card's field gives the control the keyboard for
  it (Pitwall still doesn't come forward); what is typed there never goes
  to an agent's terminal.
- **Bench commands** (`PITWALL_BENCH`): `float preview
  [collapsed|small|medium|large|<w>x<rows>|off]` (a bench instance opens
  no window for the control: its view, the same model and drawing, is
  drawn over the top left of the main window on a grey backdrop, at the
  size the window would have, to capture with the main window; without
  Liquid Glass's slab and without taking the keyboard), `float expand`,
  `float minimise`, `float size
  small|medium|large`, `float resize <edge> <dx> <dy>`, `float pick <n>`,
  `float type <text>`, `float send`, `float decline`, `float open`,
  `float next`, `float prev`, `float show-all`, `float state`; and,
  for screenshots, `float demo one|two|many|question|long|remote|off`
  (made-up agents in place of the real ones), `float demo-error`,
  `float demo-sent`.

## Setup actions
*Superseded: setup actions are removed; a component runs commands through the program capability, each line confirmed ([13.2](tile-sdk/services.md#132-a-programs-the-component-starts)).*

What "setting up" means for an extension is the manifest's to say, such
as installing a tool's plugin in the user's agents. Code:
`pitwall_core::extensions::Action`, `crate::extensions::actions`.
- Settings shows each action of an extension that is on as a button
  with its label, its description beside it.
- **Every place a tool's agents run from.** A step `for` a kind runs
  once per place that kind's agents can run from on this computer: the
  tool's default config folder, then each of Pitwall's accounts of that
  kind (`accounts.json`, local accounts only), each with exactly the
  environment Pitwall launches that account's agents with
  (`pitwall_core::accounts::env`: the kind's `account_env` set to the
  account's folder; the default folder gets the app's own environment).
  Consecutive steps for one kind run as a block for each place in turn
  (default, then the accounts in their order). A step without `for` runs
  once. Code: `pitwall_core::extensions::plan::plan_for`.
- **Pressing it runs nothing.** It opens a confirmation listing each
  command exactly as it will run, one per line (arguments with spaces
  quoted for reading; no shell is involved), and each step that will be
  skipped with why: its program isn't installed ("Codex isn't
  installed", for a step `for` a kind), or a setting it uses isn't set
  ("set Server address first"). With accounts, the commands are grouped
  under where they run ("Claude Code, default", "Claude Code, account
  Work"; "Once" for steps for no tool), so nothing runs that wasn't
  shown. Run and Cancel.
- After Run: the steps run in order, off the UI thread, in the user's
  home folder, each with a 3-minute timeout and its output kept short.
  The step list shows from the moment Run is pressed, each step with its
  command line and its state: waiting, running (a spinner, its time in
  seconds, and its latest output lines in a small mono box of at most
  five lines), done, failed (why: the exit code and the last lines of
  output, or that it took too long and was stopped) or skipped (why). A
  failed step that isn't `optional` skips the later steps for the same
  kind and account (or every later step when it is for none): a failure
  for one account never hides another's result. With accounts, each
  place's steps show under its name with how they stand (done, failed,
  running). At the end: "All set" or "2 of 6 steps failed"
  ("Cancelled").
- **Cancel** while it runs stops the running step's program and skips
  the rest. "Run again" after it ends.
- The run belongs to the app, not to Settings (`actions::Runs`):
  closing Settings and opening it again mid-run shows it as it is.
- Nothing runs when an extension is found, turned on, installed,
  updated, at startup, or at an adapter's request: only from the button,
  after the confirmation.
- **A tick is for the steps that ran.** A recorded run keeps a digest of
  its action's steps (programs, arguments, `for`, `optional`;
  `extensions.<id>.actionSteps` in `ui.json`, by action, or
  `<action>@<connection>` for one kept per connection). When the steps
  change (an update moved a version pin), the step is unticked and asks
  to be run again; going back to the steps that ran ticks it again. A
  record from before digests is sealed with the steps its action has
  when Pitwall starts.
- **A tick is for every account.** A run ticks its step when the steps
  for the default folders (and for no tool) ran without a failure (an
  `optional` step's failure doesn't count) and one was done; it keeps the
  accounts it went through without a failure
  (`extensions.<id>.actionAccounts`, by the same keys as `actionSteps`).
  The step is ticked only while that covers every current account of the
  kinds the action has steps for; otherwise it stays unticked with "Not
  yet set up for: Claude Code, account Home" under it, and its button
  plans the missing accounts only. A record without accounts (from
  before they were kept) covers none: someone with accounts sees the
  step unticked until those are covered.
- **Accounts added later are covered by themselves.** Confirming a
  `setup: true` action keeps a digest of the steps confirmed and when
  (`extensions.<id>.actionConfirmed`). When an account of a targeted kind
  is added, the extension is turned on, or Pitwall starts, Pitwall runs
  that action by itself for the accounts its record doesn't cover (only
  them, each with its environment) when the extension is on, its run is
  recorded as done, and its steps' digest is the one confirmed. A record
  from before confirmations were kept stands for one with its
  `actionSteps` digest (the steps the user ran), except for a
  development folder's extension. Steps changed since (an update) never
  run by themselves: the step unticks and asks. The run shows on the card
  like a pressed one, under "Set up for Claude Code, account Home, as you
  confirmed on 23 Nov 2026", and its start and end go in the extension's
  activity log. Each account is tried once per session: a failure shows
  (on the card and as the unticked step with Run again) and isn't
  retried until the user runs it. An account counts as covered only when
  a step was done for it. Code: `crate::extensions::actions::auto_setup`.
- Only `setup: true` actions run by themselves; others (installing a
  server, a "remove" action) always need a press. Turning an extension
  off removes nothing it installed.

## A service
*Superseded: the managed `service` block is removed; a supervised program replaces it ([13.2](tile-sdk/services.md#132-a-programs-the-component-starts)).*

A format 2 manifest may declare one `service`: a program, such as the
server its agents talk to, that Pitwall keeps running on this computer
while the extension is on. The author's page is
[`docs/extensions/service.md`](../extensions/service.md). Core:
`pitwall_core::extensions::service`; app: `crate::extensions::service`
(the `Services` entity) and `settings/extensions/service_ui.rs`.

**The manifest.** `label`; `run` (argv, no shell; the program is a bare
name looked up on the login PATH); `install` (the id of one of its
actions); `version` (`run`, `match` with one group, `requires: ">=X"`);
`ready` (`listen`: a loopback address with a port, or `line`: a pattern
its output matches; absent: up after a second); `env` (at most 8,
`value` or `secret`); `firstRun` (`label`, `run`, `opens`, `again`).
Placeholders: `{setting:KEY}` (plain settings only; defaults count) and
`{data}`, the folder `<data folder>/extension-data/<id>`, made mode 700,
outside the installed folder. A secret reaches it only through `env`.
Format 1 ignores `service`.

**Consent.** Nothing starts until the user confirms its commands on the
card ("Pitwall will keep this running while the extension is on:"),
which lists the command, the version command, the first run, the
variables (a secret by name) and the data folder. `ui.json` keeps a
digest of the commands, version check, readiness, variables and first
run (`extensions.<id>.service.confirmed`, with when). The label and the
lowest version aren't part of it, its backup is. While the digest
doesn't match (Reload or a hand edit changed a command), it is stopped
and the card asks again ("An update changed what it runs, so it was
stopped", with the risk). An update's review lists new, changed and
removed services, and pressing Update confirms the new commands when the
old ones were confirmed ("Updating as one step" below).

**Before each start**, off the UI thread, in this order: a setting it
uses isn't set (waits); its address isn't on this computer (not
started); its address is taken, told by binding it, never by
connecting (*running elsewhere*: nothing is started, and nothing is ever
stopped or signalled; checked again every 30 s by binding); its program
isn't on the PATH (*not installed*: the install action is the step's
button); its version is below `requires` (*version too old*: "<label>
0.5.0; this extension needs 0.6.0 or later", and the button runs the
install action, then looks again). A version that can't be read doesn't
block. Then its data folder is made; if it was empty, the service is
*fresh* (`service.fresh`) until its first run succeeds.

**Running.** In its own process group (a job object on Windows), in the
extension's folder, stdin closed, with only PATH (the login one), HOME,
USER, LOGNAME, SHELL, LANG, LC_*, TZ, TMPDIR, XDG_RUNTIME_DIR and
Windows' system variables, plus its `env`. Its stdout and stderr go to
the extension's Log (marked `service`), secrets taken out. It is up when
its `ready.listen` address accepts a connection (it was free before it
started, so it is Pitwall's own), when a `ready.line` line comes, or
after a second.

**Restarts.** A service that exits by itself is started again after 1,
5 and 30 s. The fourth exit within 10 minutes gives up: *stopped*, with
its exit code and last 6 lines, and Start.

**Stopping.** Off, update, remove, a changed command, the user's Stop
(kept in `service.stopped` until Start) and quit: SIGTERM to the group,
then SIGKILL to the group after 3 s; on Windows the job is closed.
Remove waits for that before it deletes anything, and deletes the data
folder only when the user switched that on in the remove card (the
default keeps it, and the card names the folder).

**The first run** runs only on a press, while the service is running:
the checklist step while fresh, then under More, asked again with its
`again` text. With `opens`, the first `https` link in its output is
opened in the browser (`OpenLink`, a test hook, replaces that in tests).
The link is never logged, shown or written: its output isn't logged,
a failure's last lines have every link replaced by `[link]`, and the
card and the activity log say "Opened a one-time sign-in link in your
browser".

**On the card.** The checklist gets the settings the service uses, then
*Install*, *Confirm and start* (the confirm card, then the state:
starting, running since …, stopped, running elsewhere, not installed,
version too old; Start, Stop, Restart) and *first run*, then the other
settings, setup actions and Connected. Its install action isn't listed
under More. "What this extension does on your computer" lists the same
lines as the confirm card, and the permission screen and "Adds to
Pitwall" say it runs a local program continuously. A development folder
follows the same rules.

**Checks.** `pitwall extension check` describes it (it runs
continuously) and checks its program or install action; `test`'s
`service` part checks its commands and the verdict in each case
(installed or not, version output, address taken, settings).

Not built: starting it at login without Pitwall, HTTPS or a public
address, a sandbox, more than one service.

## Setup by an agent
*Superseded in part: the checklist and `extension.act` for setup actions, the service and connections go; reading, install, on, off, update, remove and `extension.secret` (now opening the Setup view at its secure field) stay. An extension's own setup steps for agents are its `act` tools.*

Status: built, unreleased. An agent (the Race Engineer, or any agent the
user asks) sees how an extension stands and moves its setup forward with
`pitwall extension …`, each change approved by the user in Pitwall. Code:
`pitwall_proto::extensions` (the shapes, `next_of`, the text),
`pitwall-daemon/src/extension_setup.rs` (the methods, approvals, one at
a time), `crates/pitwall-app/src/extensions/agent_setup.rs` (the app's
side), `pitwall-cli/src/extension.rs`.
- **Reading** (`extension.list`, `extension.status`, `extension.log`,
  open to any caller, never an extension): the card as data. `status` has
  the checklist (each step `done`, `todo` or `blocked` with why, from the
  same `checklist` the card draws), its connections (plain values; each
  secret only as set or not), the adapter's state and reason, the
  service's (state, both versions when too old, "running elsewhere", the
  first run pending), permissions, accounts a setup action hasn't covered,
  and `next`: one step, with the command for the agent or what only the
  user can do. Nothing is fetched (an update already found is shown).
- **Acting** (`extension.act`): install, on, off, update, remove, setup,
  service confirm|start|stop|restart|update, first-run, set (plain
  settings), connection add|rename|remove. Each asks the user in
  Pitwall's approval dialog, naming the agent that asks and listing
  exactly what will run: an action's command lines per account; a
  service's command, variables and data folder; turning on's permission
  screen (reads granted, nothing that acts) and "what it does"; an
  update's changes. What was shown is what runs: the host prepares it
  first and runs exactly that, and a service whose commands changed since
  (its digest) isn't started. Approved, it runs what the card's button
  runs and the answer says how it went, with the new status; declined,
  nothing runs and the agent is told not to ask again. `set` and
  connection add/rename can be remembered for that agent (low risk); the
  rest is asked every time. One request per extension at a time: another
  while one waits or runs is refused `busy`.
- `extension.checkUpdates` checks a source as "Check for updates" does,
  without asking (it installs nothing).
- **Secrets**: never through the CLI. `extension.secret` only opens the
  Settings tile next to the agent's terminal at the card, the field
  marked ("An agent asked you to set this…"), and answers that it waits
  for the user. Showing Settings off, or no agent to put it beside: the
  answer says where the user enters it.
- **First run**: its one-time link goes from the service to the browser
  only; the approval and the answer say a link is opened, never which.
- **Remove** keeps the service's data (only the user deletes it, in
  Settings).
- Never offered to agents: permissions beyond what turning on grants,
  deleting data, "send without asking", entering a secret. Extensions
  never call any `extension.*` method.

## Updating as one step
*Superseded in part: the plugin and service steps go; an update stops the extension's programs, backs up its data folder, installs and starts the new component ([13.2](tile-sdk/services.md#132-a-programs-the-component-starts)).*

The author's page is [`docs/extensions/updates.md`](../extensions/updates.md).
Core: `pitwall_core::extensions::update` (the plan, backup folders, copy
and restore, generations); app: `crate::extensions::safe_update` (the
run), `crate::extensions::stale` (agents on a previous setup) and
`settings/extensions/{updates,stale_ui}.rs`.

**The manifest.** Format 2 adds `update: { order, agents }` (`order`:
`serviceFirst` | `agentsFirst`, `agents`: `compatible` | `restart`;
unknown keys refused) and `service.backup`: an argv with `{backup}`, or
`{ run, replaces }` (`replaces`: up to 16 plain file names in `{data}`).
Its program is a bare name. The backup is part of the service's digest.

**The run.** Review computes the plan (`update::plan`) from the
installed manifest, the new one and what is so now: on or off, whether
Pitwall runs the service here (confirmed, not stopped by the user, not
*running elsewhere*), whether `{data}` has anything, the setup actions
set up before (`actionSteps` keys), the running agents that started with
the extension on (`Engine::extension_users`) and the restart setting.
Update then runs, live on the card (`extension-update-run-<id>`):

1. `Services::hold(id, true)` stops the service (and keeps it stopped
   across the switch off and on); Pitwall waits for its group. A backup
   folder `<data>/extension-backups/<id>/<YYYYMMDD-HHMMSS>` (UTC, `-2`… on
   a clash, mode 700) gets the backup: the declared command (minimal
   environment, in `{data}`, 10 minutes at most), else a copy of `{data}`
   (at most 2 GiB; bigger needs the review's "Update without a backup"),
   or a copy when the command fails. `pitwall-backup.json` notes the
   versions, the time and how to put it back. Only the newest 3 are
   kept. A failure here releases the service and changes nothing.
2. `catalog::apply`, keeping `.<id>.previous`. A folder changed by hand
   stops here and asks, as before. The new service's commands are
   confirmed (when the old ones were) and `extensions.<id>.lastUpdate`
   is written (versions, time, backup folder, the previous confirmation).
3. With the service on: released, then, if its state is *not installed*
   or *version too old*, its install action runs (through `Runs`, as a
   press would) and it is looked at again.
4. Up within 60 s (`Timeouts`, shorter in tests), then: still up, and
   when the adapter was `ready` before, `ready` again within 30 s; a
   crashed adapter fails it.
5. Each setup action set up before whose steps changed runs for every
   account (and per connection when it uses a setting), its steps
   recorded as confirmed after the run starts. With `agentsFirst` this
   runs between 2 and 3.

A failure in 1–4 after the files changed rolls back: the service held,
`catalog::rollback_one`, the previous confirmation and no `lastUpdate`,
the data put back from the note (`restore_copy`: `{data}` moved into the
backup's `replaced/`, the copy copied back; `restore_files`: the
`replaces` files moved there, the backup's files copied in mode 600),
the service released, and a line for each, with what couldn't be put back
(a server program upgraded by step 3 stays, and the card says what to do).
A failure in 5 ends *SetupFailed* with the accounts and Retry. Each start
and end is in the extension's Activity.

**Go back** with a `lastUpdate` backup that can be put back asks first
(`extension-back-data-<id>`): Keep current data (default) or Put back the
backup too (the service held meanwhile). Go back confirms the previous
commands again and clears `lastUpdate`.

**Agents on a previous setup.** At each start and restart, an agent's
record keeps `extSetup`: for each extension on with setup actions or
agent variables, `update::generation` (FNV-1a over the setup actions'
run keys and the digests of the steps they last ran with, the expanded
agent variables, and the connection). The app sends the extensions that
are on with their `actionSteps` (`Engine::set_extension_setup`, when that
or the accounts change); the engine works out each agent's generation now.
A running agent whose recorded generation for an extension differs from
now carries it in `AgentView.staleFor`; one turned off since isn't
counted. The sidebar row and the pane header show a chip ("old setup",
warning tone with `agents: "restart"`), the card lists them with Restart
now and Restart all idle, and `pitwall agent list` adds a line.

**The restart setting** (`extensions.restartAfterUpdate`, `ui.json`
`extensionsRestartAfterUpdate`; per extension `restartAfterUpdate`):
`ask` (default), `whenIdle`, `never`. `whenIdle` looks every 2 s while
an extension that is on uses it; a stale agent that runs, can restart
and can resume its conversation, and whose status is idle or done, is
restarted once it has been so for 30 s, one at a time (`lifecycle::restart`,
as Restart). Working, blocked, unknown or not resumable: never. A restart
that fails isn't tried again by itself. Each restart is a notice and an
Activity line.

## For authors
*Superseded in part: the adapter's log, Reload, the Node starter and `test`'s adapter parts go with adapters.*

The author's guide is [`docs/extensions/`](../extensions/README.md).
Code: `crate::extensions::{load_dev, reload_dev, forget_dev}`,
`settings/extensions_dev.rs`, `pitwall_core::extensions::{check, log,
plan, process, starter}`, `pitwall_core::platform::group`.
- **Load from folder…** (Settings → Extensions, beside "Look again")
  picks a folder with an `extension.json` and records it in `ui.json`
  (`extensions.<id>.devPath`). It is read where it is: never copied,
  changed, updated or deleted by Pitwall. Loading it turns that id
  **off**: nothing runs until the user turns it on. A folder whose id is
  already loaded from another folder is refused, saying so; one whose
  folder or id changed later shows as a problem card with why.
- Its card is marked **development** (the folder in its tooltip, and a
  line under its name: "loaded from …, never copied or updated"). It has
  no Update, no Go back, no source checks and no Remove (Add from Git
  refuses its id too); it has **Reload** (its manifest read again
  and, while it is on, its adapter started again) and **Remove from
  list** (turned off and no longer listed; its settings and secrets
  stay, its folder is untouched, so it asks nothing).
- **Log** (any card with an adapter): the last 500 lines while it runs,
  both ways, its stderr and Pitwall's own notes (started, stopped and
  why), each with its time, newest at the bottom, with Copy. Secrets
  are taken out before a line is kept: the `secrets` of the hello, the
  values of its secret settings wherever they appear, and anything
  shaped like an API key or a bearer token. Lines are cut at 2000
  characters. It is kept across restarts of the adapter and cleared when
  the extension is turned off. In memory only.
- **Process groups:** every adapter, and every program `check`/`test`
  starts, is started in its own process group (Unix: `setpgid`; Windows:
  a job object that kills its processes when closed) and the whole
  group is killed when it is stopped. OS code is in
  `pitwall_core::platform::group`.
- **`pitwall extension check <folder>`**, offline (no app): the manifest
  parses and is within the limits, its version, the adapter's program
  is found (PATH, as Pitwall would start it), the files its command
  names are there and inside the folder, a README and the fixture; then
  what it would run and set, in plain words: its settings, its adapter's
  command, the variables it sets on agents, how threads match agents,
  each action's commands. A folder name that differs from the id is a
  note, not a failure.
- **`pitwall extension test <folder>`** runs what `test/pitwall.json`
  expects, each part only when there:
  - `actions`: each action's exact command lines, with the programs
    taken as installed;
  - `env`: the variables Settings shows with no settings (`unset`) and
    with `values` set (`set`);
  - `run`: the adapter started with `settings` and `secrets`; it answers
    hello with protocol 1, its status (default `ready`), at least
    `threads` threads, thread `open` with its messages, and `reply`
    answered ok;
  - `agreement`: a fake server of the folder's own (`fake`, a command;
    its first stdout line is JSON with its `thread` and the values
    `{field}` in `settings` and `secrets` stand for) the real adapter is
    pointed at: it connects, lists that thread with its blocking ask,
    opens it, and carries `answer` back.
  Every adapter must exit when its stdin closes. A part whose program
  (such as Node) isn't installed is skipped, not failed. JSON by default,
  plain words with `--human`; exit status 1 when a check fails.
- **`pitwall extension new <folder> [--name …]`** writes a starter into
  a new or empty folder (its id is the folder's name): `extension.json`,
  `adapter.mjs` (Node, no dependencies, protocol 1, one inbox thread that
  answers a reply), `README.md` and `test/pitwall.json`. It passes
  `check` and `test` as written (Pitwall's tests make sure).
  `--protocol 2` writes a format 2 manifest asking for `agents.read` and
  an adapter that lists the agents through calls and events, with a
  `calls` fixture (a fake Pitwall side: `grants`, `answers`, `events`,
  `expect`).

## Later (not built)
- Pitwall registering an extension's MCP server in agents itself, rather
  than a setup action running the agent tools' own commands.
- In the protocol: an ask's priority and answer-by time, references,
  topics, and replaced messages (an adapter leaves replaced ones out).
- A badge count on the Dock / taskbar from extensions.
- Marking a thread unread (the protocol has only "mark read"), and a
  list of archived threads.
- Watching a development folder for changes (Reload is by hand).
- View components beyond list, detail, Markdown and status (forms, tables).
- Floating-control cards from an extension's view items.
