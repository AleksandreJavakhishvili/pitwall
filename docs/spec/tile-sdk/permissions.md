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
| `net` (`hosts` list) | HTTPS to exactly those hosts (exact name or `*.example.com`; no IP literals unless loopback; `{setting:KEY}` for the host of a `url` setting's value, following it); `http://` only to `localhost`, `127.0.0.1`, `[::1]` | on | Medium: data the tile has can leave to those hosts. The list is shown as written. |
| `net.ask` | HTTPS to hosts the user approves at run time, one at a time ("API client wants to connect to api.example.com — Allow once / Always / Deny") | on (each host asks) | Medium; for tools whose hosts are the user's choice (an API client) |
| `files.read` | Read files in the project each of its views is open in (not all projects; as built, [14.5](platform-services.md#145-files-in-projects-as-built)) | on | Medium: source, `.env` files |
| `files.write` | Write in those projects; deletes go to the Trash | off | High: can change code agents and the user run |
| `git.read` | Status, diffs, history of granted projects | on | Medium |
| `git.write` | Stage, commit, checkout | off | High |
| `git.remote` | Fetch, pull, push with the user's git credentials | off, asks each push | High: publishes work |
| `clipboard.read` | Read the clipboard | off | Medium |
| `memory` (a number, MiB) | Linear memory above the 64 MiB default | shown, not a switch | Low |
| `programs` (`commands`, `downloads` patterns) [new, 2026-11-28] | Ask Pitwall to run native commands matching the patterns, once or kept running, each exact command line confirmed by the user the first time; download files pinned by SHA-256 into its data folder ([13.2](services.md#132-a-programs-the-component-starts)) | off | High: a program runs as the user, outside the sandbox. The patterns are shown as written, and "Runs programs on your computer" on install, the card and Browse |
| `agents.env` (`names` list) [new, 2026-11-28] | Set environment variables with those names on agents Pitwall starts from now on, with `{project}` and `{agent_id}` filled in per agent ([14.6](platform-services.md#146-settings-secrets-and-the-setup-view)) | off | Medium: the values reach every agent it starts; the names and current values are shown on the page, secrets as "(secret)" |
| `shortcut` | Keys that open one of its tiles from anywhere in Pitwall (`contributes.tiles[].shortcut`; `cmd-` or `cmd-shift-` and a letter, digit or F-key, never Pitwall's own; the user may change them in its card) [built] | off | Low: its keys open its tile |

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
- Show misleading content inside its own tile or Setup view (a fake
  "sign in" form asking for a password). Mitigations: the host-drawn
  header, secrets entered only in the host-drawn secure field
  ([3.12](ui.md#312-the-setup-view-and-host-drawn-fields)), which looks the same everywhere and that tiles can't imitate
  without the lock mark and the "Saved in your keychain" line, and
  Verified review.
- With `programs`: whatever a granted command can do, as the user, for
  each line the user allowed. A pattern with `**` after a program that
  takes a script or a plugin (`claude plugin **`) is broad; the
  confirmation shows each exact line, not just the pattern.
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
programs other than its granted patterns' (each line confirmed), read
a secret's value (its own included), read other extensions' data or secrets, read agents'
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
| Phishing UI | Look like Pitwall's settings, an approval or a sign-in form; misleading toolbar items, popovers, notifications or toasts | Host-drawn header with the extension's name; no dialogs or drawing over other tiles ([5.2](#52-enforcement)); secrets entered only in the host-drawn secure field, which the Setup view places but can't draw ([3.12](ui.md#312-the-setup-view-and-host-drawn-fields)); toolbar items drawn by Pitwall ([3.9](ui.md#39-toolbar-items)) | Every surface a tile's text reaches, and whether each names the tile; a Setup view drawing a look-alike of the secure field |
| Secret theft | Read a secret the user entered, or send it somewhere other than where it is meant to go | The component never gets a secret's value ([14.6](platform-services.md#146-settings-secrets-and-the-setup-view)); the host puts it into a `net` header only for a request to a host the `net` grant allows, and into a program's environment only for a confirmed line that names it; secrets taken out of Log lines and program output returned to the component | A granted host or program that echoes the secret back (the component would then see it); a program that writes it to disk |
| Programs | Run something the user didn't mean to allow: a broad pattern, an argument that changes meaning (`--config=evil`), a program found earlier on PATH, a setting changed to alter a command, a downloaded binary swapped | Declared patterns, refused without asking when they don't match ([13.2](services.md#132-a-programs-the-component-starts)); each exact line, with the program's resolved path, confirmed once and asked again when it changes; shells and interpreters with `**` refused by `check`; no `sh -c`; downloads only from declared URL patterns and checked against the SHA-256 the component pins; settings used in a pattern changed only through a host-drawn setting field; `programs` off until Allow; Verified covers the patterns | Everything a confirmed program does after it starts (it isn't sandboxed); users allowing "Always" without reading; PATH changes after a line was allowed (the resolved path is part of the line, so a different path asks again) |
| Agent environment | Put a variable on agents that changes how they behave (`NODE_OPTIONS`, `LD_PRELOAD`, a proxy, a different API base) | `agents.env` names the variables in the manifest; names Pitwall or the agent uses for its own setup and loader/runtime variables (`PATH`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `*_PROXY`, `PITWALL_*`, an agent kind's own API keys and base URLs) are refused by `check` and the host; values shown on the page; off until Allow | A variable its agent's plugin trusts (by design: that is why it exists) |
| Prompt injection through agent tools | Misleading `read` or `present` results, queued text to agents, an agent skill in the package that tells agents to do harm | Tools labelled with the tile's name, `act` approved each time ([11.5](agent-tools.md#115-permissions)); queued text marked and waiting for Send ([5.3](#53-what-a-hostile-tile-can-still-do)); skills through the rules path, per project ([11.11](agent-tools.md#1111-a-skill-for-agents)) | What reaches the agent unmarked; the skill review in Verified |
| Malicious updates | A stolen author key, a compromised release or a takeover that ships harmful code with the same permissions | Signature and hash checks, key changes shown, new permissions off until Allow, auto-install only when nothing in the permission list changes ([6.4](packaging.md#64-updates-and-go-back)); the signed block list ([7.6](marketplace.md#76-takedown-and-yanking)); two index keys ([6.2](packaging.md#62-signing)) | Same-permission updates installed automatically; how fast a block reaches users |

**The outside review** covers `pitwall-tile-host` (linker, limits,
epoch ticker, cache loading), the host-API implementations (`net`,
paths, storage, `sql`), the package reader and signature checks
(`extensions::package`), the permission code (`permissions.rs`, the
permission screen, grants), the program capability (pattern
matching, the confirmation, PATH resolution, environment and secret
substitution, the archive reader of `download`), secret substitution in
`net`, the host-drawn fields, the agent tool gate (`tile.call`,
approvals) and the index and block-list checks. Its serious findings
are fixed before Browse opens; the rest are listed with dates. The
owner picks the reviewer and decides whether later changes to the
sandbox (`net.tcp`, the helper process, headless services) need
another review.

#### 5.5.1 The marketplace (phase 5)

Written with the index ([7.8](marketplace.md#78-as-built-phase-5)).
Each row says what an attacker tries, what stops it as built, what is
left, and which test shows it. Tests are in
`pitwall-core/src/extensions/index/tests.rs` (core) and
`crates/pitwall-index/tests/index.rs` (tool).

| Threat | What an attacker tries | What stops it (as built) | Left | Test |
|---|---|---|---|---|
| Index compromise: the repository or CI | Merge a malicious entry or change a listed version's file, through a stolen maintainer account, a malicious pull request that runs code in CI, or a poisoned workflow | Branch protection and required checks (owner's checklist); `check.yml` runs with a read-only token, no secrets and `persist-credentials: false`, and never runs the package as a program (only the tile runtime, with no grants); `auto-merge.yml` runs in the base repository's context and reads only the check's result file; listed versions can't be changed, only yanked (`judge`); every package must be signed by the author's key, not the index's | A maintainer who is malicious or compromised can still merge; the pinned `pitwall-index` commit must be bumped with care; `cargo install` from git trusts the Pitwall repository | `a_pull_request_is_judged_against_its_base`, `an_update_by_its_author_with_the_same_permissions_can_be_merged_by_the_bot` |
| Index compromise: the index key | Sign an `index.json` that lists a malicious version, hides a block, or marks a version Verified | Packages are pinned by SHA-256 and must be signed by an author key the index lists, so a new listing needs the attacker's own author key; Verified is checked against review keys that ship in Pitwall, not the index, so a stolen index key can't make a version Verified; the key's secret is only in a protected environment of `publish.yml` | With the index key alone, an attacker can list a new extension under their own key, or drop a block for old Pitwalls; the key isn't in a hardware token | `an_index_signed_by_another_key_is_refused`, `browse_offers_the_last_verified_version_by_default` |
| Index server or network | Serve a changed file, an old but validly signed index (replay or freeze), a huge file, or garbage | Signature checked before anything is kept; a file older than the cached one is refused; strict size and count limits; text-direction characters refused; https only; an index older than 14 days is marked stale; when a fetch fails, nothing changes | Browse doesn't show "stale" yet (Settings side); a first-time user with no cache can be frozen at whatever old index they get first, for up to 14 days | `the_https_source_caches_checks_and_falls_back`, `parsing_holds_the_limits`, `times_read_and_write` |
| Key rotation: the index key | A compromised or lost index key strands Pitwalls, or a rotation is abused | Two keys ship (current and next); to rotate, CI switches to the next key and a Pitwall release adds a new next key and drops the old one | Pitwalls that never update keep trusting a compromised key; there is no signed revocation of an index key; the procedure needs writing down and rehearsing | `an_index_signed_by_another_key_is_refused` |
| Key rotation: an author's key | A stolen author key ships a malicious update; an attacker swaps in their own key | A key change in `authors/` needs a maintainer (`judge`); a new version signed by another key than the one before it is sensitive and isn't auto-merged; installed copies keep trusting their first key (packaging 6.6) and ask before trusting another | A stolen key that is still listed can sign an update with the same permissions, which the bot auto-merges; rotation statements signed by the old key aren't read | `a_new_key_on_a_version_is_sensitive`, `into_a_clone_of_the_index_an_update_adds_a_version_that_its_checks_accept` |
| Typosquatting and impersonation | List `ci-statu5` next to `ci-status`, or use another project's name or icon | A new id always needs a maintainer; look-alike ids (one edit apart, or the same once `-` and look-alike characters are folded) are flagged; ids belong to their first author; names are plain text without direction characters; the card shows the author's GitHub login and the repository | Names (not ids) aren't compared for look-alikes; nothing compares icons or screenshots; Unicode look-alikes in names are allowed (ids are ASCII) | `look_alikes_catch_typosquats`, `a_pull_request_is_judged_against_its_base` |
| Malicious update | Ship harmful code in an update with the same permissions, or widen permissions quietly | Updates with a permission, host, memory or key change need a maintainer; new permissions stay off until Allow (6.4); a newer version isn't Verified until reviewed, and Browse offers the last Verified one; each new version must start headless; blocked versions are refused and turned off with the reason | Same-permission updates are auto-merged and, with auto-install, installed without review; how fast a block reaches users depends on the daily check (not yet wired in the app) | `a_new_version_keeps_what_was_recorded_and_says_what_it_changes`, `yanked_and_blocked_versions_are_refused_with_the_reason` |
| Takedown abuse | Block someone else's extension, or un-yank a withdrawn version | `blocked.toml` and un-yanking need a maintainer; blocks are signed with the index | A compromised index key can block anything (denial of service) | `a_pull_request_is_judged_against_its_base` |

**What the outside review must also cover, for the marketplace:** the
index parsers and their limits (`extensions::index::{parse_index,
parse_blocked}`), the signature checks with pinned keys (`verify_by`,
`HttpsSource`'s cache, its replay check and its fallback), the review
statement and its check, `refusal` and `installed_note`, the
repository checks and the pull-request judge (`index::repo::{load,
judge, build}`), `pitwall-index` (downloads, the headless run, signing),
the three workflows (tokens, `workflow_run`, artifacts, the secret's
environment), and the key handling in the owner's checklist.

**Status: the outside review is not done.** It has not started; the
owner picks the reviewer. Browse stays limited to SDK-preview authors
until it is done and its serious findings are fixed.
