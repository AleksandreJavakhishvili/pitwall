//! The pane of an agent on another machine (docs/spec/tailnet-peers.md,
//! "Live terminals"): its terminal, live and view-only, as it is on that
//! machine, under a thin banner ("Live from studio · view only"), with
//! the details card a click away.
//!
//! The terminal comes over `term.watch` ([`pitwall_peer::Peers::watch`]),
//! whatever the agent is (any agent, a plain shell, an agw session the
//! peer relays): Pitwall streams the terminal it hosts, never anything of
//! the agent's. It is drawn by the same terminal view as a local agent's,
//! in [`ViewMode::Mirror`]: at the source's size (scaled down to fit the
//! width, bottom rows first), with selection, copy, find and scrollback,
//! and nothing typed, pasted or reported goes anywhere; its stream is a
//! [`NullStream`], so there is no path to the other machine at all.
//!
//! The stream is open only while the pane is drawn: a pane not drawn for
//! [`CLOSE_AFTER`] (another space, folded, closed) closes it, and drawing
//! it again opens it with a fresh snapshot. A dropped link shows
//! "reconnecting…" over the last screen until the peer sends a new one.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Context, Entity, FontWeight, Global, HitboxBehavior, HitboxId,
    SharedString, Window, WindowId,
};
use pitwall_peer::{TermWatch, WatchEvent, WatchSink};
use pitwall_proto::AgentView;
use pitwall_term_view::{
    Feed, NullStream, TermFont, TermSize, Terminal, TerminalConfig, TerminalView, ViewMode, ViewSettings,
};

use crate::kit::{tooltip, BtnKind, MONO_FONT};
use crate::theme::Theme;

/// A pane not drawn this long closes its stream.
pub const CLOSE_AFTER: Duration = Duration::from_secs(2);
/// The smallest a mirrored terminal is scaled to fit its pane's width.
const MIN_SCALE: f32 = 0.5;

/// Where the stream is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    /// Asking the other machine for it.
    Connecting,
    /// Showing what the other machine shows.
    Live,
    /// The link dropped; the last screen stays until a fresh snapshot.
    Reconnecting(String),
    /// Nothing more comes (it exited, stopped, or was refused), in words.
    Ended(String),
    /// Closed while the pane isn't drawn.
    Closed,
    /// This Pitwall can't watch it (not showing other machines…): the
    /// details card instead.
    Unavailable(String),
}

/// Feeds a watch's events into the terminal (on the watch's thread) and
/// tells the pane how the stream is.
struct Sink {
    feed: Feed,
    tx: mpsc::UnboundedSender<Live>,
}

impl WatchSink for Sink {
    fn event(&self, ev: WatchEvent) {
        let resize = |cols: u16, rows: u16| {
            if let Some(t) = self.feed.terminal() {
                t.resize(TermSize::new(cols, rows));
            }
        };
        match ev {
            WatchEvent::Reset { cols, rows } => {
                resize(cols, rows);
                self.feed.reset();
                let _ = self.tx.unbounded_send(Live::Live);
            }
            WatchEvent::Output(bytes) => self.feed.push(&bytes),
            WatchEvent::Resized { cols, rows } => resize(cols, rows),
            WatchEvent::Reconnecting(why) => {
                let _ = self.tx.unbounded_send(Live::Reconnecting(why));
            }
            WatchEvent::Ended(why) => {
                let _ = self.tx.unbounded_send(Live::Ended(why));
            }
        }
    }
}

/// One mirrored agent's pane body in one window.
pub struct PeerPane {
    agent: AgentView,
    terminal: Terminal,
    view: Entity<TerminalView>,
    watch: Option<TermWatch>,
    pub live: Live,
    /// The details card instead of the terminal.
    pub details: bool,
    /// Running when last heard of: a stream that ended opens again when it
    /// runs again.
    was_running: bool,
    /// Where the pane was drawn (the frame listener looks for it).
    slot: Rc<Cell<Option<HitboxId>>>,
    /// Not drawn since the frame numbered so (its close timer checks).
    hidden: Option<u64>,
    hides: u64,
    tx: mpsc::UnboundedSender<Live>,
    _tasks: Vec<gpui::Task<()>>,
}

impl PeerPane {
    fn new(a: &AgentView, font: f32, window: &mut Window, cx: &mut Context<Self>) -> PeerPane {
        let config = TerminalConfig { theme: crate::terminal::term_theme(crate::theme::theme(cx).mode), ..TerminalConfig::default() };
        // The source's size arrives with the snapshot; until then, the last known.
        let terminal = Terminal::new(NullStream, TermSize::new(a.cols.max(20), a.rows.max(5)), config);
        let settings = ViewSettings { font: TermFont { size: font, ..TermFont::default() }, ..ViewSettings::default() };
        let t = terminal.clone();
        let view = cx.new(|cx| TerminalView::new(t, ViewMode::Mirror { min_scale: MIN_SCALE }, settings, window, cx));
        let (tx, mut rx) = mpsc::unbounded::<Live>();
        let follow = cx.spawn(async move |this, cx| {
            while let Some(live) = rx.next().await {
                if this.update(cx, |p, cx| p.set_live(live, cx)).is_err() {
                    break;
                }
            }
        });
        // After every frame: was the pane drawn?
        let (weak, slot) = (cx.weak_entity(), Rc::new(Cell::new(None)));
        let seen = slot.clone();
        window.on_frame_drawn(move |window, cx| {
            let drawn = seen.get().is_some_and(|id: HitboxId| window.rendered_hitboxes().iter().any(|h| h.id == id));
            weak.update(cx, |p, cx| p.frame(drawn, cx)).is_ok()
        });
        PeerPane {
            agent: a.clone(),
            terminal,
            view,
            watch: None,
            live: Live::Closed,
            details: false,
            was_running: a.running,
            slot,
            hidden: None,
            hides: 0,
            tx,
            _tasks: vec![follow],
        }
    }

    fn set_live(&mut self, live: Live, cx: &mut Context<Self>) {
        // A late word from a stream already closed changes nothing.
        if self.watch.is_none() && !matches!(live, Live::Ended(_)) {
            return;
        }
        if matches!(live, Live::Ended(_)) {
            self.watch = None;
        }
        self.live = live;
        cx.notify();
    }

    /// Open the stream (if this Pitwall can).
    fn open(&mut self, cx: &mut Context<Self>) {
        let Some(model) = crate::peers::model(cx) else {
            self.live = Live::Unavailable("this Pitwall doesn't show other machines".into());
            return;
        };
        let peers = model.read(cx).peers().clone();
        let sink = Arc::new(Sink { feed: self.terminal.feed(), tx: self.tx.clone() });
        match peers.watch(&self.agent.id, sink) {
            Ok(w) => {
                self.watch = Some(w);
                self.live = Live::Connecting;
            }
            Err(e) => self.live = Live::Unavailable(e.message),
        }
    }

    fn close(&mut self) {
        self.watch = None;
        if matches!(self.live, Live::Connecting | Live::Live | Live::Reconnecting(_)) {
            self.live = Live::Closed;
        }
    }

    /// Drawn now: the stream should be open (unless it ended for good).
    fn shown(&mut self, a: &AgentView, cx: &mut Context<Self>) {
        let again = !self.was_running && a.running;
        self.was_running = a.running;
        self.agent = a.clone();
        self.hidden = None;
        let open = match &self.live {
            Live::Closed => true,
            Live::Ended(_) => again,
            _ => false,
        };
        if open && self.watch.is_none() {
            self.open(cx);
        }
    }

    /// A frame was drawn (with or without this pane).
    fn frame(&mut self, drawn: bool, cx: &mut Context<Self>) {
        if drawn || self.watch.is_none() {
            self.hidden = None;
            return;
        }
        if self.hidden.is_none() {
            self.hides += 1;
            let n = self.hides;
            self.hidden = Some(n);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLOSE_AFTER).await;
                let _ = this.update(cx, |p, cx| {
                    // Still not drawn since then.
                    if p.hidden == Some(n) {
                        p.close();
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    /// Whether a stream is open (tests).
    pub fn streaming(&self) -> bool {
        self.watch.is_some()
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    fn banner(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let from = self.agent.machine.label.clone();
        let (dot, words) = match &self.live {
            Live::Live => (t.green, format!("Live from {from} · view only")),
            Live::Connecting | Live::Closed => (t.text_3, format!("Connecting to {from}…")),
            Live::Reconnecting(_) => (t.amber, format!("{from}: reconnecting…")),
            Live::Ended(why) => (t.text_4, format!("{from}: the terminal ended ({why}) · view only")),
            Live::Unavailable(why) => (t.text_4, format!("No live terminal: {why}")),
        };
        let why = match &self.live {
            Live::Reconnecting(w) => Some(w.clone()),
            _ => None,
        };
        let id = self.agent.id.clone();
        let label = if self.details { "Terminal" } else { "Details" };
        div()
            .id(SharedString::from(format!("peer-banner-{id}")))
            .debug_selector({
                let id = id.clone();
                move || format!("peer-banner-{id}")
            })
            .flex_none()
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .bg(t.surface_2)
            .border_b_1()
            .border_color(t.line)
            .text_size(px(11.5))
            .text_color(t.text_3)
            .child(div().flex_none().size(px(6.)).rounded(px(3.)).bg(dot))
            .child(
                div()
                    .id(SharedString::from(format!("peer-live-{id}")))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .when_some(why, |d, w| d.tooltip(tooltip(w)))
                    .child(crate::kit::one_line(words)),
            )
            .when(self.can_retry(), |d| {
                d.child(
                    crate::kit::text_button(SharedString::from(format!("peer-retry-{id}")), "Watch again", BtnKind::GhostSm, false, t)
                        .on_click(cx.listener(|p, _, _, cx| {
                            p.live = Live::Closed;
                            p.open(cx);
                            cx.notify();
                        })),
                )
            })
            .child(
                crate::kit::text_button(SharedString::from(format!("peer-toggle-{id}")), label, BtnKind::GhostSm, false, t)
                    .debug_selector({
                        let id = id.clone();
                        move || format!("peer-toggle-{id}")
                    })
                    .on_click(cx.listener(|p, _, _, cx| {
                        p.details = !p.details;
                        cx.notify();
                    })),
            )
    }

    fn can_retry(&self) -> bool {
        matches!(self.live, Live::Ended(_) | Live::Unavailable(_)) && self.watch.is_none()
    }
}

impl Render for PeerPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        let a = self.agent.clone();
        // Nothing to watch here: what is known of it.
        let card = self.details || matches!(self.live, Live::Unavailable(_));
        let slot = self.slot.clone();
        let body: AnyElement = if card {
            super::peers::remote_details(&a, &t)
        } else {
            div()
                .id(SharedString::from(format!("peer-term-{}", a.id)))
                .debug_selector({
                    let id = a.id.clone();
                    move || format!("peer-term-{id}")
                })
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .child(self.view.clone())
                .when(matches!(self.live, Live::Reconnecting(_) | Live::Connecting), |d| {
                    d.child(
                        div().absolute().top(px(8.)).right(px(16.)).px(px(8.)).py(px(3.)).rounded(px(4.)).bg(t.surface_2).text_size(px(11.5)).text_color(t.text_3).font_family(MONO_FONT).child(
                            if matches!(self.live, Live::Connecting) { "connecting…" } else { "reconnecting…" },
                        ),
                    )
                })
                .into_any_element()
        };
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            // Under everything: where the pane was drawn (CLOSE_AFTER).
            .child(
                canvas(
                    move |bounds, window, _| {
                        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                        slot.set(Some(hitbox.id));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(self.banner(&t, cx))
            .child(body)
            .font_weight(FontWeight::NORMAL)
    }
}

/// Every mirrored agent's pane, per window.
#[derive(Default)]
struct PeerPanes(HashMap<(WindowId, String), Entity<PeerPane>>);

impl Global for PeerPanes {}

/// The pane body for `a`, another machine's agent, in this window.
pub(super) fn pane(a: &AgentView, font: f64, live_ids: &[String], window: &mut Window, cx: &mut App) -> AnyElement {
    let here = window.window_handle().window_id();
    let key = (here, a.id.clone());
    let known = {
        let panes = &mut cx.default_global::<PeerPanes>().0;
        // Agents no longer listed let their panes (and streams) go.
        panes.retain(|(_, id), _| live_ids.contains(id));
        panes.get(&key).cloned()
    };
    let pane = match known {
        Some(p) => p,
        None => {
            let p = cx.new(|cx| PeerPane::new(a, font as f32, window, cx));
            cx.global_mut::<PeerPanes>().0.insert(key, p.clone());
            p
        }
    };
    pane.update(cx, |p, cx| p.shown(a, cx));
    pane.into_any_element()
}

/// `a`'s pane in this window, if it was drawn here (tests).
pub fn pane_of(agent: &str, window: &Window, cx: &App) -> Option<Entity<PeerPane>> {
    cx.try_global::<PeerPanes>()?.0.get(&(window.window_handle().window_id(), agent.to_string())).cloned()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
    use pitwall_core::provider::TermSize as PtySize;
    use pitwall_core::term::{ScreenSource, TermHost};
    use pitwall_core::testing::{FakeTerm, FakeTermCtl, TempDir};
    use pitwall_daemon::events::{self, Hub, Queue};
    use pitwall_peer::fake::FakeWorld;
    use pitwall_peer::{Feed, PeerSettings, Peers, PeersConfig, Timing};
    use pitwall_proto::AgentView;

    use super::super::{init, MainScreen};
    use super::{pane_of, Live, CLOSE_AFTER};
    use crate::agents::tests::agent;
    use crate::agents::AgentStore;
    use crate::peers::{ModelHandle, PeersModel};

    /// `studio`'s agents and their terminals, without an engine.
    struct TestFeed {
        views: Mutex<Vec<AgentView>>,
        hub: Arc<Hub>,
        host: Mutex<Option<Arc<TermHost>>>,
    }

    impl Feed for TestFeed {
        fn views(&self) -> Vec<AgentView> {
            self.views.lock().unwrap().clone()
        }

        fn subscribe(&self, topics: Vec<&'static str>) -> Arc<Queue> {
            let q = self.hub.subscribe(topics);
            events::snapshot_of(&self.views.lock().unwrap(), &q);
            q
        }

        fn terminal(&self, agent_id: &str) -> Option<Arc<TermHost>> {
            self.host.lock().unwrap().clone().filter(|_| agent_id == "a1")
        }
    }

    fn peers(world: &Arc<FakeWorld>, node: &str, dir: &TempDir, feed: Option<Arc<dyn Feed>>) -> Arc<Peers> {
        let ms = Duration::from_millis;
        Peers::new(PeersConfig {
            root: dir.path().to_path_buf(),
            tailnet: world.tailnet(node),
            version: "0.2.1".into(),
            sink: Arc::new(pitwall_peer::NullPeerSink),
            feed,
            timing: Timing { backoff: vec![ms(20), ms(50)], ping: ms(200), dead_after: Duration::from_secs(5), status_poll: ms(100), ..Timing::default() },
            limits: Default::default(),
        })
        .unwrap()
    }

    fn frame(cx: &mut VisualTestContext) {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    /// Draw frames while the peers' threads work, until `f` holds.
    fn until(cx: &mut VisualTestContext, what: &str, mut f: impl FnMut(&mut VisualTestContext) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            frame(cx);
            if f(cx) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting until {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    struct Setup {
        _dirs: (TempDir, TempDir),
        studio: Arc<Peers>,
        laptop: Arc<Peers>,
        ctl: FakeTermCtl,
    }

    const ID: &str = "peer:studio:a1";
    const BANNER: &str = "peer-banner-peer:studio:a1";
    const TERM: &str = "peer-term-peer:studio:a1";
    const TOGGLE: &str = "peer-toggle-peer:studio:a1";

    fn screen(cx: &mut TestAppContext) -> (Setup, Entity<MainScreen>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::Theme::dark());
            init(cx);
        });
        let world = FakeWorld::new();
        world.add("nLAPTOP", "laptop", "alice@example.com").add("nSTUDIO", "studio", "alice@example.com");
        let (sdir, ldir) = (TempDir::new("app-peer-term-studio"), TempDir::new("app-peer-term-laptop"));
        // Studio runs `a1`, whose program printed a prompt.
        let (term, ctl) = FakeTerm::new(PtySize::new(80, 24), None);
        ctl.set_history(b"hello from studio\r\n> ");
        let host = TermHost::new(Box::new(term), PtySize::DEFAULT, Arc::new(pitwall_core::clock::SystemClock::new()));
        let feed = Arc::new(TestFeed {
            views: Mutex::new(vec![agent("a1", "/work/api", "working", 1, true)]),
            hub: Hub::new(Arc::new(pitwall_core::events::NullSink)),
            host: Mutex::new(Some(host)),
        });
        let studio = peers(&world, "nSTUDIO", &sdir, Some(feed));
        studio.set_settings(PeerSettings { share: true, show: false, port: 0 }).unwrap();
        studio.start();
        let laptop = peers(&world, "nLAPTOP", &ldir, None);
        laptop.set_settings(PeerSettings { share: false, show: true, port: 0 }).unwrap();
        laptop.start();
        laptop.pair("studio").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while laptop.views().is_empty() {
            assert!(Instant::now() < deadline, "studio's agent is mirrored");
            std::thread::sleep(Duration::from_millis(10));
        }
        let (p, root) = (laptop.clone(), ldir.path().to_path_buf());
        let tailnet = world.tailnet("nLAPTOP");
        cx.update(|cx| {
            let m = cx.new(|_| PeersModel::new(p, tailnet, root));
            cx.set_global(ModelHandle(m));
        });
        let mut agents = vec![agent("api", "/work/alpha", "idle", 1, true)];
        agents.extend(laptop.views());
        let store = cx.new(|_| AgentStore::new(agents));
        let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(store.clone(), window, cx));
        cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
        frame(cx);
        (Setup { _dirs: (sdir, ldir), studio, laptop, ctl }, view, cx)
    }

    fn pane(cx: &mut VisualTestContext) -> Option<Entity<super::PeerPane>> {
        cx.update(|window, cx| pane_of(ID, window, cx))
    }

    fn live(cx: &mut VisualTestContext) -> Option<Live> {
        let p = pane(cx)?;
        Some(p.read_with(cx, |p, _| p.live.clone()))
    }

    fn text(cx: &mut VisualTestContext) -> String {
        let Some(p) = pane(cx) else { return String::new() };
        p.read_with(cx, |p, _| p.terminal().feed().screen_source().text())
    }

    #[gpui::test]
    fn a_peer_agents_pane_shows_its_terminal_live_and_view_only(cx: &mut TestAppContext) {
        let (s, view, cx) = screen(cx);
        view.update_in(cx, |m, window, cx| m.show_agent(ID, window, cx));
        until(cx, "the snapshot is on screen", |cx| text(cx).contains("hello from studio"));
        assert_eq!(live(cx), Some(Live::Live));
        assert!(cx.debug_bounds(BANNER).is_some(), "the banner");
        assert!(cx.debug_bounds(TERM).is_some(), "the terminal");
        let size = pane(cx).unwrap().read_with(cx, |p, _| p.terminal().size());
        assert_eq!((size.cols, size.rows), (80, 24), "the source's size, not the pane's");

        // Live output.
        s.ctl.output(b"thinking...");
        until(cx, "live output", |cx| text(cx).contains("thinking..."));

        // Typing, pasting and clicking in it go nowhere.
        let b = cx.debug_bounds(TERM).unwrap();
        cx.simulate_click(b.center(), Modifiers::none());
        cx.simulate_input("rm -rf ~");
        cx.simulate_keystrokes("enter ctrl-c");
        cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("echo hi".into())));
        cx.simulate_keystrokes("cmd-v");
        std::thread::sleep(Duration::from_millis(100));
        frame(cx);
        assert!(s.ctl.input().is_empty(), "nothing reached studio: {:?}", s.ctl.input());
        assert!(s.ctl.resizes().is_empty(), "studio's terminal was never resized");

        // The details card is a click away.
        let t = cx.debug_bounds(TOGGLE).unwrap();
        cx.simulate_click(t.center(), Modifiers::none());
        frame(cx);
        assert!(pane(cx).unwrap().read_with(cx, |p, _| p.details));
        assert!(pane(cx).unwrap().read_with(cx, |p, _| p.streaming()), "still streaming while shown");
        pane(cx).unwrap().update(cx, |p, cx| {
            p.details = false;
            cx.notify();
        });

        // Not shown: the stream closes (only the link stays connected)…
        view.update_in(cx, |m, window, cx| m.show_agent("api", window, cx));
        frame(cx);
        cx.executor().advance_clock(CLOSE_AFTER + Duration::from_millis(100));
        until(cx, "the stream closed", |cx| !pane(cx).unwrap().read_with(cx, |p, _| p.streaming()));
        until(cx, "studio dropped it", |_| s.studio.connections() == 1);
        // …and shown again, it opens with a fresh snapshot.
        view.update_in(cx, |m, window, cx| m.show_agent(ID, window, cx));
        until(cx, "live again", |cx| live(cx) == Some(Live::Live) && text(cx).contains("thinking..."));
        assert_eq!(s.studio.connections(), 2);

        // The link drops: "reconnecting…" over the last screen, then live.
        s.studio.set_settings(PeerSettings { share: false, show: false, port: 0 }).unwrap();
        until(cx, "reconnecting", |cx| matches!(live(cx), Some(Live::Reconnecting(_))));
        assert!(text(cx).contains("thinking..."), "the last screen stays");
        s.studio.set_settings(PeerSettings { share: true, show: false, port: 0 }).unwrap();
        until(cx, "resumed", |cx| live(cx) == Some(Live::Live));

        // The program exits: the pane says so.
        s.ctl.exit(0);
        until(cx, "ended", |cx| matches!(live(cx), Some(Live::Ended(_))));
        s.laptop.stop();
        s.studio.stop();
    }
}
