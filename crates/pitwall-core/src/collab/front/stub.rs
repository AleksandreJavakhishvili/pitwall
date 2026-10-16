//! In-memory [`Conversations`] and [`Teams`] for the fronts' tests: shares
//! with audiences of people, turns the test moves along by hand, recorded
//! asks. Only what the fronts need; the real core is the Core track's.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard};

use crate::collab::team::{InviteRedeem, JoinRequest, PendingJoin, PitwallInfo, TeamHead, TeamId, TeamRecord};
use crate::collab::turn::{
    Ask, Asker, ChangesPart, ConversationId, ConversationSummary, ShareCard, Turn, TurnId, TurnStatus, Who,
};
use crate::collab::{Conversations, Refusal, Teams, CHANGES_PART_CHARS};
use crate::error::{PwError, Result};
use crate::net::{Caller, Identity, PersonId};

pub const AT: &str = "2026-10-16T10:00:00Z";

/// What a new message becomes.
#[derive(Clone)]
pub enum OnAsk {
    /// Working, until the test moves it.
    Work,
    /// Queued at this position.
    Queue(u32),
    /// Done at once with this reply.
    Reply(String),
    Refuse(Refusal),
}

#[derive(Default)]
struct S {
    shares: Vec<(ShareCard, Vec<PersonId>)>,
    turns: BTreeMap<TurnId, (PersonId, Turn)>,
    conversations: BTreeMap<ConversationId, (PersonId, String, u32)>,
    current: BTreeMap<(PersonId, String), ConversationId>,
    patches: BTreeMap<TurnId, String>,
    watchers: Vec<(TurnId, Sender<Turn>)>,
    ticks: Vec<Sender<()>>,
    asks: Vec<(Asker, Ask)>,
    canceled: Vec<TurnId>,
    next: u32,
    on_ask: Option<OnAsk>,
}

#[derive(Default)]
pub struct StubConversations {
    s: Mutex<S>,
}

fn person(asker: &Asker) -> PersonId {
    match &asker.who {
        Who::Network(c) => c.person().clone(),
        Who::Guest => PersonId("guest".into()),
    }
}

impl StubConversations {
    fn s(&self) -> MutexGuard<'_, S> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A live share for these people.
    pub fn share(&self, card: ShareCard, audience: &[&Identity]) {
        let mut s = self.s();
        s.shares.retain(|(c, _)| c.slug != card.slug);
        s.shares.push((card, audience.iter().map(|i| PersonId::of(i)).collect()));
        s.ticks.retain(|t| t.send(()).is_ok());
    }

    pub fn unshare(&self, slug: &str) {
        let mut s = self.s();
        s.shares.retain(|(c, _)| c.slug != slug);
        s.ticks.retain(|t| t.send(()).is_ok());
    }

    /// A change that isn't anyone's list (a tick nobody should hear of).
    pub fn tick(&self) {
        self.s().ticks.retain(|t| t.send(()).is_ok());
    }

    pub fn on_ask(&self, what: OnAsk) {
        self.s().on_ask = Some(what);
    }

    pub fn asks(&self) -> Vec<(Asker, Ask)> {
        self.s().asks.clone()
    }

    pub fn canceled(&self) -> Vec<TurnId> {
        self.s().canceled.clone()
    }

    pub fn set_patch(&self, turn: &TurnId, patch: &str) {
        self.s().patches.insert(turn.clone(), patch.to_string());
    }

    /// Move a turn along; watchers hear of it.
    pub fn update(&self, turn: &TurnId, f: impl FnOnce(&mut Turn)) {
        let mut s = self.s();
        let Some((_, t)) = s.turns.get_mut(turn) else { panic!("no turn {turn:?}") };
        f(t);
        let t = t.clone();
        let final_ = t.status.is_final();
        s.watchers.retain(|(id, tx)| id != turn || (tx.send(t.clone()).is_ok() && !final_));
    }

    /// Streams and waits following a turn right now.
    pub fn watchers(&self, turn: &TurnId) -> usize {
        self.s().watchers.iter().filter(|(id, _)| id == turn).count()
    }

    pub fn turn_now(&self, turn: &TurnId) -> Turn {
        self.s().turns[turn].1.clone()
    }

    fn visible(s: &S, asker: &Asker, slug: &str) -> bool {
        let p = person(asker);
        s.shares.iter().any(|(c, people)| c.slug == slug && people.contains(&p))
    }

    fn own(s: &S, asker: &Asker, turn: &TurnId) -> Result<Turn> {
        match s.turns.get(turn) {
            Some((p, t)) if *p == person(asker) => Ok(t.clone()),
            _ => Err(PwError::not_found("no such turn")),
        }
    }
}

impl Conversations for StubConversations {
    fn shares(&self, asker: &Asker) -> Vec<ShareCard> {
        let p = person(asker);
        self.s().shares.iter().filter(|(_, people)| people.contains(&p)).map(|(c, _)| c.clone()).collect()
    }

    fn ask(&self, asker: &Asker, ask: Ask) -> Result<Turn> {
        let mut s = self.s();
        if !Self::visible(&s, asker, &ask.share) {
            return Err(PwError::not_found("no such share"));
        }
        let p = person(asker);
        let key = (p.clone(), ask.share.clone());
        let conversation = if let Some(c) = &ask.conversation {
            match s.conversations.get(c) {
                Some((owner, slug, _)) if *owner == p && *slug == ask.share => c.clone(),
                _ => return Err(PwError::not_found("no such conversation")),
            }
        } else if let (false, Some(c)) = (ask.new_conversation, s.current.get(&key)) {
            c.clone()
        } else {
            s.next += 1;
            let c = ConversationId(format!("c_{}", s.next));
            s.conversations.insert(c.clone(), (p.clone(), ask.share.clone(), 0));
            c
        };
        s.current.insert(key, conversation.clone());
        let entry = s.conversations.get_mut(&conversation).expect("conversation");
        entry.2 += 1;
        let n = entry.2;
        let status = match s.on_ask.clone().unwrap_or(OnAsk::Work) {
            OnAsk::Work => (TurnStatus::Working, None),
            OnAsk::Queue(position) => (TurnStatus::Queued { position }, None),
            OnAsk::Reply(text) => {
                (TurnStatus::Done, Some(crate::collab::Reply { text, source: crate::collab::turn::ReplySource::Hook }))
            }
            OnAsk::Refuse(r) => (TurnStatus::Refused(r), None),
        };
        let turn = Turn {
            id: TurnId::new(&conversation, n),
            conversation,
            share: ask.share.clone(),
            status: status.0,
            status_message: None,
            reply: status.1,
            changes: None,
            left: None,
            at: AT.into(),
            done_at: None,
        };
        s.turns.insert(turn.id.clone(), (p, turn.clone()));
        s.asks.push((asker.clone(), ask));
        Ok(turn)
    }

    fn turn(&self, asker: &Asker, turn: &TurnId) -> Result<Turn> {
        Self::own(&self.s(), asker, turn)
    }

    fn watch(&self, asker: &Asker, turn: &TurnId) -> Result<Receiver<Turn>> {
        let mut s = self.s();
        let t = Self::own(&s, asker, turn)?;
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(t.clone());
        if !t.status.is_final() {
            s.watchers.push((turn.clone(), tx));
        }
        Ok(rx)
    }

    fn turns(&self, asker: &Asker, conversation: Option<&ConversationId>) -> Result<Vec<Turn>> {
        let s = self.s();
        let p = person(asker);
        if let Some(c) = conversation {
            if s.conversations.get(c).is_none_or(|(owner, _, _)| *owner != p) {
                return Err(PwError::not_found("no such conversation"));
            }
        }
        let mut out: Vec<Turn> = s
            .turns
            .values()
            .filter(|(owner, t)| *owner == p && conversation.is_none_or(|c| &t.conversation == c))
            .map(|(_, t)| t.clone())
            .collect();
        out.sort_by_key(|t| (t.conversation.clone(), t.id.number()));
        Ok(out)
    }

    fn cancel(&self, asker: &Asker, turn: &TurnId) -> Result<Turn> {
        let t = self.turn(asker, turn)?;
        if t.status.is_final() {
            return Ok(t);
        }
        self.s().canceled.push(turn.clone());
        self.update(turn, |t| t.status = TurnStatus::Canceled);
        self.turn(asker, turn)
    }

    fn patch(&self, asker: &Asker, turn: &TurnId) -> Result<String> {
        let s = self.s();
        let t = Self::own(&s, asker, turn)?;
        match (&t.changes, s.patches.get(turn)) {
            (Some(c), Some(p)) if c.state == crate::collab::turn::ChangesState::Sent => Ok(p.clone()),
            _ => Err(PwError::not_found("no changes")),
        }
    }

    fn patch_part(&self, asker: &Asker, turn: &TurnId, part: u32) -> Result<ChangesPart> {
        let patch = self.patch(asker, turn)?;
        let chars: Vec<char> = patch.chars().collect();
        let parts = chars.len().div_ceil(CHANGES_PART_CHARS).max(1) as u32;
        if part == 0 || part > parts {
            return Err(PwError::not_found("no such part"));
        }
        let from = (part as usize - 1) * CHANGES_PART_CHARS;
        let text: String = chars[from..(from + CHANGES_PART_CHARS).min(chars.len())].iter().collect();
        Ok(ChangesPart { turn: turn.clone(), part, parts, text })
    }

    fn conversations(&self, asker: &Asker) -> Result<Vec<ConversationSummary>> {
        let s = self.s();
        let p = person(asker);
        Ok(s.conversations
            .iter()
            .filter(|(_, (owner, _, _))| *owner == p)
            .map(|(id, (_, slug, n))| ConversationSummary {
                id: id.clone(),
                share: slug.clone(),
                turns: *n,
                current: s.current.get(&(p.clone(), slug.clone())) == Some(id),
                last_at: Some(AT.into()),
            })
            .collect())
    }

    fn forget(&self, asker: &Asker, conversation: &ConversationId) -> Result<()> {
        let mut s = self.s();
        let p = person(asker);
        match s.conversations.get(conversation) {
            Some((owner, _, _)) if *owner == p => {
                s.conversations.remove(conversation);
                s.turns.retain(|_, (_, t)| &t.conversation != conversation);
                s.current.retain(|_, c| c != conversation);
                Ok(())
            }
            _ => Err(PwError::not_found("no such conversation")),
        }
    }

    fn shares_changed(&self) -> Receiver<()> {
        let (tx, rx) = mpsc::channel();
        self.s().ticks.push(tx);
        rx
    }
}

/// Teams: one owner, members by identity, records and announcements.
pub struct StubTeams {
    pub owner: Identity,
    pub members: Vec<Identity>,
    pub records: Mutex<Vec<TeamRecord>>,
    pub joins: Mutex<Vec<(Identity, TeamId, JoinRequest)>>,
    subscribers: Mutex<Vec<Sender<TeamHead>>>,
}

impl StubTeams {
    pub fn new(owner: Identity, members: Vec<Identity>) -> StubTeams {
        StubTeams { owner, members, records: Mutex::default(), joins: Mutex::default(), subscribers: Mutex::default() }
    }

    fn member(&self, caller: &Caller) -> Result<()> {
        if self.members.iter().any(|m| m.is(caller.identity())) {
            Ok(())
        } else {
            Err(PwError::not_found("not found"))
        }
    }

    pub fn announce_head(&self, head: TeamHead) {
        self.subscribers.lock().unwrap().retain(|tx| tx.send(head.clone()).is_ok());
    }
}

impl Teams for StubTeams {
    fn info(&self, _caller: &Caller) -> PitwallInfo {
        PitwallInfo {
            version: "0.0.0-test".into(),
            person: self.owner.clone(),
            label: "studio".into(),
            listed_teams: vec![TeamId("t_9f2c".into())],
            a2a: true,
            mcp: "/mcp".into(),
        }
    }

    fn listed(&self, _caller: &Caller) -> Vec<TeamRecord> {
        self.records.lock().unwrap().clone()
    }

    fn heads(&self, caller: &Caller, theirs: &[TeamHead]) -> Result<Vec<TeamHead>> {
        self.member(caller)?;
        Ok(theirs.iter().map(|h| TeamHead { version: h.version + 1, ..h.clone() }).collect())
    }

    fn records(&self, caller: &Caller, _team: &TeamId, after: u64) -> Result<Vec<TeamRecord>> {
        self.member(caller)?;
        Ok(self.records.lock().unwrap().iter().filter(|r| r.version > after).cloned().collect())
    }

    fn announce(&self, caller: &Caller, _head: TeamHead) -> Result<()> {
        self.member(caller)
    }

    fn request_join(&self, caller: &Caller, team: &TeamId, request: JoinRequest) -> Result<()> {
        self.joins.lock().unwrap().push((caller.identity().clone(), team.clone(), request));
        Ok(())
    }

    fn redeem_invite(&self, _caller: &Caller, _team: &TeamId, redeem: InviteRedeem) -> Result<()> {
        if redeem.code == "good" {
            Ok(())
        } else {
            Err(PwError::not_found("no such invite"))
        }
    }

    fn join_requests(&self, caller: &Caller, _team: &TeamId) -> Result<Vec<PendingJoin>> {
        self.member(caller)?;
        Ok(Vec::new())
    }

    fn announcements(&self, caller: &Caller) -> Result<Receiver<TeamHead>> {
        self.member(caller)?;
        let (tx, rx) = mpsc::channel();
        self.subscribers.lock().unwrap().push(tx);
        Ok(rx)
    }
}
