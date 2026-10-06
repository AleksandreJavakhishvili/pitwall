# Architecture: providers + daemon (Wave 2 design)

Status: **approved design, migration in progress** (see §6 "Progress").
Written against the code as of wave 1.5, with worktrees done the new way
(worktrees.md: the agent's own flag, then discovery).

Goals, from the user: proper code and proper abstractions, so new things plug
in easily, with few ties to specific things. In concrete terms:

- A new **place to run** (agw, SSH, later cloud) is one new `Provider`
  implementation. Nothing else changes.
- A new **agent CLI** is one TOML file, as it is today.
- The UI turns features on and off from **capabilities**. It never asks which
  provider or kind an agent has.
- Agents keep running when the app quits. A **daemon** (`pitwalld`) owns
  them. The app and a `pitwall` CLI are clients of the daemon.
- Pitwall stays a tool. Providers use the agents' own features (resume flags,
  worktree flags, agw harness integrations). Pitwall never invents its own.

---

## 1. Crates

```
                      ┌────────────────────┐
                      │   pitwall-proto    │  wire types, AgentView, caps,
                      │ (serde only, no IO)│  protocol version; TS generated
                      └─────────┬──────────┘
            ┌───────────────────┼─────────────────────────┐
            │                   │                         │
┌───────────▼─────────┐ ┌───────▼───────────┐   ┌─────────▼─────────┐
│   pitwall-detect    │ │   pitwall-core    │   │  pitwall-client   │
│ Screen (vt100) +    │◄┤ domain, traits,   │   │ UDS connect,      │
│ TOML screen rules   │ │ Engine, status,   │   │ handshake, calls, │
│ (pure)              │ │ queue, tasks, git │   │ events, term      │
└─────────────────────┘ │ over Exec, kinds  │   │ streams           │
                        │ testing::{Fake*,  │   └───┬──────────┬────┘
                        │  contract}        │       │          │
                        └───────▲───────────┘   ┌───▼────┐ ┌───▼──────────┐
                                │               │pitwall │ │ src-tauri    │
                      ┌─────────┴──────────┐    │ (CLI)  │ │ (app: windows│
                      │ pitwall-providers  │    └────────┘ │ webview,     │
                      │ local │ agw │ ssh  │               │ notifications│
                      │ (cargo features)   │               │ menu bar)    │
                      └─────────▲──────────┘               └──────────────┘
                                │                               ▲
                      ┌─────────┴──────────┐   Unix socket      │
                      │  pitwall-daemon    │◄───────────────────┘
                      │  bin: pitwalld     │◄── pitwall CLI
                      │  socket server,    │◄── hook relay (HTTP, unchanged)
                      │  FileStore, hooks, │
                      │  approvals, rules, │
                      │  scan              │
                      └────────────────────┘
```

| Crate | Owns | Why it is its own crate |
|---|---|---|
| `pitwall-proto` | Request/response/event types, `AgentView`, `KindView`, `Caps`, `PROTOCOL_VERSION`. Depends only on serde. | The one contract that the daemon, the client, the CLI and the TS types share. Clients must not have to pull in the engine to talk to it. |
| `pitwall-detect` | `screen.rs`, `detect.rs`, `detect/*.toml` and their fixtures. | Pure, has heavy dependencies (vt100, regex), and has its own owner (the Detection workstream). Today's boundary, now enforced by the compiler. |
| `pitwall-hold` (bin + lib) | The per-agent terminal holder (§9 decision 1): owns one PTY + child, serves a frozen, versioned protocol (documented in its `lib.rs`) on `run/hold/<agentId>.sock`; the lib has the protocol and the client. OS code only in `platform/` (unix today). Deps: `interprocess`, `libc`. | Must outlive every other Pitwall process and almost never change, so it is tiny and has no Pitwall dependencies. |
| `pitwall-core` | Domain model, the traits in §2, the `Engine` (today's registry, ticker policy, auto-send, status folding, tasks), kinds and launch planning, git operations written against `Exec`, the `Store` trait. Also `testing::{FakeProvider, MemStore, contract}` behind the `testing` feature. | All the logic, runnable and testable without sockets, Tauri, real processes or `$HOME`. It has no knowledge of macOS, PTYs, ssh or agw. |
| `pitwall-providers` | `local` (portable-pty, lsof, login shell), `agw` (agw CLI + ssh), `ssh` (wave 3). Each is a cargo feature. | It is the only crate that knows about specific places to run. Every provider runs the same contract test suite. |
| `pitwall-daemon` (`pitwalld`) | Hosts the Engine: socket server, `FileStore` (`~/.pitwall`), hook ingest, approvals, rules (rulesync), onboarding scan, the daemon lifecycle. | It is the process that owns sessions. Tokio lives here only. |
| `pitwall-client` | Connect, handshake, typed calls, event stream, terminal streams. | Shared by the app and the CLI, so the protocol is implemented on the client side exactly once. |
| `pitwall-cli` (`pitwall`) | Argument parsing, JSON and human output. | A tiny binary built on `pitwall-client`. |
| `src-tauri` | Windows, window bounds (`windows.json`), the webview bridge, notifications, Dock badge, menu-bar item, and starting the daemon. | A thin client. It never touches agents or state files directly. |

What I deliberately did **not** split out:
- No separate git or vcs crate. Git is ~600 lines of parsing over `Exec`, and
  the Engine and the daemon are its only users.
- No separate rules crate. Rules run rulesync through `Exec` and are a daemon
  service.
- No separate crate per provider. Cargo features are enough until a provider
  gets large.

Threading: `pitwall-core` uses no async runtime. Its traits are blocking
(they mostly start subprocesses, as today), and the Engine uses threads and
channels, as today. The daemon uses tokio for the socket and calls the core
through `spawn_blocking`. This keeps the core easy to test and keeps the
proven threading model.

---

## 2. Core traits

All of these live in `pitwall-core::provider` unless noted. `Result<T>` is
`Result<T, PwError>`, where `PwError` has a `code` (`NotFound`,
`Unsupported`, `NotRunning`, `Unreachable`, `Conflict`, `Denied`, `Other`)
and a human-readable message. Strings are the only error type today; a code
lets clients react without parsing text.

### 2.1 Identity

```rust
/// "local", "agw", "ssh:devbox". Stable; part of persisted state.
pub struct ProviderId(pub String);
/// Within a provider: "this-mac", an agw VM name, an ssh host alias.
pub struct MachineId(pub String);

/// Where an agent lives, in the provider's own terms. Unique across Pitwall.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Locator {
    pub provider: ProviderId,
    pub machine: MachineId,
    /// The provider's own handle: an agw session name, a tmux session name,
    /// or (local) the Pitwall agent id, because a local PTY has no other name.
    pub native: String,
}

/// Pitwall's handle for an agent: an opaque UUID, as today.
pub struct AgentId(pub String);
```

The decision is **UUID primary key plus a unique `Locator` index**, rather
than using the locator as the id:
- The id is baked into places that must not change. These are
  `PITWALL_AGENT_ID` in running agents' environments, hook URLs,
  `refs/pitwall/<agent>/<task>`, rules exclude blocks, and the UI's spaces
  blob.
- Native handles are not always stable. A local agent has none. A
  conversation id changes on every fresh start, so it stays a separate field
  (`conversation_id`, today's `session_id`).
- Adopting an existing agw session looks up the `Locator` first. Adopting the
  same session twice is therefore idempotent, and onboarding's `inPitwall`
  check becomes a locator lookup.

### 2.2 Provider

```rust
pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn caps(&self) -> ProviderCaps;

    /// Machines this provider can run on (local: exactly one).
    fn machines(&self) -> Result<Vec<Machine>>;
    /// Kinds usable on a machine, with "installed" resolved there.
    fn kinds(&self, m: &MachineId, catalog: &KindCatalog) -> Result<Vec<KindOnMachine>>;
    /// Sessions that exist on a machine but may not be in Pitwall yet
    /// (agw sessions, or local CLIs running in another terminal). Read-only.
    fn discover(&self, m: &MachineId) -> Result<Vec<Discovered>>;

    /// Start a new agent. Returns the locator and, when the provider's
    /// session *is* the process (local), the live terminal.
    fn create(&self, spec: &CreateSpec) -> Result<Started>;
    /// Start an existing agent again. `Resume` continues its conversation
    /// (the kind's resume args, or agw `start --resume-only`).
    fn start(&self, loc: &Locator, mode: StartMode, size: TermSize) -> Result<Started>;
    /// Connect to a session that is already running (agw/tmux). A local
    /// provider returns `Unsupported`, because its terminal comes from `start`.
    fn attach(&self, loc: &Locator, size: TermSize) -> Result<Box<dyn TermIo>>;
    fn stop(&self, loc: &Locator) -> Result<()>;
    /// Forget the session on the provider's side (agw `session delete`);
    /// the local provider has nothing to delete.
    fn remove(&self, loc: &Locator) -> Result<()>;
    fn state(&self, loc: &Locator) -> Result<NativeState>; // Running | Stopped | Gone

    /// Run commands and read files where the agent works (§2.4).
    fn exec(&self, m: &MachineId) -> Result<Arc<dyn Exec>>;
    /// The folder a running agent is in (worktree discovery). Local: lsof.
    fn process_cwd(&self, loc: &Locator, pid: Option<u32>) -> Result<Option<String>>;
    /// Plain-text screen without attaching (agw: tmux capture-pane).
    fn capture(&self, loc: &Locator) -> Result<String>;
}

pub struct CreateSpec<'a> {
    pub agent: &'a AgentId,
    pub machine: MachineId,
    pub kind: &'a AgentKind,
    pub workspace: WorkspaceSel,   // Path(String) | Existing(String) | New{name, template}
    pub name: &'a str,
    pub intent: LaunchIntent,      // Fresh | Resume(conversation_id)
    pub worktree: Option<String>,  // name for the kind's own worktree flag
    pub size: TermSize,
    pub hooks: Option<HookWiring>, // only when caps.hooks != None
}

pub struct Started {
    pub locator: Locator,
    pub conversation_id: Option<String>,
    pub resumed: bool,
    pub cwd: String,                    // on the agent's machine
    pub term: Option<Box<dyn TermIo>>,  // None ⇒ the engine calls attach()
}
```

`launch::plan()` (today's launch.rs) remains a core helper. Providers that
launch a command themselves (local, ssh) call it to turn
`AgentKind + LaunchIntent + worktree + hooks` into a command line. Providers
whose platform owns the launch (agw harness integrations) ignore it.

### 2.3 Terminals

A provider supplies only the raw byte pipe. Everything Pitwall does with the
bytes stays in core and is written once: ring buffer and replay, fan-out to
clients, the headless `Screen`, activity and echo timing, bracketed-paste
sending, and the redraw nudge.

```rust
/// A raw terminal connection: a local PTY, or a PTY running
/// `agw session attach <name>` / `ssh -t host tmux attach -t <s>`.
pub trait TermIo: Send {
    fn take_reader(&mut self) -> Box<dyn Read + Send>;   // called once
    fn take_writer(&mut self) -> Box<dyn Write + Send>;  // called once
    fn resize(&self, size: TermSize) -> Result<()>;
    fn try_wait(&mut self) -> Result<Option<ExitInfo>>;
    /// Local: SIGHUP then kill the agent. Remote: end the *attachment* only.
    fn close(&mut self, grace: Duration);
    fn pid(&self) -> Option<u32>;
    /// Whether EOF means "the agent exited" (local) or only
    /// "the attachment dropped" (tmux), in which case the engine asks
    /// `Provider::state` and may reattach.
    fn eof_is_exit(&self) -> bool;
}

/// Core-owned wrapper (today's `session::Session` minus portable-pty).
pub struct TermHost { /* ring, subscribers, Screen, seq, last_activity, last_input */ }
impl TermHost {
    pub fn new(io: Box<dyn TermIo>, size: TermSize) -> Arc<Self>;
    pub fn subscribe(&self, sink: Box<dyn FnMut(&[u8]) -> bool + Send>, replay: bool) -> SubId;
    pub fn unsubscribe(&self, id: SubId);
    pub fn write(&self, bytes: &[u8]) -> Result<()>;
    pub fn send_text(self: &Arc<Self>, text: String); // paste + Enter, never modified
    pub fn resize(&self, size: TermSize) -> bool;
    pub fn screen_text(&self) -> (String, Option<String>);
}
```

The subscriber sink is a closure, not `tauri::ipc::Channel`. The daemon's sink
writes binary frames to a socket. Tests collect into a `Vec`.

### 2.4 Exec, files and git where the agent works

Git, tasks, Review, worktree discovery and rules all need "run a program in
that folder". Today each module calls `Command::new("git")` itself. Instead:

```rust
pub struct Cmd<'a> {
    pub argv: &'a [&'a str],
    pub cwd: &'a str,                 // path on the target machine
    pub env: &'a [(&'a str, &'a str)],
    pub stdin: Option<&'a [u8]>,
    pub timeout: Duration,
}
pub struct Out { pub status: i32, pub stdout: Vec<u8>, pub stderr: Vec<u8> }

pub trait Exec: Send + Sync {
    fn run(&self, cmd: &Cmd) -> Result<Out>;
    fn read_file(&self, path: &str, max: u64) -> Result<Option<Vec<u8>>>;
    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()>;
    fn remove_file(&self, path: &str) -> Result<()>;
    /// A scratch dir on the machine (temp index files for task snapshots).
    fn temp_dir(&self) -> Result<String>;
    fn home(&self) -> Result<String>;  // for "~/…" display
}

/// pitwall-core::vcs — today's git.rs/tasks.rs/review.rs plumbing, unchanged
/// in logic, but every call goes through `exec`.
pub struct Git<'a> { exec: &'a dyn Exec, dir: String }
impl Git<'_> {
    pub fn repo_root(&self) -> Option<String>;
    pub fn head(&self) -> Option<String>;
    pub fn current_branch(&self) -> Option<String>;
    pub fn changes(&self, base: Option<&str>) -> Result<Vec<FileChange>>;
    pub fn file_diff(&self, base: Option<&str>, path: &str, untracked: bool) -> Result<String>;
    pub fn snapshot_tree(&self) -> Result<String>;       // GIT_INDEX_FILE=<temp_dir>/…
    pub fn worktree_list(&self) -> Result<Vec<WorktreeEntry>>;
    pub fn merge_base(&self, a: &str, b: &str) -> Option<String>;
    pub fn commit_all(&self, msg: &str) -> Result<String>;
    pub fn merge_no_ff(&self, branch: &str) -> Result<MergeResult>;
    // …
}
```

- `LocalExec` runs `std::process::Command` with `GIT_OPTIONAL_LOCKS=0`, as
  today.
- `SshExec` runs `ssh -o BatchMode=yes -o ControlMaster=auto
  -o ControlPath=~/.pitwall/run/ssh-%C -o ControlPersist=120 <dest> -- 'cd <q>
  && exec <argv…>'`. A persistent master connection keeps the 3-second git
  refresh cheap.

### 2.5 Status sources

Today `status::raw_state(hook, screen, activity)` hard-codes three inputs.
They become a priority-ordered list. A provider can add its own source, such
as agw `--status` reporting running or stopped:

```rust
pub enum Raw { Working, Blocked, Idle, Done, Unknown }
pub struct Signal { pub raw: Raw, pub detail: Option<String>, pub source: SourceTag }

/// One input to an agent's status. Sources are updated by whoever feeds
/// them (hook ingest, TermHost output, provider poll) and read by the ticker.
pub trait StatusSource: Send {
    fn tag(&self) -> SourceTag;                      // "hooks" | "screen" | "activity" | …
    fn signal(&mut self, now: Mono) -> Option<Signal>; // None = no opinion
    fn reset(&mut self);                             // on (re)start
}

pub struct StatusInputs { sources: Vec<Box<dyn StatusSource>> } // priority order
impl StatusInputs {
    /// First source with an opinion wins (hooks > screen > activity).
    pub fn fold(&mut self, now: Mono) -> Signal;
}
```

`HookSource` is fed by hook ingest. `ScreenSource` holds the kind id and calls
`pitwall_detect::detect` when `TermHost.seq` changes. `ActivitySource` reads
`last_activity`. `next_status()` and the `done`/`mark_seen` logic stay as they
are. The Engine builds an agent's list from kind and provider capabilities, so
`HookSource` is added only when hooks can actually arrive.

### 2.6 Agent kinds: still data

`AgentKind` stays TOML (built-ins plus `~/.pitwall/agents/*.toml`). There are
two additive fields:

```toml
aliases = ["claude-code"]     # names other platforms use (agw harness integrations)
transcripts = "claude-jsonl"  # which built-in parser lists its past conversations
```

Kind capabilities are derived from the definition and never hard-coded:
`resume = !resume_args.is_empty()`, `worktree = !worktree_args.is_empty()`,
`hooks = hooks != none`, `rules = rulesync_target.is_some()`. Conversation
discovery (today `scan::parse_claude_transcript` and
`parse_codex_session`) becomes a registry of parsers keyed by `transcripts`.
`scan.rs` then stops branching on "claude" or "codex".

### 2.7 How agw maps onto the traits

This mapping comes from agw 0.19, which was inspected read-only. In agw, a
session is a tmux session that runs a harness integration in a workspace on a
VM, as the admin user or as an agent user. VMs can be reached over ssh at
their Tailscale IP.

| Pitwall | agw |
|---|---|
| `machines()` | `agw vm list --output json` → `data.vms[].{name, site, tailscale_host}` |
| `discover(vm)` | `agw session list --vm <vm> --status --output json` → `data.sessions[].{name, workspace_name, harness_integration, mode, agent_name, status}` |
| `Locator` | `agw:<vm>/<session name>` (agw names are unique) |
| kind | `harness_integration` matched against kind `id`/`aliases` (`claude-code` → `claude`); no match → a view-only "shell-like" kind with screen/activity status |
| `create` | `agw session create <name> --vm <vm> --workspace <ws> --template <t>` (or `--new-workspace …`); risky, so it needs an approval |
| `start(Resume)` / `start(Fresh)` | `agw session start <name> --resume-only` / `--force-new` (agw's harness integration owns the resume logic, so Pitwall's `resume_args` are not used) |
| `attach` | a local PTY running `agw session attach <name>` (a tmux client). Its EOF means the attachment dropped, not that the agent exited |
| `stop` / `remove` | `agw session stop <name>` / `agw session delete <name> --yes` (behind approval) |
| `state` | `agw session describe <name> --output json` → `status` |
| `exec(vm)` | `SshExec` to `tailscale_host` (see open question 3); workspace path from `agw workspace describe <ws> --output json` → `path` |
| `capture` | `SshExec`: `tmux capture-pane -p -t <session>` |
| `process_cwd` | `SshExec`: `tmux display -p -t <s> '#{pane_current_path}'` |
| hooks | none at first (`caps.hooks = None`): status comes from screen detection on the attached stream. Later, a hook bundle delivered through agw artifacts that relays over a forwarded socket |
| rules | later, through agw artifact bundles (`caps.rules = false` until then) |

An **SSH provider** (wave 3) has the same shape with less in it. `machines`
come from config (`~/.pitwall/providers/ssh.toml`: alias, host, user). The
`Locator` native id is a tmux session name `pw-<agentId8>`. `create` runs
`ssh host tmux new-session -d -s pw-… -c <dir> '<launch::plan command>'`.
`attach` runs `ssh -t host tmux attach -t pw-…` in a local PTY. `exec` is
`SshExec`. `stop` uses `tmux kill-session`. Hooks can be added with
`ssh -R <remote.sock>:<~/.pitwall/run/pitwall.sock>` plus the same relay
script on the host (`caps.hooks = Forwarded`).

The **local provider** is today's code. `create` and `start` call
`launch::plan` and then portable-pty through the login shell, and the
returned `TermIo` closes with SIGHUP and then kill. `discover` lists CLIs
running in other terminals via lsof (info only) and past conversations
through the transcript parsers. `exec` is `LocalExec`. `process_cwd` uses
lsof.

---

## 3. Capabilities instead of special cases

```rust
pub struct ProviderCaps {
    pub create: bool,          // can start new agents
    pub resume: bool,          // can continue a conversation on restart
    pub attach_existing: bool, // can adopt sessions found by discover()
    pub survives_detach: bool, // the session outlives the attachment (tmux)
    pub exec: bool,            // can run commands where the agent works (diffs, Review)
    pub process_cwd: bool,     // can find where a running agent works
    pub capture: bool,
    pub hooks: HookTransport,  // None | LocalSocket | Forwarded
    pub rules: bool,
    pub custom_command: bool,
}
```

The Engine computes an **effective per-agent set** and sends it to clients in
`AgentView.caps`. The UI reads only this:

```rust
#[serde(rename_all = "camelCase")]
pub struct AgentCaps {
    pub input: bool,        // running and attached
    pub restart: bool,      // provider.create || provider.resume
    pub resume: bool,       // provider.resume && kind.resume && has conversation
    pub stop: bool,
    pub remove_worktree: bool, // exec && record.worktree.is_some()
    pub diff: bool,         // exec && cwd is a git repo
    pub review: bool,       // diff
    pub merge: bool,        // diff && worktree found && branch not detached
    pub rules: bool,        // provider.rules && kind.rules
    pub hooks: bool,        // provider.hooks != None && kind.hooks != none
}
```

For new agents, `list_kinds(machine)` returns
`KindView { …, installed, caps: { worktree, resume, rules, hooks } }`, where
`worktree` = kind.worktree && provider.exec && provider.process_cwd. The
New-agent dialog uses `ProviderView.caps.create` and `KindView.caps.worktree`,
and Review uses `AgentView.caps.diff`.

Rules:
- No `provider === "agw"` and no `kind === "claude"` in `src/`. A CI grep
  enforces this.
- `AgentView.location` (today hard-coded `"local"`) becomes
  `machine: { provider, id, label }`. It is used only for display and
  grouping.
- In the daemon, the same rule applies. Engine code branches on caps, never on
  `ProviderId`.

---

## 4. Daemon ↔ client protocol

**Transport.** A Unix socket at `~/.pitwall/run/pitwalld.sock` (directory
mode 0700, socket mode 0600). The existing hook socket `run/pitwall.sock`
stays as it is: an HTTP `POST /hook/<id>` endpoint, now served by the daemon.
Hook scripts and Codex `hooks.json` entries that are already installed keep
working.

**Framing.** One connection carries everything:

```
frame  := len:u32be  type:u8  body
type 0 := JSON message (UTF-8)
type 1 := terminal data: stream:u32be  bytes…   (both directions)
```

**Handshake.** The first frame each way:

```json
→ {"hello":{"protocol":{"min":1,"max":1},"client":"pitwall-app/0.2.0","role":"ui"}}
← {"welcome":{"protocol":1,"daemon":"0.2.0","caps":["agents","review","rules","approvals","provider:agw"],"instance":"…"}}
← {"reject":{"code":"incompatible","daemon":"0.3.0","protocol":{"min":2,"max":2}}}
```

Compatibility rules:
- Within a protocol major version, changes are additive only. New fields are
  optional, and both sides ignore fields they don't know.
- An unknown method returns `{"code":"unknown_method"}`.
- Features that only some daemons have are gated by the `caps` list in
  `welcome`, never by version comparisons.
- If the app finds an older but compatible daemon, it uses it. If the daemon
  is incompatible, the app offers to restart it (see open question 1).

**Requests and events.**

```json
→ {"id":7,"method":"agent.create","params":{…CreateAgentRequest…}}
← {"id":7,"result":{…AgentView…}}
← {"id":8,"error":{"code":"not_running","message":"agent is not running"}}
→ {"id":9,"method":"events.subscribe","params":{"topics":["agents","attention","approvals","projects","scan","ui_state"]}}
← {"event":"agents.changed","data":[…AgentView…]}
→ {"id":10,"method":"term.attach","params":{"agent":"…","replay":true,"control":false}}
← {"id":10,"result":{"stream":3}}       then type-1 frames on stream 3
→ type-1 frame on stream 3             = write_input
→ {"id":11,"method":"term.detach","params":{"stream":3}}
```

Method names map one to one onto today's commands. For example,
`list_agents` becomes `agent.list`, `queue_add` becomes `queue.add`,
`get_task_changes` becomes `review.task_changes`, and `apply_rules` becomes
`rules.apply`. This keeps api.md as the reference and lets the CLI mirror it.

`term.attach` takes `control: false` for view-only attachments (the Wall).
Those never resize the terminal. A `term.resize` only counts from a
`control: true` attachment. The last controlling client to resize wins. This
matches "an agent is shown in at most one pane".

**Multiple clients.** Every window of the app shares one connection through
`pitwall-client`. The CLI opens short-lived connections. Events go to every
subscriber. The UI-state blob (`ui.get`, `ui.set`, `ui_state.changed`) moves
into the daemon as an opaque document, so CLI `space` commands and all
windows see the same spaces. Window bounds stay in the app.

**Tauri bridge.** `src-tauri` exposes one generic command,
`daemon_call(method, params)`. It forwards daemon events to the webview with
`emit` and maps terminal streams to `Channel<ArrayBuffer>`. `api.ts` keeps its
`Api` interface, and only `createTauriApi` changes. The mock API is
untouched.

**Approvals.**
- Every method carries a risk level in a single table in the daemon. Risky
  methods are:
  - `agent.remove` with `deleteWorktree`
  - `review.merge`, `review.discard`, `review.commit`
  - `hooks.install`
  - `rules.apply` to the main checkout
  - any provider `create` or `remove` on agw
  - `daemon.stop`
- For a risky request, the daemon creates
  `Approval { id, action, summary, requester }`. It emits
  `approval.requested` to UI clients and waits, with a timeout that ends in a
  denial.
- The answer is `approval.answer { id, allow }`, and the daemon accepts it
  only from a verified UI client. The peer pid comes from `LOCAL_PEERPID`.
  It must be the signed Pitwall.app executable (path plus code-signing
  check), and it must **not** be a descendant of any hosted agent's process.
- A request whose peer is a descendant of an agent is tagged
  `requester: agent(<name>)`, so an agent can never answer its own prompt.
- The UI's own buttons go through the same path: the dialog the user sees is
  the approval. There is no second confirmation step.

---

## 5. Identity, state and persistence

The daemon is the only writer under `~/.pitwall/`. A single-instance lock,
`run/pitwalld.lock` (flock), stops two daemons, or the old in-app backend,
from writing at the same time.

| File | Owner after migration | Change |
|---|---|---|
| `state.json` | daemon | `version: 2`. Each record gains `locator`. The conversation id keeps its on-disk name `sessionId` (in Rust: `conversation_id` with `#[serde(rename)]`). |
| `projects.json` | daemon | Entries gain an optional `machine` (default local). |
| `rules/`, `rules-sources/`, rules store | daemon | Unchanged. |
| `ui.json` | daemon (opaque blob) | Unchanged contents. |
| `windows.json` | app | Unchanged. |
| `agents/*.toml`, `detect/*.toml` | daemon reads | Unchanged. |
| `bin/pitwall-hook`, `run/pitwall.sock` | daemon | Unchanged. |
| `run/pitwalld.sock`, `run/pitwalld.lock`, `run/ssh-*` | daemon | New. |
| `providers/*.toml` | daemon | New: provider config (ssh hosts; agw binary override). |

**Migrating to v2** happens on first daemon start:
1. Read v1.
2. Make a one-time backup at `state.v1.json`.
3. Set each record's `locator` to `local:this-mac/<id>`.
4. Write v2.

Only fields are added, so an older app still loads v2, because serde ignores
unknown fields. A downgrade keeps working for local agents.

In memory, `AgentRecord` stays what persists. Runtime state (today's
`registry::Agent` fields such as `hook`, `screen_state`, `git_*` and `watch`)
moves into an `AgentRuntime` that holds `StatusInputs`, `Option<Arc<TermHost>>`
and the git poller state. The `Store` trait (`load() -> Snapshot`,
`save(&Snapshot)`) is implemented by `FileStore` in the daemon and `MemStore`
in tests.

Paths are no longer free functions over `$HOME` (`paths::state_file()` and
others). A `Paths { root }` value is injected, following the pattern
`rules::Dirs` already uses, so tests run in a tempdir.

---

## 6. Migration plan

Each step ships on its own, keeps the app working, and keeps `cargo test` and
the UI build green.

**Progress** (2026-10-06):
- Step 0 done: root `Cargo.toml` workspace (`src-tauri`, `crates/*`); one
  `Cargo.lock` and `target/` at the root; release profile moved there.
- Step 1 done: `crates/pitwall-detect`; the app uses
  `pitwall_detect::{Screen, detect, Detected, Detection}`. The "every rule file
  has an agent kind" test moved to `src-tauri/src/agents.rs`.
- §9 decision 1, first part done ahead of the daemon: `crates/pitwall-hold`.
  `session.rs` starts each agent in a holder and talks to it over the holder
  protocol instead of owning the PTY; on app start it re-attaches persisted
  agents whose holder still runs (no respawn). ⌘Q no longer stops agents
  ("Quit and Stop Agents" in the app menu does). The holder binary is built
  by `src-tauri/build.rs` and shipped as a Tauri `externalBin` (backend.md).
  In step 4 the Local provider's `TermIo` wraps this client unchanged.

**Step 0 — Workspace skeleton.**
- Add a root `Cargo.toml` workspace and make `src-tauri` a member.
- No code changes.

**Step 1 — Extract `pitwall-detect`.**
- Move `screen.rs`, `detect.rs`, `detect/`, the scan fixtures that detect
  uses, and `detect/tests*.rs`.
- The backend imports `pitwall_detect::{Screen, detect}`.
- This is a pure move.

**Step 2 — Remove Tauri from the logic (the biggest cut).** Create
`pitwall-core` and move into it everything that does not need Tauri. The app
still links it in-process. The couplings to cut:
- `registry::Core.app: AppHandle` → `events: Arc<dyn EventSink>`, with
  `emit(Event::AgentsChanged | Attention | BlockedCount | ScanProgress |
  ProjectsChanged)`.
  - `ticker::start` uses `core.app.emit("agents-changed")`.
  - `ticker::tick` calls `attention::raise` and `attention::set_badge`.
  - `onboarding::projects_changed` and `scan_environment` emit directly.
  - After the cut, the app implements `EventSink`: it emits to webviews,
    shows notifications and sets the Dock badge. `attention.rs` stays in the
    app.
- `session::Output.subscribers: Vec<(u64, Channel<InvokeResponseBody>)>` →
  sink closures (§2.3). `Session::attach` loses its Tauri type.
- `#[tauri::command]` functions mixed with logic in `review.rs`, `rules.rs`
  and `onboarding.rs` → split them. Plain service functions go to core, and
  the thin `#[tauri::command]` adapters stay in `src-tauri/src/commands/`.
  Each module's `blocking()` helper (`tauri::async_runtime::spawn_blocking`)
  stays in the adapters.
- `paths::*` free functions → `Paths`. `persist::load` and `save` → `Store`.
- Global statics become Engine-owned values: the `kinds::cache()` OnceLock,
  the `detect::cache()` RwLock, and the `project_list` lock.
- `model::mono_ms()` global clock → a `Clock` trait, so auto-send and status
  timing can be tested deterministically.

**Step 3 — Run git and other commands through `Exec`.** With `LocalExec`
only, behaviour stays identical. Port these:
- `git.rs`: `fn git(cwd) -> Command`
- `tasks.rs`: its own `Command::new("git")`, `temp_index_path()`, `snapshot`,
  `keep`, `drop_refs`
- `review.rs`: `disk()` reads the file system directly, plus `commit` and
  `merge`
- `worktree.rs`: `roots`, `pick_name`, `check`, `discover_new`
- `rules/apply.rs`: `git()`, `tracked`, `write_exclude`
- `rules/library.rs`: its `git()` stays local, because the library is on this
  Mac
- `ticker::refresh_git` and `commands::get_changes` / `get_file_diff` call
  `Git` through the agent's machine `Exec`

**Step 4 — Provider, `TermIo`, capabilities, `FakeProvider` and contract
tests.**
- Add `pitwall-providers::local`, which wraps today's `Session::spawn` (login
  shell, env, `CLAUDE_CODE_*` stripping) as `TermIo`.
- `registry::Agent.session: Option<Arc<Session>>` →
  `Option<Arc<TermHost>>` plus `locator`.
- In `lifecycle::create`, `restart`, `stop` and `remove`, replace the direct
  `Session::spawn`, `git::head`, `hooks::hook_command` and
  `worktree::process_cwd` calls with `Provider` and `Exec` calls.
- In `worktree.rs`, `process_cwd` (lsof) moves into the local provider.
- `kinds::list` / `shell::which` → `Provider::kinds`.
- `status::raw_state` → `StatusInputs`.
- `Agent::view` stops hard-coding `location: "local"` and `paths::tildify`;
  it uses the machine label and `Exec::home`.
- `AgentView.caps` and `KindView.caps` are added. The UI switches to caps in
  `NewAgentDialog`, `RemoveDialog`, `PaneHeader` and Review.
- Move `state.json` to v2.
- Move the agw parsing in `scan.rs` (`agw()`, `agw_rows`, `agw_kind`) into
  `providers::agw::discover`. The onboarding scan asks each provider to
  `discover`.

**Step 5 — Protocol in-process, then the CLI.**
- Add `pitwall-proto`, generate TS types with ts-rs into `src/gen/`, and
  replace hand-written duplicates in `types.ts` step by step.
- Run the socket server inside the app process.
- Add `pitwall-client` and a read-only CLI (`pitwall agent list`,
  `queue list`, `review changes`). The CLI ships before the daemon split.

**Step 6 — Out-of-process daemon.**
- `pitwalld` is bundled as a Tauri sidecar. The app connects, and if it
  fails, it spawns the daemon detached (`setsid`) and retries the handshake.
- The Tauri commands collapse into the `daemon_call` bridge.
- ⌘Q quits only the UI. There is a menu-bar item and a "Quit Pitwall and stop
  agents" menu entry that calls `daemon.stop`.
- `lifecycle::shutdown` and `persist::save` move out of the app's
  `RunEvent::Exit` and run only when the daemon stops.
- `hooks::serve` moves to the daemon.

**Step 7 — Approvals and mutating CLI commands** (engineer.md part 1 and 2).

**Step 8 — agw provider, in slices:**
- a) adopt and attach existing sessions: terminal, screen status, Next up,
  stop and start
- b) `SshExec`: diffs and Review
- c) create on a VM
- d) rules and hooks through artifact bundles

**Step 9 (wave 3) — SSH provider.**

Where today's modules end up:

| Today | After |
|---|---|
| `model.rs` | Views and requests → proto; `AgentRecord` → core |
| `registry.rs`, `ticker.rs`, `status.rs`, `lifecycle.rs`, `tasks.rs`, `launch.rs`, `agents.rs`, `kinds.rs` | core (`engine::*`, `kind::*`) |
| `git.rs`, git parts of `review.rs`, `worktree.rs` | `core::vcs` over `Exec` |
| `session.rs` | `core::term::TermHost` + `providers::local::pty` |
| `shell.rs` | `providers::local` (quote → core) |
| `hooks.rs` | mapping → core; socket, script, Codex install → daemon |
| `rules.rs`, `rules/*` | daemon `rules` service (uses `Exec`) |
| `scan.rs`, `projects.rs`, `project_list.rs`, `onboarding.rs` | providers' `discover` and transcript parsers (core) + daemon `onboarding` service |
| `persist.rs`, `paths.rs` | daemon `FileStore`, core `Paths` |
| `attention.rs`, `windows.rs`, `lib.rs` | app |
| `commands.rs` | app bridge (then deleted in step 6) |

---

## 7. Testing

- **`FakeProvider`** (in `pitwall-core::testing`):
  - In-memory machines and kinds.
  - `create` returns a `FakeTerm`, a pair of pipes. The test writes "agent
    output" bytes and reads what the agent was sent.
  - `resize` calls are recorded, and exit is controllable.
  - `caps` can be set per test.
  - `exec` is a `ScriptExec` with canned outputs, or a `LocalExec` on a temp
    git repo.
  - Together with `MemStore` and a manual `Clock`, Engine tests cover
    status folding, `done`→`mark_seen`, auto-send timing, task boundaries,
    worktree discovery and restarts without real processes or sleeps.
- **Provider contract suite** in `pitwall_core::testing::contract`. Each
  provider crate calls
  `contract::run(&mut impl ContractHarness)`. The harness provides a machine,
  a scratch workspace, and a "shell-like kind". The suite checks:
  - create → output arrives; write `echo pw-$X\r` → it echoes; resize →
    `stty size` matches
  - a second subscriber gets the replay
  - stop → `state() == Stopped`, plus `TermIo` EOF semantics as declared by
    `eof_is_exit`
  - start(Resume) is `Unsupported` exactly when `caps.resume` is false (no
    lying caps); the same for `attach`, `capture` and `process_cwd`
  - `exec` runs `git status` in the workspace; `read_file` and `write_file`
    round-trip; `temp_dir` is writable
  - `discover` is read-only: it changes nothing between two calls
  - Local runs in CI. The agw run is opt-in (`PITWALL_CONTRACT_AGW=<vm>`)
    because it creates and deletes a real session, so it needs the user's
    consent. The SSH run is opt-in against localhost.
- **Protocol:**
  - Golden JSON fixtures for the handshake, every method and every event.
  - Client↔daemon tests over a socketpair, including version reject, unknown
    method, multiple subscribers, view-only attach not resizing, and the
    approval flow, where an agent-descendant peer is denied the answer.
  - TS types are generated from proto, so the UI compile catches drift.
- **Existing tests move with their modules.** The detect fixtures, rules e2e,
  review and worktree temp-repo tests, launch and persist tests all keep
  running. The persist tests gain a v1→v2 migration fixture.
- **UI:** the mock API gains caps, and component tests check, for example,
  that no worktree checkbox appears when `caps.worktree` is false.

---

## 8. Risks and open questions (for the user)

1. **Daemon updates restart local agents.** A new `pitwalld` can't take over
   the PTYs of the old one without fd-passing (SCM_RIGHTS), which is
   non-trivial. Proposal: on an incompatible upgrade, ask the user, then
   restart and resume every agent by its conversation id. Is that OK for v1?
2. **How the daemon starts.** Option A: the app spawns it on demand, so
   agents survive app quit but not logout. Option B: a launchd LaunchAgent
   (login item, installed with approval). Proposal: A now, B as an opt-in
   setting.
3. **agw exec path.** Direct `ssh <admin>@<tailscale_host>` with
   ControlMaster is fast, but it assumes agw's ssh user and key setup.
   `agw vm exec <vm> --workspace` is sanctioned but too slow for a 3-second
   git poll. Is direct ssh acceptable? Can agw expose the ssh destination in
   JSON?
4. **Agent-user sessions on agw.** Their tmux server and files belong to
   another Linux user. Attach works through agw. Capture, `process_cwd` and
   git as admin may need `sudo -u`. Is that acceptable, or should diffs be
   view-only for those sessions?
5. **Remote hooks.** For agw, start with screen detection only, and add hooks
   later through an artifact bundle plus a forwarded socket. Is screen-only
   status for VM agents acceptable at first?
6. **Old app and daemon at the same time.** Until step 6, an old app and a
   new daemon could both try to own agents. The lock in §5 prevents that,
   but the old app doesn't know about the lock. Proposal: ship step 5 (which
   takes the lock in-process) before any standalone daemon build.
7. **Spaces from the CLI.** Moving the UI blob into the daemon lets
   `pitwall space …` edit it, but the CLI must then understand the UI's blob
   schema. Alternative: the daemon relays `ui.command` events to the app,
   which applies them. Which one?

## 9. Decisions (user review, 2026-10-06)

1. **Terminals are never killed by Pitwall housekeeping.** Not on rearrange
   (tiles, spaces, windows, Wall, resize — view changes only re-attach
   streams), not on daemon update, daemon crash or app quit. Killing a terminal
   loses running dev servers, ports and shells, which resume can't restore.
   → Add **`pitwall-hold`**: one tiny per-agent holder process (dtach/tmux-like)
   that owns the PTY master and the child process, exposes a minimal, frozen,
   versioned socket protocol (attach stream, write, resize, snapshot/ring
   replay, exit status), and outlives `pitwalld`. The daemon re-discovers
   holders on start (`~/.pitwall/run/hold/<agentId>.sock`) and re-attaches.
   The Local provider's `TermIo` talks to a holder, never to a PTY directly.
   A holder is replaced only if its own protocol version changes, and only
   when the user chooses, after a warning listing the affected terminals.
   Contract test: restart the daemon while a fake agent holds a listening
   port → port still open, output continues, no respawn.
2. **Daemon start:** the app starts `pitwalld` on demand; Settings has
   "Start at login" (login item) so agents' holders are adopted after reboot
   (processes themselves don't survive a reboot — resume covers that).
3. **agw exec path:** direct ssh over Tailscale, the way agw connects.
4. **agw agent users:** follow agw's own rules (check `agw guide` in that step).
5. **agw status:** screen detection first, hooks later.
6. **Spaces via CLI:** allowed through the daemon (Race Engineer can
   rearrange), the app remains the primary editor.
7. **Windows is a target** (port after Wave 2). Rules from now on: OS-specific
   code lives in a platform module behind narrow interfaces (no `cfg` in core
   logic); local IPC (hooks, daemon, holder) via `interprocess` local sockets
   (UDS on macOS/Linux, named pipes on Windows); the hook becomes a tiny
   cross-platform `pitwall-hook` binary instead of sh+curl; paths via a
   platform data-dir helper (`~/.pitwall` / `%APPDATA%\Pitwall`); shell launch
   behind a `LoginShell` abstraction (zsh/bash login on Unix, PowerShell/cmd on
   Windows); tray/badge behind the app's platform layer.
