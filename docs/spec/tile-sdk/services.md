# Tile SDK: tile apps with a server

Part of the [Tile SDK and marketplace](README.md) design. A tile plus a managed service, and headless wasm services. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 13. Tile apps with a server

### 13.1 Why, and which first
A tile is drawn, suspended when hidden and dropped after ten minutes
([2.3](runtime.md#23-lifecycle)). That is right for a view, and wrong for a program that must keep
data, answer other programs, or work while nobody looks at it: a
team's shared board, a local API the user's agents call, a sync job.
So a tile can't itself be a server. Its extension can have one, in one
of two ways:

| | A. A program the component starts | B. Headless wasm service |
|---|---|---|
| The server is | Any native program, any language | A `service.wasm` component written with `pitwall-extension` |
| Runs | As the user, unsandboxed | In Wasmtime, with only what is granted |
| Runs where | A process Pitwall starts and supervises while the extension is on and Pitwall runs | A small host process of Pitwall's, which keeps running when the window closes |
| Exists | Replaces the managed `service` block (#127), which is removed ([15.5](extension-model.md#155-removed)) | No: new |
| Trust shown | "Runs programs on your computer", with the commands | Same as a tile: its grants |

**Decided (owner, 2026-11-26): A first, B later.** A is now the generic
program capability (owner, 2026-11-28, [13.6](#136-decisions) item 9), not
a manifest block: the component decides what to run and when; Pitwall
only starts, confines and supervises it.

### 13.2 A. Programs the component starts

Zed's way (S26): an extension's wasm names a command, and the
host runs it. Nothing about the program is declared except what the
user must see before granting.

**Manifest.** One permission, `programs`, with the reason and the
commands the component may ask for, as patterns:
```json
"settings": [
  { "key": "listen", "type": "text", "default": "127.0.0.1:8787" }
],
"permissions": {
  "programs": {
    "reason": "To run Agention's server on this computer and set up its plugin in your agents",
    "commands": [
      { "command": "npm", "args": ["install", "--prefix", "{data}/server", "@wayfarerlabs/agention@*"] },
      { "command": "{data}/server/node_modules/.bin/agention", "args": ["**"] },
      { "command": "claude", "args": ["plugin", "**"] },
      { "command": "codex", "args": ["plugin", "**"] }
    ],
    "downloads": []
  }
}
```
- `command` is a name found on the user's login-shell PATH, or a path
  under `{data}` (the extension's data folder,
  `<data>/extension-data/<id>/`). Nothing else: no `sh -c`, no paths
  elsewhere.
- `args` match one by one: a literal, `*` for any one argument, `**`
  for any rest (last only). `{data}` and `{setting:KEY}` are filled in
  before matching.
- At most 16 patterns. `pitwall extension check` refuses a pattern
  whose command is a shell or interpreter with `**` (`sh`, `bash`,
  `zsh`, `cmd`, `powershell`, `node`, `python`…), since that is any
  program.

**WIT** (`programs`, sketch):
```wit
interface programs {
  use types.{host-error};
  variant env-value { text(string), secret(string) }   // secret: a secret setting's key
  record env-var { name: string, value: env-value }
  record command {
    program: string, args: list<string>, env: list<env-var>,
    cwd: option<string>,            // relative to {data}; default {data}
  }
  enum state { asking, starting, running, exited, failed, stopped, denied }
  record status { state: state, pid: option<u32>, exit: option<s32>, why: option<string> }

  /// Run once to the end. A task; `program-finished(task, exit, stdout, stderr)`
  /// follows, each output cut to 1 MiB. ≤ 4 at once, ≤ 10 min each.
  run: func(cmd: command, timeout-ms: u32) -> result<u32, host-error>;
  /// Keep `name` running: restarted after 1, 5 and 30 s, then `failed`.
  /// `listen` (loopback only) lets the component reach it over `net`.
  start: func(name: string, cmd: command, listen: option<string>) -> result<_, host-error>;
  stop: func(name: string);
  status: func(name: string) -> status;
  /// Fetch a file whose SHA-256 the component pins; unpack into {data}.
  download: func(url: string, sha256: string, to: string, unpack: unpack) -> result<u32, host-error>;
  enum unpack { none, zip, tar-gz, gzip }
}
```
Events: `program-changed(name, status)`, `program-finished`,
`download-finished`. SDK: `programs::run`, `programs::start`,
`Command::new("agention").arg("serve").env_secret("KEY", "apiKey")`.

**Every exact command is confirmed once.** A call whose command
doesn't match a declared pattern is `denied` without asking. One that
matches, and whose exact line (program resolved to its path, every
argument, the names of its variables, its folder) hasn't been allowed,
puts up a host-drawn sheet naming the extension: "Agention wants to
run: `/Users/me/.pitwall/extension-data/agention/server/node_modules/.bin/agention serve --data … --listen 127.0.0.1:8787`",
with **Run** (this time), **Always** (this exact line) and **Deny**. A
supervised program's sheet says "and keep it running". The call waits
in `asking`. "Always" lines are listed on the extension's page with
Remove. A line that changes (a setting, an update, another version in
the arguments) asks again. The component never sees the user's PATH.

**How it runs.**
- In its own process group (a job on Windows), as the user, with a
  clean environment: `PATH`, `HOME`, `USER`, `LANG`, `TMPDIR` and the
  listed variables. A `secret(KEY)` value is put in by the host from
  the secure store; the component never holds it, and it is taken out
  of Log lines.
- stdout and stderr go to the extension's Log (a 500-line ring per
  program); a one-shot run also returns them to the component.
- Supervised programs (≤ 4 per extension) stop when the extension is
  turned off, updated or removed, or when Pitwall quits; their state is
  on the page with Stop, Restart and Log. A program already listening
  on the `listen` address when asked to start is left alone and
  reported `running` with `pid: none` ("running elsewhere", as the
  service did).
- `listen` gives the component `http://` access to exactly that
  loopback address while the program is running, without a `net`
  grant: the replacement of the service's automatic loopback grant.
  The component checks readiness itself, by calling it.
- **Download helper.** `url` must match one of `downloads` (patterns
  like `https://github.com/owner/repo/releases/download/*`, https only)
  and the bytes must hash to the `sha256` the component passes (pinned
  in its code, so a release can't change under it). Unpacked under
  `{data}` with Pitwall's own archive reader (no paths out, no links),
  executables marked. Running the result still needs a matching
  command pattern and the confirmation.

**Updates.** `safe_update` stops the extension's programs, backs up
`{data}` (≤ 2 GiB, as today), installs, and starts the new component,
which starts its programs again. A failure puts the old files and the
backup back. There is no per-extension backup command; the programs are
stopped, so a copy of the folder is consistent (SQLite files with their
WAL).

**Trust.** The permission screen says **"Runs programs on your
computer"** with the patterns as written; Browse marks such extensions
and "Verified only" can filter on it. Verified covers what the patterns
can run (the source of anything downloaded, at its pinned hash).
`programs` starts off: the user must Allow it.

What a hostile extension can do with `programs` is what the user can,
within the patterns it was granted and each line the user allowed
([5.5](permissions.md#55-threat-model-and-outside-review), "Programs"). The component beside it stays
sandboxed; that protects little once a program it starts isn't.

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
so a component written against [13.2](#132-a-programs-the-component-starts) moves with little change.

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
  interface, the same as for a supervised program ([13.2](#132-a-programs-the-component-starts)).
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

**How the tile and agents reach it.** The tile, as with a program's `listen` ([13.2](#132-a-programs-the-component-starts)):
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
  HTTP route, a migration and a test; `--program` (A) adds a `programs`
  permission and a `programs::start` call for the author to fill.
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
| A, during phase 2 | built as the managed `service` block (#127); replaced by the program capability ([plan](plan.md) 8a), which the `service` block's removal follows (8c) |
| B1, after phase 7 | `pitwall-svc` (process, socket, logs, rediscovery), the service world 0.1, incoming-handler over hyper, the HTTP front (Host check, key, caps), `net.listen` on loopback |
| B2 | backups and `migrate` in `safe_update` (the `sql` host code comes from tiles, [14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)), limits and watchdog, the quit question |
| B3 | `interfaces` (Tailscale), service tools for agents, `extension new --service`, test harness |

### 13.6 Decisions
1. **May a package ship a native server binary** for A, instead of an
   install action with the author's package manager? It would make A
   one file, but means per-OS binaries in the package and Pitwall
   running a downloaded executable. Decided (owner, 2026-11-26): not in v1: keep today's
   rule (a program on PATH, installed by a confirmed action); authors
   who want one file move to B. **Superseded** by item 9: the download
   helper fetches a release binary pinned by hash.
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
9. **Native programs, Zed's way** (owner, 2026-11-28, [15.9](extension-model.md#159-decisions) item 8):
   the component names a command; Pitwall runs and supervises it behind
   the `programs` permission, with declared patterns, each exact line
   confirmed once, and a download helper with hash pinning. The managed
   `service` block, install actions, version and ready checks, backup
   commands and first run are removed: the component does each with
   programs and `net`.
