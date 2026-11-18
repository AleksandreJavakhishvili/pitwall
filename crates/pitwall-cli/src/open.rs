//! `pitwall open` and the MCP tools' one code path: a [`ViewerOpen`]
//! request to the running Pitwall (`viewer.open`), and its answer in words.

use std::path::Path;

use pitwall_client::{Client, Error};
use pitwall_proto::{caps, method, ViewerOpen, ViewerOpened, ViewerPlace};

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
    }
}

pub fn open(c: &mut Client, req: &ViewerOpen) -> Result<ViewerOpened, Error> {
    c.call_cap(caps::VIEWER, method::VIEWER_OPEN, req)
}

/// What was opened, in words (the CLI's `--human` and the MCP tools'
/// result).
pub fn text(o: &ViewerOpened) -> String {
    let mut parts = vec![];
    for f in &o.opened {
        parts.push(match (f.line, f.column) {
            (Some(l), Some(c)) => format!("{} at line {l}, column {c}", f.path),
            (Some(l), None) => format!("{} at line {l}", f.path),
            _ => f.path.clone(),
        });
    }
    if let Some(d) = &o.folder {
        parts.push(format!("the folder {d} (as a tree)"));
    }
    let mut s = format!("Opened in {} in Pitwall: {}.\n", o.shown_in, parts.join(", "));
    if let Some(n) = &o.note {
        s.push_str(n);
        s.push('\n');
    }
    if o.in_background {
        s.push_str(match o.place {
            ViewerPlace::Beside => "Pitwall isn't in front, so it opened there without coming forward; the user sees it when they switch to Pitwall.\n",
            ViewerPlace::Window => "The user is in another app: the window opened behind it without taking the keyboard.\n",
        });
    }
    if !o.skipped.is_empty() {
        let list: Vec<String> = o.skipped.iter().map(|k| format!("{} ({})", k.path, k.reason)).collect();
        s.push_str(&format!("Not opened: {}.\n", list.join(", ")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::{OpenedFile, Skipped};

    #[test]
    fn says_what_opened_and_what_didnt() {
        let o = ViewerOpened {
            agent_id: "a1".into(),
            agent_name: "api".into(),
            window: "main".into(),
            in_background: true,
            opened: vec![OpenedFile { path: "/w/a.rs".into(), line: Some(4), column: None }],
            folder: None,
            skipped: vec![Skipped { path: "b.rs".into(), reason: "not found".into() }],
            place: ViewerPlace::Window,
            asked: Some(ViewerPlace::Beside),
            shown_in: "a file viewer window of its own for api, showing /w".into(),
            note: Some("api isn't shown in any Pitwall window, so it opened in a window of its own.".into()),
        };
        let t = text(&o);
        assert!(t.starts_with("Opened in a file viewer window of its own for api, showing /w in Pitwall: /w/a.rs at line 4.\n"), "{t}");
        assert!(t.contains("so it opened in a window of its own.\n"), "{t}");
        assert!(t.contains("behind it without taking the keyboard"), "{t}");
        assert!(t.ends_with("Not opened: b.rs (not found).\n"));
    }
}
