//! One Wall tile (Tauri: `WallTile` in `src/components/Wall.tsx`): status
//! glyph, name, status word, blocked detail and diffstat over the agent's
//! screen. Each tile is its own entity drawn as a cached view, so a frame
//! repaints only its tile.

use gpui::{
    div, prelude::*, px, BoxShadow, Context, FontWeight, Hsla, IntoElement, Render, SharedString,
    WeakEntity, Window,
};

use pitwall_proto::{AgentView, Status};

use super::paint::{screen_view, term_colors, ScreenOpts};
use super::screen::Screen;
use super::WallView;
use crate::kit;
pub use crate::main_screen::AgentDrag;
use crate::theme::{self, Theme};

pub struct WallTile {
    pub agent: AgentView,
    pub screen: Screen,
    /// The screen could not be watched (the agent is gone or not running).
    pub failed: bool,
    /// The keyboard selection.
    pub selected: bool,
    pub font_size: f32,
    pub body_h: f32,
    /// Draw the cursor (the agent's own terminal would show it).
    pub show_cursor: bool,
    wall: WeakEntity<WallView>,
}

impl WallTile {
    pub fn new(agent: AgentView, font_size: f32, body_h: f32, wall: WeakEntity<WallView>) -> Self {
        WallTile {
            agent,
            screen: Screen::default(),
            failed: false,
            selected: false,
            font_size,
            body_h,
            show_cursor: false,
            wall,
        }
    }

    /// The words over the body when there is no screen to show.
    pub fn off_label(&self) -> Option<&'static str> {
        if self.screen.has_frame() {
            return None;
        }
        if !self.agent.running || self.failed {
            return Some(if self.agent.status == Status::Stopped {
                "STOPPED"
            } else {
                "NOT RUNNING"
            });
        }
        None
    }
}

/// `--amber-glow`.
fn amber_glow(t: &Theme) -> Hsla {
    t.amber.opacity(if t.mode == theme::Mode::Dark {
        0.38
    } else {
        0.35
    })
}

/// A 1 px ring outside the tile, and the blocked glow
/// (`0 0 26px -6px var(--amber-glow)`).
fn ring_shadows(ring: Hsla, glow: Option<Hsla>) -> Vec<BoxShadow> {
    let mut v = vec![BoxShadow {
        color: ring,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(1.),
    }];
    if let Some(color) = glow {
        v.push(BoxShadow {
            color,
            offset: gpui::point(px(0.), px(0.)),
            // CSS blur radius = 2 sigma; gpui's is the sigma.
            blur_radius: px(13.),
            spread_radius: px(-6.),
        });
    }
    v
}

impl Render for WallTile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let a = &self.agent;
        let blocked = a.status == Status::Blocked;
        let faded = matches!(a.status, Status::Exited | Status::Stopped);
        let id = a.id.clone();
        let wall = self.wall.clone();
        let tip: SharedString =
            format!("Go to {} · drag onto a space tab to move it there", a.name).into();
        let drag = AgentDrag {
            id: a.id.clone(),
            name: a.name.clone().into(),
        };
        let ring = if self.selected {
            t.focus
        } else if blocked {
            t.amber.opacity(0.6)
        } else {
            t.line
        };
        let head_bg = if blocked {
            t.surface.blend(t.amber_soft)
        } else {
            t.surface
        };

        let head = div()
            .h(px(32.))
            .flex_none()
            .px(px(10.))
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .border_b_1()
            .border_color(t.line)
            .bg(head_bg)
            .child(kit::glyph(a.status, &t, false))
            .child(
                div()
                    .flex_none()
                    .max_w(px(220.))
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight(650.))
                    .text_color(t.text)
                    .child(a.name.clone()),
            )
            .child(kit::status_label(a.status, &t))
            .when_some(a.status_detail.clone().filter(|_| blocked), |d, detail| {
                d.child(
                    div()
                        .min_w_0()
                        .flex_shrink()
                        .truncate()
                        .font_family(kit::MONO_FONT)
                        .text_size(px(11.5))
                        .text_color(t.amber)
                        .child(detail),
                )
            })
            .child(div().flex_1())
            .child(kit::diffstat(a.added, a.removed, &t));

        let colors = term_colors(t.mode);
        let body = div()
            .relative()
            .h(px(self.body_h))
            .flex_none()
            .overflow_hidden()
            .when(self.screen.has_frame(), |d| {
                d.child(screen_view(
                    &self.screen,
                    ScreenOpts {
                        font_size: self.font_size,
                        colors,
                        show_cursor: self.show_cursor,
                    },
                ))
            })
            .when_some(self.off_label(), |d, label| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(kit::label(label, t.text_4, 13.)),
                )
            });

        div()
            .id(SharedString::from(format!("wall-tile-{}", a.id)))
            .size_full()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(t.term_bg)
            // `box-shadow: 0 0 0 1px`: a ring outside the tile, as in wall.css.
            .shadow(ring_shadows(ring, blocked.then(|| amber_glow(&t))))
            .when(faded, |d| d.opacity(0.6))
            .when(!self.selected && !blocked, |d| {
                d.hover(|s| s.shadow(ring_shadows(t.text_4, None)))
            })
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let id = id.clone();
                let _ = wall.update(cx, |w, cx| w.open(id, window, cx));
            })
            .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
            .tooltip(kit::tooltip(tip))
            .child(head)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::agent;

    #[test]
    fn stopped_tiles_say_why_until_a_screen_exists() {
        let mut a = agent("a", "/work/alpha", "stopped", 0, true);
        a.running = false;
        let mut tile = WallTile::new(a, 13., 250., WeakEntity::new_invalid());
        assert_eq!(tile.off_label(), Some("STOPPED"));
        tile.agent.status = Status::Exited;
        assert_eq!(tile.off_label(), Some("NOT RUNNING"));
        tile.screen.apply(&pitwall_proto::ScreenFrame {
            cols: 2,
            rows: 1,
            cursor: None,
            full: true,
            lines: vec![],
        });
        assert_eq!(tile.off_label(), None, "the last screen stays");
        let running = WallTile::new(
            agent("b", "/work/alpha", "idle", 0, true),
            13.,
            250.,
            WeakEntity::new_invalid(),
        );
        assert_eq!(running.off_label(), None, "waiting for the first frame");
    }

    #[test]
    fn status_words_take_their_colours() {
        let t = Theme::dark();
        assert_eq!(kit::word_color(Status::Blocked, &t), t.amber);
        assert_eq!(kit::word_color(Status::Done, &t), t.finish);
        assert_eq!(kit::word_color(Status::Stopped, &t), t.text_4);
    }
}
