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

mod commands;
#[doc(hidden)]
pub mod demo;
mod dialogs;
mod elsewhere;
mod engineer;
mod menu;
pub mod model;
mod panel;
mod part;
mod preview;
mod sidebar;
mod space;
mod strip;
mod tile_font;
mod topbar;
pub mod worktrees;
pub mod wt_list;
pub mod tree;
pub mod workspace;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, AnyElement, AnyView, App, Context, Entity, EventEmitter,
    FocusHandle, Global, IntoElement, KeyBinding, Pixels, Point, Render, SharedString, Size,
    Subscription, Task, WeakEntity, Window,
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
pub use self::topbar::{AgentDrag, SpaceDrag, SpaceRelease};
use self::workspace::{UiState, ALL_SPACE, MAIN};
pub use self::commands::CommandState;
pub use self::part::cached as parts_cached;

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
        /// ⌘+ / ⌘− / ⌘0: the focused tile's font (Settings' when no tile).
        FontBigger,
        FontSmaller,
        FontReset,
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

/// Hook for the Wall: the view the centre shows on `route` (built once per
/// window, when first shown). One builder serves every window, so it takes
/// what belongs to a window from its [`RouteContext`], never from what it
/// captured; a view made with a window's own entities is given to that
/// window's screen with [`MainScreen::set_route_view`] instead.
pub fn register_route(
    cx: &mut App,
    route: Route,
    build: impl Fn(RouteContext, &mut Window, &mut App) -> AnyView + 'static,
) {
    cx.default_global::<RouteViews>()
        .0
        .insert(route, Arc::new(build));
}

/// Changed files' rows on screen: where, whose, which.
type FileRows = std::rc::Rc<
    std::cell::RefCell<Vec<(gpui::Bounds<Pixels>, AgentView, pitwall_core::vcs::git::FileChange)>>,
>;
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
    /// The pane is the space's focused one.
    pub focused: bool,
    /// The screen itself holds the keyboard (no dialog, menu, field or
    /// sidebar has it): the focused pane's terminal should take it.
    pub screen_focused: bool,
    /// The font the tile shows: its own (⌘+ / ⌘−), else the Settings font.
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
    /// "Move to new window" on a space (multi-window answers it), with
    /// where on screen the window goes when its tab was dragged out (else
    /// it cascades from this one).
    MoveSpaceToWindow(String, Option<Point<Pixels>>),
    /// A space's tab was let go over another open window (its label): the
    /// space moves there.
    MoveSpaceIntoWindow(String, String),
    /// Open Review on one worktree (a sidebar worktree row; the route is
    /// already Review).
    ReviewWorktree { project_id: String, path: String },
    /// One of its dialogs opened (New agent, Remove, …): the window shows
    /// one modal at a time, as the React app's single `modal` state does.
    ModalOpened,
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
        KeyBinding::new(&p("="), FontBigger, None),
        KeyBinding::new(&p("+"), FontBigger, None),
        KeyBinding::new(&p("-"), FontSmaller, None),
        KeyBinding::new(&p("0"), FontReset, None),
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
    if !mac {
        b.extend(shifted_symbols());
    }
    for ctx in ["PwSidebar", "PwMenu"] {
        b.extend([
            KeyBinding::new("up", SelectPrev, Some(ctx)),
            KeyBinding::new("down", SelectNext, Some(ctx)),
            KeyBinding::new("enter", Confirm, Some(ctx)),
        ]);
    }
    b
}

/// Windows and Linux: gpui 0.2.2 reports Shift with a symbol or digit as the
/// shifted character without Shift (Ctrl+Shift+1 arrives as `ctrl-!`), so
/// the Ctrl+Shift shortcuts on those keys are bound by what they type too
/// (US layout), plus the numpad's + and −.
fn shifted_symbols() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("ctrl->", ToggleDetails, None),
        KeyBinding::new("ctrl-+", FontBigger, None),
        KeyBinding::new("ctrl-shift-add", FontBigger, None),
        KeyBinding::new("ctrl-add", FontBigger, None),
        KeyBinding::new("ctrl-_", FontSmaller, None),
        KeyBinding::new("ctrl-shift-subtract", FontSmaller, None),
        KeyBinding::new("ctrl-subtract", FontSmaller, None),
        KeyBinding::new("ctrl-)", FontReset, None),
        KeyBinding::new("ctrl-!", SelectAgent1, None),
        KeyBinding::new("ctrl-@", SelectAgent2, None),
        KeyBinding::new("ctrl-#", SelectAgent3, None),
        KeyBinding::new("ctrl-$", SelectAgent4, None),
        KeyBinding::new("ctrl-%", SelectAgent5, None),
        KeyBinding::new("ctrl-^", SelectAgent6, None),
        KeyBinding::new("ctrl-&", SelectAgent7, None),
        KeyBinding::new("ctrl-*", SelectAgent8, None),
        KeyBinding::new("ctrl-(", SelectAgent9, None),
    ]
}

/// A Ctrl key a terminal would get (Windows, Linux): Ctrl without Shift,
/// unless the key is one `shifted_symbols` binds (Shift was pressed).
#[cfg(test)]
pub(crate) fn terminal_ctrl(s: &gpui::KeybindingKeystroke) -> bool {
    const SHIFTED: &[&str] = &[
        "~", "!", "@", "#", "$", "%", "^", "&", "*", "(", ")", "_", "+", "{", "}", "|", ":",
        "\"", "<", ">", "?", "add", "subtract",
    ];
    s.modifiers().control && !s.modifiers().shift && !SHIFTED.contains(&s.key())
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
    ProjectAccounts(Entity<dialogs::ProjectAccountsDialog>),
    Terminal(Entity<TerminalDialog>),
    Remove(RemoveState),
    Diff(Entity<DiffDialog>),
    RemoveWorktree(worktrees::RemoveWt),
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
    /// Agents waiting for their new account's login to move there.
    pending_moves: Vec<Move>,
    login_watch: Option<Task<()>>,
    /// Agents to restart once they aren't busy (a project's git identity
    /// changed), and the timer that checks.
    pending_restarts: Vec<String>,
    restart_watch: Option<Task<()>>,
    /// What this window last told the engine it shows the changes of
    /// (`Engine::show_changes`; its key, the agent).
    changes_shown: (u64, Option<String>),
    label: String,
    pub ui: UiState,
    active: String,
    route: Route,
    sidebar_collapsed: bool,
    right_pinned: bool,
    overlay_sidebar: bool,
    drawer_open: bool,
    modal: Option<Modal>,
    menu: Option<CtxMenu>,
    rename: Option<(String, Entity<TextInput>)>,
    /// An agent's name being edited in its sidebar row.
    agent_rename: Option<sidebar::AgentRenaming>,
    projects: Vec<Project>,
    changes: ChangesState,
    toasts: Vec<Toast>,
    focus: FocusHandle,
    sidebar_focus: FocusHandle,
    /// The space area's size, measured each frame.
    area: Option<Size<Pixels>>,
    /// The center's size less its space bar, estimated each frame (a first
    /// agent's size while no space is shown: `tiledAgentSize(0, 1)`).
    main_est: Option<Size<Pixels>>,
    divider: Option<DividerDrag>,
    /// The pane and zone an agent is dragged over (`None` zone: centre).
    drop_hint: Option<(String, Option<tree::Side>)>,
    /// A space tab being dragged (out to a new window).
    space_drag: Option<topbar::SpaceDragState>,
    /// A space tab dragged in another window would land here (its name).
    space_incoming: Option<SharedString>,
    /// The sidebar's keyboard cursor.
    cursor: Option<String>,
    /// The Changes list's rows where they were last drawn (live over an
    /// open diff's backdrop: [`Self::diff_row_targets`]).
    file_rows: FileRows,
    /// Changes list sections folded: (agent id, repository: "" its own).
    folded_repos: std::collections::HashSet<(String, String)>,
    route_views: HashMap<Route, AnyView>,
    /// The font each shown tile shrank to (agent id → px), from the last
    /// layout ([`tile_font`]).
    auto_font: HashMap<String, f64>,
    /// The window's worktree list (sidebar chips and rows).
    wt: worktrees::WtState,
    /// The window's one worktree list (shared with Review).
    wt_list_entity: Entity<wt_list::WorktreeList>,
    seen_task: Option<Task<()>>,
    last_selected: Option<String>,
    agent_ids: Vec<String>,
    elsewhere: elsewhere::ElsewhereState,
    _poll_elsewhere: Option<Task<()>>,
    /// The big, rarely changing pieces as cached views (part.rs).
    parts: Option<part::Parts>,
    /// Agents' in-app previews (preview.rs).
    previews: preview::Previews,
    _subs: Vec<Subscription>,
}

impl MainScreen {
    pub fn new(
        store: Entity<AgentStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MainScreen {
        let engine = cx.try_global::<EngineHandle>().map(|e| e.0.clone());
        let ui_store = crate::ui_state::store(cx);
        let ui = ui_store.read(cx).state.clone();
        let projects = engine
            .as_ref()
            .map(|e| e.projects().list())
            .unwrap_or_default();
        let focus = cx.focus_handle();
        window.focus(&focus);
        let wt_list = wt_list::WorktreeList::of(window, cx);
        let accounts = crate::accounts::model(cx);
        let subs = vec![
            // The screen draws from the store: it re-renders itself (it is
            // a cached view; its window doesn't redraw it for a model).
            cx.observe(&store, |this, _, cx| {
                this.agents_changed(cx);
                cx.notify();
            }),
            cx.observe(&wt_list, |this, _, cx| this.wt_taken(cx)),
            // Settings, the explorer and other windows change it too.
            cx.observe(&ui_store, |this, s, cx| {
                let next = s.read(cx).state.clone();
                if next != this.ui {
                    this.ui = next;
                    cx.notify();
                }
            }),
            cx.subscribe(&store, |this, _, e: &crate::agents::StoreEvent, cx| {
                // `projects-changed`: the sidebar's projects follow at once.
                if let crate::agents::StoreEvent::ProjectsChanged(list) = e {
                    if *list != this.projects {
                        this.projects = list.clone();
                        cx.notify();
                    }
                }
                this.attention(e, cx)
            }),
            // Accounts renamed, added or removed: the badges and menus.
            cx.observe(&accounts, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                // Leaving the window cancels a drag and closes a popup menu
                // (`dnd.ts` and `Menu.tsx` listen for `blur`).
                if !window.is_window_active() {
                    if cx.has_active_drag() {
                        cx.stop_active_drag(window);
                        this.drop_hint = None;
                        this.end_space_drag(cx);
                    }
                    if this.menu.take().is_some() {
                        window.focus(&this.focus);
                    }
                    cx.notify();
                }
                // Coming back counts as looking at the agent you left selected.
                if window.is_window_active() && this.route == Route::Space {
                    if let Some(a) = this.selected(cx).filter(|a| a.status == Status::Done) {
                        this.mark_seen(&a.id, cx);
                    }
                }
            }),
        ];
        let (sidebar_collapsed, right_pinned) = (ui.sidebar_collapsed, ui.right_open);
        let mut screen = MainScreen {
            store,
            engine,
            changes_shown: (0, None),
            pending_moves: Vec::new(),
            login_watch: None,
            pending_restarts: Vec::new(),
            restart_watch: None,
            label: crate::windows::label_of(window, cx),
            active: ALL_SPACE.into(),
            ui,
            route: Route::Space,
            sidebar_collapsed,
            main_est: None,
            right_pinned,
            overlay_sidebar: false,
            drawer_open: false,
            modal: None,
            menu: None,
            rename: None,
            agent_rename: None,
            projects,
            changes: ChangesState::default(),
            toasts: vec![],
            focus,
            sidebar_focus: cx.focus_handle(),
            area: None,
            divider: None,
            drop_hint: None,
            space_drag: None,
            space_incoming: None,
            cursor: None,
            file_rows: FileRows::default(),
            folded_repos: Default::default(),
            route_views: HashMap::new(),
            auto_font: HashMap::new(),
            wt: Default::default(),
            wt_list_entity: wt_list.clone(),
            seen_task: None,
            last_selected: None,
            agent_ids: vec![],
            elsewhere: Default::default(),
            _poll_elsewhere: None,
            parts: None,
            previews: Default::default(),
            _subs: subs,
        };
        screen.start_elsewhere(cx);
        if screen.ui.wall.iter().any(|w| w == &screen.label) {
            screen.route = Route::Wall;
        }
        screen.agents_changed(cx);
        MainScreen::debug_open_engineer(window, cx);
        screen
    }

    // ── queries ──────────────────────────────────────────────────────────

    pub fn route(&self) -> Route {
        self.route
    }

    /// The view another module registered for `route`, once shown.
    pub fn route_view_of(&self, route: Route) -> Option<AnyView> {
        self.route_views.get(&route).cloned()
    }

    /// This window's own view for `route` (Review's, the explorer's: made
    /// with the window's entities, so not an app-wide [`register_route`]).
    pub fn set_route_view(&mut self, route: Route, view: AnyView) {
        self.route_views.insert(route, view);
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

    /// The active space (Review scopes to it).
    pub fn current_space(&self) -> Option<workspace::Space> {
        self.active_space().cloned()
    }

    /// The agent in the active space's focused pane.
    pub fn focused_agent_id(&self) -> Option<String> {
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

    /// Change `ui.json` (`crate::ui_state` saves it 150 ms after the last
    /// change).
    fn update_ui(&mut self, f: impl FnOnce(UiState) -> UiState, cx: &mut Context<Self>) {
        // From the store's state, not this window's copy: another window may
        // have changed it since (its notification not delivered yet).
        let store = crate::ui_state::store(cx);
        let base = store.read(cx).state.clone();
        let next = f(base.clone());
        if next == base {
            return;
        }
        self.ui = next.clone();
        cx.notify();
        store.update(cx, |s, cx| s.set(next, cx));
    }

    fn with_active(&mut self, f: impl FnOnce(UiState, &str) -> UiState, cx: &mut Context<Self>) {
        let Some(id) = self.active_space().map(|s| s.id.clone()) else {
            return;
        };
        self.update_ui(|s| f(s, &id), cx);
    }

    pub fn set_route(&mut self, route: Route, cx: &mut Context<Self>) {
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
        self.wt_follow(cx);
        self.previews_follow(cx);
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
    /// Onboarding's hand-over: the new agents tiled in the All space
    /// (overflow as chips), Wall and Review off, the first one focused.
    pub fn hand_over(&mut self, ids: &[String], window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        self.set_route(Route::Space, cx);
        self.overlay_sidebar = false;
        self.active = ALL_SPACE.into();
        let max = (Self::breakpoint(window).max_per_row() * 2).max(2);
        let ids = ids.to_vec();
        self.update_ui(|s| s.tile_agents(ALL_SPACE, &ids, max), cx);
        self.cursor = Some(ids[0].clone());
        cx.notify();
    }

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

    /// Show one of this window's spaces (multi-window: a space moved in).
    pub fn show_space(&mut self, id: &str, cx: &mut Context<Self>) {
        self.active = id.to_string();
        self.set_route(Route::Space, cx);
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

    /// The size a new agent starts at (`newAgentSize`): the size the
    /// terminal already in the target pane (the first empty one, else the
    /// focused one) shows, else that pane's box fitted in cells, else the
    /// focused pane's terminal; the backend default otherwise.
    fn new_agent_size(&self, cx: &App) -> (Option<u16>, Option<u16>) {
        let Some(sp) = self.active_space().filter(|_| self.area.is_some()) else {
            // No space shown yet (the first agent): it gets the whole space.
            return match self.main_fit().or_else(crate::terminal::last_fitted) {
                Some((c, r)) => (Some(c), Some(r)),
                None => (None, None),
            };
        };
        let leaves = sp.layout.leaves();
        let focused = sp
            .focused_pane_id
            .as_ref()
            .and_then(|f| leaves.iter().find(|p| &p.id == f));
        let target = leaves.iter().find(|p| p.agent_id.is_none()).or(focused);
        let in_pane = |p: Option<&tree::Pane>| {
            p.and_then(|p| p.agent_id.as_deref())
                .and_then(|id| crate::terminal::size_of(id, cx))
        };
        if let Some((c, r)) = in_pane(target) {
            return (Some(c), Some(r));
        }
        let fit = self.area.and_then(|area| {
            let rects = tree::pane_rects(
                &sp.layout,
                tree::Rect {
                    x: 0.,
                    y: 0.,
                    w: f32::from(area.width) as f64,
                    h: f32::from(area.height) as f64,
                },
            );
            let r = rects.into_iter().find(|(id, _)| Some(id) == target.map(|t| &t.id))?.1;
            let font = self.ui.font_size;
            let (cw, ch) = crate::theme::CELL_PER_PX;
            let (pw, ph) = crate::theme::PANE_CHROME;
            let cols = ((r.w - pw) / (font * cw)).floor();
            let rows = ((r.h - ph) / (font * ch)).floor();
            (cols >= 2. && rows >= 1.).then_some((cols as u16, rows as u16))
        });
        let fit = fit
            .or_else(|| in_pane(focused))
            .or_else(crate::terminal::last_fitted);
        match fit {
            Some((c, r)) => (Some(c), Some(r)),
            None => (None, None),
        }
    }

    /// Cells for one pane filling the center (no space area measured yet).
    fn main_fit(&self) -> Option<(u16, u16)> {
        let area = self.main_est?;
        let font = self.ui.font_size;
        let (cw, ch) = crate::theme::CELL_PER_PX;
        let (pw, ph) = crate::theme::PANE_CHROME;
        let cols = ((f32::from(area.width) as f64 - pw) / (font * cw)).floor();
        let rows = ((f32::from(area.height) as f64 - ph) / (font * ch)).floor();
        (cols >= 2. && rows >= 1.).then_some((cols as u16, rows as u16))
    }

    /// The font an agent's tile shows: its own (⌘+ / ⌘−), else the Settings
    /// font shrunk to fit its pane (down to 10 px).
    pub fn tile_font(&self, agent: &str) -> f64 {
        self.ui
            .tile_font
            .get(agent)
            .or_else(|| self.auto_font.get(agent))
            .copied()
            .unwrap_or(self.ui.font_size)
    }

    /// ⌘+ / ⌘− / ⌘0 on the focused tile (`stepTileFont`); with no tile
    /// (or on the Wall), the Settings font.
    fn step_font(&mut self, delta: Option<f64>, cx: &mut Context<Self>) {
        let clamp = |f: f64| crate::theme::clamp_font(f.round() as i32) as f64;
        let id = self
            .focused_agent_id()
            // Review and the explorer still size the focused tile (React
            // only skips it on the Wall).
            .filter(|_| self.route != Route::Wall);
        let current = id.as_ref().map(|id| self.tile_font(id));
        self.update_ui(
            |mut s| {
                match (id, current) {
                    (Some(id), Some(cur)) => match delta {
                        None => {
                            s.tile_font.remove(&id);
                        }
                        Some(d) => {
                            s.tile_font.insert(id, clamp(cur + d));
                        }
                    },
                    _ => {
                        s.font_size = match delta {
                            None => workspace::DEFAULT_FONT,
                            Some(d) => clamp(s.font_size + d),
                        }
                    }
                }
                s
            },
            cx,
        );
    }

    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Self::breakpoint(window).full_sidebar() {
            self.sidebar_collapsed = !self.sidebar_collapsed;
            let v = self.sidebar_collapsed;
            self.update_ui(
                |mut s| {
                    s.sidebar_collapsed = v;
                    s
                },
                cx,
            );
        } else {
            self.overlay_sidebar = !self.overlay_sidebar;
        }
        cx.notify();
    }

    fn toggle_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Self::breakpoint(window).right_docked() {
            self.right_pinned = !self.right_pinned;
            let v = self.right_pinned;
            self.update_ui(
                |mut s| {
                    s.right_open = v;
                    s
                },
                cx,
            );
        } else {
            self.drawer_open = !self.drawer_open;
        }
        cx.notify();
    }

    /// Bench commands (`crate::bench`): `wall`/`review` on or off;
    /// `visit-all` shows each agent once (so each gets its terminal), then
    /// tiles them with Auto grid (`visit-all-4`: 2×2, the rest folded, the
    /// perf brief's standard load).
    pub fn bench(&mut self, what: &str, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        match what {
            "wall" | "review" => {
                let route = if what == "wall" { Route::Wall } else { Route::Review };
                if (self.route == route) != on {
                    self.toggle_route(route, cx);
                }
            }
            "visit-all" | "visit-all-4" => {
                let preset = if what == "visit-all" { Preset::Auto } else { Preset::G2x2 };
                let ids: Vec<String> = self.ordered(cx).into_iter().map(|a| a.id).collect();
                let win = window.window_handle();
                cx.spawn(async move |this, cx| {
                    for id in ids {
                        let _ = win.update(cx, |_, window, cx| {
                            let _ = this.update(cx, |s, cx| s.show_agent(&id, window, cx));
                        });
                        cx.background_executor().timer(std::time::Duration::from_millis(120)).await;
                    }
                    let _ = win.update(cx, |_, window, cx| {
                        let _ = this.update(cx, |s, cx| s.apply_preset(preset, window, cx));
                    });
                })
                .detach();
            }
            _ => {}
        }
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
    /// An error toast with its own title.
    pub fn toast_error(&mut self, title: String, detail: String, cx: &mut Context<Self>) {
        self.note_access_error(&detail, cx);
        self.toast(Toast::error(title, detail), cx);
    }

    pub fn failed(&mut self, what: &str, err: String, cx: &mut Context<Self>) {
        self.note_access_error(&err, cx);
        self.toast(Toast::error(format!("Couldn't {what}"), err), cx);
    }

    /// macOS refused a folder (privacy): once ever, point at Full Disk
    /// Access (App.tsx `accessHint`; the flag is `pitwall.accessHintShown`).
    pub(super) fn note_access_error(&mut self, err: &str, cx: &mut Context<Self>) {
        if !strip::is_access_error(err) || self.ui.access_hint_shown {
            return;
        }
        let task = cx.background_executor().spawn(async { pitwall_core::permissions::status() });
        cx.spawn(async move |this, cx| {
            let s = task.await;
            if !s.applies || s.full_disk_access == pitwall_core::permissions::Access::Granted {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.ui.access_hint_shown {
                    return;
                }
                this.update_ui(
                    |mut s| {
                        s.access_hint_shown = true;
                        s
                    },
                    cx,
                );
                let mut t = Toast::info(
                    "macOS blocked a folder",
                    Some(
                        "Give Pitwall Full Disk Access and macOS stops asking. Click for Settings → Permissions."
                            .into(),
                    ),
                );
                t.opens_settings = true;
                this.toast(t, cx);
            });
        })
        .detach();
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
    pub fn open_terminal(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let taken: Vec<String> = self.agents(cx).iter().map(|a| a.name.clone()).collect();
        let (cols, rows) = self.new_agent_size(cx);
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
    /// A new agent: in the store at once, shown and focused.
    pub(crate) fn created(&mut self, view: AgentView, window: &mut Window, cx: &mut Context<Self>) {
        let id = view.id.clone();
        self.store.update(cx, |s, cx| {
            if !s.agents.iter().any(|a| a.id == id) {
                s.agents.push(view);
                cx.notify();
            }
        });
        self.show_agent(&id, window, cx);
        // Rule problems never block the agent: a toast (rules §21).
        if let Some(check) = crate::rules::error_after_create(cx, id) {
            cx.spawn(async move |this, cx| {
                if let Some(e) = check.await {
                    let _ = this.update(cx, |s, cx| s.failed("apply rules", e, cx));
                }
            })
            .detach();
        }
    }

    fn open_new_agent(
        &mut self,
        project: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self.agents(cx).to_vec();
        let size = self.new_agent_size(cx);
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
        cx.emit(ScreenEvent::ModalOpened);
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
        cx.emit(ScreenEvent::ModalOpened);
        cx.notify();
    }

    /// The accounts a project's new agents use (its right-click menu).
    pub(super) fn open_project_accounts(&mut self, project: String, window: &mut Window, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        let agents: Vec<AgentView> = self.agents(cx).iter().filter(|a| a.project == project).cloned().collect();
        let dialog = cx.new(|cx| dialogs::ProjectAccountsDialog::new(engine, project, agents, cx));
        self._subs.push(cx.subscribe_in(&dialog, window, |this, _, e: &dialogs::DialogEvent, window, cx| {
            this.dialog_event(e.clone(), window, cx)
        }));
        self.modal = Some(Modal::ProjectAccounts(dialog));
        self.menu = None;
        cx.emit(ScreenEvent::ModalOpened);
        cx.notify();
    }

    fn open_remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Remove(RemoveState::new(id)));
        self.menu = None;
        cx.emit(ScreenEvent::ModalOpened);
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
        cx.emit(ScreenEvent::ModalOpened);
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
            dialogs::DialogEvent::GitIdentity { project, name } => self.git_identity_changed(project, name, cx),
            dialogs::DialogEvent::MoveAgents { ids, account, name, kind, project, login } => {
                self.close_modal(window, cx);
                self.move_agents(Move { ids, account, name, kind }, login.then_some(project), window, cx);
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

    /// Whether one of its dialogs shows.
    /// The Changes list's files drawn last frame (tests).
    #[cfg(test)]
    pub(crate) fn drawn_files(&self) -> Vec<String> {
        self.file_rows.borrow().iter().map(|(_, _, f)| f.path.clone()).collect()
    }

    /// The open diff (tests).
    #[cfg(test)]
    pub(crate) fn diff_dialog(&self) -> Option<Entity<DiffDialog>> {
        match &self.modal {
            Some(Modal::Diff(d)) => Some(d.clone()),
            _ => None,
        }
    }

    pub fn has_dialog(&self) -> bool {
        self.modal.is_some() || self.elsewhere.bring.is_some()
    }

    /// Close its dialogs (another modal opened over the window).
    pub fn close_dialogs(&mut self, cx: &mut Context<Self>) {
        if self.has_dialog() {
            self.modal = None;
            self.elsewhere.bring = None;
            cx.notify();
        }
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

    /// A space with the agents on `account` (`account_key`), named `name`.
    pub(super) fn open_account_space(&mut self, account: &str, name: &str, cx: &mut Context<Self>) {
        let agents = self.agents(cx).to_vec();
        let label = self.label.clone();
        let mut opened = String::new();
        self.update_ui(
            |s| {
                let (s, id) = s.open_account_space(account, name, &agents, &label);
                opened = id;
                s
            },
            cx,
        );
        if self.ui.window_of_space(&opened) == self.label {
            self.active = opened;
        } else {
            crate::windows::show_space(&opened, cx);
        }
        self.overlay_sidebar = false;
        self.set_route(Route::Space, cx);
        cx.notify();
    }

    /// Group the sidebar by project or by account (the setting).
    pub(super) fn set_sidebar_group(&mut self, group: workspace::SidebarGroup, cx: &mut Context<Self>) {
        self.update_ui(|mut s| { s.sidebar_group = group; s }, cx);
        cx.notify();
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
        } else {
            crate::windows::show_space(&opened, cx);
        }
        self.overlay_sidebar = false;
        self.set_route(Route::Space, cx);
        cx.notify();
    }

    fn move_space(&mut self, id: String, cx: &mut Context<Self>) {
        self.move_space_at(id, None, cx);
    }

    /// Move a space to a new window, at `at` on screen (its tab dragged
    /// out) or cascaded.
    fn move_space_at(&mut self, id: String, at: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        if id == ALL_SPACE {
            self.toast(
                Toast::info("The All space stays in the main window", None),
                cx,
            );
            return;
        }
        cx.emit(ScreenEvent::MoveSpaceToWindow(id, at));
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
        let size = self.new_agent_size(cx);
        let size = size.0.zip(size.1);
        let task = cx
            .background_executor()
            .spawn(async move { pitwall_core::engine::lifecycle::restart(&engine, &id, size) });
        cx.spawn(async move |this, cx| match task.await {
            Ok(view) => {
                let _ = cx.update(|cx| crate::agents::patch(view, cx));
            }
            Err(e) => {
                let _ = this.update(cx, |s, cx| s.failed("restart", e, cx));
            }
        })
        .detach();
    }

    /// Run agent `id` on another account (`None`: the user's own): its
    /// conversation goes with it and it restarts there.
    /// Move agents to an account: now when it's logged in (or can't tell),
    /// else once it is, with its login terminal started in `login_in` (a
    /// project folder) and shown, unless one already runs.
    fn move_agents(&mut self, mv: Move, login_in: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = mv.account.clone().filter(|_| !mv.ready(cx)) else {
            for id in mv.ids {
                self.switch_account(id, mv.account.clone(), mv.name.clone(), cx);
            }
            return;
        };
        if !mv.ids.is_empty() {
            let n = mv.ids.len();
            let what = if n == 1 { "its agent moves".to_string() } else { format!("its {n} agents move") };
            self.toast(Toast::info(&format!("Log in to {} in its terminal", mv.name), Some(format!("Then {what} there on their own."))), cx);
            self.pending_moves.push(mv.clone());
            self.watch_logins(cx);
        }
        let (Some(project), Some(engine)) = (login_in, self.engine.clone()) else { return };
        let req = crate::accounts::login_request(&mv.kind, &account, &project);
        let task = cx.background_executor().spawn(async move { pitwall_core::engine::lifecycle::create(&engine, req) });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update_in(cx, |s, window, cx| match res {
                Ok(view) => s.created(view, window, cx),
                Err(e) => s.failed("start the login", e, cx),
            });
        })
        .detach();
    }

    /// Every couple of seconds while moves wait for a login: those whose
    /// account is logged in now go.
    fn watch_logins(&mut self, cx: &mut Context<Self>) {
        if self.login_watch.is_some() {
            return;
        }
        self.login_watch = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            let more = this.update(cx, |s, cx| {
                let (go, wait): (Vec<Move>, Vec<Move>) = std::mem::take(&mut s.pending_moves).into_iter().partition(|m| m.ready(cx));
                s.pending_moves = wait;
                for mv in go {
                    for id in mv.ids {
                        s.switch_account(id, mv.account.clone(), mv.name.clone(), cx);
                    }
                }
                if s.pending_moves.is_empty() {
                    s.login_watch = None;
                }
                !s.pending_moves.is_empty()
            });
            if !matches!(more, Ok(true)) {
                break;
            }
        }));
    }

    /// `project`'s git identity changed (`name`; `None`: the user's own
    /// setup): its running agents (the project is a folder on this Mac)
    /// that can continue their conversation restart to take it, busy ones
    /// once they finish. Plain terminals don't: what runs in them would end.
    fn git_identity_changed(&mut self, project: String, name: Option<String>, cx: &mut Context<Self>) {
        let (restart, keep) = git_restarts(self.agents(cx), &project);
        let who = name.unwrap_or_else(|| "your own git setup".into());
        let mut parts = Vec::new();
        if !restart.is_empty() {
            parts.push(format!("{} restarting as it", count(restart.len(), "agent", "agents")));
        }
        if !keep.is_empty() {
            parts.push(format!("{} take it when restarted", count(keep.len(), "terminal", "terminals")));
        }
        let detail = (!parts.is_empty()).then(|| format!("{}.", parts.join("; ")));
        let folder = project.trim_end_matches('/').rsplit('/').next().unwrap_or(&project).to_string();
        self.toast(Toast::info(&format!("{folder} uses {who}"), detail), cx);
        self.pending_restarts.extend(restart);
        self.restart_when_free(cx);
    }

    /// Restart the pending agents that aren't working or waiting on the
    /// user, one every couple of seconds; busy ones once they aren't (or
    /// are gone or stopped).
    fn restart_when_free(&mut self, cx: &mut Context<Self>) {
        self.restart_free(cx);
        if self.pending_restarts.is_empty() || self.restart_watch.is_some() {
            return;
        }
        self.restart_watch = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            let more = this.update(cx, |s, cx| {
                s.restart_free(cx);
                if s.pending_restarts.is_empty() {
                    s.restart_watch = None;
                }
                !s.pending_restarts.is_empty()
            });
            if !matches!(more, Ok(true)) {
                break;
            }
        }));
    }

    /// One per call: agents started together (claude --resume all at once)
    /// can hang on each other.
    fn restart_free(&mut self, cx: &mut Context<Self>) {
        let agents = self.agents(cx).to_vec();
        let busy = |id: &String| agents.iter().find(|a| &a.id == id).filter(|a| a.running).map(|a| matches!(a.status, Status::Working | Status::Blocked));
        let mut started = false;
        for id in std::mem::take(&mut self.pending_restarts) {
            match busy(&id) {
                Some(false) if !started => {
                    started = true;
                    self.restart_agent(id, cx);
                }
                Some(_) => self.pending_restarts.push(id),
                None => {}
            }
        }
    }

    pub(super) fn switch_account(&mut self, id: String, account: Option<String>, name: String, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let size = self.new_agent_size(cx);
        let size = size.0.zip(size.1);
        let task = cx.background_executor().spawn(async move {
            pitwall_core::engine::lifecycle::switch_account(&engine, &id, account.as_deref(), size)
        });
        cx.spawn(async move |this, cx| match task.await {
            Ok(view) => {
                let resumed = view.status != pitwall_proto::Status::Stopped;
                let agent = view.name.clone();
                let _ = cx.update(|cx| crate::agents::patch(view, cx));
                let _ = this.update(cx, |s, cx| {
                    let detail = resumed.then(|| format!("{agent} picked up its conversation there"));
                    s.toast(Toast::info(&format!("{agent} now runs on {name}"), detail), cx)
                });
            }
            Err(e) => {
                let _ = this.update(cx, |s, cx| s.failed("switch the account", e, cx));
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
        } else if self.elsewhere.bring.is_some() {
            self.elsewhere.bring = None;
        } else if self.modal.is_some() {
            self.close_modal(window, cx);
        } else if self.rename.is_some() {
            self.rename = None;
        } else if self.agent_rename.is_some() {
            self.agent_rename = None;
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
        // A change made while this window was being opened reached the
        // store before this screen's observer was active: catch up.
        let store = crate::ui_state::store(cx);
        if store.read(cx).state != self.ui {
            self.ui = store.read(cx).state.clone();
        }
        let t = theme::theme(cx).clone();
        // A dialog, menu or overlay that had the keyboard closed: the screen
        // takes it back (and passes it on to the focused pane's terminal).
        if window.focused(cx).is_none() {
            window.focus(&self.focus);
        }
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
        {
            let vp = window.viewport_size();
            let side = match sidebar_mode {
                SidebarMode::Full => f32::from(crate::theme::SIDEBAR_W),
                SidebarMode::Rail => 56.,
                SidebarMode::Hidden => 0.,
            };
            // A docked right panel appears with the first agent.
            let right = if bp.right_docked() { f32::from(crate::theme::RIGHT_W) } else { 0. };
            let top = f32::from(crate::theme::TOPBAR_H) + strip::STRIP_H + 32.;
            self.main_est = Some(gpui::size(
                px((f32::from(vp.width) - side - right).max(0.)),
                px((f32::from(vp.height) - top).max(0.)),
            ));
        }
        let right = match (&selected, self.route) {
            (Some(_), Route::Space) if bp.right_docked() && self.right_pinned => Some(false),
            (Some(_), Route::Space) if !bp.right_docked() && self.drawer_open => Some(true),
            _ => None,
        };
        // Git runs on its own only for agents whose changes a window shows.
        let shown = right.and(selected.as_ref()).filter(|a| a.caps.diff).map(|a| a.id.clone());
        if shown != self.changes_shown.1 {
            self.changes_shown = (cx.entity_id().as_u64(), shown.clone());
            if let Some(e) = &self.engine {
                e.show_changes(self.changes_shown.0, shown);
            }
        }

        // Glass: chrome is translucent over the window material, floating
        // surfaces solid, panes and code views stay opaque (glass.css).
        let chrome = t.chrome();
        let slab = crate::theme::panel(&t, cx);
        let float = t.float();
        // The big, rarely changing pieces are cached views (part.rs).
        let parts = part::cached(cx).then(|| {
            let me = cx.entity();
            &*self.parts.get_or_insert_with(|| part::Parts::new(&me, cx))
        });
        let mut drawn = Vec::new();
        let sidebar_part = parts.filter(|_| sidebar_mode == SidebarMode::Full).map(|p| {
            drawn.push(part::PartKind::Sidebar);
            p.sidebar.clone()
        });
        let strip_part = parts.map(|p| {
            drawn.push(part::PartKind::Strip);
            p.strip.clone()
        });
        let topbar_part = parts.map(|p| {
            drawn.push(part::PartKind::Topbar);
            p.topbar.clone()
        });
        let panel_part = parts.filter(|_| right == Some(false)).map(|p| {
            drawn.push(part::PartKind::Panel);
            p.panel.clone()
        });
        if parts.is_some() && self.route != Route::Space {
            drawn.push(part::PartKind::Route);
        }
        if parts.is_some() && self.route == Route::Space {
            drawn.extend([part::PartKind::Bar, part::PartKind::Chips]);
        }
        if let Some(p) = &self.parts {
            p.forget_unused(&drawn, window);
        }
        let main_row = div()
            .flex_1()
            .min_h_0()
            .flex()
            .relative()
            .when(sidebar_mode != SidebarMode::Hidden, |d| match &sidebar_part {
                Some(p) => d.child(part::slot(
                    p,
                    gpui::StyleRefinement::default().w(crate::theme::SIDEBAR_W).h_full().flex_none(),
                )),
                None => {
                    let bar = self.render_sidebar(sidebar_mode, &agents, &slab, window, cx);
                    d.child(crate::kit::chrome_panel("sidebar", bar, cx))
                }
            })
            .child(self.render_center(&agents, bp, &t, window, cx))
            .when(right == Some(false), |d| match &panel_part {
                Some(p) => d.child(part::slot(
                    p,
                    gpui::StyleRefinement::default().w(crate::theme::RIGHT_W).h_full().flex_none(),
                )),
                None => d.children(selected.clone().map(|a| {
                    let panel = self.render_panel(&a, false, &slab, window, cx);
                    crate::kit::chrome_panel("right-panel", panel, cx)
                })),
            })
            .when(
                self.overlay_sidebar && sidebar_mode != SidebarMode::Full,
                |d| {
                    d.child(crate::kit::scrim_in(
                        "sidebar-scrim",
                        div()
                            .id("sidebar-scrim")
                            // Takes the click: nothing under a backdrop is clicked through it.
                            .occlude()
                            .absolute()
                            .inset_0()
                            .bg(t.scrim())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.overlay_sidebar = false;
                                cx.notify();
                            })),
                    ))
                    .child(crate::kit::motion::enter(
                        "sidebar-overlay",
                        crate::kit::motion::Fx::SLIDE_LEFT,
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .shadow(t.drop_shadow())
                            .child(self.render_sidebar(
                                SidebarMode::Full,
                                &agents,
                                &float,
                                window,
                                cx,
                            )),
                    ))
                },
            )
            .when(right == Some(true), |d| {
                d.child(crate::kit::scrim_in(
                    "drawer-scrim",
                    div()
                        .id("drawer-scrim")
                        // Takes the click: nothing under a backdrop is clicked through it.
                        .occlude()
                        .absolute()
                        .inset_0()
                        .bg(t.scrim())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.drawer_open = false;
                            cx.notify();
                        })),
                ))
                .children(selected.clone().map(|a| {
                    crate::kit::motion::enter(
                        "right-drawer",
                        crate::kit::motion::Fx::SLIDE_RIGHT,
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .shadow(t.drop_shadow())
                            .child(self.render_panel(&a, true, &float, window, cx)),
                    )
                }))
            });
        let topbar = if let Some(p) = &topbar_part {
            part::slot(p, gpui::StyleRefinement::default().w_full().h(TOPBAR_H).flex_none())
        } else if crate::theme::glass_regions(cx) {
            let topbar = self.render_topbar(&agents, bp, &chrome, window, cx);
            crate::kit::glass_region("topbar", 0., topbar)
                .w_full()
                .flex_none()
                .into_any_element()
        } else {
            self.render_topbar(&agents, bp, &chrome, window, cx).into_any_element()
        };

        let modal = self.modal.is_some() || self.elsewhere.bring.is_some();
        let screen = div()
            .id("main-screen")
            .key_context("MainScreen")
            .track_focus(&self.focus)
            // Esc cancels an agent drag (`dnd.ts`), before anything else
            // (a terminal, Dismiss) sees the key.
            .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" && cx.has_active_drag() {
                    cx.stop_active_drag(window);
                    this.drop_hint = None;
                    this.end_space_drag(cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
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
            .on_action(cx.listener(|this, _: &RefreshChanges, _, cx| {
                this.load_changes(true, cx);
                // The Changes panel's worktree list, when open.
                let open = this
                    .selected(cx)
                    .is_some_and(|a| this.wt.is_open(&worktrees::panel_key(&a.id)));
                if open {
                    this.wt_list(true, cx);
                    this.wt_load_files(true, cx);
                }
                // The explorer (on the window root) refreshes too.
                cx.propagate();
            }))
            .on_action(cx.listener(|this, _: &FontBigger, _, cx| this.step_font(Some(1.), cx)))
            .on_action(cx.listener(|this, _: &FontSmaller, _, cx| this.step_font(Some(-1.), cx)))
            .on_action(cx.listener(|this, _: &FontReset, _, cx| this.step_font(None, cx)))
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
            .on_mouse_move(cx.listener(Self::preview_drag_move))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::preview_drag_up))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(crate::theme::backdrop(cx))
            .map(|d| crate::platform::decorations::round_top(d, window))
            .text_color(t.text)
            .font_family(crate::kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .child(topbar)
            .child(main_row)
            .child(match &strip_part {
                Some(p) => part::slot(
                    p,
                    gpui::StyleRefinement::default().w_full().h(px(strip::STRIP_H)).flex_none(),
                ),
                None => self.render_strip(&chrome, bp, cx).into_any_element(),
            })
            .child(self.render_toasts(&float, cx))
            .children(self.render_modal(&t, window, cx))
            .children(self.render_bring_in(&t, cx))
            .children(self.render_menu(&float, cx))
            .child(self.space_drag_watch(cx));
        // Its working dots hand their rings to the window's pulse layer,
        // unless a modal is over them (then they animate under it).
        crate::kit::motion::pulse_scope(screen, modal)
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
#[cfg(test)]
mod preview_tests;

impl Drop for MainScreen {
    /// A closed window shows no agent's changes any more.
    fn drop(&mut self) {
        if let (Some(e), (key, Some(_))) = (&self.engine, &self.changes_shown) {
            e.show_changes(*key, None);
        }
    }
}

/// Agents of `kind` to move to `account` (`None`: the user's own), `name`.
/// When `project`'s git identity changes: the running agents that restart
/// to take it (they continue their conversation), and those that don't
/// (plain terminals, whose programs would end; agents that would start
/// over). The Race Engineer isn't a project's.
fn git_restarts(agents: &[AgentView], project: &str) -> (Vec<String>, Vec<String>) {
    let (restart, keep): (Vec<&AgentView>, Vec<&AgentView>) = agents
        .iter()
        .filter(|a| a.project == project && a.running && !a.engineer)
        .partition(|a| (!a.terminal || a.agent_in_terminal) && a.caps.restart && a.caps.resume);
    let ids = |v: Vec<&AgentView>| v.into_iter().map(|a| a.id.clone()).collect();
    (ids(restart), ids(keep))
}

/// "1 agent", "3 agents".
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[derive(Clone)]
struct Move {
    ids: Vec<String>,
    account: Option<String>,
    name: String,
    kind: String,
}

impl Move {
    /// Its account is logged in, or can't say (only Claude Code's and
    /// Codex's logins can be read): the move can go.
    fn ready(&self, cx: &mut App) -> bool {
        let Some(id) = &self.account else { return true };
        let m = crate::accounts::model(cx);
        let m = m.read(cx);
        match (m.kind(&self.kind), m.list.get(id)) {
            (Some(def), Some(a)) => pitwall_core::accounts::logged_in(&def, Some(a)) != Some(false),
            _ => true,
        }
    }
}
