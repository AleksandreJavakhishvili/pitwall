//! Pitwall's services for a tile: the Log, its own key-value store, its
//! settings and requests about its views. None needs a permission in WIT
//! 0.1: each reaches only the tile's own things. Timers are on
//! [`Cx`](crate::Cx) (`after`, `every`), since they call back into the
//! tile.

use crate::bindings::pitwall::tile as wit;

/// Why a host call didn't do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// The tile isn't granted the permission this needs.
    Denied,
    /// Too many calls or things at once (over 200 host calls a second, or
    /// 32 timers); try again later.
    Busy,
    /// Over a size limit or a quota.
    TooLarge,
    /// An argument isn't valid; the text says which and why.
    Invalid(String),
    /// Pitwall couldn't do it (a disk error, say); the text says what
    /// happened.
    Failed(String),
    /// Stored data couldn't be read as the type asked for.
    Decode(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Denied => write!(f, "not allowed: the permission isn't granted"),
            HostError::Busy => write!(f, "too many calls at once; try again later"),
            HostError::TooLarge => write!(f, "over a size limit"),
            HostError::Invalid(why) => write!(f, "not valid: {why}"),
            HostError::Failed(why) => write!(f, "failed: {why}"),
            HostError::Decode(why) => write!(f, "can't be read: {why}"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<wit::types::HostError> for HostError {
    fn from(e: wit::types::HostError) -> HostError {
        match e {
            wit::types::HostError::Denied => HostError::Denied,
            wit::types::HostError::Busy => HostError::Busy,
            wit::types::HostError::TooLarge => HostError::TooLarge,
            wit::types::HostError::Invalid(s) => HostError::Invalid(s),
            wit::types::HostError::Failed(s) => HostError::Failed(s),
        }
    }
}

/// A toast's tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// Neutral news.
    #[default]
    Info,
    /// Something worked.
    Good,
    /// Something needs attention.
    Warn,
    /// Something failed.
    Bad,
}

impl Tone {
    pub(crate) fn wit(self) -> wit::types::Tone {
        match self {
            Tone::Info => wit::types::Tone::Info,
            Tone::Good => wit::types::Tone::Good,
            Tone::Warn => wit::types::Tone::Warn,
            Tone::Bad => wit::types::Tone::Bad,
        }
    }
}

/// Lines for the tile's Log, which the user and `pitwall extension dev`
/// read. Each line is at most 4 KiB (longer lines are cut). `println!`
/// and `eprintln!` reach the same Log (as info and warnings).
pub mod log {
    // Built for the host (the SDK's own unit tests), lines go to stderr:
    // there is no Pitwall to take them.
    #[cfg(target_arch = "wasm32")]
    use super::wit::log;
    #[cfg(not(target_arch = "wasm32"))]
    mod log {
        pub fn debug(m: &str) {
            eprintln!("debug: {m}");
        }
        pub fn info(m: &str) {
            eprintln!("info: {m}");
        }
        pub fn warn(m: &str) {
            eprintln!("warn: {m}");
        }
        pub fn error(m: &str) {
            eprintln!("error: {m}");
        }
    }

    /// Detail for whoever is debugging the tile.
    pub fn debug(message: impl AsRef<str>) {
        log::debug(message.as_ref());
    }

    /// Something worth knowing happened.
    pub fn info(message: impl AsRef<str>) {
        log::info(message.as_ref());
    }

    /// Something went wrong, and the tile carried on.
    pub fn warn(message: impl AsRef<str>) {
        log::warn(message.as_ref());
    }

    /// Something failed.
    pub fn error(message: impl AsRef<str>) {
        log::error(message.as_ref());
    }
}

/// The tile's own key-value store: only this extension's data, kept on
/// disk outside its folder (it survives updates), written after each call
/// into the tile that changed it. At most 10 MiB in all; keys are 1 to 256
/// bytes of UTF-8 without control characters.
pub mod storage {
    use super::wit::storage;
    use super::HostError;
    use serde::de::DeserializeOwned;
    use serde::Serialize;

    /// The value of `key`, or `None` when it has none.
    pub fn get(key: &str) -> Result<Option<Vec<u8>>, HostError> {
        storage::get(key).map_err(HostError::from)
    }

    /// The values of `keys`, in the same order (`None` for a key with no
    /// value), in one host call. At most 1024 keys a call
    /// ([`HostError::TooLarge`] over that; [`entries`] takes any number).
    pub fn get_many<K: AsRef<str>>(keys: &[K]) -> Result<Vec<Option<Vec<u8>>>, HostError> {
        let keys: Vec<String> = keys.iter().map(|k| k.as_ref().to_string()).collect();
        storage::get_many(&keys).map_err(HostError::from)
    }

    /// Every key that starts with `prefix` and its value, in key order:
    /// [`list`] and [`get_many`] (one call per 1024 keys), so a tile
    /// loads hundreds of items at start in a couple of host calls.
    pub fn entries(prefix: &str) -> Result<Vec<(String, Vec<u8>)>, HostError> {
        let keys = list(prefix)?;
        let mut out = Vec::with_capacity(keys.len());
        for chunk in keys.chunks(1024) {
            for (k, v) in chunk.iter().zip(get_many(chunk)?) {
                if let Some(v) = v {
                    out.push((k.clone(), v));
                }
            }
        }
        Ok(out)
    }

    /// [`entries`] with each value read as JSON (kept by [`set_json`]). A
    /// value that isn't a `T` is an error.
    pub fn entries_json<T: DeserializeOwned>(prefix: &str) -> Result<Vec<(String, T)>, HostError> {
        entries(prefix)?
            .into_iter()
            .map(|(k, b)| serde_json::from_slice(&b).map(|v| (k.clone(), v)).map_err(|e| HostError::Decode(format!("{k}: {e}"))))
            .collect()
    }

    /// Set `key` to `value`. [`HostError::TooLarge`] when the store would
    /// pass 10 MiB.
    pub fn set(key: &str, value: impl AsRef<[u8]>) -> Result<(), HostError> {
        storage::set(key, value.as_ref()).map_err(HostError::from)
    }

    /// Remove `key` (a key with no value is no error).
    pub fn delete(key: &str) -> Result<(), HostError> {
        storage::delete(key).map_err(HostError::from)
    }

    /// The keys that start with `prefix` (all for `""`), in byte order.
    pub fn list(prefix: &str) -> Result<Vec<String>, HostError> {
        storage::list(prefix).map_err(HostError::from)
    }

    /// The bytes the store takes now, out of its 10 MiB.
    pub fn usage() -> u64 {
        storage::usage()
    }

    /// The value of `key` as text.
    pub fn get_string(key: &str) -> Result<Option<String>, HostError> {
        get(key)?.map(|b| String::from_utf8(b).map_err(|e| HostError::Decode(e.to_string()))).transpose()
    }

    /// The value of `key`, kept as JSON by [`set_json`].
    pub fn get_json<T: DeserializeOwned>(key: &str) -> Result<Option<T>, HostError> {
        get(key)?.map(|b| serde_json::from_slice(&b).map_err(|e| HostError::Decode(e.to_string()))).transpose()
    }

    /// Keep `value` under `key` as JSON.
    pub fn set_json<T: Serialize + ?Sized>(key: &str, value: &T) -> Result<(), HostError> {
        let bytes = serde_json::to_vec(value).map_err(|e| HostError::Invalid(e.to_string()))?;
        set(key, bytes)
    }
}

/// The tile's settings, as the user set them in Settings. The manifest
/// declares them (`settings`: key, type, label, default); Pitwall draws the
/// form and sends [`Event::SettingsChanged`](crate::events::Event) when
/// they change.
pub mod settings {
    use super::wit::settings;
    use super::HostError;

    /// A setting's value.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        /// A `text`, `url` or `choice` setting (a choice gives its value).
        Text(String),
        /// A `number` setting.
        Number(f64),
        /// A `bool` setting (a switch).
        Flag(bool),
    }

    /// The value of setting `key`: what the user set, else the manifest's
    /// default; `None` when it has neither or isn't a setting. Never a
    /// secret: use [`secret`].
    pub fn get(key: &str) -> Option<Value> {
        settings::get(key).map(|v| match v {
            settings::SettingValue::Text(t) => Value::Text(t),
            settings::SettingValue::Number(n) => Value::Number(n),
            settings::SettingValue::Flag(b) => Value::Flag(b),
        })
    }

    /// A `text`, `url` or `choice` setting's value.
    pub fn text(key: &str) -> Option<String> {
        match get(key) {
            Some(Value::Text(t)) => Some(t),
            _ => None,
        }
    }

    /// A `number` setting's value.
    pub fn number(key: &str) -> Option<f64> {
        match get(key) {
            Some(Value::Number(n)) => Some(n),
            _ => None,
        }
    }

    /// A `bool` setting's value.
    pub fn flag(key: &str) -> Option<bool> {
        match get(key) {
            Some(Value::Flag(b)) => Some(b),
            _ => None,
        }
    }

    /// The value of `secret` setting `key`, from the system's secure
    /// store; `None` when the user hasn't set it. Keep it in memory only:
    /// never store, log or show it.
    pub fn secret(key: &str) -> Result<Option<String>, HostError> {
        settings::secret(key).map_err(HostError::from)
    }

    /// Open the extension's card in Settings (for a "Set up" button when a
    /// required setting is missing).
    pub fn open() {
        settings::open();
    }
}

/// Requests about the tile's own views. They act only inside the tile's
/// own area of the window.
pub mod ui {
    use super::wit::ui;
    use super::Tone;

    /// Render `view` again once the current call returns. Inside a
    /// listener, [`Cx::notify`](crate::Cx::notify) is simpler.
    pub fn request_render(view: &str) {
        ui::request_render(view);
    }

    /// Scroll the node at `key_path` (keys from the root down, joined by
    /// `/`, without the root's, as [`Cx::focus`](crate::Cx::focus) takes
    /// it) into sight after `view`'s next render.
    pub fn scroll_to(view: &str, key_path: &str) {
        ui::scroll_to(view, key_path);
    }

    /// Show a short message in a toast, labelled with the tile's name. At
    /// most 200 characters and 3 toasts a minute; more are dropped and
    /// noted in the Log.
    pub fn toast(text: impl AsRef<str>, tone: Tone) {
        ui::toast(text.as_ref(), tone.wit());
    }

    /// Open one of the extension's tiles (an id from its manifest's
    /// `contributes.tiles`), or focus its view when one is open; the tile
    /// hears [`Tile::on_view_opened`](crate::Tile::on_view_opened).
    pub fn open_view(tile: &str) {
        ui::open_view(tile);
    }

    /// Hand Pitwall an image's bytes (PNG, JPEG, GIF, WebP or SVG; at most
    /// 8 MiB) under `name`, for [`image_data`](crate::image_data). Kept
    /// until put again or dropped; at most 64 MiB for all of a tile's
    /// images. `Invalid` when the bytes aren't one of those formats,
    /// `TooLarge` over a limit.
    pub fn put_image(name: &str, bytes: &[u8]) -> Result<(), super::HostError> {
        ui::put_image(name, bytes).map_err(Into::into)
    }

    /// Forget the image put under `name`.
    pub fn drop_image(name: &str) {
        ui::drop_image(name);
    }

    /// How wide each of `texts` is on one line in Pitwall's `font` at
    /// `size` logical pixels and `weight`, in logical pixels, kerned as
    /// Pitwall draws it: to size a canvas's boxes around their labels
    /// ([`Label::width`](crate::canvas::Label::width) measures one label).
    /// One width per text, in order. A character Pitwall's font lacks
    /// counts as an average letter. At most 1 000 texts and 64 KiB a call
    /// (more: an empty list, noted in the Log). The test harness gives the
    /// same numbers.
    pub fn measure_text<S: AsRef<str>>(texts: &[S], font: crate::canvas::TextFont, size: f32, weight: crate::canvas::TextWeight) -> Vec<f32> {
        let texts: Vec<String> = texts.iter().map(|t| t.as_ref().to_string()).collect();
        ui::measure_text(&texts, font.wit(), size, weight.wit())
    }
}
