# Collaboration: sessions and isolation

Part of the [collaboration spec](README.md).

## 6. Sessions

### 6.1 One session per caller
- **Caller identity**, decided on the owner's side from the request:
  - a person: their person id (from the `Caller` the network proved,
    [§8.2](protocol.md)), the same on all their devices and on every network once
    their identities are linked ([§15.7](network.md));
  - a Pitwall agent asking: `login` + `agent`, where `agent` is the
    `Pitwall-Agent` label Bob's Pitwall sends ([§8.2](protocol.md)). The owner trusts
    Bob's Pitwall to label Bob's agents honestly, as it trusts Bob;
  - a plain MCP client (Claude Code or any other): `login` only. All of
    that person's plain clients share one conversation unless they pass
    `new_conversation`.
- **Same caller → same session**, resumed with the kind's own resume: every
  message from that caller continues one conversation. A new conversation
  is explicit: "New conversation" in the Ask panel, `--new` on the CLI,
  `new_conversation: true` on the MCP tool ([§8.3](protocol.md)). The old one is kept until
  `keepSessionsDays`.
- **Different caller → a new, fresh session** of the same agent kind, for
  every kind: its own copy, sandbox, process and agent session id. Nothing
  from Bob's session is visible to Carol's.
- A session is an ordinary engine agent with
  `AgentRecord.shared = { share, caller }`, grouped under the shared agent in
  the owner's sidebar ([§11](ui.md)).

### 6.2 The session's copy: only what was picked
- **Where.** `<data>/collab/shares/<share>/s/<session>/work/`, outside the
  user's projects.
- **A snapshot repository, not a worktree.** Pitwall runs `git init` there
  and writes one commit, "snapshot of main @ 3f2a1c9", holding only the
  scoped tracked files minus exclusions (plus the uncommitted changes if
  chosen). It does **not** link to the owner's repository: no `.git` pointer
  back, no object database, no other branches, no history. So `git log -p`,
  `git show HEAD~3:.env` or reading `.git` can't reach anything outside the
  scope, whatever the capability.
  - Blobs are content-addressed, so every file in the snapshot has the same
    blob id as in the owner's (and the caller's) repository. That is what
    lets `git am -3` do three-way merges on the caller's side ([§7.5](messages.md)).
  - "Include git history" is a later option ([§14](plan.md) question 12).
- **Skills.** The agent's assigned skills come along: Pitwall generates the
  agent's skills into the session folder the same way it does for the agent
  scope (rules.md, `rulesync generate --features skills`), and tracked
  project skills are in the snapshot if inside the scope. Kinds without
  skills support (rules.md: Gemini CLI, Aider) get their rules file only.
  The Share dialog lists which skills come along.
- **Refresh.** A resumed session keeps its copy. "Update copies" in the owner
  view (or a scope change) rebuilds idle copies at the current `ref`. With
  "Can change code", undelivered turn changes are kept on top when they
  still apply, else the caller sees "files updated; earlier changes kept as
  patch".
- **Cleanup.** Removing a session deletes its folder.

### 6.3 Isolation: the session's surroundings, not the agent's flags
Every shared session's **whole process tree** runs inside an OS-level
sandbox that Pitwall sets up around the agent process, for every agent
kind. The agent keeps all its own tools and behaves as it always does; only
what its processes can reach is limited.

#### Two layers, and what each can see
- **The OS layer restricts effects**, for every kind: which **paths** can be
  read or written, which **network** destinations can be reached, and which
  **programs** may be executed (e.g. allow `git`, `rg`, `cargo`; deny
  `curl`, `ssh`, `scp`). It works on system calls, so it applies equally to
  a file the agent reads with its built-in tool and to a shell command it
  runs.
- **It cannot see an agent's built-in tools by name.** "Read", "Edit",
  "WebFetch" or "Bash" run inside the agent's own process; the OS only sees
  the files they open and the connections they make. So "may not use its web
  tool" is not an OS rule; "may not reach the network" is.
- **The agent's native tool settings are the second layer** ([§6.8](sessions.md)),
  used where the kind has them, written in the kind's own format (flags or
  its own settings file in the session folder). They mainly let the agent
  *know* its limits (it doesn't try to write and fail), and add a second
  wall. Nothing depends on them, so any kind can be shared. This stays
  within the native-behaviour principle: Pitwall only uses settings the
  agent already understands.
- **"Programs it may run"** in the Share dialog ([§5.4](sharing.md) step 5) is an
  OS-layer exec rule: an allowlist (default: the agent's own binary and
  runtimes, the shell, `git`, common read-only tools such as `ls`, `cat`,
  `rg`, `grep`, `find`, plus the project's build tools the owner ticks) or a
  denylist (default denied: `curl`, `wget`, `ssh`, `scp`, `nc`,
  `osascript`). Where the kind has per-command rules (Claude Code `Bash(…)`,
  Cursor `Shell(…)`, Copilot `shell(…)`, opencode and Amp bash patterns,
  Gemini `run_shell_command(…)`), Pitwall writes the same list there too, so
  the agent sees refusals as its own.

| | Allowed | Everything else |
|---|---|---|
| Files | Read the copy (`work/`); write it only at "Can change code"; read and write the session's own `HOME` and temp folders; read the system and the agent's own install (paths from kind data); read and write the agent account's config folder (below) | Denied: the owner's home, projects, Pitwall's data folder and sockets, other sessions |
| Programs | The "Programs it may run" list | Denied |
| Network | Pitwall's per-share egress proxy on localhost only; the proxy allows the kind's model hosts (kind data, e.g. `api.anthropic.com`) plus hosts the owner added | Denied |
| Environment | Built by Pitwall from nothing: `PATH` (system + the agent's install), `HOME` = session folder, `TMPDIR`, `LANG`, `TERM`, `HTTPS_PROXY`/`HTTP_PROXY`, the agent account's config variable, what the owner chose to pass | Nothing else: no `SSH_AUTH_SOCK`, no tokens, no `PITWALL_*` |
| Pitwall | The hook endpoint, for hook events only ([§8.5](protocol.md)) | `pitwalld.sock` and every other socket |

#### One abstraction, one backend per OS
A `SessionSandbox` in `pitwall-core::platform` (beside the existing
`unix.rs` / `macos.rs` / `linux.rs` / `windows.rs` split) takes one policy:
`{ read: [paths], write: [paths], exec: allow|deny list, egress: proxy
port, env }` and wraps the agent's argv for the holder. Each OS provides a
backend; the rest of Pitwall never sees which.

**macOS: Seatbelt (C1a).**

| Option | What it gives | Cost | Verdict |
|---|---|---|---|
| **Seatbelt profile (`sandbox-exec`)** | Deny-by-default file, exec, network and IPC rules per process, inherited by children; the mechanism Codex documents for its own macOS sandbox, and Gemini CLI offers as `SEATBELT_PROFILE` | No admin rights, no install. `sandbox-exec` is marked deprecated in its man page but still works; the profile language is undocumented | **Recommended first** |
| Separate macOS user per share | Strong file isolation by Unix permissions, its own Keychain | Admin to create the user, a privileged helper to start a terminal as that user, the agent logged in as that user | Later option |
| Container or VM (Docker, Apple's container tooling, agw VM) | Full isolation, Linux parity | Linux builds of the agents, logins inside, a runtime to install | Later (with agw) |

Pitwall writes a profile per session: `(deny default)`, `process-exec`
only for the programs list, the file rules as `subpath` rules, network
outbound only to the proxy's localhost port and the hook endpoint socket.
It starts the agent as `sandbox-exec -f <profile> <agent argv>` inside the
holder. Because the sandbox is inherited by every descendant, a process that
double-forks and detaches is still inside it.

**A quick demo already done** (macOS, `sandbox-exec`, a profile denying
`~/` and a secrets file, allowing a copy folder read-only, denying network):
reading the secrets file was blocked, listing `~/.ssh` was blocked, writing
into the read-only copy was blocked, a detached double-forked child trying
to read the secret was blocked, and reading the copy worked. The network
block could not be judged in isolation because the test environment had its
own sandbox. This is a smoke test, not the C1a test suite ([§12](plan.md)).

Still to confirm in C1a (**unverified**): which Mach services each agent
needs (Keychain for agents that keep their login there, DNS), how agents
behave behind an HTTP proxy, behaviour on macOS 26, and nesting with an
agent's own Seatbelt sandbox (Claude Code's Bash sandbox, Codex's,
Gemini's); where nesting fails, the agent's own sandbox is turned off and
the outer one stays.

**Linux: bubblewrap, plus Landlock where available (C3).**
- `bwrap` with a new mount namespace that binds only the copy (read-only or
  read-write), the session `HOME` and temp, and the system and agent install
  read-only; a new network namespace in which the only reachable endpoint is
  the egress proxy (bound into the namespace through a Unix socket, the
  approach Claude Code takes with `socat`); `--unshare-pid`, `--die-with-parent`,
  a clean environment. Exec rules: only the allowed programs are bound in,
  plus Landlock execute rules where the kernel supports them.
- This is the mechanism both Codex and Claude Code use on Linux and WSL2.
- **Catches, which Pitwall detects and explains in Settings:**
  - `bubblewrap` (and `socat` for the proxy relay) must be installed
    (`apt install bubblewrap socat`).
  - Ubuntu 24.04 and later restrict unprivileged user namespaces through
    AppArmor (`kernel.apparmor_restrict_unprivileged_userns = 1`), so `bwrap`
    fails until an AppArmor profile for `/usr/bin/bwrap` is loaded. Pitwall
    checks the sysctl, and if it is `1` shows the exact profile and
    commands (as Codex's and Claude Code's docs do), never disables the
    restriction itself. Ubuntu 25.04's packaged profile works without extra
    setup per Codex's docs.
  - Inside containers `bwrap` may fail with "Operation not permitted":
    hosting shares from a container is unsupported.

**Windows (C5).**
1. **WSL2 route first.** Shared sessions run inside a WSL2 distribution
   with the agent installed there, using the Linux backend unchanged. Codex
   and Claude Code document the same route (their Linux sandboxes work in
   WSL2). Pitwall's holder starts `wsl.exe -d <distro> -- bwrap …`; the copy
   lives in the distro's filesystem.
2. **Native later:** an AppContainer or restricted-token backend, after an
   evaluation (Codex ships a native Windows sandbox for PowerShell, which
   shows it is possible).
3. **Until then** Windows machines can ask but not host shares.

#### The agent account
The agent needs its own login (model access). Its config folder is readable
inside the sandbox, and that folder may hold the account's other
conversations. So a share **requires a separate account for sharing**
(decided 2026-10-16; Pitwall's accounts already support several logins per
kind, README "Accounts"), so only shared sessions live there. Going live,
or widening to an account, is refused when the share would run on the
kind's default (the owner's main) account, or on the account the owner's
own agent (the one being shared) runs on, since that agent's conversations
live in its folder. Moving a live share onto such an account is refused the
same way, before any approval is asked. The Share dialog offers to pick or
add one. Guest tries may use any account, with an amber warning. Accounts
are stored by id; a name given on the CLI (`--account Work`) is resolved to
its id, never left to fall back to the default.

**Gemini CLI** keeps its login in `~/.gemini` and has no variable to move
it, so a sandboxed session (its own `HOME`) has no login. A Gemini share
goes live only when the owner passes `GEMINI_API_KEY` explicitly in the
access step and it has a value; `~/.gemini` is never mounted or copied.
Pitwall reads the key from its own environment, else from the login shell
(an app started from the Dock has no shell variables). `GOOGLE_API_KEY` is
not offered: on its own it is Gemini's Vertex AI setting and doesn't sign in.

### 6.4 Starting point: fresh or an approved summary
- **Fresh** (default), for every kind: the session starts with no history,
  in its copy, with the owner's instructions ([§6.5](sessions.md)).
- **Summary:** the owner approved a text ([§5.4](sharing.md) step 4). Pitwall makes a
  **base session** once: a fresh session in a snapshot whose first message
  is the approved summary. New caller sessions then **fork the base** where
  the kind can: Claude Code `claude --resume <base> --fork-session`, Codex
  `codex fork <base>`; Pitwall learns the new id as it does today (Claude
  Code's `SessionStart` hook reports `source: "fork"`). Other kinds get the
  approved summary as their first message instead: the same text, sent like
  any message.
- Neither option lets a shared session see the owner's own conversation.
  Forking the full conversation is not offered in v1 ([§14](plan.md) question 4).
- A new summary affects new sessions only; existing ones show "from
  briefing 2 (current: 3)", and the owner can reset them.

### 6.5 Instructions (soft, the owner's own text)
The only instructions a shared session gets are the **owner's own text**:
the role and the off-limits topics from the Share dialog, written into the
files agents read on their own (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, …)
in the session folder, like the Race Engineer's (engineer.md), plus the
agent's assigned skills ([§6.2](sessions.md)). The dialog starts from a short draft the
owner edits; what gets written is exactly what the owner saved. Pitwall adds
no rules of its own: no reply protocol, no commands to run, no labels to
follow. It is the **soft** layer and is labelled so. Pitwall never edits an
caller's message.

### 6.6 Lifecycle, caps and queueing
- **States:** `new → starting → ready → working → ready … → idle-stopped
  → (resume) starting …`, plus `blocked` (needs the owner, [§7.3](messages.md)) and
  `closed`.
- **One launch per share's settings.** A session always runs with the same
  sandbox and flags for its share's capability; nothing switches per turn.
  A settings change applies from the next message: the session is restarted
  with the kind's resume.
- **Idle stop.** After `idleStopMin` with nothing in flight, the process
  stops. Its agent session id and copy are kept; the next message resumes it
  with the kind's resume. Kinds without resume (Aider, custom commands) are
  kept running instead, up to the running cap; when one must stop, its
  memory is lost and the caller sees "new conversation (the agent can't
  resume)".
- **Running cap.** At most `sessionsRunning` processes per share. When a
  message needs one and none is free, the least recently used idle session
  is stopped; when all are busy, the message waits in the queue (FIFO,
  `queue` long; "2nd in line" in progress notifications). A full queue is
  refused with `busy`.
- **One message at a time per session.** A second one from the same caller
  waits behind the first, in the same session.
- **Global cap.** Settings → Collaboration → "At most N shared sessions on
  this machine" (default 4).
- **Turn timeout.** A turn over `turnTimeoutMin` is interrupted (as Stop
  for a turn); the turn fails with `timeout` plus whatever reply exists.
- **Quota.** Messages per caller per day, reset at local midnight on the
  owner's machine. Over quota → refused with `quota` and the reset time.
  All limits, including the agent's own vendor limits, are in [§5.8](sharing.md).

### 6.7 Per-kind facts are data
Everything kind-specific lives in the kind's TOML (`[share]` table), like
the rest of an agent kind (architecture.md §2.6): resume and fork commands,
how to read the reply ([§7.2](messages.md)), its non-interactive mode, model hosts for
the proxy, install paths for the sandbox, and how to write its native
controls ([§6.8](sessions.md)). A kind without a `[share]` table can still be shared
with defaults: fresh sessions, terminal-output replies, no resume after a
stop, OS layer only.

| Kind | Reply from hooks / own session files | Resume (interactive) | Fork |
|---|---|---|---|
| Claude Code | `Stop` hook `last_assistant_message`; transcript | `--resume <id>` | `--resume <id> --fork-session` |
| Codex | its session files: each turn ends with an event carrying the last message; the same file gives the conversation id (checked with codex-cli 0.160.1) | `codex resume <id>` | `codex fork`, `codex exec fork` |
| Gemini CLI | none; each turn runs non-interactively, `gemini -p … --output-format json`, first with `--session-id <id>`, then `--resume <id>` (checked with Gemini CLI 0.64.0) | `--resume <id>` | **unverified** |
| opencode | none known | yes (Pitwall today) | `opencode run --fork` (third-party docs, **unverified**) |
| Aider | none known | no | no |
| Cursor Agent | none known | `--resume <chatId>` | **unverified** |
| Copilot CLI | none known | `--resume`, `--continue` | **unverified** |
| Qwen Code | none known | yes (Pitwall today) | **unverified** |
| Amp | none known | `amp threads continue` | **unverified** |
| Custom command, shell | — | no | no |

### 6.8 Native controls per agent kind
What each agent offers itself, checked against its own docs where possible
(sources in [references.md](references.md); "—" means none found). These are
the **second layer** ([§6.3](sessions.md)); the OS layer applies to all of them.

| Kind | Tool control (per tool, per command) | Folder control | Own sandbox | Network control | Non-interactive + resume |
|---|---|---|---|---|---|
| **Claude Code** | `--tools`, `--allowedTools`, `--disallowedTools`; rules like `Bash(git *)`, `Read(…)`; permission modes `default`, `acceptEdits`, `plan`, `auto`, `dontAsk`, `bypassPermissions` | Working dir; `--add-dir`; `permissions.additionalDirectories`; `blockReadsOutsideWorkingDirectories` | Bash sandbox (shell commands only; file tools follow permission rules): macOS, Linux and WSL2 (bubblewrap + socat); **not native Windows** | Sandbox proxy with `network.allowedDomains` (shell commands); `WebFetch` via permission rules | `claude -p --resume <id> --output-format json` |
| **Codex** | Coarse: `--sandbox read-only|workspace-write|danger-full-access`, `approval_policy` (`on-request`, `never`, granular); beta permission profiles (filesystem `read`/`write`/`deny`) | Workspace; `sandbox_workspace_write.writable_roots` | Yes: Seatbelt (macOS), bubblewrap (Linux, WSL2; AppArmor profile needed on Ubuntu 24.04), native Windows sandbox (PowerShell) | `sandbox_workspace_write.network_access` (off by default); `shell_environment_policy` for env | `codex exec resume <id> --json` |
| **Gemini CLI** | `tools.core` (allowlist), `tools.exclude`, `tools.allowed` (no prompt; e.g. `run_shell_command(git)`), `tools.confirmationRequired`; policy files (`policyPaths`); approval mode `default`, `auto_edit`, `plan`, `yolo` | Working dir; `context.includeDirectories`; `tools.sandboxAllowedPaths` | Yes: `tools.sandbox` / `GEMINI_SANDBOX` = docker, podman, lxc, `windows-native`, or Seatbelt on macOS (`SEATBELT_PROFILE`: `permissive-open`, `restrictive-open`, `strict-open`, `strict-proxied`, custom) | `tools.sandboxNetworkAccess` (default false) | `gemini -p … --output-format json` with `--session-id`, then `--resume` (checked, 0.64.0) |
| **opencode** | `permission` block: keys `read`, `edit`, `glob`, `grep`, `bash`, `task`, `skill`, `lsp`, `question`, `webfetch`, `websearch`, `external_directory`, `doom_loop`; values `allow`/`ask`/`deny`; bash patterns with `*`/`?`, last match wins; per-agent overrides | Project dir; `external_directory` rule | — (none documented) | `webfetch`/`websearch` permissions only | `opencode run -s <id> --format json` (third-party docs, **unverified**) |
| **Aider** | — (edits files it is given; `--yes` auto-confirms) | Files added to the chat; `--read` for read-only files | — | — | `--message` runs one message and exits; no resume |
| **Cursor Agent** | `permissions.allow` / `deny` in `~/.cursor/cli-config.json` or `.cursor/cli.json`: `Shell(cmd)`, `Read(glob)`, `Write(glob)`, `WebFetch(domain)`, `Mcp(server:tool)`; deny wins | Path globs in `Read`/`Write` rules | — (none documented) | `WebFetch(domain)` rules | `-p` (print) with `--output-format json`, `--force` to apply edits; `--resume <chatId>` (from the JSON `session_id`) |
| **Copilot CLI** | `--allow-tool` / `--deny-tool` with kinds `shell`, `write`, `read`, `url`, `memory`, MCP servers; `shell(git:*)`, `shell(npm test)` | `--add-dir` (repeatable); `--allow-all-paths` turns checks off | — (none documented for the agent; `copilot sandbox ca` manages a proxy CA, purpose **unverified**) | `--allow-url` / `--deny-url` (deny wins) | `-p` runs and exits; `--resume`/`--continue` with `-p` **unverified** |
| **Qwen Code** | Approval modes (Plan read-only, Ask permissions/default, Auto Edit, Auto with a classifier, YOLO; `tools.approvalMode`); per-tool settings **unverified** (likely as Gemini CLI, which it derives from) | **unverified** | **unverified** | **unverified** | **unverified** |
| **Amp** | `amp.permissions`: ordered rules, first match wins, actions allow / reject / ask / delegate; match on tool and arguments (e.g. Bash `cmd: '*git*push*'`); plugins with a `tool.call` hook recommended over legacy rules | — (none found) | — (none found) | via permission rules on its web tools (**unverified** details) | `amp -x` (execute mode); `amp threads continue -x "…"` |

How Pitwall uses this: for a Read-only share it writes the kind's native
read-only setting where one exists (Claude Code `dontAsk` with read tools,
Codex `--sandbox read-only`, Gemini `plan` approval mode, opencode
`edit: deny`, Cursor `Write(**)` deny, Copilot `--deny-tool write`, Amp a
reject rule for edit tools); for the programs list it writes the kind's
per-command rules (§6.3). Where the agent has its own sandbox that would
nest badly with the OS layer, the agent's sandbox is left off (Claude Code's
sandbox setting, Codex `--sandbox danger-full-access`,
`GEMINI_SANDBOX=false`).

Folder trust: every session runs in a new copy, which agents treat as an
unknown folder. Codex asks "Trust this folder?" in each one, so the session
trusts its copy **for that run only** (a per-run config override; nothing
is written to the account's config). Gemini's headless mode refuses
untrusted folders, so its turns pass `--skip-trust`. Claude Code's
first-run trust prompt in a new copy is still **unverified**.
