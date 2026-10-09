//! The main screen's dialogs (React `Modal.tsx`, `NewAgentDialog.tsx`,
//! `CreateFormFields.tsx`, `TerminalDialog.tsx`, `RemoveDialog.tsx`,
//! `DiffView.tsx`): one modal at a time over a backdrop; Esc or a click on
//! the backdrop closes; Tab moves between fields; ⏎ submits.

use crate::kit::Ellipsis as _;
use crate::kit::Span;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::kit::HoverText;
use gpui::{
    anchored, deferred, div, prelude::*, px, AnyElement, App, Context,
    Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    PathPromptOptions, SharedString, Subscription, Task, Window,
};

use pitwall_core::model::{CreateAgentRequest, KindView};
use pitwall_core::vcs::git::FileChange;
use pitwall_core::Shared;
use pitwall_proto::{AgentView, CreateForm, FieldInput, ProviderMachines};

use crate::kit::{icon, MONO_FONT};
use super::model::{name_problem, split_path, suggest_name};
use crate::kit::{InputEvent, TextInput};
use crate::kit::{
    button, check, chip_subtle, diffstat, icon_btn, kbd_on_primary, label, small_btn, BtnKind,
};
use super::MainScreen;
use crate::theme::{self, Theme, RADIUS};

/// A choice action in a select.
type Run = Box<dyn Fn(&mut Window, &mut App)>;
/// A select option: label, chosen, action.
type Choice = (SharedString, bool, Run);

#[derive(Debug, Clone)]
pub enum DialogEvent {
    Close,
    Created(Box<AgentView>),
    OpenTerminal(String),
}

/// The modal frame: backdrop, box, title with ✕ (`Modal.tsx`).
fn frame(
    title: Option<&str>,
    width: f32,
    t: &Theme,
    motion: bool,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    body: impl IntoElement,
) -> AnyElement {
    let m = crate::kit::Modal::new("modal", width, on_close).motion(motion);
    match title {
        Some(title) => m.title(title.to_string()),
        None => m,
    }
    .render(t, body)
}

fn field_label(text: &str, t: &Theme) -> impl IntoElement {
    label(text.to_string(), t.text_3, 12.)
}

fn hint(text: impl Into<SharedString>, t: &Theme) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(text.into())
}

fn field_error(text: impl Into<SharedString>, t: &Theme) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(t.red)
        .child(text.into())
}

use crate::kit::dialog::{body, foot, form_error};

/// A select (`select.input`): the current choice, a list below on click.
fn select_box(
    id: &str,
    current: SharedString,
    open: bool,
    t: &Theme,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    options: Vec<Choice>,
) -> impl IntoElement {
    div()
        .relative()
        .child(
            div()
                .id(SharedString::from(id.to_string()))
                .flex()
                .items_center()
                .h(px(34.))
                .pl(px(9.))
                .pr(px(10.))
                .rounded(RADIUS)
                .bg(t.bg)
                .border_1()
                .border_color(if open { t.text_3 } else { t.line_strong })
                .cursor_pointer()
                .on_click(move |_, window, cx| on_toggle(window, cx))
                .child(div().flex_1().min_w_0().ellipsis().child(crate::kit::one_line(current)))
                .child(icon("chevron", 12., t.text_3).with_transformation(
                    gpui::Transformation::rotate(gpui::radians(std::f32::consts::FRAC_PI_2)),
                )),
        )
        .when(open, |d| {
            d.child(deferred(
                anchored().snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id(SharedString::from(format!("{id}-list")))
                        .mt(px(38.))
                        .min_w(px(300.))
                        .max_h(px(260.))
                        .overflow_y_scroll()
                        .p(px(4.))
                        .rounded(RADIUS)
                        .bg(t.raised)
                        .border_1()
                        .border_color(t.line_strong)
                        .shadow_lg()
                        .children(options.into_iter().enumerate().map(|(i, (text, on, run))| {
                            div()
                                .id(("opt", i))
                                .px(px(8.))
                                .py(px(6.))
                                .rounded(px(4.))
                                .text_size(px(13.))
                                .when(on, |d| d.bg(t.surface_3))
                                .hover_probed(|s| s.bg(t.surface_3))
                                .cursor_pointer()
                                .on_click(move |_, window, cx| run(window, cx))
                                .child(text)
                        })),
                ),
            ))
        })
}

// ── New agent ─────────────────────────────────────────────────────────────

/// A machine new agents can be made on ("Runs on").
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub key: String,
    pub provider: String,
    pub machine: String,
    pub label: String,
}

/// Machines where new agents can start, in the providers' order.
pub fn create_targets(list: &[ProviderMachines]) -> Vec<Target> {
    list.iter()
        .filter(|p| p.can_create)
        .flat_map(|p| {
            p.machines.iter().map(|m| Target {
                key: format!("{}:{}", p.provider, m.id),
                provider: p.provider.clone(),
                machine: m.id.clone(),
                label: if m.label == p.label {
                    m.label.clone()
                } else {
                    format!("{} · {}", p.label, m.label)
                },
            })
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Open {
    Target,
    Project,
    Field(usize),
}

pub struct NewAgentDialog {
    engine: Option<Shared>,
    taken: Vec<String>,
    kinds: Option<Vec<KindView>>,
    /// Recent conversations, then listed projects: (path, display).
    recents: Vec<(String, String)>,
    kind: String,
    custom: Entity<TextInput>,
    /// `None`: "Choose folder…" (the typed path).
    project: Option<String>,
    other: Entity<TextInput>,
    name: Entity<TextInput>,
    name_touched: bool,
    worktree: bool,
    targets: Vec<Target>,
    target: String,
    form: Option<CreateForm>,
    form_loading: bool,
    form_error: Option<String>,
    chosen: BTreeMap<String, String>,
    texts: BTreeMap<String, Entity<TextInput>>,
    open: Option<Open>,
    busy: bool,
    error: Option<String>,
    size: (Option<u16>, Option<u16>),
    focus: FocusHandle,
    rules: Entity<crate::rules::field::RulesField>,
    _load: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<DialogEvent> for NewAgentDialog {}

impl NewAgentDialog {
    pub fn new(
        engine: Option<Shared>,
        existing: Vec<AgentView>,
        project: Option<String>,
        size: (Option<u16>, Option<u16>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> NewAgentDialog {
        let name = cx.new(|cx| TextInput::new(cx, "", "").mono().max_len(32));
        let other = cx.new(|cx| {
            TextInput::new(
                cx,
                project.as_deref().unwrap_or(""),
                "/Users/you/code/project",
            )
            .mono()
        });
        let custom = cx.new(|cx| TextInput::new(cx, "", "e.g. aider --model sonnet").mono());
        let mut subs = vec![];
        for input in [&name, &other, &custom] {
            subs.push(
                cx.subscribe_in(input, window, |this, input, e: &InputEvent, window, cx| {
                    this.input_event(input, e, window, cx)
                }),
            );
        }
        window.focus(&name.focus_handle(cx));
        let rules = cx.new(crate::rules::field::RulesField::new);
        subs.push(cx.observe(&rules, |_, _, cx| cx.notify()));
        let mut d = NewAgentDialog {
            engine: engine.clone(),
            taken: existing.into_iter().map(|a| a.name).collect(),
            kinds: None,
            recents: vec![],
            kind: String::new(),
            custom,
            project: None,
            other,
            name,
            name_touched: false,
            worktree: false,
            targets: vec![],
            target: String::new(),
            form: None,
            form_loading: false,
            form_error: None,
            chosen: BTreeMap::new(),
            texts: BTreeMap::new(),
            open: None,
            busy: false,
            error: None,
            size,
            focus: cx.focus_handle(),
            rules,
            _load: vec![],
            _subs: subs,
        };
        let Some(engine) = engine else { return d };
        // Kinds, recent conversations and the project list.
        let e = engine.clone();
        let load = cx.background_executor().spawn(async move {
            let kinds = e.list_kinds().unwrap_or_default();
            let recent = pitwall_core::onboarding::recent::recent_projects(e.paths(), 30);
            let listed = e.projects().list();
            let mut r: Vec<(String, String)> =
                recent.into_iter().map(|p| (p.path, p.display)).collect();
            for p in listed {
                if !r.iter().any(|(path, _)| *path == p.path) {
                    r.push((p.path, p.display));
                }
            }
            (kinds, r)
        });
        let wanted = project.clone();
        d._load.push(cx.spawn_in(window, async move |this, cx| {
            let (kinds, recents) = load.await;
            let _ = this.update_in(cx, |d, window, cx| {
                let first = kinds
                    .iter()
                    .find(|k| k.installed && !k.caps.custom_command)
                    .or(kinds.iter().find(|k| k.installed));
                d.kind = first.map(|k| k.id.clone()).unwrap_or_default();
                d.kinds = Some(kinds);
                match &wanted {
                    Some(p) if recents.iter().any(|(path, _)| path == p) => {
                        d.project = Some(p.clone())
                    }
                    Some(_) => d.project = None,
                    None => d.project = recents.first().map(|r| r.0.clone()),
                }
                d.recents = recents;
                d.suggest(window, cx);
                cx.notify();
            });
        }));
        // Machines ("Runs on").
        let e = engine.clone();
        let machines = cx
            .background_executor()
            .spawn(async move { e.machine_list() });
        d._load.push(cx.spawn_in(window, async move |this, cx| {
            let list = machines.await;
            let _ = this.update_in(cx, |d, window, cx| {
                d.targets = create_targets(&list);
                if let Some(first) = d.targets.first().map(|t| t.key.clone()) {
                    d.choose_target(first, window, cx);
                }
            });
        }));
        d
    }

    /// Demo only: kinds and projects without an engine.
    pub fn demo_fill(
        &mut self,
        kinds: Vec<KindView>,
        recents: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.kind = kinds.first().map(|k| k.id.clone()).unwrap_or_default();
        self.kinds = Some(kinds);
        self.project = recents.first().map(|r| r.0.clone());
        self.recents = recents;
        self.suggest(window, cx);
        cx.notify();
    }

    /// One agent kind (`.kind-card`): installed, not installed or any CLI.
    fn kind_card(&self, k: &KindView, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let on = k.id == self.kind;
        let id = k.id.clone();
        let sub = if k.caps.custom_command {
            "any CLI"
        } else if k.installed {
            "installed"
        } else {
            "not installed"
        };
        let tip: SharedString = if k.installed {
            k.path.clone().unwrap_or_default().into()
        } else {
            format!("{} isn't installed (not found on your shell PATH)", k.name).into()
        };
        let installed = k.installed;
        div()
            .id(SharedString::from(format!("kind-{}", k.id)))
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .px(px(10.))
            .py(px(8.))
            .rounded(RADIUS)
            .border_1()
            .border_color(if on { t.text_2 } else { t.line_strong })
            .when(on, |d| {
                d.bg(t.surface_2).shadow(vec![gpui::BoxShadow {
                    color: t.text_2,
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.5),
                }])
            })
            .when(!installed, |d| d.opacity(0.45))
            .when(installed, |d| {
                d.cursor_pointer().hover_probed(|s| s.bg(t.surface_2))
            })
            .tooltip(crate::kit::tooltip(tip))
            .on_click(cx.listener(move |d, _, _, cx| {
                if installed {
                    d.kind = id.clone();
                    cx.notify();
                }
            }))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(k.name.clone()),
            )
            .child(div().text_size(px(11.5)).text_color(t.text_3).child(sub))
    }

    fn target(&self) -> Option<&Target> {
        self.targets.iter().find(|t| t.key == self.target)
    }

    fn choose_target(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        self.target = key;
        self.open = None;
        let (Some(engine), Some(t)) = (self.engine.clone(), self.target().cloned()) else {
            return;
        };
        self.form_loading = true;
        self.form_error = None;
        self.chosen.clear();
        self.texts.clear();
        let task = cx
            .background_executor()
            .spawn(async move { engine.create_form(Some(&t.provider), Some(&t.machine)) });
        self._load.push(cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update_in(cx, |d, window, cx| {
                d.form_loading = false;
                match res {
                    Ok(f) => {
                        for field in f.fields.iter().filter(|f| f.input == FieldInput::Text) {
                            let input = cx.new(|cx| {
                                TextInput::new(cx, "", field.placeholder.as_deref().unwrap_or(""))
                                    .mono()
                            });
                            d._subs.push(cx.subscribe_in(
                                &input,
                                window,
                                |this, input, e: &InputEvent, window, cx| {
                                    this.input_event(input, e, window, cx)
                                },
                            ));
                            d.texts.insert(field.id.clone(), input);
                        }
                        d.form = Some(f);
                    }
                    Err(e) => {
                        d.form = None;
                        d.form_error = Some(e);
                    }
                }
                d.suggest(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn input_event(
        &mut self,
        input: &Entity<TextInput>,
        e: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match e {
            InputEvent::Submit | InputEvent::SubmitShift => self.submit(window, cx),
            InputEvent::Cancel => cx.emit(DialogEvent::Close),
            InputEvent::Changed => {
                if *input == self.name {
                    self.name_touched = true;
                } else if *input == self.other {
                    self.suggest(window, cx);
                }
                self.error = None;
                cx.notify();
            }
            InputEvent::Blur => {}
        }
    }

    fn folder(&self) -> bool {
        self.form.as_ref().is_none_or(|f| f.folder)
    }

    fn project_path(&self, cx: &App) -> String {
        match &self.project {
            Some(p) => p.clone(),
            None => self.other.read(cx).text().trim().to_string(),
        }
    }

    /// A name from the folder until the user types one.
    fn suggest(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.name_touched || !self.folder() {
            return;
        }
        let path = self.project_path(cx);
        if path.is_empty() {
            return;
        }
        let name = suggest_name(&path, &self.taken);
        self.name.update(cx, |i, cx| i.set_text(&name, cx));
    }

    fn selected_kind(&self) -> Option<&KindView> {
        self.kinds.as_ref()?.iter().find(|k| k.id == self.kind)
    }

    fn values(&self, cx: &App) -> Result<BTreeMap<String, String>, String> {
        let Some(form) = &self.form else {
            return Ok(BTreeMap::new());
        };
        let mut chosen = self.chosen.clone();
        for (id, input) in &self.texts {
            let v = input.read(cx).text().trim().to_string();
            if !v.is_empty() {
                chosen.insert(id.clone(), v);
            }
        }
        // Only fields that apply (a hidden field's value is an error there).
        let mut out = BTreeMap::new();
        for f in &form.fields {
            let shown = f.when.as_ref().is_none_or(|w| {
                out.get(&w.field) == Some(&w.value) || chosen.get(&w.field) == Some(&w.value)
            });
            if shown {
                if let Some(v) = chosen.get(&f.id) {
                    out.insert(f.id.clone(), v.clone());
                }
            }
        }
        form.values(&out)
    }

    /// The first problem that keeps Start disabled.
    fn problems(
        &self,
        cx: &App,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let name = self.name.read(cx).text().to_string();
        let rule = self
            .form
            .as_ref()
            .map(|f| f.name.clone())
            .unwrap_or_else(CreateForm::pitwall_name);
        let name_err = if name.is_empty() {
            Some("Name required".to_string())
        } else if let Some(p) = name_problem(&rule, &name) {
            Some(p)
        } else if self.taken.contains(&name) {
            Some("Another agent already has this name".into())
        } else {
            None
        };
        let folder = self.folder();
        let path = self.project_path(cx);
        let project_err = if !folder {
            None
        } else if path.is_empty() {
            Some("Pick a project folder".to_string())
        } else if !path.starts_with('/') && !path.starts_with('~') && !is_windows_abs(&path) {
            Some("Use an absolute path".into())
        } else {
            None
        };
        let custom = self.selected_kind().is_some_and(|k| k.caps.custom_command);
        let command_err = (folder && custom && self.custom.read(cx).text().trim().is_empty())
            .then(|| "Enter a command".to_string());
        let fields_err = if folder { None } else { self.values(cx).err() };
        (name_err, project_err, command_err, fields_err)
    }

    fn valid(&self, cx: &App) -> bool {
        let (a, b, c, d) = self.problems(cx);
        a.is_none()
            && b.is_none()
            && c.is_none()
            && d.is_none()
            && !self.form_loading
            && self.form_error.is_none()
            && (!self.folder() || !self.kind.is_empty())
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.valid(cx) {
            return;
        }
        let Some(engine) = self.engine.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let target = self.target().cloned();
        let name = self.name.read(cx).text().to_string();
        let mut req = CreateAgentRequest {
            name,
            provider: target.as_ref().map(|t| t.provider.clone()),
            machine: target.as_ref().map(|t| t.machine.clone()),
            cols: self.size.0,
            rows: self.size.1,
            ..Default::default()
        };
        if self.folder() {
            let custom = self.selected_kind().is_some_and(|k| k.caps.custom_command);
            let worktree = self.selected_kind().is_some_and(|k| k.caps.worktree) && self.worktree;
            req.kind = self.kind.clone();
            req.project_path = self.project_path(cx);
            req.worktree = worktree;
            if custom {
                req.custom_command = Some(self.custom.read(cx).text().to_string());
            }
            (req.rule_set_id, req.apply_to_main_checkout) = self.rules.read(cx).request(worktree);
        } else {
            req.options = self.values(cx).unwrap_or_default();
        }
        let task = cx.background_executor().spawn(async move {
            let view = pitwall_core::engine::lifecycle::create(&engine, req)?;
            pitwall_core::onboarding::add_agent_project(&engine, &view);
            Ok::<_, String>(view)
        });
        self._load.push(cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |d, cx| match res {
                Ok(view) => cx.emit(DialogEvent::Created(Box::new(view))),
                Err(e) => {
                    d.busy = false;
                    d.error = Some(e);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a project folder".into()),
        });
        self._load.push(cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |d, window, cx| {
                        let text = p.to_string_lossy().to_string();
                        d.project = None;
                        d.other.update(cx, |i, cx| i.set_text(&text, cx));
                        d.suggest(window, cx);
                        cx.notify();
                    });
                }
            }
        }));
    }
}

fn is_windows_abs(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

impl Render for NewAgentDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let folder = self.folder();
        let (name_err, project_err, _, _) = self.problems(cx);
        let valid = self.valid(cx);
        let entity = cx.entity();
        let toggle = |which: Open| {
            let e = entity.clone();
            move |_: &mut Window, cx: &mut App| {
                e.update(cx, |d, cx| {
                    d.open = if d.open == Some(which) {
                        None
                    } else {
                        Some(which)
                    };
                    cx.notify();
                })
            }
        };

        let mut b = body();
        // Runs on (only with 2+ machines).
        if self.targets.len() > 1 {
            let current: SharedString = self
                .target()
                .map(|t| t.label.clone())
                .unwrap_or_default()
                .into();
            let options = self
                .targets
                .iter()
                .map(|tg| {
                    let (e, key) = (entity.clone(), tg.key.clone());
                    let run: Run = Box::new(move |window, cx| {
                        e.update(cx, |d, cx| d.choose_target(key.clone(), window, cx))
                    });
                    (
                        SharedString::from(tg.label.clone()),
                        tg.key == self.target,
                        run,
                    )
                })
                .collect();
            let label_text = self
                .target()
                .map(|t| t.label.clone())
                .unwrap_or_else(|| "it".into());
            b = b.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(field_label("Runs on", &t))
                    .child(select_box(
                        "na-target",
                        current,
                        self.open == Some(Open::Target),
                        &t,
                        toggle(Open::Target),
                        options,
                    ))
                    .when(self.form_loading, |d| {
                        d.child(hint(format!("Reading what {label_text} has…"), &t))
                    })
                    .when_some(self.form_error.clone(), |d, e| d.child(field_error(e, &t)))
                    .when_some(self.form.as_ref().and_then(|f| f.error.clone()), |d, e| {
                        d.child(hint(format!("Some choices couldn't be listed: {e}"), &t))
                    }),
            );
        }

        if folder {
            // Agent kind cards.
            let kinds = match &self.kinds {
                None => hint("Looking for installed agents…", &t).into_any_element(),
                Some(kinds) => div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(kinds.chunks(3).map(|row| {
                        let pad = 3 - row.len();
                        div()
                            .flex()
                            .gap(px(6.))
                            .children(
                                row.iter()
                                    .map(|k| self.kind_card(k, &t, cx).into_any_element()),
                            )
                            .children((0..pad).map(|_| div().flex_1().into_any_element()))
                    }))
                    .into_any_element(),
            };
            let custom = self.selected_kind().is_some_and(|k| k.caps.custom_command);
            b = b.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(field_label("Agent", &t))
                    .child(kinds)
                    .when(custom, |d| d.child(self.custom.clone())),
            );

            // Project.
            let current: SharedString = match &self.project {
                Some(p) => self
                    .recents
                    .iter()
                    .find(|r| r.0 == *p)
                    .map(|r| r.1.clone())
                    .unwrap_or(p.clone())
                    .into(),
                None => "Choose folder…".into(),
            };
            let mut options: Vec<Choice> = self
                .recents
                .iter()
                .map(|(path, display)| {
                    let (e, p) = (entity.clone(), path.clone());
                    let run: Run = Box::new(move |window, cx| {
                        e.update(cx, |d, cx| {
                            d.project = Some(p.clone());
                            d.open = None;
                            d.suggest(window, cx);
                            cx.notify();
                        })
                    });
                    (
                        SharedString::from(display.clone()),
                        self.project.as_deref() == Some(path),
                        run,
                    )
                })
                .collect();
            let e = entity.clone();
            options.push((
                "Choose folder…".into(),
                self.project.is_none(),
                Box::new(move |window, cx| {
                    e.update(cx, |d, cx| {
                        d.project = None;
                        d.open = None;
                        window.focus(&d.other.focus_handle(cx));
                        d.browse(window, cx);
                        cx.notify();
                    })
                }),
            ));
            let typed = self.other.read(cx).text().to_string();
            b = b.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(field_label("Project", &t))
                    .child(select_box(
                        "na-project",
                        current,
                        self.open == Some(Open::Project),
                        &t,
                        toggle(Open::Project),
                        options,
                    ))
                    .when(self.project.is_none(), |d| {
                        d.child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(div().flex_1().child(self.other.clone()))
                                .child(small_btn("na-browse", "Browse…", &t).h(px(34.)).on_click(
                                    cx.listener(|d, _, window, cx| d.browse(window, cx)),
                                )),
                        )
                        .when(!typed.is_empty(), |d| {
                            d.when_some(project_err.clone(), |d, e| d.child(field_error(e, &t)))
                        })
                    }),
            );
        }

        // Name.
        let name_text = self.name.read(cx).text().to_string();
        b = b.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(field_label("Name", &t))
                .child(self.name.clone())
                .map(|d| match (&name_err, &self.form) {
                    (Some(e), _) if !name_text.is_empty() => d.child(field_error(e.clone(), &t)),
                    (_, Some(f)) if !folder => d.child(hint(
                        format!("Also its name on {}. {}", f.machine_label, f.name.hint),
                        &t,
                    )),
                    _ => d,
                }),
        );

        // A platform's own fields.
        if let Some(form) = self.form.clone().filter(|f| !f.folder) {
            let values = self.values(cx).unwrap_or_default();
            for (i, f) in form.fields.iter().enumerate() {
                let shown = f.when.as_ref().is_none_or(|w| {
                    values.get(&w.field) == Some(&w.value)
                        || self.chosen.get(&w.field) == Some(&w.value)
                });
                if !shown {
                    continue;
                }
                let el: AnyElement = match f.input {
                    FieldInput::Select => {
                        let value = self
                            .chosen
                            .get(&f.id)
                            .cloned()
                            .or(f.default.clone())
                            .unwrap_or_default();
                        let current = f
                            .choices
                            .iter()
                            .find(|c| c.value == value)
                            .map(|c| c.label.clone())
                            .unwrap_or_else(|| format!("Choose {}", f.label));
                        let options = f
                            .choices
                            .iter()
                            .map(|c| {
                                let (e, fid, v) = (entity.clone(), f.id.clone(), c.value.clone());
                                let text = match &c.detail {
                                    Some(d) => format!("{} — {d}", c.label),
                                    None => c.label.clone(),
                                };
                                let run: Run = Box::new(move |_, cx| {
                                    e.update(cx, |d, cx| {
                                        d.chosen.insert(fid.clone(), v.clone());
                                        d.open = None;
                                        cx.notify();
                                    })
                                });
                                (SharedString::from(text), c.value == value, run)
                            })
                            .collect();
                        select_box(
                            &format!("na-f-{}", f.id),
                            current.into(),
                            self.open == Some(Open::Field(i)),
                            &t,
                            toggle(Open::Field(i)),
                            options,
                        )
                        .into_any_element()
                    }
                    FieldInput::Text => match self.texts.get(&f.id) {
                        Some(input) => input.clone().into_any_element(),
                        None => div().into_any_element(),
                    },
                };
                let problem = match (f.input, &f.rule, self.texts.get(&f.id)) {
                    (FieldInput::Text, Some(rule), Some(input)) => {
                        let v = input.read(cx).text();
                        (!v.is_empty()).then(|| name_problem(rule, v)).flatten()
                    }
                    _ => None,
                };
                b = b.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(field_label(&f.label, &t))
                        .child(el)
                        .map(|d| match (problem, &f.hint) {
                            (Some(p), _) => d.child(field_error(p, &t)),
                            (None, Some(h)) => d.child(hint(h.clone(), &t)),
                            _ => d,
                        }),
                );
            }
            let summary = form.summarize(&name_text, &values);
            if name_err.is_none()
                && !name_text.is_empty()
                && (!summary.text.is_empty() || !summary.creates.is_empty())
            {
                b = b.child(
                    div()
                        .px(px(10.))
                        .py(px(8.))
                        .rounded(RADIUS)
                        .border_1()
                        .border_color(t.line_strong)
                        .bg(t.surface_2)
                        .text_size(px(12.5))
                        .child(summary.text)
                        .children(summary.creates.into_iter().map(|c| {
                            div()
                                .mt(px(4.))
                                .px(px(8.))
                                .py(px(3.))
                                .rounded(RADIUS)
                                .bg(t.amber_soft)
                                .border_l_2()
                                .border_color(t.amber)
                                .child(format!("Also creates {c}"))
                        })),
                );
            }
        }

        // Separate worktree.
        if folder && self.selected_kind().is_some_and(|k| k.caps.worktree) {
            let on = self.worktree;
            b = b.child(
                div()
                    .id("na-worktree")
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .cursor_pointer()
                    .on_click(cx.listener(|d, _, _, cx| {
                        d.worktree = !d.worktree;
                        cx.notify();
                    }))
                    .child(check(on, &t))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child("Separate worktree")
                            .child(hint(
                                "Separate copy of the repo, so this agent doesn't clash with others in the same project.",
                                &t,
                            )),
                    ),
            );
        }
        if folder {
            let can = self.selected_kind().is_some_and(|k| k.caps.rules);
            let wt = self.selected_kind().is_some_and(|k| k.caps.worktree) && self.worktree;
            let path = self.project_path(cx);
            self.rules.update(cx, |f, cx| f.sync(can, &path, wt, cx));
            if self.rules.read(cx).visible() {
                b = b.child(self.rules.clone());
            }
        }
        if self.busy && !folder {
            if let Some(f) = &self.form {
                b = b.child(hint(
                    format!(
                        "Creating on {}… a new workspace or agent user can take a few minutes.",
                        f.machine_label
                    ),
                    &t,
                ));
            }
        }
        if let Some(e) = &self.error {
            b = b.child(form_error(e.clone(), &t));
        }

        let submit_label = if self.busy {
            if folder {
                "Starting…".to_string()
            } else {
                "Creating…".to_string()
            }
        } else {
            self.form
                .as_ref()
                .map(|f| f.submit.clone())
                .unwrap_or_else(|| "Start".into())
        };
        let close = entity.clone();
        frame(
            Some("New agent"),
            520.,
            &t,
            crate::theme::motion_on(cx),
            move |_, cx| close.update(cx, |_, cx| cx.emit(DialogEvent::Close)),
            div()
                .track_focus(&self.focus)
                .flex()
                .flex_col()
                .min_h_0()
                .child(
                    crate::kit::scroll_area("na-body-area", &t, move |h| {
                        div()
                            .id("na-body")
                            .track_scroll(h)
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(b)
                            .into_any_element()
                    })
                    .min_h_0()
                    .flex_shrink(),
                )
                .child(
                    foot(&t)
                        .child(
                            button("na-cancel", BtnKind::Ghost, false, &t)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
                                .child("Cancel"),
                        )
                        .child(
                            button("na-start", BtnKind::Primary, !valid || self.busy, &t)
                                .on_click(cx.listener(|d, _, window, cx| d.submit(window, cx)))
                                .child(submit_label)
                                .child(kbd_on_primary("↵", &t)),
                        ),
                ),
        )
    }
}

// ── New terminal ──────────────────────────────────────────────────────────

pub struct TerminalDialog {
    input: Entity<TextInput>,
    folders: Vec<String>,
    _subs: Vec<Subscription>,
    _browse: Option<Task<()>>,
}

impl EventEmitter<DialogEvent> for TerminalDialog {}

impl TerminalDialog {
    pub fn new(
        initial: String,
        folders: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TerminalDialog {
        let input = cx.new(|cx| TextInput::new(cx, &initial, "~/code/project").mono());
        window.focus(&input.focus_handle(cx));
        let subs =
            vec![
                cx.subscribe_in(&input, window, |this, _, e: &InputEvent, _, cx| match e {
                    InputEvent::Submit => this.open(cx),
                    InputEvent::Cancel => cx.emit(DialogEvent::Close),
                    _ => cx.notify(),
                }),
            ];
        TerminalDialog {
            input,
            folders,
            _subs: subs,
            _browse: None,
        }
    }

    fn valid(path: &str) -> bool {
        path.starts_with('/') || path.starts_with('~') || is_windows_abs(path)
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let path = self.input.read(cx).text().trim().to_string();
        if Self::valid(&path) {
            cx.emit(DialogEvent::OpenTerminal(path));
        }
    }

    /// Up to 8 known folders matching what is typed.
    fn matches(&self, typed: &str) -> Vec<String> {
        let q = typed.trim().to_lowercase();
        self.folders
            .iter()
            .filter(|f| {
                q.is_empty()
                    || f.to_lowercase().contains(&q)
                    || q.starts_with('~')
                    || q.starts_with('/')
            })
            .filter(|f| q.is_empty() || f.to_lowercase().contains(q.trim_start_matches('~')))
            .take(8)
            .cloned()
            .collect()
    }
}

impl Render for TerminalDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let typed = self.input.read(cx).text().to_string();
        let ok = Self::valid(typed.trim());
        let list = self.matches(&typed);
        let close = cx.entity();
        let _ = window;
        frame(
            Some("New terminal"),
            480.,
            &t,
            crate::theme::motion_on(cx),
            move |_, cx| close.update(cx, |_, cx| cx.emit(DialogEvent::Close)),
            div()
                .flex()
                .flex_col()
                .min_h_0()
                .child(crate::kit::dialog::scrolling(
                    "td-body",
                    body()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(field_label("Folder", &t))
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(8.))
                                        .child(div().flex_1().child(self.input.clone()))
                                        .child(
                                            small_btn("td-browse", "Browse…", &t)
                                                .h(px(34.))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    let rx =
                                                        cx.prompt_for_paths(PathPromptOptions {
                                                            files: false,
                                                            directories: true,
                                                            multiple: false,
                                                            prompt: Some(
                                                                "Open a terminal in".into(),
                                                            ),
                                                        });
                                                    this._browse = Some(cx.spawn_in(
                                                        window,
                                                        async move |this, cx| {
                                                            if let Ok(Ok(Some(p))) = rx.await {
                                                                let p: Option<PathBuf> =
                                                                    p.into_iter().next();
                                                                let _ = this.update(cx, |d, cx| {
                                                                    if let Some(p) = p {
                                                                        let s = p
                                                                            .to_string_lossy()
                                                                            .to_string();
                                                                        d.input.update(
                                                                            cx,
                                                                            |i, cx| {
                                                                                i.set_text(&s, cx)
                                                                            },
                                                                        );
                                                                    }
                                                                });
                                                            }
                                                        },
                                                    ));
                                                })),
                                        ),
                                ),
                        )
                        .when(!list.is_empty(), |d| {
                            d.child(div().flex().flex_col().children(
                                list.into_iter().enumerate().map(|(i, f)| {
                                    let path = f.clone();
                                    div()
                                        .id(("td-folder", i))
                                        .px(px(8.))
                                        .py(px(5.))
                                        .rounded(px(4.))
                                        .font_family(MONO_FONT)
                                        .text_size(px(12.))
                                        .text_color(t.text_2)
                                        .ellipsis()
                                        .cursor_pointer()
                                        .hover_text(t.text, |s| s.bg(t.surface_3))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.emit(DialogEvent::OpenTerminal(path.clone()));
                                            let _ = this;
                                        }))
                                        .child(crate::kit::one_line(f))
                                }),
                            ))
                        }),
                    &t,
                ))
                .child(
                    foot(&t)
                        .child(
                            button("td-cancel", BtnKind::Ghost, false, &t)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
                                .child("Cancel"),
                        )
                        .child(
                            button("td-open", BtnKind::Primary, !ok, &t)
                                .on_click(cx.listener(|this, _, _, cx| this.open(cx)))
                                .child("Open terminal")
                                .child(kbd_on_primary("↵", &t)),
                        ),
                ),
        )
    }
}

// ── Remove agent ──────────────────────────────────────────────────────────

pub struct RemoveState {
    pub agent: String,
    pub delete_worktree: bool,
    pub busy: bool,
}

impl RemoveState {
    pub fn new(agent: &str) -> RemoveState {
        RemoveState {
            agent: agent.to_string(),
            delete_worktree: false,
            busy: false,
        }
    }
}

impl MainScreen {
    fn remove_now(&mut self, cx: &mut Context<Self>) {
        let Some(super::Modal::Remove(r)) = self.modal.as_mut() else {
            return;
        };
        let (Some(engine), false) = (self.engine.clone(), r.busy) else {
            return;
        };
        r.busy = true;
        let (id, wt) = (r.agent.clone(), r.delete_worktree);
        let name = self
            .agents(cx)
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let task = cx
            .background_executor()
            .spawn(async move { pitwall_core::engine::lifecycle::remove(&engine, &id, wt) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |s, cx| {
                match res {
                    Ok(()) => s.modal = None,
                    Err(e) => {
                        if let Some(super::Modal::Remove(r)) = s.modal.as_mut() {
                            r.busy = false;
                        }
                        s.failed(&format!("remove {name}"), e, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_remove(&mut self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(super::Modal::Remove(r)) = &self.modal else {
            return None;
        };
        let a = self.agents(cx).iter().find(|a| a.id == r.agent)?.clone();
        let (wt, busy) = (r.delete_worktree, r.busy);
        let text = if a.caps.remove_keeps_session {
            format!(
                "Pitwall stops tracking {}. Its session on {} is left as it is{}; you can add it again from Settings → Scan again.",
                a.name,
                a.machine.label,
                if a.running { " and keeps running" } else { "" }
            )
        } else {
            format!(
                "{}will be removed from Pitwall. Its terminal history goes with it.",
                if a.running {
                    "The agent process will be stopped and "
                } else {
                    "The agent "
                }
            )
        };
        let entity = cx.entity();
        let body = body()
            .child(div().line_height(px(19.)).child(text))
            .when(a.caps.remove_worktree, |d| {
                d.child(
                    div()
                        .id("rm-worktree")
                        .flex()
                        .items_start()
                        .gap(px(10.))
                        .cursor_pointer()
                        .on_click(cx.listener(|s, _, _, cx| {
                            if let Some(super::Modal::Remove(r)) = s.modal.as_mut() {
                                r.delete_worktree = !r.delete_worktree;
                            }
                            cx.notify();
                        }))
                        .child(check(wt, t))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child("Also remove worktree")
                                .child(
                                    div()
                                        .font_family(MONO_FONT)
                                        .text_size(px(12.))
                                        .text_color(t.text_3)
                                        .child(a.cwd_display.clone()),
                                )
                                // `.hint` with the command and branch in `.mono`.
                                .child(
                                    div().text_size(px(12.)).child(match &a.branch {
                                        Some(b) => crate::kit::rich(
                                            &[
                                                ("Runs ", Span::Text),
                                                ("git worktree remove", Span::Mono),
                                                (", which refuses if it has uncommitted changes. Branch ", Span::Text),
                                                (b, Span::Mono),
                                                (" is kept.", Span::Text),
                                            ],
                                            t.text_3,
                                        ),
                                        None => crate::kit::rich(
                                            &[
                                                ("Runs ", Span::Text),
                                                ("git worktree remove", Span::Mono),
                                                (", which refuses if it has uncommitted changes. Its branch is kept.", Span::Text),
                                            ],
                                            t.text_3,
                                        ),
                                    }),
                                ),
                        ),
                )
            })
            .when(!a.caps.remove_worktree && a.worktree_pending, |d| {
                d.child(hint("Its worktree (if the agent made one) is left as it is.", t))
            })
            .when(wt && a.caps.remove_worktree, |d| d.child(form_error("The worktree folder will be deleted from disk.", t)));
        let label_text = if busy {
            "Removing…"
        } else if wt && a.caps.remove_worktree {
            "Remove agent & worktree"
        } else {
            "Remove"
        };
        let close = entity.clone();
        Some(
            frame(
                Some(&format!("Remove {}", a.name)),
                440.,
                t,
                crate::theme::motion_on(cx),
                move |window, cx| close.update(cx, |s, cx| s.close_modal(window, cx)),
                div().flex().flex_col().min_h_0().child(crate::kit::dialog::scrolling("rm-body", body, t)).child(
                    foot(t)
                        .child(
                            button("rm-cancel", BtnKind::Ghost, false, t)
                                .on_click(cx.listener(|s, _, window, cx| s.close_modal(window, cx)))
                                .child("Cancel"),
                        )
                        .child(
                            button("rm-go", BtnKind::Danger, busy, t)
                                .on_click(cx.listener(|s, _, _, cx| s.remove_now(cx)))
                                .child(label_text),
                        ),
                ),
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_modal(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let _ = window;
        match &self.modal {
            None => None,
            Some(super::Modal::NewAgent(d)) => Some(
                div()
                    .absolute()
                    .inset_0()
                    .child(d.clone())
                    .into_any_element(),
            ),
            Some(super::Modal::Terminal(d)) => Some(
                div()
                    .absolute()
                    .inset_0()
                    .child(d.clone())
                    .into_any_element(),
            ),
            Some(super::Modal::Diff(d)) => Some(
                div()
                    .absolute()
                    .inset_0()
                    .child(d.clone())
                    .into_any_element(),
            ),
            Some(super::Modal::Remove(_)) => self.render_remove(t, cx),
            Some(super::Modal::RemoveWorktree(_)) => self.render_remove_worktree(t, cx),
        }
    }
}

// ── Diff of one file ──────────────────────────────────────────────────────

pub struct DiffDialog {
    agent: AgentView,
    file: FileChange,
    /// Review's code view, inline (`Layout::Unified`): highlighting, find,
    /// selection.
    code: Entity<crate::code_view::CodeView>,
    state: DiffState,
    _load: Task<()>,
}

#[derive(Debug, Clone, PartialEq)]
enum DiffState {
    Loading,
    Shown,
    Binary,
    Empty,
    Error(String),
}

impl EventEmitter<DialogEvent> for DiffDialog {}

impl DiffDialog {
    pub fn new(
        engine: Option<Shared>,
        agent: AgentView,
        file: FileChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DiffDialog {
        let code = cx.new(|cx| {
            let mut c = crate::code_view::CodeView::new(cx);
            c.set_layout(crate::code_view::Layout::Unified, cx);
            c.set_escape_passes(true);
            c
        });
        let (id, path) = (agent.id.clone(), file.path.clone());
        let task = cx.background_executor().spawn(async move {
            match engine {
                Some(e) => pitwall_core::engine::changes::file_versions(&e, &id, &path),
                None => Err("no engine".into()),
            }
        });
        let load = cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |d, cx| match res {
                Ok(v) if v.binary => {
                    d.state = DiffState::Binary;
                    cx.notify();
                }
                Ok(v) => d.set_versions(v.original, v.modified, cx),
                Err(e) => {
                    d.state = DiffState::Error(e);
                    cx.notify();
                }
            });
        });
        window.focus(&code.focus_handle(cx));
        DiffDialog {
            agent,
            file,
            code,
            state: DiffState::Loading,
            _load: load,
        }
    }

    /// Show these two versions of the file (`None`: it isn't there).
    pub fn set_versions(
        &mut self,
        old: Option<String>,
        new: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if old.is_none() && new.is_none() {
            self.state = DiffState::Empty;
        } else {
            self.state = DiffState::Shown;
            let path = self.file.path.clone();
            self.code.update(cx, |c, cx| {
                c.set_diff(&path, old.unwrap_or_default(), new.unwrap_or_default(), cx)
            });
        }
        self._load = Task::ready(());
        cx.notify();
    }
}

impl Render for DiffDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let (dir, base) = split_path(&self.file.path);
        let status = crate::kit::file_status(&self.file);
        let height = (f32::from(window.viewport_size().height) * 0.88).min(1000.);
        let head = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(16.))
            .pr(px(10.))
            .py(px(10.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.surface)
            .child(crate::kit::file_icon(&self.file.path, 16.))
            .child(
                div()
                    .min_w_0()
                    .ellipsis()
                    .flex()
                    .font_family(MONO_FONT)
                    .text_size(px(13.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(base.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_color(t.text_3)
                            .child(crate::kit::one_line(format!(" {}", dir.trim_end_matches('/')))),
                    ),
            )
            .child(crate::kit::status_letter(status, &t))
            .child(if self.file.binary {
                chip_subtle("bin", &t).into_any_element()
            } else {
                diffstat(self.file.added, self.file.removed, &t).into_any_element()
            })
            .child(div().flex_1())
            .child(label(self.agent.name.clone(), t.text_3, 12.))
            .child(hint("esc to close", &t))
            .child(
                icon_btn("diff-close", "x", "Close", false, &t)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
            );

        let content: AnyElement = match &self.state {
            DiffState::Error(e) => div()
                .p(px(16.))
                .text_size(px(12.))
                .text_color(t.red)
                .child(format!("Couldn't load diff: {e}"))
                .into_any_element(),
            DiffState::Loading => div()
                .p(px(16.))
                .child(hint("Loading diff…", &t))
                .into_any_element(),
            DiffState::Binary => div()
                .p(px(16.))
                .child(hint("Binary or very large file — not shown", &t))
                .into_any_element(),
            DiffState::Empty => div()
                .p(px(16.))
                .child(hint("No textual changes.", &t))
                .into_any_element(),
            DiffState::Shown => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.code.clone())
                .into_any_element(),
        };
        let close = cx.entity();
        frame(
            None,
            1080.,
            &t,
            crate::theme::motion_on(cx),
            move |_, cx| close.update(cx, |_, cx| cx.emit(DialogEvent::Close)),
            div().h(px(height)).flex().flex_col().child(head).child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .bg(t.term_bg)
                    .child(content),
            ),
        )
    }
}
