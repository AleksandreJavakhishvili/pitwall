//! Extensions' toolbar buttons (`contributes.toolbar`,
//! docs/spec/tile-sdk/platform-services.md §12): at most one per
//! extension that is on, in a group of their own after the top bar's
//! counts, behind a hairline.
//!
//! - Shown when the user said so (`ui.json`
//!   `extensions.<id>.toolbar`), else by its `visibleByDefault`. Hide from
//!   toolbar (its right-click menu) keeps it hidden: an update never shows
//!   it again. Its tile may hide it for a while (`toolbar.set-shown`).
//! - Its badge (`toolbar.set-badge`) shows only while its component runs;
//!   with `"badge": true` Pitwall keeps the component running for it
//!   ([`crate::tiles::Tiles::keep`]).
//! - A press runs its action: opens its tile, toggles its popover (a
//!   view of its tile, [`POPOVER_VIEW`], sized as the manifest says within
//!   [`toolbar::POPOVER_MIN`]..[`toolbar::POPOVER_MAX`]), runs its command
//!   or opens the menu of its commands. The tile hears `toolbar-clicked`.
//!   Esc, a press outside, a press on the button or the tile's
//!   `toolbar.close-popover` closes the popover.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, App, Context, Entity, Global, IntoElement, MouseButton, MouseDownEvent, SharedString, Window};
use pitwall_core::extensions::toolbar::{self, Action, ToolbarDef};
use pitwall_core::extensions::Manifest;
use pitwall_tile_host::ui::{BadgeTone, ToolbarBadge as Badge};
use pitwall_tile_host::{ToolbarRequest, POPOVER_VIEW};

use super::menu::MenuItem;
use super::MainScreen;
use crate::kit::{icon_btn_state, tooltip};
use crate::theme::{Theme, RADIUS};
use crate::tiles::TileView;

/// What an extension's tile set on its button.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub badge: Option<(Badge, BadgeTone)>,
    /// A project's badge, by project (`toolbar.set-project-badge`).
    pub projects: HashMap<String, (Badge, BadgeTone)>,
    pub tooltip: Option<String>,
    /// The tile hid its button (`toolbar.set-shown(false)`).
    pub hidden_by_tile: bool,
}

/// Every extension's button state, as their tiles set it.
#[derive(Default)]
pub struct Items(pub HashMap<String, Item>);

impl Global for Items {}

/// A button the top bar shows.
#[derive(Debug, Clone)]
pub struct Shown {
    pub ext: String,
    pub name: String,
    pub def: ToolbarDef,
    pub item: Item,
    /// Its component runs: its badge counts.
    pub running: bool,
}

/// Whether extension `m`'s button shows, as the user set it, else its
/// `visibleByDefault`.
pub fn visible(m: &Manifest, cx: &App) -> bool {
    let Some(def) = &m.contributes.toolbar else { return false };
    crate::ui_state::get(cx).extension(&m.id).toolbar.unwrap_or(def.visible_by_default)
}

/// The buttons shown now, in folder order.
pub fn shown(cx: &App) -> Vec<Shown> {
    let ui = crate::ui_state::get(cx);
    let items = cx.try_global::<Items>();
    let tiles = crate::tiles::try_tiles(cx);
    let mut out = Vec::new();
    for m in crate::extensions::enabled(&crate::extensions::found(cx), &ui) {
        let Some(def) = m.contributes.toolbar.clone() else { continue };
        if !visible(&m, cx) {
            continue;
        }
        let item = items.and_then(|i| i.0.get(&m.id)).cloned().unwrap_or_default();
        if item.hidden_by_tile {
            continue;
        }
        let running = tiles.as_ref().is_some_and(|t| t.read(cx).get(&m.id).is_some_and(|r| r.state == crate::tiles::State::Running));
        out.push(Shown { ext: m.id.clone(), name: m.name.clone(), def, item, running });
    }
    out
}

/// Apply a toolbar change extension `ext`'s tile asked for.
pub fn apply(ext: &str, req: ToolbarRequest, cx: &mut App) {
    if let ToolbarRequest::ClosePopover = req {
        let ext = ext.to_string();
        crate::windows::each_screen(cx, move |s, cx| s.close_toolbar_popover(Some(&ext), cx));
        return;
    }
    let item = cx.default_global::<Items>().0.entry(ext.to_string()).or_default();
    match req {
        ToolbarRequest::Badge { project: None, badge, tone } => item.badge = Some((badge, tone)),
        ToolbarRequest::Badge { project: Some(p), badge, tone } => {
            item.projects.insert(p, (badge, tone));
        }
        ToolbarRequest::Tooltip(t) => item.tooltip = (!t.is_empty()).then_some(t),
        ToolbarRequest::Shown(b) => item.hidden_by_tile = !b,
        ToolbarRequest::ClosePopover => {}
    }
    cx.refresh_windows();
}

/// The popover's size: the manifest's, within the limits.
pub fn popover_size(def: &ToolbarDef) -> Option<(f32, f32)> {
    let Action::Popover { width, height, .. } = &def.action else { return None };
    let w = (*width).clamp(toolbar::POPOVER_MIN.0, toolbar::POPOVER_MAX.0);
    let h = (*height).clamp(toolbar::POPOVER_MIN.1, toolbar::POPOVER_MAX.1);
    Some((w as f32, h as f32))
}

/// The open popover: whose, its view, its size.
pub(super) struct ToolbarPop {
    pub ext: String,
    view: Entity<TileView>,
    size: (f32, f32),
}

/// What a badge says: `None` for none, `Some("")` for a dot.
fn badge_text(b: Badge) -> Option<String> {
    match b {
        Badge::None => None,
        Badge::Dot => Some(String::new()),
        Badge::Count(0) => None,
        Badge::Count(n) if n > 99 => Some("99+".into()),
        Badge::Count(n) => Some(n.to_string()),
    }
}

fn badge_el(b: Badge, tone: BadgeTone, t: &Theme) -> Option<gpui::Div> {
    let text = badge_text(b)?;
    let (bg, fg) = match tone {
        BadgeTone::Attention => (t.amber, t.bg),
        BadgeTone::Normal => (t.surface_3, t.text),
    };
    let el = div().absolute().flex().items_center().justify_center().rounded(px(8.)).bg(bg).border_1().border_color(t.surface);
    Some(if text.is_empty() {
        el.top(px(4.)).right(px(4.)).size(px(8.))
    } else {
        el.top(px(-2.)).right(px(-4.)).min_w(px(15.)).h(px(15.)).px(px(3.)).font_family(crate::kit::MONO_FONT).text_size(px(9.5)).text_color(fg).child(text)
    })
}

impl MainScreen {
    /// The group of extensions' buttons (with its hairline), or nothing
    /// when none shows.
    pub(super) fn render_toolbar_items(&mut self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let items = shown(cx);
        // Kept running for their badges.
        let keep: Vec<String> = items.iter().filter(|s| s.def.badge && !s.running).map(|s| s.ext.clone()).collect();
        if !keep.is_empty() {
            cx.defer(move |cx| {
                let tiles = crate::tiles::tiles(cx);
                tiles.update(cx, |t, cx| keep.iter().for_each(|e| t.keep(e, cx)));
            });
        }
        if items.is_empty() {
            return None;
        }
        let project = self.active_space().and_then(|s| s.project.clone());
        let keep_press = |_: &MouseDownEvent, _: &mut Window, cx: &mut App| cx.stop_propagation();
        let buttons = items.into_iter().map(|s| {
            let open = self.toolbar_pop.as_ref().is_some_and(|p| p.ext == s.ext);
            let tip = s.item.tooltip.clone().unwrap_or_else(|| if s.def.tooltip.is_empty() { s.name.clone() } else { s.def.tooltip.clone() });
            // No icon of its own: the extension's (its image, when it has one).
            let icon = if s.def.icon.is_empty() { crate::extensions::manifest_of(&s.ext, cx).and_then(|m| crate::kit::icons::known(crate::extensions::icon_of(&m))) } else { crate::kit::icons::known(&s.def.icon) }.unwrap_or("extension");
            let badge = s.running.then(|| {
                let project_badge = (s.def.scope == toolbar::Scope::Project).then(|| project.as_ref().and_then(|p| s.item.projects.get(p).copied())).flatten();
                project_badge.or(s.item.badge)
            });
            let (ext, ext2) = (s.ext.clone(), s.ext.clone());
            let btn = icon_btn_state(format!("tb-ext-{}", s.ext), icon, "", false, 16., if open { Some(true) } else { None }, t)
                .on_mouse_down(MouseButton::Left, keep_press)
                .on_click(cx.listener(move |this, _, window, cx| this.press_toolbar(&ext, window, cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.toolbar_item_menu(&ext2, e.position, window, cx);
                }))
                .tooltip(tooltip(SharedString::from(tip)));
            let pop = if open { self.render_toolbar_pop(t, cx) } else { None };
            div()
                .relative()
                .flex_none()
                .child(btn)
                .children(badge.flatten().and_then(|(b, tone)| badge_el(b, tone, t)))
                .children(pop)
        });
        Some(
            div()
                .id("tb-extensions")
                .flex()
                .items_center()
                .gap(px(2.))
                .flex_none()
                .child(div().w(px(1.)).h(px(16.)).mr(px(4.)).bg(t.line))
                .children(buttons)
                .into_any_element(),
        )
    }

    /// Extension `ext`'s button was pressed: its action runs.
    pub fn press_toolbar(&mut self, ext: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = crate::extensions::manifest_of(ext, cx) else { return };
        let Some(def) = m.contributes.toolbar.clone() else { return };
        let project = self.active_space().and_then(|s| s.project.clone());
        let clicked = |cx: &mut Context<Self>| {
            let tiles = crate::tiles::tiles(cx);
            tiles.read(cx).send(ext, pitwall_tile_host::ui::Event::ToolbarClicked(project.clone()));
        };
        match &def.action {
            Action::Tile(tile) => {
                self.open_view(ext, tile, None, window, cx);
                clicked(cx);
            }
            Action::Popover { .. } => {
                let just_closed = self.toolbar_dismissed.take().is_some_and(|(e, at)| e == ext && at.elapsed() < Duration::from_millis(500));
                if self.toolbar_pop.as_ref().is_some_and(|p| p.ext == ext) {
                    self.close_toolbar_popover(None, cx);
                } else if !just_closed {
                    let size = popover_size(&def).unwrap_or((toolbar::POPOVER_DEFAULT.0 as f32, toolbar::POPOVER_DEFAULT.1 as f32));
                    let e = ext.to_string();
                    let view = cx.new(|cx| TileView::new(&e, POPOVER_VIEW, cx));
                    self.menu = None;
                    self.toolbar_pop = Some(ToolbarPop { ext: ext.to_string(), view, size });
                    clicked(cx);
                    cx.notify();
                }
            }
            Action::Command(cmd) => {
                let title = m.contributes.commands.iter().find(|c| &c.id == cmd).map(|c| c.title.clone()).unwrap_or_else(|| cmd.clone());
                self.extension_command(ext, cmd, &title, false, cx);
            }
            Action::Menu(cmds) => {
                let items = cmds
                    .iter()
                    .filter_map(|id| m.contributes.commands.iter().find(|c| &c.id == id))
                    .map(|c| {
                        let (ext, id, title) = (ext.to_string(), c.id.clone(), c.title.clone());
                        MenuItem::new("extension", &c.title, move |s, _, cx| s.extension_command(&ext, &id, &title, false, cx))
                    })
                    .collect();
                let at = window.mouse_position();
                self.open_menu(at, items, window, cx);
            }
        }
    }

    /// Its right-click menu: Hide from toolbar.
    fn toolbar_item_menu(&mut self, ext: &str, at: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let ext = ext.to_string();
        let items = vec![MenuItem::new("minus", "Hide from toolbar", move |s, _, cx| s.set_toolbar_shown(&ext, false, cx))];
        self.open_menu(at, items, window, cx);
    }

    /// The user shows or hides extension `ext`'s button (kept in
    /// `ui.json`; an update doesn't change it).
    pub fn set_toolbar_shown(&mut self, ext: &str, shown: bool, cx: &mut Context<Self>) {
        if !shown {
            self.close_toolbar_popover(Some(ext), cx);
        }
        let ext = ext.to_string();
        self.update_ui(|s| s.with_extension(&ext, |e| e.toolbar = Some(shown)), cx);
        cx.notify();
    }

    /// Close the popover (only extension `ext`'s, when given).
    pub fn close_toolbar_popover(&mut self, ext: Option<&str>, cx: &mut Context<Self>) {
        if self.toolbar_pop.as_ref().is_some_and(|p| ext.is_none_or(|e| e == p.ext)) {
            self.toolbar_pop = None;
            cx.notify();
        }
    }

    fn dismiss_toolbar_popover(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.toolbar_pop.take() {
            self.toolbar_dismissed = Some((p.ext, Instant::now()));
            cx.notify();
        }
    }

    fn render_toolbar_pop(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.toolbar_pop.as_ref()?;
        let (w, h) = p.size;
        let panel = div()
            .id("tb-ext-popover")
            .occlude()
            .w(px(w))
            .h(px(h))
            .overflow_hidden()
            .rounded(RADIUS)
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .shadow(t.float_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_toolbar_popover(cx)))
            .child(p.view.clone());
        Some(
            div()
                .absolute()
                .right_0()
                .top(px(32.))
                .child(div().absolute().right_0().w(px(w)).child(deferred(anchored().snap_to_window_with_margin(px(4.)).child(crate::kit::motion::enter("tb-ext-pop-in", crate::kit::motion::Fx::MENU, panel))).with_priority(2)))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_say_little() {
        assert_eq!(badge_text(Badge::None), None);
        assert_eq!(badge_text(Badge::Count(0)), None);
        assert_eq!(badge_text(Badge::Dot).as_deref(), Some(""));
        assert_eq!(badge_text(Badge::Count(7)).as_deref(), Some("7"));
        assert_eq!(badge_text(Badge::Count(240)).as_deref(), Some("99+"));
    }

    #[test]
    fn a_popover_keeps_within_its_limits() {
        let def = |width, height| ToolbarDef { icon: "server".into(), tooltip: "S".into(), badge: false, scope: toolbar::Scope::Window, visible_by_default: true, action: Action::Popover { tile: "main".into(), width, height } };
        assert_eq!(popover_size(&def(300, 200)), Some((300., 200.)));
        assert_eq!(popover_size(&def(9999, 10)), Some((420., 100.)));
        assert_eq!(popover_size(&ToolbarDef { action: Action::Tile("main".into()), ..def(1, 1) }), None);
    }
}
