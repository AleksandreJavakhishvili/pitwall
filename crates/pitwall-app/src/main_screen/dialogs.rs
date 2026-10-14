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
    /// Move these agents of `kind` in `project` to this account (`None`:
    /// the user's own) named `name`; one not logged in yet logs in first
    /// (`login`: start its login terminal in the project).
    MoveAgents { ids: Vec<String>, account: Option<String>, name: String, kind: String, project: String, login: bool },
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
                .debug_selector({
                    let id = id.to_string();
                    move || format!("{id}-sel")
                })
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
            let id = id.to_string();
            d.child(deferred(
                anchored().snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id(SharedString::from(format!("{id}-list")))
                        // Takes its clicks: one below the dialog's box must
                        // not reach the backdrop under it (which closes it).
                        .occlude()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
                            let sel = format!("{id}-opt-{i}");
                            div()
                                .id(("opt", i))
                                .debug_selector(move || sel.clone())
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
    /// `None`: a folder outside the list (`other`: its path).
    project: Option<String>,
    other: Entity<TextInput>,
    /// The project picker's search (or a pasted path).
    query: Entity<TextInput>,
    /// The picker's keyboard row.
    pick_active: usize,
    /// "More options" (worktree, rules) shown.
    more: bool,
    /// The account chosen here (`"default"`: the user's own); `None`: the
    /// project's, else the last used.
    account: Option<String>,
    /// Make the chosen account the project's.
    account_for_project: bool,
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
        let query = cx.new(|cx| TextInput::new(cx, "", "Search projects, or paste a path").bare());
        let mut subs = vec![];
        for input in [&name, &other, &custom, &query] {
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
            query,
            pick_active: 0,
            more: false,
            account: None,
            account_for_project: false,
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
                // The kind started last, else the first installed one.
                let last = crate::ui_state::get(cx).new_agent_kind;
                let first = kinds
                    .iter()
                    .find(|k| k.installed && Some(&k.id) == last.as_ref())
                    .or(kinds.iter().find(|k| k.installed && !k.caps.custom_command))
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

    /// Demo only: the project picker open.
    pub fn demo_open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        self.open_picker(window, cx);
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
        // (Only its own list: the machines arriving after the dialog opened
        // must not close the project picker the user just opened.)
        if self.open == Some(Open::Target) {
            self.open = None;
        }
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
        if *input == self.query {
            match e {
                InputEvent::Changed => {
                    self.pick_active = 0;
                    cx.notify();
                }
                InputEvent::Submit | InputEvent::SubmitShift => {
                    if let Some(p) = self.picks(cx).into_iter().nth(self.pick_active) {
                        self.apply_pick(p, window, cx);
                    }
                }
                InputEvent::Cancel => self.close_picker(window, cx),
                InputEvent::Blur => {}
            }
            return;
        }
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
            let kind = self.kind.clone();
            // The account: asked for explicitly (the project's default is
            // the engine's when none is).
            let account = self.chosen_account(cx);
            req.account = Some(account.clone().unwrap_or_else(|| pitwall_core::accounts::DEFAULT.into()));
            if self.account_for_project {
                let path = self.project_path(cx);
                let m = crate::accounts::model(cx);
                let _ = m.update(cx, |m, cx| m.set_project_default(&path, &kind, account.as_deref(), cx));
            }
            let remembered = account.clone().unwrap_or_else(|| pitwall_core::accounts::DEFAULT.into());
            let k = kind.clone();
            crate::ui_state::update(cx, |s| {
                s.new_agent_accounts.insert(k, remembered);
            });
            crate::ui_state::update(cx, |s| s.new_agent_kind = Some(kind));
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

    /// The account the agent will run on (`None`: the user's own): the one
    /// chosen here, else the project's, else the last used for its kind.
    fn chosen_account(&self, cx: &mut App) -> Option<String> {
        use pitwall_core::accounts::DEFAULT;
        let m = crate::accounts::model(cx);
        let m = m.read(cx);
        let exists = |id: &str| m.list.get(id).is_some_and(|a| a.kind == self.kind);
        let pick = match &self.account {
            Some(a) => Some(a.clone()),
            None => {
                let path = self.project_path(cx);
                m.list
                    .project_default(&path, &self.kind)
                    .map(|a| a.id.clone())
                    .or_else(|| crate::ui_state::get(cx).new_agent_accounts.get(&self.kind).cloned())
            }
        };
        pick.filter(|a| a != DEFAULT && exists(a))
    }

    /// The account row: Default and the kind's accounts as chips (shown
    /// once the kind has one), "+ Add", and "use for this project".
    fn account_field(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let _ = window;
        if !self.folder() || self.target().is_some_and(|t| t.provider != pitwall_core::provider::ProviderId::LOCAL) {
            return None;
        }
        let list = crate::accounts::model(cx).read(cx).of_kind(&self.kind);
        if list.is_empty() {
            return None;
        }
        let chosen = self.chosen_account(cx);
        let path = self.project_path(cx);
        let project_default = crate::accounts::model(cx).read(cx).list.project_default(&path, &self.kind).map(|a| a.id.clone());
        let chip = |id: Option<String>, name: String, i: usize, cx: &mut Context<Self>| {
            let on = chosen == id;
            let is_project = id.is_some() && id == project_default;
            let pick = id.clone().unwrap_or_else(|| pitwall_core::accounts::DEFAULT.into());
            div()
                .id(("na-acct", i))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(28.))
                .px(px(10.))
                .rounded_full()
                .border_1()
                .border_color(if on { t.text_2 } else { t.line_strong })
                .when(on, |d| d.bg(t.surface_2))
                .cursor_pointer()
                .hover_probed(|s| s.bg(t.surface_2))
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.account = Some(pick.clone());
                    d.account_for_project = false;
                    cx.notify();
                }))
                .child(div().text_size(px(12.5)).when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(name))
                .when(is_project, |d| d.child(div().text_size(px(10.5)).text_color(t.text_3).child("project")))
                .into_any_element()
        };
        let mut chips: Vec<AnyElement> = vec![chip(None, "Default".into(), 0, cx)];
        for (i, a) in list.iter().enumerate() {
            chips.push(chip(Some(a.id.clone()), a.name.clone(), i + 1, cx));
        }
        chips.push(
            div()
                .id("na-acct-add")
                .flex()
                .items_center()
                .h(px(28.))
                .px(px(10.))
                .rounded_full()
                .border_1()
                .border_dashed()
                .border_color(t.line_strong)
                .text_size(px(12.5))
                .text_color(t.text_3)
                .cursor_pointer()
                .hover_text(t.text, |s| s)
                .tooltip(crate::kit::tooltip("Settings → Agents → Accounts"))
                .on_click(cx.listener(|_, _, window, cx| {
                    cx.emit(DialogEvent::Close);
                    window.dispatch_action(Box::new(crate::menu::OpenSettings), cx);
                }))
                .child("+ Add")
                .into_any_element(),
        );
        let can_remember = !path.is_empty() && chosen != project_default;
        let name = folder_name(&path).to_string();
        let on = self.account_for_project;
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(field_label("Account", t))
                .child(div().flex().flex_wrap().gap(px(6.)).children(chips))
                .when(can_remember, |d| {
                    d.child(
                        div()
                            .id("na-acct-project")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(|d, _, _, cx| {
                                d.account_for_project = !d.account_for_project;
                                cx.notify();
                            }))
                            .child(check(on, t))
                            .child(div().text_size(px(12.5)).text_color(t.text_2).child(format!("Use it for new agents in {name}"))),
                    )
                })
                .into_any_element(),
        )
    }

    /// The project picker's rows for what's typed: a pasted path to use,
    /// "Choose folder…", then the listed projects that match (name
    /// matches first).
    fn picks(&self, cx: &App) -> Vec<Pick> {
        let q = self.query.read(cx).text().trim().to_string();
        let mut out = Vec::new();
        if q.starts_with('/') || q.starts_with('~') || is_windows_abs(&q) {
            out.push(Pick::Path(q.clone()));
        }
        out.push(Pick::Choose);
        let needle = q.to_lowercase();
        let mut hits: Vec<(bool, usize)> = self
            .recents
            .iter()
            .enumerate()
            .filter_map(|(i, (path, display))| {
                if needle.is_empty() {
                    return Some((false, i));
                }
                let name = folder_name(path).to_lowercase();
                if name.contains(&needle) {
                    Some((!name.starts_with(&needle), i))
                } else if display.to_lowercase().contains(&needle) || path.to_lowercase().contains(&needle) {
                    Some((true, i))
                } else {
                    None
                }
            })
            .collect();
        hits.sort_by_key(|(later, _)| *later);
        out.extend(hits.into_iter().map(|(_, i)| Pick::Recent(i)));
        out
    }

    fn apply_pick(&mut self, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        match pick {
            Pick::Choose => self.browse(window, cx),
            Pick::Path(p) => self.use_path(p, window, cx),
            Pick::Recent(i) => {
                if let Some((path, _)) = self.recents.get(i) {
                    self.project = Some(path.clone());
                }
                self.suggest(window, cx);
            }
        }
        window.focus(&self.name.focus_handle(cx));
        cx.notify();
    }

    /// A folder outside the list (picked, pasted or dropped).
    fn use_path(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        match self.recents.iter().find(|(p, _)| *p == path) {
            Some((p, _)) => self.project = Some(p.clone()),
            None => {
                self.project = None;
                self.other.update(cx, |i, cx| i.set_text(&path, cx));
            }
        }
        self.suggest(window, cx);
        cx.notify();
    }

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == Some(Open::Project) {
            return self.close_picker(window, cx);
        }
        self.open = Some(Open::Project);
        self.pick_active = 0;
        self.query.update(cx, |i, cx| i.set_text("", cx));
        window.focus(&self.query.focus_handle(cx));
        cx.notify();
    }

    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        window.focus(&self.name.focus_handle(cx));
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
                        d.use_path(p.to_string_lossy().to_string(), window, cx)
                    });
                }
            }
        }));
    }
}

/// A row of New agent's project picker.
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    /// The system's folder picker.
    Choose,
    /// A typed or pasted path.
    Path(String),
    /// A listed project (index into `recents`).
    Recent(usize),
}

/// A path's last folder (`~/code/shop` → `shop`).
fn folder_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path)
}

fn is_windows_abs(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

impl Render for NewAgentDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

            // Project: a searchable picker (VS Code's quick pick).
            b = b.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(field_label("Project", &t))
                    .child(self.project_field(&t, window, cx))
                    .when_some(project_err.clone().filter(|_| !self.project_path(cx).is_empty()), |d, e| {
                        d.child(field_error(e, &t))
                    }),
            );
        }

        // The account (once the kind has more than one), after the project
        // whose default it may be.
        if folder {
            if let Some(acct) = self.account_field(&t, window, cx) {
                b = b.child(acct);
            }
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

        // More options: a separate worktree, rules.
        if folder {
            let can = self.selected_kind().is_some_and(|k| k.caps.rules);
            let wt_cap = self.selected_kind().is_some_and(|k| k.caps.worktree);
            let wt = wt_cap && self.worktree;
            let path = self.project_path(cx);
            self.rules.update(cx, |f, cx| f.sync(can, &path, wt, cx));
            let rules = self.rules.read(cx).visible();
            if wt_cap || rules {
                let open = self.more;
                let summary = if wt { "Separate worktree" } else { "" };
                b = b.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .pt(px(10.))
                        .border_t_1()
                        .border_color(t.line)
                        .child(
                            div()
                                .id("na-more")
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .cursor_pointer()
                                .text_size(px(12.5))
                                .text_color(t.text_2)
                                .hover_text(t.text, |s| s)
                                .on_click(cx.listener(|d, _, _, cx| {
                                    d.more = !d.more;
                                    cx.notify();
                                }))
                                .child(crate::kit::chevron("na-more-chev", open, 12., t.text_3))
                                .child("More options")
                                .when(!open && !summary.is_empty(), |d| {
                                    d.child(div().text_color(t.text_3).child(format!("· {summary}")))
                                }),
                        )
                        .when(open && wt_cap, |d| {
                            let on = self.worktree;
                            d.child(
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
                                                "Its own copy of the repo, so it doesn't clash with other agents in the same project.",
                                                &t,
                                            )),
                                    ),
                            )
                        })
                        .when(open && rules, |d| d.child(self.rules.clone())),
                );
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
                // ⌘↵ starts it from anywhere in the dialog (not the window's
                // maximize).
                .on_action(cx.listener(|d, _: &super::ToggleMaximize, window, cx| d.submit(window, cx)))
                // A folder dropped from Finder is the project.
                .drag_over::<gpui::ExternalPaths>(|s, _, _, _| s.opacity(0.85))
                .on_drop(cx.listener(|d, paths: &gpui::ExternalPaths, window, cx| {
                    let dir = paths.paths().iter().find(|p| p.is_dir()).cloned();
                    if let (Some(dir), true) = (dir, d.folder()) {
                        d.open = None;
                        d.use_path(dir.to_string_lossy().to_string(), window, cx);
                    }
                }))
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
                                .child(kbd_on_primary(if cfg!(target_os = "macos") { "⌘↵" } else { "Ctrl↵" }, &t)),
                        ),
                ),
        )
    }
}

impl NewAgentDialog {
    /// The project field: the chosen folder (name, path), opening the
    /// picker: search, "Choose folder…", a pasted path, the listed projects.
    fn project_field(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _ = window;
        let open = self.open == Some(Open::Project);
        let path = self.project_path(cx);
        let shown = match &self.project {
            Some(p) => Some((
                folder_name(p).to_string(),
                self.recents.iter().find(|r| r.0 == *p).map(|r| r.1.clone()).unwrap_or(p.clone()),
            )),
            None if !path.is_empty() => Some((folder_name(&path).to_string(), path.clone())),
            None => None,
        };
        let trigger = div()
            .id("na-project")
            .debug_selector(|| "na-project-sel".into())
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(44.))
            .px(px(10.))
            .rounded(RADIUS)
            .bg(t.bg)
            .border_1()
            .border_color(if open { t.text_3 } else { t.line_strong })
            .cursor_pointer()
            .hover_probed(|s| s.border_color(t.text_3))
            .on_click(cx.listener(|d, _, window, cx| d.open_picker(window, cx)))
            .child(icon("folder", 16., t.text_2))
            .child(match shown {
                Some((name, path)) => div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .ellipsis()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::kit::one_line(name)),
                    )
                    .child(
                        div()
                            .ellipsis()
                            .font_family(MONO_FONT)
                            .text_size(px(11.))
                            .text_color(t.text_3)
                            .child(crate::kit::one_line(path)),
                    )
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .text_color(t.text_3)
                    .child("Choose a project folder…")
                    .into_any_element(),
            })
            .child(icon("chevron", 12., t.text_3).with_transformation(gpui::Transformation::rotate(
                gpui::radians(std::f32::consts::FRAC_PI_2),
            )));
        div()
            .flex()
            .flex_col()
            .child(trigger)
            .when(open, |d| d.child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(self.picker(t, cx)))))
            .into_any_element()
    }

    fn picker(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let picks = self.picks(cx);
        let active = self.pick_active.min(picks.len().saturating_sub(1));
        let count = picks.len();
        let rows = picks.into_iter().enumerate().map(|(i, p)| {
            let on = i == active;
            let current = matches!(&p, Pick::Recent(r) if self.recents.get(*r).map(|x| &x.0) == self.project.as_ref());
            let row = div()
                .id(("na-pick", i))
                .debug_selector(move || format!("na-pick-{i}"))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(6.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.surface_3))
                .hover_probed(|s| s.bg(t.surface_3));
            let row = match &p {
                Pick::Choose => row
                    .child(icon("folder", 14., t.text_2))
                    .child(div().flex_1().child("Choose folder…"))
                    .child(div().text_size(px(11.5)).text_color(t.text_3).child("Browse")),
                Pick::Path(path) => row
                    .child(icon("folder", 14., t.text_2))
                    .child(div().flex_none().child("Use"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ellipsis()
                            .font_family(MONO_FONT)
                            .text_size(px(12.))
                            .child(crate::kit::one_line(path.clone())),
                    ),
                Pick::Recent(r) => {
                    let (path, display) = self.recents[*r].clone();
                    row.child(div().w(px(14.)).flex_none().when(current, |d| d.child(icon("check", 14., t.text_2))))
                        .child(
                            div()
                                .flex_none()
                                .max_w(px(180.))
                                .ellipsis()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(crate::kit::one_line(folder_name(&path).to_string())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .ellipsis()
                                .font_family(MONO_FONT)
                                .text_size(px(11.))
                                .text_color(t.text_3)
                                .child(crate::kit::one_line(display)),
                        )
                }
            };
            row.on_click(cx.listener(move |d, _, window, cx| d.apply_pick(p.clone(), window, cx)))
                .into_any_element()
        });
        let empty = count == 1 && !self.query.read(cx).text().trim().is_empty();
        div()
            .id("na-picker")
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|d, ev: &gpui::KeyDownEvent, _, cx| {
                let n = d.picks(cx).len();
                match ev.keystroke.key.as_str() {
                    "down" => d.pick_active = (d.pick_active + 1).min(n.saturating_sub(1)),
                    "up" => d.pick_active = d.pick_active.saturating_sub(1),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            // Below the field (an anchored box starts where its parent does).
            .mt(px(48.))
            .w(px(484.))
            .max_h(px(360.))
            // (Drawn later, outside the dialog: nothing inherited.)
            .text_color(t.text)
            .font_family(crate::kit::UI_FONT)
            .flex()
            .flex_col()
            .rounded(RADIUS)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .h(px(40.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(icon("search", 14., t.text_3))
                    .child(div().flex_1().child(self.query.clone())),
            )
            .child(
                div()
                    .id("na-pick-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(4.))
                    .text_size(px(13.))
                    .children(rows)
                    .when(empty, |d| {
                        d.child(div().px(px(10.)).py(px(8.)).text_color(t.text_3).child("No projects match"))
                    }),
            )
            .child(
                div()
                    .px(px(12.))
                    .py(px(6.))
                    .border_t_1()
                    .border_color(t.line)
                    .text_size(px(11.))
                    .text_color(t.text_3)
                    .child("↑↓ to move · ↵ to pick · or drop a folder from Finder on this dialog"),
            )
            .into_any_element()
    }
}

// ── A project's accounts (its right-click menu) ──────────────────────────

/// "Accounts…" on a project: for each agent that can have more than one
/// login (its definition names a login folder), which account the
/// project's new agents use, chosen from chips, or a new one: named here,
/// it becomes the project's, and an agent starts on it in the project,
/// where the tool asks for its login.
pub struct ProjectAccountsDialog {
    engine: Option<Shared>,
    project: String,
    /// The project's agents (they move along to its new account).
    agents: Vec<AgentView>,
    /// A new account being named, for this kind.
    adding: Option<(String, Entity<TextInput>)>,
    busy: bool,
    error: Option<String>,
    _subs: Vec<Subscription>,
    _task: Option<Task<()>>,
}

impl EventEmitter<DialogEvent> for ProjectAccountsDialog {}

impl ProjectAccountsDialog {
    pub fn new(engine: Option<Shared>, project: String, agents: Vec<AgentView>, cx: &mut Context<Self>) -> Self {
        let m = crate::accounts::model(cx);
        let subs = vec![cx.observe(&m, |_, _, cx| cx.notify())];
        ProjectAccountsDialog { engine, project, agents, adding: None, busy: false, error: None, _subs: subs, _task: None }
    }

    fn pick(&mut self, kind: &str, id: Option<&str>, name: String, cx: &mut Context<Self>) {
        let (m, project) = (crate::accounts::model(cx), self.project.clone());
        if let Err(e) = m.update(cx, |m, cx| m.set_project_default(&project, kind, id, cx)) {
            self.error = Some(e);
            return cx.notify();
        }
        // The project's agents of that kind follow at once (restarted there,
        // their conversations with them); an account not logged in yet logs
        // in first.
        let account = id.map(String::from);
        cx.emit(DialogEvent::MoveAgents { ids: self.movers(kind, &account), account, name, kind: kind.into(), project, login: true });
    }

    /// The project's agents of `kind` on another account than `account`.
    fn movers(&self, kind: &str, account: &Option<String>) -> Vec<String> {
        self.agents.iter().filter(|a| a.kind == kind && a.account != *account && !a.engineer).map(|a| a.id.clone()).collect()
    }

    fn start_adding(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| TextInput::new(cx, "", "Work"));
        window.focus(&input.focus_handle(cx));
        self._subs.push(cx.subscribe_in(&input, window, |this, _, e: &InputEvent, _, cx| match e {
            InputEvent::Submit => this.create(cx),
            InputEvent::Cancel => {
                this.adding = None;
                cx.notify();
            }
            _ => {
                this.error = None;
                cx.notify()
            }
        }));
        self.adding = Some((kind.to_string(), input));
        self.error = None;
        cx.notify();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let Some((kind, input)) = self.adding.clone() else { return };
        if self.busy {
            return;
        }
        let name = input.read(cx).text().trim().to_string();
        let (m, project) = (crate::accounts::model(cx), self.project.clone());
        let made = m.update(cx, |m, cx| {
            let a = m.add(&kind, &name, None, cx)?;
            m.set_project_default(&project, &kind, Some(&a.id), cx)?;
            Ok::<_, String>(a)
        });
        let account = match made {
            Ok(a) => a,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let Some(engine) = self.engine.clone() else {
            return cx.emit(DialogEvent::Close);
        };
        self.busy = true;
        let req = crate::accounts::login_request(&kind, &account.id, &project);
        let task = cx.background_executor().spawn(async move { pitwall_core::engine::lifecycle::create(&engine, req) });
        // Once it's logged in, the project's agents of that kind move there.
        let ids = self.movers(&kind, &Some(account.id.clone()));
        let then = DialogEvent::MoveAgents { ids, account: Some(account.id.clone()), name: account.name.clone(), kind, project, login: false };
        self._task = Some(cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |d, cx| match res {
                Ok(view) => {
                    cx.emit(then);
                    cx.emit(DialogEvent::Created(Box::new(view)));
                }
                Err(e) => {
                    d.busy = false;
                    d.error = Some(e);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// One agent's row: its name, then Default and its accounts as chips
    /// (the project's ticked), "+ New", or the new account's name field.
    fn kind_row(&self, def: &pitwall_core::kind::AgentKind, n: usize, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let m = crate::accounts::model(cx);
        let (list, current) = {
            let m = m.read(cx);
            (m.of_kind(&def.id), m.list.project_default(&self.project, &def.id).map(|a| a.id.clone()))
        };
        let chip = |id: Option<String>, name: String, i: usize, cx: &mut Context<Self>| {
            let on = current == id;
            let kind = def.id.clone();
            div()
                .id(("pa-chip", n * 100 + i))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(28.))
                .px(px(10.))
                .rounded_full()
                .border_1()
                .border_color(if on { t.text_2 } else { t.line_strong })
                .when(on, |d| d.bg(t.surface_2))
                .cursor_pointer()
                .hover_probed(|s| s.bg(t.surface_2))
                .on_click({
                    let name = name.clone();
                    cx.listener(move |d, _, _, cx| d.pick(&kind, id.as_deref(), name.clone(), cx))
                })
                .when(on, |d| d.child(icon("check", 12., t.text_2)))
                .child(div().text_size(px(12.5)).when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(name))
                .into_any_element()
        };
        let mut chips: Vec<AnyElement> = vec![chip(None, "Default".into(), 0, cx)];
        for (i, a) in list.iter().enumerate() {
            chips.push(chip(Some(a.id.clone()), a.name.clone(), i + 1, cx));
        }
        let adding = self.adding.as_ref().filter(|(k, _)| *k == def.id).map(|(_, i)| i.clone());
        if adding.is_none() {
            let kind = def.id.clone();
            chips.push(
                div()
                    .id(("pa-new", n))
                    .flex()
                    .items_center()
                    .h(px(28.))
                    .px(px(10.))
                    .rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(t.line_strong)
                    .text_size(px(12.5))
                    .text_color(t.text_3)
                    .cursor_pointer()
                    .hover_text(t.text, |s| s)
                    .on_click(cx.listener(move |d, _, window, cx| d.start_adding(&kind, window, cx)))
                    .child("+ New account")
                    .into_any_element(),
            );
        }
        let busy = self.busy;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(field_label(&def.name, t))
            .child(div().flex().flex_wrap().gap(px(6.)).children(chips))
            .when_some(adding, |d, input| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().child(input))
                        .child(
                            button(("pa-cancel", n), BtnKind::Ghost, false, t)
                                .on_click(cx.listener(|d, _, _, cx| {
                                    d.adding = None;
                                    cx.notify();
                                }))
                                .child("Cancel"),
                        )
                        .child(
                            button(("pa-create", n), BtnKind::Primary, busy, t)
                                .on_click(cx.listener(|d, _, _, cx| d.create(cx)))
                                .child(if busy { "Starting…" } else { "Create and log in" }),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Render for ProjectAccountsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let project = folder_name(&self.project).to_string();
        let kinds = crate::accounts::model(cx).read(cx).kinds();
        let rows: Vec<AnyElement> = kinds.iter().enumerate().map(|(n, k)| self.kind_row(k, n, &t, cx)).collect();
        let close = cx.entity();
        frame(
            Some(&format!("Accounts · {project}")),
            480.,
            &t,
            crate::theme::motion_on(cx),
            move |_, cx| close.update(cx, |_, cx| cx.emit(DialogEvent::Close)),
            div()
                .flex()
                .flex_col()
                .child(
                    body()
                        .child(hint(
                            format!("The login {project}'s agents run on. Picking one moves its agents there too, conversations included; one not logged in yet opens its login first."),
                            &t,
                        ))
                        .children(rows)
                        .when(kinds.is_empty(), |d| d.child(hint("None of your agents can have more than one login.", &t)))
                        .when_some(self.error.clone(), |d, e| d.child(form_error(e, &t))),
                )
                .child(
                    foot(&t)
                        .child(
                            div()
                                .id("pa-manage")
                                .flex_1()
                                .text_size(px(12.5))
                                .text_color(t.text_3)
                                .cursor_pointer()
                                .hover_text(t.text, |s| s.underline())
                                .on_click(cx.listener(|_, _, window, cx| {
                                    cx.emit(DialogEvent::Close);
                                    window.dispatch_action(Box::new(crate::menu::OpenSettings), cx);
                                }))
                                .child("Manage accounts…"),
                        )
                        .child(
                            button("pa-done", BtnKind::Primary, false, &t)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
                                .child("Done"),
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
            Some(super::Modal::Diff(d)) => {
                let shown = {
                    let (agent, path, stage) = d.read(cx).shows();
                    (agent.to_string(), path.to_string(), stage)
                };
                Some(
                    div()
                        .absolute()
                        .inset_0()
                        .child(d.clone())
                        .children(self.diff_row_targets(shown, t, cx))
                        .into_any_element(),
                )
            }
            Some(super::Modal::ProjectAccounts(d)) => Some(div().absolute().inset_0().child(d.clone()).into_any_element()),
            Some(super::Modal::Remove(_)) => self.render_remove(t, cx),
            Some(super::Modal::RemoveWorktree(_)) => self.render_remove_worktree(t, cx),
        }
    }
}

impl super::MainScreen {
    /// The Changes list's rows, live over an open diff's backdrop: hover and
    /// a pointer as on the list, and one click shows that file (the open
    /// one: closes it).
    fn diff_row_targets(
        &self,
        shown: (String, String, Option<pitwall_core::engine::changes::Stage>),
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // (Rows last drawn in a panel that has since closed: none.)
        let live = self.route == super::Route::Space && (self.right_pinned || self.drawer_open);
        let focused = self.focused_agent_id();
        let hover = t.text.opacity(0.08);
        self.file_rows
            .borrow()
            .iter()
            .enumerate()
            .filter(|_| live)
            .filter(|(_, (_, a, _))| focused.as_deref() == Some(a.id.as_str()))
            .map(|(i, (b, a, f))| {
                let same = shown == (a.id.clone(), f.path.clone(), super::panel::stage_of(f));
                let (agent, file) = (a.clone(), f.clone());
                anchored()
                    .position_mode(gpui::AnchoredPositionMode::Window)
                    .position(b.origin)
                    .child(
                        div()
                            .id(("diff-row-target", i))
                            .debug_selector(move || format!("diff-row-target-{i}"))
                            .occlude()
                            .w(b.size.width)
                            .h(b.size.height)
                            .rounded(crate::theme::RADIUS_SM)
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_modal(window, cx);
                                if !same {
                                    this.open_diff(agent.clone(), file.clone(), window, cx);
                                }
                            })),
                    )
                    .into_any_element()
            })
            .collect()
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
        // A staged or not-staged row: that part's diff.
        let stage = super::panel::stage_of(&file);
        let task = cx.background_executor().spawn(async move {
            match engine {
                Some(e) => pitwall_core::engine::changes::file_part_versions(&e, &id, &path, stage),
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

    /// The agent and file it shows, and which part of it.
    pub fn shows(&self) -> (&str, &str, Option<pitwall_core::engine::changes::Stage>) {
        (&self.agent.id, &self.file.path, super::panel::stage_of(&self.file))
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
            .when_some(super::panel::stage_of(&self.file), |d, s| {
                d.child(chip_subtle(
                    match s {
                        pitwall_core::engine::changes::Stage::Staged => "staged",
                        pitwall_core::engine::changes::Stage::Unstaged => "not staged",
                    },
                    &t,
                ))
            })
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

#[cfg(test)]
mod new_agent_tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    fn dialog(cx: &mut TestAppContext) -> (Entity<NewAgentDialog>, &mut VisualTestContext) {
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
        let (d, vcx) = cx.add_window_view(|window, cx| {
            NewAgentDialog::new(None, vec![], None, (None, None), window, cx)
        });
        let recents = vec![
            ("/work/shop".to_string(), "~/work/shop".to_string()),
            ("/work/api-gateway".to_string(), "~/work/api-gateway".to_string()),
            ("/work/notes/shopping-list".to_string(), "~/work/notes/shopping-list".to_string()),
        ];
        d.update_in(vcx, |d, window, cx| d.demo_fill(super::super::demo::kinds(), recents, window, cx));
        vcx.run_until_parked();
        (d, vcx)
    }

    fn typed(d: &Entity<NewAgentDialog>, vcx: &mut VisualTestContext, q: &str) -> Vec<Pick> {
        d.update(vcx, |d, cx| {
            d.query.update(cx, |i, cx| i.set_text(q, cx));
            d.picks(cx)
        })
    }

    #[gpui::test]
    fn the_picker_searches_names_first_and_takes_a_pasted_path(cx: &mut TestAppContext) {
        let (d, vcx) = dialog(cx);
        assert_eq!(
            typed(&d, vcx, ""),
            [Pick::Choose, Pick::Recent(0), Pick::Recent(1), Pick::Recent(2)],
            "Choose folder… then every project"
        );
        assert_eq!(typed(&d, vcx, "shop"), [Pick::Choose, Pick::Recent(0), Pick::Recent(2)]);
        // A name that only contains it comes after one that starts with it.
        assert_eq!(typed(&d, vcx, "list"), [Pick::Choose, Pick::Recent(2)]);
        assert_eq!(typed(&d, vcx, "zzz"), [Pick::Choose]);
        let picks = typed(&d, vcx, "/srv/app");
        assert_eq!(picks[0], Pick::Path("/srv/app".into()));
        d.update_in(vcx, |d, window, cx| d.apply_pick(picks[0].clone(), window, cx));
        d.read_with(vcx, |d, cx| {
            assert_eq!(d.project, None);
            assert_eq!(d.project_path(cx), "/srv/app");
            assert_eq!(d.name.read(cx).text(), "app", "named from the folder");
        });
        // A listed project picked from the search.
        let picks = typed(&d, vcx, "api");
        d.update_in(vcx, |d, window, cx| d.apply_pick(picks[1].clone(), window, cx));
        d.read_with(vcx, |d, cx| assert_eq!(d.project_path(cx), "/work/api-gateway"));
    }

    /// "Choose folder…" asks the system for a folder (the test platform has
    /// no picker: it panics there, which is the proof).
    #[gpui::test]
    #[should_panic(expected = "not implemented")]
    fn choose_folder_opens_the_system_picker(cx: &mut TestAppContext) {
        let (d, vcx) = dialog(cx);
        d.update_in(vcx, |d, window, cx| {
            d.open_picker(window, cx);
            d.apply_pick(Pick::Choose, window, cx)
        });
    }

    #[gpui::test]
    fn a_folder_outside_the_list_and_one_in_it(cx: &mut TestAppContext) {
        let (d, vcx) = dialog(cx);
        d.update_in(vcx, |d, window, cx| d.use_path("/work/shop".into(), window, cx));
        d.read_with(vcx, |d, _| assert_eq!(d.project.as_deref(), Some("/work/shop"), "the listed one"));
        d.update_in(vcx, |d, window, cx| d.use_path("/tmp/elsewhere".into(), window, cx));
        d.read_with(vcx, |d, cx| {
            assert_eq!(d.project, None);
            assert_eq!(d.project_path(cx), "/tmp/elsewhere");
        });
    }
}
