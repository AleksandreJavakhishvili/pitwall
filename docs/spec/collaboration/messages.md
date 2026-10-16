# Collaboration: messages, replies and results

Part of the [collaboration spec](README.md).

## 7. Messages, replies, the owner, results

### 7.1 Flow of one message
1. The caller's Pitwall (or any A2A client) sends A2A
   `SendStreamingMessage` to the share's endpoint; a plain MCP client such
   as Claude Code calls the share's `ask_<slug>` tool on `/mcp`. Both reach
   the same core ([§8.3](protocol.md)).
2. The network backend has already proved who is calling ([§8.2](protocol.md)); the
   owner's Pitwall checks audience, share state, limits and queue, and writes an audit
   line. A caller outside the audience gets `404` (A2A) or doesn't see the
   tool (MCP); limit refusals are a `rejected` task or a tool result with
   `isError`.
3. It finds or creates the caller's session, starts or resumes it, waits
   until it is idle, then delivers the message as ordinary input: typed into
   the terminal as a prompt (bracketed paste + Enter, as Pitwall sends any
   prompt), or, for kinds that use their non-interactive mode ([§7.2](messages.md) b), as
   that command's prompt argument.
4. Progress goes back as A2A status updates over SSE (`submitted` with
   position → `working` → `completed` | `failed` | `canceled`), or as MCP
   progress notifications; an MCP call that outlasts `wait_seconds` returns
   `working` with the turn id, and the caller uses `check_turn`.
5. When the turn ends, the reply ([§7.2](messages.md)) goes back at once as an artifact. If
   the turn changed the copy ([§5.3](sharing.md)), the patch series follows ([§7.5](messages.md)): in the
   same turn for `write`; after the owner's Send for `write_approval`,
   with the turn staying `working` ("changes waiting for alice's OK") until
   then.

### 7.2 Capturing the reply, for any agent kind
The reply is simply **what the agent said** in that turn. Pitwall reads it
the way it already reads agents, in this order, per kind data ([§6.7](sessions.md)):
- **(a) Hooks or the agent's own session files**, where the kind has them:
  Claude Code's `Stop` hook carries `last_assistant_message`, and its
  transcript (`transcript_path`); Codex's hooks and session files.
- **(b) The kind's own non-interactive mode**, where it exists with resume:
  the session runs each turn as `claude -p --resume <id> --output-format
  json`, `codex exec resume <id> --json`, `gemini -p … --output-format json`
  and so on, and the reply is the result field of that output. The owner
  still watches the run in a pane. This is chosen per kind when (a) isn't
  available, or by the owner ([§14](plan.md) question 10).
- **(c) Otherwise, the terminal output of that turn**, which Pitwall already
  reads for status: the text printed between sending the message and the end
  of the turn, from the terminal model, with the kind's screen rules
  stripping its chrome (input box, status bar) where rules exist. Marked
  "from terminal output" in the caller's thread.
- If nothing is captured when the turn ends, the turn fails with
  `no_answer` and the owner sees the session flagged.

The end of a turn is detected as status is today: hooks, else the kind's
screen rules, else output going quiet.

### 7.3 When the agent needs the owner
No special command. Pitwall uses the signals it already has:
- **Needs you.** When a shared session turns `blocked` by Pitwall's normal
  detection (hooks such as Claude Code's `Notification`, or the kind's
  screen rules: the agent stopped and waits for input, or shows a permission
  prompt), the owner gets the usual amber flag, badge and a notification
  "api-expert: bob@example.com's conversation needs you". The caller's turn
  stays `waiting_for_owner` with the status message "waiting for alice".
- **The owner reads the conversation.** Every shared session is visible live
  in the owner view; "Notify me on every reply" can be turned on per share.
- **The owner answers like for any agent:** take over the pane and type, or
  send a prompt to the session. Turns the owner starts are part of the
  conversation: their message and the agent's reply appear in the caller's
  thread labelled "alice" ([§14](plan.md) question 14 asks about private notes).
- Whether the agent waits for the owner, tells the caller it will check, or
  just answers is up to the agent and the owner's own instructions.

### 7.4 Precedent (later phase)
An exchange the owner answered can be kept: "Save as precedent" appends the
caller's message and the owner's answer to the share's instructions (a
`PRECEDENTS.md` next to them), which new sessions read like any instruction
file. The owner sees, edits and deletes them. Never automatic.

### 7.5 How changes travel back
**Default: over the same channel as the reply.** No GitHub, no shared
remote, no push.
1. **At the end of every turn** at a "Can change code" capability, if the
   copy differs from the last delivered state, Pitwall commits the changes
   on the snapshot's branch ("turn 7 for bob@example.com"; the agent's own
   commits are kept as they are).
2. **Package.** `git format-patch --base=<owner's real base commit>
   <last delivered>..HEAD`: one patch series per turn. The `base-commit`
   line names the owner's real commit, not the snapshot's, so the caller's
   Git can find it. Summary: files, +/−.
3. **Can change code, I approve before it's sent:** the result is held. The
   owner gets "api-expert: changes for bob@example.com to review", reviews
   the turn's diff in Review (the session is an agent like others) and
   presses **Send** or **Discard**. Nothing leaves before that. Discard also
   resets the session's copy to the last delivered state, and the next
   message carries the factual line "Your changes from the previous turn
   were not sent" ([§8.3.1](protocol.md)).
4. **Send:** the patch series goes out as the turn's `changes`: an A2A
   artifact, or in the MCP result (inline up to 40,000 characters, else in
   parts through `get_changes`, [§8.3.3](protocol.md)). A Pitwall caller stores it in `collab/results/<turn>/` and shows it **in
   Review** as "changes from alice/api-expert", with the reply next to it.
   A plain client's agent gets the patch text and applies it itself
   (`git am -3` or `git apply`), as with any patch.
5. **Apply** (Pitwall callers) is the caller's click: pick the target (a new worktree at the
   base commit or at their current branch head, or an existing agent's
   worktree; never a main checkout with uncommitted changes), then
   `git am -3`. Conflicts show as normal merge conflicts in that worktree,
   in Review.
6. **Or redo on my latest code.** "Ask to redo on my code" sends a message
   in the same conversation with the caller's own text plus their commits
   since the base, limited to the share's scope, as a `text/x-patch` part
   (A2A) or the tool's `my_changes` field (MCP) (in Pitwall, after the caller approves what is sent:
   it is their code leaving the machine). The owner's Pitwall applies them to the
   session's copy before the turn and adds the factual line "Files updated:
   3 commits from bob@example.com applied". The agent just sees updated
   files and the caller's words.
7. "All changes in this conversation" is available as one series.

**Later, optional:** push a branch to a remote both can reach, or open a
pull request, as a share setting. Not in v1.
