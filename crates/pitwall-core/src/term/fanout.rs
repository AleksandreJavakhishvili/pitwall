//! One agent's output: a bounded history ring plus the live subscribers.
//! A subscriber is a plain closure, so the host decides where bytes go (the
//! app: a webview channel; the daemon: socket frames; tests: a `Vec`).

use std::collections::VecDeque;

/// Receives output bytes. Returning `false` unsubscribes it (its consumer is
/// gone).
pub type OutputSink = Box<dyn FnMut(&[u8]) -> bool + Send>;

/// Wrap `sink` so a stream of small chunks reaches it as fewer, larger ones:
/// a chunk after a quiet spell is passed on at once (typing stays instant),
/// then whatever arrives within `gap` goes as one batch, so a busy agent
/// costs the consumer (a webview IPC message each) at most one call per
/// `gap`. Runs on its own thread, which ends when the subscription is
/// dropped or `sink` refuses a batch.
pub fn coalesce(mut sink: OutputSink, gap: std::time::Duration) -> OutputSink {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let spawned = std::thread::Builder::new().name("term-out".into()).spawn(move || {
        while let Ok(mut buf) = rx.recv() {
            while let Ok(more) = rx.try_recv() {
                buf.extend_from_slice(&more);
            }
            if !sink(&buf) {
                return;
            }
            std::thread::sleep(gap);
        }
    });
    if spawned.is_err() {
        return Box::new(|_: &[u8]| false);
    }
    Box::new(move |chunk: &[u8]| tx.send(chunk.to_vec()).is_ok())
}

pub(crate) struct Fanout {
    ring: VecDeque<u8>,
    ring_cap: usize,
    subscribers: Vec<(u64, OutputSink)>,
    max_subscribers: usize,
}

impl Fanout {
    pub fn new(ring_cap: usize, max_subscribers: usize) -> Fanout {
        Fanout { ring: VecDeque::new(), ring_cap, subscribers: Vec::new(), max_subscribers }
    }

    /// Remember `chunk` and hand it to every subscriber.
    pub fn push(&mut self, chunk: &[u8]) {
        push_ring(&mut self.ring, chunk, self.ring_cap);
        self.subscribers.retain_mut(|(_, sink)| sink(chunk));
    }

    /// Everything still in the ring, oldest first.
    pub fn history(&self) -> Vec<u8> {
        let (a, b) = self.ring.as_slices();
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        out
    }

    /// Replay the history to `sink`, then keep it for live output; the oldest
    /// subscriber makes room when full. Returns `false` (and drops the sink)
    /// when it refused the replay.
    pub fn subscribe(&mut self, id: u64, mut sink: OutputSink) -> bool {
        let replay = self.history();
        if !replay.is_empty() && !sink(&replay) {
            return false;
        }
        if self.subscribers.len() >= self.max_subscribers {
            drop(self.subscribers.remove(0));
        }
        self.subscribers.push((id, sink));
        true
    }

    pub fn unsubscribe(&mut self, id: u64) {
        self.subscribers.retain(|(sub, _)| *sub != id);
    }
}

fn push_ring(ring: &mut VecDeque<u8>, chunk: &[u8], cap: usize) {
    if chunk.len() >= cap {
        ring.clear();
        ring.extend(&chunk[chunk.len() - cap..]);
        return;
    }
    let overflow = (ring.len() + chunk.len()).saturating_sub(cap);
    ring.drain(..overflow);
    ring.extend(chunk);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A sink that collects into a shared Vec; `alive` = false makes it refuse.
    fn collector(alive: bool) -> (OutputSink, Arc<Mutex<Vec<u8>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        (
            Box::new(move |b: &[u8]| {
                g.lock().unwrap().extend_from_slice(b);
                alive
            }),
            got,
        )
    }

    #[test]
    fn ring_keeps_newest_bytes() {
        let mut ring = VecDeque::new();
        push_ring(&mut ring, b"abcdef", 8);
        push_ring(&mut ring, b"ghij", 8);
        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), b"cdefghij");
        push_ring(&mut ring, b"0123456789", 8);
        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), b"23456789");
    }

    #[test]
    fn subscribers_get_history_then_live_output() {
        let mut f = Fanout::new(8, 4);
        f.push(b"old-");
        let (sink, got) = collector(true);
        assert!(f.subscribe(1, sink));
        f.push(b"new");
        assert_eq!(&*got.lock().unwrap(), b"old-new");
        assert_eq!(f.history(), b"old-new");

        f.unsubscribe(1);
        f.push(b"!");
        assert_eq!(&*got.lock().unwrap(), b"old-new", "no output after unsubscribe");
    }

    #[test]
    fn gone_consumers_are_dropped() {
        let mut f = Fanout::new(64, 4);
        // Nothing to replay yet: accepted, then dropped on the first chunk.
        let (dead, _) = collector(false);
        assert!(f.subscribe(1, dead));
        f.push(b"x");
        assert!(f.subscribers.is_empty());
        // With history, a refusing sink is never added.
        let (dead, _) = collector(false);
        assert!(!f.subscribe(2, dead));
        assert!(f.subscribers.is_empty());
    }

    #[test]
    fn coalesced_output_arrives_whole_and_batched() {
        let calls = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let c = calls.clone();
        let mut sink = coalesce(
            Box::new(move |b: &[u8]| {
                c.lock().unwrap().push(b.to_vec());
                true
            }),
            std::time::Duration::from_millis(30),
        );
        assert!(sink(b"a"));
        for _ in 0..50 {
            assert!(sink(b"b"));
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while calls.lock().unwrap().concat().len() < 51 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let got = calls.lock().unwrap().clone();
        assert_eq!(got.concat(), [b"a".to_vec(), vec![b'b'; 50]].concat(), "everything, in order");
        assert!(got.len() <= 3, "batched: {} calls", got.len());
    }

    #[test]
    fn coalesced_sink_reports_a_gone_consumer() {
        let mut sink = coalesce(Box::new(|_: &[u8]| false), std::time::Duration::from_millis(1));
        sink(b"x");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while sink(b"y") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!sink(b"z"), "unsubscribes once the consumer refused");
    }

    #[test]
    fn oldest_subscriber_makes_room() {
        let mut f = Fanout::new(64, 2);
        let (a, got_a) = collector(true);
        let (b, _) = collector(true);
        let (c, got_c) = collector(true);
        f.subscribe(1, a);
        f.subscribe(2, b);
        f.subscribe(3, c);
        f.push(b"hi");
        assert!(got_a.lock().unwrap().is_empty());
        assert_eq!(&*got_c.lock().unwrap(), b"hi");
        assert_eq!(f.subscribers.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![2, 3]);
    }
}
