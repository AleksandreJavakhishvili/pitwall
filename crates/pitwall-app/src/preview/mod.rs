//! The agent pane's Preview (docs/spec/terminals.md §4): the dev server its
//! terminal printed, in a native web view ([`crate::webview`]) beside the
//! terminal. The pieces here don't need a window and are tested with a fake
//! view; the pane, its toolbar and the wiring live in the main screen
//! (`main_screen/preview.rs`).
//!
//! - [`Follow`]: the page follows the terminal. A new address (the server
//!   came back on another port) is navigated to, the same one printed again
//!   (a restart) reloads, and an agent that stopped shows "Server stopped".
//! - [`Stage`]: where the native view is and whether it shows. It sits
//!   over GPUI's drawing, so after every frame it takes its element's
//!   bounds, hides when the element wasn't drawn, and while anything drawn
//!   after it (a menu, a dialog, the palette, a tooltip) covers it, steps
//!   aside for a snapshot of itself drawn in its place ([`covered`]).
//! - [`surface`]: the element that marks the view's place in the pane.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, Global, Hitbox, HitboxBehavior, HitboxId, Hsla, IntoElement, Pixels, RenderImage,
    Styled, Window,
};

use crate::webview::{Options, Snapshot, WebView};

/// A view hidden this long (its pane not shown, its server stopped) is
/// dropped; its address is kept and it comes back when shown.
pub const DROP_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Phone width: the page laid out as on a phone, centred.
pub const PHONE_WIDTH: f32 = 390.;

/// What the page's server is doing, as far as the preview can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Server {
    Live,
    /// The agent (and its dev server) isn't running.
    Stopped,
    /// The page couldn't load (the server is down or restarting).
    Unreachable(String),
}

/// What [`Follow::update`] asks of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Navigate(String),
    Reload,
    /// Hide the page: the server is gone.
    Stop,
}

/// The page following its terminal.
#[derive(Debug, Clone)]
pub struct Follow {
    /// The address the preview shows (the terminal's latest when opened, or
    /// the one picked).
    pub url: String,
    /// The terminal's latest address when last looked at.
    latest: String,
    /// Its `previewSeen` then.
    seen: u32,
    pub server: Server,
}

impl Follow {
    /// Opened on `url`; the terminal's addresses are `urls` (latest first),
    /// printed `seen` times.
    pub fn new(url: &str, urls: &[String], seen: u32) -> Follow {
        Follow {
            url: url.to_string(),
            latest: urls.first().cloned().unwrap_or_else(|| url.to_string()),
            seen,
            server: Server::Live,
        }
    }

    /// The agent's view changed: `urls` (latest first; none once it stops),
    /// printed `seen` times, the agent `running`.
    pub fn update(&mut self, urls: &[String], seen: u32, running: bool) -> Option<Act> {
        let Some(latest) = urls.first().filter(|_| running) else {
            if self.server == Server::Stopped {
                return None;
            }
            self.server = Server::Stopped;
            return Some(Act::Stop);
        };
        let printed = seen != self.seen;
        self.seen = seen;
        if *latest != self.latest {
            // A new address: the server moved (another port).
            self.latest = latest.clone();
            self.url = latest.clone();
            self.server = Server::Live;
            return Some(Act::Navigate(self.url.clone()));
        }
        if printed || self.server == Server::Stopped {
            // Printed again (a restart), or the agent is back.
            let back = self.server == Server::Stopped;
            self.server = Server::Live;
            return Some(if back { Act::Navigate(self.url.clone()) } else { Act::Reload });
        }
        None
    }

    /// The page couldn't load.
    pub fn failed(&mut self, error: &str) {
        if self.server == Server::Live {
            self.server = Server::Unreachable(error.to_string());
        }
    }

    /// The user reloads or goes somewhere: try again.
    pub fn retry(&mut self) {
        if matches!(self.server, Server::Unreachable(_)) {
            self.server = Server::Live;
        }
    }
}

/// What the native view does after a frame ([`Stage::frame`]).
#[derive(Debug, Clone, PartialEq)]
pub enum StageOp {
    SetBounds(Bounds<Pixels>),
    Show,
    Hide,
    /// Take a snapshot: something is about to cover the view.
    Snapshot,
}

/// The picture drawn while something covers the view.
#[derive(Clone, Default)]
pub enum Cover {
    #[default]
    None,
    /// Asked for; the view still shows meanwhile.
    Pending,
    /// The view is hidden; this is drawn in its place (`None`: the engine
    /// couldn't take one, the pane's background shows).
    Shown(Option<Arc<RenderImage>>),
}

/// The native view's place and visibility.
#[derive(Clone, Default)]
pub struct Stage {
    /// The bounds it was given last.
    pub bounds: Option<Bounds<Pixels>>,
    pub visible: bool,
    pub cover: Cover,
}

impl Stage {
    /// After a frame: the element's bounds (`None`: not drawn), whether
    /// something drawn after it covers it, whether the page is live.
    pub fn frame(&mut self, drawn: Option<Bounds<Pixels>>, covered: bool, live: bool) -> Vec<StageOp> {
        let mut ops = Vec::new();
        let drawn = drawn.filter(|b| b.size.width > gpui::px(0.) && b.size.height > gpui::px(0.));
        let Some(b) = drawn else {
            self.cover = Cover::None;
            if self.visible {
                self.visible = false;
                ops.push(StageOp::Hide);
            }
            return ops;
        };
        if self.bounds != Some(b) {
            self.bounds = Some(b);
            ops.push(StageOp::SetBounds(b));
        }
        if !live {
            self.cover = Cover::None;
            if self.visible {
                self.visible = false;
                ops.push(StageOp::Hide);
            }
        } else if covered {
            match self.cover {
                Cover::None if self.visible => {
                    self.cover = Cover::Pending;
                    ops.push(StageOp::Snapshot);
                }
                // Covered before it ever showed: nothing to picture.
                Cover::None => self.cover = Cover::Shown(None),
                Cover::Pending | Cover::Shown(_) => {}
            }
        } else {
            self.cover = Cover::None;
            if !self.visible {
                self.visible = true;
                ops.push(StageOp::Show);
            }
        }
        ops
    }

    /// The snapshot [`StageOp::Snapshot`] asked for arrived.
    pub fn snapshot(&mut self, image: Option<Arc<RenderImage>>) -> Vec<StageOp> {
        if !matches!(self.cover, Cover::Pending) {
            // Uncovered meanwhile.
            return Vec::new();
        }
        self.cover = Cover::Shown(image);
        if self.visible {
            self.visible = false;
            return vec![StageOp::Hide];
        }
        Vec::new()
    }

    /// The picture to draw in the view's place, if any.
    pub fn picture(&self) -> Option<Option<Arc<RenderImage>>> {
        match &self.cover {
            Cover::Shown(i) => Some(i.clone()),
            _ => None,
        }
    }

    /// Hidden for good (not merely covered): the drop timer runs.
    pub fn idle(&self) -> bool {
        !self.visible && !matches!(self.cover, Cover::Pending | Cover::Shown(_))
    }
}

/// Where the element `id` was drawn in the last frame, and whether anything
/// drawn after it (later in paint order: a menu, dialog, popover, toast) or
/// the tooltip overlaps it. `None`: it wasn't drawn.
pub fn placed(
    hitboxes: &[Hitbox],
    tooltip: Option<Bounds<Pixels>>,
    id: HitboxId,
) -> Option<(Bounds<Pixels>, bool)> {
    placed_by(hitboxes, |h| h.id == id, |h| h.bounds.intersect(&h.content_mask.bounds), tooltip)
}

/// [`placed`] over any list in paint order.
pub fn placed_by<T>(
    items: &[T],
    is: impl Fn(&T) -> bool,
    shown: impl Fn(&T) -> Bounds<Pixels>,
    tooltip: Option<Bounds<Pixels>>,
) -> Option<(Bounds<Pixels>, bool)> {
    let at = items.iter().position(is)?;
    let b = shown(&items[at]);
    let covered = items[at + 1..].iter().any(|h| overlaps(&shown(h), &b))
        || tooltip.is_some_and(|t| overlaps(&t, &b));
    Some((b, covered))
}

fn overlaps(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> bool {
    let i = a.intersect(b);
    i.size.width > gpui::px(0.) && i.size.height > gpui::px(0.)
}

/// The element the native view sits on: it fills its box, puts down a
/// hitbox (how the frame listener finds where it was drawn and what is
/// over it: [`placed`]), and draws `picture` (the snapshot while covered)
/// or `bg`.
pub fn surface(
    slot: Rc<Cell<Option<HitboxId>>>,
    picture: Option<Option<Arc<RenderImage>>>,
    bg: Hsla,
) -> impl IntoElement {
    gpui::canvas(
        move |bounds, window, _| {
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            slot.set(Some(hitbox.id));
            hitbox
        },
        move |bounds, _, window, _| {
            window.paint_quad(gpui::fill(bounds, bg));
            if let Some(Some(image)) = &picture {
                let _ = window.paint_image(bounds, Default::default(), image.clone(), 0, false);
            }
        },
    )
    .size_full()
}

/// A snapshot as GPUI draws it (BGRA).
pub fn to_image(s: Snapshot) -> Option<Arc<RenderImage>> {
    let mut data = s.rgba;
    if s.width == 0 || s.height == 0 || data.len() != (s.width * s.height * 4) as usize {
        return None;
    }
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(s.width, s.height, data)?;
    Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

/// The project's own cookies and storage: `<data>/webview/<name>-<hash>`.
pub fn data_dir(root: &Path, project: &str) -> PathBuf {
    let name: String = Path::new(project)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    // FNV-1a: stable across runs and Rust versions.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in project.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    root.join("webview").join(format!("{name}-{h:016x}"))
}

type CreateFn = Rc<dyn Fn(&mut Window, Options) -> Result<Box<dyn WebView>, String>>;

/// Makes the web views (tests put a fake here).
#[derive(Clone)]
pub struct Factory(pub CreateFn);

impl Global for Factory {}

/// A web view from the [`Factory`], else the platform's.
pub fn create(window: &mut Window, opts: Options, cx: &App) -> Result<Box<dyn WebView>, String> {
    match cx.try_global::<Factory>() {
        Some(f) => (f.0.clone())(window, opts),
        None => crate::webview::create(window, opts),
    }
}

#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod tests;
