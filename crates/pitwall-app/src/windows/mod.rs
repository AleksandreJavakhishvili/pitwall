//! Multi-window (docs/spec/gpui/inventory.md §25; Tauri:
//! `src-tauri/src/windows.rs` and the window logic of `src/App.tsx`).
//!
//! - The first window is `main`; "Move to new window" (⌘⇧N, the tab and
//!   space-bar buttons, the palette) opens `pitwall-N` (the next free
//!   number) owning that space. A space goes back to main when its window
//!   closes. A window whose spaces are all gone stays open ("No spaces in
//!   this window"), as in the Tauri app.
//! - Every window has its own tabs, sidebar, Wall flag, toasts and strip
//!   (`MainScreen` per window) and shares the one `ui.json` store
//!   (`crate::ui_state`): space ownership is `ui.windowOf`, Wall mode
//!   `ui.wall`, so nothing is duplicated and every write goes through one
//!   in-process store (no window overwrites another's change).
//! - Closing a secondary window returns its spaces to main; at start, main
//!   reclaims spaces of windows that no longer exist.
//! - `windows.json` keeps each window's label, space and bounds in the Tauri
//!   format ([`file`]), written 500 ms after the last change; saved windows
//!   reopen at launch where they were.
//! - Showing an agent that lives in another window's space focuses that
//!   window, then the agent (⌘J, the sidebar, toasts, the Wall, the strip).
//!   Settings and About open over the active window.

pub mod file;
pub mod ownership;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Global,
    Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowId,
    WindowOptions,
};

use crate::main_screen::{MainScreen, ScreenEvent};
use crate::ui::{Content, MainView};
use crate::window_state;

pub use file::MAIN;

/// Opens (or brings forward) the main window: `lib.rs`'s `show_main`.
pub type ShowMain = fn(&mut App) -> WindowHandle<MainView>;

/// The windows the app has open and what `windows.json` says about them.
pub struct Windows {
    content: Option<Content>,
    root: Option<PathBuf>,
    show_main: Option<ShowMain>,
    /// Open Pitwall windows by label, main included.
    open: Vec<(String, WindowHandle<MainView>)>,
    /// Labels of windows, by id (read while their view is built).
    labels: HashMap<WindowId, String>,
    /// `windows.json`.
    entries: Vec<file::WindowEntry>,
    save: Option<Task<()>>,
    /// Quitting: windows going away now are reopened next time.
    quitting: bool,
    /// An agent being routed to its window this frame (one request per show).
    routing: Option<String>,
}

impl Global for Windows {}

impl Windows {
    fn new(content: Option<Content>, root: Option<PathBuf>, show_main: Option<ShowMain>) -> Self {
        let entries = root.as_deref().map(file::load).unwrap_or_default();
        Windows {
            content,
            root,
            show_main,
            open: vec![],
            labels: HashMap::new(),
            entries,
            save: None,
            quitting: false,
            routing: None,
        }
    }

    fn handle(&self, label: &str) -> Option<WindowHandle<MainView>> {
        self.open.iter().find(|(l, _)| l == label).map(|(_, h)| *h)
    }

    fn label_of_id(&self, id: WindowId) -> Option<&str> {
        self.open
            .iter()
            .find(|(_, h)| h.window_id() == id)
            .map(|(l, _)| l.as_str())
            .or_else(|| self.labels.get(&id).map(String::as_str))
    }
}

fn state(cx: &mut App) -> &mut Windows {
    if !cx.has_global::<Windows>() {
        cx.set_global(Windows::new(None, None, None));
    }
    cx.global_mut::<Windows>()
}

/// Once, at start (after `ui_state::init`): what new windows show, where
/// `windows.json` lives, and how to open the main window.
pub fn init(content: Content, root: Option<PathBuf>, show_main: ShowMain, cx: &mut App) {
    cx.set_global(Windows::new(Some(content), root, Some(show_main)));
    cx.on_window_closed(closed).detach();
    cx.on_app_quit(|cx| {
        let w = state(cx);
        w.quitting = true;
        w.save = None;
        let (root, entries) = (w.root.clone(), w.entries.clone());
        if let Some(root) = root {
            if let Err(e) = file::save(&root, &entries) {
                eprintln!("pitwall: could not save windows: {e}");
            }
        }
        async {}
    })
    .detach();
    // Keep windows.json's spaces on what each window still owns.
    let store = crate::ui_state::store(cx);
    cx.observe(&store, |store, cx| {
        let ui = store.read(cx).state.clone();
        if ownership::follow_spaces(&mut state(cx).entries, &ui) {
            schedule_save(cx);
        }
    })
    .detach();
}

/// This window's label (`main` unless it was opened as a secondary one).
pub fn label_of(window: &Window, cx: &App) -> String {
    cx.try_global::<Windows>()
        .and_then(|w| w.label_of_id(window.window_handle().window_id()))
        .unwrap_or(MAIN)
        .to_string()
}

/// The labels of the open windows, main first.
pub fn labels(cx: &App) -> Vec<String> {
    let mut out: Vec<String> = cx
        .try_global::<Windows>()
        .map(|w| w.open.iter().map(|(l, _)| l.clone()).collect())
        .unwrap_or_default();
    out.sort_by_key(|l| (l != MAIN, file::label_number(l)));
    out
}

/// Hook a window's view up (from `MainView::new`): its bounds go to
/// `windows.json`, and its screen's "move to window" and "show agent"
/// events are answered here.
pub fn attach(
    screen: Option<&Entity<MainScreen>>,
    window: &mut Window,
    cx: &mut Context<MainView>,
) -> Vec<Subscription> {
    let label = label_of(window, cx);
    if let Some(handle) = window.window_handle().downcast::<MainView>() {
        let w = state(cx);
        w.open.retain(|(l, _)| *l != label);
        w.open.push((label.clone(), handle));
    }
    record_bounds(&label, window, cx);
    let mut subs = vec![cx.observe_window_bounds(window, {
        let label = label.clone();
        move |_, window, cx| record_bounds(&label, window, cx)
    })];
    if let Some(screen) = screen {
        subs.push(
            cx.subscribe(screen, move |_, _, e: &ScreenEvent, cx| match e {
                ScreenEvent::MoveSpaceToWindow(space) => {
                    let (space, from) = (space.clone(), label.clone());
                    cx.defer(move |cx| move_to_new_window(&space, &from, cx));
                }
                ScreenEvent::AgentShown(agent) => route_agent(&label, agent, cx),
                _ => {}
            }),
        );
    }
    subs
}

/// The window Settings, About and the like open over: the active Pitwall
/// window, else main (`menu.rs`: the focused window, fallback main).
pub fn active_or_main(cx: &mut App) -> Option<WindowHandle<MainView>> {
    let active = cx
        .active_window()
        .and_then(|w| w.downcast::<MainView>())
        .filter(|h| cx.windows().iter().any(|w| w.window_id() == h.window_id()));
    if active.is_some() {
        return active;
    }
    let show = state(cx).show_main?;
    Some(show(cx))
}

/// Move `space` out of window `from` into a new window
/// (`moveSpaceToNewWindow`). "All" stays in main.
pub fn move_to_new_window(space: &str, from: &str, cx: &mut App) {
    if space == crate::main_screen::workspace::ALL_SPACE {
        return;
    }
    // The window opens already owning the space.
    let Some(label) = open_new(space, from, cx) else {
        return;
    };
    schedule_save(cx);
    show_space_in(&label, space, cx);
}

/// Bring the window that owns `space` forward on it (a project space opened
/// from the sidebar of another window).
pub fn show_space(space: &str, cx: &mut App) {
    let label = crate::ui_state::get(cx).window_of_space(space).to_string();
    let space = space.to_string();
    cx.defer(move |cx| show_space_in(&label, &space, cx));
}

fn show_space_in(label: &str, space: &str, cx: &mut App) {
    let Some(handle) = window_for(label, cx) else {
        return;
    };
    let space = space.to_string();
    let _ = handle.update(cx, |view, window, cx| {
        crate::platform::unhide(window);
        window.activate_window();
        if let Some(screen) = view.screen.clone() {
            screen.update(cx, |s, cx| s.show_space(&space, cx));
        }
    });
}

/// An agent was shown in window `from`; when it lives in another window,
/// that window comes forward and focuses it (`focusRequest` + `focus_window`).
fn route_agent(from: &str, agent: &str, cx: &mut App) {
    let ui = crate::ui_state::get(cx);
    let Some(loc) = ui.locate(agent) else {
        return;
    };
    if loc.window == from {
        return;
    }
    let w = state(cx);
    if w.routing.as_deref() == Some(agent) {
        return;
    }
    w.routing = Some(agent.to_string());
    let agent = agent.to_string();
    cx.defer(move |cx| {
        state(cx).routing = None;
        let Some(handle) = window_for(&loc.window, cx) else {
            return;
        };
        let _ = handle.update(cx, |view, window, cx| {
            crate::platform::unhide(window);
            window.activate_window();
            if let Some(screen) = view.screen.clone() {
                screen.update(cx, |s, cx| s.show_agent(&agent, window, cx));
            }
        });
    });
}

/// The open window with `label`; main is opened when it was closed. A
/// secondary window that isn't open gives its spaces back to main.
fn window_for(label: &str, cx: &mut App) -> Option<WindowHandle<MainView>> {
    let live: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
    if let Some(h) = state(cx)
        .handle(label)
        .filter(|h| live.contains(&h.window_id()))
    {
        return Some(h);
    }
    if label != MAIN {
        crate::ui_state::update(cx, |ui| *ui = ownership::reclaim(ui.clone(), label));
    }
    let show = state(cx).show_main?;
    Some(show(cx))
}

/// Open a window for `space`, cascaded from window `from`. Its label.
fn open_new(space: &str, from: &str, cx: &mut App) -> Option<String> {
    let w = state(cx);
    let label = file::next_label(
        w.open
            .iter()
            .map(|(l, _)| l.as_str())
            .chain(w.entries.iter().map(|e| e.label.as_str())),
    );
    let source = w.handle(from);
    let at = source.and_then(|h| {
        h.update(cx, |_, window, _| {
            let b = window.bounds();
            point(b.origin.x + px(28.), b.origin.y + px(28.))
        })
        .ok()
    });
    let (dw, dh) = window_state::DEFAULT_SIZE;
    let bounds = match at {
        Some(origin) => Bounds {
            origin,
            size: size(px(dw), px(dh)),
        },
        None => Bounds::centered(None, size(px(dw), px(dh)), cx),
    };
    let w = state(cx);
    w.entries.retain(|e| e.label != label);
    w.entries.push(file::WindowEntry {
        label: label.clone(),
        space_id: Some(space.to_string()),
        bounds: None,
    });
    let before = crate::ui_state::get(cx);
    crate::ui_state::update(cx, |ui| {
        *ui = ownership::move_space(ui.clone(), space, &label)
    });
    match build(&label, WindowBounds::Windowed(bounds), None, cx) {
        Ok(_) => Some(label),
        Err(e) => {
            crate::ui_state::update(cx, |ui| *ui = before);
            state(cx).entries.retain(|en| en.label != label);
            eprintln!("pitwall: could not open a window: {e}");
            None
        }
    }
}

/// Open a secondary window. `restored`: the physical size it was saved at
/// and the scale `bounds` assumed (corrected when the window's own differs).
fn build(
    label: &str,
    bounds: WindowBounds,
    restored: Option<(file::Bounds, f32)>,
    cx: &mut App,
) -> Result<WindowHandle<MainView>, String> {
    let w = state(cx);
    let content = w.content.clone().ok_or("no window content")?;
    let root = w.root.clone();
    let (min_w, min_h) = window_state::MIN_SIZE;
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions {
            title: Some("Pitwall".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(min_w), px(min_h))),
        window_decorations: crate::platform::decorations::request(),
        app_id: Some("dev.pitwall.app".into()),
        ..Default::default()
    };
    let label = label.to_string();
    let handle = cx.open_window(options, move |window, cx| {
        crate::platform::lean_renderer(window);
        let id = window.window_handle().window_id();
        state(cx).labels.insert(id, label);
        if let Some((saved, assumed)) = restored {
            let scale = window.scale_factor();
            if (scale - assumed).abs() > 0.01 {
                window.resize(size(
                    px(saved.width as f32 / scale),
                    px(saved.height as f32 / scale),
                ));
            }
        }
        cx.new(|cx| MainView::new(content, root, window, cx))
    });
    handle.map_err(|e| e.to_string())
}

/// Reopen the saved secondary windows (call once, after the main window
/// opened), then give main the spaces of windows that are gone.
pub fn restore(cx: &mut App) {
    let ui = crate::ui_state::get(cx);
    let plan = ownership::reopen_plan(&state(cx).entries, &ui);
    let scale = scale_hint(cx);
    let displays: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
    for r in plan {
        if let Some(space) = &r.claim {
            let label = r.entry.label.clone();
            crate::ui_state::update(cx, |ui| {
                *ui = ownership::move_space(ui.clone(), space, &label)
            });
        }
        let saved = r.entry.bounds.filter(file::Bounds::usable);
        let bounds = to_logical(saved, scale, &displays, cx);
        if let Err(e) = build(&r.entry.label, bounds, saved.map(|b| (b, scale)), cx) {
            eprintln!("pitwall: could not reopen a window: {e}");
        }
    }
    let open: Vec<String> = state(cx).open.iter().map(|(l, _)| l.clone()).collect();
    let w = state(cx);
    let before = w.entries.len();
    w.entries
        .retain(|e| e.label == MAIN || open.contains(&e.label));
    if w.entries.len() != before {
        schedule_save(cx);
    }
    let open: Vec<&str> = open.iter().map(String::as_str).collect();
    for gone in ownership::stale_labels(&crate::ui_state::get(cx), &open) {
        crate::ui_state::update(cx, |ui| *ui = ownership::reclaim(ui.clone(), &gone));
    }
}

/// Debug builds: `PITWALL_DEBUG_WINDOWS=move:<agent name>` opens that
/// agent's project as a space and moves it to a new window;
/// `show:<agent name>` shows the agent from a window that doesn't hold it
/// (cross-window focus). Both run once the agents are in, and log the outcome
/// (screenshots and checks without synthetic input).
pub fn debug_hook(cx: &mut App) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(cmd) = std::env::var("PITWALL_DEBUG_WINDOWS") else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(1500))
            .await;
        let _ = cx.update(|cx| {
            let Some(store) = (match &state(cx).content {
                Some(Content::Live { store }) => Some(store.clone()),
                _ => None,
            }) else {
                return;
            };
            let agents = store.read(cx).agents.clone();
            let (verb, name) = cmd.split_once(':').unwrap_or((cmd.as_str(), ""));
            let Some(agent) = agents.iter().find(|a| a.name == name).cloned() else {
                eprintln!("pitwall-debug: no agent {name}");
                return;
            };
            match verb {
                "move" => {
                    let mut space = String::new();
                    crate::ui_state::update(cx, |ui| {
                        let display = agent.project.rsplit('/').next().unwrap_or("").to_string();
                        let (next, id) =
                            ui.clone()
                                .open_project_space(&agent.project, &display, &agents, MAIN);
                        *ui = next;
                        space = id;
                    });
                    let from = crate::ui_state::get(cx).window_of_space(&space).to_string();
                    move_to_new_window(&space, &from, cx);
                }
                "show" => {
                    // From a window that doesn't show it.
                    let owner = crate::ui_state::get(cx).locate(&agent.id).map(|l| l.window);
                    let from = labels(cx).into_iter().find(|l| Some(l) != owner.as_ref());
                    if let Some(from) = from.and_then(|l| state(cx).handle(&l)) {
                        let _ = from.update(cx, |view, window, cx| {
                            if let Some(s) = view.screen.clone() {
                                s.update(cx, |s, cx| s.show_agent(&agent.id, window, cx));
                            }
                        });
                    }
                }
                _ => {}
            }
        });
        cx.background_executor()
            .timer(Duration::from_millis(500))
            .await;
        let _ = cx.update(|cx| {
            let active = cx.active_window().map(|w| w.window_id());
            let w = state(cx);
            let label = w
                .open
                .iter()
                .find(|(_, h)| Some(h.window_id()) == active)
                .map(|(l, _)| l.clone());
            let ui = crate::ui_state::get(cx);
            eprintln!(
                "pitwall-debug: windows {:?}, active {:?}, windowOf {:?}",
                labels(cx),
                label,
                ui.window_of
            );
            for l in labels(cx) {
                let focused = state(cx)
                    .handle(&l)
                    .and_then(|h| h.read(cx).ok())
                    .and_then(|v| v.screen.clone())
                    .and_then(|s| s.read(cx).focused_agent_id());
                eprintln!("pitwall-debug: {l} focuses {focused:?}");
            }
        });
    })
    .detach();
}

/// The main window's bounds from `windows.json` (a Tauri-era file, before
/// the app saved its own), in logical pixels.
pub fn main_bounds(cx: &mut App) -> Option<WindowBounds> {
    let b = state(cx)
        .entries
        .iter()
        .find(|e| e.label == MAIN)?
        .bounds
        .filter(file::Bounds::usable)?;
    let displays: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
    let scale = scale_hint(cx);
    Some(to_logical(Some(b), scale, &displays, cx))
}

/// The scale saved physical pixels are read with: main's, else the usual
/// one for the OS (Retina on macOS).
fn scale_hint(cx: &mut App) -> f32 {
    let main = state(cx).handle(MAIN);
    main.and_then(|h| h.update(cx, |_, w, _| w.scale_factor()).ok())
        .unwrap_or(if cfg!(target_os = "macos") { 2. } else { 1. })
}

/// Saved physical bounds as window bounds: the size always, the place only
/// when its top-left corner is on a display (`bounds_visible`).
fn to_logical(
    saved: Option<file::Bounds>,
    scale: f32,
    displays: &[Bounds<gpui::Pixels>],
    cx: &App,
) -> WindowBounds {
    let (dw, dh) = window_state::DEFAULT_SIZE;
    let Some(b) = saved else {
        return WindowBounds::Windowed(Bounds::centered(None, size(px(dw), px(dh)), cx));
    };
    let (min_w, min_h) = window_state::MIN_SIZE;
    let sz = size(
        px((b.width as f32 / scale).max(min_w)),
        px((b.height as f32 / scale).max(min_h)),
    );
    let corner = point(px(b.x as f32 / scale + 40.), px(b.y as f32 / scale + 20.));
    if displays.iter().any(|d| d.contains(&corner)) {
        WindowBounds::Windowed(Bounds {
            origin: point(px(b.x as f32 / scale), px(b.y as f32 / scale)),
            size: sz,
        })
    } else {
        WindowBounds::Windowed(Bounds::centered(None, sz, cx))
    }
}

/// A window moved or resized: remember it in physical pixels (outer
/// position, inner size), as Tauri does.
fn record_bounds(label: &str, window: &Window, cx: &mut App) {
    if state(cx).quitting {
        return;
    }
    let scale = window.scale_factor();
    let frame = window.bounds();
    let content = window.viewport_size();
    let (w, h) = (f32::from(content.width), f32::from(content.height));
    if w <= 0. || h <= 0. {
        return;
    }
    let b = file::Bounds {
        x: (f32::from(frame.origin.x) * scale).round() as i32,
        y: (f32::from(frame.origin.y) * scale).round() as i32,
        width: (w * scale).round() as u32,
        height: (h * scale).round() as u32,
    };
    let entries = &mut state(cx).entries;
    let idx = match entries.iter().position(|e| e.label == label) {
        Some(i) => i,
        None if label == MAIN => {
            entries.insert(
                0,
                file::WindowEntry {
                    label: MAIN.into(),
                    space_id: None,
                    bounds: None,
                },
            );
            0
        }
        None => return,
    };
    if entries[idx].bounds == Some(b) {
        return;
    }
    entries[idx].bounds = Some(b);
    schedule_save(cx);
}

/// Write `windows.json` 500 ms after the last change.
fn schedule_save(cx: &mut App) {
    let Some(root) = state(cx).root.clone() else {
        return;
    };
    let task = cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(500))
            .await;
        let entries = cx.update(|cx| state(cx).entries.clone()).ok();
        if let Some(entries) = entries {
            let write = cx
                .background_executor()
                .spawn(async move { file::save(&root, &entries) });
            if let Err(e) = write.await {
                eprintln!("pitwall: could not save windows: {e}");
            }
        }
    });
    state(cx).save = Some(task);
}

/// A window closed: a secondary one is forgotten and its spaces go back to
/// main (unless the app is quitting: it reopens next time).
fn closed(cx: &mut App) {
    let live: Vec<AnyWindowHandle> = cx.windows();
    let w = state(cx);
    let gone: Vec<String> = w
        .open
        .iter()
        .filter(|(_, h)| !live.iter().any(|l| l.window_id() == h.window_id()))
        .map(|(l, _)| l.clone())
        .collect();
    if gone.is_empty() {
        return;
    }
    w.open.retain(|(l, _)| !gone.contains(l));
    w.labels
        .retain(|id, _| live.iter().any(|l| l.window_id() == *id));
    if w.quitting {
        return;
    }
    let secondary: Vec<String> = gone.into_iter().filter(|l| l != MAIN).collect();
    if secondary.is_empty() {
        return;
    }
    state(cx).entries.retain(|e| !secondary.contains(&e.label));
    schedule_save(cx);
    for label in secondary {
        crate::ui_state::update(cx, |ui| *ui = ownership::reclaim(ui.clone(), &label));
    }
}

#[cfg(test)]
mod tests;
