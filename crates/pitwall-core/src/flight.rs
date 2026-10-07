//! One piece of work per key at a time: a caller that asks while the same
//! work is already running waits for it and gets its result, instead of
//! running it again (forced git and worktree refreshes: two windows, a
//! double click and the ticker asking at once read git once).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

enum State<V> {
    Running,
    Done(V),
    /// The work panicked: waiters run it themselves.
    Abandoned,
}

struct Slot<V> {
    state: Mutex<State<V>>,
    done: Condvar,
    /// Callers waiting for this run (tests watch it).
    waiters: AtomicUsize,
}

/// In-flight work by key.
pub(crate) struct Flights<V> {
    slots: Mutex<HashMap<String, Arc<Slot<V>>>>,
}

impl<V> Default for Flights<V> {
    fn default() -> Self {
        Flights { slots: Mutex::default() }
    }
}

/// Removes the slot and wakes waiters even if the work panics.
struct Finish<'a, V> {
    flights: &'a Flights<V>,
    key: &'a str,
    slot: Arc<Slot<V>>,
    value: Option<V>,
}

impl<V> Drop for Finish<'_, V> {
    fn drop(&mut self) {
        lock(&self.flights.slots).remove(self.key);
        *lock(&self.slot.state) = match self.value.take() {
            Some(v) => State::Done(v),
            None => State::Abandoned,
        };
        self.slot.done.notify_all();
    }
}

impl<V: Clone> Flights<V> {
    /// Run `work` for `key`, or — when it is already running — wait for that
    /// run and return its result. Blocking.
    pub fn run(&self, key: &str, work: impl FnOnce() -> V) -> V {
        loop {
            let (slot, leader) = {
                let mut slots = lock(&self.slots);
                match slots.get(key) {
                    Some(s) => (s.clone(), false),
                    None => {
                        let s = Arc::new(Slot { state: Mutex::new(State::Running), done: Condvar::new(), waiters: AtomicUsize::new(0) });
                        slots.insert(key.to_string(), s.clone());
                        (s, true)
                    }
                }
            };
            if leader {
                let mut finish = Finish { flights: self, key, slot, value: None };
                let v = work();
                finish.value = Some(v.clone());
                return v;
            }
            let mut state = lock(&slot.state);
            slot.waiters.fetch_add(1, Ordering::SeqCst);
            while matches!(*state, State::Running) {
                state = slot.done.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if let State::Done(v) = &*state {
                return v.clone();
            }
            // Abandoned: try again (possibly running it ourselves).
        }
    }

    /// Work is running for `key`.
    #[cfg(test)]
    pub fn running(&self, key: &str) -> bool {
        lock(&self.slots).contains_key(key)
    }

    /// Callers waiting for `key`'s running work.
    #[cfg(test)]
    pub fn waiters(&self, key: &str) -> usize {
        lock(&self.slots).get(key).map_or(0, |s| s.waiters.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_second_caller_waits_for_the_running_work() {
        let f: Arc<Flights<u32>> = Arc::default();
        let runs = Arc::new(AtomicUsize::new(0));
        let (go, wait) = mpsc::channel::<()>();
        let leader = {
            let (f, runs) = (f.clone(), runs.clone());
            std::thread::spawn(move || {
                f.run("a", || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    wait.recv().unwrap();
                    7
                })
            })
        };
        while !f.running("a") {
            std::thread::sleep(Duration::from_millis(1));
        }
        let follower = {
            let (f, runs) = (f.clone(), runs.clone());
            std::thread::spawn(move || {
                f.run("a", || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    99
                })
            })
        };
        // Another key isn't held up.
        assert_eq!(f.run("b", || 1), 1);
        while f.waiters("a") == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        go.send(()).unwrap();
        assert_eq!(leader.join().unwrap(), 7);
        assert_eq!(follower.join().unwrap(), 7, "the follower gets the leader's result");
        assert_eq!(runs.load(Ordering::SeqCst), 1, "ran once");
        // Once done, the next call runs again.
        assert_eq!(f.run("a", || 8), 8);
        assert!(!f.running("a"));
    }

    #[test]
    fn a_panic_doesnt_strand_waiters() {
        let f: Arc<Flights<u32>> = Arc::default();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.run("a", || panic!("boom"))));
        assert!(r.is_err());
        assert!(!f.running("a"));
        assert_eq!(f.run("a", || 3), 3);
    }
}
