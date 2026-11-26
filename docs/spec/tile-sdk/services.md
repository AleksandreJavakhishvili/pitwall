# Tile SDK: tile apps with a server

Part of the [Tile SDK and marketplace](README.md) design. A tile plus a managed service, and headless wasm services. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 13. Tile apps with a server

### 13.1 Why, and which first
A tile is drawn, suspended when hidden and dropped after ten minutes
([2.3](runtime.md#23-lifecycle)). That is right for a view, and wrong for a program that must keep
data, answer other programs, or work while nobody looks at it: a
team's shared board, a local API the user's agents call, a sync job.
So a tile can't itself be a server. Its package can bring one, in one
of two ways:

| | A. Tile + managed service | B. Headless wasm service |
|---|---|---|
| The server is | Any program, any language | A `service.wasm` component written with `pitwall-extension` |
| Runs | As the user, unsandboxed [today] | In Wasmtime, with only what is granted |
| Runs where | A process Pitwall starts, while the extension is on and Pitwall runs [today] | A small host process of Pitwall's, which keeps running when the window closes |
| Exists | Yes: `service` in format 2 manifests ([service.md](../../extensions/service.md)) | No: new |
| Trust shown | "Runs a program on your computer" | Same as a tile: its grants |

**Decided (owner, 2026-11-26): A first, B later.** A works with machinery that is
built and tested (install action, version check, ready check, data
folder, backup, one-step update and Go back). It only needs the tile
side wired up ([13.2](#132-a-a-tile-plus-a-managed-service)), so it can ship with tiles in phase 2. B is worth
building once tiles are stable (after phase 7), because it brings the
sandbox to servers and needs no runtime on the user's machine; it is
designed in [13.3](#133-b-a-headless-wasm-service) so A's manifest and the tile's view of it don't change
when an author moves from A to B.

### 13.2 A. A tile plus a managed service

**Manifest.** The existing `service` block, beside `contributes.tiles`:
```json
{
  "v": 2,
  "id": "team-board",
  "name": "Team board",
  "version": "1.0.0",
  "pitwall": ">=0.15.0",
  "settings": [
    { "key": "listen", "type": "text", "label": "Listens on", "default": "127.0.0.1:8790" }
  ],
  "actions": [
    { "id": "server", "label": "Install the board server",
      "steps": [{ "run": ["cargo", "install", "team-board-server", "--version", "1.0.0", "--locked"] }] }
  ],
  "service": {
    "label": "The board server",
    "run": ["team-board-server", "--data", "{data}", "--listen", "{setting:listen}"],
    "install": "server",
    "version": { "run": ["team-board-server", "--version"], "match": "(\\d+\\.\\d+\\.\\d+)", "requires": ">=1.0.0" },
    "ready": { "listen": "{setting:listen}" },
    "backup": { "run": ["team-board-server", "backup", "--data", "{data}", "--to", "{backup}/board.db"],
                "replaces": ["board.db"] }
  },
  "contributes": {
    "tiles": [{ "id": "board", "title": "Team board", "icon": "list", "wasm": "tile.wasm" }]
  }
}
```
Everything about the service is as [service.md](../../extensions/service.md)
and [updates.md](../../extensions/updates.md) say [today]: the program is
a name on the user's PATH, installed by one of the extension's actions;
its exact commands are confirmed once; `{data}` is private and outside
the installed folder; restarts at 1, 5 and 30 s; `ready.listen` must be
on this computer.

**What is new: the tile reaches its own service.**
- **An automatic loopback grant.** Pitwall gives the tile `net` access
  to exactly its service's `ready.listen` address (scheme `http`, that
  host and port, nothing else on loopback). It isn't a switch: the
  service is part of the same extension and was confirmed with it. The
  permission screen lists it as "Talks to its own server at
  127.0.0.1:8790". When the address comes from a setting, the grant
  follows the setting's current value.
- **A host interface, `service`**, in the tile's world (no permission;
  own service only):
  ```wit
  interface service {
    enum state { off, needs-install, starting, up, stopped, failed }
    record info { state: state, address: option<string>, version: option<string>, why: option<string> }
    info: func() -> info;
    restart: func() -> u32;        // a task; same as the card's Restart
    open-card: func();             // shows the card (install, first run)
  }
  ```
  plus an event `service-changed(info)`. The SDK wraps it as
  `service::info()` and `service::url("/api/items")`.
- **The tile and the service live apart.** Hiding or dropping the tile
  doesn't stop the service; turning the extension off stops both
  [today]. A tile shows `needs-install` or `stopped` with a button that
  opens the card, where the install action and first run already live.
- **Agents reach it** through the tile's tools ([section 11](agent-tools.md#11-tools-for-agents)): a `read`
  or `act` tool that calls the service. Agents run as the user and can
  also reach a loopback port themselves (`curl`); Pitwall can't stop
  that and doesn't pretend to. A service that wants a key from its
  clients declares it with `env` and a secret setting [today]; the tile
  reads that secret through `settings.secret` (its own extension's).
- **Updates** run today's ordered steps (back up, install, update the
  server's program, start and check, set up the agents), with the tile's
  "instantiates and `init` returns ok" check ([6.4](packaging.md#64-updates-and-go-back)) added to step 4.

**Trust.** The package now runs a native program as the user, so:
- the install and permission screens say **"Runs a program on your
  computer"**, with the command lines (today's "What this extension does
  on your computer" disclosure), and the confirm card stays;
- Browse shows the same words as a mark on the card and lets "Verified
  only" users filter on it;
- Verified ([7.3](marketplace.md#73-verified)) covers the server too: its source at the version the
  install action pins, and the install action itself. A server installed
  from a binary nobody can read is never Verified.

What a hostile service can do is what an adapter can [today]
([extension-api.md](../extension-api.md), "What a hostile extension can
still do"): everything the user can. The tile beside it stays sandboxed,
but that protects little when its server isn't.

### 13.3 B. A headless wasm service

**What it is.** A second component in the package, `service.wasm`,
built from the same crate (`pitwall-extension` with the `service` feature).
It has no UI. Pitwall runs it while the extension is on, whether or not
a tile shows it, and whether or not Pitwall's window is open.

```json
"service": {
  "label": "The board server",
  "wasm": "service.wasm",
  "world": "pitwall:tile/service@0.1.0",
  "listen": { "port": 8790 },
  "limits": { "memory": 256, "request": 30 }
}
```
A `service` with `wasm` has no `run`, `install` or `version`: the
program is in the package. Its address, its data folder and how it is
backed up and updated keep the meaning they have for a managed service,
so a tile written against [13.2](#132-a-a-tile-plus-a-managed-service) works unchanged.

**Where it runs.** Today Pitwall's daemon (the engine, the socket
server, approvals) runs inside the app's process: `pitwall-app` hosts
it (`crates/pitwall-app/src/host.rs`); a separate `pitwalld` is
designed in [architecture.md](../architecture.md) but not built, and only
the terminal holders (`pitwall-hold`) outlive the app. So "keeps running
when the app closes" needs a process of its own. Decided ([13.6](#136-decisions), item 2): a small
binary, `pitwall-svc`, one process per running wasm service, built like
the holders: started by Pitwall, its own process group, a socket at
`<data>/run/svc/<id>.sock` for status, logs and stop, found again when
Pitwall starts, and stopped when the extension is turned off. It links
`pitwall-tile-host` and nothing of GPUI. A crash or a Wasmtime bug then
takes down one service, not Pitwall. When a standalone daemon exists,
services could move into it; the socket protocol stays.

**How requests arrive: `wasi:http/incoming-handler`.** The host runs
the HTTP server; the component only answers requests. Wasmtime supports
this directly: an embedder adds the `wasi:http` interfaces to a linker,
pre-instantiates the component once (`ProxyPre`, which "makes
per-request instantiation much quicker"), and for each request creates
a fresh `Store`, turns the hyper request into a guest one and calls the
component's `handle`; Wasmtime's own example serves with hyper over
HTTP/1.1 only (S21). This is also what `wasmtime serve` does (S21).
Consequences:
- No socket code in the guest, no accept loop, no TLS, no port
  binding by the component: Pitwall binds the address it granted.
- A fresh instance per request: memory isn't shared between requests,
  so state lives in storage (below), not in globals. Concurrency comes
  from running requests in parallel instances (≤ 16 at a time by
  default), not from threads: `wasm32-wasip2` has no `std::thread`, and
  `wasm32-wasip3` doesn't support it yet either (S16).
- WASI 0.3 brings `wasi:http/handler@0.3` with native async; Wasmtime's
  p3 HTTP support is still called experimental (S15, S21). WIT
  `service@0.1` uses the 0.2 `incoming-handler`; a later world moves.
- The alternative, a long-lived instance that listens itself through
  `wasi:sockets` (TCP bind, listen, accept), is closer to an ordinary
  server and would allow in-memory state and other protocols. Wasmtime
  implements `wasi:sockets` with every address denied unless allowed
  (S7); whether its TCP `listen` path works for this is *unverified*,
  and the docs disagree about whether TCP is on by default (S7). Not in
  `service@0.1`; decided in [13.6](#136-decisions) (3).

**The service world.**
```wit
world service {
  include wasi:cli/imports@0.2.0;   // clocks, random, /data, logs
  import storage;                   // key-value, as tiles (3.8)
  import sql;                       // SQLite run by the host (below)
  import net;                       // outgoing HTTPS to named hosts, as tiles
  import tasks;                     // timers
  import settings;                  // own settings and secrets
  import pitwall;                   // the method table, by grants (3.8)
  import log;
  export wasi:http/incoming-handler@0.2.0;
  export lifecycle;                 // start, timer, migrate, stop
}

interface lifecycle {
  start: func() -> result<_, string>;          // once per process start, ≤ 10 s
  timer: func(id: u32);                        // background work, ≤ 60 s each
  migrate: func(from: string) -> result<_, string>;   // after an update, ≤ 5 min
  stop: func();                                // ≤ 5 s
}
```

**Data: SQLite run by the host.** SQLite compiled into the guest is
possible but not settled: rusqlite's own wasm support targets
`wasm32-unknown-unknown`, not WASI; third-party builds for
`wasm32-wasip2` exist, and one notes that Wasmtime lacks the `flock`
SQLite's locking uses (S22, *unverified in practice*). With parallel
instances writing one file, missing locks mean corruption. So the
service world has a `sql` interface: the host opens
`{data}/<name>.sqlite` with native SQLite (one connection pool per
service, WAL), and the guest sends statements and parameters and gets
rows back. Transactions are a resource the guest opens and commits.
Size ≤ the storage quota; the files are ordinary SQLite, so backups and
Go back treat them like any `replaces` file [today]. A guest that
brings its own SQLite on a single instance is allowed (`/data` is a
normal folder) but not recommended.

**Permissions** [new], on the same screen as the tile's:

| Permission | Gives | Starts | Risk |
|---|---|---|---|
| `net.listen` | Pitwall serves HTTP for the service on `127.0.0.1` (and `[::1]`) at its port | on | Low–medium: any program on this computer, and web pages through the user's browser, can send it requests (see below) |
| `net.listen` with `interfaces` | Also on another interface the user picks, such as the Tailscale address | off; the user picks the interface | High: other computers can reach it |
| `storage` | `{data}` (`/data` in WASI) and `sql`, up to 1 GiB by default, more if the manifest says so (shown) | on | Low |
| `background` | Runs while no tile shows it and while Pitwall's window is closed | on, shown | Low: CPU and battery; the card shows its use |

`net`, `settings`, `pitwall` and the rest keep their tile meanings. A
service never gets `files.*` or `git.*` in `service@0.1`: those are
the tile's, granted per project for a user looking at it.

**Requests from other programs are untrusted.** A loopback port is
reachable by every local process and, through DNS rebinding or a
cross-site form, by web pages in the user's browser. So Pitwall's HTTP
front:
- accepts only `Host` headers naming the granted address (`127.0.0.1:8790`,
  `localhost:8790`), which defeats DNS rebinding;
- adds a per-service key: the tile's requests carry it automatically,
  and other callers must send it (`Authorization: Bearer …`), unless the
  manifest marks a route `public` (shown on the permission screen);
- caps request bodies (8 MiB), headers and concurrent connections.
The key is a secret in the system's secure store, shown on the card for
the user to give to other programs, and rotated by Remove.

**Limits** (higher than a tile's, and a watchdog instead of a per-event
limit):
| | Default | Manifest may ask |
|---|---|---|
| Memory per instance | 128 MiB | 1 GiB |
| Instances at once | 16 | 64 |
| Time per request | 30 s | 5 min (long polls, streams) |
| `timer` | 60 s each, ≥ 1 s apart | — |
| `migrate` | 5 min | 30 min |
| CPU watchdog | over 80 % of one core for 10 minutes: Pitwall warns on the card; the user may stop it | — |
Deadlines use epoch interruption as tiles do ([2.5](runtime.md#25-sandbox-and-limits-summary-details-in-5)); a trapped request
gets a 500 and an entry in the Log. Five traps in a minute restart the
service; three restarts in ten minutes stop it with why [today's
service rule].

**Lifecycle.**
- Turning the extension on starts `pitwall-svc` with the compiled
  component (cached as tiles are, [2.6](runtime.md#26-compile-and-cache)), calls `start`, then binds the
  address; `ready` is when the bind succeeds and `start` returned ok.
- It keeps running when Pitwall's window closes and while no tile shows
  it. Quitting Pitwall asks once: "Keep Team board's server running?"
  (remembered per extension; default keep). It doesn't start at login
  on its own: Pitwall's "Start at login" [architecture decision 2]
  starts it with Pitwall.
- Start, Stop and Restart are on the card and in the tile's `service`
  interface ([13.2](#132-a-a-tile-plus-a-managed-service)), the same as for a managed service.
- **Logs**: `stdout`, `stderr` and `log` go to the card's Log [today's
  view], kept by `pitwall-svc` in a 500-line ring so they survive the
  window being closed; secrets taken out as today.

**Updates and data migrations.** Through `safe_update` [today], with
these steps:
1. stop accepting requests, let running ones finish (≤ 10 s), call
   `stop`;
2. back up: the `sql` files and `{data}`, through the host (a SQLite
   online backup for the databases, a copy for the rest, ≤ 2 GiB
   [today's cap]), into `extension-backups/<id>/<time>/`;
3. install the new files;
4. start the new component, call `migrate(from: "1.0.0")`, then
   `start`, then bind; the tile is checked as in [6.4](packaging.md#64-updates-and-go-back);
5. any failure in 1–4 puts back the previous files and the backup,
   starts the previous version, and says what failed [today's rule].
`migrate` must be idempotent and must read the previous schema; the
SDK's `sql::migrations!` keeps numbered SQL files and a version table,
so most authors write SQL, not code. Go back offers to restore the data
taken at that update, default keep [today]. A downgrade can't migrate
backwards: Go back after a schema change needs the backup, and the
review says so.

**How the tile and agents reach it.** The tile, exactly as in [13.2](#132-a-a-tile-plus-a-managed-service):
the automatic grant, `service::url(…)`, the key added for it. Agents,
through the tile's tools ([section 11](agent-tools.md#11-tools-for-agents)), or a service may declare its own
tools of kinds `read` and `act` (no `show`/`present`: it has no UI),
answered by an `agent-tools` export in the service world with the same
grants and approvals. Agents and other local programs that hold the key
can call it directly; Pitwall shows who has called recently by process
name when the caller is local (*how much the OS tells about a loopback
peer per platform is unverified*).

**What a hostile wasm service can still do.**
- Everything its grants allow, all day: send what it stores to its
  `net` hosts; answer requests with misleading data; fill its quota.
- Use CPU and memory up to its limits while the user isn't looking
  (watchdog and card, not prevention).
- Serve something harmful to whoever calls it: its port is reachable by
  local programs, and by other computers if the user granted an
  interface.
- Exploit a Wasmtime or Pitwall host bug; the separate process limits
  that to the service's own process, which still runs as the user.
What it can't: read the user's files or repositories, run programs,
open other ports or interfaces, reach hosts outside `net`, read other
extensions' data, or keep running after the extension is turned off
(`pitwall-svc` is stopped with its process group).

### 13.4 Author experience
- `pitwall extension new <id> --service` (B) adds `src/service.rs` with an
  HTTP route, a migration and a test; `--service native` (A) adds a
  `service` block and an install action for the author to fill.
- `pitwall extension dev` builds both components and restarts the service on
  save (its `/data` kept); the Log streams both.
- `pitwall-extension-test` serves the service in-process and lets tests send
  requests (`h.service().get("/api/items")`) and run `migrate` from a
  fixture database.
- Packaging adds `service.wasm` to the `.pwtile` ([6.1](packaging.md#61-the-pwtile-file)); the index
  records whether a package runs a program (A) or a wasm service (B),
  and Browse shows it.

### 13.5 Order of work
| Step | Content |
|---|---|
| A, during phase 2 | the automatic loopback grant, the tile's `service` interface and event, the "Runs a program" wording in install, Browse and Verified, a starter, docs |
| B1, after phase 7 | `pitwall-svc` (process, socket, logs, rediscovery), the service world 0.1, incoming-handler over hyper, the HTTP front (Host check, key, caps), `net.listen` on loopback |
| B2 | backups and `migrate` in `safe_update` (the `sql` host code comes from tiles, [14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)), limits and watchdog, the quit question |
| B3 | `interfaces` (Tailscale), service tools for agents, `extension new --service`, test harness |

### 13.6 Decisions
1. **May a package ship a native server binary** for A, instead of an
   install action with the author's package manager? It would make A
   one file, but means per-OS binaries in the package and Pitwall
   running a downloaded executable. Decided (owner, 2026-11-26): not in v1: keep today's
   rule (a program on PATH, installed by a confirmed action); authors
   who want one file move to B.
2. **Run wasm services in `pitwall-svc` or in the app?** Decided (owner, 2026-11-26):
   `pitwall-svc`: the owner asked for servers that keep running when
   the window is closed, the app's process can't give that, and the
   separate process isolates crashes.
3. **Raw `wasi:sockets` listening (non-HTTP protocols, in-memory
   state)?** Decided (owner, 2026-11-26): no in `service@0.1`; revisit with WASI 0.3 if
   real services need it.
4. **Host-run SQLite vs. SQLite in the guest?** Decided (owner, 2026-11-26): host-run
   (`sql`) for safety with parallel instances; the guest's own
   SQLite only as an author's choice.
5. **The per-service key by default?** Decided (owner, 2026-11-26): yes, with
   per-route `public` for webhooks; a port open to every local program
   and every web page is the common mistake to avoid.
6. **Keep running after Pitwall quits?** Decided (owner, 2026-11-26): ask once per
   extension, default keep, and always show running services in the
   tray/menu-bar menu so the user can stop them without opening the
   window.
7. **Other computers (`interfaces`).** Decided (owner, 2026-11-26): allowed only on the
   Tailscale interface in the first version, named as such, never
   `0.0.0.0`.
8. **Which way first?** ([13.1](#131-why-and-which-first)) Decided (owner, 2026-11-26): A first, B later.
