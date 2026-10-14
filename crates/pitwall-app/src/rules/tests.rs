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
        f.sync(true, "/Users/dev/code/orders-api", false, cx);
        assert!(f.visible());
        assert_eq!(f.request(false), (None, false));
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
