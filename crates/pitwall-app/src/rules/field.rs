//! The New agent dialog's "Rules & skills" field (`src/components/rules/RulesField.tsx`):
//! the agent's own rule set on top of the project default, and the explicit
//! OK to write rule files into the main checkout. Hidden until there are
//! rule sets and the chosen kind can take rules. The dropdown groups the
//! sets: ones with rules, then ones of skills only.
//!
//! The dialog owns one [`RulesField`], tells it what it needs each render
//! ([`RulesField::sync`]), draws it as a child view and spreads
//! [`RulesField::request`] into `CreateAgentRequest`.

use std::time::Duration;

use gpui::{anchored, deferred, div, prelude::*, px, Context, SharedString, Task, Window};

use pitwall_core::rules::{RuleSet, RulesStatus};

use crate::kit;
use crate::theme::appearance;

use super::widgets::{self, check_row, rich, Span};
use super::{call, is_skill_id};

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
    /// The kind's rules are made with rulesync (Gemini CLI and Aider:
    /// Pitwall writes them itself, so rulesync isn't needed).
    rulesync: bool,
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
            rulesync: false,
            worktree: false,
            choice: RulesChoice::default(),
            open: false,
            _lookup: None,
        }
    }

    /// The dialog's current kind (takes rules; made with rulesync), folder
    /// and worktree choice.
    pub fn sync(
        &mut self,
        can_rules: bool,
        rulesync: bool,
        project: &str,
        worktree: bool,
        cx: &mut Context<Self>,
    ) {
        if can_rules != self.can_rules || rulesync != self.rulesync || worktree != self.worktree {
            self.can_rules = can_rules;
            self.rulesync = rulesync;
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

    /// "rulesync isn't available" is worth saying: rules are picked, the
    /// kind needs rulesync, and it can't run.
    pub fn warns_unavailable(&self, any_rules: bool) -> bool {
        any_rules && self.rulesync && self.status.as_ref().is_some_and(|s| !s.available)
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

/// "Web defaults (2 rules, 1 skill)".
pub fn set_label(s: &RuleSet) -> String {
    let skills = s.rule_ids.iter().filter(|id| is_skill_id(id)).count();
    let rules = s.rule_ids.len() - skills;
    let n = |k: usize, one: &str| format!("{k} {one}{}", if k == 1 { "" } else { "s" });
    let counts = match (rules, skills) {
        (0, 0) => "empty".to_string(),
        (r, 0) => n(r, "rule"),
        (0, k) => n(k, "skill"),
        (r, k) => format!("{}, {}", n(r, "rule"), n(k, "skill")),
    };
    format!("{} ({counts})", s.name)
}

/// The dropdown's groups, as indexes into `sets`: sets with any rule
/// ("Rule sets"), then sets of skills only ("Skills"); empty groups left out.
pub fn groups(sets: &[&RuleSet]) -> Vec<(&'static str, Vec<usize>)> {
    let skills_only =
        |s: &RuleSet| !s.rule_ids.is_empty() && s.rule_ids.iter().all(|id| is_skill_id(id));
    let (skills, rules): (Vec<usize>, Vec<usize>) =
        (0..sets.len()).partition(|&i| skills_only(sets[i]));
    [("Rule sets", rules), ("Skills", skills)]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// One line of the dropdown.
enum Row {
    /// A group's title.
    Head(&'static str),
    /// "No extra rules" (`id` is `None`) or a rule set.
    Set { key: usize, label: SharedString, on: bool, id: Option<String> },
}

/// The dropdown's rows in `panel`: picking one sets the field's choice
/// (from a pop-up: once it has closed, in the field's window).
fn menu(
    panel: gpui::Div,
    rows: &[Row],
    t: &crate::theme::Theme,
    field: &gpui::WeakEntity<RulesField>,
) -> gpui::Stateful<gpui::Div> {
    panel.id("na-rules-menu").max_h(px(320.)).overflow_y_scroll().children(rows.iter().map(|row| match row {
        Row::Head(heading) => kit::label(*heading, t.text_3, 11.).px(px(8.)).pt(px(8.)).pb(px(2.)).into_any_element(),
        Row::Set { key, label, on, id } => {
            let (field, id) = (field.clone(), id.clone());
            kit::menu_item(("na-rules-item", *key), if *on { "check" } else { "" }, label.clone(), None, false, t)
                .on_click(move |_, window, cx| {
                    let (field, id) = (field.clone(), id.clone());
                    kit::pop::pick(window, cx, move |_, cx| {
                        let _ = field.update(cx, |f, cx| {
                            f.choice.rule_set_id = id;
                            f.open = false;
                            cx.notify();
                        });
                    })
                })
                .into_any_element()
        }
    }))
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
        let unavailable = self.warns_unavailable(any_rules);
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
            .map(|s| set_label(s))
            .unwrap_or_else(|| none_label.to_string())
            .into();
        let e = cx.entity().downgrade();
        let (e2, e3) = (e.clone(), e.clone());
        // The kit's select draws the trigger; the menu is grouped here.
        let trigger = widgets::select(
            "na-rules",
            current,
            34.,
            13.,
            true,
            false,
            vec![],
            &t,
            move |_, cx| {
                let _ = e.update(cx, |f, cx| {
                    f.open = !f.open;
                    cx.notify();
                });
            },
            widgets::on_pick(|_, _, _| {}),
            widgets::on_close(|_, _| {}),
        );
        // The dropdown, as values: it is also drawn in a glass pop-up,
        // whose window must not touch this field while it draws.
        let mut rows = vec![Row::Set {
            key: 0,
            label: none_label.into(),
            on: self.choice.rule_set_id.is_none(),
            id: None,
        }];
        for (heading, idx) in groups(&offered) {
            rows.push(Row::Head(heading));
            for i in idx {
                let s = offered[i];
                let on = self.choice.rule_set_id.as_ref() == Some(&s.id);
                rows.push(Row::Set { key: i + 1, label: set_label(s).into(), on, id: Some(s.id.clone()) });
            }
        }
        let close: kit::OnClose = std::rc::Rc::new(move |_, cx| {
            let _ = e3.update(cx, |f, cx| {
                f.open = false;
                cx.notify();
            });
        });
        let place = div().absolute().left_0().top(px(36.));
        let menu = if !self.open {
            None
        } else if kit::pop::hosted(cx) {
            let build: kit::pop::Build = std::rc::Rc::new(move |_: &mut Window, cx: &mut gpui::App| {
                menu(kit::menu_body(), &rows, &kit::pop::theme(cx), &e2).into_any_element()
            });
            let pop = kit::pop::Pop {
                at: None,
                radius: kit::pop::MENU_RADIUS,
                focus: None,
                build,
                on_dismiss: close,
            };
            Some(place.child(kit::pop::anchor("na-rules-pop", pop)))
        } else {
            let menu = menu(kit::menu_panel(&t), &rows, &t, &e2)
                .occlude()
                .border_1()
                .border_color(t.line_strong)
                .on_mouse_down_out(move |_, window, cx| close(window, cx));
            Some(place.child(deferred(anchored().snap_to_window().child(menu))))
        };
        let select = div().relative().w_full().child(trigger).children(menu);
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
            .child(kit::label("Rules & skills", t.text_3, 12.))
            .child(select)
            .child(div().text_size(px(12.)).child(rich(&parts, t.text_3)))
            .when(unavailable, |d| {
                d.child(kit::error_text(
                    "rulesync isn't available — install it or allow npx in Settings → Rules & skills.",
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
