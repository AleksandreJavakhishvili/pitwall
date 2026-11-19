//! `web.open`: an agent (or the user's own `pitwall open --web`) shows the
//! user a web page in Pitwall (docs/spec/web.md, "Agents showing pages"):
//! a web tile next to the agent's terminal, or a window of its own. The
//! `pitwall` MCP server's `show_web` tool is a thin wrapper over this call
//! ([`crate::mcp`]). It only shows a page: nothing about the page comes
//! back, and nothing can be done in it but by the user.
//!
//! The address rules ([`address`]) are the address field's, so what an
//! agent may ask for is what the user could type: http(s) only.
//!
//! As `viewer.open`, answers are read leniently and carry Pitwall's own
//! words (`text`), so a CLI and a Pitwall of different versions get on.

use serde::{Deserialize, Serialize};

use crate::show::ViewerPlace;

/// `web.open`'s shape (the asker's and the answer's `proto`).
pub const WEB_PROTO: u32 = 1;

/// `web.open`: a page for the user, in the web tile the agent opened
/// before (moved when `where` names another place), else a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WebOpen {
    /// The agent asking (id or name). Default: the agent the caller runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The address, as typed (`None`: an empty page, its address field
    /// ready for the user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Where (`where` on the wire). `None`: where the agent's page already
    /// is, else right of its terminal.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub place: Option<ViewerPlace>,
    /// Whose terminal `where` is next to (an agent's id or name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<u32>,
}

/// What `web.open` did. Every field has a default, so any version's answer
/// is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WebOpened {
    pub proto: u32,
    pub agent_id: String,
    pub agent_name: String,
    /// The window it is in (`main`, `pitwall-2`, or a web window's title).
    pub window: String,
    pub in_background: bool,
    /// The address loading (`None`: an empty page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The agent's page tile was there already and now shows this.
    pub reused: bool,
    /// Where it is: a [`ViewerPlace`] word.
    pub place: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asked: Option<String>,
    /// Where it is, in words ("a web tile in space Main (window main),
    /// left of api's terminal").
    pub shown_in: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The browser profile it is in ("Work", "Default"): the agent's
    /// project's, never the agent's pick. `None`: a Pitwall from before
    /// profiles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Pitwall's own words for all of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// What the server hands the app: the address checked, the agents
/// resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WebShow {
    /// The agent that asked (its page tile goes next to its terminal).
    pub agent_id: String,
    pub url: Option<String>,
    pub place: Option<ViewerPlace>,
    /// The agent whose terminal `place` is next to (resolved id).
    #[serde(default)]
    pub of: Option<String>,
}

/// Where the app showed a [`WebShow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebShown {
    pub window: String,
    pub in_background: bool,
    pub place: ViewerPlace,
    /// Where it is, in words.
    pub at: String,
    #[serde(default)]
    pub note: Option<String>,
    /// The agent's page tile was reused.
    #[serde(default)]
    pub reused: bool,
    /// The name of the browser profile it is in (its project's; empty: an
    /// app from before profiles).
    #[serde(default)]
    pub profile: String,
}

/// What was opened, in words (`pitwall open --web` and the tool's result).
pub fn text(o: &WebOpened) -> String {
    let what = match (&o.url, o.reused) {
        (Some(u), _) => format!("Opened {u}"),
        (None, true) => "Showed the page you opened before (on whatever it shows now)".to_string(),
        (None, false) => "Opened an empty web page (its address field is there for the user to type in; the keyboard stayed where it was)".to_string(),
    };
    let at = if o.shown_in.is_empty() { "a web tile" } else { o.shown_in.as_str() };
    let mut s = format!("{what} in {at} in Pitwall.\n");
    if o.reused && o.url.is_some() {
        s.push_str("It is the page you opened before, now showing this.\n");
    }
    if let Some(n) = &o.note {
        s.push_str(n);
        s.push('\n');
    }
    if let Some(p) = o.profile.as_deref().filter(|p| !p.is_empty()) {
        s.push_str(&format!("It is in the {p} browser profile (the sign-ins and site data this project's pages use in Pitwall; the user picks it per project).\n"));
    }
    if o.in_background {
        s.push_str(if o.place == ViewerPlace::Window.word() {
            "The user is in another app: the window opened behind it without taking the keyboard.\n"
        } else {
            "Pitwall isn't in front, so it opened there without coming forward; the user sees it when they switch to Pitwall.\n"
        });
    }
    s.push_str("Only the user sees the page: you get nothing back from it and can't click, type or read anything in it.\n");
    s
}

/// `web.open`'s answer in words, whatever shape it came in.
pub fn opened_text(v: &serde_json::Value) -> String {
    if let Some(t) = v.get("text").and_then(serde_json::Value::as_str).filter(|t| !t.trim().is_empty()) {
        return t.to_string();
    }
    match serde_json::from_value::<WebOpened>(v.clone()) {
        Ok(o) => text(&o),
        Err(_) => "Pitwall opened the page, but answered in a shape this command-line tool doesn't read (Pitwall is newer than it); the user sees it in Pitwall.\n".into(),
    }
}

/// What text typed as an address means (the address field's rules, and
/// what an agent may ask for): the address to load, or why it isn't one.
/// A full `http(s)://` address as it is; a host without a scheme
/// (`example.com/docs`, `localhost:3000`, `192.168.1.5:8080`) gets
/// `https://`, or `http://` for local and LAN hosts. Text that isn't a host
/// (words with spaces, no dot) and every other scheme (`file:`,
/// `javascript:`, `data:`, app schemes) are refused: there's no search.
pub fn address(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Type a web address".into());
    }
    if t.chars().any(char::is_whitespace) {
        return Err(format!("“{t}” isn't a web address (there's no search here)"));
    }
    if is_http(t) {
        let rest = &t[t.find("://").map_or(0, |i| i + 3)..];
        if host_of(rest).is_empty() {
            return Err(format!("“{t}” has no host"));
        }
        return Ok(t.to_string());
    }
    if let Some((scheme, after)) = t.split_once(':') {
        // `localhost:3000` and `host:port` are hosts, not schemes.
        let port_after = after.chars().next().is_some_and(|c| c.is_ascii_digit());
        if !port_after && !scheme.contains('.') && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-".contains(c)) {
            return Err(format!("Only web addresses (http, https) open here, not {scheme}:"));
        }
    }
    let host = host_of(t).to_ascii_lowercase();
    let local = allowed(&format!("http://{t}")) || host.ends_with(".local") || host.ends_with(".internal");
    let hostish = host == "localhost"
        || host.starts_with('[')
        || (host.contains('.')
            && !host.starts_with('.')
            && !host.ends_with('.')
            && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || !c.is_ascii()));
    if !hostish {
        return Err(format!("“{t}” isn't a web address (there's no search here)"));
    }
    Ok(format!("{}://{t}", if local { "http" } else { "https" }))
}

/// An `http://` or `https://` address.
pub fn is_http(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// The host (with any port dropped) at the start of `rest` (an address
/// after its `scheme://`).
fn host_of(rest: &str) -> &str {
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = host_port.rsplit_once('@').map_or(host_port, |(_, h)| h);
    if host_port.starts_with('[') {
        return host_port.split_inclusive(']').next().unwrap_or("");
    }
    host_port.split(':').next().unwrap_or("")
}

/// Only local dev servers: localhost, loopback, private LAN and tailnet
/// addresses, over http(s) (an agent's Preview stays on these).
pub fn allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) else {
        return false;
    };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host == "::1" {
        return true;
    }
    match host.parse::<std::net::Ipv4Addr>() {
        // 100.64/10: Tailscale (CGNAT).
        Ok(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_unspecified()
                || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_request_and_answer_on_the_wire() {
        let r = WebOpen { url: Some("example.com".into()), place: Some(ViewerPlace::Left), of: Some("web".into()), ..Default::default() };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"url": "example.com", "where": "left", "of": "web"}));
        assert_eq!(serde_json::from_value::<WebOpen>(v).unwrap(), r);
        assert_eq!(serde_json::from_value::<WebOpen>(json!({})).unwrap(), WebOpen::default());
        // A newer answer, fields and places unknown here: still read.
        let newer = json!({"proto": 9, "agentId": "a1", "place": "floating", "shownIn": "a floating sheet", "url": "https://x.dev/", "badge": 1});
        assert!(opened_text(&newer).starts_with("Opened https://x.dev/ in a floating sheet in Pitwall."));
        assert_eq!(opened_text(&json!({"text": "Done.\n"})), "Done.\n");
        assert!(opened_text(&json!({"url": 3})).starts_with("Pitwall opened the page"));
    }

    #[test]
    fn the_words_say_where_and_that_nothing_comes_back() {
        let o = WebOpened { url: Some("https://example.com/".into()), place: "left".into(), shown_in: "a web tile in space Main (window main), left of api's terminal".into(), reused: true, ..Default::default() };
        let t = text(&o);
        assert!(t.starts_with("Opened https://example.com/ in a web tile in space Main (window main), left of api's terminal in Pitwall.\n"), "{t}");
        assert!(t.contains("the page you opened before"));
        assert!(t.ends_with("you get nothing back from it and can't click, type or read anything in it.\n"));
        let empty = text(&WebOpened { place: "window".into(), in_background: true, ..Default::default() });
        assert!(empty.starts_with("Opened an empty web page"), "{empty}");
        assert!(empty.contains("behind it without taking the keyboard"));
        // Moved without an address: its page, not an empty one.
        let moved = text(&WebOpened { place: "window".into(), reused: true, shown_in: "a window of its own".into(), ..Default::default() });
        assert!(moved.starts_with("Showed the page you opened before (on whatever it shows now) in a window of its own"), "{moved}");
        assert!(!moved.contains("empty"));
        assert!(!moved.contains("browser profile"), "an app from before profiles says none");
        // The profile it is in, for the agent to tell the user.
        let work = text(&WebOpened { url: Some("https://example.com/".into()), place: "right".into(), profile: Some("Work".into()), ..Default::default() });
        assert!(work.contains("It is in the Work browser profile (the sign-ins and site data this project's pages use in Pitwall; the user picks it per project).\n"), "{work}");
        let o: WebOpened = serde_json::from_value(json!({"profile": "Default"})).unwrap();
        assert_eq!(o.profile.as_deref(), Some("Default"));
        let shown: WebShown = serde_json::from_value(json!({"window": "main", "inBackground": false, "place": "right", "at": "x"})).unwrap();
        assert_eq!(shown.profile, "", "an app from before profiles");
    }

    #[test]
    fn typed_addresses() {
        for (typed, url) in [
            ("https://example.com", "https://example.com"),
            ("http://localhost:3000/x?y=1", "http://localhost:3000/x?y=1"),
            ("  example.com  ", "https://example.com"),
            ("example.com/docs#a", "https://example.com/docs#a"),
            ("teams.microsoft.com", "https://teams.microsoft.com"),
            ("localhost:3000", "http://localhost:3000"),
            ("localhost", "http://localhost"),
            ("127.0.0.1:8080/app", "http://127.0.0.1:8080/app"),
            ("192.168.1.5:5173", "http://192.168.1.5:5173"),
            ("[::1]:3000", "http://[::1]:3000"),
            ("app.localhost:8443", "http://app.localhost:8443"),
            ("printer.local", "http://printer.local"),
            ("8.8.8.8", "https://8.8.8.8"),
            ("HTTPS://Example.com", "HTTPS://Example.com"),
            ("bücher.de", "https://bücher.de"),
        ] {
            assert_eq!(address(typed).as_deref(), Ok(url), "{typed}");
        }
        for bad in [
            "",
            "   ",
            "rust web view",
            "hello",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,<b>x</b>",
            "msteams:/l/chat",
            "mailto:a@b.c",
            "about:blank",
            "https://",
            ".com",
            "example.",
        ] {
            assert!(address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_local_dev_servers() {
        for ok in [
            "http://localhost:3000",
            "http://localhost:5173/app?x=1",
            "https://app.localhost:8443/",
            "http://127.0.0.1:8000",
            "http://0.0.0.0:4000",
            "http://[::1]:3000/",
            "http://192.168.1.20:3000",
            "http://10.0.0.5",
            "http://172.20.1.1:80",
            "http://100.101.2.3:3000",
        ] {
            assert!(allowed(ok), "{ok}");
        }
        for no in ["https://example.com", "http://8.8.8.8", "http://100.200.1.1", "file:///etc/passwd", "javascript:alert(1)", "http://localhost.evil.com", "ftp://localhost"] {
            assert!(!allowed(no), "{no}");
        }
    }
}
