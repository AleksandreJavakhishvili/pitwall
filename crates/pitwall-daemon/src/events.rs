//! Events over the socket (`events.subscribe`, architecture.md §4). The host
//! gives the engine a [`Hub`] as its event sink, wrapping its own (the app's
//! bridge): the engine's events still reach the host, and each subscribed
//! connection gets them in a bounded queue of its own that a thread writes
//! out. The engine never waits on a client: a full queue drops the oldest
//! `attention`, and `agents.changed`/`blocked.changed` keep only the latest
//! (each is the whole state).

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use serde::Serialize;

use pitwall_core::events::{Event, EventSink};
use pitwall_core::model::Status;
use pitwall_core::Shared;
use pitwall_proto::{code, event, topic, AgentView, AttentionView, BlockedCount, ErrorBody, EventsSubscribe};

use crate::identity::Caller;

/// Events a slow client may have waiting (on top of one agent list and one
/// blocked count); more drop the oldest `attention`.
pub const QUEUE_CAP: usize = 64;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The engine's event sink when the server reports events: hands every event
/// to `inner` and to each subscribed connection.
pub struct Hub {
    inner: Arc<dyn EventSink>,
    subs: Mutex<Vec<Arc<Queue>>>,
}

impl Hub {
    pub fn new(inner: Arc<dyn EventSink>) -> Arc<Hub> {
        Arc::new(Hub { inner, subs: Mutex::new(Vec::new()) })
    }

    /// Connections subscribed now.
    pub fn subscribers(&self) -> usize {
        let mut subs = lock(&self.subs);
        subs.retain(|q| !q.closed());
        subs.len()
    }

    /// A queue that gets `topics` from now on (until [`Queue::close`]).
    pub fn subscribe(&self, topics: Vec<&'static str>) -> Arc<Queue> {
        let q = Arc::new(Queue::new(topics));
        lock(&self.subs).push(q.clone());
        q
    }
}

impl EventSink for Hub {
    fn emit(&self, ev: Event) {
        {
            let mut subs = lock(&self.subs);
            subs.retain(|q| !q.closed());
            if !subs.is_empty() {
                if let Some((topic, wire)) = to_wire(&ev) {
                    for q in subs.iter() {
                        q.push(topic, &wire);
                    }
                }
            }
        }
        self.inner.emit(ev);
    }
}

fn wire(name: &str, data: impl Serialize) -> pitwall_proto::Event {
    pitwall_proto::Event { event: name.into(), data: serde_json::to_value(data).unwrap_or_default() }
}

fn agents_changed(views: &[AgentView]) -> pitwall_proto::Event {
    wire(event::AGENTS_CHANGED, views)
}

fn blocked_changed(count: usize) -> pitwall_proto::Event {
    wire(event::BLOCKED_CHANGED, BlockedCount { count: count as u32 })
}

/// An engine event as clients get it, and its topic (`None`: not sent).
fn to_wire(ev: &Event) -> Option<(&'static str, pitwall_proto::Event)> {
    Some(match ev {
        Event::AgentsChanged(views) => (topic::AGENTS, agents_changed(views)),
        Event::Attention(a) => (
            topic::ATTENTION,
            wire(
                event::ATTENTION,
                AttentionView { agent_id: a.agent_id.clone(), name: a.name.clone(), reason: a.reason.into(), detail: a.detail.clone() },
            ),
        ),
        Event::BlockedCount(n) => (topic::BLOCKED, blocked_changed(*n)),
        Event::ScanProgress(_) | Event::ProjectsChanged(_) => return None,
    })
}

/// The topics a caller gets for `req`. Agents, attention and the blocked
/// count are what `agent.list` already shows, so anyone may follow them
/// (an agent too: the Race Engineer watches the others). `approvals` is
/// Pitwall's own window's only (as `approval.list`); it isn't sent over the
/// socket yet, so it is left out. Unknown topics are left out too (a newer
/// client, an older server).
pub fn topics(caller: &Caller, req: &EventsSubscribe) -> Result<Vec<&'static str>, ErrorBody> {
    let asked: Vec<&str> = if req.topics.is_empty() { topic::DEFAULT.to_vec() } else { req.topics.iter().map(String::as_str).collect() };
    let mut out = Vec::new();
    for t in asked {
        let known = match t {
            topic::AGENTS => topic::AGENTS,
            topic::ATTENTION => topic::ATTENTION,
            topic::BLOCKED => topic::BLOCKED,
            topic::APPROVALS if !caller.ui => {
                return Err(ErrorBody::new(code::DENIED, "only Pitwall's own window gets approvals"));
            }
            _ => continue,
        };
        if !out.contains(&known) {
            out.push(known);
        }
    }
    Ok(out)
}

/// What a new subscriber gets first: the agent list and blocked count now,
/// so it never has to call `agent.list` and race the events.
pub fn snapshot(engine: &Shared, q: &Queue) {
    snapshot_of(&engine.views(), q);
}

/// [`snapshot`] of a given list (the peer listener's tests).
pub fn snapshot_of(views: &[AgentView], q: &Queue) {
    let blocked = views.iter().filter(|a| a.status == Status::Blocked).count();
    q.push_first(topic::AGENTS, agents_changed(views));
    q.push_first(topic::BLOCKED, blocked_changed(blocked));
}

/// One connection's waiting events.
pub struct Queue {
    state: Mutex<State>,
    ready: Condvar,
}

struct State {
    topics: Vec<&'static str>,
    items: VecDeque<pitwall_proto::Event>,
    closed: bool,
    /// `attention` events dropped because the client was slow.
    dropped: u64,
}

impl Queue {
    fn new(topics: Vec<&'static str>) -> Queue {
        Queue { state: Mutex::new(State { topics, items: VecDeque::new(), closed: false, dropped: 0 }), ready: Condvar::new() }
    }

    /// Subscribing again: these topics from now on.
    pub fn set_topics(&self, topics: Vec<&'static str>) {
        let mut s = lock(&self.state);
        s.items.retain(|e| topics.contains(&topic_of(&e.event)));
        s.topics = topics;
    }

    fn push(&self, topic: &'static str, ev: &pitwall_proto::Event) {
        let mut s = lock(&self.state);
        if s.closed || !s.topics.contains(&topic) {
            return;
        }
        if topic == topic::ATTENTION {
            if s.items.iter().filter(|e| e.event == event::ATTENTION).count() >= QUEUE_CAP {
                if let Some(i) = s.items.iter().position(|e| e.event == event::ATTENTION) {
                    s.items.remove(i);
                    s.dropped += 1;
                }
            }
        } else {
            // The whole state: a newer one replaces one still waiting.
            s.items.retain(|e| e.event != ev.event);
        }
        s.items.push_back(ev.clone());
        self.ready.notify_one();
    }

    /// [`push`](Self::push) unless an event of the kind already waits (it
    /// came from the engine after subscribing, so it isn't older).
    fn push_first(&self, topic: &'static str, ev: pitwall_proto::Event) {
        if lock(&self.state).items.iter().any(|e| e.event == ev.event) {
            return;
        }
        self.push(topic, &ev);
    }

    /// The next event, waiting for one; `None` once closed.
    pub fn pop(&self) -> Option<pitwall_proto::Event> {
        let mut s = lock(&self.state);
        loop {
            if s.closed {
                return None;
            }
            if let Some(e) = s.items.pop_front() {
                return Some(e);
            }
            s = self.ready.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Stop: the hub forgets it, [`pop`](Self::pop) returns `None`.
    pub fn close(&self) {
        let mut s = lock(&self.state);
        s.closed = true;
        s.items.clear();
        self.ready.notify_all();
    }

    fn closed(&self) -> bool {
        lock(&self.state).closed
    }

    /// Events waiting now.
    pub fn len(&self) -> usize {
        lock(&self.state).items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `attention` events dropped so far (the client read too slowly).
    pub fn dropped(&self) -> u64 {
        lock(&self.state).dropped
    }
}

fn topic_of(name: &str) -> &'static str {
    match name {
        event::AGENTS_CHANGED => topic::AGENTS,
        event::ATTENTION => topic::ATTENTION,
        event::BLOCKED_CHANGED => topic::BLOCKED,
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::events::NullSink;
    use pitwall_core::model::Attention;

    fn attention(i: usize) -> Event {
        Event::Attention(Attention { agent_id: format!("a{i}"), name: format!("n{i}"), reason: "blocked", detail: None })
    }

    fn names(q: &Queue) -> Vec<String> {
        lock(&q.state).items.iter().map(|e| e.event.clone()).collect()
    }

    #[test]
    fn a_slow_client_keeps_the_latest_state_and_the_newest_alerts() {
        let hub = Hub::new(Arc::new(NullSink));
        let q = hub.subscribe(topic::DEFAULT.to_vec());
        hub.emit(Event::AgentsChanged(vec![]));
        hub.emit(Event::BlockedCount(1));
        for i in 0..QUEUE_CAP + 5 {
            hub.emit(attention(i));
            hub.emit(Event::AgentsChanged(vec![]));
            hub.emit(Event::BlockedCount(i));
        }
        // One agent list, one count (the latest), and the newest alerts.
        assert_eq!(q.len(), QUEUE_CAP + 2);
        assert_eq!(q.dropped(), 5);
        let first = q.pop().unwrap();
        assert_eq!(first.data["agentId"], "a5", "the 5 oldest alerts were dropped");
        let items = lock(&q.state).items.clone();
        let count = items.iter().find(|e| e.event == event::BLOCKED_CHANGED).unwrap();
        assert_eq!(count.data["count"], QUEUE_CAP + 4);
        assert_eq!(items.iter().filter(|e| e.event == event::AGENTS_CHANGED).count(), 1);
        assert_eq!(names(&q).last().unwrap(), event::BLOCKED_CHANGED, "order kept: the latest state comes last");
    }

    #[test]
    fn only_the_subscribed_topics_and_closed_queues_are_forgotten() {
        let hub = Hub::new(Arc::new(NullSink));
        let q = hub.subscribe(vec![topic::ATTENTION]);
        hub.emit(Event::AgentsChanged(vec![]));
        hub.emit(Event::BlockedCount(2));
        hub.emit(attention(1));
        hub.emit(Event::ProjectsChanged(vec![]));
        assert_eq!(names(&q), [event::ATTENTION]);
        q.set_topics(vec![topic::BLOCKED]);
        assert!(q.is_empty(), "waiting events of a dropped topic go");
        hub.emit(Event::BlockedCount(0));
        assert_eq!(names(&q), [event::BLOCKED_CHANGED]);
        assert_eq!(hub.subscribers(), 1);
        q.close();
        assert_eq!(q.pop(), None);
        assert_eq!(hub.subscribers(), 0);
    }

    #[test]
    fn events_still_reach_the_host() {
        let rec = pitwall_core::testing::RecordingSink::new();
        let hub = Hub::new(rec.clone());
        hub.emit(Event::BlockedCount(3));
        assert!(matches!(rec.take().as_slice(), [Event::BlockedCount(3)]));
    }

    #[test]
    fn who_gets_which_topics() {
        let agent = Caller { pid: Some(1), agent: Some(("a1".into(), "api".into())), process: None, ui: false };
        let ui = Caller { ui: true, ..Caller::outside() };
        let ask = |t: &[&str]| EventsSubscribe { topics: t.iter().map(|s| s.to_string()).collect() };
        assert_eq!(topics(&agent, &ask(&[])).unwrap(), topic::DEFAULT);
        assert_eq!(topics(&Caller::outside(), &ask(&["attention", "later", "attention"])).unwrap(), [topic::ATTENTION]);
        assert_eq!(topics(&agent, &ask(&["approvals"])).unwrap_err().code, code::DENIED);
        assert_eq!(topics(&ui, &ask(&["approvals", "agents"])).unwrap(), [topic::AGENTS]);
    }
}
