# Tile SDK: permissions

Part of the [Tile SDK and marketplace](README.md) design. Permissions and the security model. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 5. Permissions and security model

The existing model [today] stays: named permissions with the author's
reason, granted per extension on the permission screen, kept in
`ui.json` with dates, revocable at any time, applied to the next call,
recorded in the activity log. Tiles add enforcement: unlike an
adapter, a tile's code *can't* do more than it is given.

### 5.1 New permissions [new]

| Permission | Gives | Starts | Risk |
|---|---|---|---|
| `net` (`hosts` list) | HTTPS to exactly those hosts (exact name or `*.example.com`; no IP literals unless loopback); `http://` only to `localhost`, `127.0.0.1`, `[::1]` | on | Medium: data the tile has can leave to those hosts. The list is shown as written. |
| `net.ask` | HTTPS to hosts the user approves at run time, one at a time ("API client wants to connect to api.example.com — Allow once / Always / Deny") | on (each host asks) | Medium; for tools whose hosts are the user's choice (an API client) |
| `files.read` | Read files in projects the user picks for this tile (not all projects) | on (asks which projects) | Medium: source, `.env` files |
| `files.write` | Write in those projects | off | High: can change code agents and the user run |
| `git.read` | Status, diffs, history of granted projects | on | Medium |
| `git.write` | Stage, commit, checkout | off | High |
| `git.remote` | Fetch, pull, push with the user's git credentials | off, asks each push | High: publishes work |
| `clipboard.read` | Read the clipboard | off | Medium |
| `memory` (a number, MiB) | Linear memory above the 64 MiB default | shown, not a switch | Low |

The existing ones (`agents.read`, `projects.read`, `worktrees.read`,
`profiles.read`, `show`, `agents.focus`, `agents.queue`,
`agents.start`, `notify`) keep their meaning, defaults, risk notes and
limits [today].

What agents may do with a tile's tools is a separate grant, shown in
its own part of the permission screen ([11.5](agent-tools.md#115-permissions)). A tile's tools can never
do more than the tile's own grants allow.

Manifest form: a permission's value is the reason string, as today, or
an object with the reason and parameters (`net`'s `hosts`, `memory`'s
size). A manifest with an object value needs a Pitwall that knows
tiles, which the `tiles` contribution already guarantees (an older
Pitwall refuses unknown `contributes` keys [today]).

### 5.2 Enforcement
- **No ambient authority.** The component can only call what is in the
  linker. WASI is built from an empty context (S7). Imports a tile
  declares but isn't granted are linked to stubs that return
  `denied`, so it loads and degrades instead of failing.
- **Checks at the boundary.** URLs are parsed by the host, the host
  name compared after IDNA normalisation and before DNS; redirects are
  followed by the host only within the allowed hosts; the TLS stack is
  Pitwall's (rustls [today, via ureq]). Project paths are resolved
  against the granted roots with symlinks followed and refused if they
  leave the root.
- **What it gets is filtered** exactly as adapters' calls [today].
- **Its UI is labelled.** The tile's header shows the extension's icon
  and name, always drawn by the host, so a tile can't pass as
  Pitwall's own settings or an approval. Tiles can't draw over other
  tiles or the chrome, and can't open dialogs; the few host dialogs a
  tile triggers (pick a project, allow a host) name it.

### 5.3 What a hostile tile can still do
Honestly:
- Use what it was granted, for anything: send project files it may read
  to a host it may reach; queue misleading text to agents if
  `agents.queue` is on (still lands in Next up, marked, waiting for
  Send by default [today]).
- Show misleading content inside its own tile (a fake "sign in" form
  asking for a password). Mitigations: the host-drawn header, secrets
  entered only in Settings [today], and Verified review.
- Burn CPU up to its deadlines while shown, and memory up to its limit
  (one tile's worker is one core at most).
- Exploit a bug in Wasmtime or in Pitwall's host code to escape the
  sandbox. This is not theoretical: on 19 November 2026 Wasmtime
  shipped patch releases across its supported lines for a WASI
  filesystem sandbox escape on Windows (S1). Wasmtime backports
  security fixes to every supported release (S1), and Pitwall must ship
  those promptly ([10.3](README.md#103-decisions), item 5).
  Running tiles in a helper process later reduces the blast radius.
- Fingerprint: theme, window size, timing.
- Through its tools ([section 11](agent-tools.md#11-tools-for-agents)): answer an agent's `read` with
  misleading text, which the agent may act on. Tool results reach the
  agent as tool output, as any MCP server's do; the agent and its own
  permissions decide what it does with them. Pitwall labels every tool
  with the tile's name, and `act` calls are approved by the user.
- The other way round, an agent (possibly carrying injected
  instructions from a web page or a file) can send a tile hostile
  input. The tile's sandbox and grants bound the damage; `act` still
  asks the user.

What it can't: read files or reach hosts it wasn't given, start
programs, read other extensions' data or secrets, read agents'
terminals, keep running after it is turned off, steal focus from a
terminal, or crash Pitwall short of a Wasmtime bug (a trap ends only
the tile).

### 5.4 Kill switches and revocation
- **Per tile:** Stop in the tile's menu and on the card; turning the
  extension off.
- **All third-party tiles:** "Run tiles from others" switch in Settings
  → Extensions (on by default once installed; off stops them all and
  shows them as paused).
- **Remote:** the index's signed `blocked.json` ([7.6](marketplace.md#76-takedown-and-yanking)), checked at start
  and daily. A blocked id/version/hash is turned off with the reason
  shown; the user can't turn it back on without removing and
  reinstalling a non-blocked version. Fetch failures change nothing.
- **Grants:** each permission on the card with Granted/Refused, date
  and a switch [today]; a revoke reaches the tile as
  `grants-changed` and its next call is denied; `files.*` and
  `net.ask` grants list each project or host with Remove.
- **Activity:** every call that acted, every denial, every stop with
  its reason, kept 30 days [today].
