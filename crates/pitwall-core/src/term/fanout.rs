//! One agent's output: a bounded history ring plus the live subscribers.
//! A subscriber is a plain closure, so the host decides where bytes go (the
//! app: a webview channel; the daemon: socket frames; tests: a `Vec`).

use std::collections::VecDeque;

/// Receives output bytes. Returning `false` unsubscribes it (its consumer is
/// gone).
pub type OutputSink = Box<dyn FnMut(&[u8]) -> bool + Send>;

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
