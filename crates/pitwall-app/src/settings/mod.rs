//! Settings, appearance and onboarding (phase 6–7; inventory §19, §20, §24,
//! parts of §27): the Settings dialog (pages: [`view`], [`pages`]), About,
//! the first-run welcome screen and "Scan again", and the appearance every
//! window follows (theme, Glass, Reduce motion, density).
//!
//! Settings are also data: the registry in `pitwall_proto::settings` is
//! applied to `ui.json` by [`data`], changed by `pitwall settings` through
//! [`backend`], and by editing `ui.json` by hand through [`watch`].
//!
//! Wiring (kept to a few lines in shared files):
//! - `lib.rs` calls [`init`] once the engine is up, and opens About
//!   (`crate::menu::About`) where the OS has no About panel;
//! - `MainView` holds one [`Layer`] (the overlays), calls [`attach_window`]
//!   when a window opens and [`appearance::os_changed`] when the OS
//!   appearance changes.

pub mod about;
pub mod backend;
pub mod cli_install;
pub mod data;
mod engineer;
pub mod onboarding;
pub mod pages;
pub mod permissions;
pub mod prefs;
pub mod view;
pub mod watch;
pub mod widgets;

use std::path::PathBuf;

use gpui::{
    div, prelude::*, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Global,
    Subscription, Window,
};

use pitwall_core::Shared;
use pitwall_proto::AgentView;

use crate::agents::AgentStore;
use crate::menu::Dismiss;

use crate::theme::{self as appearance, appearance};
use self::onboarding::{Mode as OnbMode, Onboarding, OnboardingEvent};
use self::prefs::UiPrefs;
use self::view::{SettingsEvent, SettingsView};

/// What Settings needs from the app: the hosted engine (none when the data
/// folder was refused) and the data folder.
pub struct SettingsHost {
    pub engine: Option<Shared>,
    pub root: Option<PathBuf>,
}

impl Global for SettingsHost {}

/// Settings sections other modules add (Rules, §21): the Rules page.
#[derive(Default)]
pub struct ExtraSections(pub Vec<fn(&mut Window, &mut App) -> AnyElement>);

impl Global for ExtraSections {}

/// Keep the engine and data folder for Settings and onboarding. Call once,
/// after `ui_state::init` (which loads the preferences and applies the
/// appearance), before windows open.
pub fn init(cx: &mut App, engine: Option<Shared>, root: Option<PathBuf>) {
    cx.set_global(SettingsHost { engine, root });
    if !cx.has_global::<ExtraSections>() {
        cx.set_global(ExtraSections::default());
    }
    cx.bind_keys(view::bindings());
    backend::outside(cx);
    watch::start(cx);
    crate::palette::registry::register(cx, |_, _| page_commands());
}

/// `pitwall settings` for this app: give the socket server's backend the
/// app's state. Once, after [`init`].
pub fn attach_backend(backend: std::sync::Arc<backend::AppSettings>, cx: &mut App) {
    backend::attach(backend, cx);
}

/// "Settings: Appearance" and the like in the palette.
fn page_commands() -> Vec<crate::palette::registry::Command> {
    use crate::palette::registry::{rank, Command};
    pitwall_proto::settings::Page::ALL
        .into_iter()
        .map(|p| {
            Command::new(
                format!("settings-{}", p.id()),
                format!("Settings: {}", p.label()),
                format!("settings preferences {} {}", p.label().to_lowercase(), page_words(p)),
                move |w, cx| {
                    data::set_page(cx, p);
                    w.dispatch_action(Box::new(crate::menu::OpenSettings), cx);
                },
            )
            .glyph("⚙")
            .query_only()
            .rank(rank::SETTINGS)
        })
        .collect()
}

/// More words each page is found by.
fn page_words(p: pitwall_proto::settings::Page) -> &'static str {
    use pitwall_proto::settings::Page;
    match p {
        Page::General => "scan again elsewhere command line cli tool",
        Page::Agents => "claude codex hooks race engineer assistant",
        Page::Rules => "rulesync rule sets",
        Page::Appearance => "theme dark light glass flat liquid motion density font",
        Page::FolderAccess => "full disk access permissions privacy folders",
        Page::About => "version licence license",
    }
}

/// The current preferences (from `ui.json`).
pub fn prefs(cx: &App) -> UiPrefs {
    UiPrefs::of(&crate::ui_state::get(cx))
}

/// Change preferences: the appearance follows at once in every window, and
/// `ui.json` is written 150 ms after the last change (as `useUiState`).
pub fn set_prefs(window: &mut Window, cx: &mut App, f: impl FnOnce(&mut UiPrefs)) {
    let before = prefs(cx);
    let mut now = before;
    f(&mut now);
    if now == before {
        return;
    }
    crate::ui_state::update(cx, |s| now.write(s));
    // `apply` can't reach the window being updated right now.
    appearance::apply_window(window, cx);
}

/// A new window: give it the material and native appearance, and re-check
/// the OS accessibility options whenever it comes to the front (Tauri:
/// each window re-checks on focus).
pub fn attach_window<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Subscription {
    appearance::apply_window(window, cx);
    cx.observe_window_activation(window, |_, window, cx| {
        if window.is_window_active() {
            let before = appearance(cx).os;
            crate::rules::stale::refresh(cx);
            if crate::platform::glass::os_prefs() != before {
                appearance::os_changed(cx);
                appearance::apply_window(window, cx);
            }
        }
    })
}

/// What the overlay layer tells its window.
#[derive(Debug, Clone)]
pub enum LayerEvent {
    /// Settings, About or "Scan again" opened (the window shows one modal
    /// at a time).
    Opened,
    /// Onboarding started these agents: tile them, focus the first;
    /// `failures` are for toasts (the rest still started).
    HandOver {
        agents: Vec<AgentView>,
        failures: Vec<String>,
    },
}

/// The overlays of one window: Settings, About and the full-window
/// onboarding (first launch, "Scan again").
pub struct Layer {
    store: Option<Entity<AgentStore>>,
    settings: Option<Entity<SettingsView>>,
    onboarding: Option<Entity<Onboarding>>,
    about: bool,
    focus: FocusHandle,
    focus_next_render: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<LayerEvent> for Layer {}

impl Layer {
    /// The welcome screen opens by itself on a folder that was never
    /// onboarded (`get_onboarded` false).
    pub fn new(store: Option<Entity<AgentStore>>, cx: &mut Context<Self>) -> Layer {
        let mut layer = Layer {
            store,
            settings: None,
            onboarding: None,
            about: false,
            focus: cx.focus_handle(),
            focus_next_render: false,
            _subs: Vec::new(),
        };
        let onboarded = cx
            .try_global::<SettingsHost>()
            .and_then(|h| h.engine.as_ref())
            .map(|e| e.projects().onboarded());
        if onboarded == Some(false) && layer.store.is_some() {
            layer.open_onboarding(OnbMode::Welcome, cx);
        }
        // Debug builds: `PITWALL_DEBUG_OPEN=settings|about|rescan` opens one
        // at start (screenshots without synthetic key presses).
        if cfg!(debug_assertions) {
            match std::env::var("PITWALL_DEBUG_OPEN").as_deref() {
                Ok("settings" | "settings-end") => {
                    layer.onboarding = None;
                    layer.open_settings(cx);
                }
                Ok("about") => {
                    layer.onboarding = None;
                    layer.open_about(cx);
                }
                Ok("rescan") => layer.open_onboarding(OnbMode::Rescan, cx),
                _ => {}
            }
        }
        layer
    }

    pub fn is_open(&self) -> bool {
        self.settings.is_some() || self.onboarding.is_some() || self.about
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.onboarding.is_some() {
            return;
        }
        self.about = false;
        if self.settings.is_none() {
            let view = cx.new(|cx| SettingsView::new(self.store.is_some(), cx));
            let sub = cx.subscribe(&view, |this, _, e: &SettingsEvent, cx| match e {
                SettingsEvent::Close => this.close(cx),
                SettingsEvent::ScanAgain => {
                    this.settings = None;
                    this.open_onboarding(OnbMode::Rescan, cx);
                }
            });
            self._subs.push(sub);
            self.settings = Some(view);
        }
        self.focus_next_render = true;
        cx.emit(LayerEvent::Opened);
        cx.notify();
    }

    pub fn open_about(&mut self, cx: &mut Context<Self>) {
        if self.onboarding.is_some() {
            return;
        }
        self.settings = None;
        self.about = true;
        self.focus_next_render = true;
        cx.emit(LayerEvent::Opened);
        cx.notify();
    }

    fn open_onboarding(&mut self, mode: OnbMode, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let view = cx.new(|cx| Onboarding::new(mode, store, cx));
        let sub = cx.subscribe(&view, |this, _, e: &OnboardingEvent, cx| match e {
            OnboardingEvent::Close => this.close(cx),
            OnboardingEvent::Finished { created, failures } => {
                for f in failures {
                    eprintln!("pitwall: {f}");
                }
                cx.emit(LayerEvent::HandOver {
                    agents: created.clone(),
                    failures: failures.clone(),
                });
                this.close(cx);
            }
        });
        self._subs.push(sub);
        self.onboarding = Some(view);
        self.focus_next_render = true;
        if mode == OnbMode::Rescan {
            cx.emit(LayerEvent::Opened);
        }
        cx.notify();
    }

    /// Close Settings, About or "Scan again" (another modal opened); the
    /// first-launch welcome stays.
    pub fn close_dialog(&mut self, cx: &mut Context<Self>) {
        let welcome = self
            .onboarding
            .as_ref()
            .is_some_and(|o| o.read(cx).mode() == OnbMode::Welcome);
        if self.is_open() && !welcome {
            self.close(cx);
        }
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.settings = None;
        self.onboarding = None;
        self.about = false;
        self._subs.clear();
        cx.notify();
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        // The welcome screen needs an explicit choice; "Scan again" closes.
        if let Some(onb) = &self.onboarding {
            if onb.read(cx).mode() == OnbMode::Welcome {
                return;
            }
        }
        if self.is_open() {
            self.close(cx);
            cx.stop_propagation();
        }
    }

    fn modal(
        &self,
        id: &'static str,
        width: f32,
        child: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = crate::theme::theme(cx).clone();
        let this = cx.entity().downgrade();
        crate::kit::Modal::new(id, width, move |_, cx| {
            let _ = this.update(cx, |l, cx| l.close(cx));
        })
        .motion(crate::theme::motion_on(cx))
        .render(&t, child)
    }
}

impl Render for Layer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next_render) {
            // Settings: its page list, for ↑/↓.
            match &self.settings {
                Some(s) => window.focus(&s.read(cx).focus),
                None => window.focus(&self.focus),
            }
        }
        // Drops the native glass pieces no region placed this frame (painted
        // after the rest of the window: the layer is MainView's last child).
        let sweeper = crate::kit::region_sweeper();
        let root = div()
            .id("settings-layer")
            .track_focus(&self.focus)
            .key_context("SettingsLayer")
            .on_action(cx.listener(Self::dismiss))
            .absolute()
            .inset_0()
            .when(!self.is_open(), |d| d.size_0())
            .child(sweeper);
        if let Some(onb) = &self.onboarding {
            return root.child(onb.clone());
        }
        if let Some(s) = self.settings.clone() {
            let modal = self.modal("settings", view::WIDTH, s.into_any_element(), cx);
            return root.child(modal);
        }
        if self.about {
            let body = about::render(window, cx, cx.listener(|this, _, _, cx| this.close(cx)));
            let modal = self.modal("about", 400., body, cx);
            return root.child(modal);
        }
        root
    }
}
