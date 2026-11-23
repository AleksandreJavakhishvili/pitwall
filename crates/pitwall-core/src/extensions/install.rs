//! Installing extensions (docs/spec/extensions.md, "Installing"): the app
//! carries copies of some extension folders (its bundle), and an extension
//! whose manifest names a `source` gets its releases from a git repository
//! ([`super::source`]) or an npm package ([`super::npm`]). Settings lists
//! those not yet in
//! `<data>/extensions/` and copies one there on Install, updates it when a
//! newer copy is offered (the newest wins, never a downgrade), goes back
//! to the copy before the last update, and removes it. Nothing here runs
//! an extension: an installed one is off until the user turns it on.
//!
//! A folder is always written next to its place and renamed into it, so a
//! failed copy leaves nothing half-installed. Pitwall records what it
//! installed in the folder ([`MARKER`]: the version, a digest of the
//! files, and where it came from: Pitwall's copy, a source's tag and
//! commit, or a package's version and integrity), so it can tell a folder the user changed by hand and never
//! replaces one silently.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{read, Manifest};
#[cfg(test)]
use super::MANIFEST;

/// What Pitwall installed, in the installed folder (not part of the digest).
pub const MARKER: &str = ".pitwall-install.json";

/// Never copied, never part of the digest: what a package manager or the
/// system adds.
pub(crate) const SKIPPED: [&str; 3] = ["node_modules", ".git", ".DS_Store"];

/// Where an offered or installed copy came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The copy the app ships.
    Bundled,
    /// A release of a git source: the repository, the extension's folder
    /// in it, the tag and the commit it pointed at.
    Git { url: String, dir: String, tag: String, commit: String },
    /// A version of an npm package: the package, its registry, the version
    /// and the tarball's integrity (`sha512-…`) the registry gave.
    Npm { package: String, registry: String, version: String, integrity: String },
}

impl Origin {
    /// Its tag and commit, for a git one.
    pub fn tag_commit(&self) -> Option<(&str, &str)> {
        match self {
            Origin::Git { tag, commit, .. } => Some((tag, commit)),
            _ => None,
        }
    }

    /// Its version and integrity, for an npm one.
    pub fn version_integrity(&self) -> Option<(&str, &str)> {
        match self {
            Origin::Npm { version, integrity, .. } => Some((version, integrity)),
            _ => None,
        }
    }

    /// Whether `other` comes from the same place (the same repository
    /// and folder, the same package of the same registry, or both
    /// Pitwall's copy).
    pub fn same_place(&self, other: &Origin) -> bool {
        match (self, other) {
            (Origin::Bundled, Origin::Bundled) => true,
            (Origin::Git { url: a, dir: da, .. }, Origin::Git { url: b, dir: db, .. }) => same_repo(a, b) && da == db,
            (Origin::Npm { package: a, registry: ra, .. }, Origin::Npm { package: b, registry: rb, .. }) => a == b && same_registry(ra, rb),
            _ => false,
        }
    }

    /// Whether it is a release of source `s` (the same repository, or the
    /// same package of the same registry).
    pub fn is_from(&self, s: &super::Source) -> bool {
        match (self, s) {
            (Origin::Git { url, .. }, super::Source::Git { git, .. }) => same_repo(url, git),
            (Origin::Npm { package, registry, .. }, super::Source::Npm { package: p, registry: r }) => package == p && same_registry(registry, r),
            _ => false,
        }
    }

    /// In a few words: "Pitwall's copy", the repository, its folder, tag
    /// and short commit, or the package, version and short integrity.
    pub fn describe(&self) -> String {
        match self {
            Origin::Bundled => "Comes with Pitwall".into(),
            Origin::Git { url, dir, tag, commit } => {
                let at = if dir.is_empty() { String::new() } else { format!(" ({dir})") };
                format!("{}{at}, tag {tag}, commit {}", repo_name(url), super::source::short(commit))
            }
            Origin::Npm { package, registry, version, integrity } => format!("{}, version {version}, {}", npm_name(package, registry), super::npm::short(integrity)),
        }
    }

    /// Where it comes from, without the release: "with this Pitwall",
    /// "github.com/me/r", "npm: @me/x".
    pub fn place(&self) -> String {
        match self {
            Origin::Bundled => "with this Pitwall".into(),
            Origin::Git { url, .. } => repo_name(url),
            Origin::Npm { package, registry, .. } => npm_name(package, registry),
        }
    }
}

/// Repository `url` without `https://` and `.git`, for reading.
pub fn repo_name(url: &str) -> String {
    url.trim_start_matches("https://").trim_end_matches('/').trim_end_matches(".git").to_string()
}

/// Package `package` of `registry`, for reading: "npm: @me/x", with the
/// registry when it isn't npm's own ("npm: @me/x (npm.example.com)").
pub fn npm_name(package: &str, registry: &str) -> String {
    if same_registry(registry, super::NPM_REGISTRY) {
        format!("npm: {package}")
    } else {
        format!("npm: {package} ({})", repo_name(registry))
    }
}

/// Where source `s`'s releases are, in a line: "example.com/r (ext), tags
/// ext-v{version}", or "npm: @me/x, its versions".
pub fn source_line(s: &super::Source) -> String {
    match s {
        super::Source::Git { git, dir, tags } => format!("{}{}, tags {tags}", repo_name(git), if dir.is_empty() { String::new() } else { format!(" ({dir})") }),
        super::Source::Npm { package, registry } => format!("{}, its versions", npm_name(package, registry)),
    }
}

/// Where source `s`'s releases are, in a few words: "example.com/r",
/// "npm: @me/x".
pub fn source_place(s: &super::Source) -> String {
    match s {
        super::Source::Git { git, .. } => repo_name(git),
        super::Source::Npm { package, registry } => npm_name(package, registry),
    }
}

/// Whether repository addresses `a` and `b` are the same repository.
pub fn same_repo(a: &str, b: &str) -> bool {
    repo_name(a).eq_ignore_ascii_case(&repo_name(b))
}

/// Whether registry addresses `a` and `b` are the same registry.
pub fn same_registry(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

/// An extension ready to install: its manifest, its folder (in the bundle,
/// or fetched from a source) and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub manifest: Manifest,
    /// Its folder, to copy from.
    pub folder: PathBuf,
    pub origin: Origin,
}

/// The usable extensions in bundle folder `bundle`: by name, demos last.
pub fn bundled(bundle: &Path) -> Vec<Offer> {
    let mut out: Vec<Offer> = super::discover(bundle)
        .into_iter()
        .filter_map(|f| {
            let m = f.manifest.ok()?;
            Some(Offer { folder: bundle.join(&f.folder), manifest: m, origin: Origin::Bundled })
        })
        .collect();
    out.sort_by(|a, b| a.manifest.demo.cmp(&b.manifest.demo).then_with(|| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase())));
    out
}

/// A bundled extension's place in the data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Not installed: it can be.
    Available,
    /// Installed and as new as the bundled one (or newer).
    Current,
    /// The bundled copy is newer: this version.
    Newer(String),
}

/// What Pitwall can say about installed folder `dir`: whether it is as
/// Pitwall installed it (false for one changed by hand, or put there by
/// hand), the version it has, and where it came from (`None`: put there
/// by hand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub as_installed: bool,
    pub version: Option<String>,
    pub origin: Option<Origin>,
}

/// Bundled extension `offer` against data folder `extensions`.
pub fn status(offer: &Offer, extensions: &Path) -> Status {
    let dir = extensions.join(&offer.manifest.id);
    if !dir.is_dir() {
        return Status::Available;
    }
    let local = local(&dir);
    if is_newer(offer, &local) {
        Status::Newer(offer.manifest.version.clone().unwrap_or_default())
    } else {
        Status::Current
    }
}

/// Whether `offer` is newer than installed `local`. One with no version
/// (put there by hand from an older copy) is older than any version.
fn is_newer(offer: &Offer, local: &Local) -> bool {
    match (&offer.manifest.version, &local.version) {
        (Some(b), Some(l)) => compare_versions(b, l) == Ordering::Greater,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The update to offer for installed `local` among `offers` (the bundled
/// copy, a source's newest release): the newest above it, never a
/// downgrade; between two of the same version, the one from where it was
/// installed from, else Pitwall's own copy (there without a network).
pub fn best<'a>(local: &Local, offers: &'a [Offer]) -> Option<&'a Offer> {
    let rank = |o: &Offer| match &local.origin {
        Some(from) if from.same_place(&o.origin) => 2,
        _ if o.origin == Origin::Bundled => 1,
        _ => 0,
    };
    offers.iter().filter(|o| is_newer(o, local)).max_by(|a, b| {
        let (va, vb) = (a.manifest.version.as_deref().unwrap_or_default(), b.manifest.version.as_deref().unwrap_or_default());
        compare_versions(va, vb).then_with(|| rank(a).cmp(&rank(b)))
    })
}

/// One thing installing or updating changes on this computer: a title and
/// its lines (commands, variables, settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub title: String,
    pub lines: Vec<String>,
}

/// What installing `offer` changes on this computer, against the
/// installed copy (`installed`: its manifest and origin; `None`: a new
/// install, which lists everything). First where it comes from; then its
/// program, the variables it sets on agents, its setup actions' commands,
/// the settings it asks for and where its updates come from: all for a
/// new install, only what differs for an update.
pub fn changes(installed: Option<(&Manifest, Option<&Origin>)>, offer: &Offer) -> Vec<Change> {
    let new = &offer.manifest;
    let mut out = Vec::new();
    let mut push = |title: String, lines: Vec<String>| out.push(Change { title, lines });
    let cmd = |m: &Manifest| m.adapter.as_ref().map(|a| a.command.join(" "));
    let env = |m: &Manifest| -> Vec<String> { m.agent_env.iter().map(|e| format!("{}={}", e.name, e.value)).collect() };
    let steps = |a: &super::Action| -> Vec<String> { a.steps.iter().map(|s| s.run.join(" ")).collect() };
    let setting = |s: &super::SettingDef| match s.kind {
        super::SettingKind::Secret => format!("{} (a secret, kept in the system's secure store)", s.label),
        _ => s.label.clone(),
    };
    let permission = |p: &super::PermissionAsk| match super::permissions::find(&p.name).filter(|k| k.available) {
        // One that acts on agents says what can go wrong.
        Some(k) if k.risk_note.is_some() => format!("{} ({}): {} — {}", k.title, p.name, p.reason, k.risk_note.unwrap_or_default()),
        Some(k) => format!("{} ({}): {}", k.title, p.name, p.reason),
        None => format!("{}: {} ({})", p.name, p.reason, super::permissions::unavailable_why(&p.name).unwrap_or_default()),
    };
    let updates_from = |m: &Manifest| m.source.as_ref().map_or_else(|| "Pitwall's own copy only".into(), source_line);
    let Some((old, old_origin)) = installed else {
        let mut from = vec![offer.origin.describe()];
        if offer.origin != Origin::Bundled {
            from.push("Not reviewed by Pitwall".into());
        }
        push("Comes from:".into(), from);
        match cmd(new) {
            Some(c) => push("While it is on, runs this in its folder:".into(), vec![c]),
            None => push("Runs no program of its own.".into(), vec![]),
        }
        if let Some(s) = &new.service {
            push(format!("Keeps a program running on this computer while it is on, once you confirm it ({}):", s.label), service_lines(s));
        }
        if !new.agent_env.is_empty() {
            push("Sets these on agents you start while it is on:".into(), env(new));
        }
        for a in &new.actions {
            push(format!("“{}” runs, when you press it and confirm:", a.label), steps(a));
        }
        if !new.settings.is_empty() {
            push("Asks you for:".into(), new.settings.iter().map(setting).collect());
        }
        if !new.permissions.is_empty() {
            push("Asks for these permissions (you grant or refuse each when you turn it on):".into(), new.permissions.iter().map(permission).collect());
        }
        let adds = new.contributes.lines();
        if !adds.is_empty() {
            push("Adds to Pitwall, while it is on:".into(), adds);
        }
        if new.source.is_some() {
            push("Its updates come from:".into(), vec![updates_from(new)]);
        }
        return out;
    };
    match old_origin.filter(|o| !o.same_place(&offer.origin)) {
        Some(was) => push("Where it comes from changes:".into(), vec![format!("now: {}", offer.origin.describe()), format!("was: {}", was.describe())]),
        None => push("Comes from:".into(), vec![offer.origin.describe()]),
    }
    if cmd(old) != cmd(new) {
        let shown = |c: Option<String>| c.unwrap_or_else(|| "no program".into());
        push("The program it runs while it is on changes:".into(), vec![format!("now: {}", shown(cmd(new))), format!("was: {}", shown(cmd(old)))]);
    }
    match (&old.service, &new.service) {
        (None, Some(s)) => push(format!("New: it keeps a program running on this computer while it is on ({}), once you confirm it. {SERVICE_RISK}", s.label), service_lines(s)),
        (Some(o), Some(s)) if super::service::digest(o) != super::service::digest(s) => {
            let mut lines: Vec<String> = service_lines(s).into_iter().map(|l| format!("now: {l}")).collect();
            lines.extend(service_lines(o).into_iter().map(|l| format!("was: {l}")));
            push(format!("The program it keeps running changes ({}): it is stopped and asks you to confirm it again. {SERVICE_RISK}", s.label), lines);
        }
        (Some(o), None) => push(format!("It no longer runs {}: it is stopped (its data stays).", o.label), vec![]),
        _ => {}
    }
    let (was, now) = (env(old), env(new));
    let mut lines: Vec<String> = now.iter().filter(|e| !was.contains(e)).map(|e| format!("+ {e}")).collect();
    lines.extend(was.iter().filter(|e| !now.contains(e)).map(|e| format!("- {e}")));
    if !lines.is_empty() {
        push("The variables it sets on agents change:".into(), lines);
    }
    for a in &new.actions {
        match old.actions.iter().find(|o| o.id == a.id) {
            None => push(format!("New: “{}” runs, when you press it and confirm:", a.label), steps(a)),
            Some(o) if steps(o) != steps(a) => {
                let again = if a.setup { " (its step is unticked: run it again)" } else { "" };
                push(format!("“{}” runs other commands{again}:", a.label), steps(a));
            }
            Some(_) => {}
        }
    }
    for a in old.actions.iter().filter(|o| !new.actions.iter().any(|a| a.id == o.id)) {
        push(format!("“{}” is gone.", a.label), vec![]);
    }
    let added: Vec<String> = new.settings.iter().filter(|s| !old.settings.iter().any(|o| o.key == s.key)).map(setting).collect();
    if !added.is_empty() {
        push("It asks you for more:".into(), added);
    }
    let dropped: Vec<String> = old.settings.iter().filter(|o| !new.settings.iter().any(|s| s.key == o.key)).map(setting).collect();
    if !dropped.is_empty() {
        push("It no longer asks for:".into(), dropped);
    }
    let more: Vec<String> = new.permissions.iter().filter(|p| !old.permissions.iter().any(|o| o.name == p.name)).map(permission).collect();
    if !more.is_empty() {
        push("It asks for more permissions (not granted until you allow them):".into(), more);
    }
    let fewer: Vec<String> = old.permissions.iter().filter(|o| !new.permissions.iter().any(|p| p.name == o.name)).map(|o| o.name.clone()).collect();
    if !fewer.is_empty() {
        push("It no longer asks for these permissions (their grants are forgotten):".into(), fewer);
    }
    let (was, now) = (old.contributes.lines(), new.contributes.lines());
    let mut lines: Vec<String> = now.iter().filter(|l| !was.contains(l)).map(|l| format!("+ {l}")).collect();
    lines.extend(was.iter().filter(|l| !now.contains(l)).map(|l| format!("- {l}")));
    if !lines.is_empty() {
        push("What it adds to Pitwall changes:".into(), lines);
    }
    if old.source != new.source {
        push("Its updates will come from:".into(), vec![updates_from(new)]);
    }
    out
}

/// What a service is, said where it is added or changed.
pub const SERVICE_RISK: &str = "It keeps running as you, with your files, for as long as the extension is on and Pitwall runs.";

/// A service's commands as the manifest has them: what it runs, how its
/// version is checked, its first run, its variables.
fn service_lines(s: &super::service::Service) -> Vec<String> {
    let mut out = vec![s.run.join(" ")];
    if let Some(v) = &s.version {
        out.push(format!("version: {}", v.run.join(" ")));
    }
    if let Some(f) = &s.first_run {
        out.push(format!("{}: {}", f.label, f.run.join(" ")));
    }
    for e in &s.env {
        out.push(match &e.from {
            super::service::EnvFrom::Value(v) => format!("{}={v}", e.name),
            super::service::EnvFrom::Secret(k) => format!("{} = its secret “{k}”", e.name),
        });
    }
    out
}

/// What installed folder `dir` is.
pub fn local(dir: &Path) -> Local {
    local_as(dir, &dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// What folder `dir`, holding extension `id`, is.
fn local_as(dir: &Path, id: &str) -> Local {
    let version = read(dir, id).ok().and_then(|m| m.version);
    let marker: Option<Value> = std::fs::read_to_string(dir.join(MARKER)).ok().and_then(|t| serde_json::from_str(&t).ok());
    let as_installed = marker.as_ref().and_then(|m| m["digest"].as_str()).is_some_and(|d| digest(dir).as_deref() == Ok(d));
    Local { as_installed, version, origin: marker.as_ref().map(marker_origin) }
}

/// Where installed folder `dir` came from, by its marker alone (quick: no
/// digest); `None` for one put there by hand.
pub fn origin(dir: &Path) -> Option<Origin> {
    let marker: Value = std::fs::read_to_string(dir.join(MARKER)).ok().and_then(|t| serde_json::from_str(&t).ok())?;
    Some(marker_origin(&marker))
}

fn marker_origin(m: &Value) -> Origin {
    if let (Some(package), Some(version), Some(integrity)) = (m["source"]["npm"].as_str(), m["version"].as_str(), m["integrity"].as_str()) {
        let registry = m["source"]["registry"].as_str().unwrap_or(super::NPM_REGISTRY);
        return Origin::Npm { package: package.to_string(), registry: registry.to_string(), version: version.to_string(), integrity: integrity.to_string() };
    }
    match (&m["source"], m["tag"].as_str(), m["commit"].as_str()) {
        (Value::Object(s), Some(tag), Some(commit)) => Origin::Git {
            url: s.get("git").and_then(Value::as_str).unwrap_or_default().to_string(),
            dir: s.get("dir").and_then(Value::as_str).unwrap_or_default().to_string(),
            tag: tag.to_string(),
            commit: commit.to_string(),
        },
        // Written before sources (only Pitwall's copy was installed), or
        // Pitwall's copy.
        _ => Origin::Bundled,
    }
}

/// The copy kept by the last update of extension `id`, for "Go back"
/// (`None` when there is none).
pub fn previous(extensions: &Path, id: &str) -> Option<Local> {
    let dir = previous_dir(extensions, id);
    dir.is_dir().then(|| local_as(&dir, id))
}

/// Where the copy before the last update of `id` is kept (hidden).
fn previous_dir(extensions: &Path, id: &str) -> PathBuf {
    extensions.join(format!(".{id}.previous"))
}

/// Order of versions `a` and `b` ("1.2.10" > "1.2.9"; "1.0.0-beta" <
/// "1.0.0"): dot-separated parts, numbers by value, others as text.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| -> (String, Option<String>) {
        let v = v.trim().trim_start_matches('v');
        let v = v.split('+').next().unwrap_or_default();
        match v.split_once('-') {
            Some((core, pre)) => (core.to_string(), Some(pre.to_string())),
            None => (v.to_string(), None),
        }
    };
    let parts = |s: &str| -> Vec<String> { s.split('.').map(str::to_string).collect() };
    let cmp_parts = |x: &[String], y: &[String]| -> Ordering {
        for i in 0..x.len().max(y.len()) {
            let (p, q) = (x.get(i).map_or("0", String::as_str), y.get(i).map_or("0", String::as_str));
            let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                (Ok(m), Ok(n)) => m.cmp(&n),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                _ => p.cmp(q),
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    };
    let ((ca, pa), (cb, pb)) = (split(a), split(b));
    cmp_parts(&parts(&ca), &parts(&cb)).then_with(|| match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => cmp_parts(&parts(&x), &parts(&y)),
    })
}

/// Why an update wasn't made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The installed folder isn't as Pitwall installed it: replacing it
    /// would lose the user's changes (asked first).
    ChangedLocally,
    Failed(String),
}

/// Copy bundled `offer` into `extensions/<id>/`. It must not be there.
pub fn install(offer: &Offer, extensions: &Path) -> Result<(), String> {
    let id = &offer.manifest.id;
    let target = extensions.join(id);
    if target.exists() {
        return Err(format!("{} is already installed", offer.manifest.name));
    }
    std::fs::create_dir_all(extensions).map_err(|e| format!("Can't make {}: {e}", extensions.display()))?;
    let staged = stage(offer, extensions)?;
    std::fs::rename(&staged, &target).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staged);
        format!("Can't put it in place: {e}")
    })
}

/// Replace installed `extensions/<id>/` with `offer` (its settings and
/// secrets aren't in the folder: they stay); the installed copy is kept
/// as the previous one, for [`rollback`] (one step back: an older previous
/// one goes). A folder that isn't as Pitwall installed it is replaced only
/// when `replace_changed`. Refused: a release whose tag the installed copy
/// came from at another commit (the tag moved), or a version of the npm
/// package it came from with another integrity than the installed one.
pub fn update(offer: &Offer, extensions: &Path, replace_changed: bool) -> Result<(), Refused> {
    let id = &offer.manifest.id;
    let target = extensions.join(id);
    if !target.is_dir() {
        return Err(Refused::Failed(format!("{} isn't installed", offer.manifest.name)));
    }
    let now = local(&target);
    match (&now.origin, &offer.origin) {
        (Some(Origin::Git { url, tag, commit, .. }), Origin::Git { url: new_url, tag: new_tag, commit: new_commit, .. }) if same_repo(url, new_url) && tag == new_tag && !commit.eq_ignore_ascii_case(new_commit) => {
            return Err(Refused::Failed(format!("The {tag} tag moved: it pointed at {} and now points at {}. Not installed.", super::source::short(commit), super::source::short(new_commit))));
        }
        (Some(was @ Origin::Npm { version, integrity, .. }), Origin::Npm { version: new_version, integrity: new_integrity, package, .. }) if was.same_place(&offer.origin) && version == new_version && integrity != new_integrity => {
            return Err(Refused::Failed(super::npm::changed_text(package, version, integrity, new_integrity)));
        }
        _ => {}
    }
    if !replace_changed && !now.as_installed {
        return Err(Refused::ChangedLocally);
    }
    let staged = stage(offer, extensions).map_err(Refused::Failed)?;
    swap(extensions, id, &staged, true)
}

/// Put staged folder `staged` in place of installed `extensions/<id>/`:
/// the installed one moved aside, the staged one moved in (the installed
/// one back if that fails); the installed one then kept as the previous
/// copy (`keep`) or deleted.
fn swap(extensions: &Path, id: &str, staged: &Path, keep: bool) -> Result<(), Refused> {
    let target = extensions.join(id);
    let old = sibling(extensions, id, "old");
    if let Err(e) = rename_retrying(&target, &old) {
        let _ = std::fs::remove_dir_all(staged);
        return Err(Refused::Failed(format!("Can't move the installed copy aside: {e}")));
    }
    if let Err(e) = rename_retrying(staged, &target) {
        let _ = std::fs::rename(&old, &target);
        let _ = std::fs::remove_dir_all(staged);
        return Err(Refused::Failed(format!("Can't put it in place: {e}")));
    }
    if keep {
        let prev = previous_dir(extensions, id);
        let _ = std::fs::remove_dir_all(&prev);
        if rename_retrying(&old, &prev).is_ok() {
            return Ok(());
        }
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// "Go back": the copy kept by the last update of `id` put back in place
/// of the installed one (which goes). A folder that isn't as Pitwall
/// installed it is replaced only when `replace_changed`.
pub fn rollback(extensions: &Path, id: &str, replace_changed: bool) -> Result<(), Refused> {
    let prev = previous_dir(extensions, id);
    if !prev.is_dir() {
        return Err(Refused::Failed("There is no earlier copy to go back to.".into()));
    }
    read(&prev, id).map_err(|e| Refused::Failed(format!("The earlier copy can't be used: {e}")))?;
    let target = extensions.join(id);
    if !target.is_dir() {
        return Err(Refused::Failed("It isn't installed.".into()));
    }
    if !replace_changed && !local(&target).as_installed {
        return Err(Refused::ChangedLocally);
    }
    // Moved aside first, so a failed swap leaves it where it was.
    let staged = sibling(extensions, id, "back");
    rename_retrying(&prev, &staged).map_err(|e| Refused::Failed(format!("Can't take the earlier copy: {e}")))?;
    swap(extensions, id, &staged, false).inspect_err(|_| {
        let _ = std::fs::rename(&staged, &prev);
    })
}

/// Delete installed `extensions/<id>/` and the copy kept for "Go back":
/// moved aside first (so it is gone from the list at once, even if
/// deleting takes a moment), then deleted.
pub fn remove(extensions: &Path, id: &str) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(previous_dir(extensions, id));
    let target = extensions.join(id);
    if !target.is_dir() {
        return Ok(());
    }
    let gone = sibling(extensions, id, "removing");
    rename_retrying(&target, &gone).map_err(|e| format!("Can't remove it: {e}"))?;
    std::fs::remove_dir_all(&gone).map_err(|e| format!("Can't delete {}: {e}", gone.display()))
}

/// A hidden sibling of `extensions/<id>` (hidden folders aren't listed).
fn sibling(extensions: &Path, id: &str, what: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or_default();
    extensions.join(format!(".{id}.{what}-{}-{nanos}", std::process::id()))
}

/// A copy of `offer` with its marker, in a hidden sibling of its place.
fn stage(offer: &Offer, extensions: &Path) -> Result<PathBuf, String> {
    let id = &offer.manifest.id;
    // The copy must be usable as installed.
    let usable = match &offer.origin {
        Origin::Bundled => read(&offer.folder, id).map(|_| ()).map_err(|e| format!("The bundled copy can't be used: {e}")),
        // A fetched folder isn't named after its id.
        Origin::Git { .. } | Origin::Npm { .. } => match std::fs::read_to_string(offer.folder.join(super::MANIFEST)).map_err(|e| e.to_string()).and_then(|t| super::parse(&t)) {
            Ok(m) if m.id == *id => Ok(()),
            Ok(m) => Err(format!("The fetched copy is “{}”, not “{id}”", m.id)),
            Err(e) => Err(format!("The fetched copy can't be used: {e}")),
        },
    };
    usable?;
    let staged = sibling(extensions, id, "installing");
    let done = copy_tree(&offer.folder, &staged).and_then(|()| {
        let d = digest(&staged)?;
        let marker = match &offer.origin {
            Origin::Bundled => json!({ "version": offer.manifest.version, "digest": d, "source": "bundled" }),
            Origin::Git { url, dir, tag, commit } => json!({ "version": offer.manifest.version, "digest": d, "source": { "git": url, "dir": dir }, "tag": tag, "commit": commit }),
            // The package's version, which its manifest's must equal.
            Origin::Npm { package, registry, version, integrity } => json!({ "version": version, "digest": d, "source": { "npm": package, "registry": registry }, "integrity": integrity }),
        };
        std::fs::write(staged.join(MARKER), format!("{marker:#}\n")).map_err(|e| e.to_string())
    });
    match done {
        Ok(()) => Ok(staged),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            Err(format!("Can't copy it: {e}"))
        }
    }
}

/// A folder renamed; on Windows a moment later again while something
/// (an adapter just stopped, a virus scanner) still holds it.
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if tries < 20 && cfg!(windows) => {
                let _ = e;
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) || name == MARKER {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let dest = to.join(&name);
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &dest).map_err(|e| format!("{}: {e}", entry.path().display()))?;
        }
        // Links aren't copied: an extension is plain files.
    }
    Ok(())
}

/// A digest of folder `dir`'s files (names and contents; not the marker,
/// not what [`SKIPPED`] names): FNV-1a, enough to notice a change.
pub fn digest(dir: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    list(dir, Path::new(""), &mut files)?;
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for rel in files {
        let bytes = std::fs::read(dir.join(&rel)).map_err(|e| e.to_string())?;
        eat(rel.as_bytes());
        eat(&[0]);
        eat(&(bytes.len() as u64).to_le_bytes());
        eat(&bytes);
    }
    Ok(format!("{h:016x}"))
}

fn list(root: &Path, rel: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(root.join(rel)).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) || (rel.as_os_str().is_empty() && name == MARKER) {
            continue;
        }
        let path = rel.join(&name);
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            list(root, &path, out)?;
        } else if kind.is_file() {
            // `/` on every system, so a digest doesn't depend on it.
            out.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn ext(root: &Path, id: &str, version: Option<&str>, demo: bool) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("adapter/node_modules/x")).unwrap();
        let v = version.map(|v| format!(r#", "version": "{v}""#)).unwrap_or_default();
        std::fs::write(dir.join(MANIFEST), format!(r#"{{"id": "{id}", "name": "{}", "demo": {demo}{v}}}"#, id.to_uppercase())).unwrap();
        std::fs::write(dir.join("adapter/main.mjs"), "// main").unwrap();
        std::fs::write(dir.join("adapter/node_modules/x/big.js"), "// not copied").unwrap();
        dir
    }

    #[test]
    fn versions_are_ordered_by_their_parts() {
        assert_eq!(compare_versions("1.2.10", "1.2.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("v0.5.0", "0.4.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.4.0-beta.2", "0.4.0-beta.10"), Ordering::Less);
    }

    /// The bundle lists its usable extensions by name, demos last.
    #[test]
    fn the_bundle_lists_its_extensions_demos_last() {
        let t = TempDir::new("ext-bundle");
        ext(t.path(), "zeta", None, false);
        ext(t.path(), "alpha", None, true);
        ext(t.path(), "beta", None, false);
        std::fs::create_dir_all(t.path().join("broken")).unwrap();
        let ids: Vec<String> = bundled(t.path()).into_iter().map(|o| o.manifest.id).collect();
        assert_eq!(ids, ["beta", "zeta", "alpha"]);
    }

    /// Install copies the folder in (without node_modules), leaves no
    /// staging folder behind and refuses to install twice; the copy is as
    /// installed until a file changes.
    #[test]
    fn install_update_and_remove() {
        let t = TempDir::new("ext-install");
        let (bundle, data) = (t.path().join("bundle"), t.path().join("data/extensions"));
        ext(&bundle, "inbox", Some("1.0.0"), false);
        let offer = bundled(&bundle).remove(0);
        assert_eq!(status(&offer, &data), Status::Available);
        install(&offer, &data).unwrap();
        let dir = data.join("inbox");
        assert!(dir.join("adapter/main.mjs").is_file());
        assert!(!dir.join("adapter/node_modules").exists(), "node_modules isn't copied");
        let names: Vec<String> = std::fs::read_dir(&data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["inbox"], "nothing staged is left");
        assert_eq!(status(&offer, &data), Status::Current);
        assert_eq!(local(&dir), Local { as_installed: true, version: Some("1.0.0".into()), origin: Some(Origin::Bundled) });
        assert_eq!(previous(&data, "inbox"), None);
        assert!(install(&offer, &data).unwrap_err().contains("already installed"));
        // What npm adds doesn't count as a change.
        std::fs::create_dir_all(dir.join("adapter/node_modules/y")).unwrap();
        std::fs::write(dir.join("adapter/node_modules/y/a.js"), "").unwrap();
        assert!(local(&dir).as_installed);

        // A newer bundled copy: an update, which keeps the old one aside
        // (hidden, so not listed) for Go back.
        ext(&bundle, "inbox", Some("1.1.0"), false);
        std::fs::write(bundle.join("inbox/adapter/main.mjs"), "// main 1.1").unwrap();
        let offer = bundled(&bundle).remove(0);
        assert_eq!(status(&offer, &data), Status::Newer("1.1.0".into()));
        update(&offer, &data, false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("adapter/main.mjs")).unwrap(), "// main 1.1");
        assert_eq!(status(&offer, &data), Status::Current);
        let names = |data: &Path| {
            let mut n: Vec<String> = std::fs::read_dir(data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            n.sort();
            n
        };
        assert_eq!(names(&data), [".inbox.previous", "inbox"], "the old copy is kept, hidden");
        assert_eq!(super::super::discover(&data).len(), 1);
        assert_eq!(previous(&data, "inbox").and_then(|p| p.version).as_deref(), Some("1.0.0"));

        // Changed by hand: an update is refused until the user says so.
        std::fs::write(dir.join("adapter/main.mjs"), "// my fix").unwrap();
        assert!(!local(&dir).as_installed);
        ext(&bundle, "inbox", Some("1.2.0"), false);
        let offer = bundled(&bundle).remove(0);
        assert_eq!(update(&offer, &data, false), Err(Refused::ChangedLocally));
        assert_eq!(std::fs::read_to_string(dir.join("adapter/main.mjs")).unwrap(), "// my fix", "left as it was");
        update(&offer, &data, true).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("adapter/main.mjs")).unwrap(), "// main");
        assert_eq!(previous(&data, "inbox").and_then(|p| p.version).as_deref(), Some("1.1.0"), "one step back: the copy before this update");

        remove(&data, "inbox").unwrap();
        assert_eq!(std::fs::read_dir(&data).unwrap().count(), 0, "the kept copy goes too");
        assert_eq!(status(&offer, &data), Status::Available);
    }

    /// Go back: the copy before the last update returns, as Pitwall
    /// installed it, and there is then nothing more to go back to.
    #[test]
    fn go_back_restores_the_copy_before_the_update() {
        let t = TempDir::new("ext-rollback");
        let (bundle, data) = (t.path().join("bundle"), t.path().join("data"));
        ext(&bundle, "inbox", Some("1.0.0"), false);
        install(&bundled(&bundle).remove(0), &data).unwrap();
        assert_eq!(rollback(&data, "inbox", false), Err(Refused::Failed("There is no earlier copy to go back to.".into())));
        ext(&bundle, "inbox", Some("1.1.0"), false);
        std::fs::write(bundle.join("inbox/adapter/main.mjs"), "// 1.1").unwrap();
        update(&bundled(&bundle).remove(0), &data, false).unwrap();
        // Changed by hand since: asked first.
        std::fs::write(data.join("inbox/adapter/main.mjs"), "// mine").unwrap();
        assert_eq!(rollback(&data, "inbox", false), Err(Refused::ChangedLocally));
        rollback(&data, "inbox", true).unwrap();
        let l = local(&data.join("inbox"));
        assert_eq!((l.version.as_deref(), l.as_installed), (Some("1.0.0"), true));
        assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// main");
        assert_eq!(previous(&data, "inbox"), None);
        assert_eq!(std::fs::read_dir(&data).unwrap().count(), 1, "nothing else left");
        // And the newer one is offered again.
        assert_eq!(status(&bundled(&bundle).remove(0), &data), Status::Newer("1.1.0".into()));
    }

    fn git_offer(dir: &Path, id: &str, version: &str, tag: &str, commit: &str) -> Offer {
        ext(dir, id, Some(version), false);
        let manifest = read(&dir.join(id), id).unwrap();
        Offer { manifest, folder: dir.join(id), origin: Origin::Git { url: "https://example.com/r".into(), dir: "ext".into(), tag: tag.into(), commit: commit.into() } }
    }

    /// A release from a source: its marker says the source, tag and
    /// commit; the same tag at another commit later is refused.
    #[test]
    fn a_release_records_its_source_and_a_moved_tag_is_refused() {
        let t = TempDir::new("ext-git-origin");
        let data = t.path().join("data");
        let offer = git_offer(&t.path().join("f1"), "inbox", "1.0.0", "t-v1.0.0", "aaaaaaaaaa");
        install(&offer, &data).unwrap();
        let l = local(&data.join("inbox"));
        assert_eq!(l.origin, Some(offer.origin.clone()));
        assert!(l.as_installed);
        let marker: Value = serde_json::from_str(&std::fs::read_to_string(data.join("inbox").join(MARKER)).unwrap()).unwrap();
        assert_eq!((marker["source"]["git"].as_str(), marker["tag"].as_str(), marker["commit"].as_str()), (Some("https://example.com/r"), Some("t-v1.0.0"), Some("aaaaaaaaaa")));
        let moved = git_offer(&t.path().join("f2"), "inbox", "1.0.0", "t-v1.0.0", "bbbbbbbbbb");
        match update(&moved, &data, true) {
            Err(Refused::Failed(why)) => assert!(why.contains("tag moved"), "{why}"),
            other => panic!("{other:?}"),
        }
        let newer = git_offer(&t.path().join("f3"), "inbox", "1.1.0", "t-v1.1.0", "cccccccccc");
        update(&newer, &data, false).unwrap();
        assert_eq!(local(&data.join("inbox")).origin.and_then(|o| o.tag_commit().map(|(t, c)| (t.to_string(), c.to_string()))), Some(("t-v1.1.0".into(), "cccccccccc".into())));
        assert_eq!(previous(&data, "inbox").and_then(|p| p.origin), Some(offer.origin));
        // A fetched folder of another extension isn't installed under this id.
        let mut wrong = git_offer(&t.path().join("f4"), "other", "2.0.0", "t-v2.0.0", "dddddddddd");
        wrong.manifest.id = "inbox".into();
        assert_eq!(update(&wrong, &data, false), Err(Refused::Failed("The fetched copy is “other”, not “inbox”".into())));
    }

    fn npm_offer(dir: &Path, id: &str, version: &str, integrity: &str) -> Offer {
        ext(dir, id, Some(version), false);
        let manifest = read(&dir.join(id), id).unwrap();
        Offer { manifest, folder: dir.join(id), origin: Origin::Npm { package: "@me/inbox".into(), registry: super::super::NPM_REGISTRY.into(), version: version.into(), integrity: integrity.into() } }
    }

    /// A version from an npm package: its marker says the package,
    /// registry, version and integrity; the same version with another
    /// integrity later is refused; a newer one updates and keeps the
    /// installed copy for Go back.
    #[test]
    fn an_npm_version_records_its_source_and_a_changed_one_is_refused() {
        let t = TempDir::new("ext-npm-origin");
        let data = t.path().join("data");
        let (i1, i2) = (super::super::npm::integrity_of(b"1"), super::super::npm::integrity_of(b"2"));
        let offer = npm_offer(&t.path().join("f1"), "inbox", "1.0.0", &i1);
        install(&offer, &data).unwrap();
        let l = local(&data.join("inbox"));
        assert_eq!(l.origin, Some(offer.origin.clone()));
        assert!(l.as_installed);
        let marker: Value = serde_json::from_str(&std::fs::read_to_string(data.join("inbox").join(MARKER)).unwrap()).unwrap();
        assert_eq!((marker["source"]["npm"].as_str(), marker["source"]["registry"].as_str(), marker["version"].as_str(), marker["integrity"].as_str()), (Some("@me/inbox"), Some("https://registry.npmjs.org"), Some("1.0.0"), Some(i1.as_str())));
        assert!(offer.origin.describe().starts_with("npm: @me/inbox, version 1.0.0, sha512 "), "{}", offer.origin.describe());
        assert_eq!(offer.origin.place(), "npm: @me/inbox");
        assert!(offer.origin.is_from(&super::super::Source::Npm { package: "@me/inbox".into(), registry: "https://registry.npmjs.org/".into() }));
        assert!(!offer.origin.is_from(&super::super::Source::Npm { package: "@me/other".into(), registry: super::super::NPM_REGISTRY.into() }));
        let elsewhere = Origin::Npm { package: "@me/inbox".into(), registry: "https://npm.example.com".into(), version: "1.0.0".into(), integrity: i1.clone() };
        assert!(!offer.origin.same_place(&elsewhere) && elsewhere.place() == "npm: @me/inbox (npm.example.com)");
        let changed = npm_offer(&t.path().join("f2"), "inbox", "1.0.0", &i2);
        match update(&changed, &data, true) {
            Err(Refused::Failed(why)) => assert!(why.contains("Version 1.0.0 of @me/inbox changed on the registry"), "{why}"),
            other => panic!("{other:?}"),
        }
        let newer = npm_offer(&t.path().join("f3"), "inbox", "1.1.0", &i2);
        update(&newer, &data, false).unwrap();
        assert_eq!(local(&data.join("inbox")).origin.and_then(|o| o.version_integrity().map(|(v, i)| (v.to_string(), i.to_string()))), Some(("1.1.0".into(), i2.clone())));
        assert_eq!(previous(&data, "inbox").and_then(|p| p.origin), Some(offer.origin.clone()));
        rollback(&data, "inbox", false).unwrap();
        assert_eq!(local(&data.join("inbox")).origin, Some(offer.origin.clone()));
        // From a git release to an npm one: where it comes from changes.
        let m = read(&data.join("inbox"), "inbox").unwrap();
        let git = Origin::Git { url: "https://example.com/r".into(), dir: "ext".into(), tag: "t-v1.0.0".into(), commit: "aaaaaaa".into() };
        let c = changes(Some((&m, Some(&git))), &newer);
        assert_eq!(c[0].title, "Where it comes from changes:");
        assert!(c[0].lines[0].starts_with("now: npm: @me/inbox, version 1.1.0"), "{:?}", c[0].lines);
        let fresh = changes(None, &newer);
        assert_eq!(fresh[0].lines[1], "Not reviewed by Pitwall");
    }

    /// Bundled and source offers together: the newest wins, never a
    /// downgrade; on a tie, the place it was installed from.
    #[test]
    fn the_newest_offer_wins_and_never_a_downgrade() {
        let t = TempDir::new("ext-best");
        let bundled_at = |v: &str| Offer { origin: Origin::Bundled, ..git_offer(&t.path().join(format!("b{v}")), "inbox", v, "", "") };
        let git_at = |v: &str| git_offer(&t.path().join(format!("g{v}")), "inbox", v, &format!("t-v{v}"), "aaaaaaa");
        let from_git = Local { as_installed: true, version: Some("1.0.0".into()), origin: Some(git_at("1.0.0").origin) };
        let from_pitwall = Local { origin: Some(Origin::Bundled), ..from_git.clone() };
        let ver = |o: Option<&Offer>| o.map(|o| (o.manifest.version.clone().unwrap(), o.origin == Origin::Bundled));
        assert_eq!(ver(best(&from_pitwall, &[bundled_at("1.1.0"), git_at("1.2.0")])), Some(("1.2.0".into(), false)));
        assert_eq!(ver(best(&from_pitwall, &[bundled_at("1.3.0"), git_at("1.2.0")])), Some(("1.3.0".into(), true)));
        assert_eq!(ver(best(&from_pitwall, &[bundled_at("1.2.0"), git_at("1.2.0")])), Some(("1.2.0".into(), true)), "a tie: where it came from");
        assert_eq!(ver(best(&from_git, &[bundled_at("1.2.0"), git_at("1.2.0")])), Some(("1.2.0".into(), false)));
        assert_eq!(best(&from_git, &[bundled_at("0.9.0"), git_at("1.0.0")]), None, "never a downgrade, nor the same");
        let by_hand = Local { as_installed: false, version: None, origin: None };
        assert_eq!(ver(best(&by_hand, &[bundled_at("0.1.0")])), Some(("0.1.0".into(), true)));
    }

    /// What changes on this computer: everything for a new install, only
    /// the differences for an update.
    #[test]
    fn what_changes_is_listed() {
        let m = |extra: &str| super::super::parse(&format!(r#"{{"id": "x", "name": "X", "version": "1.0.0", {extra}}}"#)).unwrap();
        let old = m(r#""settings": [{"key": "server", "type": "url", "label": "Server"}],
            "adapter": {"command": ["node", "main.mjs"]},
            "agentEnv": [{"name": "X_URL", "value": "{setting:server}"}, {"name": "X_AGENT", "value": "{project}"}],
            "actions": [{"id": "setup", "setup": true, "label": "Set up", "description": "d", "steps": [{"run": ["tool", "add", "x#v0.4.0"]}]},
                        {"id": "tidy", "label": "Tidy", "description": "d", "steps": [{"run": ["tool", "tidy"]}]}]"#);
        let new = m(r#""settings": [{"key": "server", "type": "url", "label": "Server"}, {"key": "key", "type": "secret", "label": "Key"}],
            "adapter": {"command": ["node", "main.mjs"]},
            "agentEnv": [{"name": "X_URL", "value": "{setting:server}"}, {"name": "X_SESSION", "value": "{agent_id}"}],
            "actions": [{"id": "setup", "setup": true, "label": "Set up", "description": "d", "steps": [{"run": ["tool", "add", "x#v0.5.0"]}]}],
            "source": {"git": "https://example.com/r", "dir": "ext", "tags": "x-v{version}"}"#);
        let origin = Origin::Git { url: "https://example.com/r".into(), dir: "ext".into(), tag: "x-v1.1.0".into(), commit: "0123456789abcdef".into() };
        let offer = Offer { manifest: new.clone(), folder: PathBuf::new(), origin: origin.clone() };
        let fresh = changes(None, &offer);
        assert_eq!(fresh[0], Change { title: "Comes from:".into(), lines: vec!["example.com/r (ext), tag x-v1.1.0, commit 0123456".into(), "Not reviewed by Pitwall".into()] });
        assert!(fresh.iter().any(|c| c.lines == ["node main.mjs"]));
        assert!(fresh.iter().any(|c| c.lines == ["tool add x#v0.5.0"]));
        let up = changes(Some((&old, Some(&Origin::Bundled))), &offer);
        let titles: Vec<&str> = up.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, [
            "Where it comes from changes:",
            "The variables it sets on agents change:",
            "“Set up” runs other commands (its step is unticked: run it again):",
            "“Tidy” is gone.",
            "It asks you for more:",
            "Its updates will come from:",
        ]);
        assert_eq!(up[1].lines, ["+ X_SESSION={agent_id}", "- X_AGENT={project}"]);
        assert_eq!(up[2].lines, ["tool add x#v0.5.0"]);
        assert_eq!(up[4].lines, ["Key (a secret, kept in the system's secure store)"]);
        // The same manifest from the same place: only where it comes from.
        let same = Offer { manifest: old.clone(), folder: PathBuf::new(), origin: Origin::Bundled };
        assert_eq!(changes(Some((&old, Some(&Origin::Bundled))), &same), [Change { title: "Comes from:".into(), lines: vec!["Comes with Pitwall".into()] }]);
    }

    /// Format 2's permissions: all of them, with reasons, for an install;
    /// an update lists the ones it adds (not granted until allowed) and
    /// drops.
    #[test]
    fn permissions_are_part_of_what_changes() {
        let m = |perms: &str| super::super::parse(&format!(r#"{{"v": 2, "id": "x", "name": "X", "version": "1.0.0", "permissions": {{{perms}}}}}"#)).unwrap();
        let old = m(r#""agents.read": "To match runs", "agents.start": "To start one", "weather.read": "Later""#);
        let new = m(r#""agents.read": "To match runs", "projects.read": "To list projects", "agents.queue": "To send failures""#);
        let offer = Offer { manifest: old.clone(), folder: PathBuf::new(), origin: Origin::Bundled };
        let fresh = changes(None, &offer);
        let p = fresh.iter().find(|c| c.title.starts_with("Asks for these permissions")).unwrap();
        assert_eq!(p.lines[0], "Read your agents (agents.read): To match runs");
        assert!(p.lines[1].starts_with("Start agents (agents.start): To start one — An agent it starts runs as you"), "{}", p.lines[1]);
        assert_eq!(p.lines[2], "weather.read: Later (not available in this Pitwall)");
        let up = changes(Some((&old, Some(&Origin::Bundled))), &Offer { manifest: new, ..offer });
        let more = up.iter().find(|c| c.title.starts_with("It asks for more permissions")).unwrap();
        // In either key order (serde_json's `preserve_order` may be on in a
        // workspace build).
        assert!(more.lines.iter().any(|l| l == "Read your projects (projects.read): To list projects"), "{:?}", more.lines);
        assert!(more.lines.iter().any(|l| l.starts_with("Send text to your agents (agents.queue): To send failures — Text an agent gets becomes what it does")), "{:?}", more.lines);
        assert_eq!(more.lines.len(), 2);
        assert!(more.title.contains("not granted until you allow them"));
        assert_eq!(up.iter().find(|c| c.title.starts_with("It no longer asks for these")).unwrap().lines, ["agents.start", "weather.read"]);
    }

    /// A service: what it runs for an install; for an update whose
    /// commands change, both, that it stops and asks again, and the risk.
    #[test]
    fn a_service_is_part_of_what_changes() {
        let m = |pin: &str| super::super::parse(&format!(r#"{{"v": 2, "id": "x", "name": "X", "version": "1.0.0", "service": {{"label": "The server", "run": ["srv", "serve", "--data", "{{data}}"], "version": {{"run": ["srv", "version"], "match": "(\\d+\\.\\d+\\.\\d+)", "requires": ">={pin}"}}}}}}"#)).unwrap();
        let old = m("0.5.0");
        let offer = Offer { manifest: old.clone(), folder: PathBuf::new(), origin: Origin::Bundled };
        let fresh = changes(None, &offer);
        let s = fresh.iter().find(|c| c.title.starts_with("Keeps a program running")).unwrap();
        assert_eq!(s.lines, ["srv serve --data {data}", "version: srv version"]);
        // Only the lowest version it needs moved: the commands are the same.
        let up = changes(Some((&old, Some(&Origin::Bundled))), &Offer { manifest: m("0.6.0"), ..offer.clone() });
        assert!(!up.iter().any(|c| c.title.contains("keeps running")), "{up:?}");
        let mut new = m("0.6.0");
        new.service.as_mut().unwrap().run.push("--quiet".into());
        let up = changes(Some((&old, Some(&Origin::Bundled))), &Offer { manifest: new, ..offer.clone() });
        let c = up.iter().find(|c| c.title.starts_with("The program it keeps running changes")).unwrap();
        assert!(c.title.contains("asks you to confirm it again") && c.title.contains(SERVICE_RISK), "{}", c.title);
        assert_eq!(c.lines[0], "now: srv serve --data {data} --quiet");
        assert!(c.lines.contains(&"was: srv serve --data {data}".to_string()));
    }

    /// What it adds to Pitwall (views, commands, badges, notifications):
    /// all of it for an install, what is added and gone for an update.
    #[test]
    fn contributions_are_part_of_what_changes() {
        let m = |c: &str| super::super::parse(&format!(r#"{{"v": 2, "id": "x", "name": "X", "version": "1.0.0", "adapter": {{"command": ["node", "a.mjs"]}}, "permissions": {{"notify": "To tell you"}}, "contributes": {{{c}}}}}"#)).unwrap();
        let old = m(r#""views": [{"id": "runs", "title": "Runs"}], "badges": true"#);
        let new = m(r#""views": [{"id": "runs", "title": "Runs"}], "commands": [{"id": "rerun", "title": "Re-run"}], "notifications": true"#);
        let offer = Offer { manifest: old.clone(), folder: PathBuf::new(), origin: Origin::Bundled };
        let fresh = changes(None, &offer);
        let adds = fresh.iter().find(|c| c.title.starts_with("Adds to Pitwall")).unwrap();
        assert_eq!(adds.lines.len(), 2);
        assert!(adds.lines[0].contains("a view tile “Runs”"));
        let up = changes(Some((&old, Some(&Origin::Bundled))), &Offer { manifest: new, ..offer });
        let c = up.iter().find(|c| c.title.starts_with("What it adds to Pitwall changes")).unwrap();
        assert!(c.lines.iter().any(|l| l.starts_with("+ a command “Re-run”")), "{:?}", c.lines);
        assert!(c.lines.iter().any(|l| l.starts_with("+ notifications")), "{:?}", c.lines);
        assert!(c.lines.iter().any(|l| l.starts_with("- badges")), "{:?}", c.lines);
        assert!(!c.lines.iter().any(|l| l.contains("view tile")), "unchanged: not listed");
    }

    /// A folder put there by hand: not as installed, and older than any
    /// bundled version when it has none.
    #[test]
    fn a_folder_put_in_by_hand_is_never_replaced_silently() {
        let t = TempDir::new("ext-by-hand");
        let (bundle, data) = (t.path().join("bundle"), t.path().join("data"));
        ext(&bundle, "inbox", Some("0.2.0"), false);
        ext(&data, "inbox", None, false);
        let offer = bundled(&bundle).remove(0);
        assert_eq!(status(&offer, &data), Status::Newer("0.2.0".into()));
        assert!(!local(&data.join("inbox")).as_installed);
        assert_eq!(update(&offer, &data, false), Err(Refused::ChangedLocally));
    }

    /// A failed copy leaves neither a half folder nor a staging folder.
    #[test]
    fn a_failed_install_leaves_nothing() {
        let t = TempDir::new("ext-install-fail");
        let (bundle, data) = (t.path().join("bundle"), t.path().join("data"));
        ext(&bundle, "inbox", None, false);
        let offer = bundled(&bundle).remove(0);
        // The bundled copy goes wrong after it was listed.
        std::fs::write(bundle.join("inbox").join(MANIFEST), "{").unwrap();
        assert!(install(&offer, &data).unwrap_err().contains("can't be used"));
        assert_eq!(std::fs::read_dir(&data).map(|d| d.count()).unwrap_or(0), 0);
    }
}
