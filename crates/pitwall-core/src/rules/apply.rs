//! Applying rules to one agent's working directory.
//!
//! rulesync generates into an empty scratch output root (cwd = a Pitwall-owned
//! staging folder, so a project's own rulesync.jsonc — e.g. `delete: true` —
//! never applies). Input roots are the staged selection of library rules,
//! then the project's own `.rulesync/` (if any) so project rules win on a name
//! clash. Pitwall then copies the generated files into the working directory,
//! never over a git-tracked file or a file it didn't write, and lists every
//! copied path in the repo's `.git/info/exclude`.
//!
//! rulesync and its staging folder are on this Mac; everything in the agent's
//! working directory (git, reading, writing and removing files, the exclude
//! file) goes through the agent's machine [`Exec`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::exec::{self, Exec, FileKind};
use crate::vcs::git::Git;

use super::library::{self, front_block};
use super::runner::{self, Runner};
use super::{GenFile, RuleFile};

/// One rule file as handed to rulesync.
#[derive(Debug, Clone, PartialEq)]
pub struct Staged {
    pub name: String,
    pub content: Vec<u8>,
}

pub struct Inputs {
    pub staged: Vec<Staged>,
    pub missing: Vec<String>,
    pub notes: Vec<String>,
    /// The working directory's own rulesync source tree, if it has one.
    pub project_root: Option<PathBuf>,
}

fn add_local_root(src: &str) -> String {
    match front_block(src) {
        Some((block, end)) => {
            let body = &src[end..];
            let kept: String = block
                .split_inclusive('\n')
                .filter(|l| !l.trim_start().starts_with("localRoot:") || l.starts_with([' ', '\t']))
                .collect();
            format!("---\nlocalRoot: true\n{kept}---\n{body}")
        }
        None => format!("---\nlocalRoot: true\n---\n{src}"),
    }
}

fn applies_to(targets: &[String], target: &str) -> bool {
    targets.is_empty() || targets.iter().any(|t| t == "*" || t == target)
}

/// Collect the rule files for an agent: `shared` ids (project default set)
/// and `extra` ids (the agent's own set, written to local-only files).
pub fn collect(library: &[RuleFile], shared: &[String], extra: &[String], target: &str, cwd: &Path) -> Inputs {
    let mut staged: Vec<Staged> = Vec::new();
    let mut missing = Vec::new();
    let mut notes = Vec::new();
    let project_root = Some(cwd.join(".rulesync")).filter(|r| r.join("rules").is_dir());

    let project_has_root = project_root.as_ref().is_some_and(|r| {
        library::rules_in("project", r).iter().any(|f| f.root && applies_to(&f.targets, target))
    });
    let lookup = |id: &String| library.iter().find(|f| &f.id == id);
    let has_root = project_has_root
        || shared.iter().chain(extra).filter_map(lookup).any(|f| f.root && applies_to(&f.targets, target));

    let mut seen = BTreeSet::new();
    let mut taken = BTreeSet::new();
    for (id, local) in shared.iter().map(|i| (i, false)).chain(extra.iter().map(|i| (i, true))) {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(file) = lookup(id) else {
            missing.push(id.clone());
            continue;
        };
        let Ok(mut src) = std::fs::read_to_string(&file.path) else {
            missing.push(id.clone());
            continue;
        };
        if local && !file.root && !file.local_root {
            if has_root {
                src = add_local_root(&src);
            } else if notes.is_empty() {
                notes.push(
                    "No root rule for this agent, so its own rules go to regular rule files (still local-only via .git/info/exclude)."
                        .to_string(),
                );
            }
        }
        // rulesync merges by file name (case-insensitively): keep names unique.
        let base = Path::new(&file.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "rule.md".into());
        let mut name = base.clone();
        if taken.contains(&name.to_lowercase()) {
            name = format!("{}-{}", crate::slug::slug(&file.source), base);
        }
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            name = format!("{n}-{base}");
            n += 1;
        }
        taken.insert(name.to_lowercase());
        staged.push(Staged { name, content: src.into_bytes() });
    }
    Inputs { staged, missing, notes, project_root }
}

/// Stable FNV-1a over everything that decides the generated output.
pub fn fingerprint(inputs: &Inputs, target: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes.iter().chain(std::iter::once(&0xffu8)) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    feed(target.as_bytes());
    for s in &inputs.staged {
        feed(s.name.as_bytes());
        feed(&s.content);
    }
    if let Some(root) = &inputs.project_root {
        for f in library::rules_in("project", root) {
            feed(f.id.as_bytes());
            feed(&std::fs::read(&f.path).unwrap_or_default());
        }
    }
    format!("{h:016x}")
}

pub fn content_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn git(exec: &dyn Exec, cwd: &str, args: &[&str]) -> Option<String> {
    Git::new(exec, cwd).run(args).ok()
}

/// Which of `paths` (relative to `cwd`) git tracks.
fn tracked(exec: &dyn Exec, cwd: &str, paths: &[String]) -> BTreeSet<String> {
    if paths.is_empty() {
        return BTreeSet::new();
    }
    let mut args = vec!["ls-files", "-z", "--"];
    args.extend(paths.iter().map(String::as_str));
    git(exec, cwd, &args)
        .map(|s| s.split('\0').filter(|p| !p.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// Location of the info/exclude file and the path prefix of `cwd` inside the
/// repo, or `None` outside git.
pub fn exclude_target(exec: &dyn Exec, cwd: &str) -> Option<(String, String)> {
    let file = git(exec, cwd, &["rev-parse", "--git-path", "info/exclude"])?.trim().to_string();
    let file = if Path::new(&file).is_absolute() { file } else { exec::join(cwd, &file) };
    let prefix = git(exec, cwd, &["rev-parse", "--show-prefix"])?.trim().to_string();
    Some((file, prefix))
}

/// The whole file as text (empty when missing or unreadable).
fn read_text(exec: &dyn Exec, path: &str) -> String {
    exec.read_file(path, u64::MAX)
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

fn begin(agent_id: &str) -> String {
    format!("# >>> pitwall rules {agent_id} (generated, do not edit)")
}
fn end(agent_id: &str) -> String {
    format!("# <<< pitwall rules {agent_id}")
}

/// Escape gitignore metacharacters so a path matches only itself.
fn exclude_pattern(path: &str) -> String {
    let mut out = String::from("/");
    for c in path.chars() {
        if "\\*?[]!# ".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Replace this agent's block in an exclude file (removed when `paths` is empty).
pub fn write_exclude(exec: &dyn Exec, file: &str, agent_id: &str, paths: &[String]) -> Result<(), String> {
    let old = read_text(exec, file);
    let (b, e) = (begin(agent_id), end(agent_id));
    let mut out = String::new();
    let mut skipping = false;
    for line in old.lines() {
        if line == b {
            skipping = true;
        } else if skipping && line == e {
            skipping = false;
        } else if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !paths.is_empty() {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&b);
        out.push('\n');
        for p in paths {
            out.push_str(&exclude_pattern(p));
            out.push('\n');
        }
        out.push_str(&e);
        out.push('\n');
    }
    if out == old {
        return Ok(());
    }
    exec.write_file(file, out.as_bytes()).map_err(|e| format!("could not update {file}: {e}"))
}

fn safe_rel(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && !p.split('/').any(|c| c == ".." || c.is_empty() || c == ".git")
}

/// Delete files Pitwall generated earlier, unless the user changed them or
/// git tracks them now.
pub fn remove_generated(exec: &dyn Exec, cwd: &str, files: &[GenFile], log: &mut Vec<String>) {
    let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    let tracked = tracked(exec, cwd, &paths);
    for f in files {
        if !safe_rel(&f.path) || tracked.contains(&f.path) {
            continue;
        }
        let full = exec::join(cwd, &f.path);
        match exec.read_file(&full, u64::MAX) {
            Ok(Some(bytes)) if content_hash(&bytes) == f.hash => {
                let _ = exec.remove_file(&full);
                // Tidy directories this left empty (e.g. .claude/rules).
                let mut rel = Path::new(&f.path).parent();
                while let Some(d) = rel.filter(|d| !d.as_os_str().is_empty()) {
                    if exec.remove_dir(&exec::join(cwd, &d.to_string_lossy())).is_err() {
                        break;
                    }
                    rel = d.parent();
                }
            }
            Ok(Some(_)) => log.push(format!("kept {} (edited since Pitwall wrote it)", f.path)),
            _ => {}
        }
    }
}

pub struct Outcome {
    pub files: Vec<GenFile>,
    pub log: Vec<String>,
    pub exclude_file: Option<String>,
}

/// Run rulesync (here, in `stage`) and place its output in `cwd` on the
/// agent's machine.
#[allow(clippy::too_many_arguments)]
pub fn run(
    runner: &Runner,
    stage: &Path,
    exec: &dyn Exec,
    agent_id: &str,
    cwd: &str,
    target: &str,
    inputs: &Inputs,
    previous: &[GenFile],
) -> Result<Outcome, String> {
    let mut log: Vec<String> = Vec::new();
    for id in &inputs.missing {
        log.push(format!("rule {id} is no longer in the library; skipped"));
    }
    log.extend(inputs.notes.iter().cloned());

    // Fresh staging folder (Pitwall-owned).
    if stage.exists() {
        std::fs::remove_dir_all(stage).map_err(|e| e.to_string())?;
    }
    let input = stage.join("input");
    let out = stage.join("out");
    std::fs::create_dir_all(input.join("rules")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    std::fs::write(stage.join("rulesync.jsonc"), "{ \"delete\": false }\n").map_err(|e| e.to_string())?;
    for s in &inputs.staged {
        std::fs::write(input.join("rules").join(&s.name), &s.content).map_err(|e| e.to_string())?;
    }
    if out.to_string_lossy().contains(',') {
        return Err("Pitwall's folder path contains a comma, which rulesync can't take as an output root".into());
    }

    let mut args: Vec<String> = vec![
        "generate".into(),
        "--targets".into(),
        target.into(),
        "--features".into(),
        "rules".into(),
        "--output-roots".into(),
        out.to_string_lossy().into_owned(),
        "--input-roots".into(),
        input.to_string_lossy().into_owned(),
    ];
    if let Some(root) = &inputs.project_root {
        args.push(root.to_string_lossy().into_owned());
        log.push(format!("also using the project's own {}", root.display()));
    }
    let doc = runner.run_json(stage, &args)?;
    log.extend(runner::warnings(&doc).into_iter().map(|w| format!("rulesync: {w}")));

    let ops = doc
        .pointer("/data/plan/operations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let written: Vec<String> = ops
        .iter()
        .filter(|o| o["action"] == "write" && o["kind"] == "file")
        .filter_map(|o| o["path"].as_str().map(String::from))
        .collect();

    let tracked = tracked(exec, cwd, &written);
    let mut files = Vec::new();
    for rel in &written {
        if !safe_rel(rel) {
            log.push(format!("skipped unexpected path {rel}"));
            continue;
        }
        let Ok(bytes) = std::fs::read(out.join(rel)) else { continue };
        let dest = exec::join(cwd, rel);
        if tracked.contains(rel) {
            log.push(format!("kept the project's {rel} (tracked by git; Pitwall never changes tracked files)"));
            continue;
        }
        if let Ok(Some(st)) = exec.stat(&dest) {
            let ours = previous.iter().find(|g| &g.path == rel);
            let unchanged = st.kind == FileKind::File
                && ours.is_some_and(|g| {
                    exec.read_file(&dest, u64::MAX).ok().flatten().is_some_and(|b| content_hash(&b) == g.hash)
                });
            if !unchanged {
                log.push(format!("kept the existing {rel} (not written by Pitwall)"));
                continue;
            }
        }
        exec.write_file(&dest, &bytes).map_err(|e| format!("could not write {rel}: {e}"))?;
        files.push(GenFile { path: rel.clone(), hash: content_hash(&bytes) });
    }

    // Files from an earlier apply that rulesync no longer produces.
    let stale: Vec<GenFile> = previous
        .iter()
        .filter(|g| !files.iter().any(|f| f.path == g.path))
        .cloned()
        .collect();
    remove_generated(exec, cwd, &stale, &mut log);

    if files.iter().any(|f| f.path == "CLAUDE.md" || f.path == "CLAUDE.local.md")
        && exec::exists(exec, &exec::join(cwd, "AGENTS.md"))
        && !tracked.contains("CLAUDE.md")
    {
        log.push("note: with a CLAUDE.md / CLAUDE.local.md present, Claude Code no longer falls back to AGENTS.md".into());
    }

    let exclude_file = match exclude_target(exec, cwd) {
        Some((file, prefix)) => {
            let rel: Vec<String> = files.iter().map(|f| format!("{prefix}{}", f.path)).collect();
            write_exclude(exec, &file, agent_id, &rel)?;
            Some(file)
        }
        None => {
            if !files.is_empty() {
                log.push("not a git repository: generated files are not excluded from anything".into());
            }
            None
        }
    };
    Ok(Outcome { files, log, exclude_file })
}

#[cfg(test)]
mod tests {
    use super::super::library::parse_front;
    use super::*;
    use crate::exec::LocalExec;

    #[test]
    fn local_root_is_injected() {
        assert_eq!(add_local_root("body"), "---\nlocalRoot: true\n---\nbody");
        assert_eq!(
            add_local_root("---\nroot: false\nlocalRoot: false\n---\nbody\n"),
            "---\nlocalRoot: true\nroot: false\n---\nbody\n"
        );
        assert!(parse_front(&add_local_root("---\ntargets: [\"*\"]\n---\nx")).local_root);
    }

    #[test]
    fn exclude_patterns_escape() {
        assert_eq!(exclude_pattern(".claude/rules/a.md"), "/.claude/rules/a.md");
        assert_eq!(exclude_pattern("x [1]*.md"), "/x\\ \\[1\\]\\*.md");
        assert!(safe_rel(".claude/rules/a.md"));
        assert!(!safe_rel("../x") && !safe_rel("/x") && !safe_rel(".git/config") && !safe_rel("a//b"));
    }

    #[test]
    fn exclude_blocks_are_replaced_and_removed() {
        let dir = std::env::temp_dir().join(format!("pw-excl-{}", uuid::Uuid::new_v4()));
        let path = dir.join("info").join("exclude");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# user\n*.log\n").unwrap();
        let (x, file) = (&LocalExec, path.to_str().unwrap());
        write_exclude(x, file, "a1", &["CLAUDE.local.md".into()]).unwrap();
        write_exclude(x, file, "b2", &["AGENTS.md".into()]).unwrap();
        write_exclude(x, file, "a1", &[".claude/rules/x.md".into()]).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.starts_with("# user\n*.log\n"));
        assert!(s.contains("/.claude/rules/x.md") && s.contains("/AGENTS.md") && !s.contains("CLAUDE.local"));
        write_exclude(x, file, "a1", &[]).unwrap();
        write_exclude(x, file, "b2", &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim_end(), "# user\n*.log");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn collect_marks_extra_rules_local_when_a_root_exists() {
        let dir = std::env::temp_dir().join(format!("pw-collect-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, body: &str, root: bool| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            RuleFile {
                id: format!("library:{name}"),
                path: p.to_string_lossy().into_owned(),
                description: None,
                targets: vec!["*".into()],
                root,
                local_root: false,
                source: "library".into(),
            }
        };
        let lib = vec![
            mk("base.md", "---\nroot: true\n---\nbase", true),
            mk("mine.md", "---\nroot: false\n---\nmine", false),
        ];
        let ids = |v: &[&str]| v.iter().map(|s| format!("library:{s}")).collect::<Vec<_>>();
        let i = collect(&lib, &ids(&["base.md"]), &ids(&["mine.md", "gone.md"]), "claudecode", &dir);
        assert_eq!(i.staged.len(), 2);
        assert!(parse_front(std::str::from_utf8(&i.staged[1].content).unwrap()).local_root);
        assert_eq!(i.missing, vec!["library:gone.md"]);
        // Without a root rule the extra rule stays a plain rule.
        let i = collect(&lib, &[], &ids(&["mine.md"]), "claudecode", &dir);
        assert!(!parse_front(std::str::from_utf8(&i.staged[0].content).unwrap()).local_root);
        assert_eq!(i.notes.len(), 1);
        let f1 = fingerprint(&i, "claudecode");
        assert_eq!(f1, fingerprint(&i, "claudecode"));
        assert_ne!(f1, fingerprint(&i, "codexcli"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
