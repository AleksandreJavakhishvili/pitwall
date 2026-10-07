//! Detection tests for the agents ported from Herdr's manifests (Gemini CLI,
//! opencode, Cursor Agent, Copilot CLI, Qwen Code, Amp) and Pitwall's own
//! Aider rules.
//!
//! Herdr ships no screen fixtures for these agents, and none of the CLIs
//! except cursor-agent was installed when the rules were written, so the
//! screens below are synthetic: they reproduce the UI strings Herdr's
//! manifests (Apache-2.0, see NOTICE) match on, laid out the way each CLI
//! draws them. Replace them with captured screens when available.

use super::*;
use Detected::{Blocked, Idle, Working};

/// Detect using the built-in rules only (never the developer's overrides).
fn builtin(kind: &str, screen: &str, title: Option<&str>) -> Detection {
    let src = BUILTIN
        .iter()
        .find(|(k, _)| *k == kind)
        .unwrap_or_else(|| panic!("no built-in rules for {kind}"))
        .1;
    RuleSet::parse(src).unwrap().evaluate(screen, title.unwrap_or(""))
}

fn state(kind: &str, screen: &str) -> Option<Detected> {
    builtin(kind, screen, None).state
}

// --- Gemini CLI ------------------------------------------------------------

const GEMINI_IDLE: &str = "\
 ███            █████████  ██████████ ██████   ██████
Tips for getting started:
1. Ask questions, edit files, or run commands.

╭──────────────────────────────────────────────────────────────╮
│ >   Type your message or @path/to/file                        │
╰──────────────────────────────────────────────────────────────╯
~/projects/demo (main*)        no sandbox        gemini-2.5-pro (100% context left)";

const GEMINI_WORKING: &str = "\
> refactor the parser

⠴ Reading the parser module (esc to cancel, 12s)

╭──────────────────────────────────────────────────────────────╮
│ >   Type your message or @path/to/file                        │
╰──────────────────────────────────────────────────────────────╯
~/projects/demo (main*)        no sandbox        gemini-2.5-pro (98% context left)";

const GEMINI_APPLY: &str = "\
> fix the typo

╭──────────────────────────────────────────────────────────────╮
│ ?  Edit src/parser.rs: let x = 1 => let x = 2                 │
│                                                                │
│ 12 - let x = 1;                                                │
│ 12 + let x = 2;                                                │
│                                                                │
│ Apply this change?                                             │
│                                                                │
│ ● 1. Yes, allow once                                           │
│   2. Yes, allow always                                         │
│   3. Modify with external editor                               │
│   4. No, suggest changes (esc)                                 │
╰──────────────────────────────────────────────────────────────╯
~/projects/demo (main*)        no sandbox        gemini-2.5-pro (97% context left)";

const GEMINI_SHELL: &str = "\
╭──────────────────────────────────────────────────────────────╮
│ ?  Shell cargo test (run the tests)                            │
│                                                                │
│ cargo test                                                     │
│                                                                │
│ Allow execution of: 'cargo'?                                   │
│                                                                │
│ ● 1. Yes, allow once                                           │
│   2. Yes, allow always ...                                     │
│   3. No, suggest changes (esc)                                 │
╰──────────────────────────────────────────────────────────────╯";

#[test]
fn gemini_states() {
    assert_eq!(state("gemini", GEMINI_IDLE), Some(Idle));
    assert_eq!(state("gemini", GEMINI_WORKING), Some(Working));
    let apply = builtin("gemini", GEMINI_APPLY, None);
    assert_eq!(apply.state, Some(Blocked));
    assert_eq!(apply.detail.as_deref(), Some("Apply this change?"));
    let shell = builtin("gemini", GEMINI_SHELL, None);
    assert_eq!(shell.state, Some(Blocked));
    assert_eq!(shell.detail.as_deref(), Some("Allow execution of: 'cargo'?"));
}

#[test]
fn gemini_old_dialog_scrolled_away_is_not_blocked() {
    // The same dialog far above the bottom of the screen is history.
    let mut screen = GEMINI_APPLY.to_string();
    for i in 0..30 {
        screen.push_str(&format!("\nline {i} of later output"));
    }
    screen.push('\n');
    screen.push_str(GEMINI_IDLE);
    assert_eq!(state("gemini", &screen), Some(Idle));
}

#[test]
fn blank_screen_is_unknown_for_fallback_agents() {
    for kind in ["gemini", "opencode", "cursor", "copilot", "qwen", "amp"] {
        assert_eq!(state(kind, ""), None, "{kind}");
        assert_eq!(state(kind, "\n   \n"), None, "{kind}");
    }
}

// --- opencode --------------------------------------------------------------

const OPENCODE_IDLE: &str = "\
  █▀▀█ █▀▀█ █▀▀ █▀▀▄ █▀▀ █▀▀█ █▀▀▄ █▀▀
  █░░█ █░░█ █▀▀ █░░█ █░░ █░░█ █░░█ █▀▀

┃  Ask anything... \"Fix a TODO in the codebase\"
┃
┃  Build  Claude Sonnet 4.5 Anthropic
                                       tab switch agent  ctrl+p commands";

const OPENCODE_WORKING: &str = "\
┃  explain the build script
┃

   Thinking: reading build.rs

┃
┃  Build  Claude Sonnet 4.5 Anthropic
  ⬝⬝⬝⬝■■■■  esc interrupt                tab switch agent  ctrl+p commands";

const OPENCODE_PERMISSION: &str = "\
┃  run the tests
┃

  △ Permission required
  $ cargo test --workspace

   Allow once   Allow always   Reject
                                  ⇆ tab  ↑↓ select  enter confirm  esc dismiss";

#[test]
fn opencode_states() {
    assert_eq!(state("opencode", OPENCODE_IDLE), Some(Idle));
    assert_eq!(state("opencode", OPENCODE_WORKING), Some(Working));
    let blocked = builtin("opencode", OPENCODE_PERMISSION, None);
    assert_eq!(blocked.state, Some(Blocked));
    assert_eq!(blocked.detail.as_deref(), Some("Permission required"));
    // Keyboard hints alone (no permission header) still block.
    let question = "┃  pick one\n\n  1. Rust\n  2. Go\n  ↑↓ select  enter submit  esc dismiss";
    assert_eq!(state("opencode", question), Some(Blocked));
}

// --- Cursor Agent ----------------------------------------------------------

const CURSOR_IDLE: &str = "\
  Cursor Agent
  ~/projects/demo · main

 ┌──────────────────────────────────────────────────────────┐
 │ → Plan, search, build anything                            │
 └──────────────────────────────────────────────────────────┘
  GPT-5 · 100%                                     / commands · @ files";

const CURSOR_WORKING: &str = "\
  ┃ add a unit test for the parser

  ⬢ Generating.
 ┌──────────────────────────────────────────────────────────┐
 │ → Add a follow-up                                         │
 └──────────────────────────────────────────────────────────┘
  GPT-5 · 97%                                   ctrl+c to stop";

const CURSOR_RUN: &str = "\
  ┃ run the tests

 ┌──────────────────────────────────────────────────────────┐
 │ Run this command?                                         │
 │ Not in allowlist: cargo test                              │
 │  → Run (once) (y) (enter)                                 │
 │    Add Shell(cargo) to allowlist? (tab)                   │
 │    Skip (esc or n)                                        │
 └──────────────────────────────────────────────────────────┘
  Waiting for approval...";

const CURSOR_WRITE: &str = "\
 ┌──────────────────────────────────────────────────────────┐
 │ Write to this file?                                       │
 │ src/parser.rs                                             │
 │  → Proceed (y) (enter)                                    │
 │    Add Write(src/**) to allowlist? (tab)                  │
 │    Reject & propose changes (esc or n or p)               │
 └──────────────────────────────────────────────────────────┘";

#[test]
fn cursor_states() {
    assert_eq!(state("cursor", CURSOR_IDLE), Some(Idle));
    assert_eq!(state("cursor", CURSOR_WORKING), Some(Working));
    let run = builtin("cursor", CURSOR_RUN, None);
    assert_eq!(run.state, Some(Blocked));
    assert_eq!(run.detail.as_deref(), Some("Run this command?"));
    let write = builtin("cursor", CURSOR_WRITE, None);
    assert_eq!(write.state, Some(Blocked));
    assert_eq!(write.detail.as_deref(), Some("Write to this file?"));
    let bg = "  ┃ start the dev server\n\n 1 background task\n  GPT-5 · 97%";
    assert_eq!(state("cursor", bg), Some(Working));
}

// --- GitHub Copilot CLI ----------------------------------------------------

const COPILOT_IDLE: &str = "\
 Welcome to GitHub Copilot CLI
 ~/projects/demo [⎇ main]                                  claude-sonnet-4.5

────────────────────────────────────────────────────────────────────
 >  Enter @ to mention files or / for commands
────────────────────────────────────────────────────────────────────
 Ctrl+c Exit · Ctrl+r Expand recent";

const COPILOT_WORKING: &str = "\
 > summarise the README

 ◉ Thinking (Esc to cancel)

────────────────────────────────────────────────────────────────────
 >  Enter @ to mention files or / for commands
────────────────────────────────────────────────────────────────────";

const COPILOT_APPROVAL: &str = "\
 ╭──────────────────────────────────────────────────────────────╮
 │ Run shell command                                             │
 │ $ npm test                                                    │
 │                                                               │
 │ Do you want to run this command?                              │
 │ ❯ 1. Yes                                                      │
 │   2. Yes, and approve npm for the rest of the session         │
 │   3. No, and tell Copilot what to do differently (Esc)        │
 ╰──────────────────────────────────────────────────────────────╯
 ↑↓ to navigate · Enter to select · Esc to cancel";

#[test]
fn copilot_states() {
    assert_eq!(state("copilot", COPILOT_IDLE), Some(Idle));
    assert_eq!(state("copilot", COPILOT_WORKING), Some(Working));
    let blocked = builtin("copilot", COPILOT_APPROVAL, None);
    assert_eq!(blocked.state, Some(Blocked));
    assert_eq!(blocked.detail.as_deref(), Some("Do you want to run this command?"));
    let bg = " > ship it\n\n ◎ Waiting for background agents · 2 running\n";
    assert_eq!(state("copilot", bg), Some(Working));
}

// --- Qwen Code -------------------------------------------------------------

const QWEN_IDLE: &str = "\
Tips for getting started:
1. Ask questions, edit files, or run commands.

────────────────────────────────────────────────────────────────
>   Type your message or @path/to/file
────────────────────────────────────────────────────────────────
~/projects/demo (main*)        no sandbox        qwen3-coder-plus";

const QWEN_WORKING: &str = "\
> add logging

⠼ Brewing a plan (12s · esc to cancel)

────────────────────────────────────────────────────────────────
>   Type your message or @path/to/file
────────────────────────────────────────────────────────────────
~/projects/demo (main*)        no sandbox        qwen3-coder-plus";

const QWEN_CONFIRM: &str = "\
╭──────────────────────────────────────────────────────────────╮
│ ?  Shell cargo build                                           │
│                                                                │
│ Allow execution of: 'cargo'?                                   │
│                                                                │
│ ● 1. Yes, allow once                                           │
│   2. Yes, allow always ...                                     │
│   3. No, suggest changes (esc)                                 │
╰──────────────────────────────────────────────────────────────╯
⠏ Waiting for user confirmation...";

const QWEN_TRUST: &str = "\
╭──────────────────────────────────────────────────────────────╮
│ Do you trust this folder?                                      │
│                                                                │
│ ● 1. Trust folder (demo)                                       │
│   2. Trust parent folder (projects)                            │
│   3. Don't trust (esc)                                         │
╰──────────────────────────────────────────────────────────────╯";

#[test]
fn qwen_states() {
    assert_eq!(state("qwen", QWEN_IDLE), Some(Idle));
    assert_eq!(state("qwen", QWEN_WORKING), Some(Working));
    let confirm = builtin("qwen", QWEN_CONFIRM, None);
    assert_eq!(confirm.state, Some(Blocked));
    assert_eq!(confirm.detail.as_deref(), Some("Allow execution of: 'cargo'?"));
    let trust = builtin("qwen", QWEN_TRUST, None);
    assert_eq!(trust.state, Some(Blocked));
    assert_eq!(trust.detail.as_deref(), Some("Trust this folder?"));
    // The composer stays visible while working; the cancel hint wins.
    assert_eq!(state("qwen", "(5s · esc to cancel)\n>   Type your message or @path/to/file"), Some(Working));
}

#[test]
fn qwen_titles() {
    let t = |title: &str| builtin("qwen", QWEN_IDLE, Some(title)).state;
    assert_eq!(t("◐ qwen - demo"), Some(Working));
    assert_eq!(t("✳\u{FE0E} qwen - demo"), Some(Blocked));
    assert_eq!(t("qwen - demo"), Some(Idle));
}

// --- Amp -------------------------------------------------------------------

const AMP_IDLE: &str = "\
  Welcome to Amp

╭──────────────────────────────────────────────────────────────╮
│                                                                │
╰─ smart ─────────────────────────────────────────── ~/demo ────╯";

const AMP_WORKING: &str = "\
  > add a changelog entry

  I'll update CHANGELOG.md.

╭──────────────────────────────────────────────────────────────╮
│                                                                │
╰ ≋ Streaming ─────────────────────────── Esc to cancel ────────╯";

const AMP_APPROVAL: &str = "\
  > run the migrations

╭──────────────────────────────────────────────────────────────╮
│ Run this command?                                              │
│ $ npm run migrate                                              │
│ ▸ Approve                                                      │
│   Allow All for This Session                                   │
│   Deny with feedback                                           │
╰──────────────────────────────────────────────────────────────╯
  Waiting for approval…";

#[test]
fn amp_states() {
    assert_eq!(state("amp", AMP_IDLE), Some(Idle));
    assert_eq!(state("amp", AMP_WORKING), Some(Working));
    let blocked = builtin("amp", AMP_APPROVAL, None);
    assert_eq!(blocked.state, Some(Blocked));
    assert_eq!(blocked.detail.as_deref(), Some("Run this command?"));
}

#[test]
fn amp_titles() {
    let t = |title: &str| builtin("amp", AMP_IDLE, Some(title)).state;
    assert_eq!(t("⠋ demo - amp - main"), Some(Working));
    assert_eq!(t("demo - amp - main"), Some(Idle));
    let plugin = builtin("amp", AMP_IDLE, Some("Plugin confirmation needed - amp"));
    assert_eq!(plugin.state, Some(Blocked));
    assert_eq!(plugin.detail.as_deref(), Some("Plugin confirmation needed"));
}

// --- Aider -----------------------------------------------------------------

#[test]
fn aider_states() {
    let idle = "Aider v0.86.1\nMain model: anthropic/claude-sonnet-4 with diff edit format\nGit repo: .git with 42 files\n\n>";
    assert_eq!(state("aider", idle), Some(Idle));
    assert_eq!(state("aider", "src/parser.rs\nask> what does this do"), Some(Idle));
    assert_eq!(state("aider", "architect multi> "), Some(Idle));

    let confirm = builtin(
        "aider",
        "> add the parser\n\nAdd src/parser.rs to the chat? (Y)es/(N)o/(D)on't ask again [Yes]:",
        None,
    );
    assert_eq!(confirm.state, Some(Blocked));
    assert_eq!(confirm.detail.as_deref(), Some("Add src/parser.rs to the chat?"));

    assert_eq!(state("aider", "> explain\n\n░█        Waiting for anthropic/claude-sonnet-4"), Some(Working));
    // Mid-response text: no rule, status falls back to output activity.
    assert_eq!(state("aider", "> explain\n\nThe parser reads tokens and"), None);
    // An answered confirmation scrolled above the prompt is history.
    assert_eq!(
        state("aider", "Run shell command? (Y)es/(N)o/(D)on't ask again [Yes]: y\nok\n>"),
        Some(Idle)
    );
}
