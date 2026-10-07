//! Approvals (architecture.md §4): a risky request waits here until the user
//! answers Pitwall's dialog, or the timeout denies it. Only the host's UI
//! answers (the app in-process; over the socket only a verified UI client),
//! so an agent can never approve its own request.

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pitwall_proto::{ApprovalAnswer, ApprovalView, Requester, Risk};

/// How a request was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    /// The user allowed it earlier for this caller ("Remember").
    Remembered,
    Denied,
    /// Nobody answered in time (counts as a denial).
    TimedOut,
}

impl Decision {
    pub fn allowed(self) -> bool {
        matches!(self, Decision::Allowed | Decision::Remembered)
    }
}

/// What to ask the user.
pub struct Ask {
    /// The method ("session.add").
    pub action: String,
    pub summary: String,
    pub details: Vec<String>,
    pub requester: Requester,
    /// Remembered answers are per caller (`Caller::key`) and action.
    pub caller_key: String,
    pub risk: Risk,
}

struct Pending {
    view: ApprovalView,
    caller_key: String,
    answer: Option<(bool, bool)>,
}

#[derive(Default)]
struct State {
    pending: Vec<Pending>,
    /// (caller key, action) the user allowed without asking again.
    remembered: HashSet<(String, String)>,
}

type Listener = Box<dyn Fn(Vec<ApprovalView>) + Send + Sync>;

pub struct Approvals {
    state: Mutex<State>,
    answered: Condvar,
    listeners: Mutex<Vec<Listener>>,
    timeout: Duration,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Approvals {
    /// Unanswered requests are denied after `timeout`.
    pub fn new(timeout: Duration) -> Arc<Approvals> {
        Arc::new(Approvals { state: Mutex::default(), answered: Condvar::new(), listeners: Mutex::default(), timeout })
    }

    /// Called with the pending list whenever it changes (the app shows the
    /// dialog from it). Called on the requesting thread; must not block.
    pub fn on_change(&self, f: impl Fn(Vec<ApprovalView>) + Send + Sync + 'static) {
        lock(&self.listeners).push(Box::new(f));
    }

    pub fn pending(&self) -> Vec<ApprovalView> {
        lock(&self.state).pending.iter().map(|p| p.view.clone()).collect()
    }

    fn changed(&self) {
        let list = self.pending();
        for f in lock(&self.listeners).iter() {
            f(list.clone());
        }
    }

    /// Ask the user and wait for the answer (or the timeout). Blocking.
    pub fn ask(&self, ask: Ask) -> Decision {
        let rememberable = ask.risk == Risk::Low;
        let key = (ask.caller_key.clone(), ask.action.clone());
        let id = uuid::Uuid::new_v4().to_string();
        {
            let mut st = lock(&self.state);
            if rememberable && st.remembered.contains(&key) {
                return Decision::Remembered;
            }
            let now = unix_ms();
            st.pending.push(Pending {
                view: ApprovalView {
                    id: id.clone(),
                    action: ask.action,
                    summary: ask.summary,
                    details: ask.details,
                    requester: ask.requester,
                    risk: ask.risk,
                    rememberable,
                    created_at: now,
                    expires_at: now + self.timeout.as_millis() as u64,
                },
                caller_key: ask.caller_key,
                answer: None,
            });
        }
        self.changed();
        let deadline = Instant::now() + self.timeout;
        let decision = {
            let mut st = lock(&self.state);
            loop {
                let i = st.pending.iter().position(|p| p.view.id == id).expect("only ask() removes it");
                if let Some((allow, remember)) = st.pending[i].answer {
                    let p = st.pending.remove(i);
                    if allow && remember && rememberable {
                        st.remembered.insert((p.caller_key, p.view.action));
                    }
                    break if allow { Decision::Allowed } else { Decision::Denied };
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    st.pending.remove(i);
                    break Decision::TimedOut;
                }
                st = self.answered.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner()).0;
            }
        };
        self.changed();
        decision
    }

    /// The user's answer. Callers must have checked that it comes from
    /// Pitwall's own UI.
    pub fn answer(&self, a: &ApprovalAnswer) -> Result<(), String> {
        let mut st = lock(&self.state);
        let p = st
            .pending
            .iter_mut()
            .find(|p| p.view.id == a.id && p.answer.is_none())
            .ok_or_else(|| format!("no pending approval {}", a.id))?;
        p.answer = Some((a.allow, a.remember));
        self.answered.notify_all();
        Ok(())
    }

    /// Forget every remembered answer.
    pub fn forget_remembered(&self) {
        lock(&self.state).remembered.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::RequesterKind;

    fn ask(risk: Risk, caller: &str) -> Ask {
        Ask {
            action: "session.add".into(),
            summary: "start".into(),
            details: vec![],
            requester: Requester { kind: RequesterKind::Outside, agent_id: None, name: "x".into(), pid: None, process: None },
            caller_key: caller.into(),
            risk,
        }
    }

    /// Answer the first approval that shows up.
    fn answer_when_asked(a: &Arc<Approvals>, allow: bool, remember: bool) -> std::thread::JoinHandle<()> {
        let a = a.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = a.pending().first() {
                a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember }).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        })
    }

    #[test]
    fn allow_deny_and_timeout() {
        let a = Approvals::new(Duration::from_secs(10));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        a.on_change(move |l| lock(&s).push(l.len()));
        let t = answer_when_asked(&a, true, false);
        assert_eq!(a.ask(ask(Risk::Low, "outside")), Decision::Allowed);
        t.join().unwrap();
        assert_eq!(*lock(&seen), [1, 0], "shown, then gone");
        let t = answer_when_asked(&a, false, false);
        assert_eq!(a.ask(ask(Risk::Low, "outside")), Decision::Denied);
        t.join().unwrap();
        assert!(a.pending().is_empty());
        let quick = Approvals::new(Duration::from_millis(50));
        assert_eq!(quick.ask(ask(Risk::High, "outside")), Decision::TimedOut);
        assert!(quick.pending().is_empty(), "a timed-out request is withdrawn");
        assert!(quick.answer(&ApprovalAnswer { id: "nope".into(), allow: true, remember: false }).is_err());
    }

    #[test]
    fn remembering_is_per_caller_and_only_for_low_risk() {
        let a = Approvals::new(Duration::from_millis(200));
        let t = answer_when_asked(&a, true, true);
        assert_eq!(a.ask(ask(Risk::Low, "agent:1")), Decision::Allowed);
        t.join().unwrap();
        assert_eq!(a.ask(ask(Risk::Low, "agent:1")), Decision::Remembered);
        assert_eq!(a.ask(ask(Risk::Low, "agent:2")), Decision::TimedOut, "another caller is asked");
        let t = answer_when_asked(&a, true, true);
        assert_eq!(a.ask(ask(Risk::High, "agent:3")), Decision::Allowed);
        t.join().unwrap();
        assert_eq!(a.ask(ask(Risk::High, "agent:3")), Decision::TimedOut, "high risk is asked every time");
        a.forget_remembered();
        assert_eq!(a.ask(ask(Risk::Low, "agent:1")), Decision::TimedOut);
    }
}
