//! Installing and updating extensions (docs/spec/extensions.md,
//! "Installing" and "Updates from a git source"): where the app's bundled
//! copies are, which of them aren't installed or have a newer copy; the
//! sources installed extensions name, checked for newer releases at start,
//! once a day and on demand; "Add from Git"; and Install, Update, Go back
//! and Remove for Settings. The file and git work is
//! [`pitwall_core::extensions::install`]'s and
//! [`pitwall_core::extensions::source`]'s, off the UI thread; here is what
//! goes with it (the switch, settings, secrets and the adapter). Nothing
//! is installed or updated without the user's click, and installing runs
//! nothing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use gpui::{App, AppContext, Entity, Global};
use pitwall_core::extensions::install::{self, Change, Local, Offer, Origin, Refused, Status};
use pitwall_core::extensions::source::{self, Tag};
use pitwall_core::extensions::{Manifest, SettingKind};

use super::{dir, hub, rescan, Installed};

/// How often the sources are checked while Pitwall runs.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

/// A source's last check, for one installed extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub at: SystemTime,
    /// The repository checked (as the manifest names it) and the folder in
    /// it.
    pub url: String,
    pub dir: String,
    /// Its newest release (`None`: it has none yet), or why the check
    /// failed, in plain words.
    pub newest: Result<Option<Tag>, String>,
    /// Why its newest release, or the installed one's tag, is refused: the
    /// tag moved.
    pub moved: Option<String>,
}

/// An install or update looked at before the user says yes: the copy
/// (fetched from its source, or Pitwall's own) and what it changes on this
/// computer; `replaces`, the installed version it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub offer: Offer,
    pub changes: Vec<Change>,
    pub replaces: Option<Option<String>>,
}

/// The checks of the sources since Pitwall started.
#[derive(Default)]
pub struct Updates {
    checks: HashMap<String, Check>,
    checking: HashSet<String>,
    /// The commit each (repository, tag) pointed at when it was first
    /// seen: one that moves since is refused.
    seen: HashMap<(String, String), String>,
    last_all: Option<Instant>,
}

impl Updates {
    /// Extension `id`'s last check.
    pub fn check(&self, id: &str) -> Option<&Check> {
        self.checks.get(id)
    }

    /// Whether `id` is being checked.
    pub fn checking(&self, id: &str) -> bool {
        self.checking.contains(id)
    }
}

struct UpdatesHandle(Entity<Updates>);

impl Global for UpdatesHandle {}

/// The checks (made on first use).
pub fn updates(cx: &mut App) -> Entity<Updates> {
    if let Some(h) = cx.try_global::<UpdatesHandle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| Updates::default());
    cx.set_global(UpdatesHandle(e.clone()));
    e
}

/// Tests: repository addresses fetched from somewhere else (a local
/// repository in place of an https one).
#[cfg(test)]
pub struct GitRedirect(pub HashMap<String, String>);

#[cfg(test)]
impl Global for GitRedirect {}

/// Where repository `url` is fetched from.
fn fetch_url(url: &str, cx: &App) -> String {
    #[cfg(test)]
    if let Some(to) = cx.try_global::<GitRedirect>().and_then(|r| r.0.get(url)) {
        return to.clone();
    }
    let _ = cx;
    url.to_string()
}

/// The usable installed extension `id`.
fn installed(id: &str, cx: &App) -> Option<Manifest> {
    super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == id).cloned()
}

/// Why extension `id` isn't checked, updated, replaced or put back: it is
/// loaded from a development folder, the author's own (`None`: it isn't).
pub fn dev_refusal(id: &str, cx: &App) -> Option<String> {
    super::dev_folder(id, cx).map(|_| format!("“{id}” is loaded from a development folder, which Pitwall never replaces: remove it from the list first."))
}

/// Check the sources of every installed extension that names one, then
/// again once a day; folders a fetch left behind (Pitwall quit midway) go.
pub fn watch_updates(cx: &mut App) {
    if let Some(data) = dir(cx) {
        cx.background_executor()
            .spawn(async move {
                for e in std::fs::read_dir(&data).into_iter().flatten().flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with(".clone-") || name.starts_with(".fetched-") {
                        let _ = std::fs::remove_dir_all(e.path());
                    }
                }
            })
            .detach();
    }
    check_all(cx);
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_secs(3600)).await;
        let due = cx.update(|cx| updates(cx).read(cx).last_all.is_none_or(|t| t.elapsed() >= CHECK_EVERY));
        if due {
            cx.update(check_all);
        }
    })
    .detach();
}

/// Check every installed extension's source now.
pub fn check_all(cx: &mut App) {
    updates(cx).update(cx, |u, _| u.last_all = Some(Instant::now()));
    let ids: Vec<String> = super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).filter(|m| m.source.is_some() && super::dev_folder(&m.id, cx).is_none()).map(|m| m.id.clone()).collect();
    for id in ids {
        check_one(&id, cx);
    }
}

/// Check installed extension `id`'s source for a newer release, off the UI
/// thread: `git ls-remote`, nothing written. Offline or without git, the
/// check says why and the card says when it last checked. A development
/// folder is never checked.
pub fn check_one(id: &str, cx: &mut App) {
    if dev_refusal(id, cx).is_some() {
        return;
    }
    let (Some(m), Some(data)) = (installed(id, cx), dir(cx)) else { return };
    let Some(src) = m.source.clone() else { return };
    let u = updates(cx);
    if !u.update(cx, |u, cx| {
        cx.notify();
        u.checking.insert(id.to_string())
    }) {
        return;
    }
    let url = fetch_url(&src.git, cx);
    let folder = data.join(id);
    let pattern = src.tags.clone();
    let job = cx.background_executor().spawn(async move {
        let origin = install::local(&folder).origin;
        (origin, source::tags(&url, &pattern))
    });
    let (id, version) = (id.to_string(), m.version.clone());
    cx.spawn(async move |cx| {
        let (origin, listed) = job.await;
        cx.update(|cx| {
            u.update(cx, |u, cx| {
                u.checking.remove(&id);
                let pre = version.as_deref().is_some_and(source::is_pre);
                let mut moved = None;
                let newest = listed.map(|tags| {
                    let mine = origin.as_ref().filter(|o| o.url().is_some_and(|x| install::same_repo(x, &src.git))).and_then(|o| o.tag_commit());
                    let newest = source::newest(&tags, pre).cloned();
                    // The installed release's tag, then the newest's,
                    // against what was seen before.
                    for t in tags.iter().filter(|t| mine.is_some_and(|(n, _)| n == t.name)).chain(newest.iter()) {
                        let seen = u.seen.get(&(src.git.clone(), t.name.clone())).map(String::as_str);
                        moved = moved.take().or_else(|| source::moved(t, mine, seen));
                    }
                    for t in &tags {
                        u.seen.entry((src.git.clone(), t.name.clone())).or_insert_with(|| t.commit.clone());
                    }
                    newest
                });
                u.checks.insert(id.clone(), Check { at: SystemTime::now(), url: src.git.clone(), dir: src.dir.clone(), newest, moved });
                cx.notify();
            });
        });
    })
    .detach();
}

/// The update to offer for installed extension `m` (installed from
/// `origin`): the newest of Pitwall's copy and its source's newest
/// release, above the installed version (never a downgrade). A release
/// from a source is a stand-in until [`prepare`] fetches it. None for a
/// development folder.
pub fn update_offer(m: &Manifest, origin: Option<&Origin>, shipped: Option<&Shipped>, cx: &App) -> Option<Offer> {
    if dev_refusal(&m.id, cx).is_some() {
        return None;
    }
    let local = Local { as_installed: true, version: m.version.clone(), origin: origin.cloned() };
    let mut offers: Vec<Offer> = shipped.map(|s| s.offer.clone()).into_iter().collect();
    let check = cx.try_global::<UpdatesHandle>().and_then(|h| h.0.read(cx).check(&m.id).cloned());
    if let Some(Check { newest: Ok(Some(tag)), moved: None, url, dir, .. }) = check {
        let stand_in = Manifest { version: Some(tag.version.clone()), ..Manifest::named(&m.id, &m.name) };
        offers.push(Offer { manifest: stand_in, folder: PathBuf::new(), origin: Origin::Git { url, dir, tag: tag.name, commit: tag.commit } });
    }
    install::best(&local, &offers).cloned()
}

/// Look at `offer` for installed extension `id` before updating: a release
/// from a source is fetched (off the UI thread, into a hidden folder) and
/// checked; `done` gets what it would change on this computer, or why it
/// can't be had.
pub fn prepare(id: &str, offer: Offer, cx: &mut App, done: impl FnOnce(Result<Prepared, String>, &mut App) + 'static) {
    if let Some(why) = dev_refusal(id, cx) {
        cx.defer(move |cx| done(Err(why), cx));
        return;
    }
    let (Some(m), Some(data)) = (installed(id, cx), dir(cx)) else {
        let why: String = "It isn't installed.".into();
        cx.defer(move |cx| done(Err(why), cx));
        return;
    };
    let folder = data.join(id);
    let job = match offer.origin.clone() {
        Origin::Bundled => cx.background_executor().spawn(async move { Ok(offer) }),
        Origin::Git { url, dir: sub, tag, commit } => {
            let tag = Tag { name: tag, version: offer.manifest.version.clone().unwrap_or_default(), commit };
            let (from, id) = (fetch_url(&url, cx), id.to_string());
            cx.background_executor().spawn(async move {
                let f = source::fetch(&from, &tag, &sub, Some(&id), &data)?;
                Ok(Offer { manifest: f.manifest, folder: f.folder, origin: Origin::Git { url, dir: sub, tag: tag.name, commit: tag.commit } })
            })
        }
    };
    let origin = cx.background_executor().spawn(async move { install::local(&folder).origin });
    cx.spawn(async move |cx| {
        let (res, origin) = (job.await, origin.await);
        cx.update(|cx| {
            done(res.map(|offer| Prepared { changes: install::changes(Some((&m, origin.as_ref())), &offer), replaces: Some(m.version.clone()), offer }), cx);
        });
    })
    .detach();
}

/// The tag patterns tried for a repository added by hand, folder `dir`
/// (its last part `name`): `name-v1.0.0`, `name-1.0.0`, `v1.0.0`, `1.0.0`.
pub fn guessed_patterns(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(name) = dir.rsplit('/').next().filter(|n| !n.is_empty()) {
        out.push(format!("{name}-v{{version}}"));
        out.push(format!("{name}-{{version}}"));
    }
    out.extend(["v{version}".to_string(), "{version}".to_string()]);
    out
}

/// "Add from Git…": the newest release of the repository at `url` (an
/// https address; `sub`, the folder in it holding `extension.json`),
/// fetched and checked off the UI thread; `done` gets what installing it
/// would change on this computer (replacing the installed one of the same
/// id, when there is one), or why it can't be had. Nothing is installed.
pub fn prepare_from_git(url: &str, sub: &str, cx: &mut App, done: impl FnOnce(Result<Prepared, String>, &mut App) + 'static) {
    let (url, sub) = (url.trim().trim_end_matches('/').to_string(), sub.trim().trim_matches('/').to_string());
    if let Err(e) = pitwall_core::extensions::check_git_url(&url) {
        let why = format!("That address can't be used: {e}.");
        cx.defer(move |cx| done(Err(why), cx));
        return;
    }
    if let Err(e) = pitwall_core::extensions::check_source_dir(&sub) {
        let why = format!("That folder can't be used: {e}.");
        cx.defer(move |cx| done(Err(why), cx));
        return;
    }
    let Some(data) = dir(cx) else {
        let why: String = "Extensions need Pitwall's data folder.".into();
        cx.defer(move |cx| done(Err(why), cx));
        return;
    };
    let from = fetch_url(&url, cx);
    let job = cx.background_executor().spawn({
        let (url, sub, data) = (url.clone(), sub.clone(), data.clone());
        async move {
            let mut best: Option<Tag> = None;
            let listed = source::list(&from)?;
            for p in guessed_patterns(&sub) {
                let tags = source::parse_tags(&listed, &p);
                if let Some(t) = source::newest(&tags, false).or(tags.first()) {
                    if best.as_ref().is_none_or(|b| install::compare_versions(&t.version, &b.version) == std::cmp::Ordering::Greater) {
                        best = Some(t.clone());
                    }
                }
            }
            let Some(tag) = best else {
                return Err(format!("{} has no release tags (like v1.0.0{}).", install::repo_name(&url), sub.rsplit('/').next().filter(|n| !n.is_empty()).map(|n| format!(" or {n}-v1.0.0")).unwrap_or_default()));
            };
            let f = source::fetch(&from, &tag, &sub, None, &data)?;
            let id = f.manifest.id.clone();
            let origin = install::local(&data.join(&id)).origin;
            Ok((Offer { manifest: f.manifest, folder: f.folder, origin: Origin::Git { url, dir: sub, tag: tag.name, commit: tag.commit } }, origin))
        }
    });
    cx.spawn(async move |cx| {
        let res = job.await;
        cx.update(|cx| {
            // A development folder with its id is never replaced.
            let res = res.and_then(|(offer, origin)| match dev_refusal(&offer.manifest.id, cx) {
                Some(why) => {
                    let _ = std::fs::remove_dir_all(&offer.folder);
                    Err(why)
                }
                None => Ok((offer, origin)),
            });
            done(
                res.map(|(offer, origin)| {
                    let now = installed(&offer.manifest.id, cx);
                    Prepared { changes: install::changes(now.as_ref().map(|m| (m, origin.as_ref())), &offer), replaces: now.map(|m| m.version), offer }
                }),
                cx,
            );
        });
    })
    .detach();
}

/// A prepared copy the user didn't take: a fetched folder is deleted.
pub fn discard(p: &Prepared, cx: &mut App) {
    if matches!(p.offer.origin, Origin::Git { .. }) {
        let folder = p.offer.folder.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = std::fs::remove_dir_all(folder);
            })
            .detach();
    }
}

/// Install or update from prepared `p` (the user clicked): installed, or
/// put in place of the installed one as [`update_one`] does. A fetched
/// folder is deleted afterwards (kept while the user is asked whether to
/// replace a folder changed by hand).
pub fn apply(p: &Prepared, replace_changed: bool, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    let offer = p.offer.clone();
    let fetched = matches!(offer.origin, Origin::Git { .. }).then(|| offer.folder.clone());
    let cleanup = move |o: Outcome, cx: &mut App| {
        if o != Outcome::ChangedLocally {
            if let Some(f) = fetched {
                let _ = std::fs::remove_dir_all(f);
            }
        }
        done(o, cx);
    };
    if let Some(why) = dev_refusal(&offer.manifest.id, cx) {
        cleanup(Outcome::Failed(why), cx);
        return;
    }
    if p.replaces.is_some() {
        update_with(offer, replace_changed, cx, cleanup);
    } else {
        let Some(data) = dir(cx) else {
            cleanup(Outcome::Failed("Extensions need Pitwall's data folder.".into()), cx);
            return;
        };
        let id = offer.manifest.id.clone();
        let job = cx.background_executor().spawn(async move { install::install(&offer, &data) });
        cx.spawn(async move |cx| {
            let res = job.await;
            cx.update(|cx| {
                rescan(cx);
                check_one(&id, cx);
                cleanup(res.map_or_else(Outcome::Failed, |()| Outcome::Done), cx);
            });
        })
        .detach();
    }
}

/// "Go back": extension `id`'s copy from before its last update put back
/// (settings and secrets stay); one that was on is stopped for it, then
/// started again. A folder changed by hand is replaced only when
/// `replace_changed`.
pub fn rollback_one(id: &str, replace_changed: bool, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    if let Some(why) = dev_refusal(id, cx) {
        done(Outcome::Failed(why), cx);
        return;
    }
    let Some(data) = dir(cx) else {
        done(Outcome::Failed("Extensions need Pitwall's data folder.".into()), cx);
        return;
    };
    let was_on = crate::ui_state::get(cx).extension_on(id);
    if was_on {
        switch(id, false, cx);
    }
    let id = id.to_string();
    let job = cx.background_executor().spawn({
        let id = id.clone();
        async move { install::rollback(&data, &id, replace_changed) }
    });
    cx.spawn(async move |cx| {
        let res = job.await;
        cx.update(|cx| {
            rescan(cx);
            if was_on {
                switch(&id, true, cx);
            }
            done(outcome(res), cx);
        });
    })
    .detach();
}

fn outcome(res: Result<(), Refused>) -> Outcome {
    match res {
        Ok(()) => Outcome::Done,
        Err(Refused::ChangedLocally) => Outcome::ChangedLocally,
        Err(Refused::Failed(e)) => Outcome::Failed(e),
    }
}

/// The repo's `extensions/` (workspace builds and tests).
const DEV_BUNDLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../extensions");

/// The folder of bundled extensions, like the other things the app ships
/// (`engineer::find_skills`): `$PITWALL_BUNDLED_EXTENSIONS`; in a macOS
/// bundle `Contents/Resources/extensions`; `../lib/<package>/extensions`
/// (Linux packages); `extensions/` next to the executable (Windows); else
/// the repo's (`dev`). One counts when it holds an extension.
pub fn find_bundle(env: Option<PathBuf>, exe_dir: Option<PathBuf>, dev: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = env.into_iter().collect();
    if let Some(dir) = exe_dir {
        if let Some(up) = dir.parent() {
            candidates.push(up.join("Resources").join("extensions"));
            for pkg in ["pitwall", "pitwall-preview"] {
                candidates.push(up.join("lib").join(pkg).join("extensions"));
            }
        }
        candidates.push(dir.join("extensions"));
    }
    candidates.push(PathBuf::from(dev));
    candidates.into_iter().find(|d| holds_extension(d))
}

fn holds_extension(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| e.path().join(pitwall_core::extensions::MANIFEST).is_file()))
}

/// This app's bundle folder.
pub fn locate() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    find_bundle(std::env::var_os("PITWALL_BUNDLED_EXTENSIONS").map(PathBuf::from), exe_dir, DEV_BUNDLE)
}

/// A bundled extension and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shipped {
    pub offer: Offer,
    pub status: Status,
}

/// The bundled extensions, by name, demos last, each with its status (none
/// without a bundle or a data folder).
pub fn shipped(cx: &App) -> Vec<Shipped> {
    let (Some(bundle), Some(data)) = (bundle(cx), dir(cx)) else { return Vec::new() };
    install::bundled(&bundle).into_iter().map(|offer| Shipped { status: install::status(&offer, &data), offer }).collect()
}

/// The bundled copy of extension `id`, when the app ships it.
pub fn shipped_one(id: &str, cx: &App) -> Option<Shipped> {
    shipped(cx).into_iter().find(|s| s.offer.manifest.id == id)
}

fn bundle(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Installed>().and_then(|i| i.bundle.clone())
}

/// Tests: this folder is the bundle.
#[cfg(test)]
pub fn set_bundle(bundle: Option<PathBuf>, cx: &mut App) {
    let mut i = cx.try_global::<Installed>().cloned().unwrap_or_default();
    i.bundle = bundle;
    cx.set_global(i);
}

/// What `done` hears: nothing, the reason it failed, or (Update) that the
/// folder was changed by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    ChangedLocally,
    Failed(String),
}

/// Install bundled extension `id`: copied in, listed, still off.
pub fn install_one(id: &str, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    let (Some(s), Some(data)) = (shipped_one(id, cx), dir(cx)) else {
        done(Outcome::Failed("It isn't shipped with this Pitwall.".into()), cx);
        return;
    };
    let job = cx.background_executor().spawn(async move { install::install(&s.offer, &data) });
    cx.spawn(async move |cx| {
        let res = job.await;
        cx.update(|cx| {
            rescan(cx);
            done(res.map_or_else(Outcome::Failed, |()| Outcome::Done), cx);
        });
    })
    .detach();
}

/// Replace extension `id` with its bundled copy: its settings and secrets
/// stay; one that was on is stopped for the copy, then started again. A
/// folder changed by hand is replaced only when `replace_changed`.
pub fn update_one(id: &str, replace_changed: bool, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    let Some(s) = shipped_one(id, cx) else {
        done(Outcome::Failed("It isn't shipped with this Pitwall.".into()), cx);
        return;
    };
    update_with(s.offer, replace_changed, cx, done);
}

/// Replace installed extension `offer.manifest.id` with `offer` (the
/// installed copy kept for Go back): its settings and secrets stay; one
/// that was on is stopped for the copy, then started again. A folder
/// changed by hand is replaced only when `replace_changed`.
fn update_with(offer: Offer, replace_changed: bool, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    let Some(data) = dir(cx) else {
        done(Outcome::Failed("Extensions need Pitwall's data folder.".into()), cx);
        return;
    };
    let id = offer.manifest.id.clone();
    if let Some(why) = dev_refusal(&id, cx) {
        done(Outcome::Failed(why), cx);
        return;
    }
    let was_on = crate::ui_state::get(cx).extension_on(&id);
    if !replace_changed && !install::local(&data.join(&id)).as_installed {
        done(Outcome::ChangedLocally, cx);
        return;
    }
    if was_on {
        switch(&id, false, cx);
    }
    let job = cx.background_executor().spawn(async move { install::update(&offer, &data, replace_changed) });
    cx.spawn(async move |cx| {
        let res = job.await;
        cx.update(|cx| {
            rescan(cx);
            if was_on {
                switch(&id, true, cx);
            }
            done(outcome(res), cx);
        });
    })
    .detach();
}

/// Remove extension `id`: its adapter stopped, it turned off, its secrets
/// deleted from the secure store, its settings forgotten and its folder
/// deleted.
pub fn remove_one(m: &Manifest, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    let Some(data) = dir(cx) else {
        done(Outcome::Failed("Extensions need Pitwall's data folder.".into()), cx);
        return;
    };
    let id = m.id.clone();
    let prefs = crate::ui_state::get(cx).extension(&id);
    let mut keys: Vec<String> = m.settings.iter().filter(|s| s.kind == SettingKind::Secret).map(|s| s.key.clone()).collect();
    keys.extend(prefs.secrets_set.iter().cloned());
    keys.sort();
    keys.dedup();
    let mut accounts: Vec<String> = keys.iter().map(|k| super::account(&id, k, cx)).collect();
    // Its connections go too, with their secrets.
    accounts.extend(super::connections::forget_all(&id, cx));
    accounts.sort();
    accounts.dedup();
    // Off and forgotten first: the adapter stops before its folder goes.
    crate::ui_state::update(cx, |ui| *ui = std::mem::take(ui).forget_extension(&id));
    let h = hub::hub(cx);
    h.update(cx, |h, cx| h.reconcile(cx));
    let store = super::secret_store(cx);
    let job = cx.background_executor().spawn(async move {
        let secrets: Vec<String> = accounts.iter().filter_map(|a| store.delete(a).err()).collect();
        let folder = install::remove(&data, &id);
        (secrets, folder)
    });
    cx.spawn(async move |cx| {
        let (secrets, folder) = job.await;
        cx.update(|cx| {
            rescan(cx);
            let outcome = match (folder, secrets.first()) {
                (Err(e), _) => Outcome::Failed(e),
                (Ok(()), Some(e)) => Outcome::Failed(format!("Removed, but a secret couldn't be deleted from the secure store: {e}")),
                (Ok(()), None) => Outcome::Done,
            };
            done(outcome, cx);
        });
    })
    .detach();
}

fn switch(id: &str, on: bool, cx: &mut App) {
    crate::ui_state::update(cx, |ui| *ui = std::mem::take(ui).set_extension(id, on));
    let h = hub::hub(cx);
    h.update(cx, |h, cx| h.reconcile(cx));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn the_bundle_is_found_like_the_other_shipped_files() {
        let t = pitwall_core::testing::TempDir::new("ext-bundle-find");
        let contents = t.path().join("Pitwall.app/Contents");
        let res = contents.join("Resources/extensions");
        std::fs::create_dir_all(res.join("inbox")).unwrap();
        let exe = Some(contents.join("MacOS"));
        assert_eq!(find_bundle(None, exe.clone(), "/nowhere"), None, "a folder without an extension doesn't count");
        std::fs::write(res.join("inbox/extension.json"), "{}").unwrap();
        assert_eq!(find_bundle(None, exe, "/nowhere"), Some(res));
        assert!(find_bundle(None, None, DEV_BUNDLE).is_some(), "a workspace build: the repo's extensions");
        assert_eq!(find_bundle(None, None, "/nowhere"), None);
    }

    /// What the packaging script bundles (`scripts/stage-extensions.sh`):
    /// every extension folder of the repo, usable as shipped, without its
    /// test files or node_modules.
    #[cfg(unix)]
    #[test]
    fn bundling_lists_the_right_folders() {
        let t = pitwall_core::testing::TempDir::new("ext-stage");
        let dest = t.path().join("extensions");
        let script = Path::new(DEV_BUNDLE).join("../scripts/stage-extensions.sh");
        let out = std::process::Command::new("bash").arg(&script).arg(DEV_BUNDLE).arg(&dest).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let mut ids: Vec<String> = std::fs::read_dir(&dest).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        ids.sort();
        let mut want: Vec<String> = std::fs::read_dir(DEV_BUNDLE).unwrap().flatten().filter(|e| e.path().join(pitwall_core::extensions::MANIFEST).is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        want.sort();
        assert_eq!(ids, want, "every extension folder of the repo");
        let mut stack = vec![dest.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                assert!(!["test", "node_modules", ".DS_Store"].contains(&name.as_str()), "{} is bundled", e.path().display());
                if e.path().is_dir() {
                    stack.push(e.path());
                }
            }
        }
        // Each as shipped still has the files its manifest names.
        for id in &ids {
            let r = pitwall_core::extensions::check::check(&dest.join(id), None);
            assert!(r.passed(), "{}", r.text());
        }
        let offers = install::bundled(&dest);
        assert_eq!(offers.len(), ids.len(), "each usable as bundled");
        // And each installs from the bundle.
        let data = t.path().join("data");
        for o in &offers {
            install::install(o, &data).unwrap();
        }
    }

    /// The app ships the repo's extensions meant for users, the demo
    /// last, each with its version; one with a source names an https
    /// repository whose folder is its own folder in this repository.
    #[test]
    fn the_repo_ships_its_extensions_demos_last() {
        let list = install::bundled(Path::new(DEV_BUNDLE));
        let ids: Vec<&str> = list.iter().map(|o| o.manifest.id.as_str()).collect();
        assert_eq!(ids.last(), Some(&"demo-inbox"));
        assert!(list.iter().all(|o| o.manifest.version.is_some()), "a shipped extension says its version");
        assert!(list.iter().filter(|o| o.manifest.demo).count() == 1);
        for o in &list {
            if let Some(s) = &o.manifest.source {
                assert!(s.dir.ends_with(&o.manifest.id), "{}: its source folder is its own", o.manifest.id);
                let v = o.manifest.version.as_deref().unwrap();
                assert_eq!(source::version_of(&s.tags.replace("{version}", v), &s.tags).as_deref(), Some(v), "{}: its tags give its version", o.manifest.id);
            }
        }
    }

    /// Install, Update (on: stopped, then started again; settings and
    /// secrets kept), a hand-edited folder asked about, and Remove
    /// (switch, settings, secrets and folder gone), on a temporary data
    /// folder with the in-memory secret store.
    #[gpui::test]
    fn install_update_and_remove_from_settings(cx: &mut TestAppContext) {
        let t = pitwall_core::testing::TempDir::new("ext-catalog");
        let (bundle, root) = (t.path().join("bundle"), t.path().join("data"));
        let write = |version: &str, main: &str| {
            let d = bundle.join("inbox");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("extension.json"), format!(r#"{{"id": "inbox", "name": "Inbox", "version": "{version}", "settings": [{{"key": "apiKey", "type": "secret", "label": "API key"}}, {{"key": "server", "type": "url", "label": "Server"}}]}}"#)).unwrap();
            std::fs::write(d.join("main.mjs"), main).unwrap();
        };
        write("1.0.0", "// 1");
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let note = |seen: &std::rc::Rc<std::cell::RefCell<Vec<Outcome>>>| {
            let seen = seen.clone();
            move |o: Outcome, _: &mut App| seen.borrow_mut().push(o)
        };
        cx.update(|cx| {
            crate::ui_state::store(cx);
            super::super::init(Some(root.clone()), cx);
            set_bundle(Some(bundle.clone()), cx);
            assert_eq!(shipped(cx)[0].status, Status::Available);
            install_one("inbox", cx, note(&seen));
        });
        cx.run_until_parked();
        let data = root.join("extensions");
        cx.update(|cx| {
            assert_eq!(seen.borrow().last(), Some(&Outcome::Done));
            assert!(data.join("inbox/main.mjs").is_file());
            assert_eq!(super::super::found(cx).len(), 1, "listed under Installed");
            assert!(!crate::ui_state::get(cx).extension_on("inbox"), "installing turns nothing on");
            assert_eq!(shipped(cx)[0].status, Status::Current);
            // On, with a setting and a secret.
            crate::ui_state::update(cx, |ui| *ui = std::mem::take(ui).set_extension("inbox", true).with_extension("inbox", |e| {
                e.values.insert("server".into(), "https://s.example".into());
                e.secrets_set.push("apiKey".into());
            }));
            let acct = super::super::account("inbox", "apiKey", cx);
            super::super::secret_store(cx).set(&acct, "s3cret").unwrap();
        });
        write("1.1.0", "// 2");
        cx.update(|cx| {
            assert_eq!(shipped(cx)[0].status, Status::Newer("1.1.0".into()));
            update_one("inbox", false, cx, note(&seen));
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(seen.borrow().last(), Some(&Outcome::Done));
            assert_eq!(std::fs::read_to_string(data.join("inbox/main.mjs")).unwrap(), "// 2");
            let prefs = crate::ui_state::get(cx).extension("inbox");
            assert!(prefs.enabled && prefs.values.contains_key("server") && prefs.secrets_set == ["apiKey"], "on again, settings kept");
        });
        // Changed by hand: asked, not replaced.
        std::fs::write(data.join("inbox/main.mjs"), "// mine").unwrap();
        write("1.2.0", "// 3");
        cx.update(|cx| update_one("inbox", false, cx, note(&seen)));
        cx.run_until_parked();
        assert_eq!(seen.borrow().last(), Some(&Outcome::ChangedLocally));
        assert_eq!(std::fs::read_to_string(data.join("inbox/main.mjs")).unwrap(), "// mine");
        cx.update(|cx| {
            let m = super::super::found(cx)[0].manifest.clone().unwrap();
            remove_one(&m, cx, note(&seen));
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(seen.borrow().last(), Some(&Outcome::Done));
            assert!(!data.join("inbox").exists());
            assert!(super::super::found(cx).is_empty());
            assert_eq!(crate::ui_state::get(cx).extension("inbox"), Default::default(), "off, settings forgotten");
            let acct = super::super::account("inbox", "apiKey", cx);
            assert_eq!(super::super::secret_store(cx).get(&acct).unwrap(), None, "the secret is deleted");
            assert_eq!(shipped(cx)[0].status, Status::Available);
        });
    }

    /// A setup action of the releases in `git_source_tests`, pinned at `pin`.
    fn setup_action(pin: &str) -> String {
        format!(r#"[{{"id": "setup", "setup": true, "label": "Set up", "description": "d", "steps": [{{"run": ["tool", "add", "x#{pin}"]}}]}}]"#)
    }

    /// Updates from a git source, on a local repository standing in for
    /// the manifest's https one: Add from Git installs the newest release
    /// (off, its source and commit recorded); a newer tag is found by a
    /// check, fetched and looked at (its changed setup step listed), and
    /// installed only by `apply`, which keeps the settings, starts it again
    /// and unticks the changed setup step; Go back returns the copy before.
    #[gpui::test]
    fn add_check_update_and_go_back_from_a_git_source(cx: &mut TestAppContext) {
        use pitwall_core::testing::{extension_release, git_in};
        let t = pitwall_core::testing::TempDir::new("ext-git-flow");
        let (repo, root) = (t.path().join("repo"), t.path().join("data"));
        let c1 = extension_release(&repo, "inbox", "1.0.0", "// 1", &setup_action("v1"), false);
        let data = root.join("extensions");
        let got: std::rc::Rc<std::cell::RefCell<Vec<Result<Prepared, String>>>> = Default::default();
        let keep = |got: &std::rc::Rc<std::cell::RefCell<Vec<Result<Prepared, String>>>>| {
            let got = got.clone();
            move |r: Result<Prepared, String>, _: &mut App| got.borrow_mut().push(r)
        };
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let note = |seen: &std::rc::Rc<std::cell::RefCell<Vec<Outcome>>>| {
            let seen = seen.clone();
            move |o: Outcome, _: &mut App| seen.borrow_mut().push(o)
        };
        cx.update(|cx| {
            crate::ui_state::store(cx);
            super::super::init(Some(root.clone()), cx);
            set_bundle(None, cx);
            cx.set_global(GitRedirect(HashMap::from([("https://example.com/r".to_string(), repo.to_string_lossy().into_owned())])));
            // Not https: refused before git runs.
            prepare_from_git("http://example.com/r", "ext", cx, keep(&got));
            prepare_from_git("https://example.com/r", "ext", cx, keep(&got));
        });
        cx.run_until_parked();
        let p = {
            let got = got.borrow();
            assert!(got[0].as_ref().unwrap_err().contains("https://"));
            got[1].clone().unwrap()
        };
        assert_eq!(p.offer.origin, Origin::Git { url: "https://example.com/r".into(), dir: "ext".into(), tag: "ext-v1.0.0".into(), commit: c1.clone() });
        assert_eq!(p.replaces, None);
        assert!(p.changes.iter().any(|c| c.lines == ["tool add x#v1"]), "its setup commands are shown before installing");
        assert!(!data.join("inbox").exists(), "looking installs nothing");
        cx.update(|cx| apply(&p, false, cx, note(&seen)));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(seen.borrow().last(), Some(&Outcome::Done));
            assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 1");
            assert!(!p.offer.folder.exists(), "the fetched folder is gone");
            assert!(!crate::ui_state::get(cx).extension_on("inbox"), "installing turns nothing on");
            assert_eq!(install::origin(&data.join("inbox")), Some(p.offer.origin.clone()));
            let check = updates(cx).read(cx).check("inbox").cloned().expect("checked once installed");
            assert_eq!(check.newest.unwrap().map(|t| t.version), Some("1.0.0".into()));
            let m = installed("inbox", cx).unwrap();
            assert_eq!(update_offer(&m, Some(&p.offer.origin), None, cx), None, "nothing newer");
            // On, with a setting, and its setup run recorded.
            crate::ui_state::update(cx, |ui| {
                *ui = std::mem::take(ui).set_extension("inbox", true).with_extension("inbox", |e| {
                    e.values.insert("server".into(), "https://s.example".into());
                })
            });
            super::super::connections::record_action(&m, "setup", None, true, cx);
            assert_eq!(super::super::connections::prefs_for(&m, None, cx).actions_done, ["setup"]);
        });

        // A new release moves the setup pin.
        let c2 = extension_release(&repo, "inbox", "1.1.0", "// 2", &setup_action("v2"), true);
        cx.update(|cx| check_one("inbox", cx));
        cx.run_until_parked();
        let offer = cx.update(|cx| {
            let m = installed("inbox", cx).unwrap();
            update_offer(&m, install::origin(&data.join("inbox")).as_ref(), None, cx).expect("1.1.0 offered")
        });
        assert_eq!(offer.manifest.version.as_deref(), Some("1.1.0"));
        cx.update(|cx| prepare("inbox", offer, cx, keep(&got)));
        cx.run_until_parked();
        let p2 = got.borrow().last().cloned().unwrap().unwrap();
        assert_eq!(p2.replaces, Some(Some("1.0.0".into())));
        assert!(p2.changes.iter().any(|c| c.title.contains("runs other commands") && c.lines == ["tool add x#v2"]), "{:?}", p2.changes);
        assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 1", "looking installs nothing");
        cx.update(|cx| apply(&p2, false, cx, note(&seen)));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(seen.borrow().last(), Some(&Outcome::Done));
            assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 2");
            assert_eq!(install::origin(&data.join("inbox")).and_then(|o| o.tag_commit().map(|(_, c)| c.to_string())), Some(c2.clone()));
            let prefs = crate::ui_state::get(cx).extension("inbox");
            assert!(prefs.enabled && prefs.values.contains_key("server"), "on again, settings kept");
            let m = installed("inbox", cx).unwrap();
            assert!(super::super::connections::prefs_for(&m, None, cx).actions_done.is_empty(), "the changed setup step asks to be run again");
            assert_eq!(install::previous(&data, "inbox").and_then(|p| p.version).as_deref(), Some("1.0.0"));
            rollback_one("inbox", false, cx, note(&seen));
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(seen.borrow().last(), Some(&Outcome::Done));
            assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 1");
            assert!(crate::ui_state::get(cx).extension_on("inbox"), "on again after going back");
            let m = installed("inbox", cx).unwrap();
            assert_eq!(super::super::connections::prefs_for(&m, None, cx).actions_done, ["setup"], "the steps that ran are the ones it has again");
            assert!(update_offer(&m, install::origin(&data.join("inbox")).as_ref(), None, cx).is_some(), "1.1.0 is offered again");
        });

        // The 1.1.0 tag moved to another commit: refused, not offered.
        std::fs::write(repo.join("ext/adapter/main.mjs"), "// sneaky").unwrap();
        git_in(&repo, &["commit", "--quiet", "-am", "sneaky"]);
        git_in(&repo, &["tag", "-f", "-a", "ext-v1.1.0", "-m", "moved"]);
        cx.update(|cx| check_one("inbox", cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let check = updates(cx).read(cx).check("inbox").cloned().unwrap();
            assert!(check.moved.as_deref().is_some_and(|m| m.contains("The ext-v1.1.0 tag moved")), "{check:?}");
            let m = installed("inbox", cx).unwrap();
            assert_eq!(update_offer(&m, install::origin(&data.join("inbox")).as_ref(), None, cx), None);
        });

        // Offline (or gone): the check says so, nothing else changes.
        cx.update(|cx| {
            cx.set_global(GitRedirect(HashMap::from([("https://example.com/r".to_string(), t.path().join("gone").to_string_lossy().into_owned())])));
            check_one("inbox", cx);
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let check = updates(cx).read(cx).check("inbox").cloned().unwrap();
            assert!(check.newest.is_err());
            assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 1");
        });

        // Loaded from a development folder (the repository's own, with
        // its source): never checked, offered, replaced or put back.
        let checked_at = cx.update(|cx| {
            cx.set_global(GitRedirect(HashMap::from([("https://example.com/r".to_string(), repo.to_string_lossy().into_owned())])));
            assert_eq!(super::super::load_dev(&repo.join("ext"), cx).unwrap(), "inbox");
            let at = updates(cx).read(cx).check("inbox").map(|c| c.at);
            check_all(cx);
            prepare_from_git("https://example.com/r", "ext", cx, keep(&got));
            rollback_one("inbox", true, cx, note(&seen));
            at
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(updates(cx).read(cx).check("inbox").map(|c| c.at), checked_at, "not checked");
            let m = installed("inbox", cx).unwrap();
            assert_eq!(update_offer(&m, None, None, cx), None);
            assert!(got.borrow().last().unwrap().as_ref().unwrap_err().contains("development folder"));
            assert!(matches!(seen.borrow().last(), Some(Outcome::Failed(why)) if why.contains("development folder")));
            let left: Vec<String> = std::fs::read_dir(&data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(".clone-") || n.starts_with(".fetched-")).collect();
            assert!(left.is_empty(), "{left:?}");
            assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 1", "the installed copy is untouched");
            // Off the list: the installed copy is checked again.
            super::super::forget_dev("inbox", cx);
            check_one("inbox", cx);
            assert!(updates(cx).read(cx).checking("inbox"));
        });
        cx.run_until_parked();
    }

    #[test]
    fn hand_added_repositories_try_the_usual_tags() {
        assert_eq!(guessed_patterns("extensions/ci-status"), ["ci-status-v{version}", "ci-status-{version}", "v{version}", "{version}"]);
        assert_eq!(guessed_patterns(""), ["v{version}", "{version}"]);
    }
}
