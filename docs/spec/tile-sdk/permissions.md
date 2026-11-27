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
| `files.read` | Read files in the project each of its views is open in (not all projects; as built, [14.5](platform-services.md#145-files-in-projects-as-built)) | on | Medium: source, `.env` files |
| `files.write` | Write in those projects; deletes go to the Trash | off | High: can change code agents and the user run |
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
  those promptly ([10.3](README.md#103-decisions), item 5). Tiles get
  no folders through WASI, but the same platform needs its own tests
  of Pitwall's path checks ([10.3](README.md#103-decisions), item 18).
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

### 5.5 Threat model and outside review
[new] Before Browse opens to the public, two things are done: a
threat model document, and a security review of the host and
permission code by someone outside the project. Both are an exit
condition of phase 5 ([10.2](README.md#102-order-of-work), decided in
[10.3](README.md#103-decisions) item 17). Until then Browse stays
limited to SDK-preview authors, and tiles travel as preview folders
([6.3](packaging.md#63-install)) or unlisted packages.

**The threat model document** is written once packages and the index
exist (phases 4–5), and kept with this design. For each threat it
says what a malicious tile can try, what stops it today, what is
left, and how a test shows it. The outline:

| Threat | What a malicious tile tries | What stops it (this design) | Left to check |
|---|---|---|---|
| Escape | Break out of the sandbox through a Wasmtime bug, a host-API bug, a crafted package (paths, symlinks, zip bombs) or a tampered compile cache file | Empty WASI context and linked stubs ([5.2](#52-enforcement)); package rules ([6.1](packaging.md#61-the-pwtile-file)); the cache is written only by Pitwall, in a private folder ([2.6](runtime.md#26-compile-and-cache)); prompt Wasmtime security updates ([10.3](README.md#103-decisions) item 5) | Host-API input handling; path checks on every OS, Windows included; the in-process blast radius ([10.3](README.md#103-decisions) item 1) |
| Resource exhaustion | Burn CPU, memory, host calls, storage or the main thread; a tree or a wasm file made to be slow to diff or to compile | Epoch deadlines, memory and tree limits, host-call counters, storage quotas ([2.5](runtime.md#25-sandbox-and-limits-summary-details-in-5)); suspension while hidden; the [phase 0 gates](README.md#phase-0-gates) | Compile time of hostile wasm; limits under many tiles at once |
| Data exfiltration via `net` | Send what it may read to a granted host, a host the user approves through `net.ask`, or through redirects, DNS tricks or IDNA look-alikes | Host checks after IDNA normalisation and before DNS, redirects kept inside the allowed hosts ([5.2](#52-enforcement)); hosts shown as written ([5.1](#51-new-permissions-new)) | What can still leave through a granted host ([5.3](#53-what-a-hostile-tile-can-still-do)); how clear the `net.ask` prompt is |
| Phishing UI | Look like Pitwall's settings, an approval or a sign-in form; misleading toolbar items, popovers, notifications or toasts | Host-drawn header with the extension's name; no dialogs or drawing over other tiles ([5.2](#52-enforcement)); secrets entered only in Settings; toolbar items drawn by Pitwall ([3.9](ui.md#39-toolbar-items)) | Every surface a tile's text reaches, and whether each names the tile |
| Prompt injection through agent tools | Misleading `read` or `present` results, queued text to agents, an agent skill in the package that tells agents to do harm | Tools labelled with the tile's name, `act` approved each time ([11.5](agent-tools.md#115-permissions)); queued text marked and waiting for Send ([5.3](#53-what-a-hostile-tile-can-still-do)); skills through the rules path, per project ([11.11](agent-tools.md#1111-a-skill-for-agents)) | What reaches the agent unmarked; the skill review in Verified |
| Malicious updates | A stolen author key, a compromised release or a takeover that ships harmful code with the same permissions | Signature and hash checks, key changes shown, new permissions off until Allow, auto-install only when nothing in the permission list changes ([6.4](packaging.md#64-updates-and-go-back)); the signed block list ([7.6](marketplace.md#76-takedown-and-yanking)); two index keys ([6.2](packaging.md#62-signing)) | Same-permission updates installed automatically; how fast a block reaches users |

**The outside review** covers `pitwall-tile-host` (linker, limits,
epoch ticker, cache loading), the host-API implementations (`net`,
paths, storage, `sql`), the package reader and signature checks
(`extensions::package`), the permission code (`permissions.rs`, the
permission screen, grants), the agent tool gate (`tile.call`,
approvals) and the index and block-list checks. Its serious findings
are fixed before Browse opens; the rest are listed with dates. The
owner picks the reviewer and decides whether later changes to the
sandbox (`net.tcp`, the helper process, headless services) need
another review.
