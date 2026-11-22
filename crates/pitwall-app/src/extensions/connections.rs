//! Extension connections in the app (docs/spec/profiles.md): accounts of
//! kind `ext:<id>` (`pitwall_core::accounts::picks`), each with its own
//! settings and secrets. Their list is the accounts model's; their
//! secrets are in the system's secure store under
//! `<secrets name>.<key>` ([`super::account`]).
//!
//! Settings → Extensions edits one connection per extension (until it
//! draws them all): [`settings_target`], the extension's default
//! connection, else its first; a value or secret set where there is none
//! makes one named after the extension.
//!
//! [`migrate`] carries an extension set up before connections (its
//! `values` and `secretsSet` in `ui.json`) over to one connection, the
//! extension's default, its keychain items as they are.

use std::collections::BTreeMap;

use gpui::App;
use pitwall_core::accounts::Account;

/// The connection Settings shows for extension `ext`: its default, else
/// its first.
pub fn settings_target(ext: &str, cx: &App) -> Option<Account> {
    let list = crate::accounts::list(cx)?;
    list.default_connection(ext).or_else(|| list.connections(ext).next()).cloned()
}

/// The plain settings Settings shows for extension `ext`.
pub fn settings_values(ext: &str, cx: &App) -> BTreeMap<String, String> {
    settings_target(ext, cx).map(|c| c.values).unwrap_or_default()
}

/// The keys of the secrets set that Settings shows for extension `ext`.
pub fn settings_secrets_set(ext: &str, cx: &App) -> Vec<String> {
    settings_target(ext, cx).map(|c| c.secrets_set).unwrap_or_default()
}

/// Extension `ext`'s `ui.json` entry with Settings' connection's
/// settings and secrets in it, for Settings to read.
pub fn prefs(ext: &str, cx: &App) -> crate::main_screen::workspace::ExtensionPrefs {
    let mut p = crate::ui_state::get(cx).extension(ext);
    p.values = settings_values(ext, cx);
    p.secrets_set = settings_secrets_set(ext, cx);
    p
}

/// Settings' connection of extension `m`, made (named after it) when it
/// has none.
fn ensure(m_id: &str, cx: &mut App) -> Result<Account, String> {
    let model = crate::accounts::model(cx);
    if let Some(c) = settings_target(m_id, cx) {
        return Ok(c);
    }
    let name = super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == m_id).map(|m| m.name.clone()).unwrap_or_else(|| m_id.to_string());
    model.update(cx, |m, cx| m.add_connection(m_id, &name, cx))
}

/// Set plain setting `key` of Settings' connection of extension `m`
/// (`None`: unset); its adapter starts again.
pub fn set_setting_value(ext: &str, key: &str, value: Option<String>, cx: &mut App) -> Result<(), String> {
    if value.is_none() && settings_target(ext, cx).is_none() {
        return Ok(());
    }
    let c = ensure(ext, cx)?;
    let model = crate::accounts::model(cx);
    model.update(cx, |a, cx| a.set_connection_value(&c.id, key, value.as_deref(), cx))
}

/// Keep `secret` as Settings' connection of extension `ext`'s `key`
/// (`None`: forget it), off the UI thread; then record it and start its
/// adapter again. `done` gets the error, if any.
pub fn set_settings_secret(ext: &str, key: &str, secret: Option<String>, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    if secret.is_none() && settings_target(ext, cx).is_none() {
        return done(None, cx);
    }
    match ensure(ext, cx) {
        Ok(c) => set_secret(&c.id, key, secret, cx, done),
        Err(e) => done(Some(e), cx),
    }
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

/// Remove connection `id`: its secrets leave the secure store, and the
/// projects, profiles and default that named it forget it.
pub fn remove(id: &str, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    let Some(c) = crate::accounts::list(cx).and_then(|l| l.get(id)).cloned() else {
        return done(None, cx);
    };
    let store = super::secret_store(cx);
    let accts: Vec<String> = c.secrets_set.iter().map(|k| super::account(c.secrets_name(), k, cx)).collect();
    let job = cx.background_executor().spawn(async move { accts.iter().map(|a| store.delete(a)).find_map(Result::err) });
    let id = id.to_string();
    cx.spawn(async move |cx| {
        let secret_err = job.await;
        cx.update(|cx| {
            let model = crate::accounts::model(cx);
            let err = model.update(cx, |m, cx| m.remove(&id, cx)).err().or(secret_err.map(|e| format!("Removed, but its secrets couldn't all be deleted: {e}")));
            done(err, cx);
        });
    })
    .detach();
}

/// Carry each extension set up before connections over to one connection
/// (docs/spec/profiles.md, "Migration"): its `ui.json` `values` and
/// `secretsSet` become the connection's, its secrets stay where they are
/// in the secure store, it becomes the extension's default, and the
/// thread its inbox showed is kept. Without accounts that can be kept
/// (no engine), nothing changes.
pub fn migrate(cx: &mut App) {
    let ui = crate::ui_state::get(cx);
    let names: BTreeMap<String, String> = super::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).map(|m| (m.id.clone(), m.name.clone())).collect();
    let model = crate::accounts::model(cx);
    if !model.read(cx).live() {
        return;
    }
    for (ext, prefs) in ui.extensions.iter() {
        if prefs.values.is_empty() && prefs.secrets_set.is_empty() {
            continue;
        }
        let name = names.get(ext).cloned().unwrap_or_else(|| ext.clone());
        let (values, secrets) = (prefs.values.clone(), prefs.secrets_set.clone());
        let adopted = model.update(cx, |m, cx| m.change(|l| Ok(l.adopt_connection(ext, &name, &values, &secrets)), cx));
        let Ok(adopted) = adopted else { continue };
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension(ext, |e| {
                e.values.clear();
                e.secrets_set.clear();
                if let (Some(c), Some(sel)) = (&adopted, &e.selected) {
                    e.selected = Some(super::hub::thread_id(&c.id, sel));
                }
            });
        });
    }
}
