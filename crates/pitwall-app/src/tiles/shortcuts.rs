//! Tiles' shortcuts (`contributes.tiles[].shortcut`): keys that open a
//! tile from anywhere in Pitwall. Bound only while its extension is on and
//! the user granted it `shortcut` (never on by default: the permission
//! screen offers it switched off); the user may give it other keys in the
//! extension's card. Keys Pitwall keeps for itself are refused, and of two
//! extensions wanting the same keys the first (by name) gets them.
//!
//! gpui can't take one binding back, so each keystroke is bound once, to
//! [`TileShortcut`] carrying it, and what it opens is looked up when it
//! fires: a keystroke no longer any tile's goes on to whatever else binds
//! it.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use gpui::{Action, App, KeyBinding};
use pitwall_core::extensions::{keys, permissions, Manifest};

/// A tile's shortcut was pressed: `keystroke` as gpui bound it.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = tiles, no_json)]
pub struct TileShortcut {
    pub keystroke: String,
}

/// A tile's shortcut, as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub ext: String,
    pub tile: String,
    /// The tile's title.
    pub title: String,
    /// The keys it has now (the user's, else the ones it suggests), as
    /// tiles write them (`cmd-shift-u`).
    pub keys: String,
    /// The keys it suggests.
    pub suggested: String,
    /// Why it isn't bound: not granted, keys Pitwall keeps, another
    /// extension's. `None`: bound.
    pub off: Option<String>,
}

impl Shortcut {
    /// Its keys as the user reads them here (`⌘⇧U`).
    pub fn label(&self) -> String {
        keys::shortcut_label(&self.keys, mac())
    }
}

fn mac() -> bool {
    cfg!(target_os = "macos")
}

/// Whether `m` is granted `shortcut`.
fn granted(m: &Manifest, cx: &App) -> bool {
    let grants = crate::ui_state::get(cx).extension(&m.id).grants;
    permissions::effective(m, &grants).iter().any(|p| p == permissions::SHORTCUT)
}

/// The shortcuts of the extensions that are on, bound or not.
pub fn all(cx: &App) -> Vec<Shortcut> {
    let ui = crate::ui_state::get(cx);
    let mut out: Vec<Shortcut> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut on = crate::extensions::enabled(&crate::extensions::found(cx), &ui);
    on.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    for m in on {
        let granted = granted(&m, cx);
        let mine = ui.extension(&m.id).shortcuts;
        for t in &m.contributes.tiles {
            let Some(suggested) = t.shortcut.clone() else { continue };
            let keys = mine.get(&t.id).and_then(|k| keys::shortcut_normal(k)).unwrap_or_else(|| suggested.clone());
            let off = if !granted {
                Some("Off: it isn't granted “Use a keyboard shortcut”".to_string())
            } else if let Some(why) = keys::shortcut_problem(&keys) {
                Some(why)
            } else if taken.contains(&keys) {
                Some("Another extension has these keys".to_string())
            } else {
                None
            };
            if off.is_none() {
                taken.insert(keys.clone());
            }
            out.push(Shortcut { ext: m.id.clone(), tile: t.id.clone(), title: t.title.clone(), keys, suggested, off });
        }
    }
    out
}

/// Tile `tile` of `ext`'s shortcut, bound or not.
pub fn of_tile(ext: &str, tile: &str, cx: &App) -> Option<Shortcut> {
    all(cx).into_iter().find(|s| s.ext == ext && s.tile == tile)
}

/// What `keystroke` opens now: an extension and its tile.
pub fn target(keystroke: &str, cx: &App) -> Option<(String, String)> {
    all(cx).into_iter().find(|s| s.off.is_none() && keys::shortcut_keystroke(&s.keys, mac()).as_deref() == Some(keystroke)).map(|s| (s.ext, s.tile))
}

/// `s` kept for good: gpui's key bindings and the palette's hints take
/// `&'static str`. Each distinct string is kept once.
pub fn forever(s: &str) -> &'static str {
    static KEPT: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let map = kept.get_or_insert_with(HashMap::new);
    if let Some(s) = map.get(s) {
        return s;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    map.insert(s.to_string(), leaked);
    leaked
}

/// The keystrokes bound so far.
#[derive(Default)]
struct Bound(BTreeSet<String>);
impl gpui::Global for Bound {}

/// Bind the keystrokes of the shortcuts that are on and not bound yet
/// (after the extensions, their grants or the user's keys changed).
pub fn sync(cx: &mut App) {
    let want: Vec<String> = all(cx).iter().filter(|s| s.off.is_none()).filter_map(|s| keys::shortcut_keystroke(&s.keys, mac())).collect();
    let bound = cx.default_global::<Bound>();
    let new: Vec<String> = want.into_iter().filter(|k| !bound.0.contains(k)).collect();
    if new.is_empty() {
        return;
    }
    bound.0.extend(new.iter().cloned());
    cx.bind_keys(new.into_iter().map(|k| KeyBinding::new(forever(&k), TileShortcut { keystroke: k }, None)));
}

/// The user gives tile `tile` of `ext` the shortcut `keys`, or (empty, or
/// the keys it suggests) its own back: `Err` with why when Pitwall keeps
/// them or another extension has them.
pub fn set_keys(ext: &str, tile: &str, keys_given: &str, cx: &mut App) -> Result<(), String> {
    let given = keys_given.trim();
    let suggested = of_tile(ext, tile, cx).map(|s| s.suggested).unwrap_or_default();
    let normal = if given.is_empty() {
        None
    } else {
        if let Some(why) = keys::shortcut_problem(given) {
            return Err(why);
        }
        keys::shortcut_normal(given).filter(|k| *k != suggested)
    };
    let mine = normal.clone().unwrap_or_else(|| suggested.clone());
    if all(cx).iter().any(|s| s.off.is_none() && !(s.ext == ext && s.tile == tile) && s.keys == mine) {
        return Err("Another extension has these keys".into());
    }
    let (ext, tile) = (ext.to_string(), tile.to_string());
    crate::ui_state::update(cx, |s| {
        *s = s.clone().with_extension(&ext, |e| match &normal {
            Some(k) => {
                e.shortcuts.insert(tile.clone(), k.clone());
            }
            None => {
                e.shortcuts.remove(&tile);
            }
        })
    });
    sync(cx);
    Ok(())
}
