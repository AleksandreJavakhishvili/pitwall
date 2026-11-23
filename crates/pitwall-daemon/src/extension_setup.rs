//! `extension.*`: an agent sees how the user's extensions stand and moves
//! their setup forward (docs/spec/extensions.md, "Setup by an agent"). The
//! host ([`ExtensionsBackend`], the app) gathers the state and does what
//! Settings' buttons do; this module asks the user first.
//!
//! - Reading (`list`, `status`, `log`) is open to any caller but an
//!   extension, and changes nothing.
//! - `checkUpdates` looks at a source as "Check for updates" does (nothing
//!   is installed) and `secret` only shows the user the field in Pitwall;
//!   neither asks.
//! - Every `act` waits for the user's approval in Pitwall, which says
//!   exactly what will run (the host prepares it first and runs exactly
//!   that), whoever asks: any agent, or a program outside Pitwall. One
//!   request per extension at a time: another while one waits (or runs) is
//!   refused `busy`, so nobody can pile prompts up. A declined request is
//!   never asked again by Pitwall.
//! - Extensions never call these (`Ext::Never` in the method table).

use std::collections::HashSet;
use std::sync::Mutex;

use serde_json::Value;

use pitwall_proto::extensions::{ExtAct, ExtActed, ExtList, ExtLog, ExtLogRequest, ExtRef, ExtSecret, ExtStatus, EXT_PROTO};
use pitwall_proto::{code, method, ErrorBody, Risk, SettingsOpen, SETTINGS_PROTO};

use crate::approvals::{Ask, Decision};
use crate::identity::Caller;
use crate::server::Server;

/// The error code of a request refused while another for the same
/// extension waits.
pub const BUSY: &str = "busy";

/// What an act will do, to ask the user: kept by the host as `token`
/// until [`ExtensionsBackend::run`] or [`ExtensionsBackend::discard`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub token: u64,
    /// "run “Set up my agents” of Inbox" (the dialog says "<who> wants to
    /// <summary>.").
    pub summary: String,
    /// Exactly what runs, line by line.
    pub details: Vec<String>,
    pub risk: Risk,
}

/// The host's extensions. Called on connection threads; may block while
/// the host's UI thread works (and, in `run`, until what it started ends).
pub trait ExtensionsBackend: Send + Sync {
    fn list(&self) -> Result<ExtList, ErrorBody>;
    fn status(&self, r: &ExtRef) -> Result<ExtStatus, ErrorBody>;
    fn log(&self, r: &ExtLogRequest) -> Result<ExtLog, ErrorBody>;
    /// Check its source for a newer release and wait for the answer.
    fn check_updates(&self, id: &str) -> Result<ExtActed, ErrorBody>;
    /// What `act` would do, or why it can't be done now.
    fn prepare(&self, act: &ExtAct) -> Result<Prepared, ErrorBody>;
    /// Do what `token` was prepared as (the user approved), and wait for
    /// how it went.
    fn run(&self, token: u64) -> Result<ExtActed, ErrorBody>;
    /// The user declined `token` (or nobody answered): forget it.
    fn discard(&self, token: u64);
}

/// The extensions a request waits for (or runs) for.
#[derive(Default)]
pub struct Waiting(Mutex<HashSet<String>>);

/// While it lives, extension `id` has a request under way.
struct Turn<'a>(&'a Waiting, String);

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0 .0.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.1);
    }
}

impl Waiting {
    fn take(&self, id: &str) -> Result<Turn<'_>, ErrorBody> {
        let mut w = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !w.insert(id.to_string()) {
            return Err(ErrorBody::new(BUSY, format!("Another request for “{id}” still waits for the user's answer, or still runs: wait for it to end (it answers when it does), then run `pitwall extension status {id}`.")));
        }
        Ok(Turn(self, id.to_string()))
    }
}

type Res = Result<Value, ErrorBody>;

fn backend(s: &Server) -> Result<&dyn ExtensionsBackend, ErrorBody> {
    s.ext_backend.as_deref().ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "this Pitwall can't set up extensions from the command line (update Pitwall)"))
}

fn ok(v: impl serde::Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

fn id_of(id: &str) -> Result<&str, ErrorBody> {
    let id = id.trim();
    if id.is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "which extension? (its id, from `pitwall extension list`)"));
    }
    Ok(id)
}

pub fn list(s: &Server) -> Res {
    ok(backend(s)?.list()?)
}

pub fn status(s: &Server, r: ExtRef) -> Res {
    id_of(&r.id)?;
    ok(backend(s)?.status(&r)?)
}

pub fn log(s: &Server, r: ExtLogRequest) -> Res {
    id_of(&r.id)?;
    ok(backend(s)?.log(&r)?)
}

pub fn check_updates(s: &Server, r: ExtRef) -> Res {
    let id = id_of(&r.id)?;
    let b = backend(s)?;
    let _turn = s.ext_waiting.take(id)?;
    ok(b.check_updates(id)?)
}

/// Show the user the secret's field in Settings, next to the caller's
/// terminal; the value never passes here.
pub fn secret(s: &Server, caller: &Caller, r: ExtSecret) -> Res {
    let id = id_of(&r.id)?;
    let st = backend(s)?.status(&pitwall_proto::extensions::ExtRef { id: id.to_string(), connection: r.connection.clone() })?;
    let key = r.key.trim();
    let Some(def) = st.settings.iter().find(|x| x.key == key) else {
        let keys: Vec<&str> = st.settings.iter().filter(|x| x.kind == "secret").map(|x| x.key.as_str()).collect();
        return Err(ErrorBody::new(code::NOT_FOUND, format!("{} has no setting “{key}” (its secrets: {})", st.name, if keys.is_empty() { "none".to_string() } else { keys.join(", ") })));
    };
    if def.kind != "secret" {
        return Err(ErrorBody::new(code::BAD_PARAMS, format!("“{key}” isn't a secret: set it with `pitwall extension set {id} {key} <value>`")));
    }
    let open = SettingsOpen { agent_id: r.agent_id.clone(), page: Some("extensions".into()), extension: Some(id.to_string()), secret: Some(key.to_string()), connection: r.connection.clone(), proto: Some(SETTINGS_PROTO) };
    let set = st.connections.iter().find(|c| st.connection.as_deref().is_none_or(|n| c.name == n || c.id == n)).and_then(|c| c.secrets.get(key)).copied().unwrap_or(false);
    let already = if set { format!("{} is set already; the user may replace it. ", def.label) } else { String::new() };
    let text = match crate::viewer::settings(s, caller, open) {
        Ok(shown) => format!(
            "{already}Waiting for the user to enter {} in Pitwall (its field is marked on {}'s card{}). Its value never comes to you: run `pitwall extension status {id}` to see when it is set. Never ask the user to paste it into the chat.",
            def.label,
            st.name,
            if shown.in_background { "; Pitwall isn't in front, so they see it when they switch to it" } else { "" },
        ),
        // Settings can't be shown from here (turned off, or not from an
        // agent): the user is told where instead.
        Err(e) => format!(
            "{already}Settings wasn't shown ({}). Ask the user to open Pitwall's Settings → Extensions → {} and enter {} there. Its value never comes to you: run `pitwall extension status {id}` to see when it is set. Never ask the user to paste it into the chat.",
            e.message.trim_end_matches('.'),
            st.name,
            def.label,
        ),
    };
    ok(ExtActed { proto: EXT_PROTO, id: id.to_string(), op: "secret".into(), approved: false, text, running: false, status: None })
}

/// Ask the user, then do it.
pub fn act(s: &Server, caller: &Caller, a: ExtAct) -> Res {
    let id = id_of(&a.id)?.to_string();
    let b = backend(s)?;
    let _turn = s.ext_waiting.take(&id)?;
    let what = a.op.word();
    let p = b.prepare(&a)?;
    let decision = s.approvals.ask(Ask {
        action: format!("{}:{what}", method::EXTENSION_ACT),
        summary: p.summary.clone(),
        details: p.details.clone(),
        requester: caller.requester(),
        caller_key: caller.key(),
        risk: p.risk,
    });
    match decision {
        Decision::Allowed | Decision::Remembered => {}
        Decision::Denied => {
            b.discard(p.token);
            return Err(ErrorBody::new(code::DENIED, format!("The user declined in Pitwall ({what} {id}). Nothing ran. Don't ask again unless the user asks you to.")));
        }
        Decision::TimedOut => {
            b.discard(p.token);
            return Err(ErrorBody::new(code::APPROVAL_TIMEOUT, format!("Nobody answered the approval in Pitwall in time ({what} {id}). Nothing ran.")));
        }
    }
    let mut done = b.run(p.token)?;
    done.approved = true;
    ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_request_per_extension_at_a_time() {
        let w = Waiting::default();
        let first = w.take("kit").unwrap();
        let e = w.take("kit").err().unwrap();
        assert_eq!(e.code, BUSY);
        assert!(e.message.contains("still waits"), "{}", e.message);
        assert!(w.take("inbox").is_ok(), "another extension is its own");
        drop(first);
        assert!(w.take("kit").is_ok(), "once it ended");
    }
}
