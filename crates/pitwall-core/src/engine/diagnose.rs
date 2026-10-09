//! Why an agent shows the status it does (`pitwall agent status`): the
//! three inputs the ticker folds (hooks > screen rules > output activity),
//! what each said last, and a plain-language explanation — the answer to
//! "why is this agent unknown?".

use std::sync::atomic::Ordering;

use pitwall_proto::{ActivitySignal, AgentDiagnosis, HookSignal, ScreenSignal};

use super::status::ACTIVITY_WINDOW_MS;
use super::Engine;
use crate::hooks::HookState;
use crate::kind::HookMode;
use crate::model::{Source, Status};

fn hook_word(h: HookState) -> &'static str {
    match h {
        HookState::Working => "working",
        HookState::Blocked => "blocked",
        HookState::Done => "done",
        HookState::Idle => "idle",
    }
}

fn screen_word(d: pitwall_detect::Detected) -> &'static str {
    match d {
        pitwall_detect::Detected::Working => "working",
        pitwall_detect::Detected::Blocked => "blocked",
        pitwall_detect::Detected::Idle => "idle",
    }
}

fn mode_word(m: Option<&HookMode>) -> &'static str {
    match m {
        Some(HookMode::ClaudeSettings) => "claude-settings",
        Some(HookMode::CodexGlobal) => "codex-global",
        Some(HookMode::None) | None => "none",
    }
}

/// Agent `id`'s status and the signals behind it. Reads only what the
/// engine already has (no I/O but the Codex hooks file).
pub fn diagnose(engine: &Engine, id: &str) -> Result<AgentDiagnosis, String> {
    let now = engine.now();
    let mut d = engine.with(id, |a| {
        let last = a.host.as_ref().map(|h| h.last_activity.load(Ordering::Relaxed)).unwrap_or(0);
        AgentDiagnosis {
            agent_id: a.rec.id.clone(),
            name: a.rec.name.clone(),
            kind: a.kind().to_string(),
            status: a.status,
            source: a.source,
            detail: a.detail.clone(),
            running: a.running(),
            attached: a.host.is_some(),
            status_for_ms: now.saturating_sub(a.status_since),
            hooks: HookSignal {
                supported: a.facts.kind.hooks,
                mode: String::new(),
                seen: a.hook.is_some(),
                last: a.hook.as_ref().map(|h| hook_word(h.0).to_string()),
                installed: None,
            },
            screen: ScreenSignal {
                rules: false,
                last: a.screen_state.map(|s| screen_word(s).to_string()),
                detail: a.screen_detail.clone(),
            },
            activity: ActivitySignal {
                last_output_ms_ago: (last > 0).then(|| now.saturating_sub(last)),
                window_ms: ACTIVITY_WINDOW_MS,
            },
            explanation: vec![],
        }
    })?;
    let kind = engine.kinds().find(&d.kind);
    let mode = kind.as_ref().map(|k| &k.hooks);
    d.hooks.mode = mode_word(mode).into();
    if mode == Some(&HookMode::CodexGlobal) {
        d.hooks.installed = Some(crate::hooks::codex_status(engine.paths()).installed);
    }
    d.screen.rules = pitwall_detect::builtin_rule_kinds().any(|k| k == d.kind);
    d.explanation = explain(&d);
    Ok(d)
}

/// Why it shows what it shows, most important first.
pub fn explain(d: &AgentDiagnosis) -> Vec<String> {
    let mut out = vec![];
    match d.status {
        Status::Stopped => out.push("It isn't running: Pitwall has no terminal for it (stopped, or never started). `agent restart` starts it.".into()),
        Status::Exited => out.push("Its process ended (the program quit or crashed). `agent restart` starts it again.".into()),
        _ => {}
    }
    if matches!(d.status, Status::Stopped | Status::Exited) {
        return out;
    }
    match d.source {
        Source::Hooks => out.push(format!(
            "Its hooks decide the status: the last hook said {}.",
            d.hooks.last.as_deref().unwrap_or("nothing")
        )),
        Source::Screen => out.push(format!(
            "Hooks haven't reported, so its screen decides: the screen rules read {}{}.",
            d.screen.last.as_deref().unwrap_or("nothing"),
            d.screen.detail.as_deref().map(|x| format!(" ({x})")).unwrap_or_default()
        )),
        Source::Activity => out.push(match d.activity.last_output_ms_ago {
            Some(ms) if ms < d.activity.window_ms => "Neither hooks nor screen rules gave a state; it printed something just now, so it counts as working.".into(),
            Some(ms) => format!(
                "Neither hooks nor screen rules gave a state, and it last printed {}s ago, so the status is unknown.",
                ms / 1000
            ),
            None => "Neither hooks nor screen rules gave a state, and it hasn't printed anything yet, so the status is unknown.".into(),
        }),
    }
    if d.source != Source::Hooks {
        if !d.hooks.supported {
            out.push(match d.hooks.mode.as_str() {
                "none" => format!("Its kind ({}) has no hooks in Pitwall.", d.kind),
                _ => "Its machine can't deliver hooks to Pitwall.".into(),
            });
        } else if d.hooks.installed == Some(false) {
            out.push("Codex hooks aren't installed: `pitwall settings set agents.codexHooks true` (asks the user).".into());
        } else if !d.hooks.seen {
            out.push("Hooks are set up but none arrived since it started (it may predate them: restart it).".into());
        }
    }
    if d.source == Source::Activity {
        if !d.screen.rules {
            out.push(format!("Pitwall has no screen rules for {}.", d.kind));
        } else if d.screen.last.is_none() {
            out.push("Its screen rules matched nothing on the current screen.".into());
        }
    }
    if d.status == Status::Done {
        out.push("Done means it finished a turn the user hasn't looked at yet; it turns idle once seen.".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{record, Harness};

    #[test]
    fn a_stopped_agent_says_why() {
        let h = Harness::new(vec![record("a", "/tmp")]);
        let d = diagnose(&h.engine, "a").unwrap();
        assert_eq!((d.status, d.running, d.attached), (Status::Stopped, false, false));
        assert!(d.explanation[0].contains("isn't running"), "{:?}", d.explanation);
        assert!(diagnose(&h.engine, "ghost").is_err());
    }

    #[test]
    fn unknown_is_explained_by_the_missing_signals() {
        let base = AgentDiagnosis {
            agent_id: "a".into(),
            name: "a".into(),
            kind: "mystery".into(),
            status: Status::Unknown,
            source: Source::Activity,
            detail: None,
            running: true,
            attached: true,
            status_for_ms: 0,
            hooks: HookSignal { supported: false, mode: "none".into(), seen: false, last: None, installed: None },
            screen: ScreenSignal { rules: false, last: None, detail: None },
            activity: ActivitySignal { last_output_ms_ago: Some(9000), window_ms: ACTIVITY_WINDOW_MS },
            explanation: vec![],
        };
        let why = explain(&base);
        assert!(why[0].contains("last printed 9s ago"), "{why:?}");
        assert!(why.iter().any(|l| l.contains("has no hooks")), "{why:?}");
        assert!(why.iter().any(|l| l.contains("no screen rules for mystery")), "{why:?}");
        let codex = AgentDiagnosis {
            kind: "codex".into(),
            hooks: HookSignal { supported: true, mode: "codex-global".into(), seen: false, last: None, installed: Some(false) },
            screen: ScreenSignal { rules: true, last: None, detail: None },
            ..base
        };
        let why = explain(&codex);
        assert!(why.iter().any(|l| l.contains("agents.codexHooks")), "{why:?}");
        assert!(why.iter().any(|l| l.contains("matched nothing")), "{why:?}");
    }
}
