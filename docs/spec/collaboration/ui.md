# Collaboration: UI

Part of the [collaboration spec](README.md).

## 11. UI (GPUI app)

Same visual language as tailnet-peers.md §7.

- **People** (new top-level screen, ⌘⇧P, also in ⌘K), three tabs:
  - **Directory.** Shared agents I can use, grouped by owner: name,
    description, capability chip (READ / WRITE / WRITE ✓), "runs on Claude
    Code", what's left today, online dot. Click → Ask panel on that agent.
  - **Teams.** My teams (name, fingerprint `9F2-C1A`, members, my role,
    "bound to a group" chip), then Discovered (listed teams on my
    networks, with the network shown, "Request to join"), then "Join with a link…" and "Create
    team…".
    - **Team detail:** members with devices and online state, admins (★),
      version and "last change by dave · 2 d", Requests (admins: Approve /
      Decline), Invite (make link, copy), Leave; for admins: **Add
      member…** by login (also for people without Pitwall, marked "Claude
      only"), **Setup for people without Pitwall** (the `claude mcp add`
      lines to send them, with Copy), rename,
      listed/invite-only, join by request/closed, promote/demote, remove,
      Bind to a group….
    - Conflict note inline when it happened: "Your change was re-applied
      after dave's".
  - **People.** Everyone in my teams, their devices and shared agents.
    "Block" stops all of a person's messages to my agents.
- **Share dialog** (agent menu → Share…): the seven steps of [§5.4](sharing.md), one short
  page each with Back / Next, each field marked **Enforced** or **Not
  guaranteed**:
  1. Who · 2. Capability · 3. What it can see
     (folder picker, then the scan result as a ticked list with "why":
     "matches *.pem", "contains a private key") · 4. Start from (Fresh ·
     Summary: "Write summary…" → editor → Approve) · 5. Secrets and access
     (account, with "use a separate account for sharing"; environment
     variables to pass, none by default; extra network hosts; programs it
     may run, as an allow or deny list) · 6.
     Off-limits topics and the role text · 7. Limits (messages per person
     per day, conversations at once, time per turn, agent time per day,
     per-person overrides; [§5.8](sharing.md)).
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
  session, named by caller (`bob`, `bob · client-fix agent`, `erin · Claude Code (MCP)`,
  `guest`), with status flag, last message time and usage. Needs-you and
  held results flag amber like any blocked agent and count in the badge.
- **Session pane, owner side.** The terminal (watch), with a strip "Shared
  with bob@example.com · read only · sandboxed · Take over". A side tab
  **Messages** lists what was typed, replies (with their source) and results
  as a thread. Review works on the session's copy; at `write_approval`,
  "Send" / "Discard" for each held result.
- **Usage.** Shared agent heading → "Usage…": per caller today and 30 days
  (messages, results, minutes running, tokens where the agent reports
  them) against each limit, the share's agent time today against its daily
  total, a "limited by Claude until 14:30" banner when the agent hit its
  vendor limit; per person: change limit, Block, Forget conversation; the
  audit log filtered to this share.
- **Ask panel** (caller side; right-panel tab "Ask", and ⌘⇧A): agent picker,
  one thread per shared agent (my messages, replies, statuses "2nd in line",
  "waiting for alice", "changes waiting for alice's OK"), "New
  conversation". Replies with changes show a result card (files, +/−, "Open
  in Review", "Apply…", "Redo on my code").
- **Review (caller side).** A result is a Review entry "changes from
  alice/api-expert · turn 7", with per-file diffs, Apply (target picker), and
  conflicts shown as for any merge.
- **Notifications.** Owner: needs-you in a shared session, results to
  review, join requests (admins), suggestions (quiet), optionally every
  reply. Caller: replies that arrived while the window was unfocused, results
  ready.
- **Settings → Collaboration.** The three switches, "Look for teams on",
  the global session cap, budgets for my agents, "Copy my MCP setup line",
  rotate key, and the audit log.
- **Settings → Networks.** One row per backend: Tailscale (state shared
  with Peers, "Look for teams on", "Let people on example.ts.net reach this
  machine") and, when it exists, Pitwall Cloud (sign in, org, "Let people
  in acme reach this machine"). People → a person shows their identities
  and "Link identities…".
