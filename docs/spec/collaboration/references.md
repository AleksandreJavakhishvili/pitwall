# Collaboration: references

Part of the [collaboration spec](README.md).

Checked on 2026-10-16. "Unverified" items are marked in the text.

**Tailscale**
- WhoIs response (`Node`, `UserProfile`, `CapMap`; Node and UserProfile
  never nil on success): https://pkg.go.dev/tailscale.com/client/tailscale/apitype#WhoIsResponse
- Grants, `app` field, capability names `domain/name`, parameters as opaque
  JSON: https://tailscale.com/kb/1324/grants
- Application capabilities, golink example (`tailscale.com/cap/golink`,
  `{"admin": true}`), apps read them via tsnet, LocalAPI, `tailscale whois`
  or `tailscale serve` headers: https://tailscale.com/kb/1537/grants-app-capabilities
- Golink reading `whois.CapMap`: https://tailscale.com/blog/grants-ga
- tsidp uses LocalAPI WhoIs (user, node, app capabilities) and the
  `tailscale.com/cap/tsidp` capability: https://tailscale.com/docs/features/tsidp,
  https://tailscale.com/blog/building-tsidp
- No broadcast or multicast over Tailscale (staff answer, older; no newer
  docs found saying it changed): https://forum.tailscale.com/t/broadcast-multicast-support/156
- `tailscale status --json` fields used for discovery: as listed and caveated
  in tailnet-peers.md §3.

**MCP** (Model Context Protocol): https://modelcontextprotocol.io/specification
- Streamable HTTP transport (JSON-RPC over `POST`, JSON or SSE answers,
  `Mcp-Session-Id`), tools (`tools/list`, `tools/call`, `inputSchema`,
  `outputSchema`, `structuredContent` with a text copy, `isError`,
  annotations, `listChanged`), progress and cancellation notifications,
  authorization optional for HTTP. Not rechecked field by field: golden
  fixtures at implementation.
- Claude Code as a client (checked 2026-10-16; C1a interop with Claude Code
  2.1.301 showed it speaks MCP 2026-07-28 and needs `ttlMs`/`cacheScope` on
  `tools/list`):
  https://code.claude.com/docs/en/mcp
  - `claude mcp add --transport http <name> <url>` (optional `--header`).
  - `MCP_TIMEOUT` (startup), `MCP_TOOL_TIMEOUT` and per-server `timeout`
    (hard limit per call; default about 28 hours); progress notifications
    don't extend it.
  - `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`: idle window 5 minutes for HTTP
    servers by default.
  - Output: warning above 10,000 tokens, limit 25,000 (`MAX_MCP_OUTPUT_TOKENS`);
    text results over 50,000 characters are saved to a file unless the tool
    sets `anthropic/maxResultSizeChars`.
  - `list_changed` for tools refreshes the list without reconnecting
    (tools only in `-p` mode).

**A2A** (specification 1.0.0), the Pitwall front ([§8.3.2](protocol.md)):
https://a2a-protocol.org/latest/specification/
- Agent Card required fields (`name`, `description`, `supportedInterfaces`,
  `version`, `capabilities`, `defaultInputModes`, `defaultOutputModes`,
  `skills`), `AgentSkill`, `AgentInterface` (`url`, `protocolBinding`:
  `JSONRPC`, `GRPC`, `HTTP+JSON`; `protocolVersion`); discovery by
  `/.well-known/agent-card.json`, catalogs or pre-configured card URLs.
- `contextId` (server may generate; groups tasks into a conversation); a
  server may return a Message or a Task; `metadata` on messages.
- Task states, including input-required (client continues) and
  auth-required (in-task authorization delegated to the client; the state
  by itself is not authorization).
- `Part` (exactly one of `text`, `raw`, `url`, `data`; `mediaType`,
  `filename`), `Artifact` (`artifactId`, `name`, `parts`), artifact update
  events (`append`, `lastChunk`).
- HTTP+JSON endpoints (`POST /message:send`, `POST /message:stream` with
  `text/event-stream`, `GET /tasks/{id}`, `GET /tasks`,
  `POST /tasks/{id}:cancel`, `POST /tasks/{id}:subscribe`), body media type
  `application/a2a+json`, `A2A-Version` header; push notifications
  optional.

**Claude Code**
- `-p`, `--resume`, `--fork-session`, `--session-id`, `--output-format`,
  `--permission-mode` values, `--tools`, `--allowedTools`,
  `--disallowedTools`, `--settings`, `--append-system-prompt`:
  https://code.claude.com/docs/en/cli-reference
- Permission modes (`plan`: reads plus classifier-approved commands, edits
  blocked until a plan is approved; `dontAsk`: reads inside working
  directories and pre-approved tools, everything else denied):
  https://code.claude.com/docs/en/permission-modes
- Hooks: `Stop` `last_assistant_message`; `SessionStart` `source` values
  including `"fork"`: https://code.claude.com/docs/en/hooks
- Bash sandbox (shell commands only; reads default
  to most of the machine; network through a proxy with an empty allowlist;
  environment inherited unless scrubbed; not on native Windows):
  https://code.claude.com/docs/en/sandboxing

**Codex CLI**
- `codex resume`, `codex exec resume`, `codex fork`, `codex exec fork`,
  `--sandbox read-only|workspace-write|danger-full-access`,
  `--ask-for-approval`, `codex exec --json`:
  https://learn.chatgpt.com/docs/developer-commands?surface=cli (redirect
  target of https://developers.openai.com/codex/cli/reference)
- Config: `sandbox_mode`, `sandbox_workspace_write.network_access`,
  `shell_environment_policy`, `approval_policy`, beta permission profiles:
  https://learn.chatgpt.com/docs/config-file/config-reference
- Sandboxing (Seatbelt on macOS, bubblewrap on Linux and WSL2, Windows
  sandbox): https://learn.chatgpt.com/docs/sandboxing

**Other agents** (native controls table, [§6.8](sessions.md))
- Claude Code on Linux and WSL2: sandbox needs `bubblewrap` and `socat`;
  Ubuntu 24.04+ AppArmor blocks bwrap's user namespaces until a `bwrap`
  profile is loaded (`sysctl kernel.apparmor_restrict_unprivileged_userns`):
  https://code.claude.com/docs/en/sandboxing
- Codex on Linux/WSL2 uses bubblewrap (install it; bundled helper needs
  unprivileged user namespaces; Ubuntu 24.04 needs the AppArmor profile,
  25.04 works as packaged); native Windows sandbox in PowerShell; WSL2 uses
  the Linux sandbox: https://learn.chatgpt.com/docs/sandboxing
- Gemini CLI configuration (`tools.core`, `tools.exclude`, `tools.allowed`
  incl. `run_shell_command(git)`, `tools.confirmationRequired`,
  `context.includeDirectories`, `tools.sandbox` / `GEMINI_SANDBOX` docker,
  podman, lxc, windows-native, `SEATBELT_PROFILE` profiles,
  `tools.sandboxAllowedPaths`, `tools.sandboxNetworkAccess`,
  `general.defaultApprovalMode`, `--approval-mode=yolo`, `policyPaths`):
  https://geminicli.com/docs/reference/configuration
- Gemini CLI headless mode (`-p`/`--prompt`, `--output-format json` /
  stream JSON): https://geminicli.com/docs/cli/headless.md. `-p` with
  `--session-id` and `--resume`, `--skip-trust`, and the error JSON on
  stderr were checked against Gemini CLI 0.64.0 with a local fake model
  server (no account).
- opencode permissions (keys, `allow`/`ask`/`deny`, bash patterns, last
  match wins, per-agent overrides): https://opencode.ai/docs/permissions/.
  `opencode run -s/--session`, `-c`, `--fork`, `--format json`: third-party
  references only (https://learnopencode.com/en/appendix/cli), **unverified**.
- Aider scripting (`--message`: one message, apply edits, exit; `--yes`):
  https://aider.chat/docs/scripting.html
- Cursor Agent CLI permissions (`Shell()`, `Read()`, `Write()`,
  `WebFetch()`, `Mcp()`; deny wins; `~/.cursor/cli-config.json`,
  `.cursor/cli.json`): https://cursor.com/docs/cli/reference/permissions.
  Headless `-p`, `--force`, `--output-format json` with `session_id`:
  https://cursor.com/docs/cli/headless,
  https://docs.cursor.com/en/cli/reference/output-format; `--resume
  <chatId>` from a forum report (https://forum.cursor.com/t/cli-headless-doesnt-work-with-ls-no-way-to-get-a-list-of-chat-ids/151821).
- GitHub Copilot CLI (`-p`, `--allow-tool`/`--deny-tool` with `shell`,
  `write`, `read`, `url`, `memory`, MCP; `shell(git:*)`; `--add-dir`;
  `--allow-all-paths`; `--allow-url`/`--deny-url`, deny wins for URLs):
  https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-programmatic-reference;
  `--resume`/`--continue`: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference
- Qwen Code approval modes (Plan, Ask permissions/default, Auto Edit, Auto,
  YOLO; `tools.approvalMode`):
  https://qwenlm.github.io/qwen-code-docs/en/users/features/approval-mode/
- Amp permissions (`amp.permissions`, ordered rules, first match wins,
  allow/reject/ask/delegate, Bash `cmd` patterns, plugins recommended):
  https://ampcode.com/permissions,
  https://ampcode.com/manual/appendix/legacy-permissions-rules.txt; execute
  mode `amp -x` and `amp threads continue -x`:
  https://ampcode.com/docs/cli/execute-mode, https://ampcode.com/news/amp-x
- Ubuntu's AppArmor user-namespace restriction (24.04):
  https://documentation.ubuntu.com/release-notes/24.04/,
  https://discourse.ubuntu.com/t/understanding-apparmor-user-namespace-restriction/58007

**macOS sandbox**
- `sandbox-exec` deprecated in its man page but working on recent macOS;
  deny-by-default profiles with file, network and exec rules:
  https://jmmv.dev/2019/11/macos-sandbox-exec.html,
  https://gastown.dev/docs/research/macos-sandbox-exec (secondary sources).


**Versions checked in C1a** (each against the integrated core, or a
local fake model server where noted; no owner account used except Claude
Code's `-p --model haiku` interop run)
- codex-cli 0.160.1 (`exec` and the interactive screen; session-file
  replies, per-run trust; fake model server)
- Gemini CLI 0.64.0 (headless turns, resume; fake model server)
- A2A Python SDK (`a2a-sdk`) 1.2.2: send and stream over HTTP+JSON
- MCP Inspector CLI 2.11.0: list tools, call a share's tool
- Claude Code 2.1.301: `claude -p --model haiku` with an HTTP MCP config

**Pitwall**
- Skills library and per-project / per-agent assignment: [rules.md](../rules.md)
  (on the `rules-scopes` work).

**Unverified** (to check before the phase that needs it)
- Seatbelt details per agent: Mach services (Keychain, DNS), proxy
  behaviour, nesting with Claude Code's and Codex's own Seatbelt sandboxes,
  behaviour on macOS 26 ([§6.3](sessions.md)).
- Reply capture and fork for opencode,
  Cursor Agent, Copilot CLI, Qwen Code, Amp ([§6.7](sessions.md)).
- Native controls not found in the agents' own docs ([§6.8](sessions.md)): Qwen Code
  per-tool settings, folders, sandbox and network; Copilot CLI
  allow/deny precedence for tools and `--resume` with `-p`; Amp folder and
  network controls; opencode `run` flags; whether opencode, Cursor Agent,
  Copilot CLI or Amp have their own OS sandbox (none documented).
- Linux backend details: Landlock exec rules on the kernels we target,
  the proxy relay inside a network namespace, `bwrap` inside WSL2
  ([§6.3](sessions.md)).
- Claude Code's folder-trust prompt in a new copy, and whether Gemini CLI
  honours the egress proxy inside the sandbox ([§6.3](sessions.md)).
- `ureq` SSE reading over long streams in practice ([§8.1](protocol.md)).
- Whether A2A and MCP clients accept plain `http://` on a tailnet address
  ([§8.1](protocol.md), [§14](plan.md) question 9).
- Whether progress notifications reset Claude Code's idle timer, and tool
  timeouts and output limits in other MCP clients ([§8.3.3](protocol.md), [§14](plan.md)
  question 17).
- How each agent kind shows its vendor usage limit and reset time on
  screen or in hooks ([§5.8](sharing.md)).
- The exact grant shape (`dst`) for a Pitwall node to see a caller's app
  capability via LocalAPI `whois` ([§4.8](teams.md)), and how tagged devices appear in
  `UserProfile`.
- Tailscale multicast: only an older staff forum answer ([§3](teams.md)).
- Vendor terms for letting others use an agent subscription ([§14](plan.md) question 6).
