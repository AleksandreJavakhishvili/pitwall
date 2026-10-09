//! The first-launch welcome screen and Settings → "Scan again"
//! (`src/components/onboarding/Onboarding.tsx`, `FolderAccess.tsx`;
//! docs/spec/onboarding.md). Full window.
//!
//! The welcome screen starts with folder access (macOS asks once per
//! protected folder; Full Disk Access is the alternative) and only then
//! scans: the scan reads project folders, and the first read of
//! Desktop/Documents/Downloads is what makes macOS ask. The scan is
//! read-only; nothing changes until the user ticks a box and confirms.

pub mod plan;
pub mod source;

use crate::kit::Ellipsis as _;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    div, linear_color_stop, linear_gradient, prelude::*, px, relative, Animation, AnimationExt,
    AnyElement, App, Context, Div, Entity, EventEmitter, FontWeight, Hsla, PathPromptOptions,
    SharedString, Subscription, Task, Window,
};

use pitwall_core::engine::lifecycle;
use pitwall_core::model::{AdoptSessionRequest, CreateAgentRequest};
use pitwall_core::onboarding::scan::{Conversation, RunningAgent, ScanProgress, ScanResult};
use pitwall_core::onboarding::{self as core_onb, ContinueRequest};
use pitwall_core::permissions::PermissionsStatus;
use pitwall_core::Shared;
use pitwall_proto::{AgentView, ScannedMachine, ScannedSession};

use crate::agents::{AgentStore, StoreEvent};
use crate::theme::{Theme, RADIUS, RADIUS_LG};

use crate::theme::{appearance, chrome};
use super::permissions::{AccessEvent, AccessPhase, POLL};
use super::widgets::Ui;
use crate::kit::motion::{enter, tween, Ease, Fx};
use crate::kit::{tooltip, BtnKind, InputEvent, TextInput};
use super::SettingsHost;

use plan::{
    conv_key, default_conversations, default_selection, now_ms, plan_agents, rel_time,
    remembered_show_under, session_key, PlanInput, PlannedAgent,
};

/// The checklist (`STEPS`).
pub const STEPS: [(&str, &str); 6] = [
    ("agents", "Agents on your PATH"),
    ("projects", "Projects"),
    ("conversations", "Conversations you can continue"),
    ("running", "Running now"),
    ("rules", "Rule files"),
    ("hooks", "Codex hooks"),
];

const FIRST_PROJECTS: usize = 8;
const FIRST_CONVERSATIONS: usize = 6;

/// A dropdown choice applied to the onboarding view.
type PickIn = Rc<dyn Fn(&mut Onboarding, usize, &mut Window, &mut Context<Onboarding>)>;

fn source_label(s: &str) -> &str {
    match s {
        "claude" => "Claude",
        "codex" => "Codex",
        "vscode" => "VS Code",
        "cursor" => "Cursor",
        "folder" => "Folder",
        other => other,
    }
}

fn plural(n: usize, w: &str) -> String {
    format!("{n} {w}{}", if n == 1 { "" } else { "s" })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// First launch (`get_onboarded` false).
    Welcome,
    /// Settings → "Scan again".
    Rescan,
}

pub enum OnboardingEvent {
    /// Closed without starting anything (Skip, Cancel, Esc).
    Close,
    /// Started: hand these agents over to the main screen; `failures` are
    /// for toasts (the rest still started).
    Finished {
        created: Vec<AgentView>,
        failures: Vec<String>,
    },
}

/// Where an inline "Add folder…" form is open.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AddFor {
    /// Under the project list.
    List,
    /// For a "Show under" picker (conversation key).
    Picker(String),
}

pub struct Onboarding {
    mode: Mode,
    store: Entity<AgentStore>,
    engine: Option<Shared>,
    /// The scan and folder access (the engine, or a fixture).
    source: Arc<dyn source::OnboardingSource>,
    access: AccessPhase,
    access_error: Option<String>,
    steps: HashMap<String, (&'static str, Option<String>)>,
    result: Option<ScanResult>,
    scan_error: Option<String>,
    scan_started: bool,
    selected: BTreeSet<String>,
    all_projects: bool,
    all_convs: bool,
    conv_sel: BTreeSet<String>,
    run_sel: BTreeSet<u32>,
    fresh: BTreeMap<String, String>,
    adopt_sel: BTreeSet<String>,
    show_under: HashMap<String, String>,
    adding: Option<AddFor>,
    add_input: Entity<TextInput>,
    add_error: Option<String>,
    /// The open dropdown ("kind:<path>" or "under:<key>").
    open_select: Option<String>,
    progress: Option<(usize, usize)>,
    hooks: bool,
    show_hook_changes: bool,
    finishing: bool,
    finish_error: Option<String>,
    scroll: gpui::ScrollHandle,
    /// The checklist's column (top in the scrolled content, height) and the
    /// card's height, measured each frame for `position: sticky`.
    sticky: Rc<std::cell::Cell<(f32, f32, f32)>>,
    _subs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<OnboardingEvent> for Onboarding {}

impl Onboarding {
    pub fn new(mode: Mode, store: Entity<AgentStore>, cx: &mut Context<Self>) -> Onboarding {
        let engine = cx
            .try_global::<SettingsHost>()
            .and_then(|h| h.engine.clone());
        let source = source::get(cx, engine.clone());
        let add_input = cx.new(|cx| TextInput::new(cx, "", "/Users/you/code/project").mono());
        let subs = vec![
            cx.subscribe(&store, |this, _, e: &StoreEvent, cx| {
                if let StoreEvent::ScanProgress(p) = e {
                    this.progress(p.clone(), cx);
                }
            }),
            cx.subscribe(&add_input, |this, input, e: &InputEvent, cx| match e {
                InputEvent::Submit => {
                    let path = input.read(cx).text().trim().to_string();
                    if !path.is_empty() {
                        let target = this.adding.clone();
                        this.add_folder(path, target, cx);
                    }
                }
                InputEvent::Cancel => this.close_add(cx),
                _ => {}
            }),
        ];
        let mut this = Onboarding {
            mode,
            store,
            engine,
            source,
            access: if mode == Mode::Welcome {
                AccessPhase::Checking
            } else {
                AccessPhase::Done
            },
            access_error: None,
            steps: HashMap::new(),
            result: None,
            scan_error: None,
            scan_started: false,
            selected: BTreeSet::new(),
            all_projects: false,
            all_convs: false,
            conv_sel: BTreeSet::new(),
            run_sel: BTreeSet::new(),
            fresh: BTreeMap::new(),
            adopt_sel: BTreeSet::new(),
            show_under: HashMap::new(),
            adding: None,
            add_input,
            add_error: None,
            open_select: None,
            progress: None,
            hooks: false,
            show_hook_changes: false,
            finishing: false,
            finish_error: None,
            scroll: gpui::ScrollHandle::new(),
            sticky: Rc::default(),
            _subs: subs,
            _tasks: Vec::new(),
        };
        if this.access == AccessPhase::Done {
            this.start_scan(cx);
        } else {
            this.watch_access(cx);
        }
        this
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    // ── folder access ─────────────────────────────────────────────────

    /// Read the status now, then every 2 s while the step waits for the
    /// switch (`usePermissions`).
    fn watch_access(&mut self, cx: &mut Context<Self>) {
        let source = self.source.clone();
        let task = cx.spawn(async move |this, cx| loop {
            let src = source.clone();
            let s: PermissionsStatus = cx
                .background_executor()
                .spawn(async move { src.permissions() })
                .await;
            let polls = this.update(cx, |v, cx| {
                v.access_event(AccessEvent::Status(s), cx);
                v.access.polls()
            });
            match polls {
                Ok(true) => cx.background_executor().timer(POLL).await,
                _ => break,
            }
        });
        self._tasks.push(task);
    }

    fn access_event(&mut self, e: AccessEvent, cx: &mut Context<Self>) {
        let was = self.access;
        self.access = self.access.step(&e);
        if self.access == AccessPhase::Done && was != AccessPhase::Done {
            self.start_scan(cx);
        }
        if self.access.polls() && !was.polls() && was != AccessPhase::Checking {
            self.watch_access(cx);
        }
        cx.notify();
    }

    fn open_privacy(&mut self, cx: &mut Context<Self>) {
        self.access_error = None;
        self.access_event(AccessEvent::Open, cx);
        if let Err(e) = self.source.open_privacy() {
            self.access_error = Some(e);
        }
    }

    // ── scan ──────────────────────────────────────────────────────────

    fn start_scan(&mut self, cx: &mut Context<Self>) {
        if self.scan_started {
            return;
        }
        self.scan_started = true;
        if self.engine.is_none() && !self.source.is_fixture() {
            self.scan_error = Some("Pitwall's engine isn't running".into());
            return;
        }
        // Steps a source reports itself (the engine's come through the store).
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<ScanProgress>();
        let source = self.source.clone();
        let scan = cx.background_executor().spawn(async move {
            let tx = std::sync::Mutex::new(tx);
            source.scan(&move |p| {
                let _ = tx.lock().map(|t| t.unbounded_send(p));
            })
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            while let Some(p) = rx.next().await {
                if this.update(cx, |v, cx| v.progress(p, cx)).is_err() {
                    break;
                }
            }
        }));
        let task = cx.spawn(async move |this, cx| {
            let r = scan.await;
            let _ = this.update(cx, |v, cx| match r {
                Ok(r) => v.scanned(r, cx),
                Err(e) => {
                    v.scan_error = Some(e);
                    cx.notify();
                }
            });
        });
        self._tasks.push(task);
    }

    /// One checklist step's progress.
    fn progress(&mut self, p: ScanProgress, cx: &mut Context<Self>) {
        let summary = if self.demo() && p.step == "running" && p.summary.is_some() {
            Some("see below".into())
        } else {
            p.summary
        };
        self.steps.insert(p.step.to_string(), (p.status, summary));
        cx.notify();
    }

    /// The debug-only `$HOME` filter (`PITWALL_SCAN_DEMO`); never on a fixture.
    fn demo(&self) -> bool {
        demo() && !self.source.is_fixture()
    }

    fn agents(&self, cx: &App) -> Vec<AgentView> {
        self.store.read(cx).agents.clone()
    }

    fn scanned(&mut self, mut r: ScanResult, cx: &mut Context<Self>) {
        if self.demo() {
            // Screenshots: only what lives under the (made-up) $HOME.
            let home = pitwall_core::paths::home();
            r.running.retain(|a| {
                a.cwd
                    .as_deref()
                    .is_some_and(|c| std::path::Path::new(c).starts_with(&home))
            });
            r.places.clear();
            let names: Vec<&str> = r
                .agents
                .iter()
                .filter(|a| a.installed)
                .map(|a| a.name.as_str())
                .collect();
            if let Some(st) = self.steps.get_mut("agents") {
                st.1 = Some(names.join(" · "));
            }
        }
        let now = now_ms();
        let sel = default_selection(&r.projects, now);
        let mut ticked = sel.clone();
        ticked.extend(
            r.projects
                .iter()
                .filter(|p| p.added)
                .map(|p| p.path.clone()),
        );
        let installed = installed(&r);
        let agents: Vec<(String, Option<String>)> = self
            .agents(cx)
            .into_iter()
            .map(|a| (a.cwd, Some(a.project)))
            .collect();
        self.conv_sel = default_conversations(&r.conversations, &ticked, &agents, &installed, now);
        self.show_under = remembered_show_under(&r.conversations, &r.running);
        self.selected = sel;
        self.result = Some(r);
        cx.notify();
    }

    fn plan(&self, cx: &App) -> Vec<PlannedAgent> {
        let Some(r) = &self.result else {
            return Vec::new();
        };
        plan_agents(PlanInput {
            conversations: &r.conversations,
            conv_sel: &self.conv_sel,
            running: &r.running,
            run_sel: &self.run_sel,
            fresh: &self.fresh,
            taken: self.agents(cx).into_iter().map(|a| a.name).collect(),
            show_under: &self.show_under,
        })
    }

    fn adoptable(&self) -> Vec<ScannedSession> {
        self.result
            .iter()
            .flat_map(|r| r.places.iter())
            .flat_map(|p| p.machines.iter().flatten())
            .flat_map(|m| m.sessions.iter())
            .filter(|s| !s.in_pitwall)
            .cloned()
            .collect()
    }

    fn adopts(&self) -> Vec<ScannedSession> {
        self.adoptable()
            .into_iter()
            .filter(|s| self.adopt_sel.contains(&session_key(s)))
            .collect()
    }

    // ── choices ───────────────────────────────────────────────────────

    fn project_added(&self, path: &str) -> bool {
        self.result
            .as_ref()
            .is_some_and(|r| r.projects.iter().any(|p| p.path == path && p.added))
    }

    fn select_project(&mut self, path: Option<String>) {
        let Some(path) = path else { return };
        let known = self
            .result
            .as_ref()
            .is_some_and(|r| r.projects.iter().any(|p| p.path == path));
        if known && !self.project_added(&path) {
            self.selected.insert(path);
        }
    }

    fn toggle_project(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.selected.remove(path) {
            // Unticking a project drops what would have started in it.
            let convs = self
                .result
                .as_ref()
                .map(|r| r.conversations.clone())
                .unwrap_or_default();
            let under = self.show_under.clone();
            self.conv_sel.retain(|k| {
                let c = convs
                    .iter()
                    .find(|c| &conv_key(&c.kind, &c.session_id) == k);
                !c.is_some_and(|c| {
                    c.project_path == path
                        || (c.outside_project && under.get(k).map(String::as_str) == Some(path))
                })
            });
            self.fresh.remove(path);
        } else {
            self.selected.insert(path.to_string());
        }
        cx.notify();
    }

    fn toggle_conv(&mut self, c: &Conversation, cx: &mut Context<Self>) {
        let key = conv_key(&c.kind, &c.session_id);
        if self.conv_sel.remove(&key) {
            cx.notify();
            return;
        }
        self.conv_sel.insert(key.clone());
        // Continuing a conversation makes its folder (or the project it's
        // shown under) a project.
        let p = if c.outside_project {
            self.show_under.get(&key).cloned()
        } else {
            Some(c.project_path.clone())
        };
        self.select_project(p);
        cx.notify();
    }

    fn toggle_run(&mut self, r: &RunningAgent, cx: &mut Context<Self>) {
        if self.run_sel.remove(&r.pid) {
            cx.notify();
            return;
        }
        self.run_sel.insert(r.pid);
        if let Some(sid) = &r.session_id {
            let p = if r.outside_project {
                self.show_under.get(&conv_key(&r.kind, sid)).cloned()
            } else {
                r.cwd.clone()
            };
            self.select_project(p);
        }
        cx.notify();
    }

    fn default_kind(&self) -> Option<String> {
        self.result
            .as_ref()?
            .agents
            .iter()
            .find(|a| a.installed)
            .map(|a| a.kind.clone())
    }

    fn toggle_fresh(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.fresh.remove(path).is_none() {
            if let Some(k) = self.default_kind() {
                self.fresh.insert(path.to_string(), k);
            }
        }
        cx.notify();
    }

    /// Bulk "All" ticks only runnable rows not already in Pitwall and not
    /// open in another terminal (those need a deliberate, single tick).
    fn set_group(&mut self, outside: bool, on: bool, cx: &mut Context<Self>) {
        let Some(r) = &self.result else { return };
        let installed = installed(r);
        for c in r
            .conversations
            .iter()
            .filter(|c| c.outside_project == outside)
        {
            let key = conv_key(&c.kind, &c.session_id);
            if !on {
                self.conv_sel.remove(&key);
            } else if !c.in_pitwall && !c.running_elsewhere && installed.contains(&c.kind) {
                self.conv_sel.insert(key);
            }
        }
        cx.notify();
    }

    fn choose_under(
        &mut self,
        key: String,
        value: Option<String>,
        ticked: bool,
        cx: &mut Context<Self>,
    ) {
        self.open_select = None;
        match value {
            Some(v) => {
                self.show_under.insert(key, v.clone());
                if ticked {
                    self.select_project(Some(v));
                }
            }
            None => {
                self.show_under.remove(&key);
            }
        }
        cx.notify();
    }

    // ── Add folder ────────────────────────────────────────────────────

    /// The inline form (typed path) and the native folder picker.
    fn open_add(&mut self, target: AddFor, window: &mut Window, cx: &mut Context<Self>) {
        self.open_select = None;
        self.adding = Some(target.clone());
        self.add_error = None;
        self.add_input.update(cx, |i, cx| {
            i.set_text("", cx);
            i.focus_all(window, cx)
        });
        self.browse(target, cx);
        cx.notify();
    }

    fn browse(&mut self, target: AddFor, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update(cx, |v, cx| {
                        v.add_folder(p.to_string_lossy().into_owned(), Some(target), cx)
                    });
                }
            }
        })
        .detach();
    }

    fn close_add(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        self.add_error = None;
        cx.notify();
    }

    fn add_folder(&mut self, path: String, target: Option<AddFor>, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        self.add_error = None;
        let typed = path.clone();
        let run = cx
            .background_executor()
            .spawn(async move { core_onb::add_project(&engine, path) });
        cx.spawn(async move |this, cx| {
            let r = run.await;
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(list) => {
                        let norm = typed.trim_end_matches('/');
                        let added = list
                            .iter()
                            .find(|p| p.path == norm || p.display == norm)
                            .or_else(|| list.iter().max_by_key(|p| p.added_at))
                            .cloned();
                        if let (Some(r), Some(added)) = (v.result.as_mut(), added.as_ref()) {
                            match r.projects.iter_mut().find(|p| p.path == added.path) {
                                Some(p) => p.added = true,
                                None => r.projects.insert(
                                    0,
                                    pitwall_core::onboarding::scan::ScannedProject {
                                        path: added.path.clone(),
                                        display: added.display.clone(),
                                        is_git: added.is_git,
                                        last_used: None,
                                        sources: Vec::new(),
                                        agent_history: false,
                                        added: true,
                                        rules: Default::default(),
                                    },
                                ),
                            }
                        }
                        if let (Some(AddFor::Picker(key)), Some(added)) = (target, added) {
                            v.show_under.insert(key, added.path);
                        }
                        v.adding = None;
                    }
                    Err(e) => v.add_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ── finish ────────────────────────────────────────────────────────

    fn finish(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let Some(r) = &self.result else {
            return;
        };
        self.finishing = true;
        self.finish_error = None;
        let paths: Vec<String> = self
            .selected
            .iter()
            .filter(|p| !self.project_added(p))
            .cloned()
            .collect();
        let codex = r.codex_hooks.as_ref();
        let hooks = self.hooks && codex.is_some_and(|h| !h.installed);
        let plan = self.plan(cx);
        let adopts = self.adopts();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let e2 = engine.clone();
            let saved = cx
                .background_executor()
                .spawn(async move { core_onb::complete(&e2, &paths, hooks) })
                .await;
            let mut failures = Vec::new();
            if let Err(msg) = saved {
                if !msg.starts_with("Projects were saved") {
                    let _ = this.update(cx, |v, cx| {
                        v.finish_error = Some(msg);
                        v.finishing = false;
                        cx.notify();
                    });
                    return;
                }
                // Projects are in; only the hooks step failed.
                failures.push(format!("Couldn't install Codex hooks: {msg}"));
            }
            let total = plan.len() + adopts.len();
            let mut created = Vec::new();
            for (i, p) in plan.into_iter().enumerate() {
                let _ = this.update(cx, |v, cx| {
                    v.progress = Some((i, total));
                    cx.notify();
                });
                let e = engine.clone();
                let name = p.name.clone();
                let r = cx
                    .background_executor()
                    .spawn(async move { start_one(&e, p) })
                    .await;
                match r {
                    Ok(a) => created.push(a),
                    Err(e) => failures.push(format!("Couldn't start {name}: {e}")),
                }
            }
            // Sessions on other machines: tracked and attached, nothing
            // changes there.
            let offset = created.len() + failures.len();
            for (i, s) in adopts.into_iter().enumerate() {
                let _ = this.update(cx, |v, cx| {
                    v.progress = Some((offset + i, total));
                    cx.notify();
                });
                let e = engine.clone();
                let name = s.name.clone();
                let req = AdoptSessionRequest {
                    provider: s.provider,
                    machine: s.machine,
                    native: s.native,
                    cols: None,
                    rows: None,
                };
                let r = cx
                    .background_executor()
                    .spawn(async move { core_onb::adopt_session(&e, req) })
                    .await;
                match r {
                    Ok(a) => created.push(a),
                    Err(e) => failures.push(format!("Couldn't add {name}: {e}")),
                }
            }
            let _ = this.update(cx, |_, cx| {
                cx.emit(OnboardingEvent::Finished { created, failures })
            });
        })
        .detach();
    }

    /// Skip on the welcome screen still marks the folder onboarded.
    fn skip(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Rescan {
            cx.emit(OnboardingEvent::Close);
            return;
        }
        let Some(engine) = self.engine.clone() else {
            cx.emit(OnboardingEvent::Close);
            return;
        };
        self.finishing = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { core_onb::complete(&engine, &[], false) })
                .await;
            if let Err(e) = r {
                eprintln!("pitwall: couldn't finish setup: {e}");
            }
            let _ = this.update(cx, |_, cx| cx.emit(OnboardingEvent::Close));
        })
        .detach();
    }
}

/// How far a `position: sticky; top: 20px` box moves down: `scrolled` px
/// scrolled, its column `top` in the content and `col_h` tall, the box
/// `card_h` tall.
pub fn sticky_shift(scrolled: f32, top: f32, col_h: f32, card_h: f32) -> f32 {
    (scrolled + 20. - top).clamp(0., (col_h - card_h).max(0.))
}

/// `PITWALL_SCAN_DEMO=1` (debug builds only): the scan shows only agents
/// running under `$HOME` and no other places, so screenshots taken with a
/// made-up `$HOME` never show the machine's real sessions.
fn demo() -> bool {
    cfg!(debug_assertions) && std::env::var_os("PITWALL_SCAN_DEMO").is_some()
}

fn installed(r: &ScanResult) -> HashSet<String> {
    r.agents
        .iter()
        .filter(|a| a.installed)
        .map(|a| a.kind.clone())
        .collect()
}

/// Start one planned agent: resume its conversation, or a fresh one. Agents
/// start at the default size and resize when their tile attaches (the
/// tiling module owns hand-over sizes).
fn start_one(engine: &Shared, p: PlannedAgent) -> Result<AgentView, String> {
    match p.session_id {
        Some(session_id) => core_onb::continue_conversation(
            engine,
            ContinueRequest {
                kind: p.kind,
                session_id,
                project_path: p.project_path,
                name: p.name,
                display_project: p.display_project,
                cols: None,
                rows: None,
            },
        ),
        None => {
            let view = lifecycle::create(
                engine,
                CreateAgentRequest {
                    name: p.name,
                    kind: p.kind,
                    project_path: p.project_path,
                    ..Default::default()
                },
            )?;
            core_onb::add_agent_project(engine, &view);
            Ok(view)
        }
    }
}

// ───────────────────────────────────────────── drawing

struct Paint {
    ui: Ui,
    t: Theme,
    motion: bool,
    narrow: bool,
    /// The results column's width. Set explicitly: GPUI 0.2.2 can keep a
    /// paragraph shaped for a wider intrinsic-size pass than the width it
    /// lays out at (text then runs past the column).
    results_w: gpui::Pixels,
}

impl Paint {
    fn board(&self, cells: [(String, bool); 3]) -> Div {
        let t = &self.t;
        div().flex().child(
            div()
                .flex()
                .flex_none()
                .gap(px(4.))
                .p(px(6.))
                .rounded(px(6.))
                .bg(t.surface_2)
                .border_1()
                .border_color(t.line_strong)
                .children(cells.into_iter().enumerate().map(|(i, (text, live))| {
                    let color = if live {
                        t.green
                    } else if i == 2 {
                        t.text
                    } else {
                        t.text_2
                    };
                    let cell = div()
                        .min_w(px(40.))
                        .px(px(9.))
                        .py(px(3.))
                        .flex()
                        .justify_center()
                        .rounded(px(3.))
                        .bg(t.bg)
                        .text_color(color)
                        // Barlow Condensed 700, 15 px, 0.1em.
                        .child(
                            crate::kit::tracked(&text, 15., 0.1)
                                .font_family(crate::kit::LABEL_FONT)
                                .font_weight(FontWeight::BOLD),
                        );
                    if live && self.motion {
                        // `onb-blink`: 1 → 0.45 in `steps(2, start)` over
                        // 1.1 s: it shows 0.725, then 0.45.
                        cell.with_animation(
                            ("board-blink", i),
                            Animation::new(std::time::Duration::from_millis(1100)).repeat(),
                            |el, d| el.opacity(if d < 0.5 { 0.725 } else { 0.45 }),
                        )
                        .into_any_element()
                    } else {
                        cell.into_any_element()
                    }
                })),
        )
    }

    fn title(&self, text: &str) -> Div {
        // `.onb-title`: Barlow Condensed 700, 34 px, 0.06em.
        div().mt(px(6.)).child(
            crate::kit::tracked(&text.to_uppercase(), 34., 0.06)
                .line_height(px(35.))
                .font_family(crate::kit::LABEL_FONT)
                .font_weight(FontWeight::BOLD)
                .text_color(self.t.text),
        )
    }

    fn lede(&self, text: impl Into<SharedString>) -> Div {
        div()
            .max_w(px(560.))
            .line_height(px(20.))
            .text_color(self.t.text_2)
            .child(text.into())
    }

    fn section_head(&self, label: &str, count: Option<usize>) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(crate::kit::label(label.to_string(), self.ui.t.text_3, 12.).text_color(self.t.text_2))
            .when_some(count, |d, n| {
                d.child(
                    div()
                        .font_family(crate::kit::MONO_FONT)
                        .text_size(px(11.))
                        .text_color(self.t.text_3)
                        .child(n.to_string()),
                )
            })
            .child(div().flex_1())
    }

    fn list(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .rounded(RADIUS)
            .border_1()
            .border_color(self.t.line)
            .bg(self.t.surface)
    }

    /// One list row (`.onb-row`); `top` draws the separator above it.
    fn row(
        &self,
        id: impl Into<gpui::ElementId>,
        top: bool,
        disabled: bool,
    ) -> gpui::Stateful<Div> {
        let hover = self.t.surface_2;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(8.))
            .min_h(px(42.))
            .when(top, |d| d.border_t_1().border_color(self.t.line))
            .when(!disabled, |d| {
                d.cursor_pointer().hover(move |s| s.bg(hover))
            })
    }

    /// [`Paint::glyph`] with its motion: running spins (`onb-spin` 0.7 s),
    /// done pops in (`pop` 0.22 s).
    fn glyph_in(&self, id: SharedString, status: &str) -> AnyElement {
        match status {
            "running" => crate::kit::spinner_ms(id, 16., 700, self.motion, &self.ui.t)
                .into_any_element(),
            "done" => enter(id, Fx::ONB_POP, self.glyph(status)).into_any_element(),
            _ => self.glyph(status).into_any_element(),
        }
    }

    fn glyph(&self, status: &str) -> Div {
        let t = &self.t;
        let (bg, border, fg, text): (Option<Hsla>, Hsla, Hsla, &str) = match status {
            "done" => (Some(t.green_soft), gpui::transparent_black(), t.green, "✓"),
            "error" => (Some(t.red_soft), gpui::transparent_black(), t.red, "✕"),
            "skipped" => (Some(t.surface_3), gpui::transparent_black(), t.text_3, "–"),
            "running" => (None, t.text, t.text, ""),
            _ => (None, t.text_4, t.text_4, ""),
        };
        div()
            .flex_none()
            .mt(px(1.))
            .size(px(16.))
            .rounded_full()
            .border(px(1.5))
            .border_color(border)
            .when_some(bg, |d, bg| d.bg(bg))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(9.5))
            .font_weight(FontWeight::EXTRA_BOLD)
            .text_color(fg)
            .child(text)
    }
}

impl Onboarding {
    fn checklist(&self, p: &Paint) -> Div {
        let t = &p.t;
        let done = STEPS
            .iter()
            .filter(|(s, _)| self.steps.get(*s).is_some_and(|(st, _)| *st != "running"))
            .count();
        let lap = div()
            .h(px(3.))
            .mb(px(12.))
            .rounded(px(3.))
            .bg(t.surface_3)
            .overflow_hidden()
            // `.onb-lap span`: `transition: width 0.35s ease-out`.
            .child({
                let green = t.green;
                tween(
                    "onb-lap",
                    done as f32 / STEPS.len() as f32,
                    std::time::Duration::from_millis(350),
                    Ease::EaseOut,
                    move |w| div().h_full().w(relative(w)).bg(green).into_any_element(),
                )
            });
        div()
            .w(if p.narrow {
                relative(1.)
            } else {
                px(270.).into()
            })
            .flex_none()
            .px(px(14.))
            .pt(px(14.))
            .pb(px(10.))
            .rounded(RADIUS_LG)
            .bg(t.surface)
            .border_1()
            .border_color(t.line)
            .child(lap)
            .children(STEPS.iter().enumerate().map(|(i, (step, label))| {
                let st = self.steps.get(*step);
                let status = st.map(|s| s.0).unwrap_or(if self.result.is_some() {
                    "done"
                } else {
                    "pending"
                });
                let lit = matches!(status, "running" | "done" | "error");
                div()
                    .flex()
                    .gap(px(10.))
                    .items_start()
                    .py(px(7.))
                    .when(i + 1 < STEPS.len(), |d| d.border_b_1().border_color(t.line))
                    .child(p.glyph_in(SharedString::from(format!("onb-glyph-{step}")), status))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .min_w_0()
                            .child(
                                // `.onb-step-label`: 12.5 px, 0.08em.
                                crate::kit::label_t(
                                    label.to_string(),
                                    if lit { t.text } else { t.text_3 },
                                    12.5,
                                    0.08,
                                ),
                            )
                            .when_some(st.and_then(|s| s.1.clone()), |d, s| {
                                // `.onb-step-summary` fades in (0.2 s).
                                d.child(enter(
                                    SharedString::from(format!("onb-sum-{step}")),
                                    Fx::ONB_FADE,
                                    div()
                                        .font_family(crate::kit::MONO_FONT)
                                        .text_size(px(11.))
                                        .text_color(t.text_2)
                                        // Wraps as the browser does (no lone ")").
                                        .child(crate::kit::para(s)),
                                ))
                            })
                            .when(status == "running", |d| {
                                d.child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(t.text_3)
                                        .child("looking…"),
                                )
                            }),
                    )
            }))
            .when_some(self.scan_error.clone(), |d, e| {
                d.child(crate::kit::error_text(format!("Scan failed: {e}"), &p.ui.t).mt(px(8.)))
            })
    }

    fn add_form(&self, p: &Paint, cx: &mut Context<Self>) -> Div {
        let target = self.adding.clone().unwrap_or(AddFor::List);
        let t2 = target.clone();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w(px(220.)).child(self.add_input.clone()))
            .child(
                crate::kit::text_button("add-go", "Add", BtnKind::Small, false, &p.ui.t)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        let path = v.add_input.read(cx).text().trim().to_string();
                        if !path.is_empty() {
                            v.add_folder(path, Some(target.clone()), cx);
                        }
                    })),
            )
            .child(
                crate::kit::text_button("add-browse", "Browse…", BtnKind::Small, false, &p.ui.t)
                    .on_click(cx.listener(move |v, _, _, cx| v.browse(t2.clone(), cx))),
            )
            .child(
                crate::kit::text_button("add-cancel", "Cancel", BtnKind::Link, false, &p.ui.t)
                    .on_click(cx.listener(|v, _, _, cx| v.close_add(cx))),
            )
            .when_some(self.add_error.clone(), |d, e| {
                d.child(crate::kit::error_text(e, &p.ui.t).w_full())
            })
    }

    /// A dropdown: trigger + menu when open.
    fn select(
        &self,
        p: &Paint,
        key: String,
        current: String,
        items: Vec<(SharedString, bool)>,
        on_pick: PickIn,
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self.open_select.as_deref() == Some(key.as_str());
        let k2 = key.clone();
        let this = cx.entity().downgrade();
        let this2 = this.clone();
        div()
            .relative()
            .child(
                crate::kit::select_trigger(SharedString::from(format!("sel-{key}")), current, &p.ui.t)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.open_select = if v.open_select.as_deref() == Some(k2.as_str()) {
                            None
                        } else {
                            Some(k2.clone())
                        };
                        cx.notify();
                    })),
            )
            .when(open, |d| {
                d.child(crate::kit::select_menu(SharedString::from(format!("menu-{key}")), items, &p.ui.t, Rc::new(move |i, window, cx| {
                        let pick = on_pick.clone();
                        let _ = this.update(cx, |v, cx| {
                            v.open_select = None;
                            pick(v, i, window, cx);
                            cx.notify();
                        });
                    }), Rc::new(move |_, cx| {
                        let _ = this2.update(cx, |v, cx| {
                            v.open_select = None;
                            cx.notify();
                        });
                    })))
            })
    }

    /// "Show under project…" for a conversation or running session started
    /// outside a project.
    fn picker(
        &self,
        p: &Paint,
        key: String,
        where_: String,
        ticked: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let r = self.result.as_ref();
        let value = self.show_under.get(&key).cloned();
        let mut options: Vec<(String, String)> = r
            .map(|r| {
                r.projects
                    .iter()
                    .map(|p| (p.path.clone(), p.display.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(v) = &value {
            if !options.iter().any(|(p, _)| p == v) {
                options.insert(0, (v.clone(), v.clone()));
            }
        }
        let current = value
            .as_ref()
            .and_then(|v| options.iter().find(|(p, _)| p == v).map(|(_, d)| d.clone()))
            .unwrap_or_else(|| format!("{where_} (where it started)"));
        let mut items: Vec<(SharedString, bool)> = vec![(
            format!("{where_} (where it started)").into(),
            value.is_none(),
        )];
        items.extend(
            options
                .iter()
                .map(|(path, d)| (d.clone().into(), value.as_ref() == Some(path))),
        );
        items.push(("Add folder…".into(), false));
        let paths: Vec<String> = options.into_iter().map(|(p, _)| p).collect();
        let n = paths.len();
        let k = key.clone();
        let pick: PickIn = Rc::new(move |v, i, window, cx| {
            if i == 0 {
                v.choose_under(k.clone(), None, ticked, cx);
            } else if i <= n {
                v.choose_under(k.clone(), Some(paths[i - 1].clone()), ticked, cx);
            } else {
                v.open_add(AddFor::Picker(k.clone()), window, cx);
            }
        });
        let t = &p.t;
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(35.))
                    .pr(px(12.))
                    .pb(px(8.))
                    .text_color(if value.is_some() { t.text } else { t.text_3 })
                    .child(crate::kit::hint("Show under", &p.ui.t))
                    .child(self.select(p, format!("under:{key}"), current, items, pick, cx))
                    .when(value.is_some(), |d| {
                        d.child(crate::kit::hint(format!("runs in {where_}"), &p.ui.t))
                    }),
            )
            .when(
                self.adding.as_ref() == Some(&AddFor::Picker(key.clone())),
                |d| {
                    d.child(
                        div()
                            .pl(px(35.))
                            .pr(px(12.))
                            .pb(px(8.))
                            .child(self.add_form(p, cx)),
                    )
                },
            )
    }

    fn projects_section(&self, p: &Paint, r: &ScanResult, cx: &mut Context<Self>) -> Div {
        let t = &p.t;
        let plan = self.plan(cx);
        let covered: HashSet<String> = plan
            .iter()
            .filter(|a| a.session_id.is_some())
            .map(|a| {
                a.display_project
                    .clone()
                    .unwrap_or_else(|| a.project_path.clone())
            })
            .collect();
        let default_kind = self.default_kind();
        let kinds: Vec<(String, String)> = r
            .agents
            .iter()
            .filter(|a| a.installed)
            .map(|a| (a.kind.clone(), a.name.clone()))
            .collect();
        let shown = if self.all_projects {
            r.projects.len()
        } else {
            FIRST_PROJECTS
        };
        let now = now_ms();
        let mut list = p.list();
        for (i, pr) in r.projects.iter().take(shown).enumerate() {
            let checked = pr.added || self.selected.contains(&pr.path);
            let path = pr.path.clone();
            let mut chips: Vec<Div> = Vec::new();
            if pr.added {
                chips.push(crate::kit::chip_sm("in Pitwall", crate::kit::Tone::Ok, &p.ui.t));
            }
            chips.extend(
                pr.sources
                    .iter()
                    .map(|s| crate::kit::chip_sm(source_label(s).to_string(), crate::kit::Tone::Plain, &p.ui.t)),
            );
            if !pr.is_git {
                chips.push(crate::kit::chip_sm("no git", crate::kit::Tone::Subtle, &p.ui.t));
            }
            if pr.rules.rulesync {
                chips.push(crate::kit::chip_sm("rulesync", crate::kit::Tone::Subtle, &p.ui.t));
            }
            if pr.rules.claude_md {
                chips.push(crate::kit::chip_sm("CLAUDE.md", crate::kit::Tone::Subtle, &p.ui.t));
            }
            if pr.rules.agents_md {
                chips.push(crate::kit::chip_sm("AGENTS.md", crate::kit::Tone::Subtle, &p.ui.t));
            }
            let row = p
                .row(("proj", i), i > 0, pr.added)
                .when(!pr.added, |d| {
                    d.on_click(cx.listener(move |v, _, _, cx| v.toggle_project(&path, cx)))
                })
                .tooltip(tooltip(pr.path.clone()))
                .child(crate::kit::checkbox_box(checked, pr.added, 14., &p.ui.t))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .font_family(crate::kit::MONO_FONT)
                                .text_size(px(12.5))
                                .ellipsis()
                                .text_color(if checked { t.text } else { t.text_2 })
                                .child(crate::kit::one_line(pr.display.clone())),
                        )
                        .child(div().flex().flex_wrap().gap(px(4.)).children(chips)),
                )
                .child(crate::kit::hint(match pr.last_used {
                    Some(ms) => rel_time(ms, now),
                    None => "recently opened".into(),
                }, &p.ui.t));
            let show_fresh = checked && !covered.contains(&pr.path) && default_kind.is_some();
            let fresh_on = self.fresh.get(&pr.path).cloned();
            let item = div().flex().flex_col().child(row).when(show_fresh, |d| {
                let path = pr.path.clone();
                let path2 = pr.path.clone();
                let ks = kinds.clone();
                let current = fresh_on
                    .as_ref()
                    .and_then(|k| ks.iter().find(|(id, _)| id == k).map(|(_, n)| n.clone()))
                    .unwrap_or_default();
                let items = ks
                    .iter()
                    .map(|(id, n)| (SharedString::from(n.clone()), Some(id) == fresh_on.as_ref()))
                    .collect();
                let ids: Vec<String> = ks.iter().map(|(id, _)| id.clone()).collect();
                let pick: PickIn = Rc::new(move |v, i, _, _| {
                    v.fresh.insert(path2.clone(), ids[i].clone());
                });
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(10.))
                        .mt(px(-4.))
                        .pl(px(35.))
                        .pr(px(12.))
                        .pb(px(8.))
                        .text_size(px(12.))
                        .text_color(if fresh_on.is_some() { t.text } else { t.text_3 })
                        .child(
                            div()
                                .id(SharedString::from(format!("fresh-{}", pr.path)))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .cursor_pointer()
                                .on_click(cx.listener(move |v, _, _, cx| v.toggle_fresh(&path, cx)))
                                .child(crate::kit::checkbox_box(fresh_on.is_some(), false, 14., &p.ui.t))
                                .child("Start a new agent"),
                        )
                        .when(fresh_on.is_some(), |d| {
                            d.child(self.select(
                                p,
                                format!("kind:{}", pr.path),
                                current,
                                items,
                                pick,
                                cx,
                            ))
                        }),
                )
            });
            list = list.child(item);
        }
        let all: Vec<String> = r
            .projects
            .iter()
            .filter(|p| !p.added)
            .map(|p| p.path.clone())
            .collect();
        let n = r.projects.len();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                p.section_head("Projects", Some(n))
                    .child(
                        crate::kit::text_button("proj-all", "All", BtnKind::Link, false, &p.ui.t)
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.selected = all.iter().cloned().collect();
                                cx.notify();
                            })),
                    )
                    .child(crate::kit::text_button("proj-none", "None", BtnKind::Link, false, &p.ui.t).on_click(
                        cx.listener(|v, _, _, cx| {
                            v.selected.clear();
                            cx.notify();
                        }),
                    )),
            )
            .child(
                crate::kit::hint("Projects appear in the sidebar even before they have agents. Recent first.", &p.ui.t),
            )
            .when(n == 0, |d| {
                d.child(crate::kit::hint("No projects found. Add a folder below.", &p.ui.t))
            })
            .when(n > 0, |d| d.child(list))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(n > FIRST_PROJECTS, |d| {
                        d.child(
                            crate::kit::text_button("proj-more", if self.all_projects {
                                    "Show fewer".to_string()
                                } else {
                                    format!("Show all {n}")
                                }, BtnKind::Link, false, &p.ui.t)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.all_projects = !v.all_projects;
                                cx.notify();
                            })),
                        )
                    })
                    .child(div().flex_1())
                    .when(self.adding != Some(AddFor::List), |d| {
                        d.child(
                            crate::kit::button("add-folder", BtnKind::Small, false, &p.ui.t)
                                .child(crate::kit::icon("folder", 14., p.ui.t.text))
                                .child("Add folder…")
                                .on_click(cx.listener(|v, _, window, cx| {
                                    v.open_add(AddFor::List, window, cx)
                                })),
                        )
                    }),
            )
            .when(self.adding == Some(AddFor::List), |d| {
                d.child(self.add_form(p, cx))
            })
    }

    fn conv_row(
        &self,
        p: &Paint,
        c: &Conversation,
        top: bool,
        installed: &HashSet<String>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = &p.t;
        let key = conv_key(&c.kind, &c.session_id);
        let can_run = installed.contains(&c.kind);
        let checked = !c.in_pitwall && self.conv_sel.contains(&key);
        let disabled = c.in_pitwall || !can_run;
        let c2 = c.clone();
        let row = p
            .row(SharedString::from(format!("conv-{key}")), top, disabled)
            .when(!disabled, |d| {
                d.on_click(cx.listener(move |v, _, _, cx| v.toggle_conv(&c2, cx)))
            })
            .when(!can_run, |d| {
                d.tooltip(tooltip(format!("{} isn't installed", c.kind_name)))
            })
            .child(crate::kit::checkbox_box(checked, disabled, 14., &p.ui.t))
            .child(
                crate::kit::chip_tone(c.kind_name.clone(), crate::kit::Tone::Subtle, &p.ui.t)
                    .min_w(px(76.))
                    .justify_center(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .when(disabled, |d| d.opacity(0.6))
                    .child(
                        div()
                            .ellipsis()
                            .child(crate::kit::one_line(c.title.clone())),
                    )
                    .child(
                        div()
                            .font_family(crate::kit::MONO_FONT)
                            .text_size(px(11.5))
                            .text_color(t.text_3)
                            .ellipsis()
                            .child(crate::kit::one_line(format!(
                                "{} · {}",
                                c.project_display,
                                rel_time(c.last_used, now_ms())
                            ))),
                    ),
            )
            .when(c.in_pitwall, |d| d.child(crate::kit::chip_tone("in Pitwall", crate::kit::Tone::Ok, &p.ui.t)))
            .when(!c.in_pitwall && c.running_elsewhere, |d| {
                d.child(crate::kit::chip_tone("open elsewhere", crate::kit::Tone::Warn, &p.ui.t))
            });
        div()
            .flex()
            .flex_col()
            .child(row)
            .when(c.running_elsewhere && !c.in_pitwall && checked, |d| {
                d.child(
                    div()
                        .pl(px(35.))
                        .pr(px(12.))
                        .pb(px(8.))
                        .text_size(px(12.))
                        .text_color(t.text)
                        .child("This conversation is open in another terminal — close it first so two copies don't run."),
                )
            })
            .when(c.outside_project && !c.in_pitwall && can_run, |d| {
                d.child(self.picker(p, key.clone(), c.project_display.clone(), checked, cx))
            })
    }

    fn conversations_section(
        &self,
        p: &Paint,
        r: &ScanResult,
        outside: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let installed = installed(r);
        let convs: Vec<&Conversation> = r
            .conversations
            .iter()
            .filter(|c| c.outside_project == outside)
            .collect();
        if convs.is_empty() {
            return None;
        }
        let n = convs.len();
        let shown: Vec<&Conversation> = if outside || self.all_convs {
            convs.clone()
        } else {
            convs
                .iter()
                .enumerate()
                .filter(|(i, c)| {
                    *i < FIRST_CONVERSATIONS
                        || self.conv_sel.contains(&conv_key(&c.kind, &c.session_id))
                })
                .map(|(_, c)| *c)
                .collect()
        };
        let any_ticked = convs
            .iter()
            .any(|c| self.conv_sel.contains(&conv_key(&c.kind, &c.session_id)));
        let title = if !outside {
            "Conversations to continue"
        } else if convs.iter().all(|c| c.project_display == "~") {
            "Started in ~"
        } else {
            "Started outside a project"
        };
        let mut list = p.list();
        for (i, c) in shown.iter().enumerate() {
            list = list.child(self.conv_row(p, c, i > 0, &installed, cx));
        }
        let mut head = p.section_head(title, Some(n));
        if outside {
            head = head.child(
                crate::kit::text_button("out-all", "All", BtnKind::Link, false, &p.ui.t)
                    .tooltip(tooltip("Tick every conversation that isn't open in another terminal"))
                    .on_click(cx.listener(|v, _, _, cx| v.set_group(true, true, cx))),
            );
        }
        if outside || any_ticked {
            head = head.child(
                crate::kit::text_button(if outside { "out-none" } else { "conv-none" }, "None", BtnKind::Link, false, &p.ui.t)
                .on_click(cx.listener(move |v, _, _, cx| v.set_group(outside, false, cx))),
            );
        }
        Some(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(head)
                .child(crate::kit::hint(if outside {
                    "Conversations started outside a project folder. Pick a project to show one under — it still runs in the folder it started in, because that's the only place it can be resumed."
                } else {
                    "Each ticked conversation starts an agent in its folder and resumes that session. The latest one of each project from the last 3 days is ticked for you."
                }, &p.ui.t).w_full())
                .child(list)
                .when(!outside && n > FIRST_CONVERSATIONS, |d| {
                    d.child(div().flex().child(
                        crate::kit::text_button("conv-more", if self.all_convs { "Show fewer".to_string() } else { format!("Show all {n}") }, BtnKind::Link, false, &p.ui.t)
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.all_convs = !v.all_convs;
                            cx.notify();
                        })),
                    ))
                }),
        )
    }

    fn machine_group(
        &self,
        p: &Paint,
        place: &str,
        m: &ScannedMachine,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = &p.t;
        let mut list = p.list();
        if m.sessions.is_empty() {
            list = list.child(
                p.row("no-sessions", false, true)
                    .child(crate::kit::hint("No sessions", &p.ui.t)),
            );
        }
        for (i, s) in m.sessions.iter().enumerate() {
            let key = session_key(s);
            let checked = !s.in_pitwall && self.adopt_sel.contains(&key);
            let k2 = key.clone();
            let detail = [
                s.workspace.as_ref().map(|w| format!("workspace {w}")),
                s.user.as_ref().map(|u| format!("as {u}")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let status = match s.status.as_str() {
                "running" => "running",
                "stopped" => "stopped",
                _ => "status unknown",
            };
            list = list.child(
                p.row(
                    SharedString::from(format!("adopt-{key}")),
                    i > 0,
                    s.in_pitwall,
                )
                .when(!s.in_pitwall, |d| {
                    d.on_click(cx.listener(move |v, _, _, cx| {
                        if !v.adopt_sel.remove(&k2) {
                            v.adopt_sel.insert(k2.clone());
                        }
                        cx.notify();
                    }))
                })
                .child(crate::kit::checkbox_box(checked, s.in_pitwall, 14., &p.ui.t))
                .child(
                    crate::kit::chip_tone(s.kind_name.clone(), crate::kit::Tone::Subtle, &p.ui.t)
                        .min_w(px(76.))
                        .justify_center(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .font_family(crate::kit::MONO_FONT)
                                .text_size(px(12.5))
                                .child(s.name.clone()),
                        )
                        .child(div().text_size(px(11.5)).text_color(t.text_3).child(detail)),
                )
                .child(crate::kit::chip_tone(status, if s.status == "running" {
                        crate::kit::Tone::Ok
                    } else {
                        crate::kit::Tone::Subtle
                    }, &p.ui.t))
                .child(if s.in_pitwall {
                    crate::kit::chip_tone("in Pitwall", crate::kit::Tone::Ok, &p.ui.t)
                } else {
                    crate::kit::hint("Add to Pitwall", &p.ui.t)
                }),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .mt(px(4.))
                    .child(
                        // `.onb-machine-name.mono`: text-2, 600, 12 px × 0.94.
                        div()
                            .font_family(crate::kit::MONO_FONT)
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text_2)
                            .child(m.label.clone()),
                    )
                    .child(
                        crate::kit::hint([Some(place.to_string()), m.detail.clone()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" · "), &p.ui.t),
                    ),
            )
            .child(list)
    }

    fn running_section(&self, p: &Paint, r: &ScanResult, cx: &mut Context<Self>) -> Option<Div> {
        if r.running.is_empty() && r.places.is_empty() {
            return None;
        }
        let t = &p.t;
        let installed = installed(r);
        let count = r.running.len()
            + r.places
                .iter()
                .flat_map(|p| p.machines.iter().flatten())
                .map(|m| m.sessions.len())
                .sum::<usize>();
        let mut sec = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(p.section_head("Running now", Some(count)));
        if !r.running.is_empty() {
            let mut list = p.list();
            for (i, ra) in r.running.iter().enumerate() {
                let can = ra.session_id.is_some()
                    && ra.cwd.is_some()
                    && installed.contains(&ra.kind)
                    && !ra.in_pitwall;
                let checked = can && self.run_sel.contains(&ra.pid);
                let r2 = ra.clone();
                let sub = format!(
                    "{} · pid {}",
                    ra.title.clone().unwrap_or_else(|| match &ra.session_id {
                        Some(s) => format!("session {}", &s[..s.len().min(8)]),
                        None => "no conversation found".into(),
                    }),
                    ra.pid
                );
                let row = p
                    .row(("run", i), i > 0, !can)
                    .when(can, |d| {
                        d.on_click(cx.listener(move |v, _, _, cx| v.toggle_run(&r2, cx)))
                    })
                    .child(crate::kit::checkbox_box(checked, !can, 14., &p.ui.t))
                    .child(
                        crate::kit::chip_tone(ra.kind_name.clone(), crate::kit::Tone::Subtle, &p.ui.t)
                            .min_w(px(76.))
                            .justify_center(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .when(!can, |d| d.opacity(0.6))
                            .child(
                                div().font_family(crate::kit::MONO_FONT).text_size(px(12.5)).child(
                                    ra.cwd_display
                                        .clone()
                                        .unwrap_or_else(|| "unknown folder".into()),
                                ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(t.text_3)
                                    .ellipsis()
                                    .child(crate::kit::one_line(sub)),
                            ),
                    )
                    .child(if ra.in_pitwall {
                        crate::kit::chip_tone("in Pitwall", crate::kit::Tone::Ok, &p.ui.t)
                    } else {
                        crate::kit::hint(if can {
                            "Bring into Pitwall"
                        } else {
                            "can't bring over"
                        }, &p.ui.t)
                    });
                let key = ra.session_id.as_ref().map(|s| conv_key(&ra.kind, s));
                list = list.child(div().flex().flex_col().child(row).when_some(
                    key.filter(|_| ra.outside_project && can),
                    |d, key| {
                        d.child(self.picker(
                            p,
                            key,
                            ra.cwd_display.clone().unwrap_or_else(|| "~".into()),
                            checked,
                            cx,
                        ))
                    },
                ));
            }
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(t.line)
                    .text_size(px(12.))
                    .text_color(if self.run_sel.is_empty() { t.text_3 } else { t.text })
                    .child("Pitwall resumes the same conversation in its own terminal — close the old terminal first so two copies don't run."),
            );
            sec = sec
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .mt(px(4.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text_2)
                                .child(pitwall_core::host::machine_label()),
                        )
                        .child(crate::kit::hint("running outside Pitwall", &p.ui.t)),
                )
                .child(list);
        }
        for place in &r.places {
            match &place.machines {
                Some(ms) => {
                    for m in ms {
                        sec = sec.child(self.machine_group(p, &place.label, m, cx));
                    }
                }
                None => {
                    let name = [Some(place.label.clone()), place.version.clone()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    sec = sec.child(crate::kit::hint(format!(
                        "{name} is installed, but its machines and sessions couldn't be listed."
                    ), &p.ui.t));
                }
            }
        }
        if !self.adoptable().is_empty() {
            sec = sec.child(crate::kit::hint("Adding a session only starts tracking it: Pitwall attaches to its terminal and reads its status from the screen. Nothing changes on the machine.", &p.ui.t));
        }
        Some(sec)
    }

    fn codex_section(&self, p: &Paint, r: &ScanResult, cx: &mut Context<Self>) -> Option<Div> {
        let h = r.codex_hooks.as_ref()?;
        let mut sec = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(p.section_head("Codex status", None));
        if h.installed {
            return Some(
                sec.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(crate::kit::chip_tone("hooks installed", crate::kit::Tone::Ok, &p.ui.t))
                        .child(crate::kit::hint("Codex status is exact.", &p.ui.t)),
                ),
            );
        }
        sec = sec.child(
            div()
                .flex()
                .gap(px(10.))
                .items_start()
                .child(
                    div()
                        .id("hooks-check")
                        .mt(px(2.))
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.hooks = !v.hooks;
                            cx.notify();
                        }))
                        .child(crate::kit::checkbox_box(self.hooks, false, 14., &p.ui.t)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .id("hooks-label")
                                .cursor_pointer()
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.hooks = !v.hooks;
                                    cx.notify();
                                }))
                                .child("Exact Codex status (install hooks)"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(px(4.))
                                .child(crate::kit::hint("Without hooks Pitwall reads Codex's screen to tell working, blocked and done apart.", &p.ui.t))
                                .child(
                                    crate::kit::text_button("hook-changes", if self.show_hook_changes { "Hide changes" } else { "Show changes" }, BtnKind::Link, false, &p.ui.t)
                                    .on_click(cx.listener(|v, _, _, cx| {
                                        v.show_hook_changes = !v.show_hook_changes;
                                        cx.notify();
                                    })),
                                ),
                        ),
                ),
        );
        if self.show_hook_changes {
            sec = sec.child(
                p.ui.confirm_box()
                    .child(format!("This edits {}:", h.path))
                    .child(p.ui.list(
                        vec![
                            "A backup of the current file is made first (hooks.json.pitwall-backup).".into_any_element(),
                            "Pitwall entries are appended; your existing hooks stay.".into_any_element(),
                            "Outside Pitwall the hook does nothing and exits silently.".into_any_element(),
                        ],
                        false,
                    )),
            );
        }
        Some(sec)
    }

    fn footer(&self, p: &Paint, cx: &mut Context<Self>) -> Div {
        let t = &p.t;
        let plan_n = self.plan(cx).len();
        let start = plan_n + self.adopts().len();
        let new_projects = self
            .selected
            .iter()
            .filter(|p| !self.project_added(p))
            .count();
        let primary: String = if self.finishing {
            if self.progress.is_some() {
                "Starting…"
            } else {
                "Saving…"
            }
            .into()
        } else if self.mode == Mode::Welcome {
            if start > 0 {
                format!("Start Pitwall · {} →", plural(start, "agent"))
            } else {
                "Start Pitwall →".into()
            }
        } else {
            let mut parts = Vec::new();
            if new_projects > 0 {
                parts.push(format!("Add {}", plural(new_projects, "project")));
            }
            if start > 0 {
                parts.push(format!(
                    "{}tart {}",
                    if parts.is_empty() { "S" } else { "s" },
                    plural(start, "agent")
                ));
            }
            if parts.is_empty() {
                "Done".into()
            } else {
                parts.join(" · ")
            }
        };
        let can_finish = !self.finishing && (self.result.is_some() || self.scan_error.is_some());
        let note: Option<Div> = if let Some(e) = &self.finish_error {
            Some(crate::kit::error_text(e.clone(), &p.ui.t))
        } else if let Some((done, total)) = self.progress {
            Some(crate::kit::hint(format!(
                "Starting {}… {}/{}",
                plural(total, "agent"),
                done + 1,
                total
            ), &p.ui.t))
        } else if !self.finishing && self.result.is_some() && start == 0 {
            Some(crate::kit::hint("No agents will start — tick a conversation or “Start a new agent”.", &p.ui.t))
        } else {
            None
        };
        let _ = t;
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .children(note)
            .child(div().flex_1())
            .child(
                crate::kit::text_button("onb-skip", if self.mode == Mode::Welcome {
                        "Skip"
                    } else {
                        "Cancel"
                    }, BtnKind::Ghost, self.finishing, &p.ui.t)
                .on_click(cx.listener(|v, _, _, cx| {
                    if !v.finishing {
                        v.skip(cx)
                    }
                })),
            )
            .child(
                crate::kit::text_button("onb-start", primary, BtnKind::PrimaryLg, !can_finish, &p.ui.t)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        if can_finish {
                            if v.result.is_some() {
                                v.finish(cx)
                            } else {
                                v.skip(cx)
                            }
                        }
                    })),
            )
    }

    /// Folder access: macOS's per-folder prompts are the normal path
    /// ("Continue"); Full Disk Access is offered as a secondary choice with
    /// what it means. Granting it is still noticed by itself.
    fn access_screen(&self, p: &Paint, cx: &mut Context<Self>) -> (AnyElement, Div) {
        let t = &p.t;
        let granted = self.access == AccessPhase::Granted;
        let waiting = self.access == AccessPhase::Waiting;
        let folders = ["Desktop", "Documents", "Downloads"];
        let prompts = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(p.section_head("macOS asks once per folder", None).child(if granted {
                crate::kit::chip_tone("not needed now", crate::kit::Tone::Subtle, &p.ui.t)
            } else {
                crate::kit::chip_tone("recommended", crate::kit::Tone::Ok, &p.ui.t)
            }))
            .child(crate::kit::muted("When Pitwall first reads a project in one of these folders, macOS asks “Pitwall would like to access…”. Allow it and it won't ask about that folder again.", &p.ui.t).line_height(px(20.)))
            .child(div().flex().gap(px(8.)).children(folders.into_iter().map(|f| {
                div()
                    .px(px(10.))
                    .py(px(3.))
                    .rounded(RADIUS)
                    .bg(t.surface_2)
                    .border_1()
                    .border_color(t.line)
                    .text_size(px(12.5))
                    .text_color(t.text)
                    .child(f)
            })))
            .child(crate::kit::hint("At most three prompts, once each. Projects anywhere else never prompt.", &p.ui.t));
        let mut fda = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pt(px(12.))
            .border_t_1()
            .border_color(t.line);
        if granted {
            fda = fda.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(p.glyph_in("fda-done".into(), "done"))
                    .child(div().text_color(t.green).child("Full Disk Access is on. macOS won't ask about your folders.")),
            );
        } else if waiting {
            fda = fda
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("Full Disk Access"))
                .child(crate::kit::hint("In Privacy & Security → Full Disk Access, turn on Pitwall (if it isn't listed, click + and choose it in Applications), then come back: this page notices by itself.", &p.ui.t))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(crate::kit::spinner_ms("fda-spin", 14., 900, p.motion, &p.ui.t))
                        .child(div().text_color(t.text_2).child("Waiting for the switch…"))
                        .child(div().flex_1())
                        .child(
                            crate::kit::text_button("fda-open", "Open Settings again", BtnKind::Link, false, &p.ui.t)
                                .on_click(cx.listener(|v, _, _, cx| v.open_privacy(cx))),
                        ),
                )
                .child(crate::kit::hint("If macOS offers “Quit & Reopen”, either choice is fine — Pitwall picks up where you left off.", &p.ui.t));
        } else {
            fda = fda.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_none().text_size(px(12.)).text_color(t.text_3).child("Rather not see prompts?"))
                    .child(
                        crate::kit::text_button("fda-open", "Use Full Disk Access instead…", BtnKind::Link, false, &p.ui.t)
                            .on_click(cx.listener(|v, _, _, cx| v.open_privacy(cx))),
                    ),
            );
        }
        if !granted {
            fda = fda.child(crate::kit::hint("Full Disk Access lets Pitwall and every agent it starts read everything on this Mac, not just your projects.", &p.ui.t));
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .max_w(px(640.))
            .px(px(20.))
            .py(px(18.))
            .rounded(RADIUS_LG)
            .bg(t.surface)
            .border_1()
            .border_color(if granted { t.green.opacity(0.4) } else { t.line })
            .child(prompts)
            .child(fda)
            .when_some(self.access_error.clone(), |d, e| d.child(crate::kit::error_text(e, &p.ui.t)));
        let footer = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(crate::kit::hint("You can change this later in Settings → Folder access.", &p.ui.t))
            .child(div().flex_1())
            .child(if granted {
                crate::kit::text_button("fda-continue", "Continue →", BtnKind::PrimaryLg, false, &p.ui.t)
                    .on_click(cx.listener(|v, _, _, cx| v.access_event(AccessEvent::Continue, cx)))
            } else {
                crate::kit::text_button("fda-continue", "Continue", BtnKind::PrimaryLg, false, &p.ui.t)
                    .on_click(cx.listener(|v, _, _, cx| v.access_event(AccessEvent::Skip, cx)))
            });
        let content = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .mb(px(28.))
                    .child(p.board([
                        ("PIT".into(), false),
                        ("1/2".into(), false),
                        (if granted { "GO" } else { "ACCESS" }.into(), !granted),
                    ]))
                    .child(p.title("Welcome to Pitwall"))
                    .child(p.lede("First, folder access. Your agents work inside your projects, and projects often live in Desktop, Documents or Downloads, which macOS guards.")),
            )
            .child(body)
            .into_any_element();
        (content, footer)
    }

    /// `.onb-checklist { position: sticky; top: 20px }`: the card moves down
    /// with the scroll, inside its column.
    fn sticky_checklist(&self, p: &Paint, view: gpui::EntityId) -> Div {
        let (top, col_h, card_h) = self.sticky.get();
        let scrolled = -f32::from(self.scroll.offset().y);
        let shift = sticky_shift(scrolled, top, col_h, card_h);
        let (s1, s2, h) = (self.sticky.clone(), self.sticky.clone(), self.scroll.clone());
        div()
            .flex_none()
            .relative()
            .child(
                gpui::canvas(
                    move |b, _, cx| {
                        let scrolled = -f32::from(h.offset().y);
                        let top = f32::from(b.origin.y - h.bounds().origin.y) + scrolled;
                        let (old_top, old_h, card) = s1.get();
                        let now = (top, f32::from(b.size.height), card);
                        s1.set(now);
                        // Moved since this frame was laid out: once more.
                        if (old_top - now.0).abs() > 0.5 || (old_h - now.1).abs() > 0.5 {
                            cx.notify(view);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .relative()
                    .top(px(shift))
                    .child(self.checklist(p))
                    .child(
                        gpui::canvas(
                            move |b, _, cx| {
                                let (top, col, old) = s2.get();
                                let card = f32::from(b.size.height);
                                s2.set((top, col, card));
                                if (old - card).abs() > 0.5 {
                                    cx.notify(view);
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    ),
            )
    }

    fn scan_screen(&self, p: &Paint, cx: &mut Context<Self>) -> (AnyElement, Div) {
        let scanning = self.result.is_none() && self.scan_error.is_none();
        let done = STEPS
            .iter()
            .filter(|(s, _)| self.steps.get(*s).is_some_and(|(st, _)| *st != "running"))
            .count();
        let (title, lede) = match self.mode {
            Mode::Welcome => (
                "Welcome to Pitwall",
                "Pitwall is looking around for your coding agents and projects. It only reads — nothing on your Mac changes unless you tick a box below.",
            ),
            Mode::Rescan => ("Scan again", "Read-only. Pick projects to add and conversations to continue."),
        };
        let mut results = div()
            .flex()
            .flex_col()
            .gap(px(26.))
            .w(p.results_w)
            .flex_none()
            .pb(px(24.));
        match &self.result {
            None if self.scan_error.is_none() => {
                results = results.child(
                    div()
                        .py(px(32.))
                        .child(crate::kit::hint("Results appear here as soon as the scan finishes.", &p.ui.t)),
                );
            }
            None => {}
            Some(r) => {
                let r = r.clone();
                // `.onb-section`: each rises in (`rise` 0.22 s).
                let rise = |id: &'static str, d: Div| enter(id, Fx::ONB_RISE, d);
                results = results
                    .child(rise("onb-sec-projects", self.projects_section(p, &r, cx)))
                    .children(
                        self.conversations_section(p, &r, false, cx)
                            .map(|d| rise("onb-sec-conv", d)),
                    )
                    .children(
                        self.conversations_section(p, &r, true, cx)
                            .map(|d| rise("onb-sec-home", d)),
                    )
                    .children(self.running_section(p, &r, cx).map(|d| rise("onb-sec-running", d)))
                    .children(self.codex_section(p, &r, cx).map(|d| rise("onb-sec-codex", d)));
            }
        }
        let head = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(10.))
            .mb(px(28.))
            .child(p.board([
                ("PIT".into(), false),
                (
                    if scanning {
                        format!("{done}/{}", STEPS.len())
                    } else {
                        "—".into()
                    },
                    false,
                ),
                (if scanning { "SCAN" } else { "BOX" }.into(), scanning),
            ]))
            .child(p.title(title))
            .child(p.lede(lede))
            .when(self.mode == Mode::Rescan, |d| {
                d.child(
                    div().absolute().top_0().right_0().child(
                        crate::kit::glyph_btn("onb-close", "✕", &p.ui.t)
                            .tooltip(tooltip("Close (esc)"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(OnboardingEvent::Close))),
                    ),
                )
            });
        let grid = if p.narrow {
            div().flex().flex_col().gap(px(20.))
        } else {
            div().flex().gap(px(32.))
        }
        .flex_1()
        // A plain wrapper takes the row's stretch; the card keeps its height
        // and sticks 20 px under the top while the results scroll.
        .child(if p.narrow {
            div().flex_none().child(self.checklist(p))
        } else {
            self.sticky_checklist(p, cx.entity_id())
        })
        .child(results);
        let content = div()
            .flex()
            .flex_col()
            .child(head)
            .child(grid)
            .into_any_element();
        (content, self.footer(p, cx))
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ap = appearance(cx).clone();
        let base = ap.theme();
        let t = if self.mode == Mode::Rescan {
            chrome(&base, cx)
        } else {
            base.clone()
        };
        let narrow = window.viewport_size().width < px(860.);
        let p = Paint {
            ui: Ui::new(t.clone()),
            t: t.clone(),
            motion: !ap.reduce_motion,
            narrow,
            results_w: {
                let pad = if narrow { 18. } else { 32. };
                let inner = f32::from(window.viewport_size().width).min(1040.) - 2. * pad;
                px(if narrow { inner } else { inner - 270. - 32. })
            },
        };
        let (content, footer) = if self.access == AccessPhase::Checking {
            // A blank welcome background while the first (instant) check runs.
            (div().into_any_element(), div())
        } else if self.access != AccessPhase::Done {
            self.access_screen(&p, cx)
        } else {
            self.scan_screen(&p, cx)
        };
        let pad = px(if narrow { 18. } else { 32. });
        let bg = if self.mode == Mode::Rescan {
            base.bg.into()
        } else if ap.glass.is_some() {
            crate::theme::backdrop(cx)
        } else {
            linear_gradient(
                180.,
                linear_color_stop(base.bg.blend(base.surface_3.opacity(0.7)), 0.),
                linear_color_stop(base.bg, 0.35),
            )
        };
        // The footer stays at the bottom (`.onb-foot` is sticky), over a
        // fade into the background.
        let page_bg = base.bg;
        // Debug builds: `PITWALL_DEBUG_ONB_SCROLL=<px>` scrolls the results
        // once they are in (screenshots of the lower sections).
        if cfg!(debug_assertions) && self.result.is_some() {
            if let Some(y) = std::env::var("PITWALL_DEBUG_ONB_SCROLL")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
            {
                self.scroll.set_offset(gpui::point(px(0.), px(-y)));
            }
        }
        let root = div()
            .id("onboarding")
            .absolute()
            .inset_0()
            .occlude()
            .bg(bg)
            .text_size(px(13.))
            .text_color(t.text)
            .flex()
            .flex_col()
            .child(
                div()
                    .id("onb-scroll")
                    .track_scroll(&self.scroll)
                    // The sticky checklist follows the scroll.
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(1040.))
                            .px(pad)
                            .pt(px(if narrow { 28. } else { 40. }))
                            .pb(px(24.))
                            .child(content),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(page_bg.opacity(0.), 0.),
                        linear_color_stop(page_bg, 0.3),
                    ))
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(1040.))
                            .px(pad)
                            .pt(px(14.))
                            .pb(px(18.))
                            .child(footer),
                    ),
            );
        // `.onb`: fades in over 0.2 s.
        enter("onboarding-in", Fx::ONB_FADE, root)
    }
}
