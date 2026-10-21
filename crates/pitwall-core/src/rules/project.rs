//! Project rules (docs/spec/rules.md, "Project rules"): library rules copied
//! into a repository's `.rulesync/rules/`, where the user commits them, and
//! the files rulesync generates from them (CLAUDE.md, AGENTS.md, …), which
//! are committed too. Nothing here goes in `.git/info/exclude`: these
//! changes are meant to show up in Review.
//!
//! Pitwall records where each copy came from in `.rulesync/.pitwall.json`
//! (the library rule and its content hash at copy time); the rule files
//! stay plain rulesync files. Updates are opt-in: nothing is synced on its
//! own, and a file that isn't as Pitwall left it is never replaced unless
//! the caller names it.
//!
//! Projects are folders on this Mac, like the library: files through
//! `std::fs`, git through [`LocalExec`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use pitwall_proto::{
    GeneratedFiles, ProjectAssigned, ProjectGenerated, ProjectImported, ProjectRule, ProjectRuleState, ProjectRuleUpdate, ProjectRulesStatus,
    ProjectUnassigned, ProjectUpdated,
};

use super::apply::{self, content_hash};
use super::library;
use super::runner::{self, Runner};
use super::RuleFile;
use crate::exec::LocalExec;

/// The manifest, in `.rulesync/`.
pub const MANIFEST: &str = ".pitwall.json";
/// Hand-written instruction files rulesync can import, and its target for each.
pub const INSTRUCTIONS: &[(&str, &str)] = &[("CLAUDE.md", "claudecode"), ("AGENTS.md", "agentsmd"), ("GEMINI.md", "geminicli")];
/// A project's main instruction files: never generated for a project
/// without rules of Pitwall's (skills only).
pub const MAIN_FILES: &[&str] = &["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md", "AGENTS.local.md", "GEMINI.md", ".github/copilot-instructions.md"];
/// Targets when nothing says which: Claude Code and Codex (AGENTS.md).
pub const DEFAULT_TARGETS: &[&str] = &["claudecode", "codexcli"];

type Res<T> = Result<T, String>;

/// `.rulesync/.pitwall.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Name under `.rulesync/rules/` → where it came from.
    #[serde(default)]
    pub rules: BTreeMap<String, Copied>,
    /// Skill folder name under `.rulesync/skills/` → where it came from
    /// (the hash covers every file in the folder).
    #[serde(default)]
    pub skills: BTreeMap<String, Copied>,
    /// rulesync features the last generation wrote (`skills`, `rules`).
    #[serde(default)]
    pub features: Vec<String>,
    /// rulesync targets the project generates for.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Hand-written instruction files imported into `.rulesync/` (so the
    /// first generation may replace them: their text is in the rules now).
    #[serde(default)]
    pub imported: Vec<String>,
    /// Rules made by those imports: name under `.rulesync/rules/` → the
    /// instruction file it came from (`source`) and its hash as Pitwall
    /// left it (after setting its targets).
    #[serde(default)]
    pub imported_rules: BTreeMap<String, Copied>,
    /// The last generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<Generated>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Copied {
    /// The library rule (`library:style.md`).
    pub source: String,
    /// Content hash of the library rule when it was copied (= the copy's).
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    /// Fingerprint of `.rulesync/rules/` and the targets it was made from
    /// (empty: some files weren't written, so it's out of date).
    pub inputs: String,
    /// Path → content hash of what it wrote (or found already right).
    pub files: BTreeMap<String, String>,
}

pub fn rules_dir(root: &Path) -> PathBuf {
    root.join(".rulesync").join("rules")
}

pub fn skills_dir(root: &Path) -> PathBuf {
    root.join(".rulesync").join("skills")
}

/// One hash over a skill folder: every file's path and content (`None`
/// when it has no SKILL.md).
pub fn dir_hash(dir: &Path) -> Option<String> {
    if !dir.join("SKILL.md").is_file() {
        return None;
    }
    let mut all = Vec::new();
    for rel in library::files_under(dir) {
        all.extend_from_slice(rel.as_bytes());
        all.push(0xff);
        all.extend(std::fs::read(dir.join(&rel)).ok()?);
        all.push(0xff);
    }
    Some(content_hash(&all))
}

/// The project's manifest (empty when it has none).
pub fn load(root: &Path) -> Manifest {
    let path = root.join(".rulesync").join(MANIFEST);
    let Ok(src) = std::fs::read_to_string(&path) else { return Manifest::default() };
    serde_json::from_str(&src).unwrap_or_else(|e| {
        eprintln!("pitwall: ignoring unreadable {}: {e}", path.display());
        Manifest::default()
    })
}

fn save(root: &Path, m: &Manifest) -> Res<()> {
    let dir = root.join(".rulesync");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(m).map_err(|e| e.to_string())? + "\n";
    std::fs::write(dir.join(MANIFEST), json).map_err(|e| format!("could not write .rulesync/{MANIFEST}: {e}"))
}

fn hash_file(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| content_hash(&b))
}

fn read_text(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// `library:web/style.md` → `web/style.md`.
fn rel_of(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, r)| r)
}

/// `.md` files under `dir`, as sorted paths relative to it.
fn md_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                walk(base, &p, out, depth + 1);
            } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                if let Ok(rel) = p.strip_prefix(base) {
                    out.push(rel.to_string_lossy().into_owned());
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out, 0);
    out.sort();
    out
}

/// Remove `rel` under `base`, and folders that leaves empty (not `base`).
fn remove_tidy(base: &Path, rel: &str) -> Res<()> {
    let path = base.join(rel);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("could not remove {rel}: {e}")),
    }
    let mut dir = path.parent();
    while let Some(d) = dir.filter(|d| *d != base && d.starts_with(base)) {
        if std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> Res<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Where a library rule goes: where it went before, else its path in its
/// source, or under a folder named for the source when another source's
/// rule already has that name.
fn name_for(m: &Manifest, id: &str, source: &str) -> String {
    if let Some((n, _)) = m.rules.iter().find(|(_, c)| c.source == id) {
        return n.clone();
    }
    let rel = rel_of(id).to_string();
    match m.rules.get(&rel) {
        Some(c) if c.source != id => format!("{}/{rel}", crate::slug::slug(source)),
        _ => rel,
    }
}

/// Where a library skill goes: its folder name (what rulesync writes it
/// as), or `<source>-<name>` when another source's skill has that name.
fn skill_name_for(m: &Manifest, id: &str, source: &str) -> String {
    if let Some((n, _)) = m.skills.iter().find(|(_, c)| c.source == id) {
        return n.clone();
    }
    let name = library::skill_name(id).unwrap_or("skill").to_string();
    match m.skills.get(&name) {
        Some(c) if c.source != id => format!("{}-{name}", crate::slug::slug(source)),
        _ => name,
    }
}

fn state_from(file: Option<String>, upstream: Option<String>, copied: &str) -> ProjectRuleState {
    match (file, upstream) {
        (None, _) => ProjectRuleState::Deleted,
        (Some(_), None) => ProjectRuleState::SourceGone,
        (Some(f), Some(l)) => match (f != copied, l != copied) {
            (false, false) => ProjectRuleState::Current,
            (false, true) => ProjectRuleState::Update,
            (true, false) => ProjectRuleState::Modified,
            // Both changed the same way.
            (true, true) if f == l => ProjectRuleState::Current,
            (true, true) => ProjectRuleState::Conflict,
        },
    }
}

fn upstream<'a>(lib: &'a [RuleFile], c: &Copied) -> Option<&'a RuleFile> {
    lib.iter().find(|f| f.id == c.source)
}

fn state_of(root: &Path, lib: &[RuleFile], name: &str, c: &Copied) -> ProjectRuleState {
    let up = upstream(lib, c).filter(|f| !f.skill).and_then(|f| hash_file(Path::new(&f.path)));
    state_from(hash_file(&rules_dir(root).join(name)), up, &c.hash)
}

fn skill_state(root: &Path, lib: &[RuleFile], name: &str, c: &Copied) -> ProjectRuleState {
    let up = upstream(lib, c).filter(|f| f.skill).and_then(|f| dir_hash(Path::new(&f.path)));
    state_from(dir_hash(&skills_dir(root).join(name)), up, &c.hash)
}

/// Fingerprint of what generation reads: `.rulesync/rules/` and
/// `.rulesync/skills/`, the targets and the features.
pub fn inputs_fingerprint(root: &Path, targets: &[String], features: &[String]) -> String {
    let mut all = Vec::new();
    for t in targets.iter().chain(features) {
        all.extend_from_slice(t.as_bytes());
        all.push(0xff);
    }
    for (dir, files) in [(rules_dir(root), md_under(&rules_dir(root))), (skills_dir(root), library::files_under(&skills_dir(root)))] {
        for rel in files {
            all.extend_from_slice(rel.as_bytes());
            all.push(0xff);
            all.extend(std::fs::read(dir.join(&rel)).unwrap_or_default());
            all.push(0xff);
        }
    }
    content_hash(&all)
}

fn generated_state(root: &Path, m: &Manifest) -> GeneratedFiles {
    let Some(g) = &m.generated else {
        // Pitwall copied rules or skills but never generated from them.
        let copied = !m.rules.is_empty() || !m.skills.is_empty();
        return GeneratedFiles { generated: false, out_of_date: copied, changed: vec![], targets: m.targets.clone(), features: features(root, m) };
    };
    let changed: Vec<String> = g.files.iter().filter(|(p, h)| hash_file(&root.join(p)).as_ref() != Some(*h)).map(|(p, _)| p.clone()).collect();
    let out_of_date = !changed.is_empty() || g.inputs != inputs_fingerprint(root, &m.targets, &features(root, m));
    GeneratedFiles { generated: true, out_of_date, changed, targets: m.targets.clone(), features: m.features.clone() }
}

fn tracked(root: &Path, names: &[String]) -> BTreeSet<String> {
    apply::tracked(&LocalExec, &root.to_string_lossy(), names)
}

/// Hand-written instruction files to import before generating rules:
/// only once rules were copied into the project (a skills-only project
/// never touches its CLAUDE.md / AGENTS.md); tracked, not imported or
/// generated by Pitwall, while `.rulesync/rules/` holds nothing but
/// Pitwall's copies.
fn existing_instructions(root: &Path, m: &Manifest) -> Vec<String> {
    if m.rules.is_empty() {
        return vec![];
    }
    // Rules the project wrote itself (not Pitwall's copies or imports).
    let own = md_under(&rules_dir(root)).iter().any(|r| !m.rules.contains_key(r) && !m.imported_rules.contains_key(r));
    if own {
        return vec![];
    }
    let candidates: Vec<String> = INSTRUCTIONS
        .iter()
        .map(|(n, _)| n.to_string())
        .filter(|n| root.join(n).is_file() && !m.imported.contains(n))
        .filter(|n| !m.generated.as_ref().is_some_and(|g| g.files.contains_key(n)))
        .collect();
    let tracked = tracked(root, &candidates);
    candidates.into_iter().filter(|n| tracked.contains(n)).collect()
}

/// Skill folders in `.rulesync/skills/` (each with a SKILL.md).
fn skill_dirs(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(skills_dir(root)) else { return vec![] };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join("SKILL.md").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// What generating writes: skills when the project has any, rules only
/// when Pitwall copied rules in or imported its instruction files (the
/// main instruction file stays the project's own otherwise).
pub fn features(root: &Path, m: &Manifest) -> Vec<String> {
    let mut f = Vec::new();
    if !m.rules.is_empty() || !m.imported.is_empty() {
        f.push("rules".to_string());
    }
    if !skill_dirs(root).is_empty() {
        f.push("skills".to_string());
    }
    f
}

fn description_of(path: &Path) -> Option<String> {
    read_text(path).and_then(|s| library::parse_front(&s).description)
}

/// The project's rules and skills, updates, generated files and what to
/// offer.
pub fn status(root: &Path, lib: &[RuleFile], fallback_set: Option<String>) -> ProjectRulesStatus {
    let m = load(root);
    let dir = rules_dir(root);
    let sdir = skills_dir(root);
    let mut rules: Vec<ProjectRule> = Vec::new();
    for (name, c) in &m.skills {
        rules.push(ProjectRule {
            name: name.clone(),
            skill: true,
            source: Some(c.source.clone()),
            state: skill_state(root, lib, name, c),
            description: description_of(&sdir.join(name).join("SKILL.md")),
        });
    }
    for name in skill_dirs(root) {
        if !m.skills.contains_key(&name) {
            let description = description_of(&sdir.join(&name).join("SKILL.md"));
            rules.push(ProjectRule { name, skill: true, source: None, state: ProjectRuleState::Own, description });
        }
    }
    let mut plain: Vec<ProjectRule> = m
        .rules
        .iter()
        .map(|(name, c)| ProjectRule {
            name: name.clone(),
            skill: false,
            source: Some(c.source.clone()),
            state: state_of(root, lib, name, c),
            description: description_of(&dir.join(name)),
        })
        .collect();
    for rel in md_under(&dir) {
        if !m.rules.contains_key(&rel) {
            let description = description_of(&dir.join(&rel));
            plain.push(ProjectRule { name: rel, skill: false, source: None, state: ProjectRuleState::Own, description });
        }
    }
    rules.sort_by(|a, b| a.name.cmp(&b.name));
    plain.sort_by(|a, b| a.name.cmp(&b.name));
    rules.extend(plain);
    let updates = rules.iter().filter(|r| matches!(r.state, ProjectRuleState::Update | ProjectRuleState::Conflict)).count();
    ProjectRulesStatus {
        project: root.to_string_lossy().into_owned(),
        rulesync: dir.is_dir() || sdir.is_dir(),
        updates,
        generated: generated_state(root, &m),
        existing: existing_instructions(root, &m),
        fallback_set: fallback_set.filter(|_| !dir.is_dir()),
        rules,
    }
}

/// Copy library rules and skills (`ids`, in order) into `.rulesync/rules/`
/// and `.rulesync/skills/`. A file (or skill folder) already there with
/// other content is only replaced when `overwrite` names it (its name or
/// its id).
pub fn assign(root: &Path, lib: &[RuleFile], ids: &[String], overwrite: &[String]) -> Res<ProjectAssigned> {
    let mut m = load(root);
    let dir = rules_dir(root);
    let mut out = ProjectAssigned::default();
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(file) = lib.iter().find(|f| &f.id == id) else {
            out.skipped.push(format!("{id}: not in the library"));
            continue;
        };
        if file.skill {
            let name = skill_name_for(&m, id, &file.source);
            let label = format!("skills/{name}");
            let dest = skills_dir(root).join(&name);
            let hash = dir_hash(Path::new(&file.path)).ok_or_else(|| format!("{id}: its folder has no SKILL.md"))?;
            match dir_hash(&dest) {
                Some(cur) if cur == hash => out.unchanged.push(label),
                _ if dest.exists() && !overwrite.iter().any(|o| *o == label || *o == name || o == id) => {
                    out.conflicts.push(label);
                    continue;
                }
                _ => {
                    library::copy_dir(Path::new(&file.path), &dest)?;
                    out.copied.push(label);
                }
            }
            m.skills.insert(name, Copied { source: id.clone(), hash });
            continue;
        }
        let content = std::fs::read(&file.path).map_err(|e| format!("could not read {id}: {e}"))?;
        let name = name_for(&m, id, &file.source);
        if !apply::safe_rel(&name) {
            out.skipped.push(format!("{id}: unusable name {name}"));
            continue;
        }
        let dest = dir.join(&name);
        match std::fs::read(&dest) {
            Ok(cur) if cur == content => out.unchanged.push(name.clone()),
            Ok(_) if !overwrite.iter().any(|o| o == &name || o == id) => {
                out.conflicts.push(name);
                continue;
            }
            _ => {
                write(&dest, &content)?;
                out.copied.push(name.clone());
            }
        }
        m.rules.insert(name, Copied { source: id.clone(), hash: content_hash(&content) });
    }
    if !out.copied.is_empty() || !out.unchanged.is_empty() {
        save(root, &m)?;
    }
    Ok(out)
}

/// A copied rule or skill, by name.
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Rule(String),
    Skill(String),
}

/// The manifest entry a key means: a rule's name, a skill's name (or
/// `skills/<name>`), or a library id.
fn entry(m: &Manifest, key: &str) -> Option<Entry> {
    let key = key.trim();
    if m.rules.contains_key(key) {
        return Some(Entry::Rule(key.to_string()));
    }
    let bare = key.strip_prefix("skills/").unwrap_or(key);
    if m.skills.contains_key(bare) {
        return Some(Entry::Skill(bare.to_string()));
    }
    if let Some((n, _)) = m.rules.iter().find(|(_, c)| c.source == key) {
        return Some(Entry::Rule(n.clone()));
    }
    m.skills.iter().find(|(_, c)| c.source == key).map(|(n, _)| Entry::Skill(n.clone()))
}

/// Remove copied rules and skills from the project: only as copied (or
/// `force`).
pub fn unassign(root: &Path, keys: &[String], force: bool) -> Res<ProjectUnassigned> {
    let mut m = load(root);
    let dir = rules_dir(root);
    let mut out = ProjectUnassigned::default();
    for key in keys {
        match entry(&m, key) {
            None => out.unknown.push(key.clone()),
            Some(Entry::Rule(name)) => {
                let c = &m.rules[&name];
                match hash_file(&dir.join(&name)) {
                    Some(h) if h != c.hash && !force => {
                        out.modified.push(name);
                        continue;
                    }
                    Some(_) => remove_tidy(&dir, &name)?,
                    None => {}
                }
                m.rules.remove(&name);
                out.removed.push(name);
            }
            Some(Entry::Skill(name)) => {
                let c = &m.skills[&name];
                let path = skills_dir(root).join(&name);
                let label = format!("skills/{name}");
                match dir_hash(&path) {
                    Some(h) if h != c.hash && !force => {
                        out.modified.push(label);
                        continue;
                    }
                    _ if path.exists() => {
                        std::fs::remove_dir_all(&path).map_err(|e| format!("could not remove {label}: {e}"))?;
                        let _ = std::fs::remove_dir(skills_dir(root));
                    }
                    _ => {}
                }
                m.skills.remove(&name);
                out.removed.push(label);
            }
        }
    }
    if !out.removed.is_empty() {
        save(root, &m)?;
    }
    Ok(out)
}

fn diff(project: &str, library: &str) -> String {
    similar::TextDiff::from_lines(project, library).unified_diff().context_radius(3).header("project", "library").to_string()
}

/// One diff per file that differs between a project's skill folder and
/// the library's.
fn dir_diff(project: &Path, library: &Path) -> String {
    let mut files: BTreeSet<String> = library::files_under(project).into_iter().collect();
    files.extend(library::files_under(library));
    let mut out = String::new();
    for rel in files {
        let a = read_text(&project.join(&rel)).unwrap_or_default();
        let b = read_text(&library.join(&rel)).unwrap_or_default();
        if a != b {
            out.push_str(
                &similar::TextDiff::from_lines(&a, &b)
                    .unified_diff()
                    .context_radius(3)
                    .header(&format!("project/{rel}"), &format!("library/{rel}"))
                    .to_string(),
            );
        }
    }
    out
}

/// Rules and skills with something to decide: an update, a local change,
/// a conflict.
pub fn updates(root: &Path, lib: &[RuleFile]) -> Vec<ProjectRuleUpdate> {
    let m = load(root);
    let dir = rules_dir(root);
    let pending = |s: ProjectRuleState| matches!(s, ProjectRuleState::Update | ProjectRuleState::Modified | ProjectRuleState::Conflict);
    let mut out: Vec<ProjectRuleUpdate> = m
        .skills
        .iter()
        .filter_map(|(name, c)| {
            let state = skill_state(root, lib, name, c);
            if !pending(state) {
                return None;
            }
            let here = skills_dir(root).join(name);
            let there = upstream(lib, c).map(|f| PathBuf::from(&f.path));
            Some(ProjectRuleUpdate {
                name: name.clone(),
                skill: true,
                source: c.source.clone(),
                state,
                library: there.as_ref().and_then(|d| read_text(&d.join("SKILL.md"))),
                project: read_text(&here.join("SKILL.md")),
                diff: there.map(|d| dir_diff(&here, &d)).unwrap_or_default(),
            })
        })
        .collect();
    out.extend(m.rules.iter().filter_map(|(name, c)| {
        let state = state_of(root, lib, name, c);
        if !pending(state) {
            return None;
        }
        let library = upstream(lib, c).and_then(|f| read_text(Path::new(&f.path)));
        let project = read_text(&dir.join(name));
        let diff = diff(project.as_deref().unwrap_or(""), library.as_deref().unwrap_or(""));
        Some(ProjectRuleUpdate { name: name.clone(), skill: false, source: c.source.clone(), state, library, project, diff })
    }));
    out
}

/// Take library updates (`keys`; empty: every rule and skill with one).
/// Conflicts need `take`: `library` replaces the project's copy, `project`
/// keeps it.
pub fn update(root: &Path, lib: &[RuleFile], keys: &[String], take: Option<&str>) -> Res<ProjectUpdated> {
    if let Some(t) = take.filter(|t| !matches!(*t, "library" | "project")) {
        return Err(format!("{t}: take library or project"));
    }
    let mut m = load(root);
    let dir = rules_dir(root);
    let names: Vec<Entry> = if keys.is_empty() {
        m.skills.keys().map(|n| Entry::Skill(n.clone())).chain(m.rules.keys().map(|n| Entry::Rule(n.clone()))).collect()
    } else {
        keys.iter().map(|k| entry(&m, k).ok_or_else(|| format!("no rule or skill \"{k}\" copied into this project"))).collect::<Res<_>>()?
    };
    let mut out = ProjectUpdated::default();
    for e in names {
        let (c, state, label) = match &e {
            Entry::Rule(n) => {
                let c = m.rules[n].clone();
                let st = state_of(root, lib, n, &c);
                (c, st, n.clone())
            }
            Entry::Skill(n) => {
                let c = m.skills[n].clone();
                let st = skill_state(root, lib, n, &c);
                (c, st, format!("skills/{n}"))
            }
        };
        let from = || -> Res<&RuleFile> { upstream(lib, &c).ok_or_else(|| "gone from the library".to_string()) };
        // Write the library's version; its hash.
        let take_library = || -> Res<String> {
            let f = from()?;
            match &e {
                Entry::Rule(n) => {
                    let bytes = std::fs::read(&f.path).map_err(|e| e.to_string())?;
                    write(&dir.join(n), &bytes)?;
                    Ok(content_hash(&bytes))
                }
                Entry::Skill(n) => {
                    library::copy_dir(Path::new(&f.path), &skills_dir(root).join(n))?;
                    dir_hash(Path::new(&f.path)).ok_or_else(|| "the library's skill has no SKILL.md".to_string())
                }
            }
        };
        let library_hash = || -> Res<String> {
            let f = from()?;
            if f.skill { dir_hash(Path::new(&f.path)) } else { hash_file(Path::new(&f.path)) }.ok_or_else(|| "unreadable in the library".to_string())
        };
        let set = |m: &mut Manifest, hash: String| match &e {
            Entry::Rule(n) => {
                m.rules.insert(n.clone(), Copied { source: c.source.clone(), hash });
            }
            Entry::Skill(n) => {
                m.skills.insert(n.clone(), Copied { source: c.source.clone(), hash });
            }
        };
        match (state, take) {
            (ProjectRuleState::Update, _) | (ProjectRuleState::Conflict, Some("library")) => {
                let h = take_library()?;
                set(&mut m, h);
                out.updated.push(label);
            }
            (ProjectRuleState::Conflict, Some(_)) => {
                // Keep the project's copy; the library's version counts as seen.
                let h = library_hash()?;
                set(&mut m, h);
                out.kept.push(label);
            }
            (ProjectRuleState::Conflict, None) => out.conflicts.push(label),
            _ => {}
        }
    }
    if !out.updated.is_empty() || !out.kept.is_empty() {
        save(root, &m)?;
    }
    Ok(out)
}

/// Import the hand-written instruction files ([`status`]'s `existing`)
/// into `.rulesync/rules/` with `rulesync import`, run in a scratch folder
/// under `stage` holding a copy, so the project only gains new rule files
/// (`claude.md`, `agents.md`, …), each for the tools that read its file
/// ([`imported_targets`]).
pub fn import_existing(root: &Path, runner: &Runner, stage: &Path) -> Res<ProjectImported> {
    let mut m = load(root);
    let files = existing_instructions(root, &m);
    if files.is_empty() {
        return Err("there are no hand-written instruction files (CLAUDE.md, AGENTS.md, GEMINI.md tracked by git) to import".into());
    }
    let dir = rules_dir(root);
    let mut out = ProjectImported::default();
    let mut log = Vec::new();
    for file in &files {
        let target = INSTRUCTIONS.iter().find(|(n, _)| n == file).map(|(_, t)| *t).unwrap_or("claudecode");
        let scratch = stage.join(format!("import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
        let res = (|| -> Res<Vec<String>> {
            std::fs::copy(root.join(file), scratch.join(file)).map_err(|e| e.to_string())?;
            let args: Vec<String> = ["import", "--targets", target, "--features", "rules"].map(String::from).into();
            let doc = runner.run_json(&scratch, &args)?;
            log.extend(runner::warnings(&doc).into_iter().map(|w| format!("rulesync: {w}")));
            let produced = scratch.join(".rulesync").join("rules");
            let stem = file.trim_end_matches(".md").to_lowercase();
            let mut added = Vec::new();
            for rel in md_under(&produced) {
                // rulesync names it after the file it came from (CLAUDE.md → claude.md).
                let base = if rel.eq_ignore_ascii_case(file) { format!("{stem}.md") } else { format!("{stem}-{}", rel.replace('/', "-")) };
                let name = (1..)
                    .map(|n| if n == 1 { base.clone() } else { format!("{}-{n}.md", base.trim_end_matches(".md")) })
                    .find(|n| !dir.join(n).exists())
                    .expect("unbounded");
                let bytes = std::fs::read(produced.join(&rel)).map_err(|e| e.to_string())?;
                write(&dir.join(&name), &bytes)?;
                added.push(name);
            }
            Ok(added)
        })();
        let _ = std::fs::remove_dir_all(&scratch);
        let added = res?;
        if added.is_empty() {
            log.push(format!("rulesync found no rules in {file}"));
        }
        for name in &added {
            m.imported_rules.insert(name.clone(), Copied { source: file.clone(), hash: String::new() });
        }
        out.added.extend(added);
        out.files.push(file.clone());
        m.imported.push(file.clone());
    }
    retarget_imports(root, &mut m, &mut log)?;
    save(root, &m)?;
    out.log = log.join("\n");
    Ok(out)
}

/// The skill file in `.rulesync/skills/` a generated path comes from
/// (`.claude/skills/review/run.sh` → `.rulesync/skills/review/run.sh`).
fn skill_source(root: &Path, rel: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = rel.split('/').collect();
    let i = parts.iter().position(|p| *p == "skills")?;
    let (name, rest) = (parts.get(i + 1)?, parts.get(i + 2..)?);
    if rest.is_empty() {
        return None;
    }
    let src = skills_dir(root).join(name).join(rest.join("/"));
    src.is_file().then_some(src)
}

/// rulesync drops the executable bit of a skill's scripts (checked with
/// rulesync 33): give a generated copy the bit its source has.
fn keep_executable(root: &Path, rel: &str, dest: &Path) {
    let executable = skill_source(root, rel).is_some_and(|s| crate::platform::is_executable(&s));
    if executable && !crate::platform::is_executable(dest) {
        let _ = crate::platform::make_executable(dest);
    }
}

/// Tools that read AGENTS.md (besides whatever the project targets).
const AGENTS_READERS: &[&str] = &["agentsmd", "codexcli", "opencode", "amp", "copilot", "copilotcli", "cursor"];

/// The targets an imported instruction file's rule should have, so that
/// generating reproduces each file as it was: `rulesync import` makes every
/// one a root rule for all tools, and two of them would be merged into
/// both CLAUDE.md and AGENTS.md. CLAUDE.md's rule is Claude Code's,
/// GEMINI.md's Gemini CLI's; AGENTS.md's goes to every tool but Claude Code
/// when a CLAUDE.md was imported too (else Claude Code still gets it).
pub fn imported_targets(file: &str, claude_imported: bool, in_use: &[String]) -> Vec<String> {
    match file {
        "CLAUDE.md" => vec!["claudecode".into()],
        "GEMINI.md" => vec!["geminicli".into()],
        "AGENTS.md" if claude_imported => {
            let mut out: Vec<String> = Vec::new();
            for t in in_use.iter().map(String::as_str).chain(AGENTS_READERS.iter().copied()) {
                if !matches!(t, "claudecode" | "geminicli" | "*") && !out.iter().any(|o| o == t) {
                    out.push(t.to_string());
                }
            }
            out
        }
        _ => vec!["*".into()],
    }
}

/// `src` with its frontmatter `targets` set to `targets`; everything else
/// byte for byte.
pub fn set_targets(src: &str, targets: &[String]) -> String {
    let line = format!("targets: [{}]\n", targets.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", "));
    let Some((block, _)) = library::front_block(src) else {
        return format!("---\n{line}---\n{src}");
    };
    // `block` is a slice of `src`, right after the opening `---` line.
    let start = block.as_ptr() as usize - src.as_ptr() as usize;
    let mut out = String::from(&src[..start]);
    let mut done = false;
    let mut skipping = false;
    for l in block.split_inclusive('\n') {
        let indented = l.starts_with([' ', '\t']);
        if skipping && indented {
            continue;
        }
        skipping = false;
        if !indented && l.trim_start().starts_with("targets:") {
            out.push_str(&line);
            done = true;
            skipping = true;
            continue;
        }
        out.push_str(l);
    }
    if !done {
        out.push_str(&line);
    }
    out.push_str(&src[start + block.len()..]);
    out
}

/// Give every imported rule its [`imported_targets`]: new ones always,
/// earlier ones only while unchanged since Pitwall last wrote them.
fn retarget_imports(root: &Path, m: &mut Manifest, log: &mut Vec<String>) -> Res<()> {
    let dir = rules_dir(root);
    let claude = m.imported.iter().any(|f| f == "CLAUDE.md");
    let in_use = if m.targets.is_empty() { DEFAULT_TARGETS.iter().map(|t| t.to_string()).collect() } else { m.targets.clone() };
    for (name, c) in m.imported_rules.clone() {
        let path = dir.join(&name);
        let Some(text) = read_text(&path) else { continue };
        let want = imported_targets(&c.source, claude, &in_use);
        let have = library::parse_front(&text).targets;
        if have == want {
            if c.hash.is_empty() {
                m.imported_rules.insert(name, Copied { hash: content_hash(text.as_bytes()), ..c });
            }
            continue;
        }
        if !c.hash.is_empty() && content_hash(text.as_bytes()) != c.hash {
            log.push(format!(
                "kept {name}: changed since it was imported; for {} to come out as it was, set its targets to [{}]",
                c.source,
                want.join(", ")
            ));
            continue;
        }
        let new = set_targets(&text, &want);
        write(&path, new.as_bytes())?;
        log.push(format!("{name} ({}) now applies to {}", c.source, want.join(", ")));
        m.imported_rules.insert(name, Copied { hash: content_hash(new.as_bytes()), ..c });
    }
    Ok(())
}

/// Run rulesync for the project's `.rulesync/` (in a staging folder under
/// `stage`, so the project's own rulesync.jsonc doesn't apply) and place
/// what it generates in the project. A file is only replaced when it is
/// what Pitwall generated last time, an imported instruction file not
/// generated yet, one of `released` (an agent's local copy of a file the
/// project now generates), or named in `overwrite`. Earlier outputs that
/// aren't generated any more are removed when unchanged. Returns what it
/// did and the paths written.
pub fn generate(root: &Path, runner: &Runner, stage: &Path, targets: &[String], overwrite: &[String], released: &BTreeSet<String>) -> Res<ProjectGenerated> {
    let source = root.join(".rulesync");
    if targets.is_empty() {
        return Err("say which rulesync targets to generate for".into());
    }
    let mut m = load(root);
    let features = features(root, &m);
    if features.is_empty() {
        return Err("nothing to generate: assign skills or rules to the project first".into());
    }
    let with_rules = features.iter().any(|f| f == "rules");
    let _ = std::fs::remove_dir_all(stage);
    let out_root = stage.join("out");
    std::fs::create_dir_all(&out_root).map_err(|e| e.to_string())?;
    std::fs::write(stage.join("rulesync.jsonc"), "{ \"delete\": false }\n").map_err(|e| e.to_string())?;
    let args: Vec<String> = vec![
        "generate".into(),
        "--targets".into(),
        targets.join(","),
        "--features".into(),
        features.join(","),
        "--output-roots".into(),
        out_root.to_string_lossy().into_owned(),
        "--input-roots".into(),
        source.to_string_lossy().into_owned(),
    ];
    let result = (|| -> Res<ProjectGenerated> {
        let doc = runner.run_json(stage, &args)?;
        let mut log: Vec<String> = runner::warnings(&doc).into_iter().map(|w| format!("rulesync: {w}")).collect();
        let ops = doc.pointer("/data/plan/operations").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut outputs: Vec<String> = ops
            .iter()
            .filter(|o| o["action"] == "write" && o["kind"] == "file")
            .filter_map(|o| o["path"].as_str().map(String::from))
            .collect();
        // rulesync lists them in directory order, which differs by filesystem.
        outputs.sort();
        let before = m.generated.clone().map(|g| g.files).unwrap_or_default();
        let mut res = ProjectGenerated { targets: targets.to_vec(), features: features.clone(), ..Default::default() };
        let mut files = BTreeMap::new();
        for rel in &outputs {
            if !apply::safe_rel(rel) || rel.starts_with(".rulesync/") {
                log.push(format!("skipped unexpected path {rel}"));
                continue;
            }
            // Without rules, the project's main instruction file stays its own.
            if !with_rules && MAIN_FILES.contains(&rel.as_str()) {
                log.push(format!("left {rel} alone: the project's own (no rules were copied in)"));
                continue;
            }
            let Ok(bytes) = std::fs::read(out_root.join(rel)) else { continue };
            let dest = root.join(rel);
            let current = std::fs::read(&dest).ok();
            if current.as_deref() == Some(&bytes[..]) {
                res.unchanged.push(rel.clone());
                files.insert(rel.clone(), content_hash(&bytes));
                continue;
            }
            let ours = current.as_ref().is_some_and(|c| before.get(rel) == Some(&content_hash(c)));
            let imported = m.imported.contains(rel) && !before.contains_key(rel);
            let allowed = current.is_none() || ours || imported || released.contains(rel) || overwrite.contains(rel);
            if !allowed {
                log.push(format!("kept {rel}: not written by Pitwall (pass it to overwrite to replace it)"));
                res.needs_confirm.push(rel.clone());
                continue;
            }
            write(&dest, &bytes)?;
            keep_executable(root, rel, &dest);
            res.written.push(rel.clone());
            files.insert(rel.clone(), content_hash(&bytes));
        }
        // Earlier outputs no longer generated (a target or feature dropped), if unchanged.
        for (rel, h) in &before {
            if !outputs.contains(rel) && hash_file(&root.join(rel)).as_ref() == Some(h) {
                remove_tidy(root, rel)?;
                log.push(format!("removed {rel} (no longer generated)"));
            }
        }
        let inputs = if res.needs_confirm.is_empty() { inputs_fingerprint(root, targets, &features) } else { String::new() };
        m.targets = targets.to_vec();
        m.features = features.clone();
        m.generated = Some(Generated { inputs, files });
        save(root, &m)?;
        res.log = log.join("\n");
        Ok(res)
    })();
    let _ = std::fs::remove_dir_all(stage);
    result
}
