//! Browser profiles (docs/spec/web.md, "Browser profiles"): separate
//! sign-ins for Pitwall's web pages, one store of cookies and site data
//! each. A profile is an account of kind [`WEB`] (`accounts.json`, beside
//! the tools' accounts): a name and its folder. Default is the store every
//! page used before profiles (`<data>/webview/profile-default`).
//!
//! Which profile a page opens in:
//! - an agent's (`show_web`, `pitwall open --web`): its project's (the
//!   "Browser" row of the project's Accounts…), else Default; an agent
//!   can't pick one;
//! - one the user opens (⌘L, "Open web page…"): the project of the agent
//!   whose tile has the focus, else Default;
//! - a page keeps its profile (`ui.json` `webTiles[id].profile`) until the
//!   user switches it from its bar; one whose profile is gone or can't be
//!   used here opens in Default and says so once.
//!
//! macOS before 14 has one store for every view: only Default there.
//! Previews keep their project's own store and have no profile.

use std::path::PathBuf;

use gpui::{App, Global};
use pitwall_core::accounts::{Account, Accounts, DEFAULT, WEB};

/// "Default".
pub const DEFAULT_NAME: &str = "Default";

/// Why a profile other than Default can't be used on this system.
pub const NEEDS_STORES: &str = "Browser profiles need macOS 14 or later: before it, every page shares one store, so pages here use Default.";

/// Profile `id` (`None`: Default) can be used where each data folder has
/// a store of its own (`separate`); Default always.
pub fn usable(id: Option<&str>, separate: bool) -> bool {
    id.is_none_or(|i| i == DEFAULT) || separate
}

/// The profile a page asking for `wanted` gets: that one when it is a
/// profile in `list` that can be used here, else Default (`None`) and
/// `true`: it fell back.
pub fn resolve(list: &Accounts, wanted: Option<&str>, separate: bool) -> (Option<String>, bool) {
    match wanted.filter(|w| *w != DEFAULT) {
        None => (None, false),
        Some(id) => match list.get(id).filter(|a| a.kind == WEB) {
            Some(a) if usable(Some(id), separate) => (Some(a.id.clone()), false),
            _ => (None, true),
        },
    }
}

/// The profile `project`'s pages open in (`None`: Default, also where its
/// profile can't be used).
pub fn of_project(list: &Accounts, project: &str, separate: bool) -> Option<String> {
    list.project_web(project).filter(|_| separate).map(|a| a.id.clone())
}

/// Tests: whether this system has a store per folder (as macOS 14+).
#[derive(Clone, Copy)]
pub struct Separate(pub bool);

impl Global for Separate {}

/// This system keeps a store per profile ([`crate::webview::separate_stores`]).
pub fn separate(cx: &App) -> bool {
    match cx.try_global::<Separate>() {
        Some(s) => s.0,
        None => crate::webview::separate_stores(),
    }
}

/// The accounts as now.
fn list(cx: &mut App) -> Accounts {
    crate::accounts::model(cx).read(cx).list.clone()
}

/// The profiles besides Default, in their order.
pub fn all(cx: &mut App) -> Vec<Account> {
    list(cx).of_kind(WEB).cloned().collect()
}

/// What profile `id` is called ("Default" for `None`, or one gone).
pub fn name(id: Option<&str>, cx: &mut App) -> String {
    id.and_then(|i| list(cx).get(i).filter(|a| a.kind == WEB).map(|a| a.name.clone())).unwrap_or_else(|| DEFAULT_NAME.into())
}

/// The profile `project`'s pages open in now.
pub fn for_project(project: &str, cx: &mut App) -> Option<String> {
    let sep = separate(cx);
    of_project(&list(cx), project, sep)
}

/// The profile a page kept as `wanted` opens in, and whether it fell back
/// to Default.
pub fn resolve_now(wanted: Option<&str>, cx: &mut App) -> (Option<String>, bool) {
    let sep = separate(cx);
    resolve(&list(cx), wanted, sep)
}

/// Profile `id`'s folder (`None`: Default's).
pub fn dir(id: Option<&str>, cx: &mut App) -> PathBuf {
    crate::preview::profile_dir(&super::page::data_root(cx), id.unwrap_or(DEFAULT))
}

/// The pages kept in profile `id` (tile ids; in layouts or windows).
pub fn pages_in(id: &str, cx: &App) -> Vec<String> {
    crate::ui_state::get(cx).web_tiles.iter().filter(|(_, t)| t.agent.is_none() && t.profile.as_deref() == Some(id)).map(|(k, _)| k.clone()).collect()
}

/// Why profile `id` can't be removed now: pages are open in it (they'd
/// lose their store under them). Switch or close them first.
pub fn in_use(id: &str, name: &str, cx: &App) -> Option<String> {
    match pages_in(id, cx).len() {
        0 => None,
        1 => Some(format!("A page is open in {name}: switch it to another profile from its bar, or close it, first.")),
        n => Some(format!("{n} pages are open in {name}: switch them to another profile from their bars, or close them, first.")),
    }
}

/// Add profile `name`.
pub fn add(name: &str, cx: &mut App) -> Result<Account, String> {
    let m = crate::accounts::model(cx);
    m.update(cx, |m, cx| m.add_web(name, cx))
}

/// Remove profile `id` and delete its sign-ins, cookies and site data from
/// this computer. Refused while pages are open in it ([`in_use`]). The
/// projects that named it go back to Default. Its data is deleted after
/// (the engine's store, then its folder), tried again a few times while
/// the engine lets go of it; a folder left behind keeps its id from being
/// used again.
pub fn remove(id: &str, cx: &mut App) -> Result<(), String> {
    let m = crate::accounts::model(cx);
    let name = m.read(cx).list.get(id).filter(|a| a.kind == WEB).map(|a| a.name.clone()).ok_or("No such browser profile")?;
    if let Some(why) = in_use(id, &name, cx) {
        return Err(why);
    }
    let folder = dir(Some(id), cx);
    m.update(cx, |m, cx| m.remove(id, cx))?;
    erase(folder, 0, cx);
    Ok(())
}

/// Tests: the folders whose stores were erased (nothing is touched).
#[cfg(test)]
#[derive(Default)]
pub struct Erased(pub Vec<PathBuf>);

#[cfg(test)]
impl Global for Erased {}

fn erase(folder: PathBuf, attempt: u32, cx: &mut App) {
    #[cfg(test)]
    {
        let _ = attempt;
        if !cx.has_global::<Erased>() {
            cx.set_global(Erased::default());
        }
        cx.global_mut::<Erased>().0.push(folder);
    }
    #[cfg(not(test))]
    {
        let (tx, rx) = futures::channel::oneshot::channel();
        crate::webview::erase_store(
            &folder,
            Box::new(move |res| {
                let _ = tx.send(res);
            }),
        );
        cx.spawn(async move |cx| {
            let res = rx.await.unwrap_or_else(|_| Err("no answer".into()));
            if let Err(e) = res {
                if attempt < 3 {
                    // The engine may still hold it a moment after its last view.
                    cx.background_executor().timer(std::time::Duration::from_secs(2 + 4 * attempt as u64)).await;
                    cx.update(|cx| erase(folder, attempt + 1, cx));
                } else {
                    eprintln!("pitwall: couldn't delete a removed browser profile's data ({}): {e}", folder.display());
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::paths::Paths;

    /// Default always; a profile only where each has its own store; one gone
    /// or a tool's account falls back to Default.
    #[test]
    fn which_profile_a_page_gets() {
        let t = pitwall_core::testing::TempDir::new("web-profiles");
        let paths = Paths::new(t.path().join("pw"));
        let mut list = Accounts::default();
        let work = list.add_web(&paths, "Work").unwrap();
        list.set_project_default("/work/alpha", WEB, Some(&work.id));

        assert!(usable(None, false) && usable(Some(DEFAULT), false));
        assert!(!usable(Some("web-work"), false), "before macOS 14: Default only");
        assert!(usable(Some("web-work"), true));

        assert_eq!(resolve(&list, None, true), (None, false));
        assert_eq!(resolve(&list, Some(DEFAULT), true), (None, false));
        assert_eq!(resolve(&list, Some("web-work"), true), (Some("web-work".into()), false));
        assert_eq!(resolve(&list, Some("web-gone"), true), (None, true), "removed: Default, said once");
        assert_eq!(resolve(&list, Some("web-work"), false), (None, true), "can't be used here: Default, said");
        list.accounts.push(Account { id: "claude-x".into(), kind: "claude".into(), name: "X".into(), dir: "/x".into() });
        assert_eq!(resolve(&list, Some("claude-x"), true), (None, true), "not a profile");

        assert_eq!(of_project(&list, "/work/alpha", true).as_deref(), Some("web-work"));
        assert_eq!(of_project(&list, "/work/beta", true), None);
        assert_eq!(of_project(&list, "/work/alpha", false), None, "before macOS 14: Default");
    }
}
