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
    /// rulesync targets the project generates for.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Hand-written instruction files imported into `.rulesync/` (so the
    /// first generation may replace them: their text is in the rules now).
    #[serde(default)]
    pub imported: Vec<String>,
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

fn state_of(root: &Path, lib: &[RuleFile], name: &str, c: &Copied) -> ProjectRuleState {
    let file = hash_file(&rules_dir(root).join(name));
    let upstream = lib.iter().find(|f| f.id == c.source).and_then(|f| hash_file(Path::new(&f.path)));
    match (file, upstream) {
        (None, _) => ProjectRuleState::Deleted,
        (Some(_), None) => ProjectRuleState::SourceGone,
        (Some(f), Some(l)) => match (f != c.hash, l != c.hash) {
            (false, false) => ProjectRuleState::Current,
            (false, true) => ProjectRuleState::Update,
            (true, false) => ProjectRuleState::Modified,
            // Both changed the same way.
            (true, true) if f == l => ProjectRuleState::Current,
            (true, true) => ProjectRuleState::Conflict,
        },
    }
}

/// Fingerprint of what generation reads: `.rulesync/rules/` and the targets.
pub fn inputs_fingerprint(root: &Path, targets: &[String]) -> String {
    let dir = rules_dir(root);
    let mut all = Vec::new();
    for t in targets {
        all.extend_from_slice(t.as_bytes());
        all.push(0xff);
    }
    for rel in md_under(&dir) {
        all.extend_from_slice(rel.as_bytes());
        all.push(0xff);
        all.extend(std::fs::read(dir.join(&rel)).unwrap_or_default());
        all.push(0xff);
    }
    content_hash(&all)
}

fn generated_state(root: &Path, m: &Manifest) -> GeneratedFiles {
    let Some(g) = &m.generated else {
        // Pitwall copied rules but never generated from them.
        return GeneratedFiles { generated: false, out_of_date: !m.rules.is_empty(), changed: vec![], targets: m.targets.clone() };
    };
    let changed: Vec<String> = g.files.iter().filter(|(p, h)| hash_file(&root.join(p)).as_ref() != Some(*h)).map(|(p, _)| p.clone()).collect();
    let out_of_date = !changed.is_empty() || g.inputs != inputs_fingerprint(root, &m.targets);
    GeneratedFiles { generated: true, out_of_date, changed, targets: m.targets.clone() }
}

fn tracked(root: &Path, names: &[String]) -> BTreeSet<String> {
    apply::tracked(&LocalExec, &root.to_string_lossy(), names)
}

/// Hand-written instruction files to import before generating: tracked,
/// not imported or generated by Pitwall, while `.rulesync/rules/` holds
/// nothing but Pitwall's copies.
fn existing_instructions(root: &Path, m: &Manifest) -> Vec<String> {
    let own = md_under(&rules_dir(root)).iter().any(|r| !m.rules.contains_key(r));
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

/// The project's rules, updates, generated files and what to offer.
pub fn status(root: &Path, lib: &[RuleFile], fallback_set: Option<String>) -> ProjectRulesStatus {
    let m = load(root);
    let dir = rules_dir(root);
    let mut rules: Vec<ProjectRule> = m
        .rules
        .iter()
        .map(|(name, c)| ProjectRule {
            name: name.clone(),
            source: Some(c.source.clone()),
            state: state_of(root, lib, name, c),
            description: read_text(&dir.join(name)).and_then(|s| library::parse_front(&s).description),
        })
        .collect();
    for rel in md_under(&dir) {
        if !m.rules.contains_key(&rel) {
            let description = read_text(&dir.join(&rel)).and_then(|s| library::parse_front(&s).description);
            rules.push(ProjectRule { name: rel, source: None, state: ProjectRuleState::Own, description });
        }
    }
    rules.sort_by(|a, b| a.name.cmp(&b.name));
    let updates = rules.iter().filter(|r| matches!(r.state, ProjectRuleState::Update | ProjectRuleState::Conflict)).count();
    ProjectRulesStatus {
        project: root.to_string_lossy().into_owned(),
        rulesync: dir.is_dir(),
        updates,
        generated: generated_state(root, &m),
        existing: existing_instructions(root, &m),
        fallback_set: fallback_set.filter(|_| !dir.is_dir()),
        rules,
    }
}

/// Copy library rules (`ids`, in order) into `.rulesync/rules/`. A file
/// already there with other content is only replaced when `overwrite`
/// names it (its name or its rule id).
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

/// The manifest entry a name or library id means.
fn entry(m: &Manifest, key: &str) -> Option<String> {
    let key = key.trim();
    if m.rules.contains_key(key) {
        return Some(key.to_string());
    }
    m.rules.iter().find(|(_, c)| c.source == key).map(|(n, _)| n.clone())
}

/// Remove copied rules from the project: only as copied (or `force`).
pub fn unassign(root: &Path, keys: &[String], force: bool) -> Res<ProjectUnassigned> {
    let mut m = load(root);
    let dir = rules_dir(root);
    let mut out = ProjectUnassigned::default();
    for key in keys {
        let Some(name) = entry(&m, key) else {
            out.unknown.push(key.clone());
            continue;
        };
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
    if !out.removed.is_empty() {
        save(root, &m)?;
    }
    Ok(out)
}

fn diff(project: &str, library: &str) -> String {
    similar::TextDiff::from_lines(project, library).unified_diff().context_radius(3).header("project", "library").to_string()
}

/// Rules with something to decide: an update, a local change, a conflict.
pub fn updates(root: &Path, lib: &[RuleFile]) -> Vec<ProjectRuleUpdate> {
    let m = load(root);
    let dir = rules_dir(root);
    m.rules
        .iter()
        .filter_map(|(name, c)| {
            let state = state_of(root, lib, name, c);
            if !matches!(state, ProjectRuleState::Update | ProjectRuleState::Modified | ProjectRuleState::Conflict) {
                return None;
            }
            let library = lib.iter().find(|f| f.id == c.source).and_then(|f| read_text(Path::new(&f.path)));
            let project = read_text(&dir.join(name));
            let diff = diff(project.as_deref().unwrap_or(""), library.as_deref().unwrap_or(""));
            Some(ProjectRuleUpdate { name: name.clone(), source: c.source.clone(), state, library, project, diff })
        })
        .collect()
}

/// Take library updates (`keys`; empty: every rule with one). Conflicts
/// need `take`: `library` replaces the project's copy, `project` keeps it.
pub fn update(root: &Path, lib: &[RuleFile], keys: &[String], take: Option<&str>) -> Res<ProjectUpdated> {
    if let Some(t) = take.filter(|t| !matches!(*t, "library" | "project")) {
        return Err(format!("{t}: take library or project"));
    }
    let mut m = load(root);
    let dir = rules_dir(root);
    let names: Vec<String> = if keys.is_empty() {
        m.rules.keys().cloned().collect()
    } else {
        keys.iter().map(|k| entry(&m, k).ok_or_else(|| format!("no rule \"{k}\" copied into this project"))).collect::<Res<_>>()?
    };
    let mut out = ProjectUpdated::default();
    for name in names {
        let c = m.rules[&name].clone();
        let state = state_of(root, lib, &name, &c);
        let upstream = || -> Res<Vec<u8>> {
            let f = lib.iter().find(|f| f.id == c.source).ok_or("gone from the library")?;
            std::fs::read(&f.path).map_err(|e| e.to_string())
        };
        match (state, take) {
            (ProjectRuleState::Update, _) | (ProjectRuleState::Conflict, Some("library")) => {
                let bytes = upstream()?;
                write(&dir.join(&name), &bytes)?;
                m.rules.insert(name.clone(), Copied { source: c.source, hash: content_hash(&bytes) });
                out.updated.push(name);
            }
            (ProjectRuleState::Conflict, Some(_)) => {
                // Keep the project's copy; the library's version counts as seen.
                let bytes = upstream()?;
                m.rules.insert(name.clone(), Copied { source: c.source, hash: content_hash(&bytes) });
                out.kept.push(name);
            }
            (ProjectRuleState::Conflict, None) => out.conflicts.push(name),
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
/// under `stage` holding a copy, so the project only gains new rule files.
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
                let base = format!("{stem}-{}", rel.replace('/', "-"));
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
        out.added.extend(added);
        out.files.push(file.clone());
        m.imported.push(file.clone());
    }
    save(root, &m)?;
    out.log = log.join("\n");
    Ok(out)
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
    if !source.join("rules").is_dir() {
        return Err("the project has no .rulesync/rules yet: assign or import rules first".into());
    }
    if targets.is_empty() {
        return Err("say which rulesync targets to generate for".into());
    }
    let mut m = load(root);
    let _ = std::fs::remove_dir_all(stage);
    let out_root = stage.join("out");
    std::fs::create_dir_all(&out_root).map_err(|e| e.to_string())?;
    std::fs::write(stage.join("rulesync.jsonc"), "{ \"delete\": false }\n").map_err(|e| e.to_string())?;
    let args: Vec<String> = vec![
        "generate".into(),
        "--targets".into(),
        targets.join(","),
        "--features".into(),
        "rules".into(),
        "--output-roots".into(),
        out_root.to_string_lossy().into_owned(),
        "--input-roots".into(),
        source.to_string_lossy().into_owned(),
    ];
    let result = (|| -> Res<ProjectGenerated> {
        let doc = runner.run_json(stage, &args)?;
        let mut log: Vec<String> = runner::warnings(&doc).into_iter().map(|w| format!("rulesync: {w}")).collect();
        let ops = doc.pointer("/data/plan/operations").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let outputs: Vec<String> = ops
            .iter()
            .filter(|o| o["action"] == "write" && o["kind"] == "file")
            .filter_map(|o| o["path"].as_str().map(String::from))
            .collect();
        let before = m.generated.clone().map(|g| g.files).unwrap_or_default();
        let mut res = ProjectGenerated { targets: targets.to_vec(), ..Default::default() };
        let mut files = BTreeMap::new();
        for rel in &outputs {
            if !apply::safe_rel(rel) || rel.starts_with(".rulesync/") {
                log.push(format!("skipped unexpected path {rel}"));
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
            res.written.push(rel.clone());
            files.insert(rel.clone(), content_hash(&bytes));
        }
        // Earlier outputs no longer generated (a target dropped), if unchanged.
        for (rel, h) in &before {
            if !outputs.contains(rel) && hash_file(&root.join(rel)).as_ref() == Some(h) {
                remove_tidy(root, rel)?;
                log.push(format!("removed {rel} (no longer generated)"));
            }
        }
        let inputs = if res.needs_confirm.is_empty() { inputs_fingerprint(root, targets) } else { String::new() };
        m.targets = targets.to_vec();
        m.generated = Some(Generated { inputs, files });
        save(root, &m)?;
        res.log = log.join("\n");
        Ok(res)
    })();
    let _ = std::fs::remove_dir_all(stage);
    result
}
