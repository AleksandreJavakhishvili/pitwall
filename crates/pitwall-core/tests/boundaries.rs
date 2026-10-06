//! Architecture rules for this crate, checked on every `cargo test`
//! (architecture.md §1 and §9 decision 7):
//! - no UI framework: neither this crate nor its path dependencies depend on
//!   Tauri;
//! - OS-specific code only in `src/platform/`.

use std::path::{Path, PathBuf};

fn manifest(dir: &Path) -> toml::Table {
    let src = std::fs::read_to_string(dir.join("Cargo.toml")).expect("read Cargo.toml");
    src.parse().expect("valid Cargo.toml")
}

/// Every dependency table of a manifest (plain, dev, build, per-target).
fn dependency_tables(m: &toml::Table) -> Vec<&toml::Table> {
    let mut out = Vec::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        out.extend(m.get(key).and_then(|v| v.as_table()));
    }
    if let Some(targets) = m.get("target").and_then(|t| t.as_table()) {
        for t in targets.values().filter_map(|t| t.as_table()) {
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                out.extend(t.get(key).and_then(|v| v.as_table()));
            }
        }
    }
    out
}

#[test]
fn no_tauri_in_core_or_its_path_dependencies() {
    let mut todo = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))];
    let mut seen = Vec::new();
    while let Some(dir) = todo.pop() {
        let dir = dir.canonicalize().expect("crate dir");
        if seen.contains(&dir) {
            continue;
        }
        let m = manifest(&dir);
        for table in dependency_tables(&m) {
            for (name, spec) in table {
                let package = spec.get("package").and_then(|p| p.as_str()).unwrap_or(name);
                assert!(
                    !package.starts_with("tauri"),
                    "{} depends on {package}: the core must stay free of Tauri",
                    dir.display()
                );
                if let Some(path) = spec.get("path").and_then(|p| p.as_str()) {
                    todo.push(dir.join(path));
                }
            }
        }
        seen.push(dir);
    }
    assert!(seen.len() >= 3, "checked core, pitwall-detect and pitwall-hold");
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn os_specific_code_stays_in_platform() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let platform = src.join("platform");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let banned = ["tauri", "std::os::", "cfg(unix", "cfg(windows", "cfg(target_os", "cfg!(target_os"];
    let mut offenders = Vec::new();
    for file in files.iter().filter(|f| !f.starts_with(&platform)) {
        let text = std::fs::read_to_string(file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if let Some(word) = banned.iter().find(|w| code.contains(*w)) {
                offenders.push(format!("{}:{}: {word}", file.strip_prefix(&src).unwrap().display(), n + 1));
            }
        }
    }
    assert!(offenders.is_empty(), "move these behind src/platform/:\n{}", offenders.join("\n"));
}
