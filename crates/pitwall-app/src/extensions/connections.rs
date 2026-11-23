//! Extension connections in the app (docs/spec/profiles.md): accounts of
//! kind `ext:<id>` (`pitwall_core::accounts::picks`), each with its own
//! settings, secrets and setup actions done. Their list is the accounts
//! model's; their secrets are in the system's secure store under
//! `<secrets name>.<key>` ([`super::account`]).
//!
//! Settings → Extensions edits one connection of a card at a time
//! ([`chosen`]: the one picked there, else the default, else the first);
//! a value or secret set where there is none makes one named after the
//! extension.
//!
//! **Carried over** ([`migrate`]): an extension set up before connections
//! (its `values`, `secretsSet` and `actionsDone` in `ui.json`) becomes one
//! connection, the extension's default, its keychain items as they are.
//! `ui.json` keeps a copy of that connection ([`mirror`]) so an older
//! Pitwall, run after this one, still finds its settings; what such a
//! Pitwall changes there is taken back into the connection on the next
//! start.

use std::collections::BTreeMap;

use gpui::App;
use pitwall_core::accounts::Account;
use pitwall_core::extensions::plan::ToolAccount;
use pitwall_core::extensions::{Action, Manifest, SETTING_PLACEHOLDER};

use crate::main_screen::workspace::ExtensionPrefs;

/// Action `a` uses a setting (`{setting:KEY}`), so it runs for one
/// connection and is recorded per connection.
pub fn uses_setting(a: &Action) -> bool {
    a.steps.iter().any(|s| s.run.iter().any(|x| x.contains(SETTING_PLACEHOLDER)))
}

/// Extension `ext`'s connections, in the accounts' order.
pub fn list(ext: &str, cx: &App) -> Vec<Account> {
    crate::accounts::list(cx).map(|l| l.connections(ext).cloned().collect()).unwrap_or_default()
}

/// Extension `ext`'s default connection's id.
pub fn default_id(ext: &str, cx: &App) -> Option<String> {
    crate::accounts::list(cx).and_then(|l| l.default_connection(ext)).map(|c| c.id.clone())
}

/// The connection a card shows: `picked` if it is still there, else the
/// default, else the first.
pub fn chosen(ext: &str, picked: Option<&str>, cx: &App) -> Option<Account> {
    let list = crate::accounts::list(cx)?;
    picked.and_then(|p| list.connections(ext).find(|c| c.id == p)).or_else(|| list.default_connection(ext)).or_else(|| list.connections(ext).next()).cloned()
}

/// The connection Settings shows for extension `ext` without a pick.
pub fn settings_target(ext: &str, cx: &App) -> Option<Account> {
    chosen(ext, None, cx)
}

/// The plain settings of `ext`'s [`settings_target`].
pub fn settings_values(ext: &str, cx: &App) -> BTreeMap<String, String> {
    settings_target(ext, cx).map(|c| c.values).unwrap_or_default()
}

/// The keys of the secrets set of `ext`'s [`settings_target`].
pub fn settings_secrets_set(ext: &str, cx: &App) -> Vec<String> {
    settings_target(ext, cx).map(|c| c.secrets_set).unwrap_or_default()
}

/// What a run of action `a` (for connection `conn`) is kept under in
/// `ui.json`: `<action>@<connection>` when it uses a setting, else the
/// action's id.
pub fn key_of(a: &Action, conn: Option<&str>) -> String {
    match conn.filter(|_| uses_setting(a)) {
        Some(c) => format!("{}@{c}", a.id),
        None => a.id.clone(),
    }
}

/// `m`'s `ui.json` entry as connection `c` sees it: `c`'s settings and
/// secrets, and as done the actions that use a setting and ran for `c`
/// plus the others that ran at all, each only once its recorded run
/// covers every current account of the tools its steps are for
/// ([`not_yet_for`]).
pub fn prefs_for(m: &Manifest, c: Option<&Account>, cx: &App) -> ExtensionPrefs {
    let mut p = recorded(m, c, cx);
    let accounts = crate::accounts::tool_accounts(cx);
    if !accounts.is_empty() {
        let conn = c.map(|c| c.id.as_str());
        let covered = p.action_accounts.clone();
        p.actions_done.retain(|id| match m.actions.iter().find(|a| a.id == *id) {
            Some(a) => pitwall_core::extensions::plan::missing(a, &accounts, covered.get(&key_of(a, conn)).map_or(&[], Vec::as_slice)).is_empty(),
            None => true,
        });
    }
    p
}

/// The accounts of the tools `m`'s action `a` has steps for that its run,
/// recorded as done for connection `c` with the steps it has now, didn't
/// go through: added since, or the record is from before accounts were
/// kept. Empty when it isn't recorded as done (it runs for everything).
pub fn not_yet_for(m: &Manifest, a: &Action, c: Option<&Account>, cx: &App) -> Vec<ToolAccount> {
    let p = recorded(m, c, cx);
    if !p.actions_done.contains(&a.id) {
        return vec![];
    }
    let accounts = crate::accounts::tool_accounts(cx);
    let covered = p.action_accounts.get(&key_of(a, c.map(|c| c.id.as_str()))).cloned().unwrap_or_default();
    pitwall_core::extensions::plan::missing(a, &accounts, &covered).into_iter().cloned().collect()
}

/// [`prefs_for`] without the accounts: the runs recorded as done with
/// the steps their actions have now.
fn recorded(m: &Manifest, c: Option<&Account>, cx: &App) -> ExtensionPrefs {
    let mut p = crate::ui_state::get(cx).extension(&m.id);
    let per_conn: Vec<&str> = m.actions.iter().filter(|a| uses_setting(a)).map(|a| a.id.as_str()).collect();
    p.actions_done.retain(|a| !per_conn.contains(&a.as_str()));
    match c {
        Some(c) => {
            p.values = c.values.clone();
            p.secrets_set = c.secrets_set.clone();
            p.actions_done.extend(c.actions_done.iter().filter(|a| per_conn.contains(&a.as_str())).cloned());
        }
        None => {
            p.values.clear();
            p.secrets_set.clear();
        }
    }
    // A run whose steps changed since (an update) isn't done any more.
    let steps = std::mem::take(&mut p.action_steps);
    p.actions_done.retain(|a| steps_current(m, a, c.map(|c| c.id.as_str()), &steps));
    p.action_steps = steps;
    p
}

/// `ext`'s entry as its [`settings_target`] sees it.
pub fn prefs(ext: &str, cx: &App) -> ExtensionPrefs {
    let m = super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == ext).cloned().unwrap_or_else(|| Manifest::named(ext, ext));
    prefs_for(&m, settings_target(ext, cx).as_ref(), cx)
}

/// Connection `conn` of `ext`, or (`None`) the one Settings shows, made
/// (named after the extension) when there is none.
fn ensure(ext: &str, conn: Option<&str>, cx: &mut App) -> Result<Account, String> {
    if let Some(c) = chosen(ext, conn, cx) {
        return Ok(c);
    }
    let name = super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == ext).map(|m| m.name.clone()).unwrap_or_else(|| ext.to_string());
    let model = crate::accounts::model(cx);
    model.update(cx, |m, cx| m.add_connection(ext, &name, cx))
}

/// Set plain setting `key` of `ext`'s connection `conn` (`None`: the one
/// Settings shows), unset with `None`; its adapter starts again.
pub fn set_value(ext: &str, conn: Option<&str>, key: &str, value: Option<String>, cx: &mut App) -> Result<(), String> {
    if value.is_none() && chosen(ext, conn, cx).is_none() {
        return Ok(());
    }
    let c = ensure(ext, conn, cx)?;
    let model = crate::accounts::model(cx);
    model.update(cx, |a, cx| a.set_connection_value(&c.id, key, value.as_deref(), cx))
}

/// [`set_value`] on the connection Settings shows.
pub fn set_setting_value(ext: &str, key: &str, value: Option<String>, cx: &mut App) -> Result<(), String> {
    set_value(ext, None, key, value, cx)
}

/// Keep `secret` as `ext`'s connection `conn`'s `key` (`None` conn: the
/// one Settings shows; `None` secret: forget it). `done` gets the error.
pub fn set_secret_in(ext: &str, conn: Option<&str>, key: &str, secret: Option<String>, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    if secret.is_none() && chosen(ext, conn, cx).is_none() {
        return done(None, cx);
    }
    match ensure(ext, conn, cx) {
        Ok(c) => set_secret(&c.id, key, secret, cx, done),
        Err(e) => done(Some(e), cx),
    }
}

/// [`set_secret_in`] on the connection Settings shows.
pub fn set_settings_secret(ext: &str, key: &str, secret: Option<String>, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    set_secret_in(ext, None, key, secret, cx, done)
}

/// Keep `secret` as connection `id`'s `key` (`None`: forget it), off the
/// UI thread; then record that it is (or isn't) set, and its adapter
/// starts again. `done` gets the error, if any.
pub fn set_secret(id: &str, key: &str, secret: Option<String>, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    let Some(c) = crate::accounts::list(cx).and_then(|l| l.get(id)).cloned() else {
        return done(Some("That connection is gone".into()), cx);
    };
    let Some(ext) = pitwall_core::accounts::ext_of(&c.kind).map(String::from) else {
        return done(Some("That isn't an extension's connection".into()), cx);
    };
    let store = super::secret_store(cx);
    let acct = super::account(c.secrets_name(), key, cx);
    let set = secret.is_some();
    let job = cx.background_executor().spawn(async move {
        match secret {
            Some(s) => store.set(&acct, &s),
            None => store.delete(&acct),
        }
    });
    let key = key.to_string();
    cx.spawn(async move |cx| {
        let res = job.await;
        cx.update(|cx| {
            let mut err = res.err();
            if err.is_none() {
                let h = super::hub::hub(cx);
                h.update(cx, |h, _| h.bump_secrets(&ext, &c.id));
                let model = crate::accounts::model(cx);
                err = model.update(cx, |m, cx| m.set_connection_secret(&c.id, &key, set, cx)).err();
            }
            done(err, cx);
        });
    })
    .detach();
}

/// Record how setup action `a` of `m` ended (`ok`: it ran without a
/// failure): for connection `conn` when it uses a setting, else for the
/// extension.
pub fn record_action(m: &Manifest, a: &str, conn: Option<&str>, ok: bool, cx: &mut App) {
    let action = m.actions.iter().find(|x| x.id == a);
    let per_conn = action.is_some_and(uses_setting);
    // The steps that ran, kept with the record: changed steps untick it.
    let steps = action.map(pitwall_core::extensions::steps_digest).filter(|_| ok);
    let key = match action {
        Some(x) => key_of(x, conn),
        None => a.to_string(),
    };
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(&m.id, |e| match steps {
            Some(d) => {
                e.action_steps.insert(key, d);
            }
            None => {
                // Not done: nor for any account.
                e.action_steps.remove(&key);
                e.action_accounts.remove(&key);
            }
        });
    });
    if let (true, Some(conn)) = (per_conn, conn) {
        let model = crate::accounts::model(cx);
        if model.update(cx, |m, cx| m.set_connection_action(conn, a, ok, cx)).is_ok() {
            return;
        }
    }
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(&m.id, |e| {
            e.actions_done.retain(|x| x != a);
            if ok {
                e.actions_done.push(a.to_string());
            }
        });
    });
}

/// Keep the accounts the run of `m`'s action `a` (for connection
/// `conn`) went through without a failure, `ok`: added to those already
/// kept for a run for missing accounts only (`partial`), else in their
/// place.
pub fn record_accounts(m: &Manifest, a: &str, conn: Option<&str>, partial: bool, ok: &[String], cx: &mut App) {
    let Some(action) = m.actions.iter().find(|x| x.id == a) else { return };
    let key = key_of(action, conn);
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(&m.id, |e| {
            let before = e.action_accounts.get(&key).cloned().unwrap_or_default();
            let now = pitwall_core::extensions::plan::covered_after(&before, partial, ok);
            if now.is_empty() {
                e.action_accounts.remove(&key);
            } else {
                e.action_accounts.insert(key, now);
            }
        });
    });
}

/// Keep that the user confirmed the steps setup action `a` of `m` (for
/// connection `conn`) has now, and when: Pitwall runs exactly those for
/// an account added later ([`confirmed`]).
pub fn record_confirmed(m: &Manifest, a: &Action, conn: Option<&str>, cx: &mut App) {
    if !a.setup {
        return;
    }
    let key = key_of(a, conn);
    let c = crate::main_screen::workspace::Confirmed::now(pitwall_core::extensions::steps_digest(a));
    crate::ui_state::update(cx, |ui| *ui = std::mem::take(ui).with_extension(&m.id, |e| {
        e.action_confirmed.insert(key, c);
    }));
}

/// When the user confirmed the steps setup action `a` of `m` (for
/// connection `conn`) has now: `Some(None)` for a run recorded before
/// confirmations were kept, whose digest (the steps the user ran) stands
/// for one, except for a development folder's extension. `None`: its
/// steps weren't confirmed as they are.
pub fn confirmed(m: &Manifest, a: &Action, conn: Option<&str>, cx: &App) -> Option<Option<u64>> {
    if !a.setup {
        return None;
    }
    let p = crate::ui_state::get(cx).extension(&m.id);
    let key = key_of(a, conn);
    let now = pitwall_core::extensions::steps_digest(a);
    match p.action_confirmed.get(&key) {
        Some(c) => (c.steps == now).then_some(Some(c.at).filter(|at| *at > 0)),
        None if super::dev_folder(&m.id, cx).is_none() => (p.action_steps.get(&key) == Some(&now)).then_some(None),
        None => None,
    }
}

/// Whether the recorded run of `m`'s action `a` (for connection `conn`)
/// ran the steps it has now: a record without a digest (from before
/// digests, sealed at start) counts.
pub fn steps_current(m: &Manifest, a: &str, conn: Option<&str>, steps: &BTreeMap<String, String>) -> bool {
    let Some(action) = m.actions.iter().find(|x| x.id == a) else { return true };
    steps.get(&key_of(action, conn)).is_none_or(|d| *d == pitwall_core::extensions::steps_digest(action))
}

/// Remove connection `id`: its secrets leave the secure store, and the
/// projects, profiles and default that named it forget it. `new_default`:
/// the connection that becomes the extension's default when `id` was it
/// (never chosen silently: `None` leaves it with none).
pub fn remove_with(id: &str, new_default: Option<&str>, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    let Some(c) = crate::accounts::list(cx).and_then(|l| l.get(id)).cloned() else {
        return done(None, cx);
    };
    let ext = pitwall_core::accounts::ext_of(&c.kind).unwrap_or_default().to_string();
    let store = super::secret_store(cx);
    let accts: Vec<String> = c.secrets_set.iter().map(|k| super::account(c.secrets_name(), k, cx)).collect();
    let job = cx.background_executor().spawn(async move { accts.iter().map(|a| store.delete(a)).find_map(Result::err) });
    let (id, new_default) = (id.to_string(), new_default.map(String::from));
    cx.spawn(async move |cx| {
        let secret_err = job.await;
        cx.update(|cx| {
            let model = crate::accounts::model(cx);
            let mut err = model.update(cx, |m, cx| m.remove(&id, cx)).err();
            if err.is_none() {
                if let Some(d) = &new_default {
                    err = model.update(cx, |m, cx| m.set_default_connection(&ext, Some(d), cx)).err();
                }
            }
            let err = err.or(secret_err.map(|e| format!("Removed, but its secrets couldn't all be deleted: {e}")));
            done(err, cx);
        });
    })
    .detach();
}

/// [`remove_with`], no other connection made the default.
pub fn remove(id: &str, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    remove_with(id, None, cx, done)
}

/// The secure-store items of every connection of `ext` (for removing the
/// extension), and the connections themselves forgotten.
pub fn forget_all(ext: &str, cx: &mut App) -> Vec<String> {
    let conns = list(ext, cx);
    let mut items = Vec::new();
    for c in &conns {
        items.extend(c.secrets_set.iter().map(|k| super::account(c.secrets_name(), k, cx)));
    }
    let model = crate::accounts::model(cx);
    for c in conns {
        let _ = model.update(cx, |m, cx| m.remove(&c.id, cx));
    }
    items
}

/// What `ui.json` keeps for an older Pitwall from `ext`'s carried-over
/// connection: its values, secrets set, and the actions using a setting
/// that are done for it.
fn copy_of(m: Option<&Manifest>, c: &Account) -> (BTreeMap<String, String>, Vec<String>, Vec<String>) {
    let per_conn: Vec<String> = m.map(|m| m.actions.iter().filter(|a| uses_setting(a)).map(|a| a.id.clone()).collect()).unwrap_or_default();
    (c.values.clone(), c.secrets_set.clone(), c.actions_done.iter().filter(|a| per_conn.contains(a)).cloned().collect())
}

/// Carry each extension set up before connections over to one connection
/// (docs/spec/profiles.md, "Migration"): its `ui.json` `values`,
/// `secretsSet` and the `actionsDone` that use a setting become the
/// connection's, its secrets stay where they are in the secure store, it
/// becomes the extension's default, and the thread its inbox showed is
/// kept. When it was carried over before and an older Pitwall changed
/// those keys since, the connection takes them. Without accounts that can
/// be kept (no engine), nothing changes.
pub fn migrate(cx: &mut App) {
    let model = crate::accounts::model(cx);
    if !model.read(cx).live() {
        return;
    }
    let ui = crate::ui_state::get(cx);
    let manifests: BTreeMap<String, Manifest> = super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).map(|m| (m.id.clone(), m.clone())).collect();
    for (ext, prefs) in ui.extensions.iter() {
        let m = manifests.get(ext);
        let per_conn: Vec<String> = m.map(|m| m.actions.iter().filter(|a| uses_setting(a)).map(|a| a.id.clone()).collect()).unwrap_or_default();
        let actions: Vec<String> = prefs.actions_done.iter().filter(|a| per_conn.contains(a)).cloned().collect();
        let has = crate::accounts::list(cx).is_some_and(|l| l.connections(ext).next().is_some());
        if !has && !(prefs.values.is_empty() && prefs.secrets_set.is_empty() && actions.is_empty()) {
            let name = m.map(|m| m.name.clone()).unwrap_or_else(|| ext.clone());
            let (values, secrets) = (prefs.values.clone(), prefs.secrets_set.clone());
            let _ = model.update(cx, |md, cx| md.change(|l| Ok(l.adopt_connection(ext, &name, &values, &secrets, &actions)), cx));
        } else if has {
            let carried = crate::accounts::list(cx).and_then(|l| l.carried_over(ext).cloned());
            if let Some(c) = carried {
                // An older Pitwall ran since: its copy differs from ours.
                if copy_of(m, &c) != (prefs.values.clone(), prefs.secrets_set.clone(), actions.clone()) {
                    let (values, secrets) = (prefs.values.clone(), prefs.secrets_set.clone());
                    let _ = model.update(cx, |md, cx| md.change(|l| Ok(l.sync_carried_over(ext, &values, &secrets, &actions)), cx));
                }
            }
        }
        // The thread shown, as an older Pitwall wrote it: given its
        // connection's prefix.
        let target = crate::accounts::list(cx).and_then(|l| l.carried_over(ext).or_else(|| l.default_connection(ext)).map(|c| c.id.clone()));
        if let (Some(c), Some(sel)) = (target, &prefs.selected) {
            let conns = list(ext, cx);
            if !conns.iter().any(|x| sel.starts_with(&format!("{}:", x.id))) {
                let sel = super::hub::thread_id(&c, sel);
                crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(ext, |e| e.selected = Some(sel)));
            }
        }
    }
    mirror(cx);
}

/// Keep `ui.json`'s copy of each carried-over connection the same as the
/// connection (for an older Pitwall); with none (removed), the copy goes.
pub fn mirror(cx: &mut App) {
    let model = crate::accounts::model(cx);
    if !model.read(cx).live() {
        return;
    }
    let ui = crate::ui_state::get(cx);
    let manifests: BTreeMap<String, Manifest> = super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).map(|m| (m.id.clone(), m.clone())).collect();
    for (ext, prefs) in ui.extensions.iter() {
        let m = manifests.get(ext);
        let per_conn: Vec<String> = m.map(|m| m.actions.iter().filter(|a| uses_setting(a)).map(|a| a.id.clone()).collect()).unwrap_or_default();
        let carried = crate::accounts::list(cx).and_then(|l| l.carried_over(ext).cloned());
        let (values, secrets, acts) = match &carried {
            Some(c) => copy_of(m, c),
            None => Default::default(),
        };
        let mut done: Vec<String> = prefs.actions_done.iter().filter(|a| !per_conn.contains(a)).cloned().collect();
        done.extend(acts);
        let mut have = prefs.actions_done.clone();
        have.sort();
        let mut want = done.clone();
        want.sort();
        if prefs.values != values || prefs.secrets_set != secrets || have != want {
            crate::ui_state::update(cx, |u| {
                *u = std::mem::take(u).with_extension(ext, |e| {
                    e.values = values;
                    e.secrets_set = secrets;
                    e.actions_done = done;
                })
            });
        }
    }
}
