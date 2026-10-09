//! `settings.*`: Pitwall's settings over the socket (`pitwall settings`).
//!
//! The host (the app) owns the values and applies changes live through a
//! [`SettingsBackend`]; this module checks keys and values against the
//! registry (`pitwall_proto::settings`) and, for `approval` settings asked
//! by anyone but Pitwall's own UI, waits for the user's OK first.

use serde_json::Value;

use pitwall_proto::settings::{self, Sensitivity, Setting, SETTINGS};
use pitwall_proto::{code, method, ErrorBody, Risk, SettingKey, SettingSet, SettingView};

use crate::approvals::{Ask, Decision};
use crate::identity::Caller;
use crate::server::Server;

/// Where the settings live and how they change (the app's `ui.json`, its
/// hooks and CLI installers). Called on connection threads; may block.
pub trait SettingsBackend: Send + Sync {
    /// The current value of a registry key (`Null` when it can't be read).
    fn get(&self, s: &'static Setting) -> Result<Value, String>;
    /// Apply an allowed value (already validated, already approved) and
    /// return the value now in effect.
    fn set(&self, s: &'static Setting, value: Value) -> Result<Value, String>;
}

type Res = Result<Value, ErrorBody>;

fn backend(s: &Server) -> Result<&dyn SettingsBackend, ErrorBody> {
    s.settings.as_deref().ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "this Pitwall doesn't host settings"))
}

fn find(key: &str) -> Result<&'static Setting, ErrorBody> {
    settings::lookup(key).map_err(|e| ErrorBody::new(code::NOT_FOUND, e))
}

fn view(b: &dyn SettingsBackend, s: &'static Setting) -> Result<SettingView, ErrorBody> {
    let v = b.get(s).map_err(|e| ErrorBody::new(code::OTHER, e))?;
    Ok(s.view(v))
}

fn ok(v: impl serde::Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

pub fn list(s: &Server) -> Res {
    let b = backend(s)?;
    let all: Result<Vec<SettingView>, ErrorBody> = SETTINGS.iter().map(|s| view(b, s)).collect();
    ok(all?)
}

pub fn get(s: &Server, req: SettingKey) -> Res {
    let b = backend(s)?;
    ok(view(b, find(&req.key)?)?)
}

pub fn set(s: &Server, caller: &Caller, req: SettingSet) -> Res {
    change(s, caller, &req.key, Some(req.value))
}

pub fn reset(s: &Server, caller: &Caller, req: SettingKey) -> Res {
    change(s, caller, &req.key, None)
}

/// Set `key` to `value` (`None`: its default).
fn change(s: &Server, caller: &Caller, key: &str, value: Option<Value>) -> Res {
    let b = backend(s)?;
    let def = find(key)?;
    let value = value.unwrap_or_else(|| def.default_value());
    let value = def.validate_change(&value).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e))?;
    let now = b.get(def).map_err(|e| ErrorBody::new(code::OTHER, e))?;
    if now == value {
        return ok(def.view(now));
    }
    if def.sensitivity == Sensitivity::Approval && !caller.ui {
        let decision = s.approvals.ask(Ask {
            action: format!("{}:{}", method::SETTINGS_SET, def.key),
            summary: format!("change the setting {} to {value}", def.key),
            details: vec![
                def.description.to_string(),
                "It changes something outside Pitwall's own look, so Pitwall asks first.".into(),
            ],
            requester: caller.requester(),
            caller_key: caller.key(),
            risk: Risk::High,
        });
        match decision {
            Decision::Allowed | Decision::Remembered => {}
            Decision::Denied => {
                return Err(ErrorBody::new(code::DENIED, format!("The user didn't allow changing {}. Nothing changed.", def.key)));
            }
            Decision::TimedOut => {
                return Err(ErrorBody::new(
                    code::APPROVAL_TIMEOUT,
                    format!("Nobody answered the approval to change {} in time. Nothing changed.", def.key),
                ));
            }
        }
    }
    let got = b.set(def, value).map_err(|e| ErrorBody::new(code::OTHER, e))?;
    ok(def.view(got))
}
