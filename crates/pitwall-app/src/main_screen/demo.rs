//! Made-up agents for looking at the main screen without an engine: the
//! same ones as the React browser mock (`src/mock.ts`), so both can be
//! compared side by side (`examples/main_screen_demo.rs`). Not used by the app.

use serde_json::json;

use pitwall_core::model::KindView;
use pitwall_core::vcs::git::FileChange;
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
    "queue": [], "autoSend": true, "lastSent": null, "lastSentAt": null, "createdAt": 0,
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let ago = |min: u64| now.saturating_sub(min * 60_000);
    vec![
        agent(json!({
            "id": "a-tests", "name": "tests", "status": "blocked", "statusSource": "hooks",
            "statusDetail": "Bash: pnpm test --run src/cart",
            "cwd": "/Users/dev/code/checkout-web/.claude/worktrees/tests",
            "cwdDisplay": "~/code/checkout-web/.claude/worktrees/tests",
            "project": "/Users/dev/code/checkout-web", "projectDisplay": "checkout-web",
            "branch": "worktree-tests", "worktree": true, "createdAt": 10,
            "lastSent": "Run the test suite and fix the flaky cart spec. Don't touch the snapshot files.",
            "lastSentAt": ago(4),
            "added": 13, "removed": 4, "filesChanged": 3, "caps": {"removeWorktree": true, "merge": true}
        })),
        agent(json!({
            "id": "a-api-fix", "name": "api-fix", "kind": "codex", "kindName": "Codex", "status": "working",
            "branch": "codex/api-fix", "worktree": true,
            "cwd": "/Users/dev/.codex/worktrees/a1b2/orders-api",
            "cwdDisplay": "~/.codex/worktrees/a1b2/orders-api", "createdAt": 20,
            "queue": [
                {"id": "q1", "text": "Now add the same guard to PATCH /v2/orders/:id and cover it with a test."},
                {"id": "q2", "text": "Summarise the change in 3 bullets for the PR description."}
            ],
            "lastSent": "Fix the 500 on /v2/orders when the cart is empty. Add a regression test.",
            "lastSentAt": ago(11),
            "added": 174, "removed": 74, "filesChanged": 7, "caps": {"removeWorktree": true, "merge": true}
        })),
        agent(json!({
            "id": "a-docs", "name": "docs", "status": "done", "statusSource": "hooks",
            "cwd": "/Users/dev/code/handbook/.claude/worktrees/docs",
            "cwdDisplay": "~/code/handbook/.claude/worktrees/docs",
            "project": "/Users/dev/code/handbook", "projectDisplay": "handbook",
            "branch": "worktree-docs", "worktree": true, "createdAt": 30,
            "lastSent": "Rewrite the onboarding page so a new hire can get a dev env running in 15 minutes.",
            "lastSentAt": ago(26),
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

fn file(path: &str, added: u32, removed: u32, untracked: bool, status: Option<&str>) -> FileChange {
    use pitwall_proto::FileStatus;
    FileChange {
        path: path.into(),
        added,
        removed,
        untracked,
        binary: path.ends_with(".png"),
        status: match status {
            Some("A") => Some(FileStatus::A),
            Some("D") => Some(FileStatus::D),
            Some("M") => Some(FileStatus::M),
            _ => None,
        },
    }
}

/// The mock's changed files per agent name.
pub fn changes(name: &str) -> Vec<FileChange> {
    match name {
        "tests" => vec![
            file("src/cart/cart.spec.ts", 9, 2, false, None),
            file("src/cart/useCart.ts", 3, 1, false, None),
            file("package.json", 1, 1, false, Some("M")),
        ],
        "api-fix" => vec![
            file("src/orders/handler.ts", 18, 6, false, None),
            file("src/orders/pricing.ts", 4, 2, false, None),
            file("src/orders/errors.ts", 22, 0, false, None),
            file("test/orders/empty-cart.test.ts", 41, 0, true, None),
            file("src/orders/schema.ts", 63, 29, false, None),
            file("src/orders/legacy-handler.ts", 0, 37, false, Some("D")),
            file("src/orders/validation/cart.ts", 26, 0, false, Some("A")),
        ],
        "docs" => vec![
            file("docs/onboarding.md", 30, 8, false, None),
            file("docs/dev-env/checklist.md", 34, 0, true, None),
            file("docs/img/setup-flow.png", 0, 0, true, None),
        ],
        _ => vec![],
    }
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

/// The two sides of a one-file diff's hunks.
fn sides(diff: &str) -> (String, String) {
    let (mut old, mut new) = (String::new(), String::new());
    let body = diff.lines().skip_while(|l| !l.starts_with("@@"));
    for l in body.filter(|l| !l.starts_with("@@")) {
        let (mark, text) = l.split_at(l.len().min(1));
        if mark != "+" {
            old.push_str(text);
            old.push('\n');
        }
        if mark != "-" {
            new.push_str(text);
            new.push('\n');
        }
    }
    (old, new)
}

const DIFF: &str = "diff --git a/src/cart/cart.spec.ts b/src/cart/cart.spec.ts
--- a/src/cart/cart.spec.ts
+++ b/src/cart/cart.spec.ts
@@ -38,9 +38,16 @@ import { createCart } from \"./cart\";

 describe(\"cart expiry\", () => {
+  beforeEach(() => vi.useFakeTimers());
+  afterEach(() => vi.useRealTimers());
+
   it(\"expires after 30 minutes\", () => {
     const cart = createCart();
-    const later = Date.now() + 30 * 60 * 1000;
-    expect(cart.isExpired(later)).toBe(true);
+    vi.advanceTimersByTime(30 * 60 * 1000);
+    expect(cart.isExpired()).toBe(true);
+  });
+
+  it(\"is still open after 29 minutes\", () => {
+    vi.advanceTimersByTime(29 * 60 * 1000);
+    expect(createCart().isExpired()).toBe(false);
   });
 });
";

pub fn recents() -> Vec<(String, String)> {
    ["orders-api", "checkout-web", "handbook"]
        .iter()
        .map(|p| (format!("/Users/dev/code/{p}"), format!("~/code/{p}")))
        .collect()
}

impl MainScreen {
    /// Demo only: fill the panel with the mock's changes for the focused agent.
    #[doc(hidden)]
    pub fn demo_changes(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(a) = self.selected(cx) {
            self.changes.agent_for_demo(&a.id, changes(&a.name));
            cx.notify();
        }
    }

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
    /// "diff", "terminal").
    #[doc(hidden)]
    pub fn demo_open(
        &mut self,
        what: &str,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match what {
            "new-agent" => {
                self.open_new_agent(None, window, cx);
                if let Some(super::Modal::NewAgent(d)) = &self.modal {
                    d.update(cx, |d, cx| d.demo_fill(kinds(), recents(), window, cx));
                }
            }
            "remove" => {
                if let Some(id) = self.focused_agent_id() {
                    self.open_remove(&id, cx);
                }
            }
            "terminal" => self.open_terminal_dialog(window, cx),
            "diff" => {
                if let Some(a) = self.selected(cx) {
                    if let Some(f) = changes(&a.name).into_iter().next() {
                        self.open_diff(a, f, window, cx);
                        if let Some(super::Modal::Diff(d)) = &self.modal {
                            d.update(cx, |d, cx| {
                                let (old, new) = sides(DIFF);
                                d.set_versions(Some(old), Some(new), cx)
                            });
                        }
                    }
                }
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
            "bring" => {
                self.demo_elsewhere(cx);
                self.elsewhere.bring = self.elsewhere.rows.first().cloned();
                cx.notify();
            }
            "menu" => {
                let agents = self.agents(cx).to_vec();
                if let Some(a) = agents.first() {
                    self.demo_agent_menu(a, window, cx);
                }
            }
            _ => {}
        }
    }
}
