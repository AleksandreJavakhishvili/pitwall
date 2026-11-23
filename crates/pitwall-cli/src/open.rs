//! `pitwall open` and the MCP tools' fallback path: a [`ViewerOpen`]
//! request to the running Pitwall (`viewer.open`), and its answer in words.
//!
//! The answer is read as it came and never turns into an error: a Pitwall
//! newer than this CLI may say things it doesn't know (its own words,
//! `text`, are used then). An older Pitwall that refuses what it doesn't
//! know is said to be older, plainly.

use std::path::Path;

use serde_json::Value;

use pitwall_client::{Client, Error};
use pitwall_proto::{caps, code, method, ErrorBody, SettingsOpen, ViewerOpen, ViewerPlace, WebOpen, SETTINGS_PROTO, VIEWER_PROTO, WEB_PROTO};

use crate::args::OpenArgs;

/// The agent this runs in (`$PITWALL_AGENT_ID`), if any; the server also
/// knows it from the calling process.
pub fn env_agent() -> Option<String> {
    std::env::var(pitwall_proto::AGENT_ENV).ok().filter(|s| !s.trim().is_empty())
}

/// `pitwall open`'s request: relative paths are this shell's (`cwd`).
pub fn request(a: &OpenArgs, cwd: &Path) -> ViewerOpen {
    ViewerOpen {
        agent_id: a.agent.clone().or_else(env_agent),
        files: a.files.clone(),
        folder: a.folder.clone(),
        root: a.root.clone(),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        place: a.place.as_deref().and_then(ViewerPlace::parse),
        of: a.of.clone(),
        proto: Some(VIEWER_PROTO),
    }
}

/// `pitwall open --web`'s request (`None`: not a web page).
pub fn web_request(a: &OpenArgs) -> Option<WebOpen> {
    let url = a.web.as_deref()?;
    Some(WebOpen {
        agent_id: a.agent.clone().or_else(env_agent),
        url: Some(url.trim().to_string()).filter(|u| !u.is_empty()),
        place: a.place.as_deref().and_then(ViewerPlace::parse),
        of: a.of.clone(),
        proto: Some(WEB_PROTO),
    })
}

/// `pitwall open --settings`'s request (`None`: not Settings).
pub fn settings_request(a: &OpenArgs) -> Option<SettingsOpen> {
    let page = a.settings.as_deref()?;
    Some(SettingsOpen {
        agent_id: a.agent.clone().or_else(env_agent),
        page: Some(page.trim().to_string()).filter(|p| !p.is_empty()),
        extension: a.extension.clone().filter(|e| !e.trim().is_empty()),
        secret: None,
        connection: None,
        proto: Some(SETTINGS_PROTO),
    })
}

/// What a Pitwall from before `settings.open` is told it can't do.
pub const NO_SETTINGS: &str = "This Pitwall is older than its command-line tool and can't show its Settings to an agent's user: quit Pitwall and open it again to update it. Nothing was opened.";

/// `settings.open`: Pitwall's answer as it came, and in words.
pub fn open_settings(c: &mut Client, req: &SettingsOpen) -> Result<(Value, String), Error> {
    if !c.has(caps::SHOW_SETTINGS) {
        return Err(Error::Server(ErrorBody::new(code::UNSUPPORTED, NO_SETTINGS)));
    }
    let v = c.call(method::SETTINGS_OPEN, req)?;
    let text = pitwall_proto::settings_open::opened_text(&v);
    Ok((v, text))
}

/// What a Pitwall from before `web.open` is told it can't do.
pub const NO_WEB: &str = "This Pitwall is older than its command-line tool and can't show web pages: quit Pitwall and open it again to update it. Nothing was opened.";

/// `web.open`: Pitwall's answer as it came, and in words.
pub fn open_web(c: &mut Client, req: &WebOpen) -> Result<(Value, String), Error> {
    if !c.has(caps::WEB) {
        return Err(Error::Server(ErrorBody::new(code::UNSUPPORTED, NO_WEB)));
    }
    let v = c.call(method::WEB_OPEN, req)?;
    let text = pitwall_proto::web::opened_text(&v);
    Ok((v, text))
}

/// `viewer.open`: Pitwall's answer as it came, and in words.
pub fn open(c: &mut Client, req: &ViewerOpen) -> Result<(Value, String), Error> {
    if !c.has(caps::VIEWER) {
        return Err(Error::Server(ErrorBody::new(code::UNSUPPORTED, format!("this Pitwall ({}) doesn't offer {}: update Pitwall", c.welcome().daemon, method::VIEWER_OPEN))));
    }
    // A Pitwall from before `mcp.*` is older than this CLI.
    let older = !c.has(caps::MCP);
    match c.call(method::VIEWER_OPEN, req) {
        Ok(v) => {
            let text = pitwall_proto::mcp::opened_text(&v);
            Ok((v, text))
        }
        Err(e) => Err(if older { said_older(e) } else { e }),
    }
}

/// A refusal from a Pitwall older than this CLI (it didn't know a word it
/// was sent), said as that rather than as its parser's message.
pub fn said_older(e: Error) -> Error {
    match e {
        Error::Server(b) if b.code == code::BAD_PARAMS && b.message.contains("unknown variant") => Error::Server(ErrorBody::new(
            code::BAD_PARAMS,
            format!("This Pitwall is older than its command-line tool and didn't understand the request ({}): quit Pitwall and open it again to update it. Nothing was opened.", b.message),
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::{OpenedFile, Skipped, ViewerOpened};

    #[test]
    fn says_what_opened_and_what_didnt() {
        let o = ViewerOpened {
            agent_id: "a1".into(),
            agent_name: "api".into(),
            window: "main".into(),
            in_background: true,
            opened: vec![OpenedFile { path: "/w/a.rs".into(), line: Some(4), column: None }],
            skipped: vec![Skipped { path: "b.rs".into(), reason: "not found".into() }],
            place: "window".into(),
            asked: Some("right".into()),
            shown_in: "a file viewer window of its own for api, showing /w".into(),
            note: Some("api isn't shown in any Pitwall window, so it opened in a window of its own.".into()),
            ..Default::default()
        };
        let t = pitwall_proto::mcp::text(&o);
        assert!(t.starts_with("Opened in a file viewer window of its own for api, showing /w in Pitwall: /w/a.rs at line 4.\n"), "{t}");
        assert!(t.contains("so it opened in a window of its own.\n"), "{t}");
        assert!(t.contains("behind it without taking the keyboard"), "{t}");
        assert!(t.ends_with("Not opened: b.rs (not found).\n"));
    }

    #[test]
    fn an_older_pitwall_refusing_a_word_is_said_to_be_older() {
        // What a Pitwall from before `right` answered (serde's words).
        let e = said_older(Error::Server(ErrorBody::new(code::BAD_PARAMS, "unknown variant `right`, expected `beside` or `window`")));
        let m = e.to_string();
        assert!(m.starts_with("This Pitwall is older than its command-line tool"), "{m}");
        assert!(m.contains("quit Pitwall and open it again") && m.contains("Nothing was opened"), "{m}");
        // Anything else is left as it is.
        assert_eq!(said_older(Error::Server(ErrorBody::new(code::NOT_FOUND, "no agent \"x\""))).to_string(), "no agent \"x\"");
    }
}
