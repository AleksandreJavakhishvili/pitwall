//! Device previews (docs/spec/device.md): an Android device's or an iOS
//! Simulator's screen in a tile beside the agent's terminal, with touch
//! and keys.
//!
//! - [`list`]: the devices `adb devices -l` and `xcrun simctl list -j`
//!   report.
//! - [`geometry`]: a point in the tile to a point on the device's screen.
//! - [`scrcpy`]: what goes to scrcpy-server (its command line, control
//!   messages) and where the server file comes from ([`scrcpy::ServerSource`]).
//! - [`stream`]: one device's stream: a [`stream::Backend`] starts it
//!   (Android: adb + scrcpy-server, [`android`]; iOS: `pitwall-simhost`,
//!   [`ios`]), a thread reads and decodes it, and stopping it undoes
//!   everything it set up (the adb forward, the helper process).
//!
//! Nothing here reaches an agent: `show_device` only opens a tile.

pub mod android;
pub mod auto;
pub mod geometry;
pub mod ios;
pub mod list;
pub mod scrcpy;
pub mod stream;

use serde::{Deserialize, Serialize};

/// Which kind of device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Android,
    Ios,
}

/// Whether it can be shown now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Ready (adb `device`, simctl `Booted`).
    Ready,
    /// Known but not usable: why, in the user's words.
    NotReady(String),
}

/// A device a tile can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub platform: Platform,
    /// adb's serial (`emulator-5554`, a USB serial, `host:port`) or the
    /// simulator's UDID.
    pub id: String,
    /// What the user calls it ("Pixel 8", "iPhone 17 Pro").
    pub name: String,
    /// An emulator or simulator (not a phone on a cable).
    pub virtual_device: bool,
    pub state: State,
}

impl Device {
    /// The key a tile is kept by (`android:<serial>`, `ios:<udid>`).
    pub fn key(&self) -> String {
        key(self.platform, &self.id)
    }
}

/// A device's key.
pub fn key(platform: Platform, id: &str) -> String {
    match platform {
        Platform::Android => format!("android:{id}"),
        Platform::Ios => format!("ios:{id}"),
    }
}

/// A key read back.
pub fn parse_key(key: &str) -> Option<(Platform, String)> {
    let (p, id) = key.split_once(':')?;
    let p = match p {
        "android" => Platform::Android,
        "ios" => Platform::Ios,
        _ => return None,
    };
    (!id.is_empty()).then(|| (p, id.to_string()))
}

#[cfg(test)]
mod tests;
