//! The client side of teams, over [`Network::client`]: probing Pitwalls for
//! listed teams (§3), asking to join or redeeming an invite (§4.6), and
//! pulling records (§4.3). The paths are the `/pitwall/v1/…` endpoints of
//! §8.4; the Fronts' handler serves them from [`super::TeamStore`]'s
//! [`Teams`](crate::collab::Teams) side, with its body shapes
//! (`{"records": […]}`, `{"heads": […]}`, `202 {}`).

use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;

use super::chain::{self, team_id};
use super::keys::SignatureCheck;
use super::{ParsedInvite, TeamStore};
use crate::collab::team::{InviteRedeem, JoinRequest, PitwallInfo, TeamHead, TeamId, TeamRecord};
use crate::error::{ErrorCode, PwError, Result};
use crate::net::{http, Body, DeviceAddr, Network, Probed, Request, Timeouts};
use pitwall_proto::collab::TeamCard;

/// Bodies read from other Pitwalls are capped like requests (§8.1).
const MAX_BODY: usize = crate::collab::MAX_BODY;
/// The connect timeout of a `teams:discover` after a probe answered (§3).
const DISCOVER_CONNECT: Duration = Duration::from_secs(2);

/// `{"records": [TeamRecord]}` (`records`, `teams:discover`).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Records {
    #[serde(default)]
    pub records: Vec<TeamRecord>,
}

/// `{"heads": [TeamHead]}`, the answer to `POST teams/{t}/heads` (whose
/// body is the asker's one `TeamHead`).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Heads {
    #[serde(default)]
    pub heads: Vec<TeamHead>,
}

pub mod paths {
    //! The team endpoints' paths (§8.4), shared with the Fronts' router.
    use crate::collab::team::TeamId;

    pub const INFO: &str = "/pitwall/v1/info";
    pub const DISCOVER: &str = "/pitwall/v1/teams:discover";

    pub fn heads(team: &TeamId) -> String {
        format!("/pitwall/v1/teams/{}/heads", team.0)
    }

    pub fn records(team: &TeamId, after: u64) -> String {
        format!("/pitwall/v1/teams/{}/records?after={after}", team.0)
    }

    pub fn announce(team: &TeamId) -> String {
        format!("/pitwall/v1/teams/{}/announce", team.0)
    }

    pub fn join(team: &TeamId) -> String {
        format!("/pitwall/v1/teams/{}/join", team.0)
    }

    pub fn redeem(team: &TeamId) -> String {
        format!("/pitwall/v1/teams/{}/invites:redeem", team.0)
    }

    pub fn requests(team: &TeamId) -> String {
        format!("/pitwall/v1/teams/{}/requests", team.0)
    }
}

fn network_for(store: &TeamStore, to: &DeviceAddr) -> Result<Arc<dyn Network>> {
    store
        .networks()
        .into_iter()
        .find(|n| n.id() == to.network)
        .ok_or_else(|| PwError::unreachable(format!("not on {}", to.network)))
}

fn call<T: DeserializeOwned>(store: &TeamStore, to: &DeviceAddr, req: Request) -> Result<T> {
    let res = network_for(store, to)?.client(to)?.send(req)?;
    let status = res.status();
    let bytes = res.into_body().read_all(MAX_BODY)?;
    if !status.is_success() {
        let code = match status.as_u16() {
            404 => ErrorCode::NotFound,
            403 => ErrorCode::Denied,
            409 => ErrorCode::Conflict,
            _ => ErrorCode::Other,
        };
        return Err(PwError::new(code, format!("{} answered {status}", to.node)));
    }
    if bytes.is_empty() {
        return serde_json::from_slice(b"null").map_err(|e| PwError::other(e.to_string()));
    }
    serde_json::from_slice(&bytes).map_err(|e| PwError::other(format!("{}: {e}", to.node)))
}

fn get(path: &str) -> Request {
    http::Request::get(path).body(Body::Empty).expect("a valid request")
}

fn post<T: serde::Serialize + ?Sized>(path: &str, body: &T) -> Request {
    http::Request::post(path).header("Content-Type", "application/json").body(Body::json(body)).expect("a valid request")
}

/// A listed record served by `teams:discover` (§3), checked as far as a
/// non-member can: signed by one of its own admins' devices. Members check
/// the whole chain when they join.
pub fn check_listed(record: &TeamRecord, check: &dyn SignatureCheck) -> Result<()> {
    if record.is_genesis() {
        return chain::validate_genesis(record, check).map(|_| ());
    }
    let state = &record.state;
    let signer_ok = state.member(&record.by.member).is_some_and(|m| m.devices.iter().any(|d| d.key == record.by.key))
        && state.is_admin(&record.by.member);
    if !signer_ok || !check.verify(&record.by.key, &chain::signing_bytes(record), &record.sig) {
        return Err(PwError::new(ErrorCode::Denied, "a listed team record that doesn't check"));
    }
    Ok(())
}

/// Pull `teams` from `from` in the background (the side behind pulls).
pub(crate) fn pull_later(store: &TeamStore, from: DeviceAddr, teams: Vec<TeamId>) {
    let Some(store) = store.arc() else { return };
    std::thread::spawn(move || {
        for team in teams {
            if let Err(e) = pull(&store, &from, &team) {
                eprintln!("pitwall: pulling team {} from {}: {e}", team.0, from.node);
            }
        }
    });
}

/// Pull one team's records after our head from `from`, checking each.
/// Returns how many were added.
pub fn pull(store: &TeamStore, from: &DeviceAddr, team: &TeamId) -> Result<usize> {
    let ours = store.head(team).map_or(0, |h| h.version);
    let records: Records = call(store, from, get(&paths::records(team, ours)))?;
    store.accept(team, records.records, ours == 0)
}

/// Exchange heads with another member device for every team we share with
/// it, and pull what we're behind on (§4.3). Returns records added.
pub fn sync_with(store: &TeamStore, peer: &DeviceAddr) -> Result<usize> {
    let mut added = 0;
    for team in store.teams() {
        let shared = store.state(&team).is_some_and(|s| s.members.iter().any(|m| m.devices.iter().any(|d| d.addr.contains(peer))));
        if !shared {
            continue;
        }
        let Some(ours) = store.head(&team) else { continue };
        let theirs: Heads = call(store, peer, post(&paths::heads(&team), &ours))?;
        if theirs.heads.iter().any(|h| h.team == team && h.version > ours.version) {
            added += pull(store, peer, &team)?;
        }
    }
    Ok(added)
}

/// Probe the devices every network lists (§3): `/info` through
/// [`crate::net::Prober`] (256 per refresh, 16 at a time, 2 s to connect;
/// a device that didn't answer is left alone for a day), then
/// `teams:discover` where `/info` lists teams. Caches what checks out and
/// returns the team cards (mine and discovered).
pub fn discover(store: &TeamStore) -> Vec<TeamCard> {
    refresh(store, false)
}

/// [`discover`]; `force` (↻) probes devices that recently didn't answer too.
pub fn refresh(store: &TeamStore, force: bool) -> Vec<TeamCard> {
    for net in store.networks() {
        let Ok(hints) = net.discover() else { continue };
        let answered: Vec<Probed<PitwallInfo>> = store.prober().probe(&*net, &hints, force);
        for p in answered.into_iter().filter(|p| !p.info.listed_teams.is_empty()) {
            let mut req = get(paths::DISCOVER);
            req.extensions_mut().insert(Timeouts { connect: DISCOVER_CONNECT, idle: Some(Duration::from_secs(5)) });
            if let Ok(listed) = call::<Records>(store, &p.hint.addr, req) {
                store.note_discovered(&p.hint.addr, listed.records);
            }
        }
    }
    store.cards()
}

/// Ask to join `team` (§4.6) on `at`, a member's device: the request
/// carries this device's key; the identity is what our network proves.
pub fn request_join(store: &TeamStore, at: &DeviceAddr, team: &TeamId, note: &str) -> Result<()> {
    let request = JoinRequest { note: note.into(), device: store.my_device() };
    call::<serde_json::Value>(store, at, post(&paths::join(team), &request))?;
    let record = store.discovered().into_iter().find(|d| d.record.team.as_ref() == Some(team) || team_id(&d.record) == *team).map(|d| d.record);
    store.note_joining(team, at.clone(), record);
    Ok(())
}

/// Redeem an invite link on the admin's device `at` (found through
/// discovery: the link names the admin's identity, §4.5), then pull the
/// team, pinned to the link's id.
pub fn redeem(store: &TeamStore, at: &DeviceAddr, link: &str) -> Result<TeamId> {
    let invite = ParsedInvite::parse(link).ok_or_else(|| PwError::other("not an invite link"))?;
    let redeem = InviteRedeem { code: invite.code.clone(), device: store.my_device() };
    call::<serde_json::Value>(store, at, post(&paths::redeem(&invite.team), &redeem))?;
    store.note_joining(&invite.team, at.clone(), None);
    pull(store, at, &invite.team)?;
    Ok(invite.team)
}

/// Teams I asked to join: pull each from where I asked, once accepted
/// (records are served to members only). Returns the teams joined.
pub fn poll_joins(store: &TeamStore) -> Vec<TeamId> {
    store
        .joining()
        .into_iter()
        .filter(|j| pull(store, &j.from, &j.team).is_ok_and(|n| n > 0))
        .map(|j| j.team)
        .collect()
}
