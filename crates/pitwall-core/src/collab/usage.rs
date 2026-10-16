//! Usage per share, per caller, per day (§5.8), kept in `collab/usage.jsonl`
//! on the owner's side: one line per accepted message and per finished
//! turn's agent time. Days are the owner's local days; the daily limits
//! reset at local midnight.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::people::Person;
use super::store::{append_jsonl, read_jsonl};

/// One line of `usage.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLine {
    /// The owner's local day (`2026-10-16`).
    pub day: String,
    pub share: String,
    /// The caller's person id, and the identity they came with.
    pub person: String,
    pub identity: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub messages: u32,
    /// Agent time of a finished turn, in milliseconds.
    #[serde(default, skip_serializing_if = "is_zero64")]
    pub agent_ms: u64,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn is_zero64(n: &u64) -> bool {
    *n == 0
}

/// A day's usage of one caller on one share (the owner's Usage view).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub day: String,
    pub share: String,
    pub person: String,
    pub messages: u32,
    pub agent_ms: u64,
}

/// What `usage.jsonl` says, summed per (day, share, identity).
pub struct Usage {
    path: PathBuf,
    /// Keyed by (day, share, identity key): a person's linked identities
    /// may change, so rows match a person on any of them.
    rows: BTreeMap<(String, String, String), UsageRow>,
}

impl Usage {
    pub fn open(path: PathBuf) -> Usage {
        let mut usage = Usage { path, rows: BTreeMap::new() };
        for line in read_jsonl::<UsageLine>(&usage.path.clone()) {
            usage.add(&line);
        }
        usage
    }

    fn add(&mut self, line: &UsageLine) {
        let key = (line.day.clone(), line.share.clone(), line.identity.clone());
        let row = self.rows.entry(key).or_insert_with(|| UsageRow {
            day: line.day.clone(),
            share: line.share.clone(),
            person: line.person.clone(),
            ..UsageRow::default()
        });
        row.person = line.person.clone();
        row.messages += line.messages;
        row.agent_ms += line.agent_ms;
    }

    fn record(&mut self, line: UsageLine) {
        if let Err(e) = append_jsonl(&self.path, &line) {
            eprintln!("pitwall: collaboration usage: {e}");
        }
        self.add(&line);
    }

    pub fn message(&mut self, day: &str, share: &str, person: &Person) {
        self.record(UsageLine {
            day: day.into(),
            share: share.into(),
            person: person.id.0.clone(),
            identity: person.identities.first().map(|i| i.key()).unwrap_or_default(),
            messages: 1,
            agent_ms: 0,
        });
    }

    pub fn agent_time(&mut self, day: &str, share: &str, person: &Person, ms: u64) {
        if ms == 0 {
            return;
        }
        self.record(UsageLine {
            day: day.into(),
            share: share.into(),
            person: person.id.0.clone(),
            identity: person.identities.first().map(|i| i.key()).unwrap_or_default(),
            messages: 0,
            agent_ms: ms,
        });
    }

    /// Messages `person` (under any of their identities) sent to `share` on `day`.
    pub fn messages_of(&self, day: &str, share: &str, person: &Person) -> u32 {
        self.rows
            .iter()
            .filter(|((d, s, i), row)| d == day && s == share && (person.is(&row.person) || person.is(i)))
            .map(|(_, row)| row.messages)
            .sum()
    }

    /// Agent time of `share` on `day`, all callers together.
    pub fn agent_ms(&self, day: &str, share: &str) -> u64 {
        self.rows.iter().filter(|((d, s, _), _)| d == day && s == share).map(|(_, r)| r.agent_ms).sum()
    }

    /// Agent time of one caller on `share` on `day`.
    pub fn agent_ms_of(&self, day: &str, share: &str, person: &Person) -> u64 {
        self.rows
            .iter()
            .filter(|((d, s, i), row)| d == day && s == share && (person.is(&row.person) || person.is(i)))
            .map(|(_, r)| r.agent_ms)
            .sum()
    }

    /// Every row of `day` (the owner's Usage view).
    pub fn day(&self, day: &str) -> Vec<UsageRow> {
        self.rows.iter().filter(|((d, _, _), _)| d == day).map(|(_, r)| r.clone()).collect()
    }
}
