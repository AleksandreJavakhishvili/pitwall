//! `pitwall events`: subscribe and print each event as it arrives, one JSON
//! line (`{"event":…,"data":…}`) or one line of text each.

use std::io::Write;
use std::path::Path;

use pitwall_client::{Client, Error};
use pitwall_proto::{event, AgentView, AttentionView, BlockedCount, Event};

use crate::args::EventsArgs;
use crate::render;

/// Follow Pitwall at `socket` until it closes, `--count` events came, or
/// `out` stops taking them (`| head`).
pub fn follow(a: &EventsArgs, socket: &Path, human: bool, out: &mut impl Write) -> Result<(), Error> {
    let mut c = Client::connect_as(socket, &format!("pitwall-cli/{}", env!("CARGO_PKG_VERSION")), pitwall_proto::Role::Cli)?;
    c.subscribe(&a.topics)?;
    let mut n = 0;
    while a.count.is_none_or(|max| n < max) {
        let Some(e) = c.next_event()? else { break };
        let line = if human { text(&e) } else { serde_json::to_string(&e).unwrap_or_default() };
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            break;
        }
        n += 1;
    }
    Ok(())
}

/// One event as a line of text.
pub fn text(e: &Event) -> String {
    match e.event.as_str() {
        event::AGENTS_CHANGED => match serde_json::from_value::<Vec<AgentView>>(e.data.clone()) {
            Ok(list) if list.is_empty() => "agents: none".into(),
            Ok(list) => format!("agents: {}", list.iter().map(|a| format!("{} {}", a.name, render::status(a))).collect::<Vec<_>>().join(" · ")),
            Err(_) => format!("agents: {}", e.data),
        },
        event::ATTENTION => match serde_json::from_value::<AttentionView>(e.data.clone()) {
            Ok(a) => match a.detail {
                Some(d) => format!("{} is {}: {d}", a.name, a.reason),
                None => format!("{} is {}", a.name, a.reason),
            },
            Err(_) => format!("attention: {}", e.data),
        },
        event::BLOCKED_CHANGED => match serde_json::from_value::<BlockedCount>(e.data.clone()) {
            Ok(b) => format!("blocked: {}", b.count),
            Err(_) => format!("blocked: {}", e.data),
        },
        other => format!("{other}: {}", e.data),
    }
}
