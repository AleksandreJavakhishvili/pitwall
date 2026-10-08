//! Architecture rules for this crate, checked on every `cargo test`
//! (architecture.md §1 and §9 decision 7):
//! - no UI framework: neither this crate nor its path dependencies depend on
//!   Tauri;
//! - OS-specific code only in `src/platform/`;
//! - processes are started only in `src/platform/` and by `LocalExec`
//!   (`src/exec/local.rs`); everything else runs programs through an `Exec`
//!   (§2.4), and git/Review/worktree code reaches files only through it;
//! - no terminal holders or places to run in core (those are providers), and
//!   the engine never asks which provider an agent has (§3).

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
                assert!(
                    !["pitwall-hold", "pitwall-providers", "portable-pty"].contains(&package),
                    "{} depends on {package}: terminals and places to run belong to providers (architecture.md §1)",
                    dir.display()
                );
                if let Some(path) = spec.get("path").and_then(|p| p.as_str()) {
                    todo.push(dir.join(path));
                }
            }
        }
        seen.push(dir);
    }
    assert!(seen.len() >= 2, "checked core and pitwall-detect");
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

/// Production code of a source file: test modules (`#[cfg(test)] mod …` to
/// the end, by this crate's convention) and comments removed.
fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((n, line)) = lines.next() {
        if line.trim() == "#[cfg(test)]" && lines.peek().is_some_and(|(_, next)| {
                next.trim_start().trim_start_matches("pub(crate) ").trim_start_matches("pub ").starts_with("mod ")
            }) {
            break;
        }
        out.push((n, line.split("//").next().unwrap_or("")));
    }
    out
}

fn offenders(files: &[PathBuf], src: &Path, banned: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in files {
        if file.file_name().is_some_and(|n| n == "tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("read source");
        for (n, code) in production_lines(&text) {
            if let Some(word) = banned.iter().find(|w| code.contains(*w)) {
                found.push(format!("{}:{}: {word}", file.strip_prefix(src).unwrap().display(), n + 1));
            }
        }
    }
    found
}

#[test]
fn processes_start_only_in_platform_and_local_exec() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowed = [src.join("platform"), src.join("exec").join("local.rs")];
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.retain(|f| !allowed.iter().any(|a| f.starts_with(a)));
    let banned = ["Command::new", "process::Command", "process::{", "Stdio", "process::Child"];
    let found = offenders(&files, &src, &banned);
    assert!(found.is_empty(), "run these through an Exec (LocalExec for this machine):\n{}", found.join("\n"));
}

#[test]
fn git_review_and_worktrees_reach_files_only_through_exec() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src.join("vcs"), &mut files);
    for f in [
        "review.rs",
        "engine/worktree.rs",
        "engine/tasks.rs",
        "engine/changes.rs",
        "engine/ticker.rs",
        "explorer/mod.rs",
        "explorer/tree.rs",
        "explorer/read.rs",
        "explorer/search.rs",
    ] {
        files.push(src.join(f));
    }
    let found = offenders(&files, &src, &["std::fs", "fs::", "File::", ".is_dir()", ".exists()", "canonicalize"]);
    assert!(found.is_empty(), "use the agent's Exec for these:\n{}", found.join("\n"));
}

/// The engine branches on capabilities, never on which provider or kind an
/// agent has (architecture.md §3).
#[test]
fn the_engine_never_asks_which_provider() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src.join("engine"), &mut files);
    let found = offenders(&files, &src, &["ProviderId::LOCAL", "ProviderId::local", "\"local\"", "\"agw\"", "\"ssh", "THIS_MAC", "pitwall_hold"]);
    assert!(found.is_empty(), "branch on ProviderCaps instead:\n{}", found.join("\n"));
}
