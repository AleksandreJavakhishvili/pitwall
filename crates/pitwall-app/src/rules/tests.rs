use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::{AppContext, TestAppContext};

use super::field::{RulesChoice, RulesField};
use super::mock::MockRules;
use super::settings::{agent_state, import_message, project_rows, toggle_pick, ImportKind};
use super::widgets::Span;
use super::*;

#[test]
fn labels_drop_the_source() {
    assert_eq!(rule_label("library:web/style.md"), "web/style.md");
    assert_eq!(rule_label("team:a:b.md"), "a:b.md");
    assert_eq!(rule_label("plain.md"), "plain.md");
}

#[test]
fn the_main_checkout_ok_needs_no_worktree() {
    let c = RulesChoice {
        rule_set_id: Some("s1".into()),
        apply_to_main_checkout: true,
    };
    assert_eq!(c.request(false), (Some("s1".into()), true));
    assert_eq!(c.request(true), (Some("s1".into()), false));
    assert_eq!(RulesChoice::default().request(false), (None, false));
}

#[test]
fn the_hint_names_the_project_default() {
    let h = RulesField::hint(Some("Web defaults"), true, true);
    assert_eq!(h[1], ("Web defaults".to_string(), Span::Bold));
    let text: String = h.iter().map(|(s, _)| s.as_str()).collect();
    assert!(text.starts_with("Project default: Web defaults. This agent's own rules"));
    assert!(text.ends_with("used from its next session."));
    let plain: String = RulesField::hint(None, false, false)
        .iter()
        .map(|(s, _)| s.clone())
        .collect();
    assert_eq!(
        plain,
        "Generated with rulesync, kept out of git via .git/info/exclude."
    );
}

#[test]
fn import_kinds_and_messages() {
    assert_eq!(
        ImportKind::Git.hint("~/.pw"),
        "Cloned into ~/.pw/rules-sources; pull to update."
    );
    assert_eq!(ImportKind::File.placeholder(), "/path/to/project/CLAUDE.md");
    assert_eq!(import_message(1, ""), "Imported 1 rule.");
    assert_eq!(import_message(2, "log"), "Imported 2 rules.\nlog");
}

#[test]
fn project_rows_dedupe_and_add_defaults() {
    let mut d = BTreeMap::new();
    d.insert("/x/only-default".to_string(), "s1".to_string());
    d.insert("/x/a".to_string(), "s1".to_string());
    let rows = project_rows(
        vec![
            ("/x/a".into(), "~/a".into()),
            ("/x/b".into(), "~/b".into()),
            ("/x/a".into(), "dup".into()),
        ],
        &d,
    );
    assert_eq!(
        rows,
        vec![
            ("/x/a".to_string(), "~/a".to_string()),
            ("/x/b".to_string(), "~/b".to_string()),
            ("/x/only-default".to_string(), "/x/only-default".to_string()),
        ]
    );
    let many: Vec<(String, String)> = (0..60)
        .map(|i| (format!("/p{i}"), format!("p{i}")))
        .collect();
    assert_eq!(project_rows(many, &BTreeMap::new()).len(), 40);
}

#[test]
fn picks_toggle_in_order() {
    let mut p = vec!["a".to_string()];
    toggle_pick(&mut p, "b");
    toggle_pick(&mut p, "a");
    assert_eq!(p, vec!["b".to_string()]);
}

#[test]
fn the_mock_keeps_names_unique_and_npx_opt_in() {
    let m = MockRules::new();
    let err = m
        .save_set(pitwall_core::rules::SaveRuleSet {
            id: None,
            name: "web DEFAULTS".into(),
            rule_ids: vec![],
        })
        .unwrap_err();
    assert!(err.contains("already exists"));
    assert!(m.apply("a1", false).is_err(), "no rulesync, no npx");
    assert_eq!(m.set_npx(true).unwrap().via, Some("npx"));
    assert!(m.apply("a1", false).is_ok());
    let r = m
        .import("git", "https://example.com/team/ai-rules.git")
        .unwrap();
    assert_eq!(r.added, vec!["ai-rules:shared.md".to_string()]);
    assert_eq!(m.sources()[0].kind, "git");
}

#[gpui::test]
fn the_field_shows_once_sets_load_and_the_kind_takes_rules(cx: &mut TestAppContext) {
    cx.update(|cx| install(cx, Arc::new(MockRules::new())));
    let field = cx.new(RulesField::new);
    cx.run_until_parked();
    field.update(cx, |f, cx| {
        assert!(!f.visible(), "kind without rules");
        f.sync(true, true, "/Users/dev/code/orders-api", false, cx);
        assert!(f.visible());
        assert_eq!(f.request(false), (None, false));
        // The mock's rulesync isn't available: said for a rulesync kind…
        assert!(f.warns_unavailable(true));
        // …not for one Pitwall writes rules for itself (Gemini CLI, Aider).
        f.sync(true, false, "/Users/dev/code/orders-api", false, cx);
        assert!(!f.warns_unavailable(true));
    });
}

#[gpui::test]
fn the_poller_reads_stale_agents(cx: &mut TestAppContext) {
    let mock = Arc::new(MockRules::new());
    mock.set_agent("a1", true, None);
    mock.set_agent("a2", false, Some("rulesync failed".into()));
    mock.set_agent("a3", false, None);
    cx.update(|cx| install(cx, mock.clone()));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(stale::get("a1", cx).is_some_and(|r| r.stale));
        let a2 = stale::get("a2", cx).unwrap();
        assert!(stale::title(&a2).starts_with("Rules not applied: rulesync failed"));
        assert!(stale::get("a3", cx).is_some_and(|r| !r.stale && r.error.is_none()));
    });
}

#[test]
fn agent_chips_say_how_rules_stand() {
    let m = MockRules::new();
    m.set_agent("a1", false, None);
    let mut v = m.agent_rules().remove(0);
    assert_eq!(agent_state(&v).0, "not applied yet");
    v.applied_at = Some(1);
    assert_eq!(agent_state(&v).0, "applied");
    v.main_checkout = true;
    assert_eq!(agent_state(&v).0, "main checkout: not written");
    v.stale = true;
    assert_eq!(agent_state(&v).0, "changed since applied");
    v.error = Some("x".into());
    assert_eq!(agent_state(&v).0, "not applied");
}

#[test]
fn the_mock_details_a_project_by_scope() {
    let m = MockRules::new();
    m.set_agent("a1", false, None);
    m.set_project_rules("/p", Some("s1".into())).unwrap();
    let d = m.project_detail("/p");
    assert_eq!(d.rule_set_id.as_deref(), Some("s1"));
    let froms: Vec<(&str, &str)> = d.agents[0]
        .effective
        .iter()
        .map(|r| (r.from, r.id.as_str()))
        .collect();
    assert_eq!(
        froms,
        vec![
            ("project", "library:typescript.md"),
            ("project", "library:testing.md")
        ]
    );
}

#[test]
fn bulk_messages() {
    use super::stale::{bulk_message, BulkOutcome};
    let name = |id: &str| format!("<{id}>");
    assert_eq!(
        bulk_message(&BulkOutcome::default(), name),
        (true, "No agent's rules changed.".to_string())
    );
    let o = BulkOutcome {
        applied: 2,
        restarted: 1,
        errors: vec![("a3".into(), "rulesync isn't available".into())],
    };
    assert_eq!(
        bulk_message(&o, name),
        (
            false,
            "Re-applied rules for 2 agents, restarted 1. Stopped agents use them when they start.\n<a3>: rulesync isn't available"
                .to_string()
        )
    );
}

/// "Re-apply & restart all" takes every agent whose rules changed or
/// failed, and none is left afterwards.
#[gpui::test]
fn reapply_all_clears_every_stale_agent(cx: &mut TestAppContext) {
    struct Probe {
        affected: Vec<String>,
        done: Option<stale::BulkOutcome>,
    }
    let mock = Arc::new(MockRules::new());
    mock.set_npx(true).unwrap();
    mock.set_agent("a1", true, None);
    mock.set_agent("a2", false, Some("rulesync failed".into()));
    mock.set_agent("a3", false, None);
    cx.update(|cx| install(cx, mock.clone()));
    cx.run_until_parked();
    let probe = cx.new(|_| Probe {
        affected: vec![],
        done: None,
    });
    probe.update(cx, |p, cx| p.affected = stale::affected(cx));
    assert_eq!(
        probe.read_with(cx, |p, _| p.affected.clone()),
        vec!["a1", "a2"]
    );
    let weak = probe.downgrade();
    cx.update(|cx| {
        stale::reapply_all(weak, cx, |p: &mut Probe, out, _| p.done = Some(out));
    });
    cx.run_until_parked();
    let done = probe.read_with(cx, |p, _| p.done.clone()).unwrap();
    assert_eq!((done.applied, done.restarted, done.errors.len()), (2, 0, 0));
    probe.update(cx, |p, cx| p.affected = stale::affected(cx));
    assert!(probe.read_with(cx, |p, _| p.affected.is_empty()));
}

#[test]
fn the_project_picker_groups_rules_by_set() {
    use super::project::{changes, picker_groups};
    let m = MockRules::new();
    let groups = picker_groups(&m.sets(), &m.library());
    assert_eq!(
        groups,
        vec![
            (
                "Web defaults".to_string(),
                vec![
                    "library:typescript.md".to_string(),
                    "library:testing.md".to_string()
                ]
            ),
            (
                "Other skills".to_string(),
                vec!["library:skills/review".to_string()]
            ),
            (
                "Other rules".to_string(),
                vec![
                    "library:overview.md".to_string(),
                    "library:terse.md".to_string()
                ]
            ),
        ]
    );
    let have: std::collections::BTreeSet<String> = ["a".to_string(), "b".to_string()].into();
    let ticked: std::collections::BTreeSet<String> = ["b".to_string(), "c".to_string()].into();
    assert_eq!(
        changes(&have, &ticked),
        (vec!["c".to_string()], vec!["a".to_string()])
    );
}

/// The project panel: tick a set, apply, take an update, and a rule
/// changed in the project is only removed once the user says so.
#[gpui::test]
fn the_project_panel_assigns_updates_and_asks(cx: &mut TestAppContext) {
    use super::project::{Confirm, ProjectPanel};
    use pitwall_proto::ProjectRuleState;
    let mock = Arc::new(MockRules::new());
    mock.hand_written("/p");
    mock.set_project_rules("/p", Some("s1".into())).unwrap();
    cx.update(|cx| install(cx, mock.clone()));
    let panel = cx.new(|cx| ProjectPanel::new("/p".into(), cx));
    cx.run_until_parked();
    panel.read_with(cx, |p, _| {
        let st = p.status().unwrap();
        assert_eq!(st.existing, vec!["CLAUDE.md"]);
        assert_eq!(st.fallback_set.as_deref(), Some("s1"));
    });
    panel.update(cx, |p, cx| p.import_existing(cx));
    cx.run_until_parked();
    assert!(panel.read_with(cx, |p, _| p.status().unwrap().existing.is_empty()));

    let ids = vec![
        "library:typescript.md".to_string(),
        "library:testing.md".to_string(),
    ];
    panel.update(cx, |p, cx| {
        p.toggle_group(&ids, cx);
        p.apply(cx);
    });
    cx.run_until_parked();
    panel.read_with(cx, |p, _| {
        let st = p.status().unwrap();
        assert_eq!(st.rules.len(), 2);
        assert!(st.generated.out_of_date);
        assert!(
            p.message()
                .unwrap()
                .1
                .starts_with("Copied testing.md, typescript.md"),
            "{:?}",
            p.message()
        );
    });

    mock.set_project_rule("/p", "typescript.md", ProjectRuleState::Update);
    mock.set_project_rule("/p", "testing.md", ProjectRuleState::Modified);
    panel.update(cx, |p, cx| p.reload(cx));
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |p, _| p.status().unwrap().updates), 1);
    panel.update(cx, |p, cx| p.update(vec![], None, cx));
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |p, _| p.status().unwrap().updates), 0);

    // Unticking a rule changed in the project asks first.
    panel.update(cx, |p, cx| {
        p.toggle("library:testing.md", cx);
        p.apply(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |p, _| p.confirm().cloned()),
        Some(Confirm::Force(vec!["testing.md".into()]))
    );
    assert_eq!(
        panel.read_with(cx, |p, _| p.status().unwrap().rules.len()),
        2
    );
    panel.update(cx, |p, cx| p.answer(true, cx));
    cx.run_until_parked();
    panel.read_with(cx, |p, _| {
        assert_eq!(p.status().unwrap().rules.len(), 1);
        assert!(p.ticked().contains("library:typescript.md"));
    });
    // Generating needs rulesync (the mock has none until npx is allowed).
    mock.set_npx(true).unwrap();
    panel.update(cx, |p, cx| p.generate(cx));
    cx.run_until_parked();
    assert!(!panel.read_with(cx, |p, _| p.status().unwrap().generated.out_of_date));
}

#[test]
fn skills_are_labelled_by_name() {
    assert_eq!(skill_label("library:skills/review"), "review");
    assert_eq!(skill_label("team:skills/deploy"), "deploy");
    assert_eq!(skill_label("library:style.md"), "style.md");
}

/// A skill is ticked and copied into the project like a rule.
#[gpui::test]
fn the_project_panel_assigns_skills(cx: &mut TestAppContext) {
    use super::project::ProjectPanel;
    let mock = Arc::new(MockRules::new());
    cx.update(|cx| install(cx, mock.clone()));
    let panel = cx.new(|cx| ProjectPanel::new("/q".into(), cx));
    cx.run_until_parked();
    panel.update(cx, |p, cx| {
        p.toggle("library:skills/review", cx);
        p.apply(cx);
    });
    cx.run_until_parked();
    panel.read_with(cx, |p, _| {
        let st = p.status().unwrap();
        assert_eq!(st.rules.len(), 1);
        assert!(st.rules[0].skill && st.rules[0].name == "review");
        assert!(st.existing.is_empty());
    });
}
