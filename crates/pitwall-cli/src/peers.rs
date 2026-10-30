//! `pitwall peers`: the user's other machines' Pitwalls over Tailscale
//! (docs/spec/tailnet-peers.md), read-only. It works on Pitwall's data
//! folder directly (`peers/`), like `settings` without the app: a running
//! app sees the files change and applies them at once. Changing pairings or switches is
//! refused inside an agent (`PITWALL_AGENT_ID` set): that is the user's
//! call.

use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;

use serde_json::json;

use pitwall_client::Error;
use pitwall_peer::{Candidate, Paired, PeerEntry, PeerEvent, PeerSettings, PeerSink, Peers, PeersConfig, Probe, TailnetInfo, TailscaleCli};
use pitwall_proto::peer::LinkState;
use pitwall_proto::{code, ErrorBody};

use crate::args::PeersCmd;
use crate::render::table;
use crate::{out, Output};

fn fail(e: ErrorBody) -> Error {
    Error::Server(e)
}

fn open(root: &Path, sink: Arc<dyn PeerSink>) -> Result<Arc<Peers>, Error> {
    Peers::new(PeersConfig {
        root: root.to_path_buf(),
        tailnet: Arc::new(TailscaleCli::new()),
        version: env!("CARGO_PKG_VERSION").into(),
        sink,
        feed: None,
        timing: Default::default(),
        limits: Default::default(),
    })
    .map_err(fail)
}

/// Whether this runs inside one of Pitwall's agents or terminals.
pub fn in_an_agent() -> bool {
    std::env::var_os(pitwall_proto::AGENT_ENV).is_some_and(|v| !v.is_empty())
}

/// Run `cmd` on the data folder `root`; `agent`: called inside an agent,
/// where pairings and switches can't be changed (they are the user's).
pub fn run(cmd: &PeersCmd, root: &Path, agent: bool) -> Result<Output, Error> {
    if agent && matches!(cmd, PeersCmd::Pair { .. } | PeersCmd::Unpair { .. } | PeersCmd::Revoke { .. } | PeersCmd::Set { .. }) {
        return Err(fail(ErrorBody::new(code::DENIED, "run this from your own terminal, not inside an agent")));
    }
    let peers = open(root, Arc::new(pitwall_peer::NullPeerSink))?;
    Ok(match cmd {
        PeersCmd::List => {
            let (info, list, viewers) = (peers.info(), peers.peers(), peers.viewers());
            let human = format!("{}{}{}", tailnet(&info), shown(&list), viewing(&viewers));
            out(&json!({ "tailnet": info, "peers": list, "viewers": viewers }), human)
        }
        PeersCmd::Discover => {
            let found = peers.discover().map_err(fail)?;
            out(&found, candidates(&found))
        }
        PeersCmd::Pair { name } => {
            let e = peers.pair(name).map_err(fail)?;
            let human = format!("Paired with {} ({}). Its agents show here while Pitwall runs.\n", e.paired.label, e.paired.login);
            out(&e, human)
        }
        PeersCmd::Unpair { name } => {
            let p = peers.unpair(name).map_err(fail)?;
            out(&p, format!("Forgot {}.\n", p.label))
        }
        PeersCmd::Revoke { name } => {
            let p = peers.revoke(name).map_err(fail)?;
            out(&p, format!("{} can no longer see this Pitwall's agents.\n", p.label))
        }
        PeersCmd::Serve => {
            let (info, viewers) = (peers.info(), peers.viewers());
            let human = format!("{}{}", tailnet(&info), viewing(&viewers));
            out(&json!({ "tailnet": info, "viewers": viewers }), human)
        }
        PeersCmd::Set { share, show, port } => {
            let mut s = peers.settings();
            s.share = share.unwrap_or(s.share);
            s.show = show.unwrap_or(s.show);
            s.port = port.unwrap_or(s.port);
            let s = peers.set_settings(s).map_err(fail)?;
            out(&s, switches(&s))
        }
        PeersCmd::Watch { .. } => unreachable!("streamed by main (peers::watch)"),
    })
}

fn on(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn switches(s: &PeerSettings) -> String {
    format!("Show agents from my other machines: {}\nLet my other machines see this Pitwall's agents: {} (port {})\n", on(s.show), on(s.share), s.port)
}

fn tailnet(i: &TailnetInfo) -> String {
    let state = serde_json::to_value(i.state).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    let mut s = match (&i.machine, &i.login) {
        (Some(m), Some(l)) => format!("Tailscale: {state}, this machine is {m} ({l})"),
        _ => format!("Tailscale: {state}"),
    };
    if !i.addresses.is_empty() {
        s += &format!(", {}", i.addresses.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
    }
    if let Some(m) = &i.message {
        s += &format!(" — {m}");
    }
    format!("{s}\n{}", switches(&i.settings))
}

fn link(l: &Option<pitwall_proto::peer::LinkView>) -> String {
    match l.as_ref().map(|l| l.state) {
        None => "-".into(),
        Some(LinkState::Connected) => "connected".into(),
        Some(LinkState::Connecting) => "connecting".into(),
        Some(LinkState::Offline) => "offline".into(),
        Some(LinkState::IdentityChanged) => "identity changed: pair again".into(),
    }
}

fn shown(list: &[PeerEntry]) -> String {
    if list.is_empty() {
        return "\nNo machines paired (see `pitwall peers discover`).\n".into();
    }
    let mut rows = vec![vec!["ID".into(), "MACHINE".into(), "LOGIN".into(), "LINK".into()]];
    rows.extend(list.iter().map(|e| vec![e.paired.id.clone(), e.paired.label.clone(), e.paired.login.clone(), link(&e.link)]));
    format!("\nShown here:\n{}", table(&rows))
}

fn viewing(list: &[Paired]) -> String {
    if list.is_empty() {
        return "\nNo machines can see this Pitwall.\n".into();
    }
    let mut rows = vec![vec!["MACHINE".into(), "LOGIN".into(), "NODE".into()]];
    rows.extend(list.iter().map(|v| vec![v.label.clone(), v.login.clone(), v.stable_id.clone()]));
    format!("\nMay see this Pitwall:\n{}", table(&rows))
}

fn candidates(list: &[Candidate]) -> String {
    if list.is_empty() {
        return "None of your other machines are on the tailnet.\n".into();
    }
    let mut rows = vec![vec!["MACHINE".into(), "OS".into(), "PITWALL".into(), "PAIRED".into()]];
    rows.extend(list.iter().map(|c| {
        let probe = match &c.probe {
            Probe::Pitwall { version, agents, .. } => format!("{version}, {agents} agents"),
            Probe::NoAnswer { .. } => "no answer".into(),
            Probe::Incompatible { .. } => "incompatible".into(),
            Probe::NotProbed => "offline".into(),
        };
        vec![c.label.clone(), c.os.clone(), probe, c.paired.clone().unwrap_or_else(|| "-".into())]
    }));
    table(&rows)
}

struct Channel(mpsc::Sender<PeerEvent>);

impl PeerSink for Channel {
    fn emit(&self, ev: PeerEvent) {
        let _ = self.0.send(ev);
    }
}

/// `peers watch`: link to `name` and print what changes, until stopped or
/// `count` lines.
pub fn watch(name: &str, count: Option<usize>, root: &Path, human: bool, w: &mut impl Write) -> Result<(), Error> {
    let (tx, rx) = mpsc::channel();
    let peers = open(root, Arc::new(Channel(tx)))?;
    if !peers.settings().show {
        return Err(fail(ErrorBody::new(code::UNSUPPORTED, "turn on \"Show agents from my other machines\" first (`pitwall peers set --show on`)")));
    }
    let id = peers.peers().into_iter().find(|e| e.paired.id == name || e.paired.label == name).map(|e| e.paired.id).ok_or_else(|| {
        fail(ErrorBody::new(code::NOT_FOUND, format!("no paired machine called {name}")))
    })?;
    peers.start();
    let mut n = 0;
    while count.is_none_or(|max| n < max) {
        let Ok(ev) = rx.recv() else { break };
        let (event, data, text) = match ev {
            PeerEvent::LinkChanged { peer, link } if peer == id => {
                let text = format!("{peer}: {}", self::link(&Some(link.clone())));
                ("link", json!(link), text)
            }
            PeerEvent::AgentsChanged { peer } if peer == id => {
                let views = peers.mirror().views_of(&id);
                let text = format!("{peer}: {}", views.iter().map(|a| format!("{} {}", a.name, crate::render::status(a))).collect::<Vec<_>>().join(" · "));
                ("agents.changed", json!(views), text)
            }
            PeerEvent::Attention { peer, label, attention } if peer == id => {
                let text = format!("{} on {label} is {}", attention.name, attention.reason);
                ("attention", json!(attention), text)
            }
            PeerEvent::BlockedChanged { peer, count } if peer == id => ("blocked.changed", json!({ "count": count }), format!("{peer}: {count} blocked")),
            _ => continue,
        };
        let line = if human { text } else { json!({ "event": event, "data": data }).to_string() };
        if writeln!(w, "{line}").and_then(|_| w.flush()).is_err() {
            break;
        }
        n += 1;
    }
    peers.stop();
    Ok(())
}
