//! Detection tests. Fixtures under `fixtures/` are real screens captured from
//! Claude Code v2.1.280 and codex-cli 0.159.2/0.160.1 running in a PTY and
//! rendered by `crate::Screen` (paths sanitized). The first line of
//! each fixture is `#title: <OSC title>` (empty = no title).

use super::*;

fn fixture(name: &str) -> (String, Option<String>) {
    let raw = match name {
        "claude_trust" => include_str!("fixtures/claude_trust.txt"),
        "claude_idle_fresh" => include_str!("fixtures/claude_idle_fresh.txt"),
        "claude_idle_typed" => include_str!("fixtures/claude_idle_typed.txt"),
        "claude_idle_done" => include_str!("fixtures/claude_idle_done.txt"),
        "claude_idle_interrupted" => include_str!("fixtures/claude_idle_interrupted.txt"),
        "claude_idle_after_question" => include_str!("fixtures/claude_idle_after_question.txt"),
        "claude_working_spinner" => include_str!("fixtures/claude_working_spinner.txt"),
        "claude_working_tool" => include_str!("fixtures/claude_working_tool.txt"),
        "claude_working_manual" => include_str!("fixtures/claude_working_manual.txt"),
        "claude_blocked_bash" => include_str!("fixtures/claude_blocked_bash.txt"),
        "claude_blocked_bash_moved" => include_str!("fixtures/claude_blocked_bash_moved.txt"),
        "claude_blocked_question" => include_str!("fixtures/claude_blocked_question.txt"),
        "codex_trust" => include_str!("fixtures/codex_trust.txt"),
        "codex_update" => include_str!("fixtures/codex_update.txt"),
        "codex_hooks_review" => include_str!("fixtures/codex_hooks_review.txt"),
        "codex_idle_fresh" => include_str!("fixtures/codex_idle_fresh.txt"),
        "codex_idle_typed" => include_str!("fixtures/codex_idle_typed.txt"),
        "codex_idle_done" => include_str!("fixtures/codex_idle_done.txt"),
        "codex_working" => include_str!("fixtures/codex_working.txt"),
        "codex_working_queued" => include_str!("fixtures/codex_working_queued.txt"),
        "codex_blocked_approval" => include_str!("fixtures/codex_blocked_approval.txt"),
        other => panic!("unknown fixture {other}"),
    };
    let (first, rest) = raw.split_once('\n').unwrap();
    let title = first.strip_prefix("#title: ").unwrap().trim();
    let title = (!title.is_empty()).then(|| title.to_string());
    (rest.to_string(), title)
}

/// Detect using the built-in rules only (never the developer's overrides).
fn builtin(kind: &str, screen: &str, title: Option<&str>) -> Detection {
    let src = BUILTIN.iter().find(|(k, _)| *k == kind).unwrap().1;
    RuleSet::parse(src).unwrap().evaluate(screen, title.unwrap_or(""))
}

fn check(kind: &str, name: &str, want: Option<Detected>) -> Detection {
    let (screen, title) = fixture(name);
    let got = builtin(kind, &screen, title.as_deref());
    assert_eq!(got.state, want, "{name} (with title {title:?})");
    got
}

/// Same fixture with the title removed: screen rules alone must agree.
fn check_screen_only(kind: &str, name: &str, want: Option<Detected>) -> Detection {
    let (screen, _) = fixture(name);
    let got = builtin(kind, &screen, None);
    assert_eq!(got.state, want, "{name} (screen only)");
    got
}

use Detected::{Blocked, Idle, Working};

#[test]
fn builtin_rule_files_parse() {
    for (kind, src) in BUILTIN {
        let set = RuleSet::parse(src).unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert!(!set.rules.is_empty());
    }
}

// --- Claude ----------------------------------------------------------------

#[test]
fn claude_idle() {
    for name in [
        "claude_idle_fresh",
        "claude_idle_typed",
        "claude_idle_done",
        "claude_idle_interrupted",
        "claude_idle_after_question",
    ] {
        check("claude", name, Some(Idle));
        let d = check_screen_only("claude", name, Some(Idle));
        assert_eq!(d.detail, None);
    }
}

#[test]
fn claude_working() {
    for name in [
        "claude_working_spinner",
        "claude_working_tool",
        "claude_working_manual",
    ] {
        check("claude", name, Some(Working));
        check_screen_only("claude", name, Some(Working));
    }
}

#[test]
fn claude_working_with_default_footer() {
    // Older/other footers without the mode glyph still separate items by "·".
    let screen = "❯ do it\n\n✢ Pondering… (12s · ↑ 1.2k tokens)\n\
                  ────────────\n❯ \n────────────\n  ? for shortcuts · esc to interrupt\n";
    assert_eq!(builtin("claude", screen, None).state, Some(Working));
}

#[test]
fn claude_blocked_permission() {
    for name in ["claude_blocked_bash", "claude_blocked_bash_moved"] {
        let d = check("claude", name, Some(Blocked));
        assert_eq!(d.detail.as_deref(), Some("Do you want to proceed?"), "{name}");
        check_screen_only("claude", name, Some(Blocked));
    }
}

#[test]
fn claude_blocked_question() {
    let d = check("claude", "claude_blocked_question", Some(Blocked));
    assert_eq!(d.detail.as_deref(), Some("Which color do you prefer?"));
}

#[test]
fn claude_blocked_trust_folder() {
    let d = check("claude", "claude_trust", Some(Blocked));
    assert_eq!(d.detail.as_deref(), Some("Trust this folder?"));
}

#[test]
fn claude_title_only() {
    let d = builtin("claude", "", Some("◐ Fix the tests"));
    assert_eq!(d.state, Some(Working));
    let d = builtin("claude", "", Some("⠂ Fix the tests"));
    assert_eq!(d.state, Some(Working));
    let d = builtin("claude", "", Some("✳ Claude Code"));
    assert_eq!(d.state, Some(Idle));
    assert_eq!(builtin("claude", "", Some("zsh")).state, None);
    assert_eq!(builtin("claude", "", None).state, None);
}

#[test]
fn claude_permission_dialog_beats_idle_title() {
    // The title stays "✳" (idle) while a permission dialog is open.
    let (screen, title) = fixture("claude_blocked_bash");
    assert!(title.as_deref().unwrap().starts_with('✳'));
    assert_eq!(builtin("claude", &screen, title.as_deref()).state, Some(Blocked));
}

// Negative cases: text the user typed or the agent printed must not
// impersonate a state.

#[test]
fn claude_typed_esc_to_interrupt_is_not_working() {
    let (screen, _) = fixture("claude_idle_fresh");
    let screen = screen.replacen("❯\n", "❯ ⏵⏵ esc to interrupt · esc to interrupt\n", 1);
    assert!(screen.contains("❯ ⏵⏵ esc to interrupt"));
    assert_eq!(builtin("claude", &screen, None).state, Some(Idle));
}

#[test]
fn claude_scrollback_mentions_are_not_states() {
    // Agent output (indented continuation lines) mentions spinner-like
    // lines, "esc to interrupt" and a fake permission dialog.
    let screen = "\
❯ explain the status line
⏺ The footer shows:
  ⏸ manual mode on · esc to interrupt · ← for agents
  * Loading…
  ✻ Thinking…
  Bash command
  Do you want to proceed?
  ❯ 1. Yes
    2. No
  Esc to cancel · Tab to amend
✻ Baked for 3s · done 10:04 AM
────────────────────────────────────────
❯
────────────────────────────────────────
  ⏸ manual mode on · ? for shortcuts · ← for agents";
    let d = builtin("claude", screen, Some("✳ Explain"));
    assert_eq!(d.state, Some(Idle));
    assert_eq!(builtin("claude", screen, None).state, Some(Idle));
}

#[test]
fn claude_multiline_prompt_cannot_fake_footer() {
    // Text typed into the prompt box sits between the two rules.
    let screen = "\
────────────────────────────────────────
❯ please print this:
  · esc to interrupt
  ✶ Symbioting…
────────────────────────────────────────
  ⏸ manual mode on · ? for shortcuts";
    assert_eq!(builtin("claude", screen, None).state, Some(Idle));
}

#[test]
fn claude_unrecognised_screen_is_none() {
    assert_eq!(builtin("claude", "some shell output\n$ ", None).state, None);
    assert_eq!(builtin("claude", "", None).state, None);
}

// --- Codex -----------------------------------------------------------------

#[test]
fn codex_idle() {
    for name in ["codex_idle_fresh", "codex_idle_typed", "codex_idle_done"] {
        check("codex", name, Some(Idle));
        check_screen_only("codex", name, Some(Idle));
    }
}

#[test]
fn codex_working() {
    for name in ["codex_working", "codex_working_queued"] {
        check("codex", name, Some(Working));
        check_screen_only("codex", name, Some(Working));
    }
}

#[test]
fn codex_blocked_approval() {
    let d = check("codex", "codex_blocked_approval", Some(Blocked));
    assert_eq!(
        d.detail.as_deref(),
        Some("Would you like to run the following command?")
    );
    let d = check_screen_only("codex", "codex_blocked_approval", Some(Blocked));
    assert!(d.detail.is_some());
}

#[test]
fn codex_startup_dialogs() {
    let d = check("codex", "codex_trust", Some(Blocked));
    assert_eq!(d.detail.as_deref(), Some("Trust this folder?"));
    let d = check("codex", "codex_update", Some(Blocked));
    assert_eq!(d.detail.as_deref(), Some("Codex update available"));
    let d = check("codex", "codex_hooks_review", Some(Blocked));
    assert_eq!(d.detail.as_deref(), Some("Hooks need review"));
}

#[test]
fn codex_legacy_working_line() {
    let screen = "› fix it\n\n• Exploring (12s • esc to interrupt) · 1 background terminal\n\n\
                  › Summarize recent commits\n\n  ? for shortcuts";
    assert_eq!(builtin("codex", screen, None).state, Some(Working));
}

#[test]
fn codex_stale_working_line_is_not_working() {
    // The timer line belongs to an earlier turn: a response block follows.
    let screen = "› first\n• Working (3s • esc to interrupt)\n• done, here you go\n\n› Ask Codex to do anything\n\n  ? for shortcuts";
    assert_eq!(builtin("codex", screen, None).state, Some(Idle));
}

#[test]
fn codex_typed_blocker_phrase_is_not_blocked() {
    let (screen, title) = fixture("codex_idle_typed");
    let screen = screen.replace(
        "› Reply with just the word hi, nothing else.",
        "› press enter to confirm or esc to cancel [y/n] allow command?",
    );
    assert_eq!(builtin("codex", &screen, title.as_deref()).state, Some(Idle));
    assert_eq!(builtin("codex", &screen, None).state, Some(Idle));
}

#[test]
fn codex_titles() {
    assert_eq!(builtin("codex", "", Some("⠙ ⠙ | proj")).state, Some(Working));
    let d = builtin("codex", "", Some("[ ! ] Action Required | Create marker file | proj"));
    assert_eq!(d.state, Some(Blocked));
    assert_eq!(builtin("codex", "", Some("Reply with hi | proj")).state, Some(Idle));
}

// --- Engine ----------------------------------------------------------------

#[test]
fn unknown_kinds_yield_default() {
    let d = detect("shell", "anything ❯ esc to interrupt", Some("◐ x"));
    assert_eq!(d.state, None);
    assert_eq!(d.detail, None);
    assert_eq!(detect("no-such-kind", "", None).state, None);
}

#[test]
fn public_detect_uses_rules() {
    // Works through the cached path too (user overrides, if any, may differ,
    // so only check a signal every sane rule set agrees on: nothing → None).
    let _ = detect("claude", "", None);
    let _ = detect("codex", "", None);
}

#[test]
fn priority_order_and_unknown_state() {
    let src = r#"
        [[rules]]
        id = "low"
        state = "idle"
        priority = 1
        contains = ["ready"]

        [[rules]]
        id = "high"
        state = "working"
        priority = 10
        contains = ["ready", "busy"]

        [[rules]]
        id = "menu"
        state = "unknown"
        priority = 20
        contains = ["menu"]
    "#;
    let set = RuleSet::parse(src).unwrap();
    assert_eq!(set.evaluate("ready", "").state, Some(Idle));
    assert_eq!(set.evaluate("ready busy", "").state, Some(Working));
    assert_eq!(set.evaluate("ready busy menu", "").state, None);
    assert_eq!(set.evaluate("nothing", "").state, None);
}

#[test]
fn combinators() {
    let src = r#"
        [[rules]]
        id = "r"
        state = "blocked"
        priority = 1
        region = "bottom_non_empty_lines(2)"
        contains = ["Footer"]
        line_regex = ['^\s*❯ 1\.']
        any = [ { contains = ["yes"] }, { regex = ['(?i)allow'] } ]
        all = [ { not = [ { contains = ["nope"] } ] } ]
        not = [ { contains = ["cancelled"] } ]
        detail = "Approve?"
    "#;
    let set = RuleSet::parse(src).unwrap();
    let hit = set.evaluate("top\n❯ 1. Yes\nfooter", "");
    assert_eq!(hit.state, Some(Blocked));
    assert_eq!(hit.detail.as_deref(), Some("Approve?"));
    assert_eq!(set.evaluate("❯ 1. Allow\nfooter", "").state, Some(Blocked));
    assert_eq!(set.evaluate("❯ 1. Maybe\nfooter", "").state, None);
    assert_eq!(set.evaluate("❯ 1. yes nope\nfooter", "").state, None);
    assert_eq!(set.evaluate("❯ 1. yes\nfooter cancelled", "").state, None);
    // Outside the region (3rd line from bottom) does not count.
    assert_eq!(set.evaluate("❯ 1. yes\nfooter\nmore", "").state, None);
    // Trailing blank lines are ignored by regions.
    assert_eq!(set.evaluate("❯ 1. yes\nfooter\n\n   \n", "").state, Some(Blocked));
}

#[test]
fn detail_only_for_blocked_and_bottom_up() {
    let src = r#"
        detail_line = ['^\s*(\S.*\?)\s*$']
        [[rules]]
        id = "b"
        state = "blocked"
        priority = 2
        contains = ["dialog"]
        [[rules]]
        id = "w"
        state = "working"
        priority = 1
        contains = ["busy"]
    "#;
    let set = RuleSet::parse(src).unwrap();
    let d = set.evaluate("Old question?\n\n  Newer question?  \ndialog", "");
    assert_eq!(d.detail.as_deref(), Some("Newer question?"));
    let d = set.evaluate("Question?\nbusy", "");
    assert_eq!(d.state, Some(Working));
    assert_eq!(d.detail, None);
    let long = format!("{}?\ndialog", "x".repeat(400));
    let d = set.evaluate(&long, "");
    assert!(d.detail.unwrap().chars().count() <= MAX_DETAIL_CHARS);
}

#[test]
fn invalid_rule_files_are_rejected() {
    // Typo in a matcher key.
    assert!(RuleSet::parse("[[rules]]\nid='x'\nstate='idle'\ncontain=['a']").is_err());
    // Unknown state / region / bad regex.
    assert!(RuleSet::parse("[[rules]]\nid='x'\nstate='done'").is_err());
    assert!(RuleSet::parse("[[rules]]\nid='x'\nstate='idle'\nregion='middle'").is_err());
    assert!(RuleSet::parse("[[rules]]\nid='x'\nstate='idle'\nregex=['(']").is_err());
    assert!(RuleSet::parse("[[rules]]\nid='x'\nstate='idle'\nregion='bottom_lines(0)'").is_err());
    // Empty file is valid (and matches nothing).
    assert_eq!(RuleSet::parse("").unwrap().evaluate("x", "").state, None);
}

#[test]
fn overrides_replace_builtins_and_add_kinds() {
    let dir = std::env::temp_dir().join(format!("pitwall-detect-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("claude.toml"),
        "[[rules]]\nid='only'\nstate='working'\npriority=1\ncontains=['custom marker']\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("aider.toml"),
        "[[rules]]\nid='p'\nstate='idle'\nregion='bottom_lines(1)'\nline_regex=['^>\\s*$']\n",
    )
    .unwrap();
    std::fs::write(dir.join("codex.toml"), "this is = not [valid toml").unwrap();
    std::fs::write(dir.join("notes.txt"), "ignored").unwrap();

    let mut map = Cache::new();
    for (kind, src) in BUILTIN {
        map.insert(kind.to_string(), Arc::new(RuleSet::parse(src).unwrap()));
    }
    load_overrides(&dir, &mut map);
    std::fs::remove_dir_all(&dir).ok();

    let claude = &map["claude"];
    assert_eq!(claude.rules.len(), 1);
    assert_eq!(claude.evaluate("custom marker", "").state, Some(Working));
    // Built-in claude rules are gone.
    assert_eq!(claude.evaluate("", "◐ busy").state, None);
    assert_eq!(map["aider"].evaluate("out\n> ", "").state, Some(Idle));
    // Broken override keeps the built-in.
    assert_eq!(map["codex"].evaluate("", "⠙ x").state, Some(Working));
    assert!(!map.contains_key("notes"));
}

#[test]
fn regions() {
    let s = "a\n\nb\n──────\nbox ❯\n──────\nfooter\n";
    let s = trim_trailing_blank_lines(s);
    assert_eq!(Region::AfterLastHorizontalRule.select(s, ""), "footer");
    assert_eq!(Region::PromptBoxBody.select(s, ""), "box ❯\n");
    assert_eq!(Region::AbovePromptBox.select(s, ""), "a\n\nb\n");
    assert_eq!(Region::LastNonEmptyAbovePromptBox.select(s, ""), "b\n");
    assert_eq!(Region::BottomNonEmptyLines(2).select(s, ""), "──────\nfooter");
    assert_eq!(Region::TopNonEmptyLines(2).select(s, ""), "a\n\nb");
    assert_eq!(Region::BottomLines(1).select(s, ""), "footer");
    assert_eq!(Region::Title.select(s, "t"), "t");
    // No rule at all → empty, never the whole screen.
    assert_eq!(Region::AfterLastHorizontalRule.select("x\ny", ""), "");
    assert_eq!(Region::PromptBoxBody.select("x\ny", ""), "");

    let c = "› old\n• answer\n› now typing\n  footer";
    assert_eq!(Region::CurrentPromptLine.select(c, ""), "› now typing");
    assert_eq!(Region::BeforeCurrentPromptMarker.select(c, ""), "› old\n• answer\n");
    assert_eq!(Region::AfterLastPromptMarker.select(c, ""), "  footer");
    assert_eq!(Region::WholeWithoutCurrentPromptMarker.select(c, ""), "");
    let stale = "› old\n• answer";
    assert_eq!(Region::CurrentPromptLine.select(stale, ""), "");
    assert_eq!(Region::WholeWithoutCurrentPromptMarker.select(stale, ""), stale);
}

#[test]
fn screen_to_detect_end_to_end() {
    // Draw a Claude-like working screen with cursor addressing, then idle.
    let mut screen = super::super::screen::Screen::new(8, 50);
    let rule = "─".repeat(50);
    screen.feed("\x1b]0;◐ Claude Code\x07".as_bytes());
    screen.feed(format!("\x1b[2;1H✶ Symbioting…\x1b[4;1H{rule}\x1b[5;1H❯ \x1b[6;1H{rule}\x1b[7;3H⏸ manual mode on · esc to interrupt").as_bytes());
    let title = screen.title();
    let d = builtin("claude", &screen.text(), title.as_deref());
    assert_eq!(d.state, Some(Working));
    assert_eq!(builtin("claude", &screen.text(), None).state, Some(Working));

    screen.feed("\x1b]0;✳ Claude Code\x07".as_bytes());
    screen.feed("\x1b[2;1H\x1b[2K✻ Baked for 3s\x1b[7;1H\x1b[2K  ⏸ manual mode on · ? for shortcuts".as_bytes());
    let title = screen.title();
    assert_eq!(builtin("claude", &screen.text(), title.as_deref()).state, Some(Idle));
}
