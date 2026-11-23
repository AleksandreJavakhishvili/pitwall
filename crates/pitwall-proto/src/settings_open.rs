//! `settings.open`: an agent (or the user's own `pitwall open --settings`)
//! shows the user Pitwall's Settings, at a page, in the Settings tile
//! (docs/spec/layout.md "The Settings tile"): where it is, else a tile next
//! to the agent's terminal. The `pitwall` MCP server's `show_settings` tool
//! is a thin wrapper over this call ([`crate::mcp`]). It only shows
//! Settings: nothing of what Settings holds comes back, nothing in it can
//! be changed by the caller, and it never takes the keyboard or brings
//! Pitwall forward.
//!
//! As `web.open`, answers are read leniently and carry Pitwall's own words
//! (`text`), so a CLI and a Pitwall of different versions get on.

use serde::{Deserialize, Serialize};

/// `settings.open`'s shape (the asker's and the answer's `proto`).
pub const SETTINGS_PROTO: u32 = 1;

/// `settings.open`: Settings shown to the user at `page`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsOpen {
    /// The agent asking (id or name): the tile goes next to its terminal.
    /// Default: the agent the caller runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The page (`general`, `agents`, `extensions`…: a
    /// [`Page`](crate::settings::Page) id). `None`: the page Settings
    /// showed last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// An extension's id: its card unfolded on the Extensions page (the
    /// page then defaults to Extensions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    /// With `extension`: its secret setting (key) the user is asked to
    /// enter, marked on the card. Its value never passes here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// With `extension`: the connection (name or id) the card shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<u32>,
}

/// What `settings.open` did. Every field has a default, so any version's
/// answer is read. Nothing of what Settings holds is in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsOpened {
    pub proto: u32,
    pub agent_id: String,
    pub agent_name: String,
    /// The window it is in (`main`, `pitwall-2`, or `Settings`: a window
    /// of its own).
    pub window: String,
    pub in_background: bool,
    /// The page shown (its id).
    pub page: String,
    /// The Settings tile was there already.
    pub reused: bool,
    /// Where it is, in words ("the Settings tile in space Main (window
    /// main), right of api's terminal").
    pub shown_in: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Pitwall's own words for all of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// What the server hands the app: the agent resolved, the page checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsShow {
    /// The agent that asked (the tile goes next to its terminal).
    pub agent_id: String,
    pub page: Option<String>,
    #[serde(default)]
    pub extension: Option<String>,
    /// The secret setting to mark on the extension's card.
    #[serde(default)]
    pub secret: Option<String>,
    /// The connection the card shows (name or id).
    #[serde(default)]
    pub connection: Option<String>,
}

/// Where the app showed a [`SettingsShow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsShown {
    pub window: String,
    pub in_background: bool,
    /// The page shown (its id).
    pub page: String,
    /// Where it is, in words.
    pub at: String,
    pub note: Option<String>,
    /// The Settings tile was there already.
    pub reused: bool,
}

/// What was shown, in words (`pitwall open --settings` and the tool's
/// result).
pub fn text(o: &SettingsOpened) -> String {
    let page = crate::settings::Page::parse(&o.page).map_or(o.page.as_str(), |p| p.label());
    let at = if o.shown_in.is_empty() { "the Settings tile" } else { o.shown_in.as_str() };
    let mut s = if page.is_empty() { format!("Showed Pitwall's Settings in {at}.\n") } else { format!("Showed Pitwall's Settings, the {page} page, in {at}.\n") };
    if let Some(n) = &o.note {
        s.push_str(n);
        s.push('\n');
    }
    if o.in_background {
        s.push_str("Pitwall isn't in front, so it opened there without coming forward; the user sees it when they switch to Pitwall.\n");
    }
    s.push_str("Only the user sees Settings: you get nothing back from it and can't change, click or type anything in it.\n");
    s
}

/// `settings.open`'s answer in words, whatever shape it came in.
pub fn opened_text(v: &serde_json::Value) -> String {
    if let Some(t) = v.get("text").and_then(serde_json::Value::as_str).filter(|t| !t.trim().is_empty()) {
        return t.to_string();
    }
    match serde_json::from_value::<SettingsOpened>(v.clone()) {
        Ok(o) => text(&o),
        Err(_) => "Pitwall showed Settings, but answered in a shape this command-line tool doesn't read (Pitwall is newer than it); the user sees it in Pitwall.\n".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_request_and_answer_on_the_wire() {
        let r = SettingsOpen { page: Some("extensions".into()), extension: Some("inbox".into()), ..Default::default() };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"page": "extensions", "extension": "inbox"}));
        assert_eq!(serde_json::from_value::<SettingsOpen>(v).unwrap(), r);
        assert_eq!(serde_json::from_value::<SettingsOpen>(json!({})).unwrap(), SettingsOpen::default());
        // A newer answer, with fields unknown here: still read.
        let newer = json!({"proto": 9, "page": "agents", "shownIn": "a floating sheet", "badge": 1});
        assert!(opened_text(&newer).starts_with("Showed Pitwall's Settings, the Agents page, in a floating sheet."), "{}", opened_text(&newer));
        assert_eq!(opened_text(&json!({"text": "Done.\n"})), "Done.\n");
        assert!(opened_text(&json!({"page": 3})).starts_with("Pitwall showed Settings"));
    }

    #[test]
    fn the_words_say_where_and_that_nothing_comes_back() {
        let o = SettingsOpened { page: "extensions".into(), shown_in: "the Settings tile in space Main (window main), right of api's terminal".into(), in_background: true, ..Default::default() };
        let t = text(&o);
        assert!(t.starts_with("Showed Pitwall's Settings, the Extensions page, in the Settings tile in space Main"), "{t}");
        assert!(t.contains("without coming forward"), "{t}");
        assert!(t.ends_with("you get nothing back from it and can't change, click or type anything in it.\n"), "{t}");
    }
}
