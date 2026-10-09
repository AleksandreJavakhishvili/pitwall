//! Window decorations (docs/spec/gpui/platform.md "Title bar"; Tauri: the
//! native GTK / Win32 frame plus `data-tauri-drag-region` on the top bar).
//!
//! - Linux: Pitwall asks for server-side decorations where the compositor
//!   offers them (KDE, sway, every X11 window manager). Where it doesn't
//!   (GNOME's mutter, Weston), the window draws its own frame: the top bar
//!   is the title bar (drag to move, double-click to maximize, right-click
//!   for the window menu) with minimize / maximize / close at its right end,
//!   rounded top corners, a hairline border and a shadow while floating, and
//!   resize areas on every edge and corner. Tiled or maximized sides get
//!   none of these.
//! - Windows: the native caption (controls, snap layouts, system menu) as in
//!   the Tauri app, the in-window File menu below it ([`super::app_menu`]),
//!   and the top bar drags the window like the caption.
//! - macOS: unchanged (the native title bar).
//!
//! Hooks: [`request`] in `WindowOptions`, [`frame`] around a window's root
//! view, [`title_bar`] on the top bar (with [`no_drag`] on its controls), and
//! [`round_top`] on full-size layers that paint the window's top corners.

use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Bounds, CursorStyle, Decorations, Div,
    HitboxBehavior, MouseButton, Pixels, Point, ResizeEdge, Size, Stateful, Tiling, Window,
    WindowDecorations,
};

use crate::kit::{icon, tooltip, HoverText};
use crate::theme::{Mode, Theme, RADIUS, RADIUS_LG};

/// The invisible band around a client-decorated window: room for the shadow
/// and the resize areas.
pub const INSET: Pixels = px(10.);
/// The window's corner radius when client-decorated and floating.
pub const CORNER: Pixels = RADIUS_LG;
/// How far a corner's resize area reaches along each edge.
const CORNER_GRAB: f32 = 20.;

// ───────────────────────────────────────────── which decorations

/// What a new window asks for (`WindowOptions::window_decorations`).
///
/// Linux on Wayland: server-side when the compositor has the decoration
/// protocol, client-side otherwise (gpui 0.2.2 would otherwise report
/// server-side there although nothing draws a frame). X11: server-side.
/// Debug builds: `PITWALL_DECORATIONS=client|server` overrides.
pub fn request() -> Option<WindowDecorations> {
    #[cfg(target_os = "linux")]
    {
        let forced = if cfg!(debug_assertions) {
            std::env::var("PITWALL_DECORATIONS").ok()
        } else {
            None
        };
        let wayland = gpui::guess_compositor() == "Wayland";
        // Only probe when the answer matters.
        let server = |_: ()| linux_probe::server_side_available();
        Some(pick(forced.as_deref(), wayland, server))
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// The decision behind [`request`] (pure: the probe runs only on Wayland
/// without an override).
pub fn pick(
    forced: Option<&str>,
    wayland: bool,
    server_available: impl FnOnce(()) -> bool,
) -> WindowDecorations {
    match forced {
        Some("client") => return WindowDecorations::Client,
        Some("server") => return WindowDecorations::Server,
        _ => {}
    }
    if wayland && !server_available(()) {
        WindowDecorations::Client
    } else {
        WindowDecorations::Server
    }
}

#[cfg(target_os = "linux")]
mod linux_probe {
    //! Whether the Wayland compositor offers `zxdg_decoration_manager_v1`
    //! (server-side decorations): one registry round trip on a short-lived
    //! connection of our own, once per run.

    use std::sync::OnceLock;

    use wayland_client::protocol::wl_registry;
    use wayland_client::{Connection, Dispatch, QueueHandle};

    struct Probe {
        decorations: bool,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Probe {
        fn event(
            state: &mut Self,
            _: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global { interface, .. } = event {
                if interface == "zxdg_decoration_manager_v1" {
                    state.decorations = true;
                }
            }
        }
    }

    fn probe() -> Option<bool> {
        let conn = Connection::connect_to_env().ok()?;
        let mut queue = conn.new_event_queue();
        let _registry = conn.display().get_registry(&queue.handle(), ());
        let mut state = Probe { decorations: false };
        queue.roundtrip(&mut state).ok()?;
        Some(state.decorations)
    }

    /// Unknown (no connection): assume yes and let gpui's reply handling
    /// decide, as before.
    pub fn server_side_available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| probe().unwrap_or(true))
    }
}

/// The window draws its own frame right now (Linux, client-side).
fn client_side(window: &Window) -> Option<Tiling> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    match window.window_decorations() {
        Decorations::Client { tiling } => Some(tiling),
        Decorations::Server => None,
    }
}

// ───────────────────────────────────────────── the frame

/// Wrap a window's root view: on Linux with client-side decorations, the
/// shadow band with resize areas around a rounded, bordered window; on
/// Windows, the File menu bar above it. Unchanged otherwise.
pub fn frame(root: Stateful<Div>, window: &mut Window, cx: &mut App) -> AnyElement {
    if super::app_menu::shown() {
        let menu = menu_for(window, cx);
        let t = crate::theme::theme(cx).chrome();
        return div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("menu-bar")
                    .h(px(26.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(4.))
                    .bg(t.surface)
                    .border_b_1()
                    .border_color(t.line)
                    .child(menu),
            )
            .child(root.w_full().h_auto().flex_1().min_h_0())
            .into_any_element();
    }
    let Some(tiling) = client_side(window) else {
        return root.into_any_element();
    };
    window.set_client_inset(INSET);
    let t = crate::theme::theme(cx);
    let shadow = shadow_color(t.mode, window.is_window_active());
    let border = t.line_strong;
    let root = round_tiled(root, tiling, CORNER)
        .border_color(border)
        .when(!tiling.top, |d| d.border_t_1())
        .when(!tiling.bottom, |d| d.border_b_1())
        .when(!tiling.left, |d| d.border_l_1())
        .when(!tiling.right, |d| d.border_r_1())
        .when(!tiling.is_tiled(), |d| {
            d.shadow(vec![gpui::BoxShadow {
                color: shadow,
                offset: gpui::point(px(0.), px(2.)),
                blur_radius: px(8.),
                spread_radius: px(0.),
            }])
        });
    div()
        .id("window-frame")
        .size_full()
        .flex()
        .relative()
        .when(!tiling.top, |d| d.pt(INSET))
        .when(!tiling.bottom, |d| d.pb(INSET))
        .when(!tiling.left, |d| d.pl(INSET))
        .when(!tiling.right, |d| d.pr(INSET))
        .child(
            canvas(
                move |bounds, window, _| {
                    zones(bounds.size, tiling)
                        .into_iter()
                        .map(|(b, edge)| {
                            let b = Bounds::new(bounds.origin + b.origin, b.size);
                            (window.insert_hitbox(b, HitboxBehavior::Normal), edge)
                        })
                        .collect::<Vec<_>>()
                },
                |_, hitboxes, window, _| {
                    for (hitbox, edge) in hitboxes {
                        window.set_cursor_style(cursor(edge), &hitbox);
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .on_mouse_down(MouseButton::Left, move |e, window, _| {
            if let Some(edge) = resize_edge(e.position, window.viewport_size(), tiling) {
                window.start_window_resize(edge);
            }
        })
        .child(root)
        .into_any_element()
}

/// Each window's File menu (it keeps its open state between frames).
#[derive(Default)]
struct MenuBars(std::collections::HashMap<gpui::WindowId, gpui::Entity<super::app_menu::AppMenu>>);

impl gpui::Global for MenuBars {}

fn menu_for(window: &Window, cx: &mut App) -> gpui::Entity<super::app_menu::AppMenu> {
    let id = window.window_handle().window_id();
    if let Some(menu) = cx.default_global::<MenuBars>().0.get(&id) {
        return menu.clone();
    }
    let open: Vec<_> = cx.windows().iter().map(|w| w.window_id()).collect();
    let menu = cx.new(|_| super::app_menu::AppMenu::new());
    let bars = cx.default_global::<MenuBars>();
    bars.0.retain(|w, _| open.contains(w));
    bars.0.insert(id, menu.clone());
    menu
}

fn shadow_color(mode: Mode, active: bool) -> gpui::Hsla {
    let alpha = match (mode, active) {
        (Mode::Dark, true) => 0.55,
        (Mode::Dark, false) => 0.35,
        (Mode::Light, true) => 0.28,
        (Mode::Light, false) => 0.16,
    };
    gpui::hsla(0., 0., 0., alpha)
}

/// Round the top corners that aren't against a tiled edge.
fn round_tiled<E: Styled + FluentBuilder>(e: E, tiling: Tiling, r: Pixels) -> E {
    e.when(!(tiling.top || tiling.left), |d| d.rounded_tl(r))
        .when(!(tiling.top || tiling.right), |d| d.rounded_tr(r))
}

/// For a full-size layer that paints the window's top corners (the main
/// screen's background, the top bar): the frame's rounding, inside its
/// 1px border.
pub fn round_top<E: Styled + FluentBuilder>(e: E, window: &Window) -> E {
    match client_side(window) {
        Some(tiling) => round_tiled(e, tiling, CORNER - px(1.)),
        None => e,
    }
}

/// The resize areas in the shadow band of a `size` window: corners first
/// (they win over the edges they overlap).
pub fn zones(size: Size<Pixels>, tiling: Tiling) -> Vec<(Bounds<Pixels>, ResizeEdge)> {
    let (w, h, i) = (
        f32::from(size.width),
        f32::from(size.height),
        f32::from(INSET),
    );
    let g = CORNER_GRAB.min(w / 2.).min(h / 2.);
    let rect = |x: f32, y: f32, rw: f32, rh: f32| {
        Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(rw), px(rh)))
    };
    let (top, bottom, left, right) = (!tiling.top, !tiling.bottom, !tiling.left, !tiling.right);
    let mut out = Vec::new();
    let mut corner = |on: bool, a: Bounds<Pixels>, b: Bounds<Pixels>, edge| {
        if on {
            out.push((a, edge));
            out.push((b, edge));
        }
    };
    corner(
        top && left,
        rect(0., 0., g, i),
        rect(0., 0., i, g),
        ResizeEdge::TopLeft,
    );
    corner(
        top && right,
        rect(w - g, 0., g, i),
        rect(w - i, 0., i, g),
        ResizeEdge::TopRight,
    );
    corner(
        bottom && left,
        rect(0., h - i, g, i),
        rect(0., h - g, i, g),
        ResizeEdge::BottomLeft,
    );
    corner(
        bottom && right,
        rect(w - g, h - i, g, i),
        rect(w - i, h - g, i, g),
        ResizeEdge::BottomRight,
    );
    if top {
        out.push((rect(0., 0., w, i), ResizeEdge::Top));
    }
    if bottom {
        out.push((rect(0., h - i, w, i), ResizeEdge::Bottom));
    }
    if left {
        out.push((rect(0., 0., i, h), ResizeEdge::Left));
    }
    if right {
        out.push((rect(w - i, 0., i, h), ResizeEdge::Right));
    }
    out
}

/// The edge or corner a press at `pos` resizes, if any.
pub fn resize_edge(pos: Point<Pixels>, size: Size<Pixels>, tiling: Tiling) -> Option<ResizeEdge> {
    zones(size, tiling)
        .into_iter()
        .find(|(b, _)| b.contains(&pos))
        .map(|(_, e)| e)
}

fn cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

// ───────────────────────────────────────────── the title bar

thread_local! {
    /// Where a left press on the bar's empty area went down (Wayland): the
    /// move starts once the pointer leaves it, so a plain click stays a
    /// click and the compositor's grab doesn't eat the release.
    static PRESS: std::cell::Cell<Option<Point<Pixels>>> = const { std::cell::Cell::new(None) };
}

/// The pointer has to travel this far before a press becomes a move.
const MOVE_THRESHOLD: f32 = 3.;

/// Make the top bar the title bar: drag to move and double-click to
/// maximize (as `data-tauri-drag-region` does on Linux and Windows; on X11
/// only the drag, see [`begin_move`]),
/// right-click for the window menu (Linux), the window controls at its
/// right end and the frame's rounded corners when client-decorated.
/// Presses on its controls never get here ([`no_drag`]).
pub fn title_bar(bar: Stateful<Div>, window: &Window, t: &Theme) -> Stateful<Div> {
    let bar = bar.window_control_area(gpui::WindowControlArea::Drag);
    if cfg!(target_os = "macos") {
        return bar;
    }
    let controls = controls(window, t);
    round_top(bar, window)
        .children(controls)
        .on_mouse_down(MouseButton::Left, |e, window, _| {
            if e.click_count >= 2 {
                PRESS.set(None);
                toggle_maximize(window);
            } else {
                begin_move(e.position, window);
            }
        })
        .on_mouse_move(|e, window, _| {
            let Some(at) = PRESS.get() else { return };
            if e.pressed_button != Some(MouseButton::Left) {
                PRESS.set(None);
            } else if (e.position - at).magnitude() > MOVE_THRESHOLD as f64 {
                PRESS.set(None);
                window.start_window_move();
            }
        })
        .on_mouse_up(MouseButton::Left, |_, _, _| PRESS.set(None))
        .on_mouse_down(MouseButton::Right, |e, window, _| {
            if cfg!(target_os = "linux") && window.window_controls().window_menu {
                window.show_window_menu(e.position);
            }
        })
}

/// Keep presses on a control inside the top bar from moving the window or
/// opening the window menu (the control still gets its click).
pub fn no_drag<E: InteractiveElement>(e: E) -> E {
    e.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

fn begin_move(at: Point<Pixels>, window: &mut Window) {
    #[cfg(windows)]
    {
        let _ = at;
        win32::drag(window);
    }
    #[cfg(not(windows))]
    {
        // X11: hand the press to the window manager right away (asked
        // for later, from a motion event, the move doesn't start). The
        // manager then keeps the release, so a double click on the bar
        // isn't seen there; its own title bar has it.
        #[cfg(target_os = "linux")]
        if gpui::guess_compositor() == "X11" {
            window.start_window_move();
            return;
        }
        let _ = window;
        PRESS.set(Some(at));
    }
}

fn toggle_maximize(window: &mut Window) {
    #[cfg(windows)]
    win32::toggle_maximize(window);
    #[cfg(not(windows))]
    window.zoom_window();
}

/// Minimize / maximize (restore) / close, when the window draws its own
/// frame; only the ones the compositor supports.
pub fn controls(window: &Window, t: &Theme) -> Option<AnyElement> {
    client_side(window)?;
    let can = window.window_controls();
    let maximized = window.is_maximized();
    let glyph_color = t.text_3;
    Some(
        no_drag(
            div()
                .id("window-controls")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .child(div().w(px(1.)).h(px(16.)).mx(px(4.)).bg(t.line))
                .when(can.minimize, |d| {
                    d.child(
                        control_btn("wc-min", "Minimize", t, false)
                            .child(
                                div()
                                    .w(px(9.))
                                    .h(px(1.25))
                                    .rounded(px(1.))
                                    .bg(glyph_color)
                                    .group_hover_probed("wc-min", |s| s.bg(t.text)),
                            )
                            .on_click(|_, window, _| window.minimize_window()),
                    )
                })
                .when(can.maximize, |d| {
                    let (name, tip) = if maximized {
                        ("copy", "Restore")
                    } else {
                        ("stop", "Maximize")
                    };
                    d.child(
                        control_btn("wc-max", tip, t, false)
                            .child(icon(name, 16., glyph_color).group_hover_text(
                                "wc-max",
                                t.text,
                                |s| s,
                            ))
                            .on_click(|_, window, _| window.zoom_window()),
                    )
                })
                .child(
                    control_btn("wc-close", "Close", t, true)
                        .child(icon("x", 16., glyph_color).group_hover_text(
                            "wc-close",
                            t.red,
                            |s| s,
                        ))
                        .on_click(|_, window, cx| {
                            let handle = window.window_handle();
                            cx.defer(move |cx| super::close_window(handle, cx));
                        }),
                ),
        )
        .into_any_element(),
    )
}

/// One window control: an `.icon-btn` (28 px, surface-3 on hover; the
/// close button turns red-soft like `.danger-btn`).
fn control_btn(id: &'static str, tip: &'static str, t: &Theme, danger: bool) -> Stateful<Div> {
    let hover_bg = if danger { t.red_soft } else { t.surface_3 };
    div()
        .id(id)
        .group(id)
        .size(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS)
        .cursor_pointer()
        .hover_probed(move |s| s.bg(hover_bg))
        .tooltip(tooltip(tip))
}

#[cfg(windows)]
mod win32 {
    //! The top bar as a caption on Windows: what Tauri's drag region does
    //! (hand the press to the window's own caption handling, so Aero Snap
    //! and the double-click work as on the native title bar).

    use gpui::Window;
    use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, IsZoomed, PostMessageW, HTCAPTION, SC_MAXIMIZE, SC_RESTORE, WM_NCLBUTTONDOWN,
        WM_SYSCOMMAND,
    };

    pub fn drag(window: &Window) {
        let Some(hwnd) = super::super::windows::hwnd(window) else {
            return;
        };
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
            let _ = ReleaseCapture();
            // Posted, not sent: the move loop must not run inside gpui's
            // event dispatch.
            let lparam = ((p.y as u32 & 0xffff) << 16) | (p.x as u32 & 0xffff);
            let _ = PostMessageW(
                Some(hwnd),
                WM_NCLBUTTONDOWN,
                WPARAM(HTCAPTION as usize),
                LPARAM(lparam as isize),
            );
        }
    }

    pub fn toggle_maximize(window: &Window) {
        let Some(hwnd) = super::super::windows::hwnd(window) else {
            return;
        };
        unsafe {
            let cmd = if IsZoomed(hwnd).as_bool() {
                SC_RESTORE
            } else {
                SC_MAXIMIZE
            };
            let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd as usize), LPARAM(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    fn sz() -> Size<Pixels> {
        size(px(800.), px(600.))
    }

    #[test]
    fn decorations_follow_the_compositor() {
        let yes = |_| true;
        let no = |_| false;
        assert_eq!(pick(None, true, no), WindowDecorations::Client);
        assert_eq!(pick(None, true, yes), WindowDecorations::Server);
        // X11: the window manager decorates; the probe isn't consulted.
        assert_eq!(
            pick(None, false, |_| panic!("no probe on X11")),
            WindowDecorations::Server
        );
        assert_eq!(pick(Some("client"), false, yes), WindowDecorations::Client);
        assert_eq!(pick(Some("server"), true, no), WindowDecorations::Server);
    }

    #[test]
    fn every_edge_and_corner_resizes_when_floating() {
        let t = Tiling::default();
        let at = |x: f32, y: f32| resize_edge(point(px(x), px(y)), sz(), t);
        assert_eq!(at(2., 2.), Some(ResizeEdge::TopLeft));
        assert_eq!(at(15., 3.), Some(ResizeEdge::TopLeft));
        assert_eq!(at(400., 3.), Some(ResizeEdge::Top));
        assert_eq!(at(795., 5.), Some(ResizeEdge::TopRight));
        assert_eq!(at(797., 300.), Some(ResizeEdge::Right));
        assert_eq!(at(797., 590.), Some(ResizeEdge::BottomRight));
        assert_eq!(at(400., 595.), Some(ResizeEdge::Bottom));
        assert_eq!(at(3., 585.), Some(ResizeEdge::BottomLeft));
        assert_eq!(at(3., 300.), Some(ResizeEdge::Left));
        assert_eq!(at(400., 300.), None, "inside the window");
        assert_eq!(at(11., 11.), None, "just inside the corner");
    }

    #[test]
    fn tiled_sides_have_no_resize_areas() {
        let t = Tiling {
            top: true,
            left: true,
            right: false,
            bottom: false,
        };
        let at = |x: f32, y: f32| resize_edge(point(px(x), px(y)), sz(), t);
        assert_eq!(at(2., 2.), None);
        assert_eq!(at(400., 3.), None);
        assert_eq!(at(3., 300.), None);
        assert_eq!(at(797., 5.), Some(ResizeEdge::Right));
        assert_eq!(at(797., 595.), Some(ResizeEdge::BottomRight));
        assert_eq!(at(3., 595.), Some(ResizeEdge::Bottom));
        assert!(zones(sz(), Tiling::tiled()).is_empty(), "maximized");
    }

    #[test]
    fn the_frame_is_rounded_only_away_from_tiled_edges() {
        let r = |t: Tiling| {
            let mut s = round_tiled(div(), t, CORNER);
            let c = s.style().corner_radii.clone();
            (c.top_left.is_some(), c.top_right.is_some())
        };
        assert_eq!(r(Tiling::default()), (true, true));
        assert_eq!(
            r(Tiling {
                left: true,
                ..Default::default()
            }),
            (false, true)
        );
        assert_eq!(r(Tiling::tiled()), (false, false));
    }

    #[test]
    fn shadows_are_softer_when_inactive_and_in_light() {
        assert!(shadow_color(Mode::Dark, true).a > shadow_color(Mode::Dark, false).a);
        assert!(shadow_color(Mode::Light, true).a < shadow_color(Mode::Dark, true).a);
    }
}
