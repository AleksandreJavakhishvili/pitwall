//! `pitwall extension new` (docs/extensions/starter.md): a folder an
//! extension's author starts from: a manifest, a minimal Node adapter that
//! speaks protocol 1 and shows one inbox thread, a README and the
//! `test/pitwall.json` its checks run. With protocol 2 the manifest is
//! format 2, asks for `agents.read`, and the adapter reads the agents and
//! shows them in its thread. The view starter (`--view`) draws a list view
//! tile with a detail, answers a palette command and puts a note on the
//! agents' rows. The service starter
//! (`--service`) adds a tiny local server (`server.mjs`) that Pitwall
//! keeps running while the extension is on. Each passes
//! [`super::check::check`] and [`super::check::test`] as written.

use std::path::{Path, PathBuf};

use super::{check_id, MANIFEST, MAX_NAME};

const ADAPTER: &str = include_str!("starter/adapter.mjs");
const ADAPTER_2: &str = include_str!("starter/adapter2.mjs");
const ADAPTER_VIEW: &str = include_str!("starter/adapter_view.mjs");

/// Which starter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// An inbox of its own (protocol 1), or the agents in it (protocol 2).
    Inbox,
    /// A view tile, a palette command and notes on agents' rows.
    View,
    /// An inbox (protocol 1) and a small local server Pitwall keeps
    /// running while it is on (a service).
    Service,
}

const SERVER: &str = include_str!("starter/server.mjs");

const MANIFEST_SERVICE: &str = r#"{
  "v": 2,
  "id": "__ID__",
  "name": __NAME_JSON__,
  "version": "0.1.0",
  "pitwall": ">=__PITWALL__",
  "icon": "inbox",
  "description": "An inbox, and a small server on this computer that Pitwall keeps running: a starting point",
  "settings": [
    {
      "key": "listen",
      "type": "text",
      "label": "Its server listens on",
      "default": "127.0.0.1:8790",
      "hint": "An address and port on this computer only."
    }
  ],
  "contributes": { "inbox": true },
  "adapter": { "command": ["node", "adapter.mjs"] },
  "service": {
    "label": "Its server",
    "run": ["node", "server.mjs", "--listen", "{setting:listen}", "--data", "{data}"],
    "version": { "run": ["node", "server.mjs", "--version"], "match": "server (\\d+\\.\\d+\\.\\d+)", "requires": ">=0.1.0" },
    "ready": { "listen": "{setting:listen}" }
  }
}
"#;

const FIXTURE_SERVICE: &str = r#"{
  "about": "What `pitwall extension test` checks: the adapter answers hello, says it is ready, lists its thread, opens it and takes a reply; the service's commands are the ones expected, and what Pitwall does with it when Node is installed or not, when its address is already taken, and with its version. Nothing here starts the server.",
  "run": {
    "settings": {},
    "secrets": {},
    "threads": 1,
    "open": "welcome",
    "reply": "Hello back"
  },
  "service": {
    "values": {},
    "expect": {
      "run": "node server.mjs --listen 127.0.0.1:8790 --data {data}",
      "version": "node server.mjs --version"
    },
    "cases": [
      { "installed": true, "version": "My server server 0.1.0", "verdict": "starts (Its server 0.1.0)" },
      { "installed": true, "portTaken": true, "verdict": "something else is already listening on 127.0.0.1:8790: that one is used, nothing is started" },
      { "installed": false, "verdict": "“node” isn't installed" },
      { "installed": true, "values": { "listen": "0.0.0.0:8790" }, "verdict": "isn't started: “0.0.0.0:8790” isn't on this computer only: use 127.0.0.1 and a port" }
    ]
  }
}
"#;

const README: &str = include_str!("starter/README.md");

const MANIFEST_VIEW: &str = r#"{
  "v": 2,
  "id": "__ID__",
  "name": __NAME_JSON__,
  "version": "0.1.0",
  "pitwall": ">=__PITWALL__",
  "icon": "list",
  "description": "A view tile, a palette command and notes on agents' rows: a starting point",
  "permissions": {
    "agents.read": "To put a short note on your agents' rows in the sidebar"
  },
  "contributes": {
    "views": [{ "id": "items", "title": __NAME_JSON__, "icon": "list" }],
    "commands": [{ "id": "refresh", "title": "Refresh" }],
    "badges": true
  },
  "adapter": { "command": ["node", "adapter.mjs"] }
}
"#;

const FIXTURE_VIEW: &str = r#"{
  "about": "What `pitwall extension test` checks: against a fake Pitwall that answers agent.list, the adapter draws its list view when shown, a detail beside it when an item is selected, answers an item's action and the palette command, and puts a note on the agent's row.",
  "views": {
    "grants": ["agents.read"],
    "answers": {
      "agent.list": [{ "id": "a1", "name": "fix-login", "kindName": "Claude Code", "status": "working" }]
    },
    "show": "items",
    "kind": "list",
    "items": 2,
    "select": "welcome",
    "detail": "detail",
    "action": { "item": "welcome", "action": "done" },
    "command": "refresh",
    "badges": 1
  }
}
"#;

const MANIFEST_2: &str = r#"{
  "v": 2,
  "id": "__ID__",
  "name": __NAME_JSON__,
  "version": "0.1.0",
  "pitwall": ">=__PITWALL__",
  "icon": "inbox",
  "description": "Shows your agents in its inbox: a starting point",
  "permissions": {
    "agents.read": "To show your agents and their statuses in its inbox"
  },
  "contributes": { "inbox": true },
  "adapter": { "command": ["node", "adapter.mjs"] }
}
"#;

const FIXTURE_2: &str = r#"{
  "about": "What `pitwall extension test` checks: against a fake Pitwall that answers agent.list, the adapter answers hello with protocol 2, reads the agents, subscribes to their changes and shows them in its thread; after an agents.changed event it shows the new list.",
  "calls": {
    "grants": ["agents.read"],
    "answers": {
      "agent.list": [{ "id": "a1", "name": "fix-login", "kindName": "Claude Code", "status": "working" }]
    },
    "events": [
      { "event": "agents.changed", "data": [{ "id": "a2", "name": "docs-pass", "kindName": "Codex", "status": "idle" }] }
    ],
    "expect": {
      "called": ["agent.list", "events.subscribe"],
      "subscribes": ["agents"],
      "threads": 1,
      "open": "agents",
      "text": "fix-login",
      "afterEvents": "docs-pass"
    }
  }
}
"#;

const MANIFEST_TEXT: &str = r#"{
  "v": 1,
  "id": "__ID__",
  "name": __NAME_JSON__,
  "version": "0.1.0",
  "icon": "inbox",
  "description": "Shows one thread in its inbox: a starting point",
  "adapter": { "command": ["node", "adapter.mjs"] }
}
"#;

const FIXTURE_TEXT: &str = r#"{
  "about": "What `pitwall extension test` checks: the adapter answers hello, says it is ready, lists its thread, opens it and takes a reply.",
  "run": {
    "settings": {},
    "secrets": {},
    "threads": 1,
    "open": "welcome",
    "reply": "Hello back"
  }
}
"#;

/// The name an id reads as: its words, the first capitalised
/// (`my-inbox`: "My inbox").
pub fn name_of(id: &str) -> String {
    let words = id.replace('-', " ");
    let mut c = words.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Write the starter into `folder` (made; it must be absent or empty),
/// its id the folder's name, its name `name` (else read from the id): the
/// files written. `protocol`: 1 (an inbox of its own) or 2 (reads the
/// agents); a view `shape` is protocol 2.
pub fn write(folder: &Path, name: Option<&str>, protocol: u64, shape: Shape) -> Result<Vec<PathBuf>, String> {
    if !super::protocol::PROTOCOLS.contains(&protocol) {
        return Err(format!("there is a starter for protocol 1 and 2, not {protocol}"));
    }
    let protocol = match shape {
        Shape::Inbox => protocol,
        Shape::Service => 1,
        Shape::View => super::protocol::PROTOCOL_2,
    };
    let id = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    check_id(&id).map_err(|e| format!("the folder's name is the extension's id, and {e}: name the folder like my-extension"))?;
    let name = match name {
        Some(n) => n.trim().to_string(),
        None => name_of(&id),
    };
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("the name must be one line of 1–{MAX_NAME} characters"));
    }
    if folder.exists() {
        let empty = std::fs::read_dir(folder).map_err(|e| format!("{} can't be read: {e}", folder.display()))?.next().is_none();
        if !empty {
            return Err(format!("{} isn't empty: pick a new folder", folder.display()));
        }
    }
    let name_json = serde_json::to_string(&name).unwrap_or_default();
    let fill = |t: &str| t.replace("__ID__", &id).replace("__NAME_JSON__", &name_json).replace("__NAME__", &name).replace("__PITWALL__", super::PITWALL).replace("__PROTOCOL__", &protocol.to_string());
    let two = protocol == 2;
    let (manifest, adapter, fixture) = match (shape, two) {
        (Shape::View, _) => (MANIFEST_VIEW, ADAPTER_VIEW, FIXTURE_VIEW),
        (Shape::Service, _) => (MANIFEST_SERVICE, ADAPTER, FIXTURE_SERVICE),
        (Shape::Inbox, true) => (MANIFEST_2, ADAPTER_2, FIXTURE_2),
        (Shape::Inbox, false) => (MANIFEST_TEXT, ADAPTER, FIXTURE_TEXT),
    };
    let mut files: Vec<(&str, String)> = vec![(MANIFEST, fill(manifest)), ("adapter.mjs", fill(adapter)), ("README.md", fill(README)), (super::check::FIXTURE, fixture.to_string())];
    if shape == Shape::Service {
        files.push(("server.mjs", fill(SERVER)));
    }
    let mut written = Vec::new();
    for (rel, text) in files {
        let p = folder.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{} can't be made: {e}", parent.display()))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("{} can't be written: {e}", p.display()))?;
        written.push(p);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::check::{check, test, Level};

    #[test]
    fn the_starter_passes_its_checks() {
        let t = crate::testing::TempDir::new("starter");
        let dir = t.path().join("my-inbox");
        let files = write(&dir, None, 1, Shape::Inbox).unwrap();
        assert_eq!(files.len(), 4);
        let m = crate::extensions::read(&dir, "my-inbox").unwrap();
        assert_eq!((m.name.as_str(), m.version.as_deref()), ("My inbox", Some("0.1.0")));
        let r = check(&dir, None);
        assert!(r.passed(), "{}", r.text());
        assert!(!r.findings.iter().any(|f| f.level == Level::Fail));
        let r = test(&dir, None);
        assert!(r.passed(), "{}", r.text());
        // Without Node the run is skipped; with it, each step is checked.
        if crate::extensions::plan::resolve(&dir, "node", None).is_ok() {
            assert!(r.text().contains("a reply is answered ok"), "{}", r.text());
        }
        // A name with quotes stays valid JSON and JavaScript.
        let other = t.path().join("quoted");
        write(&other, Some("Say \"hi\""), 1, Shape::Inbox).unwrap();
        assert_eq!(crate::extensions::read(&other, "quoted").unwrap().name, "Say \"hi\"");
        assert!(std::fs::read_to_string(other.join("adapter.mjs")).unwrap().contains(r#"const NAME = "Say \"hi\"";"#));
    }

    /// The protocol 2 starter: format 2, asks for agents.read, and its
    /// adapter reads the agents (from the fake Pitwall side) into its
    /// thread, and the new list after an event.
    #[test]
    fn the_protocol_2_starter_passes_its_checks() {
        let t = crate::testing::TempDir::new("starter2");
        let dir = t.path().join("my-agents");
        write(&dir, None, 2, Shape::Inbox).unwrap();
        let m = crate::extensions::read(&dir, "my-agents").unwrap();
        assert_eq!((m.format, m.permissions[0].name.as_str(), m.has_inbox()), (2, "agents.read", true));
        let r = check(&dir, None);
        assert!(r.passed(), "{}", r.text());
        assert!(r.text().contains("agents.read: To show your agents"), "{}", r.text());
        let r = test(&dir, None);
        assert!(r.passed(), "{}", r.text());
        if crate::extensions::plan::resolve(&dir, "node", None).is_ok() {
            let text = r.text();
            for want in ["protocol 2", "calls: it calls agent.list", "calls: it subscribes to agents", "shows what it read (“fix-login”)", "shows “docs-pass”"] {
                assert!(text.contains(want), "{want}: {text}");
            }
        }
    }

    /// The view starter: a list view with a detail, a command and a note
    /// on the agent's row, each checked against a fake Pitwall side.
    #[test]
    fn the_view_starter_passes_its_checks() {
        let t = crate::testing::TempDir::new("starter-view");
        let dir = t.path().join("my-view");
        write(&dir, Some("My view"), 1, Shape::View).unwrap();
        let m = crate::extensions::read(&dir, "my-view").unwrap();
        assert_eq!((m.format, m.contributes.views[0].id.as_str(), m.contributes.commands[0].id.as_str(), m.contributes.badges, m.has_inbox()), (2, "items", "refresh", true, false));
        let r = check(&dir, None);
        assert!(r.passed(), "{}", r.text());
        assert!(r.text().contains("a view tile “My view” (items)"), "{}", r.text());
        let r = test(&dir, None);
        assert!(r.passed(), "{}", r.text());
        if crate::extensions::plan::resolve(&dir, "node", None).is_ok() {
            let text = r.text();
            for want in ["protocol 2", "view “items” is a list of 2 item(s)", "after selecting “welcome”, a detail", "action “done” on “welcome” is answered ok", "command “refresh” is answered ok", "it sends 1 badge(s)"] {
                assert!(text.contains(want), "{want}: {text}");
            }
        }
    }

    #[test]
    fn a_bad_folder_or_name_is_refused() {
        let t = crate::testing::TempDir::new("starter-bad");
        assert!(write(&t.path().join("My Ext"), None, 1, Shape::Inbox).unwrap_err().contains("name the folder like my-extension"));
        assert!(write(&t.path().join("ok"), Some(&"n".repeat(41)), 1, Shape::Inbox).unwrap_err().contains("1–40"));
        assert!(write(&t.path().join("ok"), None, 3, Shape::Inbox).unwrap_err().contains("protocol 1 and 2"));
        let full = t.path().join("full");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("x"), "").unwrap();
        assert!(write(&full, None, 1, Shape::Inbox).unwrap_err().contains("isn't empty"));
        assert_eq!(name_of("demo-inbox"), "Demo inbox");
    }
}
