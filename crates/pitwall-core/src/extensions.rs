//! Extensions (docs/spec/extensions.md, draft): a folder in Pitwall's data
//! folder (`<data>/extensions/<id>/`) with a declarative manifest,
//! `extension.json`. No code from it is loaded or run: Pitwall reads the
//! manifest and shows the extension (a section of the sidebar, a switch in
//! Settings). This module finds the folders and reads and checks their
//! manifests; which ones are on is the app's (`ui.json` `extensions`).
//!
//! Version 1 of the manifest knows `id`, `name`, `icon` and `description`.
//! Keys it doesn't know are ignored, so a later version can add its own
//! without breaking this one; a manifest asking for a newer format (`v`)
//! is refused with a word on why.

use std::path::Path;

use serde_json::Value;

/// The manifest's file name in an extension's folder.
pub const MANIFEST: &str = "extension.json";

/// The manifest format this Pitwall reads.
pub const FORMAT: u64 = 1;

/// The longest id, icon name and name.
pub const MAX_ID: usize = 40;
pub const MAX_NAME: usize = 40;
/// The longest description.
pub const MAX_DESCRIPTION: usize = 200;

/// A manifest bigger than this isn't read (a manifest is a few lines).
const MAX_BYTES: u64 = 64 * 1024;

/// An extension's manifest, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Its id: lower-case letters, digits and `-`, starting with a letter;
    /// also its folder's name.
    pub id: String,
    /// What the sidebar and Settings call it.
    pub name: String,
    /// One of Pitwall's icon names (an app without it draws its own
    /// extension icon).
    pub icon: Option<String>,
    /// One line for Settings.
    pub description: Option<String>,
}

/// One folder of the extensions folder: its manifest, or why it can't be
/// used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The folder's name.
    pub folder: String,
    pub manifest: Result<Manifest, String>,
}

impl Found {
    /// The id when it is usable, else the folder's name.
    pub fn key(&self) -> &str {
        self.manifest.as_ref().map_or(self.folder.as_str(), |m| m.id.as_str())
    }
}

/// Why `id` can't be an extension's id (`what`: "id" or "icon").
fn check_word(what: &str, id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err(format!("\"{what}\" is empty"));
    }
    if id.len() > MAX_ID {
        return Err(format!("\"{what}\" is longer than {MAX_ID} characters"));
    }
    let ok = id.starts_with(|c: char| c.is_ascii_lowercase())
        && !id.ends_with('-')
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(format!("\"{what}\" “{id}” must be lower-case letters, digits and -, starting with a letter"));
    }
    Ok(())
}

/// Whether `id` is a usable extension id.
pub fn check_id(id: &str) -> Result<(), String> {
    check_word("id", id)
}

/// One line of text, trimmed, at most `max` characters.
fn line(what: &str, v: &str, max: usize) -> Result<String, String> {
    let t = v.trim();
    if t.is_empty() {
        return Err(format!("\"{what}\" is empty"));
    }
    if t.chars().any(char::is_control) {
        return Err(format!("\"{what}\" must be one line"));
    }
    if t.chars().count() > max {
        return Err(format!("\"{what}\" is longer than {max} characters"));
    }
    Ok(t.to_string())
}

/// An optional string field.
fn opt_str<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("\"{key}\" must be a string")),
    }
}

/// Read and check manifest `text`.
pub fn parse(text: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{MANIFEST} isn't valid JSON: {e}"))?;
    let Value::Object(obj) = v else {
        return Err(format!("{MANIFEST} must be a JSON object"));
    };
    match obj.get("v") {
        None => {}
        Some(n) => match n.as_u64() {
            Some(n) if n == FORMAT => {}
            Some(n) if n > FORMAT => return Err(format!("it needs a newer Pitwall (manifest format {n}; this one reads {FORMAT})")),
            _ => return Err(format!("\"v\" must be {FORMAT}")),
        },
    }
    let id = opt_str(&obj, "id")?.ok_or("\"id\" is missing")?;
    check_id(id)?;
    let name = line("name", opt_str(&obj, "name")?.ok_or("\"name\" is missing")?, MAX_NAME)?;
    let icon = match opt_str(&obj, "icon")? {
        Some(i) => {
            check_word("icon", i)?;
            Some(i.to_string())
        }
        None => None,
    };
    let description = opt_str(&obj, "description")?.map(|d| line("description", d, MAX_DESCRIPTION)).transpose()?;
    Ok(Manifest { id: id.to_string(), name, icon, description })
}

/// Read and check the manifest in folder `dir` (named `folder`): its id
/// must be the folder's name.
pub fn read(dir: &Path, folder: &str) -> Result<Manifest, String> {
    let file = dir.join(MANIFEST);
    let meta = std::fs::metadata(&file).map_err(|_| format!("there is no {MANIFEST} in it"))?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{MANIFEST} is bigger than {} KiB", MAX_BYTES / 1024));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{MANIFEST} can't be read: {e}"))?;
    let m = parse(&text)?;
    if m.id != folder {
        return Err(format!("its folder must be named after its id (“{}”), not “{folder}”", m.id));
    }
    Ok(m)
}

/// The extensions in `extensions_dir` (`Paths::extensions_dir`), by folder
/// name: one entry per folder (hidden ones and files are skipped), usable
/// or not. None when the folder isn't there.
pub fn discover(extensions_dir: &Path) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return Vec::new();
    };
    let mut found: Vec<Found> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()) && e.path().is_dir())
        .filter_map(|e| {
            let folder = e.file_name().to_str()?.to_string();
            if folder.starts_with('.') {
                return None;
            }
            let manifest = read(&e.path(), &folder);
            Some(Found { folder, manifest })
        })
        .collect();
    found.sort_by(|a, b| a.folder.cmp(&b.folder));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_is_read_and_checked() {
        let m = parse(r#"{"v": 1, "id": "inbox", "name": " Inbox ", "icon": "inbox", "description": "Questions from agents", "panels": [{"later": true}]}"#).unwrap();
        assert_eq!(m, Manifest { id: "inbox".into(), name: "Inbox".into(), icon: Some("inbox".into()), description: Some("Questions from agents".into()) });
        // `v` is optional; icon and description too; unknown keys are ignored.
        let m = parse(r#"{"id": "a-2", "name": "A", "badge": 3}"#).unwrap();
        assert_eq!((m.icon, m.description), (None, None));
        assert!(parse(r#"{"id": "a", "name": "A", "icon": null}"#).is_ok());
    }

    #[test]
    fn bad_ids_are_refused() {
        for id in ["", "Inbox", "2fa", "-a", "a-", "a_b", "a.b", "a b", "ä", "../x", &"a".repeat(41)] {
            let text = serde_json::json!({"id": id, "name": "X"}).to_string();
            assert!(parse(&text).unwrap_err().contains("\"id\""), "{id:?}");
        }
        assert!(check_id(&"a".repeat(40)).is_ok());
        assert!(parse(r#"{"id": 3, "name": "X"}"#).unwrap_err().contains("must be a string"));
    }

    #[test]
    fn missing_and_bad_fields_are_refused() {
        let err = |t: &str| parse(t).unwrap_err();
        assert!(err("not json").contains("isn't valid JSON"));
        assert!(err("[]").contains("JSON object"));
        assert!(err(r#"{"name": "X"}"#).contains("\"id\" is missing"));
        assert!(err(r#"{"id": "x"}"#).contains("\"name\" is missing"));
        assert!(err(r#"{"id": "x", "name": "  "}"#).contains("\"name\" is empty"));
        assert!(err(r#"{"id": "x", "name": "a\nb"}"#).contains("one line"));
        assert!(err(&serde_json::json!({"id": "x", "name": "n".repeat(41)}).to_string()).contains("longer than 40"));
        assert!(err(r#"{"id": "x", "name": "X", "icon": "Big Icon"}"#).contains("\"icon\""));
        assert!(err(r#"{"id": "x", "name": "X", "icon": 1}"#).contains("\"icon\" must be a string"));
        assert!(err(r#"{"id": "x", "name": "X", "description": ""}"#).contains("\"description\" is empty"));
        assert!(err(r#"{"v": 2, "id": "x", "name": "X"}"#).contains("needs a newer Pitwall"));
        assert!(err(r#"{"v": 0, "id": "x", "name": "X"}"#).contains("\"v\" must be 1"));
        assert!(err(r#"{"v": "1", "id": "x", "name": "X"}"#).contains("\"v\" must be 1"));
    }

    /// The examples in the repository's `extensions/` folder are usable.
    #[test]
    fn the_example_extensions_are_usable() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
        let found = discover(&dir);
        assert!(!found.is_empty());
        for f in found {
            assert!(f.manifest.is_ok(), "{}: {:?}", f.folder, f.manifest);
        }
    }

    #[test]
    fn extensions_are_found_in_their_folders() {
        let t = crate::testing::TempDir::new("extensions");
        let dir = t.path().join("extensions");
        assert!(discover(&dir).is_empty(), "no folder: none");
        let put = |folder: &str, text: Option<&str>| {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
            if let Some(text) = text {
                std::fs::write(dir.join(folder).join(MANIFEST), text).unwrap();
            }
        };
        put("zed", Some(r#"{"id": "zed", "name": "Zed"}"#));
        put("inbox", Some(r#"{"id": "inbox", "name": "Inbox", "icon": "inbox"}"#));
        put("renamed", Some(r#"{"id": "other", "name": "Other"}"#));
        put("empty", None);
        put(".hidden", Some(r#"{"id": "hidden", "name": "Hidden"}"#));
        std::fs::write(dir.join("stray.json"), "{}").unwrap();
        put("broken", Some("{"));
        let found = discover(&dir);
        let folders: Vec<&str> = found.iter().map(|f| f.folder.as_str()).collect();
        assert_eq!(folders, ["broken", "empty", "inbox", "renamed", "zed"]);
        assert!(found[0].manifest.as_ref().unwrap_err().contains("isn't valid JSON"));
        assert!(found[1].manifest.as_ref().unwrap_err().contains("no extension.json"));
        assert_eq!(found[2].manifest.as_ref().unwrap().name, "Inbox");
        assert!(found[3].manifest.as_ref().unwrap_err().contains("named after its id"));
        assert_eq!(found[3].key(), "renamed");
        assert_eq!(found[4].key(), "zed");
    }
}
