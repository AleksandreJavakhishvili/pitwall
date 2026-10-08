//! The palette's command registry: every row comes from a provider
//! registered here, the built-in ones ([`super::builtin`]) included.

use std::rc::Rc;

use gpui::{App, Global, SharedString, WeakEntity, Window};
use pitwall_proto::Status;

use crate::explorer::Explorer;
use crate::main_screen::{CommandState, MainScreen};

/// Where a provider's rows go: rows are listed by rank, then in the order
/// their provider returned them. The built-in sections use these ranks;
/// pick one in between (say `rank::FILES + 50`) to slot rows after Files.
pub mod rank {
    pub const AGENTS: u32 = 100;
    pub const REMOVE: u32 = 200;
    pub const BLOCKED: u32 = 300;
    pub const NEW: u32 = 400;
    pub const TERMINALS: u32 = 500;
    pub const QUEUE: u32 = 600;
    pub const VIEWS: u32 = 700;
    pub const FILES: u32 = 800;
    pub const WINDOW: u32 = 900;
    pub const LAYOUT: u32 = 1000;
    pub const DENSITY: u32 = 1100;
    pub const PANELS: u32 = 1200;
    pub const SETTINGS: u32 = 1300;
    pub const QUIT: u32 = 1400;
    pub const THEME: u32 = 1500;
}

/// The icon column (`.pal-lead`).
#[derive(Clone, Debug, PartialEq)]
pub enum Lead {
    /// A text glyph ("+", "⚙", "▦") in `--text-3`.
    Glyph(SharedString),
    /// A text glyph in amber (`.pal-icon-blocked`).
    Warn(SharedString),
    /// One of the kit's stroke icons (`kit::icon`), 14 px.
    Icon(&'static str),
    /// The agent's status flag.
    Status(Status),
}

/// A piece of a row's label.
#[derive(Clone, Debug, PartialEq)]
pub enum Span {
    Text(SharedString),
    /// Bold (`<strong>`).
    Strong(SharedString),
    /// Small and dim, after a gap (`.pal-sub`).
    Sub(SharedString),
    /// `--text-2` (`.pal-quote`).
    Quote(SharedString),
    /// Monospace (`.mono`).
    Mono(SharedString),
}

impl Span {
    pub fn text(&self) -> &str {
        match self {
            Span::Text(s) | Span::Strong(s) | Span::Sub(s) | Span::Quote(s) | Span::Mono(s) => s,
        }
    }
}

/// The right-hand hint (`.pal-hint`).
#[derive(Clone, Debug, PartialEq)]
pub enum Hint {
    /// A key label written as on macOS ("⌘⇧T"); other desktops see
    /// "Ctrl+Shift+T" (`kit::keys`).
    Keys(&'static str),
    /// Dim text ("current").
    Sub(SharedString),
}

pub type RunFn = Rc<dyn Fn(&mut Window, &mut App)>;

/// What choosing a row does.
#[derive(Clone)]
pub enum Run {
    /// Close the palette, then run.
    Do(RunFn),
    /// Keep the palette open with this text typed ("Queue for api-fix: …").
    Fill(SharedString),
}

/// One row of the palette.
#[derive(Clone)]
pub struct Command {
    /// Unique among the rows shown at once.
    pub id: SharedString,
    pub label: Vec<Span>,
    /// Words the query is matched against (every typed word must occur).
    pub search: String,
    pub lead: Option<Lead>,
    pub hint: Option<Hint>,
    pub rank: u32,
    /// Only listed once something is typed (presets, densities, …).
    pub query_only: bool,
    /// Listed first whatever is typed ("Terminal at <path>").
    pub pinned: bool,
    pub run: Run,
}

impl Command {
    /// A row labelled `label`, found by `search`, running `run`.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        search: impl Into<String>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Command {
        Command {
            id: id.into(),
            label: vec![Span::Text(label.into())],
            search: search.into(),
            lead: None,
            hint: None,
            rank: rank::SETTINGS,
            query_only: false,
            pinned: false,
            run: Run::Do(Rc::new(run)),
        }
    }

    pub fn label(mut self, spans: Vec<Span>) -> Command {
        self.label = spans;
        self
    }

    pub fn glyph(mut self, glyph: impl Into<SharedString>) -> Command {
        self.lead = Some(Lead::Glyph(glyph.into()));
        self
    }

    pub fn icon(mut self, name: &'static str) -> Command {
        self.lead = Some(Lead::Icon(name));
        self
    }

    pub fn lead(mut self, lead: Lead) -> Command {
        self.lead = Some(lead);
        self
    }

    pub fn keys(mut self, keys: &'static str) -> Command {
        self.hint = Some(Hint::Keys(keys));
        self
    }

    pub fn hint(mut self, hint: Hint) -> Command {
        self.hint = Some(hint);
        self
    }

    pub fn rank(mut self, rank: u32) -> Command {
        self.rank = rank;
        self
    }

    pub fn query_only(mut self) -> Command {
        self.query_only = true;
        self
    }

    pub fn pinned(mut self) -> Command {
        self.pinned = true;
        self
    }

    pub fn fill(mut self, text: impl Into<SharedString>) -> Command {
        self.run = Run::Fill(text.into());
        self
    }

    /// The label as plain text (tests, logs).
    pub fn plain(&self) -> String {
        self.label.iter().map(Span::text).collect()
    }
}

/// What providers see when the palette lists its rows.
pub struct PaletteContext<'a> {
    /// What is typed.
    pub query: &'a str,
    pub state: &'a CommandState,
    /// The window's main screen (absent in tests).
    pub screen: Option<WeakEntity<MainScreen>>,
    /// The window's file explorer, when there is one.
    pub explorer: Option<WeakEntity<Explorer>>,
}

impl PaletteContext<'_> {
    /// The focused pane's agent.
    pub fn selected(&self) -> Option<&pitwall_proto::AgentView> {
        let id = self.state.selected.as_deref()?;
        self.state.agents.iter().find(|a| a.id == id)
    }
}

pub type Provider = Rc<dyn Fn(&PaletteContext, &App) -> Vec<Command>>;

/// Every registered provider, in registration order.
#[derive(Default, Clone)]
pub struct Registry(pub(super) Vec<Provider>);

impl Global for Registry {}

/// Add rows to the palette: `provider` runs each time the palette lists
/// its rows (on open and on every keystroke) and returns the rows it
/// offers; give them a [`rank`] to place them.
pub fn register(cx: &mut App, provider: impl Fn(&PaletteContext, &App) -> Vec<Command> + 'static) {
    cx.default_global::<Registry>().0.push(Rc::new(provider));
}

/// Every provider's rows, ordered by rank (stable).
pub fn collect(pc: &PaletteContext, cx: &App) -> Vec<Command> {
    let providers = cx
        .try_global::<Registry>()
        .map(|r| r.0.clone())
        .unwrap_or_default();
    let mut rows: Vec<Command> = providers.iter().flat_map(|p| p(pc, cx)).collect();
    rows.sort_by_key(|c| c.rank);
    rows
}
