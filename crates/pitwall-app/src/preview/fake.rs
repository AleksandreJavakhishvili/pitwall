//! A web view that records what it is asked (tests): no engine, no window.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Bounds, Pixels};

use crate::webview::{Options, Snapshot, WebEvent, WebView};

/// What the fakes were asked, and a way to answer as the engine would.
#[derive(Default)]
pub struct Log {
    /// One line per call: "create http://…", "navigate …", "bounds x y w h",
    /// "show", "hide", "drop", …
    pub calls: Vec<String>,
    /// The live views' event callbacks (latest last).
    events: Vec<Rc<dyn Fn(WebEvent)>>,
    /// Snapshots asked for and not answered yet.
    snapshots: Vec<Box<dyn FnOnce(Option<Snapshot>)>>,
    /// `create` fails with this.
    pub fail: Option<String>,
}

pub type Shared = Rc<RefCell<Log>>;

impl Log {
    pub fn take(log: &Shared) -> Vec<String> {
        std::mem::take(&mut log.borrow_mut().calls)
    }

    /// The latest view reports `e`.
    pub fn emit(log: &Shared, e: WebEvent) {
        let f = log.borrow().events.last().cloned();
        if let Some(f) = f {
            f(e);
        }
    }

    /// Answer the snapshots asked for (a 2×1 picture).
    pub fn answer_snapshots(log: &Shared) -> usize {
        let pending = std::mem::take(&mut log.borrow_mut().snapshots);
        let n = pending.len();
        for done in pending {
            done(Some(Snapshot { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] }));
        }
        n
    }
}

struct Fake {
    log: Shared,
}

impl Fake {
    fn note(&self, s: String) {
        self.log.borrow_mut().calls.push(s);
    }
}

impl WebView for Fake {
    fn navigate(&mut self, url: &str) {
        self.note(format!("navigate {url}"));
    }
    fn reload(&mut self) {
        self.note("reload".into());
    }
    fn back(&mut self) {
        self.note("back".into());
    }
    fn forward(&mut self) {
        self.note("forward".into());
    }
    fn set_bounds(&mut self, b: Bounds<Pixels>) {
        self.note(format!(
            "bounds {} {} {} {}",
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height)
        ));
    }
    fn set_visible(&mut self, visible: bool) {
        self.note(if visible { "show" } else { "hide" }.into());
    }
    fn focus(&mut self) {
        self.note("focus".into());
    }
    fn blur(&mut self) {
        self.note("blur".into());
    }
    fn open_inspector(&mut self) {
        self.note("inspector".into());
    }
    fn snapshot(&mut self, done: Box<dyn FnOnce(Option<Snapshot>) + 'static>) {
        self.note("snapshot".into());
        self.log.borrow_mut().snapshots.push(done);
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.note("drop".into());
    }
}

/// Make every web view in `cx` a fake writing to the returned log.
pub fn install(cx: &mut App) -> Shared {
    let log: Shared = Default::default();
    let l = log.clone();
    cx.set_global(super::Factory(Rc::new(move |_, opts: Options| {
        if let Some(e) = l.borrow().fail.clone() {
            return Err(e);
        }
        l.borrow_mut().calls.push(format!("create {}", opts.url));
        let events: Rc<dyn Fn(WebEvent)> = Rc::from(opts.events);
        l.borrow_mut().events.push(events);
        Ok(Box::new(Fake { log: l.clone() }))
    })));
    log
}
