# Collaboration: messages, replies and results

Part of the [collaboration spec](README.md).

## 7. Messages, replies, the owner, results

### 7.1 Flow of one message
1. The asker's Pitwall (or any A2A client) sends A2A `SendStreamingMessage`
   (or `SendMessage`) to the share's endpoint ([§8.3](protocol.md)).
2. The owner's Pitwall checks `whois`, the instance-key extension if
   present, audience, share state, quota and queue, and writes an audit
   line. Refusals before work are HTTP errors (not allowed) or a task in
   `rejected` (busy, over quota).
3. It finds or creates the asker's session, starts or resumes it, waits
   until it is idle, then delivers the message as ordinary input: typed into
   the terminal as a prompt (bracketed paste + Enter, as Pitwall sends any
   prompt), or, for kinds that use their non-interactive mode ([§7.2](messages.md) b), as
   that command's prompt argument.
4. Progress streams back as A2A status updates: `submitted` (queued, with
   position) → `working` → `completed` | `failed` | `canceled`.
5. When the turn ends, the reply ([§7.2](messages.md)) goes back at once as an artifact. If
   the turn changed the copy ([§5.3](sharing.md)), the patch series follows ([§7.5](messages.md)): in the
   same task for `write`; after the owner's Send for `write_approval`, with
   the task staying `working` until then.

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
  "from terminal output" in the asker's thread.
- If nothing is captured when the turn ends, the task fails with
  `no_answer` and the owner sees the session flagged.

The end of a turn is detected as status is today: hooks, else the kind's
screen rules, else output going quiet.

### 7.3 When the agent needs the owner
No special command. Pitwall uses the signals it already has:
- **Needs you.** When a shared session turns `blocked` by Pitwall's normal
  detection (hooks such as Claude Code's `Notification`, or the kind's
  screen rules: the agent stopped and waits for input, or shows a permission
  prompt), the owner gets the usual amber flag, badge and a notification
  "api-expert: bob@example.com's conversation needs you". The asker's task
  stays `working` with the status message "waiting for alice".
- **The owner reads the conversation.** Every shared session is visible live
  in the owner view; "Notify me on every reply" can be turned on per share.
- **The owner answers like for any agent:** take over the pane and type, or
  send a prompt to the session. Turns the owner starts are part of the
  conversation: their message and the agent's reply appear in the asker's
  thread labelled "alice" ([§14](plan.md) question 14 asks about private notes).
- Whether the agent waits for the owner, tells the asker it will check, or
  just answers is up to the agent and the owner's own instructions.

### 7.4 Precedent (later phase)
An exchange the owner answered can be kept: "Save as precedent" appends the
asker's message and the owner's answer to the share's instructions (a
`PRECEDENTS.md` next to them), which new sessions read like any instruction
file. The owner sees, edits and deletes them. Never automatic.

### 7.5 How changes travel back
**Default: over the same A2A channel as the reply.** No GitHub, no shared
remote, no push.
1. **At the end of every turn** at a "Can change code" capability, if the
   copy differs from the last delivered state, Pitwall commits the changes
   on the snapshot's branch ("turn 7 for bob@example.com"; the agent's own
   commits are kept as they are).
2. **Package.** `git format-patch --base=<owner's real base commit>
   <last delivered>..HEAD`: one patch series per turn. The `base-commit`
   line names the owner's real commit, not the snapshot's, so the asker's
   Git can find it. Summary: files, +/−.
3. **Can change code, I approve before it's sent:** the result is held. The
   owner gets "api-expert: changes for bob@example.com to review", reviews
   the turn's diff in Review (the session is an agent like others) and
   presses **Send** or **Discard**. Nothing leaves before that. Discard also
   resets the session's copy to the last delivered state, and the next
   message carries the factual line "Your changes from the previous turn
   were not sent" ([§8.3.1](protocol.md)).
4. **Send:** the patch series goes out as an A2A artifact `changes` ([§8.3](protocol.md)),
   with the summary as a data part. The asker's Pitwall stores it in
   `collab/results/<task>/` and shows it **in Review** as "changes from
   alice/api-expert", with the reply next to it.
5. **Apply** is the asker's click: pick the target (a new worktree at the
   base commit or at their current branch head, or an existing agent's
   worktree; never a main checkout with uncommitted changes), then
   `git am -3`. Conflicts show as normal merge conflicts in that worktree,
   in Review.
6. **Or redo on my latest code.** "Ask to redo on my code" sends a message
   in the same conversation whose parts are the asker's own text plus their
   commits since the base, limited to the share's scope, as a
   `text/x-patch` file part (after the asker approves what is sent: it is
   their code leaving the machine). The owner's Pitwall applies them to the
   session's copy before the turn and adds the factual line "Files updated:
   3 commits from bob@example.com applied". The agent just sees updated
   files and the asker's words.
7. "All changes in this conversation" is available as one series.

**Later, optional:** push a branch to a remote both can reach, or open a
pull request, as a share setting. Not in v1.
