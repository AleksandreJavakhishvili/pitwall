//! A device's screen in a tile (docs/spec/device.md): an Android device
//! (adb + scrcpy-server) or an iOS Simulator (`pitwall-simhost`), streamed,
//! drawn as a GPU surface, and driven by clicks and typing. A peer of the
//! view tiles: opened from the command palette ("Open device: <name>"),
//! placed, moved, maximised and put in a window of its own as they are
//! ([`super::inbox_window`], key `@device:<platform>:<id>`), back after a
//! restart (`ui.json`: the pane's `device`; a window's in `deviceWindows`).
//!
//! The stream lives as long as the tile: closing it stops the stream and
//! undoes what it set up (the adb forward, the helper process). Nothing a
//! tile shows reaches an agent.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use gpui::{canvas, div, prelude::*, px, AnyElement, App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Window};

use super::files_tile::TileDrag;
use super::strip::Toast;
use super::tree::{Content, Surface};
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::device::geometry::{to_screen, Turn};
use crate::device::stream::{Event, Input, Key, Phase, Session};
use crate::device::{self, Device, Platform, State};
use crate::explorer::dock::FilesPlace;
use crate::kit;
use crate::platform::video::Frame;
use crate::theme::{theme, Theme};

type PlaceMenu = (Point<Pixels>, Vec<(String, String)>);

/// The setting that turns device tiles on and off.
pub const SETTING: &str = "general.devicePreviews";

/// Device tiles may be shown (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Where the stream is.
enum Status {
    Connecting,
    Live { input: Result<(), String> },
    Ended(String),
    Off,
}

pub struct DeviceTile {
    /// The device's key (`android:<serial>`, `ios:<udid>`).
    key: String,
    platform: Option<Platform>,
    id: String,
    name: String,
    focus: FocusHandle,
    session: Option<Session>,
    status: Status,
    frame: Option<Frame>,
    size: (u32, u32),
    /// Where the picture's box was drawn last (window coordinates).
    body: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// A finger is down (the left button held).
    touching: bool,
    in_window: bool,
    pinned: bool,
    menu: Option<PlaceMenu>,
    _events: Option<gpui::Task<()>>,
}

impl Focusable for DeviceTile {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DeviceTile {
    fn new(key: &str, cx: &mut Context<Self>) -> Self {
        let parsed = device::parse_key(key);
        let id = parsed.as_ref().map(|(_, i)| i.clone()).unwrap_or_default();
        let name = known(cx).iter().find(|d| d.key() == key).map(|d| d.name.clone()).unwrap_or_else(|| id.clone());
        let mut t = DeviceTile {
            key: key.to_string(),
            platform: parsed.map(|(p, _)| p),
            id,
            name,
            focus: cx.focus_handle(),
            session: None,
            status: Status::Connecting,
            frame: None,
            size: (0, 0),
            body: Default::default(),
            touching: false,
            in_window: false,
            pinned: false,
            menu: None,
            _events: None,
        };
        t.connect(cx);
        t
    }

    /// (Re)starts the stream.
    fn connect(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        self.frame = None;
        self._events = None;
        if !enabled(cx) {
            self.status = Status::Off;
            return;
        }
        let Some(platform) = self.platform else {
            self.status = Status::Ended("Not a device Pitwall knows".into());
            return;
        };
        let backend: Arc<dyn device::stream::Backend> = match platform {
            Platform::Android => match device::android::LocalAdb::new() {
                Ok(adb) => Arc::new(device::android::Android {
                    adb: Arc::new(adb),
                    server: Arc::new(device::scrcpy::UserFile::in_data(&crate::web::page::data_root(cx))),
                    options: device::scrcpy::ServerOptions { scid: scid(), max_size: 1600, max_fps: 60, bit_rate: 8_000_000, control: true },
                    patience: Duration::from_secs(10),
                }),
                Err(e) => {
                    self.status = Status::Ended(e);
                    return;
                }
            },
            Platform::Ios => Arc::new(device::ios::Simulator { helper: device::ios::helper(), set: None, patience: Duration::from_secs(20) }),
        };
        let decoder = match crate::platform::video::Decoder::new() {
            Ok(d) => d,
            Err(e) => {
                self.status = Status::Ended(e);
                return;
            }
        };
        let dev = Device { platform, id: self.id.clone(), name: self.name.clone(), virtual_device: true, state: State::Ready };
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        self.status = Status::Connecting;
        self.session = Some(Session::start(backend, dev, decoder, move |e| {
            let _ = tx.unbounded_send(e);
        }));
        // Events from the stream's thread, on the UI thread; pictures that
        // queued up while a frame was drawn: only the newest is kept.
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(e) = rx.try_recv() {
                    batch.push(e);
                }
                let gone = this
                    .update(cx, |v, cx| {
                        for e in batch {
                            v.event(e);
                        }
                        cx.notify();
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        }));
    }

    fn event(&mut self, e: Event<Frame>) {
        match e {
            Event::Connected { input } => self.status = Status::Live { input },
            Event::Size(w, h) => self.size = (w, h),
            Event::Picture(f) => self.frame = Some(f),
            Event::Ended(why) => {
                self.status = Status::Ended(why);
                self.session = None;
            }
        }
    }

    /// Who draws it now (a window of its own, or a tile).
    pub fn held(&mut self, in_window: bool, cx: &mut Context<Self>) {
        if self.in_window != in_window {
            self.in_window = in_window;
            self.menu = None;
            cx.notify();
        }
    }

    fn send(&mut self, i: Input, cx: &mut Context<Self>) {
        if let Some(s) = &self.session {
            if let Err(e) = s.send(&i) {
                self.status = Status::Ended(e);
                cx.notify();
            }
        }
    }

    /// A press, a drag or a release at `at` (window coordinates): a touch
    /// where it lands on the screen.
    fn touch(&mut self, phase: Phase, at: Point<Pixels>, cx: &mut Context<Self>) {
        let b = self.body.get();
        let p = (f32::from(at.x - b.origin.x), f32::from(at.y - b.origin.y));
        let tile = (f32::from(b.size.width), f32::from(b.size.height));
        let frame = self.frame.as_ref().map(|f| (f.width, f.height)).unwrap_or(self.size);
        let Some(ratio) = to_screen(p, tile, frame, Turn::None) else {
            // Left the screen while down: lifted at its edge.
            if phase != Phase::Down && self.touching {
                self.touching = false;
                let clamp = |v: f32, n: f32| v.clamp(0., n.max(1.));
                if let Some(r) = to_screen((clamp(p.0, tile.0 - 1.), clamp(p.1, tile.1 - 1.)), tile, frame, Turn::None) {
                    self.send(Input::Touch { phase: Phase::Up, at: r }, cx);
                }
            }
            return;
        };
        match phase {
            Phase::Down => self.touching = true,
            Phase::Move if !self.touching => return,
            Phase::Up if !self.touching => return,
            Phase::Up => self.touching = false,
            Phase::Move => {}
        }
        self.send(Input::Touch { phase, at: ratio }, cx);
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) || !matches!(self.status, Status::Live { input: Ok(()) }) {
            return;
        }
        let m = &e.keystroke.modifiers;
        // Pitwall's own shortcuts stay Pitwall's.
        if m.platform || m.control || m.function {
            return;
        }
        let key = match e.keystroke.key.as_str() {
            "enter" => Some(Key::Enter),
            "backspace" => Some(Key::Backspace),
            "delete" => Some(Key::Delete),
            "tab" => Some(Key::Tab),
            "escape" => Some(Key::Escape),
            "left" => Some(Key::Left),
            "right" => Some(Key::Right),
            "up" => Some(Key::Up),
            "down" => Some(Key::Down),
            "space" => Some(Key::Space),
            _ => None,
        };
        if let Some(k) = key {
            self.send(Input::Key { down: true, key: k }, cx);
            self.send(Input::Key { down: false, key: k }, cx);
        } else if let Some(text) = e.keystroke.key_char.clone().filter(|t| !t.is_empty()) {
            self.send(Input::Text(text), cx);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let in_window = self.in_window;
        let what = Content::Device(self.key.clone());
        let skey = Surface::Device(self.key.clone()).key();
        let maximized = {
            let ui = crate::ui_state::get(cx);
            ui.tile_of(&what).is_some_and(|(space, pane)| ui.space(&space).is_some_and(|s| s.maximized_pane_id.as_deref() == Some(pane.as_str())))
        };
        let btn = |name: &str, icon: &str, tip: String, on: Option<bool>| {
            let sel = format!("device-tile-{name}");
            kit::icon_btn_state(sel.clone(), icon, tip, true, 13., on, t)
                .debug_selector(move || sel.clone())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let name: SharedString = self.name.clone().into();
        let live_input = matches!(self.status, Status::Live { input: Ok(()) });
        let android = self.platform == Some(Platform::Android);
        let (chip, chip_tip) = match &self.status {
            Status::Connecting => ("connecting", String::from("Connecting to the device")),
            Status::Live { input: Ok(()) } => ("live", String::from("Showing the device; clicks and typing go to it")),
            Status::Live { input: Err(why) } => ("view only", format!("Input isn't available: {why}")),
            Status::Ended(why) => ("stopped", why.clone()),
            Status::Off => ("off", "Device screens are turned off in Settings".into()),
        };
        let skey2 = skey.clone();
        let skey3 = skey.clone();
        div()
            .id("device-tile-bar")
            .debug_selector(|| "device-tile-bar".into())
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(4.))
            .pr(px(8.))
            .border_b_1()
            .border_color(if t.is_glass() { t.g.line } else { t.line })
            .when(!in_window, |d| d.on_drag(TileDrag { what: what.clone(), name: name.clone() }, |d, grab, w, cx| d.ghost(grab, w, cx)))
            .child(
                div()
                    .id("device-tile-grip")
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(kit::tooltip(if in_window { "Drag into a Pitwall window to place it in the layout" } else { "Drag to move the device" }))
                    .when(in_window, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(move |_, _, window, cx| {
                            cx.stop_propagation();
                            let k = skey.clone();
                            window.defer(cx, move |window, cx| super::inbox_window::drag_start(&k, window, cx));
                        }))
                    })
                    .child(kit::icon("grip", 13., t.text_4)),
            )
            .child(kit::icon("phone", 15., t.text_2))
            .child(div().flex_1().min_w_0().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(kit::one_line(self.name.clone())))
            .child(div().id("device-tile-state").flex_none().text_size(px(11.)).text_color(t.text_3).tooltip(kit::tooltip(chip_tip)).child(chip))
            .when(live_input, |d| {
                d.when(android, |d| d.child(btn("back", "back", "Back".into(), None).on_click(cx.listener(|v, _, _, cx| v.send(Input::Back, cx)))))
                    .child(btn("home", "grid", "Home".into(), None).on_click(cx.listener(|v, _, _, cx| v.send(Input::Home, cx))))
            })
            .when(matches!(self.status, Status::Ended(_)), |d| {
                d.child(btn("reconnect", "restart", "Connect again".into(), None).on_click(cx.listener(|v, _, _, cx| {
                    v.connect(cx);
                    cx.notify();
                })))
            })
            .child(
                btn(if in_window { "dock" } else { "move" }, "grid", (if in_window { "Place it in a Pitwall window's layout" } else { "Move it in the layout, or to a window" }).into(), Some(self.menu.is_some())).on_click(cx.listener(move |v, e: &gpui::ClickEvent, _, cx| {
                    v.menu = match v.menu {
                        Some(_) => None,
                        None => Some((e.position(), super::inbox_window::replaceable(&skey2, cx))),
                    };
                    cx.notify();
                })),
            )
            .map(|d| {
                if in_window {
                    d.when(crate::platform::can_keep_on_top(), |d| {
                        let pinned = self.pinned;
                        d.child(btn("pin", "pin", (if pinned { "Kept on top of other windows: click to let it go behind them" } else { "Keep on top of other windows" }).into(), Some(pinned)).on_click(cx.listener(|v, _, window, cx| {
                            v.pinned = !v.pinned;
                            crate::platform::keep_on_top(window, v.pinned);
                            cx.notify();
                        })))
                    })
                } else {
                    let key = self.key.clone();
                    d.child(btn("maximize", if maximized { "restore" } else { "maximize" }, kit::keys(if maximized { "Restore (⌘⏎)" } else { "Maximise (⌘⏎)" }, false), None).on_click(cx.listener(|v, _, window, cx| {
                        window.focus(&v.focus, cx);
                        window.dispatch_action(Box::new(super::ToggleMaximize), cx);
                    })))
                    .child(btn("undock", "window", "Open in a window of its own".into(), None).on_click(cx.listener(move |_, _, _, cx| {
                        let k = skey3.clone();
                        cx.defer(move |cx| {
                            if let Err(e) = super::inbox_window::pop_out(&k, None, true, cx) {
                                eprintln!("pitwall: could not open the device's window: {e}");
                            }
                        });
                    })))
                    .child(btn("close", "x", "Close the device's screen".into(), None).on_click(cx.listener(move |_, _, _, cx| {
                        let k = key.clone();
                        cx.defer(move |cx| close(&k, cx));
                    })))
                }
            })
            .into_any_element()
    }

    fn place_menu(&self, at: Point<Pixels>, replace: Vec<(String, String)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let skey = Surface::Device(self.key.clone()).key();
        let pick = move |p: Option<(FilesPlace, Option<String>)>, cx: &mut App| {
            let _ = me.update(cx, |v, cx| {
                v.menu = None;
                cx.notify();
            });
            if let Some((place, of)) = p {
                let skey = skey.clone();
                cx.defer(move |cx| {
                    if let Err(e) = super::inbox_window::place(&skey, place, of.as_deref(), cx) {
                        eprintln!("pitwall: could not move the device: {e}");
                    }
                });
            }
        };
        crate::explorer::dock::menu(at, !self.in_window, None, replace, "The device", t, pick)
    }

    fn body(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let measure = {
            let cell = self.body.clone();
            canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().size_full()
        };
        let message = |text: String| div().size_full().flex().items_center().justify_center().p(px(16.)).text_size(px(12.)).text_color(t.text_3).child(text);
        let content = match (&self.frame, &self.status) {
            (_, Status::Off) => message("Device screens are turned off (Settings → General).".into()).into_any_element(),
            (Some(f), _) => crate::platform::video::picture(f),
            (None, Status::Connecting) => message(format!("Connecting to {}…", self.name)).into_any_element(),
            (None, Status::Live { .. }) => message("Waiting for the first picture…".into()).into_any_element(),
            (None, Status::Ended(why)) => message(why.clone()).into_any_element(),
        };
        div()
            .id("device-tile-body")
            .debug_selector(|| "device-tile-body".into())
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(gpui::black())
            .child(measure)
            .child(content)
            .on_mouse_down(MouseButton::Left, cx.listener(|v, e: &MouseDownEvent, window, cx| {
                window.focus(&v.focus, cx);
                v.touch(Phase::Down, e.position, cx);
            }))
            .on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| {
                if v.touching {
                    v.touch(Phase::Move, e.position, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|v, e: &MouseUpEvent, _, cx| v.touch(Phase::Up, e.position, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|v, e: &MouseUpEvent, _, cx| v.touch(Phase::Up, e.position, cx)))
            .into_any_element()
    }
}

/// A session id for scrcpy: 31 bits, different for each tile.
fn scid() -> u32 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    (n ^ std::process::id().rotate_left(13)) & 0x7fff_ffff
}

impl Render for DeviceTile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let menu = self.menu.clone().map(|(at, replace)| self.place_menu(at, replace, &t, cx));
        div()
            .id("device-tile")
            .debug_selector(|| "device-tile".into())
            .key_context("DeviceTile")
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .text_color(t.text)
            .on_key_down(cx.listener(Self::key))
            .child(self.header(&t, cx))
            .child(self.body(&t, cx))
            .children(menu)
    }
}

// ── wherever it is ───────────────────────────────────────────────────────

/// Every device tile, one per device, wherever it is drawn: moving it
/// keeps its stream.
#[derive(Default)]
struct Tiles(HashMap<String, Entity<DeviceTile>>);

impl gpui::Global for Tiles {}

/// Device `key`'s tile (made, and its stream started, when it isn't yet).
pub fn view(key: &str, cx: &mut App) -> Entity<DeviceTile> {
    if let Some(v) = get(key, cx) {
        return v;
    }
    let v = cx.new(|cx| DeviceTile::new(key, cx));
    cx.default_global::<Tiles>().0.insert(key.to_string(), v.clone());
    v
}

/// Device `key`'s tile, if it is made.
pub fn get(key: &str, cx: &App) -> Option<Entity<DeviceTile>> {
    cx.try_global::<Tiles>()?.0.get(key).cloned()
}

/// Device `key`'s tile closed: out of every layout, its window closed, its
/// stream stopped (the tile, and with it the session, let go).
pub fn close(key: &str, cx: &mut App) {
    let s = Surface::Device(key.to_string());
    crate::ui_state::update(cx, |ui| *ui = ui.clone().close_surface(&s));
    super::inbox_window::close_for_move(&s.key(), cx);
    if cx.has_global::<Tiles>() {
        cx.global_mut::<Tiles>().0.remove(key);
    }
}

/// Its title in a window of its own.
pub fn title(key: &str, cx: &App) -> String {
    let name = known(cx).iter().find(|d| d.key() == key).map(|d| d.name.clone()).unwrap_or_else(|| device::parse_key(key).map(|(_, id)| id).unwrap_or_default());
    format!("{name} — Device")
}

// ── the devices there are ────────────────────────────────────────────────

/// The devices last listed (adb and simctl), refreshed in the background
/// when the palette opens ([`refresh`]).
#[derive(Default)]
struct Known {
    devices: Vec<Device>,
    refreshing: bool,
}

impl gpui::Global for Known {}

/// The devices last listed.
pub fn known(cx: &App) -> Vec<Device> {
    cx.try_global::<Known>().map(|k| k.devices.clone()).unwrap_or_default()
}

/// Lists the devices again, off the UI thread (adb and simctl can take a
/// moment); the palette shows them the next time it opens. Nothing runs
/// while device screens are turned off.
pub fn refresh(cx: &mut App) {
    // Tests open the palette without asking this computer's adb.
    if cfg!(test) || !enabled(cx) || cx.try_global::<Known>().is_some_and(|k| k.refreshing) {
        return;
    }
    cx.default_global::<Known>().refreshing = true;
    let task = cx.background_executor().spawn(async move {
        let mut all = Vec::new();
        if let Ok(adb) = device::android::LocalAdb::new() {
            all.extend(device::android::list(&adb).unwrap_or_default());
        }
        if crate::platform::video::supported() {
            all.extend(device::ios::list().unwrap_or_default());
        }
        all
    });
    cx.spawn(async move |cx| {
        let devices = task.await;
        cx.update(|cx| {
            let k = cx.default_global::<Known>();
            k.devices = devices;
            k.refreshing = false;
        });
    })
    .detach();
}

impl MainScreen {
    /// "Open device: <name>" in the palette: device `key`'s tile focused
    /// where it is, else placed right of the focused tile (in it when it
    /// is empty), and its stream started.
    pub fn open_device(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !enabled(cx) {
            self.toast(Toast::info("Device screens are turned off", Some("Turn them on in Settings → General.".into())), cx);
            return;
        }
        let s = Surface::Device(key.to_string());
        let skey = s.key();
        let v = view(key, cx);
        let f = v.read(cx).focus.clone();
        if let Some(h) = crate::windows::inbox_window(&skey, cx) {
            cx.defer(move |cx| {
                let _ = h.update(cx, |_, window, cx| {
                    if !crate::bench::enabled() {
                        window.activate_window();
                    }
                    window.focus(&f, cx);
                });
            });
            return;
        }
        match self.ui.tile_of(&s.content()) {
            Some((space, pane)) if self.ui.window_of_space(&space) == self.label => {
                self.active = space.clone();
                self.set_route(super::Route::Space, cx);
                self.update_ui(|u| u.focus_pane(&space, &pane), cx);
            }
            _ => {
                let Some(space) = self.active_space().cloned() else { return };
                let anchor = space.focused_pane_id.clone().and_then(|p| space.layout.find_pane(&p));
                let spot = match anchor {
                    Some(p) if p.is_empty() => FilesSpot::Take(p.id),
                    Some(p) => FilesSpot::Tile { pane: p.id, side: super::tree::Side::Right },
                    None => FilesSpot::Auto,
                };
                let fit = self.fit_for(&space.id);
                let mut placed = false;
                self.update_ui(
                    |u| {
                        let (next, at) = u.surface_to_layout(&space.id, &s, &spot, fit);
                        placed = at.is_some();
                        match at {
                            Some(pane) => next.focus_pane(&space.id, &pane),
                            None => next,
                        }
                    },
                    cx,
                );
                if !placed {
                    return;
                }
            }
        }
        window.focus(&f, cx);
        cx.notify();
    }

    /// Tile `pane` showing device `key`.
    pub(super) fn render_device_pane(&mut self, pane: &str, key: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = view(key, cx);
        v.update(cx, |v, cx| v.held(false, cx));
        let keys = Some(v.read(cx).focus.clone());
        let body = div().flex_1().min_w_0().min_h_0().flex().child(v).into_any_element();
        self.tile_shell(pane, Content::Device(key.to_string()), format!("device-tile-{key}"), body, keys, t, cx)
    }
}
