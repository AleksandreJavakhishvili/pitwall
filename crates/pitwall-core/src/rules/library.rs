//! The rule library: Pitwall's own rulesync input root (~/.pitwall/rules) plus
//! imported sources (a project's `.rulesync/`, a git clone). Files are read
//! for display only; rulesync does the real parsing when generating.

use std::path::{Path, PathBuf};

use super::runner::{self, Runner};
use super::{Dirs, RuleFile, RuleSource};
use crate::exec::{Cmd, Exec, LocalExec};

pub const LIBRARY: &str = "library";

/// Frontmatter fields Pitwall shows. A forgiving reader for the subset of
/// YAML rulesync writes (scalars, inline `[a, b]` lists and `- a` lists).
#[derive(Debug, Default, PartialEq)]
pub struct Front {
    pub description: Option<String>,
    pub targets: Vec<String>,
    pub root: bool,
    pub local_root: bool,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.split(" #").next().unwrap_or(s).trim();
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// The frontmatter block (between the leading `---` lines), if any.
pub fn front_block(src: &str) -> Option<(&str, usize)> {
    let rest = src.strip_prefix("---\n").or_else(|| src.strip_prefix("---\r\n"))?;
    let head = src.len() - rest.len();
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..offset], head + offset + line.len()));
        }
        offset += line.len();
    }
    None
}

pub fn parse_front(src: &str) -> Front {
    let mut f = Front::default();
    let Some((block, _)) = front_block(src) else { return f };
    let mut in_targets = false;
    for line in block.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let t = line.trim();
        if in_targets && indented {
            if let Some(item) = t.strip_prefix("- ") {
                f.targets.push(unquote(item));
            }
            continue;
        }
        in_targets = false;
        if indented {
            continue; // nested tool-specific blocks
        }
        let Some((key, value)) = t.split_once(':') else { continue };
        let value = value.trim();
        match key.trim() {
            "description" if !value.is_empty() => f.description = Some(unquote(value)),
            "root" => f.root = unquote(value) == "true",
            "localRoot" => f.local_root = unquote(value) == "true",
            "targets" => {
                if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                    f.targets = inner.split(',').map(unquote).filter(|s| !s.is_empty()).collect();
                } else if value.is_empty() {
                    in_targets = true;
                } else {
                    f.targets = vec![unquote(value)];
                }
            }
            _ => {}
        }
    }
    f
}

fn md_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 4 {
            md_files(&p, out, depth + 1);
        } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
            out.push(p);
        }
    }
}

/// Every rule in one input root (`<root>/rules/**/*.md`).
pub fn rules_in(source: &str, root: &Path) -> Vec<RuleFile> {
    let rules_dir = root.join("rules");
    let mut files = Vec::new();
    md_files(&rules_dir, &mut files, 0);
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(&rules_dir).ok()?.to_string_lossy().into_owned();
            let src = std::fs::read_to_string(&p).unwrap_or_default();
            let f = parse_front(&src);
            Some(RuleFile {
                id: format!("{source}:{rel}"),
                path: p.to_string_lossy().into_owned(),
                description: f.description,
                targets: if f.targets.is_empty() { vec!["*".into()] } else { f.targets },
                root: f.root,
                local_root: f.local_root,
                source: source.to_string(),
            })
        })
        .collect()
}

pub fn list(dirs: &Dirs, sources: &[RuleSource]) -> Vec<RuleFile> {
    let _ = std::fs::create_dir_all(dirs.library_rules());
    let mut all = rules_in(LIBRARY, &dirs.library());
    for s in sources {
        all.extend(rules_in(&s.name, Path::new(&s.root)));
    }
    all
}

/// A short unique name for a new source.
fn source_name(hint: &str, taken: &[RuleSource]) -> String {
    let base = crate::slug::slug(hint);
    let base = if base == LIBRARY { format!("{base}-src") } else { base };
    (1..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
        .find(|c| !taken.iter().any(|s| &s.name == c))
        .expect("unbounded")
}

/// The rulesync source tree inside a folder: `<dir>/.rulesync` or `<dir>` itself.
fn find_root(dir: &Path) -> Option<PathBuf> {
    [dir.join(".rulesync"), dir.to_path_buf()]
        .into_iter()
        .find(|r| r.join("rules").is_dir())
}

/// A project's own `.rulesync/`, referenced in place (read-only).
pub fn add_project(project: &str, sources: &[RuleSource]) -> Result<RuleSource, String> {
    let dir = Path::new(project);
    let root = dir.join(".rulesync");
    if !root.join("rules").is_dir() {
        return Err(format!("{} has no .rulesync/rules folder", dir.display()));
    }
    if let Some(s) = sources.iter().find(|s| Path::new(&s.root) == root) {
        return Err(format!("already imported as \"{}\"", s.name));
    }
    let hint = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(RuleSource {
        name: source_name(&hint, sources),
        kind: "project".into(),
        origin: project.to_string(),
        root: root.to_string_lossy().into_owned(),
    })
}

/// Git for the library's own clones: always this Mac, where the library is.
fn git(args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let dir = cwd.map(|c| c.to_string_lossy().into_owned());
    let mut argv = vec!["git"];
    if let Some(d) = &dir {
        argv.extend(["-C", d.as_str()]);
    }
    argv.extend_from_slice(args);
    let out = LocalExec.run(&Cmd::new(&argv).env(&[("GIT_TERMINAL_PROMPT", "0")]))?;
    if out.ok() {
        Ok(out.stdout_text())
    } else {
        Err(out.stderr_text())
    }
}

/// Clone a git repo into ~/.pitwall/rules-sources/<name>.
pub fn add_git(dirs: &Dirs, url: &str, sources: &[RuleSource]) -> Result<RuleSource, String> {
    let url = url.trim();
    if url.is_empty() || url.starts_with('-') {
        return Err("enter a git URL".into());
    }
    let hint = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("rules");
    let name = source_name(hint.trim_end_matches(".git"), sources);
    let dest = dirs.sources().join(&name);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    std::fs::create_dir_all(dirs.sources()).map_err(|e| e.to_string())?;
    let dest_s = dest.to_string_lossy().into_owned();
    git(&["clone", "--depth", "1", "--", url, &dest_s], None).map_err(|e| format!("git clone failed: {e}"))?;
    let Some(root) = find_root(&dest) else {
        let _ = std::fs::remove_dir_all(&dest); // our own fresh clone
        return Err("that repository has no rulesync rules (.rulesync/rules or rules/)".into());
    };
    Ok(RuleSource {
        name,
        kind: "git".into(),
        origin: url.to_string(),
        root: root.to_string_lossy().into_owned(),
    })
}

pub fn pull(dirs: &Dirs, source: &RuleSource) -> Result<String, String> {
    if source.kind != "git" {
        return Ok(format!("{} is read in place; nothing to pull", source.name));
    }
    let dir = dirs.sources().join(&source.name);
    git(&["pull", "--ff-only"], Some(&dir)).map_err(|e| format!("git pull failed: {e}"))
}

/// Forget a source; a git clone Pitwall made is deleted too.
pub fn remove(dirs: &Dirs, source: &RuleSource) -> Result<(), String> {
    if source.kind == "git" {
        let dir = dirs.sources().join(&source.name);
        if dir.starts_with(dirs.sources()) && dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The rulesync target that reads a given instruction file back in.
fn import_target(file_name: &str) -> Option<(&'static str, &'static str)> {
    match file_name {
        "CLAUDE.md" | "CLAUDE.local.md" => Some(("claudecode", "CLAUDE.md")),
        "AGENTS.md" => Some(("agentsmd", "AGENTS.md")),
        "GEMINI.md" => Some(("geminicli", "GEMINI.md")),
        _ => None,
    }
}

/// Import a CLAUDE.md / AGENTS.md / GEMINI.md with `rulesync import`, run in a
/// scratch folder holding a copy, so the user's project is never touched.
/// The resulting rules are added to the library. Returns new file names.
pub fn import_file(dirs: &Dirs, runner: &Runner, file: &str) -> Result<(Vec<String>, String), String> {
    let src = Path::new(file);
    let fname = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (target, canonical) = import_target(&fname)
        .ok_or_else(|| format!("{fname} is not supported; import a CLAUDE.md, AGENTS.md or GEMINI.md"))?;
    if !src.is_file() {
        return Err(format!("{file} is not a file"));
    }
    let scratch = dirs.run().join(format!("import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let result = (|| {
        std::fs::copy(src, scratch.join(canonical)).map_err(|e| e.to_string())?;
        let args: Vec<String> = ["import", "--targets", target, "--features", "rules"].map(String::from).into();
        let doc = runner.run_json(&scratch, &args)?;
        let mut log = runner::warnings(&doc).join("\n");
        let hint = src
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| crate::slug::slug(&n.to_string_lossy()))
            .unwrap_or_else(|| "imported".into());
        let mut produced = Vec::new();
        md_files(&scratch.join(".rulesync").join("rules"), &mut produced, 0);
        produced.sort();
        std::fs::create_dir_all(dirs.library_rules()).map_err(|e| e.to_string())?;
        let mut added = Vec::new();
        for (i, p) in produced.iter().enumerate() {
            let base = if i == 0 { hint.clone() } else { format!("{hint}-{i}") };
            let name = (1..)
                .map(|n| if n == 1 { format!("{base}.md") } else { format!("{base}-{n}.md") })
                .find(|n| !dirs.library_rules().join(n).exists())
                .expect("unbounded");
            std::fs::copy(p, dirs.library_rules().join(&name)).map_err(|e| e.to_string())?;
            added.push(name);
        }
        if added.is_empty() {
            log.push_str("\nrulesync found no rules in that file.");
        }
        Ok((added, log.trim().to_string()))
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_inline_and_block_lists() {
        let f = parse_front("---\nroot: true\ntargets: [\"claudecode\", 'codexcli']\ndescription: \"Style: tabs\"\n---\nbody");
        assert_eq!(
            f,
            Front {
                description: Some("Style: tabs".into()),
                targets: vec!["claudecode".into(), "codexcli".into()],
                root: true,
                local_root: false
            }
        );
        let f = parse_front("---\nroot: false\nlocalRoot: true\ntargets:\n  - '*'\nglobs:\n  - '**/*'\ncursor:\n  description: nested\n---\n");
        assert_eq!(f.targets, vec!["*"]);
        assert!(f.local_root && !f.root);
        assert_eq!(f.description, None);
        assert_eq!(parse_front("no frontmatter"), Front::default());
    }

    #[test]
    fn front_block_offsets() {
        let src = "---\na: 1\n---\nbody\n";
        let (block, end) = front_block(src).unwrap();
        assert_eq!(block, "a: 1\n");
        assert_eq!(&src[end..], "body\n");
        assert!(front_block("---\nunterminated").is_none());
    }
}
