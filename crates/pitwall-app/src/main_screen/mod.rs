//! The main screen: everything around the terminals (docs/spec/gpui
//! inventory §2–§6, §10 Changes, §11, §16 strip, §18 New agent / Remove,
//! §23). A port of the React `App.tsx` shell and its components:
//!
//! - top bar ([`topbar`]): sidebar toggle, wordmark, space tabs, search,
//!   live counts, Review / Wall / Settings / details buttons;
//! - sidebar ([`sidebar`]): projects and agents, needs-you first, context
//!   menus, keyboard selection, icon rail on narrow windows;
//! - space view ([`space`]): the active space's split tree of panes, presets,
//!   density, maximise, divider drags, drag and drop, chip strip;
//! - right panel ([`panel`]): the focused agent's Changes with its where
//!   line and the per-file diff;
//! - dialogs ([`dialogs`]): New agent, New terminal, Remove agent, diff;
//! - status strip and toasts ([`strip`]).
//!
//! Hooks for the other screens: [`Route`] (Space, Wall, Review, Explorer)
//! with [`register_route`] for their views, [`TerminalSlot`] for the
//! terminal element, and [`ScreenEvent`]s (palette, move to window, …).

#[doc(hidden)]
pub mod demo;
mod dialogs;
pub mod diff;
mod menu;
pub mod model;
mod panel;
mod sidebar;
mod space;
mod strip;
mod topbar;
pub mod tree;
pub mod workspace;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, AnyElement, AnyView, App, Context, Entity, EventEmitter,
    FocusHandle, Global, IntoElement, KeyBinding, Pixels, Render, SharedString, Size, Subscription,
    Task, WeakEntity, Window,
};

use pitwall_core::onboarding::project_list::Project;
use pitwall_core::Shared;
use pitwall_proto::{AgentView, Status};

use crate::agents::{group_by_project, AgentStore};
use crate::menu::Dismiss;
use crate::theme::{self, TOPBAR_H};

use self::dialogs::{DiffDialog, NewAgentDialog, RemoveState, TerminalDialog};
use self::menu::CtxMenu;
use self::panel::ChangesState;
use self::strip::Toast;
use crate::kit::TextInput;
use self::tree::Preset;
use self::workspace::{UiState, ALL_SPACE, MAIN};

actions!(
    main_screen,
    [
        /// ⌘K: the command palette (another module answers [`ScreenEvent::OpenPalette`]).
        OpenPalette,
        /// ⌘N
        NewAgent,
        /// ⌘T: a terminal where you are.
        NewTerminal,
        /// ⌘⇧T: a terminal in a folder you choose.
        NewTerminalAt,
        /// ⌘J
        JumpNextBlocked,
        /// ⌘B
        ToggleSidebar,
        /// ⌘.
        ToggleDetails,
        /// ⌘E
        ToggleWall,
        /// ⌘R
        ToggleReview,
        /// ⌘⏎
        ToggleMaximize,
        /// ⌘⇧R: source-control views refresh now.
        RefreshChanges,
        /// ⌘⇧N
        MoveSpaceToWindow,
        SelectAgent1,
        SelectAgent2,
        SelectAgent3,
        SelectAgent4,
        SelectAgent5,
        SelectAgent6,
        SelectAgent7,
        SelectAgent8,
        SelectAgent9,
        /// ↑ / ↓ / ⏎ in the sidebar and in menus.
        SelectPrev,
        SelectNext,
        Confirm,
    ]
);

/// What the centre shows (besides the empty state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    /// The active space's panes.
    Space,
    Wall,
    Review,
    /// The read-only file viewer.
    Explorer,
}

/// What another screen's view gets to build itself.
#[derive(Clone)]
pub struct RouteContext {
    pub store: Entity<AgentStore>,
    pub screen: WeakEntity<MainScreen>,
}

type RouteBuilder = Arc<dyn Fn(RouteContext, &mut Window, &mut App) -> AnyView>;

/// Views registered for the routes this module doesn't draw.
#[derive(Default)]
pub struct RouteViews(HashMap<Route, RouteBuilder>);

impl Global for RouteViews {}

/// Hook for the Wall, Review and Explorer modules: the view the centre shows
/// on `route` (built once per window, when first shown).
pub fn register_route(
    cx: &mut App,
    route: Route,
    build: impl Fn(RouteContext, &mut Window, &mut App) -> AnyView + 'static,
) {
    cx.default_global::<RouteViews>()
        .0
        .insert(route, Arc::new(build));
}

type PanelBuilder = Arc<dyn Fn(&AgentView, &mut Window, &mut App) -> AnyView>;

/// The right panel's Files tab (the explorer module's file tree).
#[derive(Clone)]
pub struct FilesPanel(pub PanelBuilder);

impl Global for FilesPanel {}

/// Hook: the view under the right panel's Files tab for an agent. The tab
/// shows only once this is registered (and the agent has `caps.explorer`).
pub fn register_files_panel(
    cx: &mut App,
    build: impl Fn(&AgentView, &mut Window, &mut App) -> AnyView + 'static,
) {
    cx.set_global(FilesPanel(Arc::new(build)));
}

/// Where another module's section goes in the right panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelSlot {
    /// Above Changes (Next up: queue and prompt box).
    Top,
    /// Below Changes (Last sent).
    Bottom,
}

#[derive(Default)]
pub struct PanelSections(HashMap<PanelSlot, PanelBuilder>);

impl Global for PanelSections {}

/// Hook: a right-panel section for the focused agent (Next up, Last sent).
pub fn register_panel_section(
    cx: &mut App,
    slot: PanelSlot,
    build: impl Fn(&AgentView, &mut Window, &mut App) -> AnyView + 'static,
) {
    cx.default_global::<PanelSections>()
        .0
        .insert(slot, Arc::new(build));
}

/// The hosted engine, for the calls the main screen makes (set by `run()`).
pub struct EngineHandle(pub Shared);

impl Global for EngineHandle {}

/// What a pane hands the terminal element.
pub struct TerminalArgs<'a> {
    pub agent: &'a AgentView,
    /// The pane has focus (the terminal should take keyboard focus).
    pub focused: bool,
    /// Base terminal font size (Settings) and this tile's override.
    pub font_size: f64,
}

type TerminalBuilder = Arc<dyn Fn(&TerminalArgs, &mut Window, &mut App) -> AnyElement>;

/// The seam where `pitwall-term-view` plugs in: one function from an agent
/// to the element drawn in its pane body. Until it is set, a placeholder.
#[derive(Clone)]
pub struct TerminalSlot(pub TerminalBuilder);

impl Global for TerminalSlot {}

/// Set the terminal element (`pitwall-term-view`).
pub fn set_terminal_slot(
    cx: &mut App,
    build: impl Fn(&TerminalArgs, &mut Window, &mut App) -> AnyElement + 'static,
) {
    cx.set_global(TerminalSlot(Arc::new(build)));
}

/// What the screen tells the rest of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenEvent {
    /// ⌘K or the Search button.
    OpenPalette,
    RouteChanged(Route),
    /// An agent was shown (sidebar, ⌘1–9, strip, …).
    AgentShown(String),
    /// "Move to new window" on a space (multi-window answers it).
    MoveSpaceToWindow(String),
}

impl EventEmitter<ScreenEvent> for MainScreen {}

/// The app's key bindings for the main screen (`HostInfo::shortcuts`: ⌘ on
/// macOS, Ctrl+Shift elsewhere, so plain Ctrl stays the terminal's).
pub fn bindings() -> Vec<KeyBinding> {
    let mac = cfg!(target_os = "macos");
    let (m, ms) = if mac {
        ("cmd-", "cmd-shift-")
    } else {
        ("ctrl-shift-", "ctrl-shift-alt-")
    };
    let p = |key: &str| format!("{m}{key}");
    let s = |key: &str| format!("{ms}{key}");
    let mut b = vec![
        KeyBinding::new(&p("k"), OpenPalette, None),
        KeyBinding::new(&p("n"), NewAgent, None),
        KeyBinding::new(&p("t"), NewTerminal, None),
        KeyBinding::new(&s("t"), NewTerminalAt, None),
        KeyBinding::new(&p("j"), JumpNextBlocked, None),
        KeyBinding::new(&p("b"), ToggleSidebar, None),
        KeyBinding::new(&p("."), ToggleDetails, None),
        KeyBinding::new(&p("e"), ToggleWall, None),
        KeyBinding::new(&p("r"), ToggleReview, None),
        KeyBinding::new(&s("r"), RefreshChanges, None),
        KeyBinding::new(&s("n"), MoveSpaceToWindow, None),
        KeyBinding::new(&p("enter"), ToggleMaximize, None),
        KeyBinding::new(&p("1"), SelectAgent1, None),
        KeyBinding::new(&p("2"), SelectAgent2, None),
        KeyBinding::new(&p("3"), SelectAgent3, None),
        KeyBinding::new(&p("4"), SelectAgent4, None),
        KeyBinding::new(&p("5"), SelectAgent5, None),
        KeyBinding::new(&p("6"), SelectAgent6, None),
        KeyBinding::new(&p("7"), SelectAgent7, None),
        KeyBinding::new(&p("8"), SelectAgent8, None),
        KeyBinding::new(&p("9"), SelectAgent9, None),
    ];
    for ctx in ["PwSidebar", "PwMenu"] {
        b.extend([
            KeyBinding::new("up", SelectPrev, Some(ctx)),
            KeyBinding::new("down", SelectNext, Some(ctx)),
            KeyBinding::new("enter", Confirm, Some(ctx)),
        ]);
    }
    b
}

/// Register the key bindings (call once at start, after `kit::init`).
pub fn init(cx: &mut App) {
    cx.bind_keys(bindings());
}

/// Window width classes (`useBreakpoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    Xl,
    Lg,
    Md,
    Sm,
    Xs,
}

impl Breakpoint {
    pub fn of(width: f32) -> Breakpoint {
        match width {
            w if w >= 2200. => Breakpoint::Xl,
            w if w >= 1500. => Breakpoint::Lg,
            w if w >= 1100. => Breakpoint::Md,
            w if w >= 700. => Breakpoint::Sm,
            _ => Breakpoint::Xs,
        }
    }
    /// Full sidebar (collapsible); else an icon rail (sm) or hidden (xs).
    pub fn full_sidebar(self) -> bool {
        matches!(self, Breakpoint::Xl | Breakpoint::Lg | Breakpoint::Md)
    }
    pub fn right_docked(self) -> bool {
        matches!(self, Breakpoint::Xl | Breakpoint::Lg)
    }
    pub fn compact(self) -> bool {
        matches!(self, Breakpoint::Sm | Breakpoint::Xs)
    }
    /// Columns the auto grid prefers.
    pub fn max_per_row(self) -> usize {
        match self {
            Breakpoint::Xl => 4,
            Breakpoint::Lg => 3,
            Breakpoint::Md => 2,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    Full,
    Rail,
    Hidden,
}

enum Modal {
    NewAgent(Entity<NewAgentDialog>),
    Terminal(Entity<TerminalDialog>),
    Remove(RemoveState),
    Diff(Entity<DiffDialog>),
}

/// A divider being dragged: the split, which divider, and the sizes so far.
#[derive(Debug, Clone)]
struct DividerDrag {
    split: String,
    index: usize,
    row: bool,
    start: Pixels,
    total: f64,
    initial: Vec<f64>,
    live: Vec<f64>,
}

pub struct MainScreen {
    store: Entity<AgentStore>,
    engine: Option<Shared>,
    root: Option<PathBuf>,
    label: String,
    pub ui: UiState,
    active: String,
    route: Route,
    sidebar_collapsed: bool,
    right_pinned: bool,
    overlay_sidebar: bool,
    drawer_open: bool,
    /// The right panel shows Files instead of Changes.
    files_tab: bool,
    modal: Option<Modal>,
    menu: Option<CtxMenu>,
    rename: Option<(String, Entity<TextInput>)>,
    projects: Vec<Project>,
    changes: ChangesState,
    toasts: Vec<Toast>,
    focus: FocusHandle,
    sidebar_focus: FocusHandle,
    /// The space area's size, measured each frame.
    area: Option<Size<Pixels>>,
    divider: Option<DividerDrag>,
    /// The pane and zone an agent is dragged over (`None` zone: centre).
    drop_hint: Option<(String, Option<tree::Side>)>,
    /// The sidebar's keyboard cursor.
    cursor: Option<String>,
    route_views: HashMap<Route, AnyView>,
    save_task: Option<Task<()>>,
    seen_task: Option<Task<()>>,
    last_selected: Option<String>,
    agent_ids: Vec<String>,
    _subs: Vec<Subscription>,
}

impl MainScreen {
    pub fn new(
        store: Entity<AgentStore>,
        root: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MainScreen {
        let engine = cx.try_global::<EngineHandle>().map(|e| e.0.clone());
        let ui = root.as_deref().map(workspace::load).unwrap_or_default();
        let projects = engine
            .as_ref()
            .map(|e| e.projects().list())
            .unwrap_or_default();
        let focus = cx.focus_handle();
        window.focus(&focus);
        let subs = vec![
            cx.observe(&store, |this, _, cx| this.agents_changed(cx)),
            cx.subscribe(&store, |this, _, e: &crate::agents::StoreEvent, cx| {
                this.attention(e, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                // Coming back counts as looking at the agent you left selected.
                if window.is_window_active() && this.route == Route::Space {
                    if let Some(a) = this.selected(cx).filter(|a| a.status == Status::Done) {
                        this.mark_seen(&a.id, cx);
                    }
                }
            }),
        ];
        let mut screen = MainScreen {
            store,
            engine,
            root,
            label: MAIN.into(),
            active: ALL_SPACE.into(),
            ui,
            route: Route::Space,
            sidebar_collapsed: false,
            right_pinned: true,
            overlay_sidebar: false,
            drawer_open: false,
            files_tab: false,
            modal: None,
            menu: None,
            rename: None,
            projects,
            changes: ChangesState::default(),
            toasts: vec![],
            focus,
            sidebar_focus: cx.focus_handle(),
            area: None,
            divider: None,
            drop_hint: None,
            cursor: None,
            route_views: HashMap::new(),
            save_task: None,
            seen_task: None,
            last_selected: None,
            agent_ids: vec![],
            _subs: subs,
        };
        if screen.ui.wall.iter().any(|w| w == &screen.label) {
            screen.route = Route::Wall;
        }
        screen.agents_changed(cx);
        screen
    }

    // ── queries ──────────────────────────────────────────────────────────

    pub fn route(&self) -> Route {
        self.route
    }

    pub fn store(&self) -> &Entity<AgentStore> {
        &self.store
    }

    fn agents<'a>(&self, cx: &'a App) -> &'a [AgentView] {
        &self.store.read(cx).agents
    }

    /// Agents in sidebar order (⌘1–9, ⌘J, the strip).
    fn ordered(&self, cx: &App) -> Vec<AgentView> {
        group_by_project(self.agents(cx))
            .into_iter()
            .flat_map(|g| g.agents)
            .collect()
    }

    fn active_space(&self) -> Option<&workspace::Space> {
        let mine = self.ui.spaces_of(&self.label);
        mine.iter()
            .find(|s| s.id == self.active)
            .or(mine.first())
            .copied()
    }

    /// The agent in the active space's focused pane.
    fn focused_agent_id(&self) -> Option<String> {
        self.active_space().and_then(|s| s.focused_agent())
    }

    pub fn selected(&self, cx: &App) -> Option<AgentView> {
        let id = self.focused_agent_id()?;
        self.agents(cx).iter().find(|a| a.id == id).cloned()
    }

    fn breakpoint(window: &Window) -> Breakpoint {
        Breakpoint::of(window.viewport_size().width.into())
    }

    fn sidebar_mode(&self, bp: Breakpoint) -> SidebarMode {
        if bp.full_sidebar() {
            if self.sidebar_collapsed {
                SidebarMode::Hidden
            } else {
                SidebarMode::Full
            }
        } else if bp == Breakpoint::Sm {
            SidebarMode::Rail
        } else {
            SidebarMode::Hidden
        }
    }

    // ── state changes ────────────────────────────────────────────────────

    /// Change `ui.json` and save it 150 ms after the last change.
    fn update_ui(&mut self, f: impl FnOnce(UiState) -> UiState, cx: &mut Context<Self>) {
        let next = f(self.ui.clone());
        if next == self.ui {
            return;
        }
        self.ui = next;
        cx.notify();
        let Some(root) = self.root.clone() else {
            return;
        };
        let state = self.ui.clone();
        self.save_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let write = cx
                .background_executor()
                .spawn(async move { workspace::save(&root, &state) });
            if let Err(e) = write.await {
                eprintln!("pitwall: could not save ui.json: {e}");
            }
        }));
    }

    fn with_active(&mut self, f: impl FnOnce(UiState, &str) -> UiState, cx: &mut Context<Self>) {
        let Some(id) = self.active_space().map(|s| s.id.clone()) else {
            return;
        };
        self.update_ui(|s| f(s, &id), cx);
    }

    fn set_route(&mut self, route: Route, cx: &mut Context<Self>) {
        if self.route == route {
            return;
        }
        self.route = route;
        let label = self.label.clone();
        self.update_ui(|s| s.set_wall(&label, route == Route::Wall), cx);
        cx.emit(ScreenEvent::RouteChanged(route));
        cx.notify();
    }

    /// Engine views changed: prune `ui.json`, place the first agent on a
    /// first run, follow the focused agent's state.
    fn agents_changed(&mut self, cx: &mut Context<Self>) {
        let agents = self.agents(cx).to_vec();
        let ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
        if ids != self.agent_ids {
            self.agent_ids = ids.clone();
            if let Some(e) = &self.engine {
                self.projects = e.projects().list();
            }
            self.update_ui(|s| s.prune_agents(&ids), cx);
            // First run: the first agent goes to the All space (main window only).
            if self.label == MAIN
                && !agents.is_empty()
                && agents.iter().all(|a| self.ui.locate(&a.id).is_none())
            {
                if let Some(first) = self.ordered(cx).first() {
                    let id = first.id.clone();
                    self.update_ui(|s| s.place_agent(ALL_SPACE, &id, None, None), cx);
                }
            }
        }
        self.follow_selected(cx);
        self.changes_follow(cx);
        cx.notify();
    }

    /// "Done" means unseen: selecting a done agent is looking at it; one
    /// finishing while you're on it is seen after a 1.5 s glance.
    fn follow_selected(&mut self, cx: &mut Context<Self>) {
        let sel = self.selected(cx);
        let id = sel.as_ref().map(|a| a.id.clone());
        let just = id != self.last_selected;
        self.last_selected = id.clone();
        let Some(a) = sel.filter(|a| a.status == Status::Done && self.route != Route::Wall) else {
            self.seen_task = None;
            return;
        };
        if just {
            self.mark_seen(&a.id, cx);
            return;
        }
        let agent = a.id.clone();
        self.seen_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.focused_agent_id().as_deref() == Some(agent.as_str()) {
                    s.mark_seen(&agent, cx);
                }
            });
        }));
    }

    fn mark_seen(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(e) = self.engine.clone() {
            let id = id.to_string();
            cx.background_executor()
                .spawn(async move {
                    let _ = e.mark_seen(&id);
                })
                .detach();
        }
    }

    /// Show agent: leave Wall/Review/viewer, close the overlay sidebar,
    /// focus its pane here, or place it in the active space.
    pub fn show_agent(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_route(Route::Space, cx);
        self.overlay_sidebar = false;
        self.cursor = Some(id.to_string());
        match self.ui.locate(id) {
            Some(loc) if loc.window == self.label => {
                self.active = loc.space_id.clone();
                let hidden = self.folded_panes().contains(&loc.pane_id);
                self.update_ui(
                    |s| {
                        let max = s
                            .space(&loc.space_id)
                            .and_then(|sp| sp.maximized_pane_id.clone());
                        let mut s = s;
                        if max.as_deref().is_some_and(|m| m != loc.pane_id) {
                            s = s.toggle_maximize(&loc.space_id, max.as_deref());
                        }
                        let focused = s
                            .space(&loc.space_id)
                            .and_then(|sp| sp.focused_pane_id.clone());
                        match focused {
                            // Folded away for lack of room: swap it into the focused pane.
                            Some(f) if hidden => s.place_agent(&loc.space_id, id, Some(&f), None),
                            _ => s.focus_pane(&loc.space_id, &loc.pane_id),
                        }
                    },
                    cx,
                );
            }
            Some(_) => {
                // Shown in another window: that window focuses it (multi-window).
                cx.emit(ScreenEvent::AgentShown(id.to_string()));
            }
            None => {
                let Some(space) = self.active_space().map(|s| s.id.clone()) else {
                    return;
                };
                self.active = space.clone();
                self.update_ui(|s| s.place_agent(&space, id, None, None), cx);
            }
        }
        if self
            .agents(cx)
            .iter()
            .any(|a| a.id == id && a.status == Status::Done)
        {
            self.mark_seen(id, cx);
        }
        self.last_selected = Some(id.to_string());
        self.changes_follow(cx);
        window.focus(&self.focus);
        cx.emit(ScreenEvent::AgentShown(id.to_string()));
        cx.notify();
    }

    fn next_blocked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blocked: Vec<String> = self
            .ordered(cx)
            .into_iter()
            .filter(|a| a.status == Status::Blocked)
            .map(|a| a.id)
            .collect();
        if blocked.is_empty() {
            return;
        }
        let at = self
            .focused_agent_id()
            .and_then(|f| blocked.iter().position(|b| *b == f));
        let next = blocked[at.map(|i| (i + 1) % blocked.len()).unwrap_or(0)].clone();
        self.show_agent(&next, window, cx);
    }

    fn select_index(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = self.ordered(cx).get(i) {
            let id = a.id.clone();
            self.show_agent(&id, window, cx);
        }
    }

    /// Panes folded away in the active space at its current size.
    fn folded_panes(&self) -> Vec<String> {
        let (Some(sp), Some(area)) = (self.active_space(), self.area) else {
            return vec![];
        };
        let min = workspace::fold_min(self.ui.density_of(sp), self.ui.font_size);
        let base = sp
            .maximized_pane_id
            .as_deref()
            .and_then(|m| sp.layout.find_node(m).cloned())
            .unwrap_or(sp.layout.clone());
        tree::fit_layout(
            &base,
            (f32::from(area.width) as f64, f32::from(area.height) as f64),
            min,
            sp.focused_pane_id.as_deref(),
        )
        .1
        .into_iter()
        .map(|p| p.id)
        .collect()
    }

    fn apply_preset(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        let agents = self.agents(cx).to_vec();
        let fit = self.fit();
        let per_row = Self::breakpoint(window).max_per_row();
        self.with_active(
            |s, id| {
                s.apply_preset(id, preset, &agents, |n| match fit {
                    Some(f) => tree::auto_grid(n, f.area, f.min, per_row),
                    None => vec![n.max(1)],
                })
            },
            cx,
        );
    }

    /// The active space's area and tile minimum (for drops and Auto grid).
    fn fit(&self) -> Option<workspace::Fit> {
        let (sp, area) = (self.active_space()?, self.area?);
        Some(workspace::Fit {
            area: (f32::from(area.width) as f64, f32::from(area.height) as f64),
            min: workspace::fold_min(self.ui.density_of(sp), self.ui.font_size),
        })
    }

    /// The size a new agent starts at: the pane it will show in.
    fn new_agent_size(&self) -> (Option<u16>, Option<u16>) {
        let (Some(sp), Some(area)) = (self.active_space(), self.area) else {
            return (None, None);
        };
        let rects = tree::pane_rects(
            &sp.layout,
            tree::Rect {
                x: 0.,
                y: 0.,
                w: f32::from(area.width) as f64,
                h: f32::from(area.height) as f64,
            },
        );
        let target = sp
            .layout
            .leaves()
            .into_iter()
            .find(|p| p.agent_id.is_none())
            .map(|p| p.id)
            .or(sp.focused_pane_id.clone());
        let Some(r) = rects
            .into_iter()
            .find(|(id, _)| Some(id) == target.as_ref())
            .map(|x| x.1)
        else {
            return (None, None);
        };
        let font = self.ui.font_size;
        let cols = ((r.w - 28.) / (font * 0.6)).floor();
        let rows = ((r.h - 44.) / (font * 1.32)).floor();
        if cols < 20. || rows < 5. {
            return (None, None);
        }
        (Some(cols as u16), Some(rows as u16))
    }

    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Self::breakpoint(window).full_sidebar() {
            self.sidebar_collapsed = !self.sidebar_collapsed;
        } else {
            self.overlay_sidebar = !self.overlay_sidebar;
        }
        cx.notify();
    }

    fn toggle_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Self::breakpoint(window).right_docked() {
            self.right_pinned = !self.right_pinned;
        } else {
            self.drawer_open = !self.drawer_open;
        }
        cx.notify();
    }

    fn toggle_route(&mut self, route: Route, cx: &mut Context<Self>) {
        let next = if self.route == route {
            Route::Space
        } else {
            route
        };
        self.set_route(next, cx);
    }

    pub fn toast(&mut self, toast: Toast, cx: &mut Context<Self>) {
        strip::push(self, toast, cx);
    }

    /// A failed engine call (React: `run(…, what)` → "Couldn't <what>").
    fn failed(&mut self, what: &str, err: String, cx: &mut Context<Self>) {
        self.toast(Toast::error(format!("Couldn't {what}"), err), cx);
    }

    fn attention(&mut self, e: &crate::agents::StoreEvent, cx: &mut Context<Self>) {
        let crate::agents::StoreEvent::Attention {
            agent_id,
            name,
            reason,
            ..
        } = e
        else {
            return;
        };
        if *reason == "done" && self.focused_agent_id().as_deref() == Some(agent_id) {
            return;
        }
        let toast = if *reason == "blocked" {
            Toast::blocked(format!("{name} needs you"), agent_id.clone())
        } else {
            Toast::done(format!("{name} is done"), agent_id.clone())
        };
        self.toast(toast, cx);
    }

    /// ⌘T: a terminal in the focused agent's folder, else the project
    /// space's project, else home (`terminalFolder`).
    fn here(&self, cx: &App) -> String {
        if let Some(a) = self.selected(cx).filter(|a| !a.cwd.is_empty()) {
            return a.cwd;
        }
        if let Some(p) = self
            .active_space()
            .filter(|s| s.kind == workspace::SpaceKind::Project)
            .and_then(|s| s.project.clone())
        {
            return p;
        }
        "~".into()
    }

    /// Start a plain terminal in `path` and show it.
    fn open_terminal(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let taken: Vec<String> = self.agents(cx).iter().map(|a| a.name.clone()).collect();
        let (cols, rows) = self.new_agent_size();
        let req = pitwall_core::model::CreateAgentRequest {
            name: model::terminal_name(&path, &taken),
            kind: "shell".into(),
            project_path: if path.trim().is_empty() {
                "~".into()
            } else {
                path
            },
            cols,
            rows,
            ..Default::default()
        };
        let task = cx.background_executor().spawn(async move {
            let view = pitwall_core::engine::lifecycle::create(&engine, req)?;
            pitwall_core::onboarding::add_agent_project(&engine, &view);
            Ok::<_, String>(view)
        });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update_in(cx, |s, window, cx| match res {
                Ok(view) => s.created(view, window, cx),
                Err(e) => s.failed("open a terminal", e, cx),
            });
        })
        .detach();
    }

    /// A new agent exists: show it as soon as the store has it.
    fn created(&mut self, view: AgentView, window: &mut Window, cx: &mut Context<Self>) {
        let id = view.id.clone();
        self.store.update(cx, |s, cx| {
            if !s.agents.iter().any(|a| a.id == id) {
                s.agents.push(view);
                cx.notify();
            }
        });
        self.show_agent(&id, window, cx);
    }

    fn open_new_agent(
        &mut self,
        project: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self.agents(cx).to_vec();
        let size = self.new_agent_size();
        let engine = self.engine.clone();
        let dialog = cx.new(|cx| NewAgentDialog::new(engine, existing, project, size, window, cx));
        self._subs.push(cx.subscribe_in(
            &dialog,
            window,
            |this, _, e: &dialogs::DialogEvent, window, cx| {
                this.dialog_event(e.clone(), window, cx)
            },
        ));
        self.modal = Some(Modal::NewAgent(dialog));
        self.menu = None;
        cx.notify();
    }

    fn open_terminal_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.here(cx);
        let mut folders: Vec<String> = vec![];
        for p in self
            .projects
            .iter()
            .map(|p| p.path.clone())
            .chain(self.agents(cx).iter().map(|a| a.cwd.clone()))
        {
            if !folders.contains(&p) {
                folders.push(p);
            }
        }
        let dialog = cx.new(|cx| TerminalDialog::new(here, folders, window, cx));
        self._subs.push(cx.subscribe_in(
            &dialog,
            window,
            |this, _, e: &dialogs::DialogEvent, window, cx| {
                this.dialog_event(e.clone(), window, cx)
            },
        ));
        self.modal = Some(Modal::Terminal(dialog));
        cx.notify();
    }

    fn open_remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Remove(RemoveState::new(id)));
        self.menu = None;
        cx.notify();
    }

    fn open_diff(
        &mut self,
        agent: AgentView,
        file: pitwall_core::vcs::git::FileChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let engine = self.engine.clone();
        let dialog = cx.new(|cx| DiffDialog::new(engine, agent, file, window, cx));
        self._subs.push(cx.subscribe_in(
            &dialog,
            window,
            |this, _, e: &dialogs::DialogEvent, window, cx| {
                this.dialog_event(e.clone(), window, cx)
            },
        ));
        self.modal = Some(Modal::Diff(dialog));
        cx.notify();
    }

    fn dialog_event(
        &mut self,
        e: dialogs::DialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match e {
            dialogs::DialogEvent::Close => self.close_modal(window, cx),
            dialogs::DialogEvent::Created(view) => {
                self.close_modal(window, cx);
                self.created(*view, window, cx);
            }
            dialogs::DialogEvent::OpenTerminal(path) => {
                self.close_modal(window, cx);
                self.open_terminal(path, window, cx);
            }
        }
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn new_space(&mut self, cx: &mut Context<Self>) {
        let label = self.label.clone();
        let mut created = String::new();
        self.update_ui(
            |s| {
                let (s, id) = s.create_custom_space(&label);
                created = id;
                s.set_wall(&label, false)
            },
            cx,
        );
        self.active = created;
        self.set_route(Route::Space, cx);
    }

    fn open_project_space(&mut self, project: &str, display: &str, cx: &mut Context<Self>) {
        let agents = self.agents(cx).to_vec();
        let label = self.label.clone();
        let mut opened = String::new();
        self.update_ui(
            |s| {
                let (s, id) = s.open_project_space(project, display, &agents, &label);
                opened = id;
                s
            },
            cx,
        );
        if self.ui.window_of_space(&opened) == self.label {
            self.active = opened;
        }
        self.overlay_sidebar = false;
        self.set_route(Route::Space, cx);
        cx.notify();
    }

    fn move_space(&mut self, id: String, cx: &mut Context<Self>) {
        if id == ALL_SPACE {
            self.toast(
                Toast::info("The All space stays in the main window", None),
                cx,
            );
            return;
        }
        cx.emit(ScreenEvent::MoveSpaceToWindow(id));
    }

    fn stop_agent(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let task = cx
            .background_executor()
            .spawn(async move { pitwall_core::engine::lifecycle::stop(&engine, &id) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |s, cx| s.failed("stop agent", e, cx));
            }
        })
        .detach();
    }

    fn restart_agent(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let size = self.new_agent_size();
        let size = size.0.zip(size.1);
        let task = cx
            .background_executor()
            .spawn(async move { pitwall_core::engine::lifecycle::restart(&engine, &id, size) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |s, cx| s.failed("restart", e, cx));
            }
        })
        .detach();
    }

    fn remove_project(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let task = cx
            .background_executor()
            .spawn(async move { pitwall_core::onboarding::remove_project(&engine, &path) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |s, cx| match res {
                Ok(list) => {
                    s.projects = list;
                    cx.notify();
                }
                Err(e) => s.failed("remove the project", e, cx),
            });
        })
        .detach();
    }

    fn route_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyView> {
        if let Some(v) = self.route_views.get(&self.route) {
            return Some(v.clone());
        }
        let build = cx.try_global::<RouteViews>()?.0.get(&self.route)?.clone();
        let ctx = RouteContext {
            store: self.store.clone(),
            screen: cx.entity().downgrade(),
        };
        let view = build(ctx, window, cx);
        self.route_views.insert(self.route, view.clone());
        Some(view)
    }

    // ── actions ──────────────────────────────────────────────────────────

    fn on_dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.menu = None;
        } else if self.modal.is_some() {
            self.close_modal(window, cx);
        } else if self.rename.is_some() {
            self.rename = None;
        } else if self.overlay_sidebar || self.drawer_open {
            self.overlay_sidebar = false;
            self.drawer_open = false;
        } else if self.route != Route::Space {
            self.set_route(Route::Space, cx);
        } else {
            cx.propagate();
            return;
        }
        cx.notify();
    }
}

impl Render for MainScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let bp = Self::breakpoint(window);
        // Drawers close when the window grows past their breakpoint.
        if bp.right_docked() {
            self.drawer_open = false;
        }
        if bp.full_sidebar() {
            self.overlay_sidebar = false;
        }
        let agents = self.agents(cx).to_vec();
        let selected = self.selected(cx);
        let sidebar_mode = self.sidebar_mode(bp);
        let right = match (&selected, self.route) {
            (Some(_), Route::Space) if bp.right_docked() && self.right_pinned => Some(false),
            (Some(_), Route::Space) if !bp.right_docked() && self.drawer_open => Some(true),
            _ => None,
        };

        let main_row = div()
            .flex_1()
            .min_h_0()
            .flex()
            .relative()
            .when(sidebar_mode != SidebarMode::Hidden, |d| {
                d.child(self.render_sidebar(sidebar_mode, &agents, &t, window, cx))
            })
            .child(self.render_center(&agents, bp, &t, window, cx))
            .when(right == Some(false), |d| {
                d.children(
                    selected
                        .clone()
                        .map(|a| self.render_panel(&a, false, &t, window, cx)),
                )
            })
            .when(
                self.overlay_sidebar && sidebar_mode != SidebarMode::Full,
                |d| {
                    d.child(
                        div()
                            .id("sidebar-scrim")
                            .absolute()
                            .inset_0()
                            .bg(t.scrim())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.overlay_sidebar = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .shadow_lg()
                            .child(self.render_sidebar(SidebarMode::Full, &agents, &t, window, cx)),
                    )
                },
            )
            .when(right == Some(true), |d| {
                d.child(
                    div()
                        .id("drawer-scrim")
                        .absolute()
                        .inset_0()
                        .bg(t.scrim())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.drawer_open = false;
                            cx.notify();
                        })),
                )
                .children(selected.clone().map(|a| {
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .shadow_lg()
                        .child(self.render_panel(&a, true, &t, window, cx))
                }))
            });

        div()
            .id("main-screen")
            .key_context("MainScreen")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(|_, _: &OpenPalette, _, cx| cx.emit(ScreenEvent::OpenPalette)))
            .on_action(
                cx.listener(|this, _: &NewAgent, window, cx| this.open_new_agent(None, window, cx)),
            )
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
                let here = this.here(cx);
                this.open_terminal(here, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewTerminalAt, window, cx| {
                this.open_terminal_dialog(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &JumpNextBlocked, window, cx| this.next_blocked(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleDetails, window, cx| this.toggle_details(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleWall, _, cx| this.toggle_route(Route::Wall, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleReview, _, cx| this.toggle_route(Route::Review, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleMaximize, _, cx| {
                this.with_active(|s, id| s.toggle_maximize(id, None), cx)
            }))
            .on_action(cx.listener(|this, _: &RefreshChanges, _, cx| this.load_changes(true, cx)))
            .on_action(cx.listener(|this, _: &MoveSpaceToWindow, _, cx| {
                if let Some(id) = this.active_space().map(|s| s.id.clone()) {
                    this.move_space(id, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAgent1, w, cx| this.select_index(0, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent2, w, cx| this.select_index(1, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent3, w, cx| this.select_index(2, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent4, w, cx| this.select_index(3, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent5, w, cx| this.select_index(4, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent6, w, cx| this.select_index(5, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent7, w, cx| this.select_index(6, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent8, w, cx| this.select_index(7, w, cx)))
            .on_action(cx.listener(|this, _: &SelectAgent9, w, cx| this.select_index(8, w, cx)))
            .on_mouse_move(cx.listener(Self::divider_move))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::divider_up))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .font_family(crate::kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(1.45))
            .child(self.render_topbar(&agents, bp, &t, window, cx))
            .child(main_row)
            .child(self.render_strip(&t, bp, cx))
            .child(self.render_toasts(&t, cx))
            .children(self.render_modal(&t, window, cx))
            .children(self.render_menu(&t, cx))
    }
}

/// The top bar's height (re-exported for the other screens).
pub const TOPBAR_HEIGHT: Pixels = TOPBAR_H;

/// The label `MainView` shows the main screen under (one per window later).
pub fn window_label() -> SharedString {
    MAIN.into()
}

#[cfg(test)]
mod tests;
