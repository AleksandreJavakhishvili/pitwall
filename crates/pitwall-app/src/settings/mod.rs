//! Settings, appearance and onboarding (phase 6–7; inventory §19, §20, §24,
//! parts of §27): the Settings dialog, About, the first-run welcome screen
//! and "Scan again", and the appearance every window follows (theme, Glass,
//! Reduce motion, density).
//!
//! Wiring (kept to a few lines in shared files):
//! - `lib.rs` calls [`init`] once the engine is up, and routes [`About`];
//! - `MainView` holds one [`Layer`] (the overlays), calls [`attach_window`]
//!   when a window opens and [`appearance::os_changed`] when the OS
//!   appearance changes.

pub mod about;
pub mod appearance;
pub mod cli_install;
pub mod native;
pub mod onboarding;
pub mod permissions;
pub mod prefs;
pub mod view;
pub mod widgets;

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    actions, canvas, div, prelude::*, px, AnimationExt, AnyElement, App, Bounds, Context, Entity,
    EventEmitter, FocusHandle, Global, Pixels, Subscription, Task, Window,
};

use pitwall_core::Shared;
use pitwall_proto::AgentView;

use crate::agents::AgentStore;
use crate::menu::Dismiss;

use self::appearance::{appearance, modal_backdrop, Motion};
use self::onboarding::{Mode as OnbMode, Onboarding, OnboardingEvent};
use self::prefs::UiPrefs;
use self::view::{SettingsEvent, SettingsView};

actions!(
    pitwall,
    [
        /// The About box (macOS app menu "About Pitwall").
        About,
    ]
);

/// What Settings needs from the app: the hosted engine (none when the data
/// folder was refused) and the data folder.
pub struct SettingsHost {
    pub engine: Option<Shared>,
    pub root: Option<PathBuf>,
}

impl Global for SettingsHost {}

/// The preferences as last set, and the pending write of `ui.json`.
pub struct PrefsState {
    pub prefs: UiPrefs,
    root: Option<PathBuf>,
    saving: Option<Task<()>>,
}

impl Global for PrefsState {}

/// Settings sections other modules add (Rules, §21): drawn after Tiles.
#[derive(Default)]
pub struct ExtraSections(pub Vec<fn(&mut Window, &mut App) -> AnyElement>);

impl Global for ExtraSections {}

/// Load the preferences from `ui.json`, set the appearance globals and keep
/// the engine for Settings and onboarding. Call once, before windows open.
pub fn init(cx: &mut App, engine: Option<Shared>, root: Option<PathBuf>) {
    let prefs = root.as_deref().map(prefs::load).unwrap_or_default();
    cx.set_global(SettingsHost {
        engine,
        root: root.clone(),
    });
    cx.set_global(PrefsState {
        prefs,
        root,
        saving: None,
    });
    if !cx.has_global::<ExtraSections>() {
        cx.set_global(ExtraSections::default());
    }
    appearance::apply(prefs.inputs(), cx);
}

/// The current preferences.
pub fn prefs(cx: &App) -> UiPrefs {
    cx.try_global::<PrefsState>()
        .map(|s| s.prefs)
        .unwrap_or_default()
}

/// Change preferences: the appearance follows at once in every window, and
/// `ui.json` is written 150 ms after the last change (as `useUiState`).
pub fn set_prefs(window: &mut Window, cx: &mut App, f: impl FnOnce(&mut UiPrefs)) {
    if !cx.has_global::<PrefsState>() {
        return;
    }
    let state = cx.global_mut::<PrefsState>();
    let before = state.prefs;
    f(&mut state.prefs);
    let (now, root) = (state.prefs, state.root.clone());
    if now == before {
        return;
    }
    appearance::apply(now.inputs(), cx);
    // `apply` can't reach the window being updated right now.
    appearance::apply_window(window, cx);
    let Some(root) = root else {
        return;
    };
    let task = cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(150))
            .await;
        let write = cx
            .background_executor()
            .spawn(async move { prefs::save(&root, &now) });
        if let Err(e) = write.await {
            eprintln!("pitwall: could not save ui.json: {e}");
        }
    });
    cx.global_mut::<PrefsState>().saving = Some(task);
}

/// A new window: give it the material and native appearance, and re-check
/// the OS accessibility options whenever it comes to the front (Tauri:
/// each window re-checks on focus).
pub fn attach_window<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Subscription {
    appearance::apply_window(window, cx);
    cx.observe_window_activation(window, |_, window, cx| {
        if window.is_window_active() {
            let before = appearance(cx).os;
            if native::os_prefs() != before {
                appearance::os_changed(cx);
                appearance::apply_window(window, cx);
            }
        }
    })
}

/// A chrome region that gets its own native glass piece under Liquid Glass
/// (sidebar, top bar, right panel). Wrap the region's element: the piece
/// follows its bounds every frame; GPUI must paint the region translucent
/// (`appearance::chrome`). Elsewhere it is just the child.
pub fn glass_region(id: &'static str, radius: f32, child: impl IntoElement) -> impl IntoElement {
    div()
        .relative()
        .size_full()
        .child(
            canvas(
                move |bounds: Bounds<Pixels>, window, cx| {
                    if appearance(cx).glass == Some(appearance::GlassTier::Liquid) {
                        native::sync_region(window, id, bounds, radius);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(child)
}

/// Chrome regions get their own Liquid Glass pieces (macOS 26+, Glass on).
/// `PITWALL_GLASS_REGIONS=0` keeps window-level glass only.
pub fn glass_regions(cx: &App) -> bool {
    appearance(cx).glass == Some(appearance::GlassTier::Liquid)
        && std::env::var("PITWALL_GLASS_REGIONS").as_deref() != Ok("0")
}

/// A chrome panel (the sidebar, the right panel): under Liquid Glass with
/// regions, a floating rounded glass slab inset from the window edges, like
/// a macOS 26 source list; otherwise just `child`. Draw `child` with
/// [`appearance::panel`]'s theme so its own background stays clear.
pub fn chrome_panel(id: &'static str, child: impl IntoElement, cx: &App) -> AnyElement {
    if !glass_regions(cx) {
        return child.into_any_element();
    }
    let line = appearance(cx).glass_tokens.line_strong;
    div()
        .h_full()
        .flex_none()
        .p(px(8.))
        .child(glass_region(
            id,
            PANEL_RADIUS,
            div()
                .h_full()
                .rounded(px(PANEL_RADIUS))
                .border_1()
                .border_color(line)
                .overflow_hidden()
                .child(child),
        ))
        .into_any_element()
}

/// Corner radius of floating glass panels (macOS 26 source lists).
pub const PANEL_RADIUS: f32 = 14.;

/// What the overlay layer tells its window.
#[derive(Debug, Clone)]
pub enum LayerEvent {
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
                SettingsEvent::About => {
                    this.settings = None;
                    this.about = true;
                    cx.notify();
                }
            });
            self._subs.push(sub);
            self.settings = Some(view);
        }
        self.focus_next_render = true;
        cx.notify();
    }

    pub fn open_about(&mut self, cx: &mut Context<Self>) {
        if self.onboarding.is_some() {
            return;
        }
        self.settings = None;
        self.about = true;
        self.focus_next_render = true;
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
        cx.notify();
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
        let ap = appearance(cx);
        let t = appearance::float(&ap.theme(), cx);
        let motion = !ap.reduce_motion;
        let panel = div()
            .id(id)
            .occlude()
            .w(px(width))
            .max_h_full()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow(appearance::shadow(cx))
            .overflow_hidden()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(child);
        let panel: AnyElement = if motion {
            panel
                .with_animation(id, Motion::open(), |el, d| {
                    let rise = Motion::spring()(d);
                    el.opacity(d).mt(px((1. - rise) * Motion::RISE))
                })
                .into_any_element()
        } else {
            panel.into_any_element()
        };
        div()
            .id("settings-backdrop")
            .absolute()
            .inset_0()
            .p(px(24.))
            .bg(modal_backdrop(cx))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
            .child(panel)
            .into_any_element()
    }
}

impl Render for Layer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next_render) {
            window.focus(&self.focus);
        }
        // Drops the native glass pieces no region placed this frame (painted
        // after the rest of the window: the layer is MainView's last child).
        let sweeper = canvas(
            |_, _, _| {},
            |_, _, window, _| native::sweep_regions(window),
        )
        .absolute()
        .size_0();
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
            let modal = self.modal("settings", 520., s.into_any_element(), cx);
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
