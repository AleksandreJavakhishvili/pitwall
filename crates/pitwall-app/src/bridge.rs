//! The event bridge: engine and server threads → the GPUI main thread
//! (docs/spec/gpui/README.md "Commands and events").
//!
//! The Tauri app turns each engine [`Event`] into a webview event
//! (`src-tauri/src/events.rs`). Here the same events go down a channel as
//! they are, typed; one task on the main thread drains it into the GPUI
//! entities (`agents::AgentStore`), which re-render whoever observes them.
//! Senders never block: the engine calls [`EventSink::emit`] from its own
//! threads.

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

use pitwall_core::events::{Event, EventSink};
use pitwall_proto::ApprovalView;

/// Everything the UI hears from the hosted engine and the CLI server.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Engine(Event),
    /// The pending `pitwall` CLI approvals (Tauri: `approvals-changed`).
    Approvals(Vec<ApprovalView>),
}

/// The sending half, cloned into every producer.
#[derive(Clone)]
pub struct Bridge(UnboundedSender<AppEvent>);

impl Bridge {
    pub fn new() -> (Bridge, UnboundedReceiver<AppEvent>) {
        let (tx, rx) = unbounded();
        (Bridge(tx), rx)
    }

    /// Send; dropped when the UI is gone (quitting).
    pub fn send(&self, event: AppEvent) {
        let _ = self.0.unbounded_send(event);
    }
}

impl EventSink for Bridge {
    fn emit(&self, event: Event) {
        self.send(AppEvent::Engine(event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn events_arrive_in_order_from_any_thread() {
        let (bridge, mut rx) = Bridge::new();
        let sink: std::sync::Arc<dyn EventSink> = std::sync::Arc::new(bridge.clone());
        std::thread::spawn(move || {
            sink.emit(Event::BlockedCount(1));
            sink.emit(Event::BlockedCount(2));
        })
        .join()
        .unwrap();
        bridge.send(AppEvent::Approvals(vec![]));
        drop(bridge);
        let got: Vec<_> = futures::executor::block_on(async {
            let mut out = Vec::new();
            while let Some(e) = rx.next().await {
                out.push(e);
            }
            out
        });
        assert!(matches!(got[0], AppEvent::Engine(Event::BlockedCount(1))));
        assert!(matches!(got[1], AppEvent::Engine(Event::BlockedCount(2))));
        assert!(matches!(got[2], AppEvent::Approvals(ref v) if v.is_empty()));
        assert_eq!(got.len(), 3);
    }
}
