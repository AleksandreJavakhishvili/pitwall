//! `pitwall people`, `team`, `share` and `ask` (collaboration/protocol.md
//! §8.6): the socket's collaboration methods (`caps::COLLAB`). Widening a
//! live share, going live and team changes wait for the user's approval in
//! Pitwall (exit 3 when denied); refusals come back with their own codes
//! (`quota`, `busy`, `offline`, `not_allowed`, …).

use std::collections::BTreeMap;
use std::fmt::Write;
use std::io::Read;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use pitwall_client::{Client, Error};
use pitwall_proto::api::collab::{
    capability_chip, describe, state_word, AskFilter, AskSend, AudienceEdit, MemberAdded, PersonView, ProbeResult, SetupLine,
    ShareBlock, ShareChanged, ShareCreate, ShareGuest, SharePatch, ShareRef, ShareSet, ShareView, TeamAddMember, TeamApprove, TeamCreate,
    TeamDetail, TeamJoin, TeamRef,
};
use pitwall_proto::collab::{AskStatus, AskView, Capability, Identity, JoinPolicy, LimitOverride, NetworkId, Programs, SharedSessionView, Start, TeamCard};
use pitwall_proto::{caps, code, method, ErrorBody};

use crate::args::{AskArgs, PeopleCmd, ShareCmd, ShareFlags, TeamCmd};
use crate::render::table;
use crate::Output;

/// How long `ask` waits by default.
pub const ASK_TIMEOUT: u64 = 600;

fn err(c: &str, m: impl Into<String>) -> Error {
    Error::Server(ErrorBody::new(c, m))
}

fn out<T: Serialize>(v: &T, human: String) -> Output {
    Output { json: serde_json::to_value(v).unwrap_or(Value::Null), human }
}

fn call<R: DeserializeOwned>(c: &mut Client, method: &str, params: impl Serialize) -> Result<R, Error> {
    c.call_cap(caps::COLLAB, method, params)
}

/// `-`: the text from stdin.
fn text_arg(s: &str) -> Result<String, Error> {
    if s != "-" {
        return Ok(s.to_string());
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).map_err(|e| err(code::BAD_PARAMS, format!("can't read stdin: {e}")))?;
    Ok(buf.trim_end().to_string())
}

fn capability(s: &str) -> Result<Capability, Error> {
    match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "read" | "read_only" => Ok(Capability::Read),
        "write" => Ok(Capability::Write),
        "write_approval" => Ok(Capability::WriteApproval),
        other => Err(err(code::BAD_PARAMS, format!("--capability {other}: expected read, write or write_approval"))),
    }
}

/// A login as an identity whose network the owner's Pitwall works out
/// (`provider:subject` names it).
fn person(login: &str) -> Identity {
    match login.split_once(':').filter(|(p, _)| !p.contains('@')) {
        Some((p, s)) => Identity::new(p, s),
        None => Identity { provider: String::new(), subject: login.trim().into(), email: login.contains('@').then(|| login.trim().into()), name: None },
    }
}

/// The flags as a patch.
pub fn patch(f: &ShareFlags) -> Result<SharePatch, Error> {
    let programs = match f.programs.as_deref() {
        None => None,
        Some(p) => {
            let (mode, list) = p.split_once(':').ok_or_else(|| err(code::BAD_PARAMS, format!("--programs {p}: expected allow:LIST or deny:LIST")))?;
            let list: Vec<String> = list.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            Some(match mode {
                "allow" => Programs::Allow(list),
                "deny" => Programs::Deny(list),
                _ => return Err(err(code::BAD_PARAMS, format!("--programs {p}: expected allow:LIST or deny:LIST"))),
            })
        }
    };
    let start = match (f.start.as_deref(), f.summary.as_deref()) {
        (Some("fresh"), _) => Some(Start::Fresh),
        (Some("summary") | None, Some(text)) => Some(Start::Summary { text: text_arg(text)?, approved_at: now() }),
        (Some("summary"), None) => return Err(err(code::BAD_PARAMS, "--start summary needs --summary TEXT (or - for stdin)")),
        (Some(other), _) => return Err(err(code::BAD_PARAMS, format!("--start {other}: expected fresh or summary"))),
        (None, None) => None,
    };
    let mut overrides = BTreeMap::new();
    for kv in &f.limit {
        let (who, n) = kv.split_once('=').ok_or_else(|| err(code::BAD_PARAMS, format!("--limit {kv}: expected LOGIN=N")))?;
        // A login: the owner's Pitwall turns it into the person id.
        let key = who.trim().to_string();
        let value = match n.trim() {
            "default" => None,
            n => Some(LimitOverride { messages_per_day: Some(n.parse().map_err(|_| err(code::BAD_PARAMS, format!("--limit {kv}: N is a number")))?), blocked: false }),
        };
        overrides.insert(key, value);
    }
    Ok(SharePatch {
        settings: None,
        slug: f.slug.clone(),
        name: f.name.clone(),
        description: f.description.clone(),
        capability: f.capability.as_deref().map(capability).transpose()?,
        add: AudienceEdit {
            people: f.add_person.iter().map(|p| person(p)).collect(),
            teams: f.add_team.clone(),
            groups: vec![],
            networks: f.add_network.iter().map(|n| NetworkId(n.clone())).collect(),
        },
        remove: AudienceEdit {
            people: f.remove_person.iter().map(|p| person(p)).collect(),
            teams: f.remove_team.clone(),
            groups: vec![],
            networks: f.remove_network.iter().map(|n| NetworkId(n.clone())).collect(),
        },
        paths: f.paths.clone(),
        exclude: f.exclude.clone(),
        include: f.include.clone(),
        uncommitted: f.uncommitted,
        start,
        env: f.env.clone(),
        hosts: f.hosts.clone(),
        account: f.account.clone(),
        programs,
        off_limits: f.instructions.as_deref().map(text_arg).transpose()?,
        messages_per_day: f.messages_per_day,
        sessions_running: f.sessions,
        queue: f.queue,
        turn_timeout_min: f.turn_timeout,
        agent_minutes_per_day: f.agent_minutes,
        overrides,
    })
}

fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // Days → civil date (UTC).
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

// ------------------------------------------------------------ commands

pub fn people_cmd(cmd: &PeopleCmd, c: &mut Client) -> Result<Output, Error> {
    match cmd {
        PeopleCmd::List => {
            let list: Vec<PersonView> = call(c, method::PEOPLE_LIST, Value::Null)?;
            Ok(out(&list, people(&list)))
        }
    }
}

pub fn team_cmd(cmd: &TeamCmd, c: &mut Client) -> Result<Output, Error> {
    Ok(match cmd {
        TeamCmd::List => {
            let list: Vec<TeamCard> = call(c, method::TEAM_LIST, Value::Null)?;
            out(&list, teams(&list))
        }
        TeamCmd::Show { team } => {
            let d: TeamDetail = call(c, method::TEAM_SHOW, TeamRef { team: team.clone() })?;
            out(&d, team_detail(&d))
        }
        TeamCmd::Join { team, note } => {
            let card: TeamCard = call(c, method::TEAM_JOIN, TeamJoin { team: team.clone(), note: note.clone().unwrap_or_default() })?;
            let human = match card.mine {
                Some(pitwall_proto::collab::TeamRole::Requested) => format!("Asked to join {}; an admin approves it.\n", card.name),
                _ => format!("Joined {} ({}).\n", card.name, card.fingerprint),
            };
            out(&card, human)
        }
        TeamCmd::Leave { team } => {
            let _: Value = call(c, method::TEAM_LEAVE, TeamRef { team: team.clone() })?;
            out(&json!({"left": team}), format!("Left {team}.\n"))
        }
        TeamCmd::AddMember { team, login, network } => {
            let added: MemberAdded = call(c, method::TEAM_ADD_MEMBER, TeamAddMember { team: team.clone(), login: login.clone(), network: network.clone() })?;
            let mut h = format!("Added {} to {}{}.\n", added.member.login, added.team.card.name, if added.member.claude_only { " (Claude only)" } else { "" });
            let _ = writeln!(h, "{}", added.note);
            h.push_str(&setup(&added.setup));
            out(&added, h)
        }
        TeamCmd::Create { name, description, listed, closed } => {
            let d: TeamDetail = call(
                c,
                method::TEAM_CREATE,
                TeamCreate { name: name.clone(), description: description.clone().unwrap_or_default(), listed: *listed, join: if *closed { JoinPolicy::Closed } else { JoinPolicy::Request } },
            )?;
            let human = format!("Created {} ({}); you are its admin.\n", d.card.name, d.card.fingerprint);
            out(&d, human)
        }
        TeamCmd::Invite { team } => {
            let link: pitwall_proto::api::collab::InviteLink = call(c, method::TEAM_INVITE, TeamRef { team: team.clone() })?;
            let human = format!("{}\nOne use, until {}.\n", link.link, link.expires_at);
            out(&link, human)
        }
        TeamCmd::Approve { team, person, decline } => {
            let d: TeamDetail = call(c, method::TEAM_APPROVE, TeamApprove { team: team.clone(), person: person.clone(), allow: !decline })?;
            let human = format!("{} {person}'s request to join {}.\n", if *decline { "Declined" } else { "Approved" }, d.card.name);
            out(&d, human)
        }
    })
}

pub fn share_cmd(cmd: &ShareCmd, c: &mut Client) -> Result<Output, Error> {
    let r = |s: &String| ShareRef { share: s.clone() };
    Ok(match cmd {
        ShareCmd::List => {
            let list: Vec<ShareView> = call(c, method::SHARE_LIST, Value::Null)?;
            out(&list, shares(&list))
        }
        ShareCmd::Show { share } => {
            let v: ShareView = call(c, method::SHARE_GET, r(share))?;
            out(&v, share_view(&v))
        }
        ShareCmd::Create { agent, flags } => {
            let v: ShareView = call(c, method::SHARE_CREATE, ShareCreate { agent: agent.clone(), patch: patch(flags)? })?;
            let mut h = share_view(&v);
            let _ = writeln!(h, "\nA draft: nobody else sees it. Try it with `pitwall share guest {} --probes`, then `pitwall share go-live {}`.", v.settings.slug, v.settings.slug);
            out(&v, h)
        }
        ShareCmd::Guest { share, probes, only, message } => {
            let message = message.as_deref().map(text_arg).transpose()?;
            let list: Vec<ProbeResult> = call(c, method::SHARE_GUEST, ShareGuest { share: share.clone(), probes: *probes, only: only.clone(), message })?;
            out(&list, guest(&list))
        }
        ShareCmd::Set { share, flags, dry_run } => {
            let changed: ShareChanged = call(c, method::SHARE_SET, ShareSet { share: share.clone(), patch: patch(flags)?, dry_run: *dry_run })?;
            out(&changed, share_changed(&changed))
        }
        ShareCmd::GoLive { share } => {
            let v: ShareView = call(c, method::SHARE_GO_LIVE, r(share))?;
            out(&v, format!("{} is live.\n", v.address))
        }
        ShareCmd::Pause { share } => {
            let v: ShareView = call(c, method::SHARE_PAUSE, r(share))?;
            out(&v, format!("{} is {}.\n", v.address, state_word(v.settings.state)))
        }
        ShareCmd::Resume { share } => {
            let v: ShareView = call(c, method::SHARE_RESUME, r(share))?;
            out(&v, format!("{} is {}.\n", v.address, state_word(v.settings.state)))
        }
        ShareCmd::Stop { share } => {
            let _: Value = call(c, method::SHARE_STOP, r(share))?;
            out(&json!({"stopped": share}), format!("Stopped sharing {share}; the agent is as it was.\n"))
        }
        ShareCmd::Sessions { share } => {
            let list: Vec<SharedSessionView> = call(c, method::SHARE_SESSIONS, r(share))?;
            out(&list, sessions(&list))
        }
        ShareCmd::Setup => {
            let lines: Vec<SetupLine> = call(c, method::SHARE_SETUP, Value::Null)?;
            let human = if lines.is_empty() {
                "Nobody can reach this machine yet: turn on Settings → Collaboration → \"Let people on <network> reach this machine\".\n".into()
            } else {
                setup(&lines)
            };
            out(&lines, human)
        }
        ShareCmd::Block { person, share, unblock } => {
            let _: Value = call(c, method::SHARE_BLOCK, ShareBlock { person: person.clone(), share: share.clone(), blocked: !unblock })?;
            let what = share.as_deref().map_or("your shared agents".to_string(), |s| s.to_string());
            let human = if *unblock { format!("{person} can reach {what} again.\n") } else { format!("Blocked {person} from {what}.\n") };
            out(&json!({"person": person, "share": share, "blocked": !unblock}), human)
        }
    })
}

pub fn ask_cmd(a: &AskArgs, c: &mut Client) -> Result<Output, Error> {
    match a.args.as_slice() {
        [list] if list == "list" => {
            let all: Vec<AskView> = call(c, method::ASK_LIST, AskFilter { owner: a.owner.clone(), share: None })?;
            Ok(out(&all, asks(&all)))
        }
        [share] => Err(err(code::BAD_PARAMS, format!("what to ask {share}: pitwall ask {share} \"…\" (or - to read stdin)"))),
        [share, message] => {
            let message = text_arg(message)?;
            let v: AskView = call(c, method::ASK_SEND, AskSend { share: share.clone(), message, new: a.new, timeout: Some(a.timeout.unwrap_or(ASK_TIMEOUT)) })?;
            if v.status == AskStatus::Failed && v.reply.is_none() {
                return Err(err(code::NO_ANSWER, v.status_message.clone().unwrap_or_else(|| format!("{} gave no answer", v.share))));
            }
            Ok(out(&v, ask_reply(&v)))
        }
        _ => Err(err(code::BAD_PARAMS, "pitwall ask OWNER/AGENT MESSAGE, or pitwall ask list")),
    }
}

// ------------------------------------------------------------ text

pub fn people(list: &[PersonView]) -> String {
    if list.is_empty() {
        return "Nobody yet: join a team (`pitwall team list`).\n".into();
    }
    let mut rows = vec![vec!["PERSON".into(), "TEAMS".into(), "DEVICES".into(), "SHARED WITH YOU".into()]];
    for p in list {
        let mut who = p.login.clone();
        if p.claude_only {
            who.push_str(" (Claude only)");
        }
        if p.blocked {
            who.push_str(" (blocked)");
        }
        let devices = if p.devices.is_empty() { "-".into() } else { p.devices.iter().map(|d| format!("{}{}", d.label, if d.online { "" } else { " (offline)" })).collect::<Vec<_>>().join(", ") };
        let shares = if p.shares.is_empty() { "-".into() } else { p.shares.iter().map(|s| format!("{}/{}", short(&s.owner), s.slug)).collect::<Vec<_>>().join(", ") };
        rows.push(vec![who, p.teams.len().to_string(), devices, shares]);
    }
    table(&rows)
}

fn short(login: &str) -> &str {
    login.split('@').next().unwrap_or(login)
}

pub fn teams(list: &[TeamCard]) -> String {
    if list.is_empty() {
        return "No teams yet: `pitwall team create NAME`, or join one with an invite link.\n".into();
    }
    let mut rows = vec![vec!["TEAM".into(), "FINGERPRINT".into(), "MEMBERS".into(), "YOU".into(), "JOIN".into()]];
    for t in list {
        let you = match t.mine {
            Some(r) => serde_json::to_value(r).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            None => "-".into(),
        };
        let join = if !t.listed {
            "invite-only".into()
        } else if t.join == JoinPolicy::Closed {
            "closed".into()
        } else {
            "by request".into()
        };
        rows.push(vec![format!("{}{}", t.name, if t.bound { " (bound)" } else { "" }), t.fingerprint.clone(), t.members.to_string(), you, join]);
    }
    table(&rows)
}

pub fn team_detail(d: &TeamDetail) -> String {
    let mut out = format!("{} ({}) — {} members, version {}\n", d.card.name, d.card.fingerprint, d.card.members, d.card.version);
    if !d.card.description.is_empty() {
        let _ = writeln!(out, "{}", d.card.description);
    }
    if let Some(by) = &d.last_change_by {
        let _ = writeln!(out, "Last change by {by}{}", d.last_change_at.as_deref().map(|a| format!(" at {a}")).unwrap_or_default());
    }
    if let Some(c) = &d.conflict {
        let _ = writeln!(out, "{c}");
    }
    let mut rows = vec![vec!["MEMBER".into(), "".into(), "DEVICES".into()]];
    for m in &d.members {
        let tag = [m.admin.then_some("★ admin"), m.claude_only.then_some("Claude only")].into_iter().flatten().collect::<Vec<_>>().join(", ");
        let devices = m.devices.iter().map(|x| format!("{}{}", x.label, if x.online { "" } else { " (offline)" })).collect::<Vec<_>>().join(", ");
        rows.push(vec![m.login.clone(), tag, if devices.is_empty() { "-".into() } else { devices }]);
    }
    out.push_str(&table(&rows));
    if !d.requests.is_empty() {
        out.push_str("Requests:\n");
        for r in &d.requests {
            let _ = writeln!(out, "  {} — {}{}", r.login, r.from, if r.note.is_empty() { String::new() } else { format!(": {}", r.note) });
        }
    }
    if !d.setup.is_empty() {
        out.push_str("For people without Pitwall:\n");
        out.push_str(&setup(&d.setup));
    }
    out
}

pub fn setup(lines: &[SetupLine]) -> String {
    lines.iter().map(|l| format!("{}\n", l.line)).collect()
}

fn audience(v: &ShareView) -> String {
    let a = &v.settings.audience;
    let n = a.people.len() + a.teams.len() + a.groups.len() + a.networks.len();
    if n == 0 {
        "nobody yet".into()
    } else {
        pitwall_proto::api::collab::audience_words(a)
    }
}

pub fn shares(list: &[ShareView]) -> String {
    if list.is_empty() {
        return "Nothing shared: `pitwall share create AGENT --team TEAM`.\n".into();
    }
    let mut rows = vec![vec!["SHARE".into(), "STATE".into(), "CAN".into(), "WHO".into(), "RUNNING".into()]];
    for v in list {
        rows.push(vec![
            v.address.clone(),
            state_word(v.settings.state).into(),
            capability_chip(v.settings.capability).into(),
            audience(v),
            format!("{}/{}", v.usage.running, v.settings.limits.sessions_running),
        ]);
    }
    table(&rows)
}

pub fn share_view(v: &ShareView) -> String {
    let mut out = format!("{} — {} ({}), {}\n", v.address, v.agent_name, v.kind_name, state_word(v.settings.state));
    let _ = writeln!(out, "{}", v.summary.text);
    if !v.sensitive.is_empty() {
        out.push_str("Sensitive files found:\n");
        for f in &v.sensitive {
            let left_out = v.settings.scope.exclude.contains(&f.path);
            let _ = writeln!(out, "  {} {} ({})", if left_out { "✕ left out:" } else { "! included:" }, f.path, f.why);
        }
    }
    out
}

pub fn share_changed(c: &ShareChanged) -> String {
    if c.diff.0.is_empty() {
        return "Nothing to change.\n".into();
    }
    let mut out: String = c.diff.0.iter().map(|l| format!("{}\n", describe(l))).collect();
    if c.dry_run {
        out.push_str(if c.diff.widens() && c.share.settings.state != pitwall_proto::collab::ShareState::Draft {
            "Not changed (--dry-run). Applying it asks you in Pitwall (▲ widens).\n"
        } else {
            "Not changed (--dry-run).\n"
        });
    } else if c.approved {
        out.push_str("Approved in Pitwall and applied.\n");
    } else {
        out.push_str("Applied.\n");
    }
    out
}

pub fn guest(list: &[ProbeResult]) -> String {
    let mut out = String::new();
    for p in list {
        let _ = writeln!(out, "» {}", p.message);
        let _ = writeln!(out, "  {}", p.reply.as_deref().unwrap_or("(no reply)").replace('\n', "\n  "));
        for t in &p.tried {
            let _ = writeln!(out, "  tried: {t}");
        }
    }
    out
}

pub fn sessions(list: &[SharedSessionView]) -> String {
    if list.is_empty() {
        return "No conversations yet.\n".into();
    }
    let word = |v: Value| v.as_str().unwrap_or_default().to_string();
    let mut rows = vec![vec!["CALLER".into(), "STATE".into(), "TURNS".into(), "TODAY".into(), "MINUTES".into(), "HELD".into(), "LAST".into()]];
    for s in list {
        let mut who = if s.caller.guest { "guest".to_string() } else { s.caller.login.clone() };
        if let Some(a) = &s.caller.agent {
            let _ = write!(who, " · {a}");
        } else if let Some(cl) = &s.caller.client {
            let _ = write!(who, " · {cl} ({})", word(serde_json::to_value(s.caller.front).unwrap_or_default()).to_uppercase());
        }
        rows.push(vec![
            who,
            word(serde_json::to_value(s.state).unwrap_or_default()),
            s.turns.to_string(),
            s.usage.messages_today.to_string(),
            s.usage.minutes_today.to_string(),
            s.held.to_string(),
            s.last_at.clone().unwrap_or_else(|| "-".into()),
        ]);
    }
    table(&rows)
}

fn status_word(s: AskStatus) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(|x| x.replace('_', " "))).unwrap_or_default()
}

pub fn ask_reply(v: &AskView) -> String {
    let mut out = String::new();
    if let Some(r) = &v.reply {
        let _ = writeln!(out, "{r}");
    }
    let status = v.status_message.clone().unwrap_or_else(|| status_word(v.status));
    let _ = writeln!(out, "— {} · {} · turn {}", v.share, status, v.turn);
    if let Some(ch) = &v.changes {
        let _ = writeln!(out, "  changes: {} files, +{} −{} ({})", ch.files, ch.added, ch.removed, status_word_changes(ch.state));
    }
    out
}

fn status_word_changes(s: pitwall_proto::collab::ChangesState) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

pub fn asks(list: &[AskView]) -> String {
    if list.is_empty() {
        return "No messages yet.\n".into();
    }
    let mut rows = vec![vec!["TURN".into(), "AGENT".into(), "STATUS".into(), "MESSAGE".into()]];
    for a in list {
        let text: String = a.text.lines().next().unwrap_or_default().chars().take(60).collect();
        rows.push(vec![a.turn.clone(), a.share.clone(), a.status_message.clone().unwrap_or_else(|| status_word(a.status)), text]);
    }
    table(&rows)
}
