//! The guest SDK (sdk/pitwall-extension) carries its own copy of `wit/`, so
//! the crate is complete when published. The copy must be this crate's
//! WIT, file for file.

use std::collections::BTreeMap;
use std::path::Path;

fn files(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "wit"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap().replace("\r\n", "\n")))
        .collect()
}

#[test]
fn the_sdks_wit_is_the_hosts() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let host = files(&here.join("wit"));
    let sdk = files(&here.join("../../sdk/pitwall-extension/wit"));
    assert!(!host.is_empty());
    assert_eq!(host.keys().collect::<Vec<_>>(), sdk.keys().collect::<Vec<_>>(), "the same files");
    for (name, text) in &host {
        assert!(sdk[name] == *text, "sdk/pitwall-extension/wit/{name} differs from crates/pitwall-tile-host/wit/{name}: copy it over");
    }
}
