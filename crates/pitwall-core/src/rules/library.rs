//! The library, a catalog of rules and skills: Pitwall's own rulesync input
//! root (~/.pitwall/rules: `rules/*.md`, `skills/<name>/SKILL.md` and the
//! skill's other files) plus imported sources (a project's `.rulesync/`, a
//! git clone). Files are read for display only; rulesync does the real
//! parsing when generating.

use std::path::{Path, PathBuf};

use super::runner::{self, Runner};
use super::{Dirs, RuleFile, RuleSource};
use crate::exec::{Cmd, Exec, LocalExec};

pub const LIBRARY: &str = "library";

/// Frontmatter fields Pitwall shows. A forgiving reader for the subset of
/// YAML rulesync writes (scalars, inline `[a, b]` lists and `- a` lists).
#[derive(Debug, Default, PartialEq)]
pub struct Front {
    /// A skill's `name`.
    pub name: Option<String>,
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
    // A folded or literal `description: >-` continues on indented lines.
    let mut in_desc = false;
    for line in block.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let t = line.trim();
        if in_desc && indented {
            let joined = f.description.take().map_or_else(|| t.to_string(), |d| format!("{d} {t}"));
            f.description = Some(joined);
            continue;
        }
        in_desc = false;
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
            "description" if !value.is_empty() && !value.starts_with(['>', '|']) => f.description = Some(unquote(value)),
            "description" => in_desc = true,
            "name" if !value.is_empty() => f.name = Some(unquote(value)),
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
                skill: false,
                source: source.to_string(),
            })
        })
        .collect()
}

/// The skill name in a skill's id (`library:skills/review` → `review`).
pub fn skill_name(id: &str) -> Option<&str> {
    let rel = id.split_once(':')?.1;
    rel.strip_prefix("skills/").filter(|n| !n.is_empty() && !n.ends_with(".md") && !n.contains('/'))
}

/// Every skill in one input root (`<root>/skills/<name>/SKILL.md`): its
/// folder is the skill, named by the folder (what rulesync writes it as).
pub fn skills_in(source: &str, root: &Path) -> Vec<RuleFile> {
    let Ok(entries) = std::fs::read_dir(root.join("skills")) else { return vec![] };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join("SKILL.md").is_file()).collect();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|dir| {
            let name = dir.file_name()?.to_string_lossy().into_owned();
            let f = parse_front(&std::fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default());
            Some(RuleFile {
                id: format!("{source}:skills/{name}"),
                path: dir.to_string_lossy().into_owned(),
                description: f.description,
                targets: if f.targets.is_empty() { vec!["*".into()] } else { f.targets },
                root: false,
                local_root: false,
                skill: true,
                source: source.to_string(),
            })
        })
        .collect()
}

/// One input root's skills and rules.
pub fn list_root(source: &str, root: &Path) -> Vec<RuleFile> {
    let mut all = skills_in(source, root);
    all.extend(rules_in(source, root));
    all
}

/// The whole catalog: skills, then rules, of the library and each source.
pub fn list(dirs: &Dirs, sources: &[RuleSource]) -> Vec<RuleFile> {
    let _ = std::fs::create_dir_all(dirs.library_rules());
    let mut all = skills_in(LIBRARY, &dirs.library());
    for s in sources {
        all.extend(skills_in(&s.name, Path::new(&s.root)));
    }
    all.extend(rules_in(LIBRARY, &dirs.library()));
    for s in sources {
        all.extend(rules_in(&s.name, Path::new(&s.root)));
    }
    all
}

/// Files under `dir`, as sorted paths relative to it (a skill's files).
pub fn files_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 8 {
                    walk(base, &p, out, depth + 1);
                }
            } else if let Ok(rel) = p.strip_prefix(base) {
                let rel = rel.to_string_lossy().into_owned();
                if rel != ".DS_Store" && !rel.ends_with("/.DS_Store") {
                    out.push(rel);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out, 0);
    out.sort();
    out
}

/// Copy a folder's files into `dest` (made fresh: anything else there goes).
pub fn copy_dir(from: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| format!("could not replace {}: {e}", dest.display()))?;
    }
    for rel in files_under(from) {
        let to = dest.join(&rel);
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        // Keeps the permission bits (a skill's scripts stay executable).
        std::fs::copy(from.join(&rel), &to).map_err(|e| format!("could not copy {rel}: {e}"))?;
    }
    Ok(())
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

/// A rulesync source tree: it has rules or skills.
fn is_root(r: &Path) -> bool {
    r.join("rules").is_dir() || r.join("skills").is_dir()
}

/// The rulesync source tree inside a folder: `<dir>/.rulesync` or `<dir>` itself.
fn find_root(dir: &Path) -> Option<PathBuf> {
    [dir.join(".rulesync"), dir.to_path_buf()].into_iter().find(|r| is_root(r))
}

/// A project's own `.rulesync/` (rules and/or skills), referenced in place
/// (read-only).
pub fn add_project(project: &str, sources: &[RuleSource]) -> Result<RuleSource, String> {
    let dir = Path::new(project);
    let root = dir.join(".rulesync");
    if !is_root(&root) {
        return Err(format!("{} has no .rulesync/rules or .rulesync/skills folder", dir.display()));
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
        return Err("that repository has no rulesync rules or skills (.rulesync/rules, .rulesync/skills, rules/ or skills/)".into());
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

/// Claude Code's own skills in a project (`.claude/skills/<name>/SKILL.md`,
/// the same shape as rulesync's): copied into the library's skills, never
/// over one already there. Returns (ids added, names skipped).
pub fn import_claude_skills(dirs: &Dirs, project: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    let from = project.join(".claude");
    let mut added = vec![];
    let mut skipped = vec![];
    for s in skills_in("claude", &from) {
        let name = skill_name(&s.id).unwrap_or_default().to_string();
        let dest = dirs.library().join("skills").join(&name);
        if dest.exists() {
            skipped.push(name);
            continue;
        }
        copy_dir(Path::new(&s.path), &dest)?;
        added.push(format!("{LIBRARY}:skills/{name}"));
    }
    Ok((added, skipped))
}

/// A new library entry (`pitwall rules new`, `pitwall rules skill new`).
#[derive(Debug, Clone, Default)]
pub struct NewEntry {
    /// A skill (else a rule).
    pub skill: bool,
    pub name: String,
    pub description: Option<String>,
    /// rulesync targets (empty: the source's, else every tool).
    pub targets: Vec<String>,
    /// A library id, a file, or a folder (a skill's) to start from.
    pub from: Option<String>,
    /// `from` is the other kind (a skill to make a rule of, or the
    /// reverse): only with this.
    pub convert: bool,
    /// Replace an entry of that name.
    pub overwrite: bool,
}

/// A usable entry name (a file or folder name of its own).
pub fn entry_name(name: &str, skill: bool) -> Result<String, String> {
    let n = name.trim();
    let n = if skill { n } else { n.trim_end_matches(".md") };
    if n.is_empty() || n == "." || n == ".." || n.starts_with('-') || n.contains(['/', '\\']) || n.ends_with(['.', ' ']) {
        return Err(format!("\"{name}\" can't be a {} name", if skill { "skill" } else { "rule" }));
    }
    Ok(n.to_string())
}

/// Where an entry of that name lives in the library.
pub fn entry_path(dirs: &Dirs, skill: bool, name: &str) -> PathBuf {
    if skill {
        dirs.library().join("skills").join(name)
    } else {
        dirs.library_rules().join(format!("{name}.md"))
    }
}

/// The text after the frontmatter.
fn body_of(src: &str) -> &str {
    match front_block(src) {
        Some((_, end)) => &src[end..],
        None => src,
    }
}

/// The frontmatter lines other than `drop` keys (and their indented
/// continuation lines).
fn other_front(src: &str, drop: &[&str]) -> String {
    let Some((block, _)) = front_block(src) else { return String::new() };
    let mut out = String::new();
    let mut skipping = false;
    for line in block.split_inclusive('\n') {
        let indented = line.starts_with([' ', '\t']);
        if indented && skipping {
            continue;
        }
        skipping = !indented && line.split_once(':').is_some_and(|(k, _)| drop.contains(&k.trim()));
        if !skipping {
            out.push_str(line);
        }
    }
    out
}

fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn yaml_list(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|t| yaml_str(t)).collect::<Vec<_>>().join(", "))
}

/// The first line of text in a body (a description when none is given).
fn first_line(body: &str) -> Option<String> {
    let line = body.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty())?;
    Some(line.chars().take(160).collect())
}

/// Write a new rule or skill into the library, from scratch or from a
/// library entry, a file, or a skill's folder. Returns its id and notes.
pub fn create(dirs: &Dirs, lib: &[RuleFile], e: &NewEntry) -> Result<(String, Vec<String>), String> {
    let name = entry_name(&e.name, e.skill)?;
    let dest = entry_path(dirs, e.skill, &name);
    if dest.exists() && !e.overwrite {
        return Err(format!("the library already has a {} called {name}", if e.skill { "skill" } else { "rule" }));
    }
    let mut notes = Vec::new();
    // (SKILL.md or rule text, a skill folder to copy, targets, description)
    let (text, folder): (String, Option<PathBuf>) = match e.from.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        None => (String::new(), None),
        Some(f) => {
            if let Some(item) = lib.iter().find(|x| x.id == f) {
                if item.skill != e.skill && !e.convert {
                    return Err(if item.skill {
                        format!("{f} is a skill: pass --as-rule to make a rule of it")
                    } else {
                        format!("{f} is a rule: pass --as-skill to make a skill of it")
                    });
                }
                if item.skill {
                    let dir = PathBuf::from(&item.path);
                    (std::fs::read_to_string(dir.join("SKILL.md")).map_err(|e| e.to_string())?, Some(dir))
                } else {
                    (std::fs::read_to_string(&item.path).map_err(|e| e.to_string())?, None)
                }
            } else {
                let p = crate::paths::expand_tilde(f);
                let skill_dir = if p.is_dir() {
                    Some(p.clone())
                } else if p.file_name().is_some_and(|n| n == "SKILL.md") {
                    p.parent().map(Path::to_path_buf)
                } else {
                    None
                };
                match skill_dir {
                    Some(dir) => {
                        if !dir.join("SKILL.md").is_file() {
                            return Err(format!("{} has no SKILL.md", dir.display()));
                        }
                        if !e.skill && !e.convert {
                            return Err(format!("{f} is a skill: pass --as-rule to make a rule of it"));
                        }
                        (std::fs::read_to_string(dir.join("SKILL.md")).map_err(|e| e.to_string())?, Some(dir))
                    }
                    None => (std::fs::read_to_string(&p).map_err(|err| format!("could not read {f}: {err}"))?, None),
                }
            }
        }
    };
    let front = parse_front(&text);
    let body = body_of(&text).trim_start_matches('\n').to_string();
    let targets = if !e.targets.is_empty() {
        e.targets.clone()
    } else if !front.targets.is_empty() {
        front.targets.clone()
    } else {
        vec!["*".into()]
    };
    let description = e.description.clone().filter(|d| !d.trim().is_empty()).or(front.description.clone());
    if e.skill {
        let description = description
            .or_else(|| first_line(&body))
            .ok_or("give the skill a description (--description): agents read it to decide when to use the skill")?;
        let body = if body.trim().is_empty() { format!("# {name}\n\n{description}\n") } else { body };
        // Keys of a skill's own frontmatter that aren't name/description/targets stay.
        let kept = if folder.is_some() { other_front(&text, &["name", "description", "targets", "root", "localRoot", "globs"]) } else { String::new() };
        let skill_md = format!("---\nname: {}\ndescription: {}\ntargets: {}\n{kept}---\n{body}", yaml_str(&name), yaml_str(&description), yaml_list(&targets));
        let tmp = dirs.library().join("skills").join(format!(".new-{}", uuid::Uuid::new_v4()));
        match &folder {
            Some(dir) => copy_dir(dir, &tmp)?,
            None => std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?,
        }
        std::fs::write(tmp.join("SKILL.md"), skill_md).map_err(|e| e.to_string())?;
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        if e.convert && folder.is_none() && e.from.is_some() {
            notes.push("made from a rule: it is used only when an agent decides it's relevant (or you invoke it), not in every session".into());
        }
        Ok((format!("{LIBRARY}:skills/{name}"), notes))
    } else {
        if let Some(dir) = &folder {
            let others: Vec<String> = files_under(dir).into_iter().filter(|f| f != "SKILL.md").collect();
            if !others.is_empty() {
                notes.push(format!("left out the skill's other files ({}): a rule is one file", others.join(", ")));
            }
            notes.push("made from a skill: a rule is in every session's context, so it costs context all the time".into());
        }
        let body = if body.trim().is_empty() { format!("# {name}\n\n{}\n", description.clone().unwrap_or_default()) } else { body };
        let mut head = format!("---\nroot: false\ntargets: {}\n", yaml_list(&targets));
        if let Some(d) = &description {
            head.push_str(&format!("description: {}\n", yaml_str(d)));
        }
        std::fs::create_dir_all(dirs.library_rules()).map_err(|e| e.to_string())?;
        std::fs::write(&dest, format!("{head}---\n{body}")).map_err(|e| e.to_string())?;
        Ok((format!("{LIBRARY}:{name}.md"), notes))
    }
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
                name: None,
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
