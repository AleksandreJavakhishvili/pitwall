# Collaboration: UI

Part of the [collaboration spec](README.md).

## 11. UI (GPUI app)

Same visual language as tailnet-peers.md §7.

- **People** (new top-level screen, ⌘⇧P, also in ⌘K), three tabs:
  - **Directory.** Shared agents I can use, grouped by owner: name,
    description, capability chip (READ / WRITE / WRITE ✓), "runs on Claude
    Code", what's left today, online dot. Click → Ask panel on that agent.
  - **Teams.** My teams (name, fingerprint `9F2-C1A`, members, my role,
    "bound to Tailscale" chip), then Discovered (listed teams on the
    tailnet, "Request to join"), then "Join with a link…" and "Create
    team…".
    - **Team detail:** members with devices and online state, admins (★),
      version and "last change by dave · 2 d", Requests (admins: Approve /
      Decline), Invite (make link, copy), Leave; for admins: rename,
      listed/invite-only, join by request/closed, promote/demote, remove,
      Bind to Tailscale….
    - Conflict note inline when it happened: "Your change was re-applied
      after dave's".
  - **People.** Everyone in my teams, their devices and shared agents.
    "Block" stops all of a person's messages to my agents.
- **Share dialog** (agent menu → Share…): the seven steps of [§5.4](sharing.md), one short
  page each with Back / Next, each field marked **Enforced** or **Not
  guaranteed**:
  1. Who (and "Only Pitwall callers") · 2. Capability · 3. What it can see
     (folder picker, then the scan result as a ticked list with "why":
     "matches *.pem", "contains a private key") · 4. Start from (Fresh ·
     Summary: "Write summary…" → editor → Approve) · 5. Secrets and access
     (account, with "use a separate account for sharing"; environment
     variables to pass, none by default; extra network hosts; programs it
     may run, as an allow or deny list) · 6.
     Off-limits topics and the role text · 7. Limits.
  - Then **Summary** ([§5.4](sharing.md)), **Try as guest** (opens the Ask panel on the
    draft with the probe buttons) and **Go live**.
  - Editing later opens the same steps; Save shows the diff and marks
    widening lines amber.
- **Suggestion cards.** In the agent's pane header and the notifications
  list: "Race Engineer suggests sharing payments-expert with team Payments
  — people asked about refunds 9 times this week. [Open Share…] [Dismiss]".
- **Sidebar, owner side.** A "Shared" group (after local projects, before
  peer machines): each shared agent is a heading with capability chip,
  running / cap (`1/2`), draft or paused state; under it one row per
  session, named by asker (`bob`, `bob · client-fix agent`, `dave (A2A)`,
  `guest`), with status flag, last message time and usage. Needs-you and
  held results flag amber like any blocked agent and count in the badge.
- **Session pane, owner side.** The terminal (watch), with a strip "Shared
  with bob@example.com · read only · sandboxed · Take over". A side tab
  **Messages** lists what was typed, replies (with their source) and results
  as a thread. Review works on the session's copy; at `write_approval`,
  "Send" / "Discard" for each held result.
- **Usage.** Shared agent heading → "Usage…": per asker today and 30 days
  (messages, results, minutes running, tokens where the agent reports
  them), Block, Forget session, the audit log filtered to this share.
- **Ask panel** (asker side; right-panel tab "Ask", and ⌘⇧A): agent picker,
  one thread per shared agent (my messages, replies, statuses "2nd in line",
  "waiting for alice", "changes waiting for alice's OK"), "New
  conversation". Replies with changes show a result card (files, +/−, "Open
  in Review", "Apply…", "Redo on my code").
- **Review (asker side).** A result is a Review entry "changes from
  alice/api-expert · task 7", with per-file diffs, Apply (target picker), and
  conflicts shown as for any merge.
- **Notifications.** Owner: needs-you in a shared session, results to
  review, join requests (admins), suggestions (quiet), optionally every
  reply. Asker: replies that arrived while the window was unfocused, results
  ready.
- **Tasks** (new screen, from the top bar and ⌘K; [§15](tasks.md)): lists on the left (My list, then each team's), tag filter chips
  with counts across all lists, status and "added by an agent" filters.
  Item detail: title, body, tags, who added it (with the agent label),
  status, history with lost ops ("bob took it first"), comments. Buttons:
  **Take**, **Give to agent…** (own agents, and shared agents I may use),
  **Release**, **Done** (with "attach changes from…"), **Drop**. A result
  patch opens in Review with Apply. Watch rules under "Watch…" per tag.
  Badges count new items in watched tags; the item's agent row in the
  sidebar shows a small task chip while it works on one.
- **Settings → Collaboration.** The three switches, "Look for teams on",
  the global session cap, budgets for my agents, rotate key, the audit log,
  and Tailscale state (shared with Peers).
