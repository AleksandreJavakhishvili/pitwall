//! The New agent dialog's "Rules" field (`src/components/rules/RulesField.tsx`):
//! the agent's own rule set on top of the project default, and the explicit
//! OK to write rule files into the main checkout. Hidden until there are
//! rule sets and the chosen kind can take rules.
//!
//! The dialog owns one [`RulesField`], tells it what it needs each render
//! ([`RulesField::sync`]), draws it as a child view and spreads
//! [`RulesField::request`] into `CreateAgentRequest`.

use std::time::Duration;

use gpui::{div, prelude::*, px, Context, SharedString, Task, Window};

use pitwall_core::rules::{RuleSet, RulesStatus};

use crate::kit;
use crate::theme::appearance;

use super::call;
use super::widgets::{self, check_row, rich, Span};

/// The project-default lookup waits for typing to pause.
const LOOKUP_DELAY: Duration = Duration::from_millis(250);

/// What the user chose (`RulesChoice`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RulesChoice {
    /// The agent's own extra set (on top of the project default).
    pub rule_set_id: Option<String>,
    /// Explicit OK to write rule files into the main checkout. Default off.
    pub apply_to_main_checkout: bool,
}

impl RulesChoice {
    /// The `CreateAgentRequest` fields (`rulesRequest`): the set, and the
    /// main-checkout OK only without a worktree.
    pub fn request(&self, worktree: bool) -> (Option<String>, bool) {
        (
            self.rule_set_id.clone(),
            !worktree && self.apply_to_main_checkout,
        )
    }
}

pub struct RulesField {
    sets: Option<Vec<RuleSet>>,
    status: Option<RulesStatus>,
    /// The project path last looked up, and its default set.
    project: String,
    project_set: Option<String>,
    can_rules: bool,
    worktree: bool,
    choice: RulesChoice,
    open: bool,
    _lookup: Option<Task<()>>,
}

impl RulesField {
    pub fn new(cx: &mut Context<Self>) -> RulesField {
        call(
            cx,
            |b| (b.sets(), b.status()),
            |f: &mut Self, (sets, status), cx| {
                f.sets = Some(sets);
                f.status = Some(status);
                cx.notify();
            },
        );
        RulesField {
            sets: None,
            status: None,
            project: String::new(),
            project_set: None,
            can_rules: false,
            worktree: false,
            choice: RulesChoice::default(),
            open: false,
            _lookup: None,
        }
    }

    /// The dialog's current kind, folder and worktree choice.
    pub fn sync(&mut self, can_rules: bool, project: &str, worktree: bool, cx: &mut Context<Self>) {
        if can_rules != self.can_rules || worktree != self.worktree {
            self.can_rules = can_rules;
            self.worktree = worktree;
            cx.notify();
        }
        if project == self.project {
            return;
        }
        self.project = project.to_string();
        if project.is_empty() {
            self.project_set = None;
            self._lookup = None;
            cx.notify();
            return;
        }
        let Some(b) = super::backend(cx) else {
            return;
        };
        let path = project.to_string();
        self._lookup = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LOOKUP_DELAY).await;
            let id = cx
                .background_executor()
                .spawn(async move { b.project_rules_for(&path) })
                .await;
            let _ = this.update(cx, |f, cx| {
                f.project_set = id;
                cx.notify();
            });
        }));
    }

    /// Fields for `CreateAgentRequest`: (`rule_set_id`, `apply_to_main_checkout`).
    pub fn request(&self, worktree: bool) -> (Option<String>, bool) {
        if !self.visible() {
            return (None, false);
        }
        self.choice.request(worktree)
    }

    /// There is something to offer: rule sets exist and the kind takes
    /// rules here (`KindView.caps.rules`).
    pub fn visible(&self) -> bool {
        self.can_rules && self.sets.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// The hint under the select.
    pub fn hint(project_set_name: Option<&str>, own: bool, worktree: bool) -> Vec<(String, Span)> {
        let mut out = vec![];
        if let Some(n) = project_set_name {
            out.push(("Project default: ".to_string(), Span::Text));
            out.push((n.to_string(), Span::Bold));
            out.push((". ".to_string(), Span::Text));
        }
        let mut rest = String::new();
        if own {
            rest.push_str("This agent's own rules go to local-only files. ");
        }
        rest.push_str("Generated with rulesync, kept out of git via .git/info/exclude.");
        if worktree {
            rest.push_str(
                " Written into the agent's worktree once it has made one; used from its next session.",
            );
        }
        out.push((rest, Span::Text));
        out
    }
}

impl Render for RulesField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(sets) = self.sets.clone().filter(|_| self.visible()) else {
            return div().into_any_element();
        };
        let t = appearance(cx).theme().float();
        let project_name = sets
            .iter()
            .find(|s| Some(&s.id) == self.project_set.as_ref())
            .map(|s| s.name.clone());
        let any_rules = self.choice.rule_set_id.is_some() || self.project_set.is_some();
        let unavailable = any_rules && self.status.as_ref().is_some_and(|s| !s.available);
        let offered: Vec<&RuleSet> = sets
            .iter()
            .filter(|s| Some(&s.id) != self.project_set.as_ref())
            .collect();
        let none_label = if project_name.is_some() {
            "Project rules only"
        } else {
            "No extra rules"
        };
        let current: SharedString = self
            .choice
            .rule_set_id
            .as_ref()
            .and_then(|id| offered.iter().find(|s| &s.id == id))
            .map(|s| format!("{} ({})", s.name, s.rule_ids.len()))
            .unwrap_or_else(|| none_label.to_string())
            .into();
        let mut items = vec![(none_label.to_string(), self.choice.rule_set_id.is_none())];
        items.extend(offered.iter().map(|s| {
            (
                format!("{} ({})", s.name, s.rule_ids.len()),
                self.choice.rule_set_id.as_ref() == Some(&s.id),
            )
        }));
        let ids: Vec<Option<String>> = std::iter::once(None)
            .chain(offered.iter().map(|s| Some(s.id.clone())))
            .collect();
        let e = cx.entity().downgrade();
        let (e2, e3) = (e.clone(), e.clone());
        let select = widgets::select(
            "na-rules",
            current,
            34.,
            13.,
            true,
            self.open,
            widgets::items(items),
            &t,
            move |_, cx| {
                let _ = e.update(cx, |f, cx| {
                    f.open = !f.open;
                    cx.notify();
                });
            },
            widgets::on_pick(move |i, _, cx| {
                let id = ids[i].clone();
                let _ = e2.update(cx, |f, cx| {
                    f.choice.rule_set_id = id;
                    f.open = false;
                    cx.notify();
                });
            }),
            widgets::on_close(move |_, cx| {
                let _ = e3.update(cx, |f, cx| {
                    f.open = false;
                    cx.notify();
                });
            }),
        );
        let hint = Self::hint(
            project_name.as_deref(),
            self.choice.rule_set_id.is_some(),
            self.worktree,
        );
        let parts: Vec<(&str, Span)> = hint.iter().map(|(s, k)| (s.as_str(), *k)).collect();
        let main = self.choice.apply_to_main_checkout;
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(kit::label("Rules", t.text_3, 12.))
            .child(select)
            .child(div().text_size(px(12.)).child(rich(&parts, t.text_3)))
            .when(unavailable, |d| {
                d.child(kit::error_text(
                    "rulesync isn't available — install it or allow npx in Settings → Rules.",
                    &t,
                ))
            })
            .when(any_rules && !self.worktree, |d| {
                d.child(
                    div().mt(px(4.)).child(check_row(
                        "na-rules-main",
                        main,
                        false,
                        div().child("Write rule files into my main checkout"),
                        Some("Without its own worktree this agent works in your checkout. Off: no rules for this agent.".into()),
                        &t,
                        cx.listener(|f, _, _, cx| {
                            f.choice.apply_to_main_checkout = !f.choice.apply_to_main_checkout;
                            cx.notify();
                        }),
                    )),
                )
            })
            .into_any_element()
    }
}
