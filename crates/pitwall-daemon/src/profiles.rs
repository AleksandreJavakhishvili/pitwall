//! Accounts, extension connections, profiles and what projects pick
//! (docs/spec/profiles.md), on `accounts.json` in the engine's data
//! folder: the same `pitwall_core::accounts` calls the app makes. The app
//! reads the file again when it changes. Changes that decide what agents
//! launch with ask the user unless Pitwall's own window asked; nothing
//! restarts: the answer says which running agents still use what they had.

use serde_json::Value;

use pitwall_core::accounts::{ext_of, Accounts, From, DEFAULT, GIT, NONE, WEB};
use pitwall_core::onboarding::project_list::normalize;
use pitwall_proto::{
    code, method, AccountItem, AccountsView, ErrorBody, GitItem, PickView, ProfilePick, ProfileRef, ProfileSave, ProfileView, ProjectAccounts, ProjectPath, ProjectPick,
    ProjectProfile, Risk, SlotView,
};

use crate::identity::Caller;
use crate::manage::approve;
use crate::server::Server;

type Res = Result<Value, ErrorBody>;

fn ok(v: impl serde::Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

fn bad(e: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::BAD_PARAMS, e)
}

fn load(s: &Server) -> Accounts {
    Accounts::load(s.engine.paths())
}

fn save(s: &Server, a: &Accounts) -> Result<(), ErrorBody> {
    a.save(s.engine.paths()).map_err(|e| ErrorBody::new(code::OTHER, e))
}

/// The slot a word names: `browser` (or `web`), `git`, `ext:<id>`, or a
/// tool kind that can have accounts.
pub fn slot_of(s: &Server, word: &str) -> Result<String, ErrorBody> {
    let w = word.trim().to_lowercase();
    match w.as_str() {
        "browser" | WEB => return Ok(WEB.into()),
        GIT => return Ok(GIT.into()),
        _ => {}
    }
    if let Some(ext) = ext_of(&w) {
        pitwall_core::extensions::check_id(ext).map_err(bad)?;
        return Ok(w);
    }
    match s.engine.kinds().find(&w) {
        Some(k) if k.account_env.is_some() => Ok(k.id),
        Some(k) => Err(bad(format!("{} has one login only", k.name))),
        None => Err(bad(format!("no slot \"{word}\": a tool's kind (claude, codex, …), git, browser or ext:<extension id>"))),
    }
}

/// What a slot is called.
fn label(s: &Server, slot: &str) -> String {
    match slot {
        WEB => "Browser account".into(),
        GIT => "Git identity".into(),
        _ => match ext_of(slot) {
            Some(ext) => {
                let found = pitwall_core::extensions::discover(&s.engine.paths().extensions_dir());
                found.into_iter().filter_map(|f| f.manifest.ok()).find(|m| m.id == ext).map(|m| m.name).unwrap_or_else(|| ext.to_string())
            }
            None => s.engine.kinds().find(slot).map(|k| k.name).unwrap_or_else(|| slot.to_string()),
        },
    }
}

/// An id's name in a slot.
fn name_of(a: &Accounts, slot: &str, id: Option<&str>) -> String {
    match id {
        None if ext_of(slot).is_some() => "None".into(),
        None if slot == WEB => "Default".into(),
        None => "Your own".into(),
        Some(id) if slot == GIT => a.git_identity(id).map(|g| g.name.clone()).unwrap_or_else(|| id.to_string()),
        Some(id) => a.get(id).map(|x| x.name.clone()).unwrap_or_else(|| id.to_string()),
    }
}

fn profile_view(s: &Server, a: &Accounts, id: &str) -> Option<ProfileView> {
    let p = a.profile(id)?;
    let picks = p
        .picks
        .iter()
        .map(|(slot, v)| PickView {
            slot: slot.clone(),
            slot_label: label(s, slot),
            value: v.clone(),
            name: match v.as_str() {
                DEFAULT => "Default".into(),
                NONE => "None".into(),
                id => name_of(a, slot, Some(id)),
            },
        })
        .collect();
    Some(ProfileView { id: p.id.clone(), name: p.name.clone(), picks, projects: a.profile_projects(&p.id) })
}

fn view(s: &Server, a: &Accounts) -> AccountsView {
    let accounts = a
        .accounts
        .iter()
        .map(|x| AccountItem {
            id: x.id.clone(),
            slot: x.kind.clone(),
            slot_label: label(s, &x.kind),
            name: x.name.clone(),
            default: a.defaults.get(&x.kind) == Some(&x.id),
        })
        .collect();
    let git = a.git.iter().map(|g| GitItem { id: g.id.clone(), name: g.name.clone(), email: g.email.clone() }).collect();
    let profiles = a.profiles.iter().filter_map(|p| profile_view(s, a, &p.id)).collect();
    AccountsView { accounts, git, profiles }
}

pub fn list(s: &Server) -> Res {
    ok(view(s, &load(s)))
}

fn find_profile(a: &Accounts, key: &str) -> Result<String, ErrorBody> {
    a.find_profile(key).map(|p| p.id.clone()).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no profile \"{key}\" (see `pitwall profile list`)")))
}

/// `profile.save`.
pub fn save_profile(s: &Server, r: ProfileSave) -> Res {
    let mut a = load(s);
    let id = match r.id.as_deref() {
        None => a.add_profile(&r.name).map_err(bad)?.id,
        Some(key) => {
            let id = find_profile(&a, key)?;
            a.rename_profile(&id, &r.name).map_err(bad)?;
            id
        }
    };
    save(s, &a)?;
    ok(profile_view(s, &a, &id))
}

pub fn set(s: &Server, caller: &Caller, r: ProfilePick) -> Res {
    let mut a = load(s);
    let id = find_profile(&a, &r.profile)?;
    let slot = slot_of(s, &r.slot)?;
    let projects = a.profile_projects(&id);
    a.set_profile_pick(&id, &slot, r.value.as_deref()).map_err(bad)?;
    if !projects.is_empty() {
        let name = a.profile(&id).map(|p| p.name.clone()).unwrap_or_default();
        let what = match &r.value {
            Some(v) => format!("{} of profile \"{name}\" becomes {v}", label(s, &slot)),
            None => format!("profile \"{name}\" no longer decides {}", label(s, &slot)),
        };
        let details = vec![format!("Agents of {} start with it from now on.", projects.join(", "))];
        approve(s, caller, method::PROFILE_SET, what, details, Risk::Low, "Nothing changed.")?;
    }
    save(s, &a)?;
    ok(profile_view(s, &a, &id))
}

pub fn delete(s: &Server, caller: &Caller, r: ProfileRef) -> Res {
    let mut a = load(s);
    let id = find_profile(&a, &r.profile)?;
    let name = a.profile(&id).map(|p| p.name.clone()).unwrap_or_default();
    let projects = a.profile_projects(&id);
    let mut details = vec!["Its accounts stay.".to_string()];
    if !projects.is_empty() {
        details.push(format!("{} go back to no profile, keeping what they run under as their own picks.", projects.join(", ")));
    }
    approve(s, caller, method::PROFILE_DELETE, format!("delete the profile \"{name}\""), details, Risk::Low, "Nothing was deleted.")?;
    a.remove_profile(&id);
    save(s, &a)?;
    ok(view(s, &a))
}

fn project_view(s: &Server, a: &Accounts, folder: &str) -> ProjectAccounts {
    let slots = a
        .effective(folder)
        .into_iter()
        .map(|(slot, r)| SlotView {
            slot_label: label(s, &slot),
            name: name_of(a, &slot, r.id.as_deref()),
            from: match r.from {
                From::Project => "project".into(),
                From::Profile(_) => "profile".into(),
                From::Default => "default".into(),
            },
            own: a.project_pick(folder, &slot).map(String::from),
            id: r.id,
            slot,
        })
        .collect();
    ProjectAccounts { project: folder.to_string(), profile: a.profile_of(folder).map(|p| p.name.clone()), slots, still_running: vec![] }
}

fn folder(path: &str) -> Result<String, ErrorBody> {
    let f = normalize(path);
    if f.is_empty() {
        return Err(bad("which project? Give its folder"));
    }
    Ok(f)
}

pub fn project(s: &Server, r: ProjectPath) -> Res {
    let f = folder(&r.path)?;
    ok(project_view(s, &load(s), &f))
}

/// The project's running agents that still use what they had for the
/// slots that changed.
fn still_running(s: &Server, before: &Accounts, after: &Accounts, folder: &str) -> Vec<String> {
    let changed = Accounts::changed_slots(before, after, folder);
    let mut out = Vec::new();
    for v in s.engine.views().into_iter().filter(|v| v.running && (v.project == folder || v.cwd == folder)) {
        for slot in &changed {
            let new = after.resolve_launch(&v.cwd, &v.project, slot).id;
            let line = if *slot == v.kind {
                (v.account != new).then(|| format!("{} still runs on its current {} login: move it in Pitwall (Run on account) to use the new one.", v.name, label(s, slot)))
            } else if slot == GIT {
                Some(format!("{} keeps its git identity until it restarts (`pitwall agent restart`).", v.name))
            } else if ext_of(slot).is_some() {
                Some(format!("{} keeps its {} variables until it restarts.", v.name, label(s, slot)))
            } else {
                None
            };
            out.extend(line);
        }
    }
    out
}

pub fn pick(s: &Server, caller: &Caller, r: ProjectPick) -> Res {
    let f = folder(&r.path)?;
    let slot = slot_of(s, &r.slot)?;
    let mut a = load(s);
    let before = a.clone();
    let value = r.value.as_deref().map(|v| a.check_pick(&slot, v)).transpose().map_err(bad)?;
    a.set_project_pick(&f, &slot, value.as_deref());
    let what = match &value {
        Some(_) => format!("{} of {f} becomes {}", label(s, &slot), name_of(&a, &slot, a.resolve_slot(&f, &slot).id.as_deref())),
        None => format!("{f} takes {} from its profile or the default", label(s, &slot)),
    };
    approve(s, caller, method::PROJECT_PICK, what, vec!["Its agents start with it from now on; nothing restarts.".into()], Risk::Low, "Nothing changed.")?;
    save(s, &a)?;
    let mut v = project_view(s, &a, &f);
    v.still_running = still_running(s, &before, &a, &f);
    ok(v)
}

pub fn assign(s: &Server, caller: &Caller, r: ProjectProfile) -> Res {
    let f = folder(&r.path)?;
    let mut a = load(s);
    let before = a.clone();
    let id = match r.profile.as_deref().map(str::trim).filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case(NONE)) {
        Some(key) => Some(find_profile(&a, key)?),
        None => None,
    };
    a.set_project_profile(&f, id.as_deref()).map_err(bad)?;
    let what = match id.as_deref().and_then(|i| a.profile(i)) {
        Some(p) => format!("{f} uses the profile \"{}\"", p.name),
        None => format!("{f} uses no profile"),
    };
    let changed: Vec<String> = Accounts::changed_slots(&before, &a, &f).iter().map(|s2| label(s, s2)).collect();
    let details = vec![if changed.is_empty() { "Nothing it runs under changes.".to_string() } else { format!("Changes: {}. Its agents start with them from now on; nothing restarts.", changed.join(", ")) }];
    approve(s, caller, method::PROJECT_PROFILE, what, details, Risk::Low, "Nothing changed.")?;
    save(s, &a)?;
    let mut v = project_view(s, &a, &f);
    v.still_running = still_running(s, &before, &a, &f);
    ok(v)
}
