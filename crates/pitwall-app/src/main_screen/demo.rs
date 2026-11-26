//! Made-up agents for looking at the main screen without an engine: the
//! same ones as the React browser mock (`src/mock.ts`), so both can be
//! compared side by side (`examples/main_screen_demo.rs`). Not used by the app.

use serde_json::json;

use pitwall_core::model::KindView;
use pitwall_proto::AgentView;

use super::MainScreen;

const BASE: &str = r#"{
    "kind": "claude", "kindName": "Claude Code", "terminal": false, "sessionId": null,
    "cwd": "/Users/dev/code/orders-api", "cwdDisplay": "~/code/orders-api",
    "project": "/Users/dev/code/orders-api", "projectDisplay": "orders-api",
    "branch": "main", "worktree": false, "worktreePending": false, "location": "local",
    "machine": {"provider": "local", "id": "this-mac", "label": "This Mac", "canCreate": true},
    "agentInTerminal": false, "restartAs": null, "statusSource": "screen", "statusDetail": null,
    "running": true, "cols": 100, "rows": 30, "added": 0, "removed": 0, "filesChanged": 0,
    "createdAt": 0,
    "currentTaskId": null,
    "caps": {"input": true, "restart": true, "resume": false, "stop": true, "removeWorktree": false,
             "diff": true, "review": true, "merge": false, "rules": true, "hooks": true,
             "worktrees": true, "explorer": true, "removeKeepsSession": false}
}"#;

fn agent(v: serde_json::Value) -> AgentView {
    let mut base: serde_json::Value = serde_json::from_str(BASE).expect("the base agent");
    for (k, val) in v.as_object().expect("an object") {
        if k == "caps" {
            for (ck, cv) in val.as_object().expect("caps") {
                base["caps"][ck] = cv.clone();
            }
        } else {
            base[k] = val.clone();
        }
    }
    serde_json::from_value(base).expect("a complete AgentView")
}

/// The mock's six agents.
pub fn agents() -> Vec<AgentView> {
    vec![
        agent(json!({
            "id": "a-tests", "name": "tests", "status": "blocked", "statusSource": "hooks",
            "statusDetail": "Bash: pnpm test --run src/cart",
            "cwd": "/Users/dev/code/checkout-web/.claude/worktrees/tests",
            "cwdDisplay": "~/code/checkout-web/.claude/worktrees/tests",
            "project": "/Users/dev/code/checkout-web", "projectDisplay": "checkout-web",
            "branch": "worktree-tests", "worktree": true, "createdAt": 10,
            "added": 13, "removed": 4, "filesChanged": 3, "caps": {"removeWorktree": true, "merge": true}
        })),
        agent(json!({
            "id": "a-api-fix", "name": "api-fix", "kind": "codex", "kindName": "Codex", "status": "working",
            "branch": "codex/api-fix", "worktree": true,
            "cwd": "/Users/dev/.codex/worktrees/a1b2/orders-api",
            "cwdDisplay": "~/.codex/worktrees/a1b2/orders-api", "createdAt": 20,
            "added": 174, "removed": 74, "filesChanged": 7, "caps": {"removeWorktree": true, "merge": true}
        })),
        agent(json!({
            "id": "a-docs", "name": "docs", "status": "done", "statusSource": "hooks",
            "cwd": "/Users/dev/code/handbook/.claude/worktrees/docs",
            "cwdDisplay": "~/code/handbook/.claude/worktrees/docs",
            "project": "/Users/dev/code/handbook", "projectDisplay": "handbook",
            "branch": "worktree-docs", "worktree": true, "createdAt": 30,
            "added": 64, "removed": 8, "filesChanged": 3, "caps": {"removeWorktree": true, "merge": true}
        })),
        agent(json!({"id": "a-refactor", "name": "refactor", "status": "idle", "createdAt": 40})),
        agent(json!({
            "id": "a-term", "name": "orders-api", "kind": "shell", "kindName": "Shell", "terminal": true,
            "status": "idle", "statusSource": "activity", "createdAt": 55,
            "caps": {"rules": false, "hooks": false}
        })),
        agent(json!({
            "id": "a-scratch", "name": "scratch", "kind": "shell", "kindName": "Shell", "terminal": true,
            "status": "stopped", "statusSource": "activity", "running": false, "createdAt": 50,
            "caps": {"input": false, "stop": false, "rules": false, "hooks": false}
        })),
    ]
}

/// The mock's agent kinds.
pub fn kinds() -> Vec<KindView> {
    let k = |id: &str, name: &str, installed: bool, worktree: bool, custom: bool| KindView {
        id: id.into(),
        name: name.into(),
        installed,
        path: installed.then(|| format!("/opt/homebrew/bin/{id}")),
        worktree,
        caps: pitwall_core::model::KindCaps {
            worktree,
            resume: true,
            rules: true,
            hooks: true,
            custom_command: custom,
            accounts: false,
            rulesync: id != "gemini" && id != "aider",
        },
    };
    vec![
        k("claude", "Claude Code", true, true, false),
        k("codex", "Codex", true, true, false),
        k("gemini", "Gemini CLI", true, false, false),
        k("aider", "Aider", false, false, false),
        k("shell", "Shell", true, false, false),
        k("custom", "Custom command", true, false, true),
    ]
}

pub fn recents() -> Vec<(String, String)> {
    ["orders-api", "checkout-web", "handbook"]
        .iter()
        .map(|p| (format!("/Users/dev/code/{p}"), format!("~/code/{p}")))
        .collect()
}

impl MainScreen {
    /// Demo only: the React mock's two agents in other terminal apps.
    #[doc(hidden)]
    pub fn demo_elsewhere(&mut self, cx: &mut gpui::Context<Self>) {
        use pitwall_core::onboarding::scan::RunningAgent;
        let row = |pid, kind: &str, name: &str, cwd: &str, disp: &str, sid: Option<&str>, title: Option<&str>| {
            RunningAgent {
                pid,
                kind: kind.into(),
                kind_name: name.into(),
                cwd: Some(cwd.into()),
                cwd_display: Some(disp.into()),
                session_id: sid.map(Into::into),
                title: title.map(Into::into),
                in_pitwall: false,
                outside_project: false,
                display_project: None,
            }
        };
        self.elsewhere.rows = vec![
            row(
                41207,
                "claude",
                "Claude Code",
                "/Users/dev/code/infra",
                "~/code/infra",
                Some("5c0ffee0-2b1e-4c55-9d1a-0d5f3e7a9b21"),
                Some("Bump the Terraform AWS provider and fix the plan diff"),
            ),
            row(36504, "codex", "Codex", "/Users/dev/code/checkout-web", "~/code/checkout-web", None, None),
        ];
        cx.notify();
    }

    /// Demo only: open something to look at ("new-agent", "remove", "menu",
    /// "terminal").
    #[doc(hidden)]
    pub fn demo_open(
        &mut self,
        what: &str,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match what {
            "new-agent" | "new-agent-picker" => {
                self.open_new_agent(None, window, cx);
                if let Some(super::Modal::NewAgent(d)) = &self.modal {
                    d.update(cx, |d, cx| {
                        d.demo_fill(kinds(), recents(), window, cx);
                        if what == "new-agent-picker" {
                            d.demo_open_picker(window, cx);
                        }
                    });
                }
            }
            "remove" => {
                if let Some(id) = self.focused_agent_id() {
                    self.open_remove(&id, cx);
                }
            }
            "terminal" => self.open_terminal_dialog(window, cx),
            // `review-prompt`, `review-discard`: Review's dialogs (in Review).
            w if w.starts_with("review-") => {
                let route = self.route_view_of(super::Route::Review).and_then(|v| v.downcast::<crate::review::ReviewRoute>().ok());
                let view = route.and_then(|r| r.read(cx).view().cloned());
                if let Some(view) = view {
                    let what = w["review-".len()..].to_string();
                    view.update(cx, |v, cx| v.demo_dialog(&what, window, cx));
                }
            }
            // `project-accounts <folder>`: that project's Accounts… dialog.
            w if w.starts_with("project-accounts ") => {
                let folder = w["project-accounts ".len()..].trim().to_string();
                self.open_project_accounts(folder, window, cx);
            }
            "toasts" => {
                use super::strip::Toast;
                let ids: Vec<(String, String)> = self
                    .agents(cx)
                    .iter()
                    .map(|a| (a.id.clone(), a.name.clone()))
                    .collect();
                if let Some((id, name)) = ids.iter().find(|(_, n)| n == "api-fix") {
                    self.toast(Toast::done(format!("{name} is done"), id.clone()), cx);
                }
                if let Some((id, name)) = ids.iter().find(|(_, n)| n == "tests") {
                    let mut t = Toast::blocked(format!("{name} needs you"), id.clone());
                    t.detail = Some("Bash: pnpm test --run src/cart".into());
                    self.toast(t, cx);
                }
            }
            // `toast <kind>`: one toast of that kind (the first agent's);
            // `toast clear`: none.
            w if w.starts_with("toast ") => {
                use super::strip::{Toast, ToastAction};
                let (id, name) = self.agents(cx).first().map(|a| (a.id.clone(), a.name.clone())).unwrap_or_default();
                let toast = match w["toast ".len()..].trim() {
                    "blocked" => {
                        let mut t = Toast::blocked(format!("{name} needs you"), id);
                        t.detail = Some("Bash: pnpm test --run src/cart".into());
                        t
                    }
                    "done" => Toast::done(format!("{name} is done"), id),
                    "error" => Toast::error("Couldn't open the worktree".into(), "fatal: not a git repository".into()),
                    "ask" => {
                        let act = |label: &str, primary| ToastAction { label: label.to_string().into(), primary, run: std::rc::Rc::new(|_| eprintln!("pitwall-demo: toast answered")) };
                        Toast::ask("demo".into(), "studio is on your tailnet".into(), Some("Show its agents here?".into()), vec![act("Show", true), act("Not now", false)])
                    }
                    "clear" => {
                        self.toasts.clear();
                        cx.notify();
                        return;
                    }
                    _ => Toast::info("The All space stays in the main window", Some("Other spaces can move to a window of their own.".into())),
                };
                self.toast(toast, cx);
            }
            "bring" => {
                self.demo_elsewhere(cx);
                self.elsewhere.bring = self.elsewhere.rows.first().cloned();
                cx.notify();
            }
            // `menu`, or `menu <x> <y>`: the first agent's menu (there).
            w if w == "menu" || w.starts_with("menu ") => {
                let mut at = w.split_whitespace().skip(1).filter_map(|v| v.parse::<f32>().ok());
                let at = at.next().zip(at.next());
                let agents = self.agents(cx).to_vec();
                if let Some(a) = agents.first() {
                    self.demo_agent_menu(a, at, window, cx);
                }
            }
            _ => {}
        }
    }
}
