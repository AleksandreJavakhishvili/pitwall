//! Settings → Rules → a project's own rules (docs/spec/rules.md, "Project
//! rules"): library rules, ticked by set or one by one, copied into the
//! repository's `.rulesync/rules/` for the user to commit; updates from the
//! library with their diff (taken only on request; conflicts need a
//! choice); the generated files (CLAUDE.md, AGENTS.md, …) and whether they
//! are out of date; importing a hand-written CLAUDE.md first; and moving the
//! old default set into the project.
//!
//! One [`ProjectPanel`] per opened project row. Files that need a decision
//! (a different file already there, a rule changed in the project, a
//! hand-written generated file) are asked about inline before anything is
//! replaced.

use std::collections::BTreeSet;

use gpui::{div, prelude::*, px, Context, FontWeight, SharedString, Window};

use pitwall_core::rules::{RuleFile, RuleSet};
use pitwall_proto::{ProjectRuleState, ProjectRuleUpdate, ProjectRulesStatus};

use crate::kit::{self, BtnKind, Tone};
use crate::theme::{appearance, Theme, RADIUS};

use super::widgets::{check_row, mini};
use super::{call, rule_label, skill_label};

/// Picker groups: each set with its skills and rules (those still in the
/// library), then "Other skills" and "Other rules" for those in no set.
pub fn picker_groups(sets: &[RuleSet], lib: &[RuleFile]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = sets
        .iter()
        .map(|s| {
            let ids = s
                .rule_ids
                .iter()
                .filter(|id| lib.iter().any(|r| &r.id == *id))
                .cloned()
                .collect();
            (s.name.clone(), ids)
        })
        .filter(|(_, ids): &(String, Vec<String>)| !ids.is_empty())
        .collect();
    for (label, skill) in [("Other skills", true), ("Other rules", false)] {
        let other: Vec<String> = lib
            .iter()
            .filter(|r| r.skill == skill && !sets.iter().any(|s| s.rule_ids.contains(&r.id)))
            .map(|r| r.id.clone())
            .collect();
        if !other.is_empty() {
            groups.push((label.into(), other));
        }
    }
    groups
}

/// Library rules the project has a copy of.
pub fn in_project(st: &ProjectRulesStatus) -> BTreeSet<String> {
    st.rules.iter().filter_map(|r| r.source.clone()).collect()
}

/// What ticking changed: (rules to copy in, rules to take out).
pub fn changes(have: &BTreeSet<String>, ticked: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    (
        ticked.difference(have).cloned().collect(),
        have.difference(ticked).cloned().collect(),
    )
}

/// A question before anything is replaced.
#[derive(Debug, Clone, PartialEq)]
pub enum Confirm {
    /// Files already in `.rulesync/rules/` with other content (names), for
    /// these rule ids.
    Overwrite(Vec<String>, Vec<String>),
    /// Rules changed in the project that unticking would delete.
    Force(Vec<String>),
    /// Files generation would replace that Pitwall didn't write.
    Regenerate(Vec<String>),
}

pub struct ProjectPanel {
    path: String,
    lib: Vec<RuleFile>,
    sets: Vec<RuleSet>,
    status: Option<ProjectRulesStatus>,
    updates: Vec<ProjectRuleUpdate>,
    ticked: BTreeSet<String>,
    diff_open: Option<String>,
    confirm: Option<Confirm>,
    busy: bool,
    msg: Option<(bool, String)>,
}

type Done = Result<String, String>;

impl ProjectPanel {
    pub fn new(path: String, cx: &mut Context<Self>) -> ProjectPanel {
        let mut p = ProjectPanel {
            path,
            lib: vec![],
            sets: vec![],
            status: None,
            updates: vec![],
            ticked: BTreeSet::new(),
            diff_open: None,
            confirm: None,
            busy: false,
            msg: None,
        };
        p.reload(cx);
        p
    }

    /// The project's rules, read now (it answers with its repo root).
    pub fn status(&self) -> Option<&ProjectRulesStatus> {
        self.status.as_ref()
    }

    pub fn ticked(&self) -> &BTreeSet<String> {
        &self.ticked
    }

    pub fn confirm(&self) -> Option<&Confirm> {
        self.confirm.as_ref()
    }

    pub fn message(&self) -> Option<&(bool, String)> {
        self.msg.as_ref()
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        call(
            cx,
            move |b| {
                (
                    b.library(),
                    b.sets(),
                    b.project_status(&path),
                    b.project_updates(&path).unwrap_or_default(),
                )
            },
            |p: &mut Self, (lib, sets, st, updates), cx| {
                p.lib = lib;
                p.sets = sets;
                match st {
                    Ok(st) => {
                        p.ticked = in_project(&st);
                        p.status = Some(st);
                    }
                    Err(e) => p.msg = Some((false, e)),
                }
                p.updates = updates;
                cx.notify();
            },
        );
    }

    /// Run a change, then say how it went and read everything again.
    fn run(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&dyn super::RulesBackend, &str) -> (Done, Option<Confirm>) + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.confirm = None;
        self.msg = None;
        cx.notify();
        let path = self.path.clone();
        call(
            cx,
            move |b| f(b, &path),
            |p: &mut Self, (done, confirm), cx| {
                p.busy = false;
                p.msg = Some(match done {
                    Ok(m) => (true, m),
                    Err(e) => (false, e),
                })
                .filter(|(ok, m)| !(*ok && m.is_empty()));
                p.confirm = confirm;
                p.reload(cx);
                super::stale::refresh(cx);
            },
        );
    }

    pub fn toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.ticked.remove(id) {
            self.ticked.insert(id.to_string());
        }
        cx.notify();
    }

    /// Tick (or untick, when all are ticked) a whole set.
    pub fn toggle_group(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if ids.iter().all(|i| self.ticked.contains(i)) {
            for i in ids {
                self.ticked.remove(i);
            }
        } else {
            self.ticked.extend(ids.iter().cloned());
        }
        cx.notify();
    }

    /// Copy what was ticked in, take out what was unticked.
    pub fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(st) = &self.status else {
            return;
        };
        let (add, remove) = changes(&in_project(st), &self.ticked);
        if add.is_empty() && remove.is_empty() {
            return;
        }
        self.run(cx, move |b, path| {
            let mut said = vec![];
            let mut confirm = None;
            if !add.is_empty() {
                match b.project_assign(path, &add, &[]) {
                    Ok(r) => {
                        if !r.copied.is_empty() {
                            said.push(format!(
                                "Copied {} into .rulesync/rules.",
                                r.copied.join(", ")
                            ));
                        }
                        said.extend(r.skipped.iter().map(|s| format!("Skipped {s}.")));
                        if !r.conflicts.is_empty() {
                            confirm = Some(Confirm::Overwrite(r.conflicts, add.clone()));
                        }
                    }
                    Err(e) => return (Err(e), None),
                }
            }
            if !remove.is_empty() {
                match b.project_unassign(path, &remove, false) {
                    Ok(r) => {
                        if !r.removed.is_empty() {
                            said.push(format!("Removed {}.", r.removed.join(", ")));
                        }
                        if !r.modified.is_empty() && confirm.is_none() {
                            confirm = Some(Confirm::Force(r.modified));
                        }
                    }
                    Err(e) => return (Err(e), None),
                }
            }
            (Ok(said.join(" ")), confirm)
        });
    }

    /// The user answered an inline question: yes does it anyway.
    pub fn answer(&mut self, yes: bool, cx: &mut Context<Self>) {
        let Some(c) = self.confirm.take() else {
            return;
        };
        if !yes {
            cx.notify();
            return;
        }
        match c {
            Confirm::Overwrite(names, ids) => self.run(cx, move |b, path| {
                match b.project_assign(path, &ids, &names) {
                    Ok(r) => (Ok(format!("Replaced {}.", r.copied.join(", "))), None),
                    Err(e) => (Err(e), None),
                }
            }),
            Confirm::Force(names) => self.run(cx, move |b, path| {
                match b.project_unassign(path, &names, true) {
                    Ok(r) => (Ok(format!("Removed {}.", r.removed.join(", "))), None),
                    Err(e) => (Err(e), None),
                }
            }),
            Confirm::Regenerate(names) => self.generate_with(names, cx),
        }
    }

    /// Take library updates (`rules` empty: every one); `take` decides
    /// conflicts.
    pub fn update(
        &mut self,
        rules: Vec<String>,
        take: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        self.run(cx, move |b, path| {
            match b.project_update(path, &rules, take) {
                Ok(r) => {
                    let mut said = vec![];
                    if !r.updated.is_empty() {
                        said.push(format!("Updated {}.", r.updated.join(", ")));
                    }
                    if !r.kept.is_empty() {
                        said.push(format!("Kept the project's {}.", r.kept.join(", ")));
                    }
                    if !r.conflicts.is_empty() {
                        said.push(format!(
                            "{} changed in both places: choose for each.",
                            r.conflicts.join(", ")
                        ));
                    }
                    (Ok(said.join(" ")), None)
                }
                Err(e) => (Err(e), None),
            }
        });
    }

    pub fn import_existing(&mut self, cx: &mut Context<Self>) {
        self.run(cx, |b, path| match b.project_import_existing(path) {
            Ok(r) => (
                Ok(format!(
                    "Imported {} into .rulesync/rules ({}).",
                    r.files.join(", "),
                    r.added.join(", ")
                )),
                None,
            ),
            Err(e) => (Err(e), None),
        });
    }

    pub fn generate(&mut self, cx: &mut Context<Self>) {
        self.generate_with(vec![], cx);
    }

    fn generate_with(&mut self, overwrite: Vec<String>, cx: &mut Context<Self>) {
        self.run(cx, move |b, path| {
            match b.project_generate(path, &overwrite) {
                Ok(r) => {
                    let said = if r.written.is_empty() {
                        "The generated files were already up to date.".to_string()
                    } else {
                        format!("Generated {}.", r.written.join(", "))
                    };
                    let confirm = (!r.needs_confirm.is_empty())
                        .then_some(Confirm::Regenerate(r.needs_confirm));
                    (Ok(said), confirm)
                }
                Err(e) => (Err(e), None),
            }
        });
    }

    pub fn move_default(&mut self, cx: &mut Context<Self>) {
        self.run(cx, |b, path| match b.project_move_default(path) {
            Ok(r) => {
                let confirm = (!r.conflicts.is_empty())
                    .then(|| Confirm::Overwrite(r.conflicts.clone(), vec![]));
                (
                    Ok(format!(
                        "Copied {} into .rulesync/rules.",
                        r.copied.join(", ")
                    )),
                    confirm,
                )
            }
            Err(e) => (Err(e), None),
        });
    }

    fn toggle_diff(&mut self, name: &str, cx: &mut Context<Self>) {
        self.diff_open = if self.diff_open.as_deref() == Some(name) {
            None
        } else {
            Some(name.to_string())
        };
        cx.notify();
    }
}

// ───────────────────────────────────────────── drawing

fn banner(t: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(6.))
        .rounded(RADIUS)
        .border_1()
        .border_color(t.amber_glow)
        .bg(t.amber_soft)
        .text_size(px(12.))
        .text_color(t.text)
}

fn state_chip(s: ProjectRuleState, t: &Theme) -> gpui::Div {
    let (text, tone) = match s {
        ProjectRuleState::Current => ("in the project", Tone::Ok),
        ProjectRuleState::Update => ("update", Tone::Warn),
        ProjectRuleState::Modified => ("changed here", Tone::Subtle),
        ProjectRuleState::Conflict => ("changed in both", Tone::Warn),
        ProjectRuleState::SourceGone => ("gone from the library", Tone::Subtle),
        ProjectRuleState::Deleted => ("deleted here", Tone::Subtle),
        ProjectRuleState::Own => ("the project's own", Tone::Plain),
    };
    kit::chip_tone(text, tone, t)
}

impl ProjectPanel {
    fn picker(&self, st: &ProjectRulesStatus, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let mut col = div().flex().flex_col().gap(px(6.));
        for (g, (name, ids)) in picker_groups(&self.sets, &self.lib).into_iter().enumerate() {
            let all = ids.iter().all(|i| self.ticked.contains(i));
            let group_ids = ids.clone();
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(kit::label(name, t.text_3, 12.))
                    .child(div().flex_1())
                    .child(
                        mini(
                            ("proj-group", g),
                            if all { "None" } else { "All" },
                            false,
                            t,
                        )
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle_group(&group_ids, cx))),
                    ),
            );
            for (i, id) in ids.into_iter().enumerate() {
                let state = st
                    .rules
                    .iter()
                    .find(|r| r.source.as_deref() == Some(id.as_str()))
                    .map(|r| r.state);
                let entry = self.lib.iter().find(|r| r.id == id);
                let desc = entry.and_then(|r| r.description.clone());
                let skill = entry.is_some_and(|r| r.skill);
                let title = div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div().font_family(kit::MONO_FONT).text_size(px(12.)).child(
                            if skill {
                                skill_label(&id)
                            } else {
                                rule_label(&id)
                            }
                            .to_string(),
                        ),
                    )
                    .when(skill, |d| d.child(kit::chip_subtle("skill", t)))
                    .children(
                        state
                            .filter(|s| *s != ProjectRuleState::Current)
                            .map(|s| state_chip(s, t)),
                    );
                let key = id.clone();
                col = col.child(check_row(
                    SharedString::from(format!("proj-pick-{g}-{i}")),
                    self.ticked.contains(&id),
                    self.busy,
                    title,
                    desc.map(Into::into),
                    t,
                    cx.listener(move |p, _, _, cx| p.toggle(&key, cx)),
                ));
            }
        }
        let own: Vec<String> = st
            .rules
            .iter()
            .filter(|r| r.state == ProjectRuleState::Own)
            .map(|r| {
                if r.skill {
                    format!("skills/{}", r.name)
                } else {
                    r.name.clone()
                }
            })
            .collect();
        if !own.is_empty() {
            col = col.child(kit::hint(
                format!("Also in .rulesync: {} (the project's own).", own.join(", ")),
                t,
            ));
        }
        col
    }

    fn updates_list(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let pending: Vec<&ProjectRuleUpdate> = self
            .updates
            .iter()
            .filter(|u| u.state != ProjectRuleState::Modified)
            .collect();
        if pending.is_empty() {
            return None;
        }
        let mut col = div().flex().flex_col().gap(px(6.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(kit::label("Library updates", t.text_3, 12.))
                .child(div().flex_1())
                .child(
                    mini("proj-update-all", "Update all", self.busy, t)
                        .on_click(cx.listener(|p, _, _, cx| p.update(vec![], None, cx))),
                ),
        );
        for (i, u) in pending.into_iter().enumerate() {
            let name = u.name.clone();
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .font_family(kit::MONO_FONT)
                        .text_size(px(12.))
                        .child(u.name.clone()),
                )
                .child(state_chip(u.state, t))
                .child(div().flex_1())
                .child({
                    let n = name.clone();
                    mini(
                        ("proj-diff", i),
                        if self.diff_open.as_deref() == Some(&u.name) {
                            "Hide diff"
                        } else {
                            "Diff"
                        },
                        false,
                        t,
                    )
                    .on_click(cx.listener(move |p, _, _, cx| p.toggle_diff(&n, cx)))
                });
            if u.state == ProjectRuleState::Conflict {
                let (a, b) = (name.clone(), name.clone());
                row = row
                    .child(
                        mini(("proj-take-lib", i), "Take library's", self.busy, t).on_click(
                            cx.listener(move |p, _, _, cx| {
                                p.update(vec![a.clone()], Some("library"), cx)
                            }),
                        ),
                    )
                    .child(
                        mini(("proj-keep", i), "Keep project's", self.busy, t).on_click(
                            cx.listener(move |p, _, _, cx| {
                                p.update(vec![b.clone()], Some("project"), cx)
                            }),
                        ),
                    );
            } else {
                let a = name.clone();
                row =
                    row.child(mini(("proj-take", i), "Update", self.busy, t).on_click(
                        cx.listener(move |p, _, _, cx| p.update(vec![a.clone()], None, cx)),
                    ));
            }
            col = col.child(row);
            if self.diff_open.as_deref() == Some(&u.name) {
                let lines: Vec<_> = u
                    .diff
                    .lines()
                    .map(|l| {
                        let color = if l.starts_with('+') && !l.starts_with("+++") {
                            t.green
                        } else if l.starts_with('-') && !l.starts_with("---") {
                            t.red
                        } else {
                            t.text_2
                        };
                        div().text_color(color).child(l.to_string())
                    })
                    .collect();
                col = col.child(
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(RADIUS)
                        .bg(t.surface_2)
                        .border_1()
                        .border_color(t.line)
                        .font_family(kit::MONO_FONT)
                        .text_size(px(11.5))
                        .children(lines),
                );
            }
        }
        Some(col)
    }

    fn confirm_box(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let text = match self.confirm.as_ref()? {
            Confirm::Overwrite(names, _) => format!(
                "{} already in .rulesync/rules with other content. Replace with the library's?",
                names.join(", ")
            ),
            Confirm::Force(names) => format!("{} changed in the project. Remove anyway?", names.join(", ")),
            Confirm::Regenerate(names) => format!(
                "{} not written by Pitwall (hand-written or edited). Replace with the generated version?",
                names.join(", ")
            ),
        };
        Some(
            banner(t)
                .child(div().flex_1().min_w_0().child(text))
                .child(
                    kit::text_button("proj-confirm-yes", "Replace", BtnKind::Small, self.busy, t)
                        .on_click(cx.listener(|p, _, _, cx| p.answer(true, cx))),
                )
                .child(
                    kit::text_button("proj-confirm-no", "Keep", BtnKind::Ghost, false, t)
                        .on_click(cx.listener(|p, _, _, cx| p.answer(false, cx))),
                ),
        )
    }
}

impl Render for ProjectPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = appearance(cx).theme().float();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(10.))
            .pt(px(2.))
            .pb(px(10.));
        let Some(st) = self.status.clone() else {
            return body
                .child(kit::hint("Reading…", &t))
                .when_some(self.msg.clone(), |d, (_, m)| {
                    d.child(kit::error_text(m, &t))
                });
        };
        let (add, remove) = changes(&in_project(&st), &self.ticked);
        let n = add.len() + remove.len();
        let g = &st.generated;
        let mut head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child("Project skills & rules"),
            )
            .child(kit::hint(".rulesync, committed with the project", &t))
            .child(div().flex_1());
        if st.updates > 0 {
            head = head.child(kit::chip_tone(
                if st.updates == 1 {
                    "1 update".to_string()
                } else {
                    format!("{} updates", st.updates)
                },
                Tone::Warn,
                &t,
            ));
        }
        if g.out_of_date {
            head = head.child(kit::chip_tone(
                "generated files out of date",
                Tone::Warn,
                &t,
            ));
        }
        let mut col = body.child(head);
        if !st.existing.is_empty() {
            col = col.child(
                banner(&t)
                    .child(div().flex_1().min_w_0().child(format!(
                        "{} is hand-written. Import it into .rulesync/ first, so generating keeps its text.",
                        st.existing.join(", ")
                    )))
                    .child(
                        kit::text_button("proj-import", "Import", BtnKind::Small, self.busy, &t)
                            .on_click(cx.listener(|p, _, _, cx| p.import_existing(cx))),
                    ),
            );
        }
        if let Some(id) = &st.fallback_set {
            let name = self
                .sets
                .iter()
                .find(|s| &s.id == id)
                .map_or_else(|| id.clone(), |s| s.name.clone());
            col = col.child(
                banner(&t)
                    .child(div().flex_1().min_w_0().child(format!(
                        "The default set \"{name}\" is kept in Pitwall. Move it into the project to commit it with the code."
                    )))
                    .child(
                        kit::text_button("proj-move", "Move into project", BtnKind::Small, self.busy, &t)
                            .on_click(cx.listener(|p, _, _, cx| p.move_default(cx))),
                    ),
            );
        }
        col = col.child(self.picker(&st, &t, cx)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    kit::text_button(
                        "proj-apply",
                        if n == 0 {
                            "Apply to project".to_string()
                        } else {
                            format!("Apply to project ({n})")
                        },
                        BtnKind::Small,
                        n == 0 || self.busy,
                        &t,
                    )
                    .on_click(cx.listener(|p, _, _, cx| p.apply(cx))),
                )
                .when(st.rulesync, |d| {
                    d.child(
                        kit::text_button(
                            "proj-generate",
                            "Generate files",
                            BtnKind::Ghost,
                            self.busy,
                            &t,
                        )
                        .tooltip(kit::tooltip(format!(
                            "Run rulesync for {} and write CLAUDE.md, AGENTS.md… into the project",
                            if g.targets.is_empty() {
                                "the project's agents".to_string()
                            } else {
                                g.targets.join(", ")
                            }
                        )))
                        .on_click(cx.listener(|p, _, _, cx| p.generate(cx))),
                    )
                })
                .when(self.busy, |d| d.child(kit::hint("Working…", &t))),
        );
        col.children(self.confirm_box(&t, cx))
            .children(self.updates_list(&t, cx))
            .when_some(self.msg.clone(), |d, (ok, m)| {
                d.child(if ok {
                    kit::hint(m, &t)
                } else {
                    kit::error_text(m, &t)
                })
            })
    }
}
