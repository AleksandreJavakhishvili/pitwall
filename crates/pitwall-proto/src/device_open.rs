//! `device.open`: an agent (or the user's own `pitwall open --device`)
//! shows the user a device's screen in Pitwall (docs/spec/device.md,
//! "Agents: show_device"): an iOS Simulator's or an Android emulator's or
//! phone's, in a device tile next to the agent's terminal. The `pitwall`
//! MCP server's `show_device` tool is a thin wrapper over this call
//! ([`crate::mcp`]). It only shows the screen: nothing of it comes back,
//! and only the user can tap or type in it.
//!
//! A device still booting is waited for (at most [`WAIT_SECS`]): an agent
//! that has just started a simulator can ask straight away.
//!
//! As `web.open`, answers are read leniently and carry Pitwall's own words
//! (`text`), so a CLI and a Pitwall of different versions get on.

use serde::{Deserialize, Serialize};

/// `device.open`'s shape (the asker's and the answer's `proto`).
pub const DEVICE_PROTO: u32 = 1;

/// How long Pitwall waits for a named device that is still booting (or,
/// with none named, for one to boot), in seconds.
pub const WAIT_SECS: u64 = 60;

/// `device.open`: a device's screen shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOpen {
    /// The agent asking (id or name): the tile goes next to its terminal.
    /// Default: the agent the caller runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The device: its name ("iPhone 17"), a simulator's UDID, an adb
    /// serial (`emulator-5554`), or a key (`ios:<udid>`,
    /// `android:<serial>`). `None`: the one booted device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<u32>,
}

/// What `device.open` did. Every field has a default, so any version's
/// answer is read. Nothing of the screen is in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceOpened {
    pub proto: u32,
    pub agent_id: String,
    pub agent_name: String,
    /// The window it is in (`main`, `pitwall-2`, or a window of its own's
    /// title).
    pub window: String,
    pub in_background: bool,
    /// The device's key (`ios:<udid>`, `android:<serial>`).
    pub device: String,
    /// Its name, as Pitwall lists it ("iPhone 17 (iOS 26.1)").
    pub name: String,
    /// Its tile was open already.
    pub reused: bool,
    /// Pitwall waited for it to finish booting.
    pub waited: bool,
    /// Where it is, in words.
    pub shown_in: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Pitwall's own words for all of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// What the server hands the app: the agent resolved; the device as asked
/// (the app knows the devices there are).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceShow {
    /// The agent that asked (the tile goes next to its terminal).
    pub agent_id: String,
    #[serde(default)]
    pub device: Option<String>,
}

/// Where the app showed a [`DeviceShow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceShown {
    pub window: String,
    pub in_background: bool,
    pub device: String,
    pub name: String,
    /// Where it is, in words.
    pub at: String,
    pub note: Option<String>,
    pub reused: bool,
    pub waited: bool,
}

/// What was shown, in words (`pitwall open --device` and the tool's
/// result).
pub fn text(o: &DeviceOpened) -> String {
    let name = if o.name.is_empty() { o.device.as_str() } else { o.name.as_str() };
    let at = if o.shown_in.is_empty() { "a device tile" } else { o.shown_in.as_str() };
    let mut s = if o.reused { format!("{name}'s screen was shown already, in {at}.\n") } else { format!("Showed {name}'s screen in {at}.\n") };
    if o.waited {
        s.push_str("Pitwall waited for it to finish booting.\n");
    }
    if let Some(n) = &o.note {
        s.push_str(n);
        s.push('\n');
    }
    if o.in_background {
        s.push_str("Pitwall isn't in front, so it opened there without coming forward; the user sees it when they switch to Pitwall.\n");
    }
    s.push_str("Only the user sees the screen and can tap or type in it: you get nothing back from it.\n");
    s
}

/// `device.open`'s answer in words, whatever shape it came in.
pub fn opened_text(v: &serde_json::Value) -> String {
    if let Some(t) = v.get("text").and_then(serde_json::Value::as_str).filter(|t| !t.trim().is_empty()) {
        return t.to_string();
    }
    match serde_json::from_value::<DeviceOpened>(v.clone()) {
        Ok(o) => text(&o),
        Err(_) => "Pitwall showed the device, but answered in a shape this command-line tool doesn't read (Pitwall is newer than it); the user sees it in Pitwall.\n".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_request_and_answer_on_the_wire() {
        let r = DeviceOpen { device: Some("iPhone 17".into()), ..Default::default() };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"device": "iPhone 17"}));
        assert_eq!(serde_json::from_value::<DeviceOpen>(v).unwrap(), r);
        assert_eq!(serde_json::from_value::<DeviceOpen>(json!({})).unwrap(), DeviceOpen::default());
        // A newer answer, with fields unknown here: still read.
        let newer = json!({"proto": 9, "name": "Pixel 8", "shownIn": "a floating sheet", "badge": 1});
        assert!(opened_text(&newer).starts_with("Showed Pixel 8's screen in a floating sheet."), "{}", opened_text(&newer));
        assert_eq!(opened_text(&json!({"text": "Done.\n"})), "Done.\n");
        assert!(opened_text(&json!({"name": 3})).starts_with("Pitwall showed the device"));
    }

    #[test]
    fn the_words_say_where_and_that_nothing_comes_back() {
        let o = DeviceOpened { name: "iPhone 17 (iOS 26.1)".into(), shown_in: "the device tile “iPhone 17” in space Main (window main), right of api's terminal".into(), waited: true, in_background: true, ..Default::default() };
        let t = text(&o);
        assert!(t.starts_with("Showed iPhone 17 (iOS 26.1)'s screen in the device tile"), "{t}");
        assert!(t.contains("waited for it to finish booting"), "{t}");
        assert!(t.contains("without coming forward"), "{t}");
        assert!(t.ends_with("you get nothing back from it.\n"), "{t}");
        let again = text(&DeviceOpened { reused: true, device: "android:emulator-5554".into(), ..Default::default() });
        assert!(again.starts_with("android:emulator-5554's screen was shown already, in a device tile."), "{again}");
    }
}
