use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use pitwall_core::extensions::activity::{Entry, Outcome, Via};
use pitwall_core::extensions::parse;
use pitwall_core::paths::Paths;
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::super::view::SettingsView;
use crate::extensions::calls;

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Click control `id`, the page scrolled first so it is in view (the
/// dialog's page area is shorter than the window).
fn click(view: &Entity<SettingsView>, cx: &mut VisualTestContext, id: &str) {
    frame(cx);
    let sel: &'static str = Box::leak(id.to_string().into_boxed_str());
    let b = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{id} isn't drawn"));
    let (top, bottom) = (gpui::px(120.), gpui::px(super::super::super::view::HEIGHT - 40.));
    let dy = if b.bottom() > bottom { b.bottom() - bottom } else if b.origin.y < top { b.origin.y - top } else { gpui::px(0.) };
    if dy != gpui::px(0.) {
        view.update(cx, |v, cx| {
            v.scroll_by(dy);
            cx.notify();
        });
        frame(cx);
    }
    let b = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{id} isn't drawn"));
    assert!(b.origin.y >= top && b.bottom() <= bottom, "{id} out of view: {b:?}");
    cx.simulate_mouse_move(b.center(), None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
    frame(cx);
}

fn drawn(cx: &mut VisualTestContext, id: &str) -> bool {
    frame(cx);
    cx.debug_bounds(Box::leak(id.to_string().into_boxed_str())).is_some()
}

const CI: &str = r#"{"v": 2, "id": "ci", "name": "CI", "version": "1.0.0",
    "adapter": {"command": ["node", "a.mjs"]},
    "permissions": {"agents.read": "To match runs to agents", "projects.read": "To list your projects", "agents.start": "Later"},
    "contributes": {"inbox": true},
    "settings": [
        {"key": "team", "type": "text", "label": "Team"},
        {"key": "level", "type": "choice", "label": "Level", "options": ["low", "high"], "default": "low"},
        {"key": "branchesOnly", "type": "bool", "label": "Only agents' branches", "default": true},
        {"key": "limit", "type": "number", "label": "Limit", "min": 1, "max": 50}
    ]}"#;

/// Extension "ci"'s folder in the data folder, with `manifest`.
fn put(t: &TempDir, manifest: &str) {
    let dir = t.path().join("pw/extensions/ci");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.json"), manifest).unwrap();
    assert!(parse(manifest).is_ok());
}

/// Settings → Extensions with `manifest` installed by hand.
fn open<'a>(cx: &'a mut TestAppContext, t: &TempDir, manifest: &str) -> (Entity<SettingsView>, &'a mut VisualTestContext) {
    put(t, manifest);
    let paths = Paths::new(t.path().join("pw"));
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::accounts::model(cx).update(cx, |a, _| a.set_paths(paths));
        crate::extensions::init(Some(t.path().join("pw")), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        crate::extensions::follow(cx);
        crate::settings::data::set_page(cx, Page::Extensions);
    });
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(2400.)));
    frame(cx);
    (view, cx)
}

fn grants(cx: &mut VisualTestContext) -> std::collections::BTreeMap<String, pitwall_core::extensions::permissions::Decision> {
    cx.update(|_, cx| crate::ui_state::get(cx).extension("ci").grants)
}

fn allowed(cx: &mut VisualTestContext, perm: &str) -> bool {
    cx.update(|_, cx| calls::api(cx).registry.allows("ci", perm))
}

/// Turning it on shows what it asks for, with its reasons; nothing is on
/// until the user answers; reads are offered on, a later permission is
/// shown as not available; Cancel leaves it off.
#[gpui::test]
fn turning_on_asks_for_its_permissions_first(cx: &mut TestAppContext) {
    let t = TempDir::new("settings-ext-permissions");
    let (view, cx) = open(cx, &t, CI);
    click(&view, cx, "extension-switch-ci");
    assert!(drawn(cx, "extension-permissions-ask-ci"), "the permission screen");
    assert!(!cx.update(|_, cx| crate::ui_state::get(cx).extension_on("ci")), "not on yet");
    assert!(drawn(cx, "extension-permission-pick-ci-agents.read"));
    assert!(drawn(cx, "extension-permission-pick-ci-agents.start"), "offered, off: it acts");
    assert!(drawn(cx, "extension-permission-risk-agents.start"), "with what can go wrong");
    assert!(view.read_with(cx, |v, _| v.exts.asking.as_ref().is_some_and(|(_, p)| p["agents.read"] && !p["agents.start"])), "a read starts on, an action off");
    click(&view, cx, "extension-permissions-cancel-ci");
    assert!(view.read_with(cx, |v, _| v.exts.asking.is_none()), "cancelled");
    assert!(!drawn(cx, "extension-permissions-ask-ci"));
    assert!(!cx.update(|_, cx| crate::ui_state::get(cx).extension_on("ci")));
    assert!(grants(cx).is_empty(), "nothing recorded");

    // Again: refuse projects.read, then Turn on.
    click(&view, cx, "extension-switch-ci");
    click(&view, cx, "extension-permission-pick-ci-projects.read");
    click(&view, cx, "extension-permissions-on-ci");
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension_on("ci")));
    let g = grants(cx);
    assert!(g["agents.read"].granted && !g["projects.read"].granted && g["agents.read"].at > 0);
    assert!(!g["agents.start"].granted, "an action is granted only when picked");
    assert!(allowed(cx, "agents.read") && !allowed(cx, "projects.read"), "the registry follows at once");

    // Revoked in its Permissions: the next call is refused.
    click(&view, cx, "extension-permission-ci-agents.read");
    assert!(!grants(cx)["agents.read"].granted);
    assert!(!allowed(cx, "agents.read"));
    click(&view, cx, "extension-permission-ci-agents.read");
    assert!(allowed(cx, "agents.read"), "and granted again");

    // An action granted later in its Permissions, and revoked.
    click(&view, cx, "extension-permission-ci-agents.start");
    assert!(allowed(cx, "agents.start"));
    click(&view, cx, "extension-permission-ci-agents.start");
    assert!(!allowed(cx, "agents.start"));
}

/// A permission an update added waits, not granted, until the user
/// answers it in the card.
#[gpui::test]
fn a_new_permission_after_an_update_waits_for_the_user(cx: &mut TestAppContext) {
    let t = TempDir::new("settings-ext-permission-new");
    let (view, cx) = open(cx, &t, CI);
    cx.update(|_, cx| {
        crate::ui_state::update(cx, |s| {
            *s = s.clone().with_extension("ci", |e| {
                e.enabled = true;
                e.grants.insert("agents.read".into(), pitwall_core::extensions::permissions::Decision { granted: true, at: 1 });
                e.grants.insert("projects.read".into(), pitwall_core::extensions::permissions::Decision { granted: true, at: 1 });
            })
        })
    });
    // The update asks for worktrees.read too.
    let newer = CI.replace(r#""agents.start": "Later""#, r#""agents.start": "Later", "worktrees.read": "To show branches""#);
    put(&t, &newer);
    cx.update(|_, cx| crate::extensions::rescan(cx));
    frame(cx);
    assert!(drawn(cx, "extension-permission-allow-ci-worktrees.read"), "asked in the card");
    assert!(!allowed(cx, "worktrees.read"), "not granted until allowed");
    assert!(allowed(cx, "agents.read"), "the others stay");
    click(&view, cx, "extension-permission-allow-ci-worktrees.read");
    assert!(grants(cx).get("worktrees.read").is_some_and(|d| d.granted), "{:?}", grants(cx));
    assert!(allowed(cx, "worktrees.read"));
    assert!(!drawn(cx, "extension-permission-allow-ci-worktrees.read"));
}

/// Text, choice, switch and number settings each have their field; the
/// activity list shows what it called and what was refused.
#[gpui::test]
fn typed_settings_and_activity_are_drawn(cx: &mut TestAppContext) {
    let t = TempDir::new("settings-ext-typed");
    let (view, cx) = open(cx, &t, CI);
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("ci", true)));
    frame(cx);
    click(&view, cx, "extension-choice-ci-level-high");
    let value = |cx: &mut VisualTestContext, k: &str| cx.update(|_, cx| crate::extensions::connections::chosen("ci", None, cx).and_then(|c| c.values.get(k).cloned()));
    assert_eq!(value(cx, "level").as_deref(), Some("high"));
    click(&view, cx, "extension-bool-ci-branchesOnly");
    assert_eq!(value(cx, "branchesOnly").as_deref(), Some("false"), "on by default; pressed: off");

    cx.update(|_, cx| {
        let log = calls::activity("ci", cx);
        let mut l = log.lock().unwrap();
        l.push(Entry::now(Via::Adapter, "agent.list", Outcome::Ok));
        l.push(Entry::now(Via::Socket, "agent.create", Outcome::Refused { code: "unsupported".into(), message: "agent.create isn't available to extensions yet".into() }));
    });
    view.update(cx, |v, cx| v.debug_activity("ci", true, cx));
    assert!(drawn(cx, "extension-activity-ci"));
    assert!(drawn(cx, "extension-activity-lines-ci"));
    let shown = view.update(cx, |v, _| v.exts.activity.contains("ci"));
    assert!(shown);
}
