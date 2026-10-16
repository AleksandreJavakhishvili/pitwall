//! People on the owner's side (§15.7): which identities are one person, who
//! is blocked, and whether a caller is in a share's audience (§8.2).
//!
//! Identities are one person only when linked: by a team record that lists
//! both for the same member, or by the owner by hand (People). Matching by
//! email is never automatic. Linked identities share conversations, limits
//! and blocks. A person's id is the smallest `provider:subject` among their
//! linked identities ([`PersonId::of`] for one nobody linked), so every
//! network's backend and every part of the core name them the same way; and
//! because ids can change when a new link is made, everything keyed by a
//! person also matches on any of their identities.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::store::{read_json, write_json};
use super::teams::TeamStore;
use crate::error::{PwError, Result};
use crate::net::{Caller, Identity, PersonId};
use pitwall_proto::collab::Audience;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PeopleFile {
    /// Groups of identities the owner linked by hand.
    #[serde(default)]
    links: Vec<Vec<Identity>>,
    /// The owner's block list: takes effect at once, on every share.
    #[serde(default)]
    blocked: Vec<Identity>,
}

/// One person, as the core sees a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: PersonId,
    /// Every identity linked to them, the one they came with first.
    pub identities: Vec<Identity>,
}

impl Person {
    /// Whether `key` (`provider:subject`, or a person id) names this person.
    pub fn is(&self, key: &str) -> bool {
        self.id.as_str() == key || self.identities.iter().any(|i| i.key() == key)
    }

    pub fn has(&self, identity: &Identity) -> bool {
        self.identities.iter().any(|i| i.is(identity))
    }
}

/// The owner's people: links and blocks, plus what team records link.
pub struct People {
    path: PathBuf,
    file: Mutex<PeopleFile>,
    teams: Option<Arc<TeamStore>>,
}

impl People {
    pub fn open(path: PathBuf, teams: Option<Arc<TeamStore>>) -> People {
        let file = read_json(&path).ok().flatten().unwrap_or_default();
        People { path, file: Mutex::new(file), teams }
    }

    fn file(&self) -> MutexGuard<'_, PeopleFile> {
        self.file.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, file: &PeopleFile) -> Result<()> {
        write_json(&self.path, file)
    }

    /// Everyone `identity` is linked to (itself first), following links
    /// transitively through the owner's and the team records'.
    pub fn person(&self, identity: &Identity) -> Person {
        let mut groups = self.file().links.clone();
        if let Some(teams) = &self.teams {
            groups.extend(teams.identity_links());
        }
        let mut found: Vec<Identity> = vec![identity.clone()];
        let mut grew = true;
        while grew {
            grew = false;
            for g in &groups {
                if g.iter().any(|i| found.iter().any(|f| f.is(i))) {
                    for i in g {
                        if !found.iter().any(|f| f.is(i)) {
                            found.push(i.clone());
                            grew = true;
                        }
                    }
                }
            }
        }
        let id = found.iter().map(Identity::key).min().map(PersonId).unwrap_or_else(|| PersonId::of(identity));
        Person { id, identities: found }
    }

    /// For [`Handler::person_of`](crate::net::Handler::person_of): the id
    /// backends put on every `Caller`.
    pub fn person_of(&self, identity: &Identity) -> PersonId {
        self.person(identity).id
    }

    /// The owner links two identities as one person (People → Link).
    pub fn link(&self, a: &Identity, b: &Identity) -> Result<()> {
        if a.is(b) {
            return Err(PwError::other("those are the same identity"));
        }
        let mut file = self.file();
        file.links.push(vec![a.clone(), b.clone()]);
        let file = file.clone();
        self.save(&file)
    }

    /// Undo the owner's own links of `identity` (team records' links stay
    /// until the team changes them).
    pub fn unlink(&self, identity: &Identity) -> Result<()> {
        let mut file = self.file();
        file.links.retain(|g| !g.iter().any(|i| i.is(identity)));
        let file = file.clone();
        self.save(&file)
    }

    /// The owner's hand-made links.
    pub fn links(&self) -> Vec<Vec<Identity>> {
        self.file().links.clone()
    }

    pub fn block(&self, identity: &Identity) -> Result<()> {
        let mut file = self.file();
        if !file.blocked.iter().any(|i| i.is(identity)) {
            file.blocked.push(identity.clone());
        }
        let file = file.clone();
        self.save(&file)
    }

    pub fn unblock(&self, identity: &Identity) -> Result<()> {
        let mut file = self.file();
        file.blocked.retain(|i| !i.is(identity));
        let file = file.clone();
        self.save(&file)
    }

    pub fn blocked(&self) -> Vec<Identity> {
        self.file().blocked.clone()
    }

    /// Blocked under any of their identities.
    pub fn is_blocked(&self, person: &Person) -> bool {
        self.file().blocked.iter().any(|b| person.has(b))
    }
}

/// Why a caller may use a share: `direct`, a team id, a group id or a
/// network id (the directory's `via`).
pub fn audience_via(audience: &Audience, person: &Person, caller: &Caller, teams: Option<&TeamStore>) -> Vec<String> {
    let mut via = Vec::new();
    if audience.people.iter().any(|p| person.has(p)) {
        via.push("direct".to_string());
    }
    if let Some(teams) = teams {
        for t in &audience.teams {
            if teams.is_member(t, &person.identities, caller.groups()) {
                via.push(t.0.clone());
            }
        }
    }
    for g in &audience.groups {
        if caller.groups().contains(g) {
            via.push(g.id.clone());
        }
    }
    if audience.networks.contains(caller.network()) {
        via.push(caller.network().0.clone());
    }
    via
}
