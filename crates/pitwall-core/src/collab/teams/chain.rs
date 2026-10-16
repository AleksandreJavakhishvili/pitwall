//! Team records as a chain (§4.2): the canonical form that is hashed and
//! signed (RFC 8785 JCS), applying ops to a state, and checking that a
//! record is a valid genesis or a valid successor.
//!
//! C1a has one admin per team, so a chain never forks. A second record at a
//! version we already hold is refused ([`Accept::Conflict`]); the fork
//! choice of §4.4 (revocation wins, removals before additions, lowest hash)
//! lands in C1b at exactly that point.

use serde_json::Value;

use super::keys::{sha256_hex, SignatureCheck};
use crate::collab::team::{DeviceKey, Member, MemberId, Removed, Signer, TeamId, TeamOp, TeamRecord, TeamState};
use crate::error::{ErrorCode, PwError, Result};

// --------------------------------------------------------------- JCS

/// RFC 8785 canonical JSON: no whitespace, object members sorted by their
/// names' UTF-16 code units, strings as JSON escapes them minimally.
/// Numbers in records are integers, written as such.
pub fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
    }
}

fn to_value(record: &TeamRecord) -> Value {
    serde_json::to_value(record).unwrap_or(Value::Null)
}

/// What is signed: the canonical form of every field but `sig` and `cosig`.
pub fn signing_bytes(record: &TeamRecord) -> Vec<u8> {
    let mut v = to_value(record);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("sig");
        obj.remove("cosig");
    }
    canonical(&v).into_bytes()
}

/// `sha256:…` of the whole record's canonical form: what the next record's
/// `prev` names, and what heads compare.
pub fn record_hash(record: &TeamRecord) -> String {
    format!("sha256:{}", sha256_hex(canonical(&to_value(record)).as_bytes()))
}

/// The team id: the genesis record's hash (§4.1), as `t_<hex>`.
pub fn team_id(genesis: &TeamRecord) -> TeamId {
    TeamId(format!("t_{}", sha256_hex(canonical(&to_value(genesis)).as_bytes())))
}

/// The team fingerprint people compare (§4.5): the id's first 6 hex
/// characters in two groups, upper case (`9F2-C1A`).
pub fn fingerprint(team: &TeamId) -> String {
    let hex: String = team.0.trim_start_matches("t_").chars().take(6).collect::<String>().to_uppercase();
    match hex.len() {
        6 => format!("{}-{}", &hex[..3], &hex[3..]),
        _ => hex,
    }
}

// --------------------------------------------------------------- ops

fn invalid(msg: impl Into<String>) -> PwError {
    PwError::new(ErrorCode::Denied, msg)
}

fn member_mut<'a>(state: &'a mut TeamState, id: &MemberId) -> Result<&'a mut Member> {
    state.members.iter_mut().find(|m| &m.member == id).ok_or_else(|| invalid(format!("{} is not a member", id.0)))
}

/// `state` with `ops` applied, as version `version` (§4.2: every member
/// recomputes it). An op that doesn't fit the state is an error.
pub fn apply(state: &TeamState, ops: &[TeamOp], version: u64) -> Result<TeamState> {
    let mut s = state.clone();
    for op in ops {
        match op {
            TeamOp::AddMember { member, identities, devices } => {
                if s.binding.is_some() {
                    return Err(invalid("a bound team follows its provider group"));
                }
                if s.member(member).is_some() {
                    return Err(invalid(format!("{} is already a member", member.0)));
                }
                if identities.is_empty() {
                    return Err(invalid("a member needs an identity"));
                }
                if identities.iter().any(|i| s.member_with(i).is_some()) {
                    return Err(invalid("that identity belongs to another member"));
                }
                s.removed.retain(|r| &r.member != member);
                s.members.push(Member { member: member.clone(), identities: identities.clone(), devices: devices.clone() });
            }
            TeamOp::RemoveMember { member } => {
                if s.binding.is_some() {
                    return Err(invalid("a bound team follows its provider group"));
                }
                if s.member(member).is_none() {
                    return Err(invalid(format!("{} is not a member", member.0)));
                }
                if s.is_admin(member) && s.admins.len() == 1 {
                    return Err(invalid("the last admin can't leave without naming another admin"));
                }
                s.members.retain(|m| &m.member != member);
                s.admins.retain(|a| a != member);
                s.removed.push(Removed { member: member.clone(), at: version });
            }
            TeamOp::AddIdentity { member, identity } => {
                if s.member_with(identity).is_some() {
                    return Err(invalid("that identity already belongs to a member"));
                }
                member_mut(&mut s, member)?.identities.push(identity.clone());
            }
            TeamOp::RemoveIdentity { member, identity } => {
                let m = member_mut(&mut s, member)?;
                if !m.identities.iter().any(|i| i.is(identity)) {
                    return Err(invalid("no such identity"));
                }
                if m.identities.len() == 1 {
                    return Err(invalid("a member keeps at least one identity"));
                }
                m.identities.retain(|i| !i.is(identity));
            }
            TeamOp::AddDevice { member, device } => {
                if s.members.iter().any(|m| m.devices.iter().any(|d| d.key == device.key)) {
                    return Err(invalid("that device is already in the team"));
                }
                member_mut(&mut s, member)?.devices.push(device.clone());
            }
            TeamOp::RemoveDevice { member, key } => {
                let m = member_mut(&mut s, member)?;
                let before = m.devices.len();
                m.devices.retain(|d| &d.key != key);
                if m.devices.len() == before {
                    return Err(invalid("no such device"));
                }
            }
            TeamOp::RotateDevice { member, old, new } => {
                let m = member_mut(&mut s, member)?;
                let d = m.devices.iter_mut().find(|d| &d.key == old).ok_or_else(|| invalid("no such device"))?;
                d.key = new.clone();
            }
            TeamOp::AddAdmin { member } => {
                if s.member(member).is_none() {
                    return Err(invalid(format!("{} is not a member", member.0)));
                }
                if !s.is_admin(member) {
                    s.admins.push(member.clone());
                }
            }
            TeamOp::RemoveAdmin { member } => {
                if !s.is_admin(member) {
                    return Err(invalid(format!("{} is not an admin", member.0)));
                }
                if s.admins.len() == 1 {
                    return Err(invalid("the last admin can't step down without naming another admin"));
                }
                s.admins.retain(|a| a != member);
            }
            TeamOp::SetInfo { name, description, listed } => {
                if let Some(n) = name {
                    s.name = n.clone();
                }
                if let Some(d) = description {
                    s.description = d.clone();
                }
                if let Some(l) = listed {
                    s.listed = *l;
                }
            }
            TeamOp::SetJoin { join } => s.join = *join,
            TeamOp::Bind { binding } => s.binding = Some(binding.clone()),
            TeamOp::Unbind => s.binding = None,
        }
    }
    Ok(s)
}

fn device_of<'a>(state: &'a TeamState, by: &Signer) -> Option<&'a Member> {
    state.member(&by.member).filter(|m| m.devices.iter().any(|d| d.key == by.key))
}

/// Whether `by` may sign `ops` on top of `prev` (§4.2, §4.6): a device of
/// an admin in the previous state; or a member's own device for their own
/// entry only: leaving, and rotating that device's key.
fn authorized(prev: &TeamState, by: &Signer, ops: &[TeamOp]) -> bool {
    if device_of(prev, by).is_none() {
        return false;
    }
    if prev.is_admin(&by.member) {
        return true;
    }
    !ops.is_empty()
        && ops.iter().all(|op| match op {
            TeamOp::RemoveMember { member } => member == &by.member,
            TeamOp::RotateDevice { member, old, .. } => member == &by.member && old == &by.key,
            _ => false,
        })
}

fn check_sig(record: &TeamRecord, check: &dyn SignatureCheck) -> Result<()> {
    if check.verify(&record.by.key, &signing_bytes(record), &record.sig) {
        Ok(())
    } else {
        Err(invalid("the record's signature doesn't check"))
    }
}

/// A valid genesis (§4.2): version 1, no `team` or `prev`, signed by a
/// device listed in its own state whose member is an admin. Returns the
/// team id it founds.
pub fn validate_genesis(record: &TeamRecord, check: &dyn SignatureCheck) -> Result<TeamId> {
    if !record.is_genesis() || record.team.is_some() {
        return Err(invalid("not a genesis record"));
    }
    if device_of(&record.state, &record.by).is_none() || !record.state.is_admin(&record.by.member) {
        return Err(invalid("a genesis record is signed by one of its admins' devices"));
    }
    if record.state.admins.iter().any(|a| record.state.member(a).is_none()) {
        return Err(invalid("every admin is a member"));
    }
    check_sig(record, check)?;
    Ok(team_id(record))
}

/// A valid successor of `prev` in team `team` (§4.2): the next version,
/// chained to `prev`'s hash, signed by an authorized device of the previous
/// state, and its state is exactly `prev`'s with its ops applied.
pub fn validate_next(team: &TeamId, prev: &TeamRecord, record: &TeamRecord, check: &dyn SignatureCheck) -> Result<()> {
    if record.team.as_ref() != Some(team) {
        return Err(invalid("the record belongs to another team"));
    }
    if record.version != prev.version + 1 {
        return Err(invalid(format!("expected version {}, got {}", prev.version + 1, record.version)));
    }
    if record.prev.as_deref() != Some(record_hash(prev).as_str()) {
        return Err(invalid("the record doesn't chain to the previous one"));
    }
    if record.ops.is_empty() {
        return Err(invalid("a record changes something"));
    }
    if !authorized(&prev.state, &record.by, &record.ops) {
        return Err(invalid("the signer may not make this change"));
    }
    let expected = apply(&prev.state, &record.ops, record.version)?;
    if expected != record.state {
        return Err(invalid("the record's state isn't its ops applied to the previous state"));
    }
    check_sig(record, check)
}

/// What became of a record offered to a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accept {
    /// Appended.
    Added,
    /// Already held (same hash).
    Known,
    /// Older than or equal to our head with a different hash: a fork. C1a
    /// keeps its own branch; C1b's fork choice (§4.4) decides here.
    Conflict,
    /// Beyond our head + 1: pull the records in between first.
    Gap,
}

/// One team's validated chain, genesis first.
#[derive(Debug, Clone)]
pub struct Chain {
    pub id: TeamId,
    pub records: Vec<TeamRecord>,
}

impl Chain {
    /// A chain from its genesis.
    pub fn found(genesis: TeamRecord, check: &dyn SignatureCheck) -> Result<Chain> {
        let id = validate_genesis(&genesis, check)?;
        Ok(Chain { id, records: vec![genesis] })
    }

    /// A whole chain as stored or received, every link checked.
    pub fn from_records(records: Vec<TeamRecord>, check: &dyn SignatureCheck) -> Result<Chain> {
        let mut it = records.into_iter();
        let genesis = it.next().ok_or_else(|| invalid("an empty chain"))?;
        let mut chain = Chain::found(genesis, check)?;
        for r in it {
            match chain.offer(r, check)? {
                Accept::Added | Accept::Known => {}
                other => return Err(invalid(format!("a broken chain ({other:?})"))),
            }
        }
        Ok(chain)
    }

    pub fn head(&self) -> &TeamRecord {
        self.records.last().expect("a chain has its genesis")
    }

    pub fn state(&self) -> &TeamState {
        &self.head().state
    }

    pub fn version(&self) -> u64 {
        self.head().version
    }

    /// Offer one record (from replication, or one we wrote).
    pub fn offer(&mut self, record: TeamRecord, check: &dyn SignatureCheck) -> Result<Accept> {
        let head = self.version();
        if record.version == 0 {
            return Err(invalid("versions start at 1"));
        }
        if record.version <= head {
            let ours = &self.records[(record.version - 1) as usize];
            return Ok(if record_hash(ours) == record_hash(&record) { Accept::Known } else { Accept::Conflict });
        }
        if record.version > head + 1 {
            return Ok(Accept::Gap);
        }
        validate_next(&self.id, self.head(), &record, check)?;
        self.records.push(record);
        Ok(Accept::Added)
    }

    /// Records after version `after` (replication, §4.3).
    pub fn after(&self, after: u64) -> Vec<TeamRecord> {
        self.records.iter().filter(|r| r.version > after).cloned().collect()
    }
}

/// Is `key` a device in the team's current state?
pub fn has_device(state: &TeamState, key: &DeviceKey) -> Option<MemberId> {
    state.members.iter().find(|m| m.devices.iter().any(|d| &d.key == key)).map(|m| m.member.clone())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_sorts_members_and_drops_whitespace() {
        let v = json!({"b": 1, "a": [true, null, "x\n\u{e9}"], "\u{20ac}": {}, "A": 2});
        assert_eq!(canonical(&v), "{\"A\":2,\"a\":[true,null,\"x\\n\u{e9}\"],\"b\":1,\"\u{20ac}\":{}}");
        // UTF-16 order: U+FB33 sorts after a surrogate pair's high half (U+1F600).
        let v = json!({"\u{fb33}": 1, "\u{1f600}": 2});
        assert_eq!(canonical(&v), "{\"\u{1f600}\":2,\"\u{fb33}\":1}");
    }

    #[test]
    fn fingerprints_are_six_characters_in_two_groups() {
        assert_eq!(fingerprint(&TeamId("t_9f2c1a77".into())), "9F2-C1A");
    }
}
