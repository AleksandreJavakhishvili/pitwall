//! "Show ignored files" kept across launches (the React app's
//! `pitwall.explorer.showIgnored` localStorage key). Stored in the data
//! folder's `prefs.json` (docs/spec/gpui/README.md "State model"), one key
//! among others: read-modify-write keeps the rest, written atomically.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const SHOW_IGNORED: &str = "explorer.showIgnored";

fn file(root: &Path) -> PathBuf {
    root.join("prefs.json")
}

fn read_all(root: &Path) -> Map<String, Value> {
    std::fs::read(file(root))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn get_bool(root: &Path, key: &str) -> Option<bool> {
    read_all(root).get(key).and_then(Value::as_bool)
}

pub fn set_bool(root: &Path, key: &str, value: bool) -> std::io::Result<()> {
    let mut all = read_all(root);
    all.insert(key.into(), Value::Bool(value));
    let path = file(root);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&Value::Object(all))?)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_other_keys() {
        let dir = std::env::temp_dir().join(format!("pw-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file(&dir), r#"{"other": 1}"#).unwrap();
        assert_eq!(get_bool(&dir, SHOW_IGNORED), None);
        set_bool(&dir, SHOW_IGNORED, true).unwrap();
        assert_eq!(get_bool(&dir, SHOW_IGNORED), Some(true));
        assert_eq!(read_all(&dir)["other"], 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
