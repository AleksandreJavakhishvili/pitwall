//! Settings → Rules (`src/components/rules/RulesSettings.tsx`): rulesync's
//! status and the npx opt-in, then "Manage rules…": the library (with
//! import and sources), rule sets with their editor, and project defaults.
//!
//! Drawn through `settings::ExtraSections`: [`section`] keeps one
//! [`RulesSection`] alive while Settings shows it (a fresh one, collapsed,
//! each time Settings opens, as the React component remounts).

use crate::kit::Ellipsis as _;
use std::collections::{BTreeMap, BTreeSet};

use gpui::{
    div, prelude::*, px, AnyElement, App, Context, Entity, Focusable, PromptLevel, SharedString,
    Window,
};

use pitwall_core::rules::{RuleFile, RuleSet, RuleSource, RulesStatus, SaveRuleSet};

use crate::kit::{self, BtnKind, InputEvent, TextInput, Tone};
use crate::theme::{appearance, Theme, RADIUS_SM};

use super::widgets::{self, check_row, list, mini, rich, row, Span};
use super::{call, rule_label};

/// Where imported rules come from (`ImportKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    File,
    Project,
    Git,
}

impl ImportKind {
    pub const ALL: [ImportKind; 3] = [ImportKind::File, ImportKind::Project, ImportKind::Git];

    pub fn id(self) -> &'static str {
        match self {
            ImportKind::File => "file",
            ImportKind::Project => "project",
            ImportKind::Git => "git",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ImportKind::File => "CLAUDE.md / AGENTS.md",
            ImportKind::Project => "Project's .rulesync",
            ImportKind::Git => "Git repository",
        }
    }

    /// The source field's placeholder (`IMPORT_HINT`).
    pub fn placeholder(self) -> &'static str {
        match self {
            ImportKind::File => "/path/to/project/CLAUDE.md",
            ImportKind::Project => "/path/to/project (with .rulesync/)",
            ImportKind::Git => "https://github.com/team/ai-rules.git",
        }
    }

    pub fn hint(self, data_dir: &str) -> String {
        match self {
            ImportKind::File => {
                "Runs rulesync import on a copy; your project is not touched.".into()
            }
            ImportKind::Project => {
                "Read in place from the project's .rulesync/ (read-only).".into()
            }
            ImportKind::Git => format!("Cloned into {data_dir}/rules-sources; pull to update."),
        }
    }
}

/// "Imported 2 rules.\n<log>".
pub fn import_message(added: usize, log: &str) -> String {
    let head = format!(
        "Imported {added} rule{}.",
        if added == 1 { "" } else { "s" }
    );
    if log.is_empty() {
        head
    } else {
        format!("{head}\n{log}")
    }
}

/// The open dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Open {
    Kind,
    Project(String),
}

/// The set being edited.
#[derive(Debug, Clone, PartialEq)]
enum Editing {
    New,
    Set(RuleSet),
}

pub struct RulesSection {
    status: Option<RulesStatus>,
    checked: bool,
    open: bool,
    lib: Vec<RuleFile>,
    sets: Vec<RuleSet>,
    sources: Vec<RuleSource>,
    data_dir: String,
    editing: Option<Editing>,
    set_name: Entity<TextInput>,
    picked: Vec<String>,
    kind: ImportKind,
    source: Entity<TextInput>,
    importing: bool,
    import_msg: Option<(bool, String)>,
    /// Project defaults: (path, display), at most 40 shown.
    projects: Vec<(String, String)>,
    defaults: BTreeMap<String, String>,
    select: Option<Open>,
    error: Option<String>,
    _subs: Vec<gpui::Subscription>,
}

/// The Settings section (an `ExtraSections` entry).
pub fn section(window: &mut Window, cx: &mut App) -> AnyElement {
    let view = window.use_keyed_state("rules-settings", cx, |_, cx| RulesSection::new(cx));
    view.into_any_element()
}

impl RulesSection {
    pub fn new(cx: &mut Context<Self>) -> RulesSection {
        let set_name = cx.new(|cx| TextInput::new(cx, "", "Set name"));
        let source = cx.new(|cx| TextInput::new(cx, "", ImportKind::File.placeholder()).mono());
        let subs = vec![
            cx.subscribe(&source, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Submit => this.import(cx),
                InputEvent::Changed => cx.notify(),
                _ => {}
            }),
            cx.subscribe(&set_name, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Submit => this.save_set(cx),
                InputEvent::Cancel => {
                    this.editing = None;
                    cx.notify();
                }
                InputEvent::Changed => cx.notify(),
                _ => {}
            }),
        ];
        let mut this = RulesSection {
            status: None,
            checked: false,
            open: false,
            lib: vec![],
            sets: vec![],
            sources: vec![],
            data_dir: "~/.pitwall".into(),
            editing: None,
            set_name,
            picked: vec![],
            kind: ImportKind::File,
            source,
            importing: false,
            import_msg: None,
            projects: vec![],
            defaults: BTreeMap::new(),
            select: None,
            error: None,
            _subs: subs,
        };
        call(
            cx,
            |b| (b.status(), b.data_dir()),
            |v: &mut Self, (s, dir), cx| {
                v.status = Some(s);
                v.checked = true;
                v.data_dir = dir;
                cx.notify();
            },
        );
        // Debug builds: `PITWALL_DEBUG_RULES=open` opens the manager
        // (screenshots without clicks).
        if cfg!(debug_assertions) && std::env::var("PITWALL_DEBUG_RULES").as_deref() == Ok("open") {
            this.toggle(cx);
        }
        this
    }

    fn fail(&mut self, what: &str, e: String, cx: &mut Context<Self>) {
        self.error = Some(format!("Couldn't {what}: {e}"));
        cx.notify();
    }

    fn set_npx(&mut self, on: bool, cx: &mut Context<Self>) {
        call(
            cx,
            move |b| b.set_npx(on),
            |v: &mut Self, r, cx| match r {
                Ok(s) => {
                    v.status = Some(s);
                    cx.notify();
                }
                Err(e) => v.fail("change the npx setting", e, cx),
            },
        );
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        if self.open {
            self.reload(cx);
            self.load_projects(cx);
        } else {
            self.editing = None;
            self.select = None;
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        call(
            cx,
            |b| (b.library(), b.sets(), b.sources()),
            |v: &mut Self, (l, s, src), cx| {
                v.lib = l;
                v.sets = s;
                v.sources = src;
                cx.notify();
            },
        );
    }

    /// Something changed: reload, and re-check every agent's rules.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
        super::stale::refresh(cx);
    }

    fn load_projects(&mut self, cx: &mut Context<Self>) {
        call(
            cx,
            |b| (b.projects(), b.project_rules()),
            |v: &mut Self, (projects, d), cx| {
                v.projects = project_rows(projects, &d);
                v.defaults = d;
                cx.notify();
            },
        );
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        call(
            cx,
            |b| b.library_dir(),
            |v: &mut Self, r, cx| match r {
                Ok(dir) => cx.open_with_system(&dir),
                Err(e) => v.fail("open the rules folder", e, cx),
            },
        );
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let source = self.source.read(cx).text().trim().to_string();
        if source.is_empty() || self.importing || !self.can_import() {
            return;
        }
        self.importing = true;
        self.import_msg = None;
        let kind = self.kind.id();
        cx.notify();
        call(
            cx,
            move |b| b.import(kind, &source),
            |v: &mut Self, r, cx| {
                v.importing = false;
                match r {
                    Ok(r) => {
                        v.import_msg = Some((true, import_message(r.added.len(), &r.log)));
                        v.source.update(cx, |i, cx| i.set_text("", cx));
                        v.changed(cx);
                    }
                    Err(e) => v.import_msg = Some((false, e)),
                }
                cx.notify();
            },
        );
    }

    fn can_import(&self) -> bool {
        self.kind != ImportKind::File || self.status.as_ref().is_some_and(|s| s.available)
    }

    fn pull(&mut self, name: String, cx: &mut Context<Self>) {
        call(
            cx,
            move |b| b.pull_source(&name),
            |v: &mut Self, r, cx| match r {
                Ok(_) => v.changed(cx),
                Err(e) => v.fail("pull rules", e, cx),
            },
        );
    }

    fn remove_source(&mut self, name: String, cx: &mut Context<Self>) {
        call(
            cx,
            move |b| b.remove_source(&name),
            |v: &mut Self, r, cx| match r {
                Ok(()) => v.changed(cx),
                Err(e) => v.fail("remove source", e, cx),
            },
        );
    }

    fn edit(&mut self, e: Editing, window: &mut Window, cx: &mut Context<Self>) {
        let (name, picked) = match &e {
            Editing::New => (String::new(), vec![]),
            Editing::Set(s) => (s.name.clone(), s.rule_ids.clone()),
        };
        self.set_name.update(cx, |i, cx| i.set_text(&name, cx));
        window.focus(&self.set_name.focus_handle(cx));
        self.picked = picked;
        self.editing = Some(e);
        cx.notify();
    }

    fn pick(&mut self, id: &str, cx: &mut Context<Self>) {
        toggle_pick(&mut self.picked, id);
        cx.notify();
    }

    fn save_set(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = &self.editing else {
            return;
        };
        let name = self.set_name.read(cx).text().to_string();
        if name.trim().is_empty() {
            return;
        }
        let set = SaveRuleSet {
            id: match editing {
                Editing::Set(s) => Some(s.id.clone()),
                Editing::New => None,
            },
            name,
            rule_ids: self.picked.clone(),
        };
        call(
            cx,
            move |b| b.save_set(set),
            |v: &mut Self, r, cx| match r {
                Ok(_) => {
                    v.editing = None;
                    v.error = None;
                    v.changed(cx);
                }
                Err(e) => v.fail("save rule set", e, cx),
            },
        );
    }

    fn delete_set(&mut self, set: RuleSet, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete rule set \"{}\"?", set.name),
            Some("Agents keep the files they have until rules are re-applied."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |v, cx| {
                let id = set.id.clone();
                call(
                    cx,
                    move |b| b.delete_set(&id),
                    |v: &mut Self, r, cx| match r {
                        Ok(()) => {
                            v.changed(cx);
                            v.load_projects(cx);
                        }
                        Err(e) => v.fail("delete rule set", e, cx),
                    },
                );
                v.error = None;
            });
        })
        .detach();
    }

    fn set_default(&mut self, path: String, id: Option<String>, cx: &mut Context<Self>) {
        self.select = None;
        let p = path.clone();
        let chosen = id.clone();
        call(
            cx,
            move |b| b.set_project_rules(&p, chosen),
            move |v: &mut Self, r, cx| match r {
                Ok(()) => {
                    match id {
                        Some(id) => {
                            v.defaults.insert(path, id);
                        }
                        None => {
                            v.defaults.remove(&path);
                        }
                    }
                    super::stale::refresh(cx);
                    cx.notify();
                }
                Err(e) => v.fail("set project rules", e, cx),
            },
        );
        cx.notify();
    }

    fn toggle_select(&mut self, which: Open, cx: &mut Context<Self>) {
        self.select = if self.select.as_ref() == Some(&which) {
            None
        } else {
            Some(which)
        };
        cx.notify();
    }
}

/// Listed and recent projects (first wins), then paths that only have a
/// default; at most 40.
pub fn project_rows(
    projects: Vec<(String, String)>,
    defaults: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (path, display) in projects {
        if seen.insert(path.clone()) {
            out.push((path, display));
        }
    }
    for path in defaults.keys() {
        if seen.insert(path.clone()) {
            out.push((path.clone(), path.clone()));
        }
    }
    out.truncate(40);
    out
}

/// Tick or untick a rule in the editor (order kept).
pub fn toggle_pick(picked: &mut Vec<String>, id: &str) {
    if let Some(i) = picked.iter().position(|x| x == id) {
        picked.remove(i);
    } else {
        picked.push(id.to_string());
    }
}

// ───────────────────────────────────────────── drawing

fn field_label(text: &str, t: &Theme) -> gpui::Div {
    kit::label(text.to_string(), t.text_3, 12.)
}

impl RulesSection {
    fn head(&self, t: &Theme) -> gpui::Div {
        let chip = match &self.status {
            None if !self.checked => kit::hint("checking…", t),
            None => kit::chip_tone("not installed", Tone::Subtle, t),
            Some(s) => match s.via {
                Some("rulesync") => kit::chip_tone(
                    format!("rulesync {}", s.version.clone().unwrap_or_default())
                        .trim()
                        .to_string(),
                    Tone::Ok,
                    t,
                ),
                Some("npx") => kit::chip_tone("npx rulesync", Tone::Ok, t),
                _ => kit::chip_tone("not installed", Tone::Subtle, t),
            },
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(kit::label("Rules", t.text_3, 12.))
            .child(kit::hint("via rulesync", t))
            .child(div().flex_1())
            .child(chip)
    }

    fn npx_box(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let s = self.status.as_ref()?;
        if s.via == Some("rulesync") {
            return None;
        }
        let allowed = s.npx_allowed;
        let disabled = !s.npx_found && !s.npx_allowed;
        let hint: SharedString = if s.npx_found {
            "npx downloads rulesync on first use.".into()
        } else {
            "npx isn't on your shell PATH.".into()
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .px(px(14.))
                .py(px(12.))
                .rounded(crate::theme::RADIUS)
                .bg(t.surface_2)
                .border_1()
                .border_color(t.line_strong)
                .child(rich(
                    &[
                        ("Install rulesync yourself: ", Span::Text),
                        ("npm install -g rulesync", Span::Mono),
                        (" — Pitwall never installs it.", Span::Text),
                    ],
                    t.text,
                ))
                .child(check_row(
                    "rules-npx",
                    allowed,
                    disabled,
                    rich(
                        &[
                            ("Use ", Span::Text),
                            ("npx -y rulesync", Span::Mono),
                            (" when it isn't installed", Span::Text),
                        ],
                        t.text,
                    ),
                    Some(hint),
                    t,
                    cx.listener(move |v, _, _, cx| v.set_npx(!allowed, cx)),
                )),
        )
    }

    fn library(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(field_label("Library", t))
            .child(
                div()
                    .font_family(kit::MONO_FONT)
                    .text_size(px(11.))
                    .text_color(t.text_3)
                    .child(self.lib.len().to_string()),
            )
            .child(div().flex_1())
            .child(
                mini("rules-reveal", "Open folder", false, t)
                    .on_click(cx.listener(|v, _, _, cx| v.reveal(cx))),
            )
            .child(
                mini("rules-refresh", "Refresh", false, t)
                    .on_click(cx.listener(|v, _, _, cx| v.changed(cx))),
            );
        let body = if self.lib.is_empty() {
            div()
                .text_size(px(12.))
                .text_color(t.text_3)
                .child(rich(
                    &[
                        ("Add rulesync rule files to ", Span::Text),
                        (&format!("{}/rules/rules/", self.data_dir), Span::Mono),
                        (", or import below.", Span::Text),
                    ],
                    t.text_3,
                ))
                .into_any_element()
        } else {
            list(
                "rules-lib",
                self.lib
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        row(("rules-lib-row", i))
                            .tooltip(kit::tooltip(r.path.clone()))
                            .child(widgets::name(rule_label(&r.id).to_string(), true, t))
                            .when(r.source != "library", |d| {
                                d.child(kit::chip_subtle(r.source.clone(), t))
                            })
                            .when(r.root, |d| {
                                d.child(
                                    div()
                                        .id(("rules-root", i))
                                        .tooltip(kit::tooltip(
                                            "root: the agent's main instruction file",
                                        ))
                                        .child(kit::chip("root", t)),
                                )
                            })
                            .when(r.local_root, |d| d.child(kit::chip("local", t)))
                            .child(widgets::desc(r.description.clone().unwrap_or_default(), t))
                            .into_any_element()
                    })
                    .collect(),
                t,
            )
            .into_any_element()
        };
        let mut sec = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(head)
            .child(body)
            .child(self.import_form(t, cx));
        if !self.sources.is_empty() {
            sec = sec.child(list(
                "rules-sources",
                self.sources
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let (pull, rm) = (s.name.clone(), s.name.clone());
                        let tip = if s.kind == "git" {
                            "Forget and delete Pitwall's clone"
                        } else {
                            "Forget (the project is not touched)"
                        };
                        row(("rules-src", i))
                            .tooltip(kit::tooltip(s.root.clone()))
                            .child(widgets::name(s.name.clone(), true, t))
                            .child(kit::chip_subtle(s.kind.clone(), t))
                            .child(widgets::desc(s.origin.clone(), t).font_family(kit::MONO_FONT))
                            .when(s.kind == "git", |d| {
                                d.child(mini(("rules-pull", i), "Pull", false, t).on_click(
                                    cx.listener(move |v, _, _, cx| v.pull(pull.clone(), cx)),
                                ))
                            })
                            .child(
                                mini(("rules-rm", i), "Remove", false, t)
                                    .tooltip(kit::tooltip(tip))
                                    .on_click(cx.listener(move |v, _, _, cx| {
                                        v.remove_source(rm.clone(), cx)
                                    })),
                            )
                            .into_any_element()
                    })
                    .collect(),
                t,
            ));
        }
        sec
    }

    fn import_form(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let source = self.source.read(cx).text().trim().to_string();
        let disabled = source.is_empty() || self.importing || !self.can_import();
        let open = self.select == Some(Open::Kind);
        let kind = widgets::select(
            "rules-kind",
            self.kind.label(),
            28.,
            12.5,
            false,
            open,
            widgets::items(
                ImportKind::ALL
                    .iter()
                    .map(|k| (k.label().to_string(), *k == self.kind)),
            ),
            t,
            {
                let e = cx.entity().downgrade();
                move |_, cx| {
                    let _ = e.update(cx, |v, cx| v.toggle_select(Open::Kind, cx));
                }
            },
            {
                let e = cx.entity().downgrade();
                widgets::on_pick(move |i, _, cx| {
                    let _ = e.update(cx, |v, cx| {
                        v.kind = ImportKind::ALL[i];
                        v.select = None;
                        let ph = v.kind.placeholder();
                        v.source.update(cx, |s, cx| {
                            s.set_placeholder(ph);
                            cx.notify();
                        });
                        cx.notify();
                    });
                })
            },
            {
                let e = cx.entity().downgrade();
                widgets::on_close(move |_, cx| {
                    let _ = e.update(cx, |v, cx| {
                        v.select = None;
                        cx.notify();
                    });
                })
            },
        );
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(kind)
                    .child(div().flex_1().min_w_0().child(self.source.clone()))
                    .child(
                        kit::text_button(
                            "rules-import",
                            if self.importing {
                                "Importing…"
                            } else {
                                "Import"
                            },
                            BtnKind::Small,
                            disabled,
                            t,
                        )
                        .on_click(cx.listener(|v, _, _, cx| v.import(cx))),
                    ),
            )
            .child(kit::hint(self.kind.hint(&self.data_dir), t))
            .when_some(self.import_msg.clone(), |d, (ok, text)| {
                let lines: Vec<_> = text.lines().map(|l| div().child(l.to_string())).collect();
                d.child(
                    kit::scroll_area("rules-log-area", t, move |h| {
                        div()
                            .id("rules-log")
                            .track_scroll(h)
                            .max_h(px(140.))
                            .overflow_y_scroll()
                            .px(px(10.))
                            .py(px(8.))
                            .children(lines)
                            .into_any_element()
                    })
                    .max_h(px(140.))
                    .rounded(RADIUS_SM)
                    .bg(if ok { t.surface_3 } else { t.red_soft })
                    .text_color(if ok { t.text_2 } else { t.red })
                    .font_family(kit::MONO_FONT)
                    .text_size(px(11.5)),
                )
            })
    }

    fn sets_section(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(field_label("Rule sets", t))
            .child(div().flex_1())
            .when(self.editing.is_none(), |d| {
                d.child(
                    mini("rules-new-set", "New set", self.lib.is_empty(), t).when(
                        !self.lib.is_empty(),
                        |b| {
                            b.on_click(
                                cx.listener(|v, _, window, cx| v.edit(Editing::New, window, cx)),
                            )
                        },
                    ),
                )
            });
        let body = if self.editing.is_some() {
            self.editor(t, cx).into_any_element()
        } else if self.sets.is_empty() {
            kit::hint(
                "A rule set is a named pick of library rules, e.g. \"Web defaults\".",
                t,
            )
            .into_any_element()
        } else {
            list(
                "rules-sets",
                self.sets
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let (edit, del) = (s.clone(), s.clone());
                        let names: Vec<&str> = s.rule_ids.iter().map(|r| rule_label(r)).collect();
                        let names = if names.is_empty() {
                            "empty".to_string()
                        } else {
                            names.join(", ")
                        };
                        row(("rules-set", i))
                            .child(widgets::name(s.name.clone(), false, t))
                            .child(widgets::desc(names, t))
                            .child(
                                mini(("rules-edit", i), "Edit", false, t).on_click(cx.listener(
                                    move |v, _, window, cx| {
                                        v.edit(Editing::Set(edit.clone()), window, cx)
                                    },
                                )),
                            )
                            .child(mini(("rules-del", i), "Delete", false, t).on_click(
                                cx.listener(move |v, _, window, cx| {
                                    v.delete_set(del.clone(), window, cx)
                                }),
                            ))
                            .into_any_element()
                    })
                    .collect(),
                t,
            )
            .into_any_element()
        };
        div().flex().flex_col().gap(px(8.)).child(head).child(body)
    }

    fn editor(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let missing: Vec<String> = self
            .picked
            .iter()
            .filter(|id| !self.lib.iter().any(|r| &r.id == *id))
            .cloned()
            .collect();
        let mut picks: Vec<AnyElement> = self
            .lib
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let id = r.id.clone();
                let mut title = div().flex().flex_wrap().child(
                    div()
                        .font_family(kit::MONO_FONT)
                        .text_size(px(12.))
                        .child(rule_label(&r.id).to_string()),
                );
                if r.source != "library" {
                    title = title.child(kit::hint(format!("\u{a0}· {}", r.source), t));
                }
                if r.root {
                    title = title.child(kit::hint("\u{a0}· root", t));
                }
                check_row(
                    ("rules-pick", i),
                    self.picked.contains(&r.id),
                    false,
                    title,
                    r.description.clone().map(Into::into),
                    t,
                    cx.listener(move |v, _, _, cx| v.pick(&id, cx)),
                )
                .into_any_element()
            })
            .collect();
        for (i, id) in missing.into_iter().enumerate() {
            let label = format!("{} (missing)", rule_label(&id));
            picks.push(
                check_row(
                    ("rules-missing", i),
                    true,
                    false,
                    div()
                        .font_family(kit::MONO_FONT)
                        .text_size(px(12.))
                        .text_color(t.red)
                        .child(label),
                    None,
                    t,
                    cx.listener(move |v, _, _, cx| v.pick(&id, cx)),
                )
                .into_any_element(),
            );
        }
        let can_save = !self.set_name.read(cx).text().trim().is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(self.set_name.clone())
            .child(
                kit::scroll_area("rules-pick-area", t, move |h| {
                    div()
                        .id("rules-pick")
                        .track_scroll(h)
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .px(px(10.))
                        .py(px(8.))
                        .max_h(px(238.))
                        .overflow_y_scroll()
                        .children(picks)
                        .into_any_element()
                })
                .max_h(px(240.))
                .rounded(crate::theme::RADIUS)
                .border_1()
                .border_color(t.line)
                .bg(t.surface_2)
                .text_size(px(12.5)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        kit::text_button("rules-save", "Save", BtnKind::Primary, !can_save, t)
                            .on_click(cx.listener(|v, _, _, cx| v.save_set(cx))),
                    )
                    .child(
                        kit::text_button("rules-cancel", "Cancel", BtnKind::Ghost, false, t)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.editing = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn defaults_section(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if self.sets.is_empty() || self.projects.is_empty() {
            return None;
        }
        let rows = self
            .projects
            .iter()
            .enumerate()
            .map(|(i, (path, display))| {
                let current = self.defaults.get(path);
                let current_name = current
                    .and_then(|id| self.sets.iter().find(|s| &s.id == id))
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "None".into());
                let mut items = vec![("None".to_string(), current.is_none())];
                items.extend(
                    self.sets
                        .iter()
                        .map(|s| (s.name.clone(), current == Some(&s.id))),
                );
                let ids: Vec<Option<String>> = std::iter::once(None)
                    .chain(self.sets.iter().map(|s| Some(s.id.clone())))
                    .collect();
                let which = Open::Project(path.clone());
                let open = self.select.as_ref() == Some(&which);
                let e = cx.entity().downgrade();
                let e2 = e.clone();
                let e3 = e.clone();
                let p = path.clone();
                row(("rules-proj", i))
                    .tooltip(kit::tooltip(path.clone()))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(260.))
                            .ellipsis()
                            .text_size(px(12.))
                            .text_color(t.text)
                            .child(crate::kit::one_line(display.clone())),
                    )
                    .child(div().flex_1())
                    .child(widgets::select(
                        ("rules-proj-set", i),
                        current_name,
                        28.,
                        12.5,
                        false,
                        open,
                        widgets::items(items),
                        t,
                        move |_, cx| {
                            let _ = e.update(cx, |v, cx| v.toggle_select(which.clone(), cx));
                        },
                        widgets::on_pick(move |k, _, cx| {
                            let id = ids[k].clone();
                            let p = p.clone();
                            let _ = e2.update(cx, |v, cx| v.set_default(p, id, cx));
                        }),
                        widgets::on_close(move |_, cx| {
                            let _ = e3.update(cx, |v, cx| {
                                v.select = None;
                                cx.notify();
                            });
                        }),
                    ))
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(field_label("Project defaults", t)),
                )
                .child(kit::hint(
                    "Every new agent in the project gets this set. Rules reach new sessions only.",
                    t,
                ))
                .child(list("rules-defaults", rows, t)),
        )
    }
}

impl Render for RulesSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = appearance(cx).theme().float();
        let mut sec = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(self.head(&t))
            .child(kit::muted(
                "Keep instructions in one library and give agents rule sets. Pitwall calls rulesync to generate each agent's files (CLAUDE.local.md, .claude/rules, AGENTS.md…) when it starts; they stay out of git.",
                &t,
            ))
            .children(self.npx_box(&t, cx))
            .child(
                div().flex().child(
                    kit::text_button(
                        "rules-manage",
                        if self.open { "Hide rules" } else { "Manage rules…" },
                        BtnKind::Small,
                        false,
                        &t,
                    )
                    .on_click(cx.listener(|v, _, _, cx| v.toggle(cx))),
                ),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(kit::error_text(e, &t).flex_1())
                        .child(
                            kit::text_button("rules-err-x", "Dismiss", BtnKind::Link, false, &t)
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.error = None;
                                    cx.notify();
                                })),
                        ),
                )
            });
        if self.open {
            sec = sec.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .pt(px(4.))
                    .child(self.library(&t, cx))
                    .child(self.sets_section(&t, cx))
                    .children(self.defaults_section(&t, cx)),
            );
        }
        sec
    }
}
