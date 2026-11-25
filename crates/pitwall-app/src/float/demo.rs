//! Bench only (`float demo <scenario>`): made-up agents asking made-up
//! things, with made-up screens, so the control can be looked at and
//! captured in a bench instance without running real agents and without
//! the user's data. Nothing here runs outside `PITWALL_BENCH`.

use pitwall_proto::{AgentView, ScreenFrame, ScreenLine, ScreenRun};
use serde_json::json;

/// Colours of the made-up screens (truecolor, as [`ScreenRun`] has them).
const DIM: u32 = 0x100_0000 | 0x8a8a8a;
const ACCENT: u32 = 0x100_0000 | 0xb1b9f9;
const RULE: u32 = 0x100_0000 | 0x5f5f87;
const GREEN: u32 = 0x100_0000 | 0x4eba65;
const RED: u32 = 0x100_0000 | 0xff6b80;

/// A screen row: its text, its colour, bold.
type Row = (&'static str, u32, bool);

/// One made-up agent: its id (also its name), kind, project, what its
/// rules say it asks, its screen's prompt (what the rules cut out) and
/// its rows, whether Pitwall can answer it, and the machine it is on
/// (`None`: this one).
struct Fake {
    id: &'static str,
    kind: (&'static str, &'static str),
    project: &'static str,
    detail: &'static str,
    ask: String,
    rows: Vec<Row>,
    yes_no: bool,
    peer: Option<&'static str>,
}

/// The scenarios: `one`, `two`, `many`, `question`, `long`, `remote`.
pub fn scenario(name: &str) -> Option<Vec<(AgentView, ScreenFrame)>> {
    let list = match name {
        "one" => vec![bash()],
        "two" => vec![bash(), codex()],
        "many" => vec![bash(), codex(), question(), edit(), remote()],
        "question" => vec![question()],
        "long" => vec![edit()],
        "remote" => vec![remote()],
        _ => return None,
    };
    list.into_iter().map(|f| Some((view(&f)?, frame(&f.rows)))).collect()
}

fn view(f: &Fake) -> Option<AgentView> {
    let mut v: AgentView = serde_json::from_value(json!({
        "id": f.id, "name": f.id, "kind": f.kind.0, "kindName": f.kind.1, "terminal": false,
        "sessionId": null, "cwd": format!("~/code/{}", f.project), "cwdDisplay": format!("~/code/{}", f.project),
        "project": format!("~/code/{}", f.project), "projectDisplay": f.project,
        "branch": null, "worktree": false, "worktreePending": false, "location": "local",
        "machine": { "provider": "local", "id": "m1", "label": "This Mac", "canCreate": true },
        "agentInTerminal": false, "restartAs": null, "status": "blocked", "statusSource": "screen",
        "statusDetail": f.detail, "running": true, "cols": 100, "rows": 30, "added": 0, "removed": 0,
        "filesChanged": 0, "queue": [], "autoSend": false, "lastSent": null, "lastSentAt": null,
        "createdAt": 1, "currentTaskId": null,
        "caps": serde_json::to_value(pitwall_proto::AgentCaps::default()).ok()?,
        "ask": { "text": f.ask, "accept": f.yes_no, "reject": f.yes_no },
    }))
    .ok()?;
    if let Some(peer) = f.peer {
        v.machine.link = Some(pitwall_proto::peer::LinkView { peer: peer.into(), state: pitwall_proto::peer::LinkState::Connected, since: 0, last_seen: Some(1) });
    }
    Some(v)
}

fn frame(rows: &[Row]) -> ScreenFrame {
    let lines = rows
        .iter()
        .enumerate()
        .map(|(i, (text, fg, bold))| ScreenLine(i as u16, vec![ScreenRun(text.to_string(), *fg, 0, if *bold { pitwall_proto::screen_attr::BOLD } else { 0 })]))
        .collect();
    ScreenFrame { cols: 100, rows: rows.len() as u16, cursor: None, full: true, lines }
}

/// Claude Code asks to run a command.
fn bash() -> Fake {
    Fake {
        id: "api-fix",
        kind: ("claude", "Claude Code"),
        project: "my-app",
        detail: "Do you want to proceed?",
        ask: "Bash command\n\n  pnpm test --run src/cart\n  Run the cart unit tests\n\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again for pnpm test commands in ~/code/my-app\n  3. No, and tell Claude what to do differently (esc)".into(),
        rows: vec![
            ("⏺ The rounding fix is in. I'll run the cart tests to check it.", 0, false),
            ("", 0, false),
            ("────────────────────────────────────────────────────────────────────────────────", RULE, false),
            (" Bash command", ACCENT, true),
            ("", 0, false),
            ("   pnpm test --run src/cart", 0, false),
            ("   Run the cart unit tests", DIM, false),
            ("", 0, false),
            (" Do you want to proceed?", 0, false),
            (" ❯ 1. Yes", ACCENT, false),
            ("   2. Yes, and don't ask again for pnpm test commands in ~/code/my-app", 0, false),
            ("   3. No, and tell Claude what to do differently (esc)", 0, false),
            ("", 0, false),
            (" Esc to cancel · Tab to amend", DIM, false),
        ],
        yes_no: true,
        peer: None,
    }
}

/// Codex asks to run a command.
fn codex() -> Fake {
    Fake {
        id: "cleanup",
        kind: ("codex", "Codex"),
        project: "web",
        detail: "Would you like to run the following command?",
        ask: "Would you like to run the following command?\n\n  Reason: remove the stale build output before rebuilding\n\n  $ rm -rf build\n\n› 1. Yes, proceed (y)\n  2. Yes, and don't ask again for this command (a)\n  3. No, and tell Codex what to do differently (esc)".into(),
        rows: vec![
            ("• Ran pnpm build", DIM, false),
            ("  └ error: stale artifacts in build/", DIM, false),
            ("", 0, false),
            ("  Would you like to run the following command?", 0, true),
            ("", 0, false),
            ("  Reason: remove the stale build output before rebuilding", DIM, false),
            ("", 0, false),
            ("  $ rm -rf build", 0, false),
            ("", 0, false),
            ("› 1. Yes, proceed (y)", ACCENT, false),
            ("  2. Yes, and don't ask again for this command (a)", 0, false),
            ("  3. No, and tell Codex what to do differently (esc)", 0, false),
            ("", 0, false),
            ("  Press enter to confirm or esc to cancel", DIM, false),
        ],
        yes_no: true,
        peer: None,
    }
}

/// Claude Code asks a question with options (not a yes or no).
fn question() -> Fake {
    Fake {
        id: "docs",
        kind: ("claude", "Claude Code"),
        project: "handbook",
        detail: "Which format should the changelog use?",
        ask: "Which format should the changelog use?\n\n❯ 1. Keep a Changelog\n  2. Plain list by date\n  3. Type something.".into(),
        rows: vec![
            ("────────────────────────────────────────────────────────────────────────────────", RULE, false),
            (" ☐ Format", ACCENT, false),
            ("", 0, false),
            (" Which format should the changelog use?", 0, true),
            ("", 0, false),
            (" ❯ 1. Keep a Changelog", ACCENT, false),
            ("      Sections per release: Added, Changed, Fixed", DIM, false),
            ("   2. Plain list by date", 0, false),
            ("      One line per change, newest first", DIM, false),
            ("   3. Type something.", 0, false),
            ("", 0, false),
            (" Enter to select · ↑/↓ to navigate · Esc to cancel", DIM, false),
        ],
        yes_no: false,
        peer: None,
    }
}

/// Claude Code asks to edit a file, with a long diff.
fn edit() -> Fake {
    let mut rows: Vec<Row> = vec![
        ("────────────────────────────────────────────────────────────────────────────────", RULE, false),
        (" Edit file", ACCENT, true),
        (" src/cart/total.ts", 0, false),
        ("╭──────────────────────────────────────────────────────────────────────────────╮", RULE, false),
    ];
    let diff: [Row; 14] = [
        ("│ 12   export function total(items: Item[]): number {", 0, false),
        ("│ 13 -   let sum = 0;", RED, false),
        ("│ 13 +   let cents = 0;", GREEN, false),
        ("│ 14     for (const item of items) {", 0, false),
        ("│ 15 -     sum += item.price * item.qty;", RED, false),
        ("│ 15 +     cents += Math.round(item.price * 100) * item.qty;", GREEN, false),
        ("│ 16     }", 0, false),
        ("│ 17 -   return sum;", RED, false),
        ("│ 17 +   return cents / 100;", GREEN, false),
        ("│ 18   }", 0, false),
        ("│ 19", 0, false),
        ("│ 20   export function tax(total: number): number {", 0, false),
        ("│ 21     return Math.round(total * 0.2 * 100) / 100;", 0, false),
        ("│ 22   }", 0, false),
    ];
    rows.extend(diff);
    rows.extend([
        ("╰──────────────────────────────────────────────────────────────────────────────╯", RULE, false),
        (" Do you want to make this edit to total.ts?", 0, false),
        (" ❯ 1. Yes", ACCENT, false),
        ("   2. Yes, allow all edits during this session (shift+tab)", 0, false),
        ("   3. No, and tell Claude what to do differently (esc)", 0, false),
    ]);
    let ask = rows[1..rows.len() - 5].iter().map(|r| r.0.trim_start_matches(['│', '╭', '╰']).trim_end_matches(['│', '╮', '╯'])).filter(|l| !l.starts_with('─')).collect::<Vec<_>>().join("\n");
    Fake {
        id: "cart-total",
        kind: ("claude", "Claude Code"),
        project: "my-app",
        detail: "Do you want to make this edit to total.ts?",
        ask: format!("{ask}\nDo you want to make this edit to total.ts?\n❯ 1. Yes\n  2. Yes, allow all edits during this session (shift+tab)\n  3. No, and tell Claude what to do differently (esc)"),
        rows,
        yes_no: true,
        peer: None,
    }
}

/// An agent on another of the user's machines (read-only here).
fn remote() -> Fake {
    let mut f = bash();
    f.id = "migrate";
    f.project = "billing";
    f.ask = "Bash command\n\n  psql -f migrations/0042_invoices.sql\n  Apply the invoices migration\n\nDo you want to proceed?\n❯ 1. Yes\n  2. No".into();
    f.rows = vec![
        (" Bash command", ACCENT, true),
        ("", 0, false),
        ("   psql -f migrations/0042_invoices.sql", 0, false),
        ("   Apply the invoices migration", DIM, false),
        ("", 0, false),
        (" Do you want to proceed?", 0, false),
        (" ❯ 1. Yes", ACCENT, false),
        ("   2. No", 0, false),
    ];
    f.peer = Some("my-vm");
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_is_a_list_of_asking_agents() {
        for name in ["one", "two", "many", "question", "long", "remote"] {
            let list = scenario(name).expect(name);
            let items = super::super::model::items(&list.iter().map(|(a, _)| a.clone()).collect::<Vec<_>>());
            assert_eq!(items.len(), list.len(), "{name}: each one asks");
            for (a, f) in &list {
                let rows: Vec<String> = f.lines.iter().map(|l| l.1.iter().map(|r| r.0.as_str()).collect()).collect();
                assert!(super::super::model::TermRows::find(&rows, &a.ask.as_ref().unwrap().text).is_some_and(|r| r.found), "{name}: {}'s prompt is on its screen", a.id);
            }
        }
        assert!(scenario("nope").is_none());
    }
}
