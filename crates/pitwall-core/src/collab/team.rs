//! Teams as signed records (§4.2): each record is the whole team state after
//! one change, plus the change, signed by an admin's device and chained to
//! the previous record. Validation, the fork choice (§4.4) and replication
//! (§4.3) build on these shapes; the canonical form that is signed is RFC
//! 8785 JCS of every field but `sig` (and `cosig`).

use serde::{Deserialize, Serialize};

use crate::net::{DeviceAddr, Identity, NetworkId};
pub use pitwall_proto::collab::{JoinPolicy, TeamId};

/// A team-local person id (`p_carol`): one member, with one or more
/// identities from any network.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MemberId(pub String);

/// A device's instance public key (`ed25519:…`), tailnet-peers.md §5.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceKey(pub String);

/// One Pitwall installation of a member: its key, and its address on each
/// network it is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamDevice {
    pub key: DeviceKey,
    pub addr: Vec<DeviceAddr>,
}

/// A member. Plain MCP users have identities and no devices (§4.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub member: MemberId,
    pub identities: Vec<Identity>,
    #[serde(default)]
    pub devices: Vec<TeamDevice>,
}

/// A member removed at a version, kept so their old signatures stay
/// explainable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub member: MemberId,
    pub at: u64,
}

/// A team that follows a provider's group instead of its own member list
/// (§4.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub network: NetworkId,
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_group: Option<String>,
}

/// The whole team after a record's ops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamState {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Returned by `teams:discover` (§3); invite-only otherwise.
    pub listed: bool,
    pub join: JoinPolicy,
    pub admins: Vec<MemberId>,
    pub members: Vec<Member>,
    #[serde(default)]
    pub removed: Vec<Removed>,
    /// Always written (`null` when unbound).
    #[serde(default)]
    pub binding: Option<Binding>,
}

impl TeamState {
    pub fn member(&self, id: &MemberId) -> Option<&Member> {
        self.members.iter().find(|m| &m.member == id)
    }

    /// The member one of whose identities this is (§4.2 "a request matches a
    /// member when its `Caller.identity` is one of theirs").
    pub fn member_with(&self, identity: &Identity) -> Option<&Member> {
        self.members.iter().find(|m| m.identities.iter().any(|i| i.is(identity)))
    }

    pub fn is_admin(&self, id: &MemberId) -> bool {
        self.admins.contains(id)
    }
}

/// One change in a record (§4.2 "Ops").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TeamOp {
    AddMember {
        member: MemberId,
        identities: Vec<Identity>,
        #[serde(default)]
        devices: Vec<TeamDevice>,
    },
    RemoveMember {
        member: MemberId,
    },
    AddIdentity {
        member: MemberId,
        identity: Identity,
    },
    RemoveIdentity {
        member: MemberId,
        identity: Identity,
    },
    AddDevice {
        member: MemberId,
        device: TeamDevice,
    },
    RemoveDevice {
        member: MemberId,
        key: DeviceKey,
    },
    /// Signed by the device's own old key, for its own entry (§4.6).
    RotateDevice {
        member: MemberId,
        old: DeviceKey,
        new: DeviceKey,
    },
    AddAdmin {
        member: MemberId,
    },
    RemoveAdmin {
        member: MemberId,
    },
    /// Only the fields given change.
    SetInfo {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        listed: Option<bool>,
    },
    SetJoin {
        join: JoinPolicy,
    },
    Bind {
        binding: Binding,
    },
    Unbind,
}

/// Who signed: a member and one of their devices' keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signer {
    pub member: MemberId,
    pub key: DeviceKey,
}

/// A second admin's signature (`remove_admin` in teams with three or more
/// admins, §4.4; open question 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cosignature {
    pub by: Signer,
    pub sig: String,
}

/// One link of a team's chain, `collab/teams/<team>/chain.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRecord {
    /// The genesis hash; absent in the genesis record itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamId>,
    pub version: u64,
    /// `sha256:…` of the accepted record `version - 1`; absent in genesis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    #[serde(default)]
    pub ops: Vec<TeamOp>,
    pub state: TeamState,
    pub by: Signer,
    /// Informative only; never used for ordering.
    pub at: String,
    /// `ed25519:…` over the canonical JSON of every other field.
    pub sig: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cosig: Vec<Cosignature>,
}

impl TeamRecord {
    pub fn is_genesis(&self) -> bool {
        self.version == 1 && self.prev.is_none()
    }
}

// ----------------------------------------------- Pitwall endpoints (§8.4)

/// `GET /pitwall/v1/info`: the discovery probe's answer (§3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitwallInfo {
    /// Pitwall's version.
    pub version: String,
    /// This device's owner, as the caller's network knows them.
    pub person: Identity,
    /// The device's name (`studio`).
    pub label: String,
    pub listed_teams: Vec<TeamId>,
    pub a2a: bool,
    /// The MCP front's path (`/mcp`).
    pub mcp: String,
}

/// A team's latest record, as replication compares them (§4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamHead {
    pub team: TeamId,
    pub version: u64,
    pub hash: String,
}

/// `POST /pitwall/v1/teams/{t}/join`. The identity is the caller's; the key
/// is bound to the device the request came from (§4.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    #[serde(default)]
    pub note: String,
    pub device: TeamDevice,
}

/// `POST /pitwall/v1/teams/{t}/invites:redeem`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteRedeem {
    /// The one-time code from the invite link.
    pub code: String,
    pub device: TeamDevice,
}

/// A join request waiting for an admin (`GET /pitwall/v1/teams/{t}/requests`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingJoin {
    pub team: TeamId,
    /// Who asked, as their network proved it.
    pub identity: Identity,
    pub device: TeamDevice,
    pub note: String,
    pub at: String,
}
