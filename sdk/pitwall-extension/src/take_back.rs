//! Send with take-back: something the user sent waits a few seconds, with
//! a countdown and Undo, before the tile really sends it.

use std::time::Duration;

use crate::element::{Interactive, IntoElement, ParentElement};
use crate::style::Styled;
use crate::events::Click;
use crate::element::v_flex;
use crate::tile::{Cx, Timer};
use crate::widgets::banner;
use crate::Tone;

/// How long [`TakeBack::new`] holds a send.
pub const DEFAULT_SECONDS: u32 = 5;

/// Sends held for a few seconds, with a countdown and Undo, before the
/// tile's own send runs: for an answer, a message or anything else the
/// user may want back. Keep one in the tile's state; [`hold`](Self::hold)
/// what the user sent, draw [`view`](Self::view) where it should show
/// (each held send is a banner, "<label>: sending in 4 s", with Undo).
/// When the time is up, `send` gets it; Undo gives it back to `undo` (put
/// the text back in its field). Its countdown runs on the tile's timers,
/// so it stands still while the tile is hidden and a test lets it pass
/// with the harness's `advance`.
///
/// ```ignore
/// struct Desk { reply: String, sending: TakeBack<String> }
///
/// fn sending(d: &mut Desk) -> &mut TakeBack<String> { &mut d.sending }
///
/// // On Send:
/// let text = std::mem::take(&mut self.reply);
/// self.sending.hold(text, "Your reply", cx, sending, |d, text, cx| d.post(text, cx));
/// // In render:
/// .child(self.sending.view(cx, sending, |d, text, _| d.reply = text))
/// ```
#[derive(Debug)]
pub struct TakeBack<P> {
    held: Vec<Held<P>>,
    next: u64,
    seconds: u32,
}

/// A send waiting in a [`TakeBack`].
#[derive(Debug)]
pub struct Held<P> {
    /// Its id in the [`TakeBack`].
    pub id: u64,
    pub item: P,
    /// What it is, in the banner ("Your reply").
    pub label: String,
    /// Seconds left before it is sent.
    pub left: u32,
    timer: Option<Timer>,
}

impl<P: 'static> Default for TakeBack<P> {
    fn default() -> Self {
        TakeBack::new()
    }
}

impl<P: 'static> TakeBack<P> {
    /// Holding each send [`DEFAULT_SECONDS`].
    pub fn new() -> TakeBack<P> {
        TakeBack::with_seconds(DEFAULT_SECONDS)
    }

    /// Holding each send `seconds` (1 to 30).
    pub fn with_seconds(seconds: u32) -> TakeBack<P> {
        TakeBack { held: Vec::new(), next: 0, seconds: seconds.clamp(1, 30) }
    }

    /// Hold `item`, labelled `label`: `send` gets it when the time is up
    /// (from a timer: call what really sends it there). `pick` finds this
    /// `TakeBack` in the tile's state. Its id, for [`undo`](Self::undo)
    /// and [`send_now`](Self::send_now).
    pub fn hold<T: 'static>(&mut self, item: P, label: impl Into<String>, cx: &mut Cx<T>, pick: fn(&mut T) -> &mut TakeBack<P>, send: fn(&mut T, P, &mut Cx<T>)) -> u64 {
        self.next += 1;
        let id = self.next;
        let timer = cx.every(Duration::from_secs(1), move |t, cx| {
            if let Some(item) = pick(t).tick(id, cx) {
                send(t, item, cx);
            }
            cx.notify();
        });
        self.held.push(Held { id, item, label: label.into(), left: self.seconds, timer });
        cx.notify();
        id
    }

    /// A second passed for send `id`: its item once its time is up.
    fn tick<T: 'static>(&mut self, id: u64, cx: &mut Cx<T>) -> Option<P> {
        let i = self.held.iter().position(|h| h.id == id)?;
        let h = &mut self.held[i];
        h.left = h.left.saturating_sub(1);
        if h.left > 0 {
            return None;
        }
        self.take(id, cx)
    }

    fn take<T: 'static>(&mut self, id: u64, cx: &mut Cx<T>) -> Option<P> {
        let i = self.held.iter().position(|h| h.id == id)?;
        let h = self.held.remove(i);
        if let Some(t) = h.timer {
            cx.cancel(t);
        }
        Some(h.item)
    }

    /// Take send `id` back: it is never sent; its item, to put back.
    pub fn undo<T: 'static>(&mut self, id: u64, cx: &mut Cx<T>) -> Option<P> {
        self.take(id, cx)
    }

    /// Stop waiting for send `id`: its item, for the tile to send now.
    pub fn send_now<T: 'static>(&mut self, id: u64, cx: &mut Cx<T>) -> Option<P> {
        self.take(id, cx)
    }

    /// The sends waiting, oldest first.
    pub fn held(&self) -> &[Held<P>] {
        &self.held
    }

    /// Nothing waits.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// The sends waiting, as banners keyed `take-back-<id>` in a column
    /// keyed `take-back`: "<label>: sending in 4 s" with Undo, which gives
    /// the item to `undo`.
    pub fn view<T: 'static>(&self, cx: &mut Cx<T>, pick: fn(&mut T) -> &mut TakeBack<P>, undo: fn(&mut T, P, &mut Cx<T>)) -> impl IntoElement {
        let mut col = v_flex().id("take-back").gap_1();
        for h in &self.held {
            let id = h.id;
            let text = format!("{}: sending in {} s", h.label, h.left);
            col = col.child(banner(Tone::Info, text).id(format!("take-back-{id}")).action(
                "Undo",
                cx.listener(move |t: &mut T, _: &Click, cx| {
                    if let Some(item) = pick(t).undo(id, cx) {
                        undo(t, item, cx);
                    }
                    cx.notify();
                }),
            ));
        }
        col
    }
}
