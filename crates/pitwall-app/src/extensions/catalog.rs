//! The extensions Pitwall ships (docs/spec/extensions.md, "Installing"):
//! where the app's bundled copies are, which of them aren't installed or
//! have a newer copy, and Install, Update and Remove for Settings. The
//! file work is [`pitwall_core::extensions::install`]'s, off the UI
//! thread; here is what goes with it (the switch, settings, secrets and
//! the adapter).

use std::path::{Path, PathBuf};

use gpui::App;
use pitwall_core::extensions::install::{self, Offer, Refused, Status};
use pitwall_core::extensions::{Manifest, SettingKind};

use super::{dir, hub, rescan, Installed};

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
    let (Some(s), Some(data)) = (shipped_one(id, cx), dir(cx)) else {
        done(Outcome::Failed("It isn't shipped with this Pitwall.".into()), cx);
        return;
    };
    let was_on = crate::ui_state::get(cx).extension_on(id);
    if !replace_changed && !install::local(&data.join(id)).as_installed {
        done(Outcome::ChangedLocally, cx);
        return;
    }
    if was_on {
        switch(id, false, cx);
    }
    let job = cx.background_executor().spawn(async move { install::update(&s.offer, &data, replace_changed) });
    let id = id.to_string();
    cx.spawn(async move |cx| {
        let res = job.await;
        cx.update(|cx| {
            rescan(cx);
            if was_on {
                switch(&id, true, cx);
            }
            done(
                match res {
                    Ok(()) => Outcome::Done,
                    Err(Refused::ChangedLocally) => Outcome::ChangedLocally,
                    Err(Refused::Failed(e)) => Outcome::Failed(e),
                },
                cx,
            );
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
    /// last, each with its version.
    #[test]
    fn the_repo_ships_its_extensions_demos_last() {
        let list = install::bundled(Path::new(DEV_BUNDLE));
        let ids: Vec<&str> = list.iter().map(|o| o.manifest.id.as_str()).collect();
        assert_eq!(ids.last(), Some(&"demo-inbox"));
        assert!(list.iter().all(|o| o.manifest.version.is_some()), "a shipped extension says its version");
        assert!(list.iter().filter(|o| o.manifest.demo).count() == 1);
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
}
