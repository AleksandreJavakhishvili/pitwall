//! The method table (architecture.md §4: every method's risk in one place)
//! and what each method does, in terms of `pitwall-core`.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use pitwall_core::engine::{changes, lifecycle};
use pitwall_core::model::{AdoptSessionRequest, CreateAgentRequest};
use pitwall_core::onboarding::{self, places};
use pitwall_core::provider::{Locator, MachineId, ProviderId};
use pitwall_proto::{code, method, AgentCreate, AgentCreated, AgentRef, AgentView, ApprovalAnswer, CreateForm, ErrorBody, FormRequest, Queued, Risk, SessionAdd, SessionAdded, SessionFilter};
use pitwall_core::extensions::permissions::AGENTS_FOCUS;

use crate::approvals::{Ask, Decision};
use crate::identity::Caller;
use pitwall_core::extensions::activity::Via;
use pitwall_core::extensions::permissions::{AGENTS_QUEUE, AGENTS_READ, AGENTS_START, PROFILES_READ, PROJECTS_READ, SHOW, WORKTREES_READ};
use crate::{manage, workspace};
use crate::server::Server;

/// Who may call a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Anyone: read-only, or only adds to Pitwall (nothing changes
    /// anywhere else).
    Open,
    /// Anyone, but the user approves in Pitwall first when the request does
    /// what the note says.
    AskWhen(&'static str, Risk),
    /// Pitwall's own (verified) UI only.
    Ui,
    /// Pitwall's own UI, or an extension granted the method's permission;
    /// never an agent or another program.
    UiOrExtension,
}

/// What an extension may call (docs/spec/extension-api.md, "Permissions"):
/// the permission column. An extension is a caller of its own, from its
/// adapter's stdio or a process its adapter started; its user's own
/// terminal keeps [`Access`] alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ext {
    /// With this permission granted.
    Grant(&'static str),
    /// A later Pitwall, with this permission: refused as not available yet.
    Later(&'static str),
    /// Never an extension's.
    Never,
}

/// Why the project-rules methods ask.
const PROJECT_WRITE: &str = "always: writes rule files into the project's folder (shown in Review, to commit)";

/// Every method: who may call it, and what an extension needs to.
pub const METHODS: &[(&str, Access, Ext)] = &[
    (method::AGENT_LIST, Access::Open, Ext::Grant(AGENTS_READ)),
    // Read-only: git reads on the agent's machine.
    (method::AGENT_REFRESH, Access::Open, Ext::Never),
    (
        method::AGENT_CREATE,
        // Low; High (asked every time) when it also makes a workspace or an
        // agent user there.
        Access::AskWhen("`machine` is a platform's (agw): creates and starts a session on that machine", Risk::Low),
        // An extension's request is asked every time (actions.rs).
        Ext::Grant(AGENTS_START),
    ),
    // Moves the focus to an agent's tile while Pitwall is in front.
    (method::AGENT_FOCUS, Access::UiOrExtension, Ext::Grant(AGENTS_FOCUS)),
    (method::MACHINE_LIST, Access::Open, Ext::Never),
    (method::MACHINE_FORM, Access::Open, Ext::Never),
    (method::SESSION_LIST, Access::Open, Ext::Never),
    (method::SESSION_ADD, Access::AskWhen("`start`: starts a stopped session on its machine", Risk::Low), Ext::Never),
    (method::APPROVAL_LIST, Access::Ui, Ext::Never),
    (method::APPROVAL_ANSWER, Access::Ui, Ext::Never),
    (method::SETTINGS_LIST, Access::Open, Ext::Never),
    (method::SETTINGS_GET, Access::Open, Ext::Never),
    // `safe` settings (Pitwall's own look) apply directly.
    (method::SETTINGS_SET, Access::AskWhen("the setting is `approval` (changes something outside Pitwall)", Risk::High), Ext::Never),
    (method::SETTINGS_RESET, Access::AskWhen("the setting is `approval` (changes something outside Pitwall)", Risk::High), Ext::Never),
    // Managing agents (manage.rs). Stop/restart: Low on this Mac, High
    // (asked every time) on another machine.
    (method::AGENT_STOP, Access::AskWhen("always: ends the agent's process", Risk::Low), Ext::Never),
    (method::AGENT_RESTART, Access::AskWhen("always: ends and starts the agent's process", Risk::Low), Ext::Never),
    (method::AGENT_REMOVE, Access::AskWhen("always; `deleteWorktree` also deletes its worktree", Risk::High), Ext::Never),
    (method::AGENT_RENAME, Access::Open, Ext::Never),
    (method::AGENT_STATUS, Access::Open, Ext::Grant(AGENTS_READ)),
    // Only which worktrees Review and the sidebar show as the agent's.
    (method::AGENT_TRACK, Access::Open, Ext::Never),
    (method::AGENT_WAIT, Access::Open, Ext::Never),
    // Queued prompts go to agents already in Pitwall, as the UI's queue.
    // An extension's item is marked as its own and paced (actions.rs).
    (method::QUEUE_ADD, Access::Open, Ext::Grant(AGENTS_QUEUE)),
    (method::QUEUE_LIST, Access::Open, Ext::Never),
    (method::QUEUE_REMOVE, Access::Open, Ext::Never),
    // Sending is the user's (or Auto-send's, as the user set it).
    (method::QUEUE_SEND, Access::Open, Ext::Never),
    // Pitwall's project list; never touches the folders.
    (method::PROJECT_LIST, Access::Open, Ext::Grant(PROJECTS_READ)),
    // Every project's git worktrees (read-only git calls).
    (method::WORKTREE_LIST, Access::Open, Ext::Grant(WORKTREES_READ)),
    (method::PROJECT_ADD, Access::Open, Ext::Never),
    (method::PROJECT_REMOVE, Access::Open, Ext::Never),
    // Accounts and profiles (profiles.rs): what agents launch with.
    (method::ACCOUNTS_LIST, Access::Open, Ext::Grant(PROFILES_READ)),
    (method::PROFILE_SAVE, Access::Open, Ext::Never),
    (method::PROFILE_SET, Access::AskWhen("a project uses the profile: decides what its agents launch with", Risk::Low), Ext::Never),
    (method::PROFILE_DELETE, Access::AskWhen("always: the profile goes (its projects keep what they run under)", Risk::Low), Ext::Never),
    (method::PROJECT_ACCOUNTS, Access::Open, Ext::Grant(PROFILES_READ)),
    (method::PROJECT_PICK, Access::AskWhen("always: decides what the project's agents launch with", Risk::Low), Ext::Never),
    (method::PROJECT_PROFILE, Access::AskWhen("always: decides what the project's agents launch with", Risk::Low), Ext::Never),
    (method::RULES_LIST, Access::Open, Ext::Never),
    (method::RULES_SETS, Access::Open, Ext::Never),
    (method::RULES_APPLY, Access::AskWhen("the agent works in the main checkout: rule files are written there", Risk::High), Ext::Never),
    (method::RULES_DEFAULT, Access::Open, Ext::Never),
    (method::RULES_SHOW, Access::Open, Ext::Never),
    // New entries only add to Pitwall's library; replacing one asks.
    (method::RULES_LIBRARY_NEW, Access::AskWhen("`overwrite` and the library has one of that name: replaces it", Risk::Low), Ext::Never),
    // Project rules write into the user's repository (to be committed, shown in Review).
    (method::RULES_PROJECT_STATUS, Access::Open, Ext::Never),
    (method::RULES_PROJECT_UPDATES, Access::Open, Ext::Never),
    (method::RULES_PROJECT_ASSIGN, Access::AskWhen(PROJECT_WRITE, Risk::Low), Ext::Never),
    (method::RULES_PROJECT_UNASSIGN, Access::AskWhen(PROJECT_WRITE, Risk::Low), Ext::Never),
    (method::RULES_PROJECT_UPDATE, Access::AskWhen(PROJECT_WRITE, Risk::Low), Ext::Never),
    (method::RULES_PROJECT_IMPORT, Access::AskWhen(PROJECT_WRITE, Risk::Low), Ext::Never),
    (method::RULES_PROJECT_GENERATE, Access::AskWhen(PROJECT_WRITE, Risk::Low), Ext::Never),
    (method::RULES_PROJECT_MOVE, Access::AskWhen(PROJECT_WRITE, Risk::Low), Ext::Never),
    // Only where rules.apply wouldn't ask: worktrees and main checkouts the user confirmed.
    (method::RULES_REAPPLY, Access::Open, Ext::Never),
    // Sets are Pitwall's own data; deleting one takes it from projects and agents.
    (method::RULES_SET_SAVE, Access::Open, Ext::Never),
    (method::RULES_SET_DELETE, Access::AskWhen("always: projects and agents that use the set lose it", Risk::High), Ext::Never),
    // A project's own .rulesync/ is read in place; a file or a repository is copied in.
    (method::RULES_IMPORT, Access::AskWhen("`kind` is file or git: copies rules from outside Pitwall into its library", Risk::High), Ext::Never),
    (method::REVIEW_CHANGES, Access::Open, Ext::Never),
    // Pitwall's own windows (workspace.rs).
    (method::SPACE_LIST, Access::Open, Ext::Never),
    (method::SPACE_CREATE, Access::Open, Ext::Never),
    (method::SPACE_RENAME, Access::Open, Ext::Never),
    (method::SPACE_MOVE, Access::Open, Ext::Never),
    (method::AGENT_MOVE, Access::Open, Ext::Never),
    // Only shows files in Pitwall's viewer, read-only (viewer.rs).
    (method::VIEWER_OPEN, Access::Open, Ext::Grant(SHOW)),
    // The agents' MCP tools: the same, as tools (viewer.rs).
    (method::MCP_TOOLS, Access::Open, Ext::Never),
    (method::MCP_CALL, Access::Open, Ext::Never),
    // Only shows the user a web page (http/https) in Pitwall; nothing of it
    // comes back (viewer.rs).
    (method::WEB_OPEN, Access::Open, Ext::Grant(SHOW)),
    // Only shows the user Pitwall's Settings at a page; nothing of it comes
    // back and nothing in it changes (viewer.rs). Not an extension's: its
    // `show` permission is for files and pages.
    (method::SETTINGS_OPEN, Access::Open, Ext::Never),
    // Extensions' state and setup for agents (extension_setup.rs): reads
    // are open; every act asks the user (one at a time per extension), and
    // none is ever an extension's own.
    (method::EXTENSION_LIST, Access::Open, Ext::Never),
    (method::EXTENSION_STATUS, Access::Open, Ext::Never),
    (method::EXTENSION_LOG, Access::Open, Ext::Never),
    // Looks at a source as "Check for updates" does; installs nothing.
    (method::EXTENSION_CHECK_UPDATES, Access::Open, Ext::Never),
    // Only shows the user a secret's field in Settings; no value passes.
    (method::EXTENSION_SECRET, Access::Open, Ext::Never),
    (method::EXTENSION_ACT, Access::AskWhen("always: shows exactly what will run (commands, variables, folders), then does what Settings' button does", Risk::High), Ext::Never),
    // What agent.list shows, as it changes; `approvals` only for the UI
    // (events.rs). Answered per connection (server.rs).
    (method::EVENTS_SUBSCRIBE, Access::Open, Ext::Grant(AGENTS_READ)),
];

/// What a paired machine may call over the tailnet (tailnet-peers.md §6),
/// and the access level it needs. Only View exists in v1: the agent list
/// and its events (`agents`, `attention`, `blocked`; never `approvals`). A
/// method missing here is never served over the network. The handshake's
/// `peer.auth` is answered by the peer listener itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAccess {
    View,
}

pub const PEER_METHODS: &[(&str, PeerAccess)] = &[(method::AGENT_LIST, PeerAccess::View), (method::EVENTS_SUBSCRIBE, PeerAccess::View)];

pub fn peer_access(name: &str) -> Option<PeerAccess> {
    PEER_METHODS.iter().find(|(m, _)| *m == name).map(|(_, a)| *a)
}

pub fn access(name: &str) -> Option<Access> {
    METHODS.iter().find(|(m, _, _)| *m == name).map(|(_, a, _)| *a)
}

/// What an extension needs to call `name` (`None`: no such method).
pub fn ext_access(name: &str) -> Option<Ext> {
    METHODS.iter().find(|(m, _, _)| *m == name).map(|(_, _, e)| *e)
}

/// Whether extension `ext` (id, name) may call `name` now, granted
/// `allows`; why not, as the error it gets.
pub fn ext_allowed(name: &str, ext: &(String, String), allows: impl Fn(&str) -> bool) -> Result<(), ErrorBody> {
    let (_, ext_name) = ext;
    match ext_access(name) {
        None => Err(ErrorBody::new(code::UNKNOWN_METHOD, format!("unknown method \"{name}\""))),
        Some(Ext::Grant(p)) if allows(p) => Ok(()),
        Some(Ext::Grant(p)) => Err(ErrorBody::new(code::DENIED, format!("Not allowed: {ext_name} isn't granted {p} (Settings → Extensions → {ext_name} → Permissions)."))),
        Some(Ext::Later(p)) => Err(ErrorBody::new(code::UNSUPPORTED, format!("{name} isn't available to extensions yet (it will need {p})."))),
        Some(Ext::Never) => Err(ErrorBody::new(code::UNSUPPORTED, format!("{name} isn't available to extensions."))),
    }
}

/// What an extension with `profiles.read` gets of `accounts.list` and
/// `project.accounts`: only its own connections (`ext:<id>`) and the
/// profiles' names, never other slots, git identities or tool accounts.
fn own_slots(name: &str, ext_id: &str, mut v: Value) -> Value {
    let slot = pitwall_core::accounts::ext_kind(ext_id);
    let mine = |x: &Value| x["slot"].as_str() == Some(slot.as_str());
    let keep = |arr: &mut Value| {
        if let Some(a) = arr.as_array_mut() {
            a.retain(&mine);
        }
    };
    match name {
        method::ACCOUNTS_LIST => {
            keep(&mut v["accounts"]);
            v["git"] = Value::Array(vec![]);
            if let Some(ps) = v["profiles"].as_array_mut() {
                for p in ps {
                    keep(&mut p["picks"]);
                }
            }
        }
        method::PROJECT_ACCOUNTS => keep(&mut v["slots"]),
        _ => {}
    }
    v
}

type Res = Result<Value, ErrorBody>;

pub(crate) fn params<T: DeserializeOwned + Default>(v: Value) -> Result<T, ErrorBody> {
    if v.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(v).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))
}

fn required<T: DeserializeOwned>(v: Value) -> Result<T, ErrorBody> {
    serde_json::from_value(v).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))
}

fn ok(v: impl Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

fn other(e: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::OTHER, e)
}

/// A call from `caller`, counted and logged when it is an extension's
/// (`via`: how it came): its budget first, then [`dispatch`].
pub fn call(s: &Server, caller: &Caller, name: &str, p: Value, via: Via) -> Res {
    let Some((id, _)) = &caller.extension else { return dispatch(s, caller, name, p) };
    let detail = crate::actions::detail(s, name, &p);
    let res = s.extensions.admit(id).and_then(|_| dispatch(s, caller, name, p));
    s.extensions.record_call_with(id, via, name, detail, &res);
    res
}

pub fn dispatch(s: &Server, caller: &Caller, name: &str, p: Value) -> Res {
    match access(name) {
        None => return Err(ErrorBody::new(code::UNKNOWN_METHOD, format!("unknown method \"{name}\""))),
        Some(Access::Ui) if !caller.ui => {
            return Err(ErrorBody::new(code::DENIED, "only Pitwall's own window can do this"));
        }
        Some(Access::UiOrExtension) if !caller.ui && caller.extension.is_none() => {
            return Err(ErrorBody::new(code::DENIED, "only Pitwall's own window (or an extension you allowed) can do this"));
        }
        _ => {}
    }
    // An extension: only what it is granted, checked at every call (a
    // revoke applies at once), and of profiles only its own.
    if let Some(ext) = &caller.extension {
        ext_allowed(name, ext, |perm| s.extensions.allows(&ext.0, perm))?;
        let v = run(s, caller, name, p)?;
        return Ok(own_slots(name, &ext.0, v));
    }
    run(s, caller, name, p)
}

fn run(s: &Server, caller: &Caller, name: &str, p: Value) -> Res {
    match name {
        method::AGENT_LIST => ok(s.engine.views()),
        method::AGENT_REFRESH => {
            let r: AgentRef = required(p)?;
            changes::refresh(&s.engine, &r.agent_id).map_err(other)?;
            let view = s.engine.views().into_iter().find(|v| v.id == r.agent_id);
            ok(view.ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no agent {}", r.agent_id)))?)
        }
        method::AGENT_CREATE => agent_create(s, caller, required(p)?),
        method::MACHINE_LIST => ok(s.engine.machine_list()),
        method::MACHINE_FORM => {
            let r: FormRequest = required(p)?;
            ok(s.engine.create_form(Some(&r.provider), Some(&r.machine)).map_err(other)?)
        }
        method::SESSION_LIST => {
            let filter: SessionFilter = params(p)?;
            let owned = s.engine.records().iter().map(|r| r.locator()).collect();
            ok(filter.apply(places::places(s.engine.providers(), s.engine.kinds(), &owned)))
        }
        method::SESSION_ADD => session_add(s, caller, required(p)?),
        method::APPROVAL_LIST => ok(s.approvals.pending()),
        method::APPROVAL_ANSWER => {
            let a: ApprovalAnswer = required(p)?;
            s.approvals.answer(&a).map_err(|e| ErrorBody::new(code::NOT_FOUND, e))?;
            Ok(Value::Null)
        }
        method::SETTINGS_LIST => crate::settings::list(s),
        method::SETTINGS_GET => crate::settings::get(s, required(p)?),
        method::SETTINGS_SET => crate::settings::set(s, caller, required(p)?),
        method::SETTINGS_RESET => crate::settings::reset(s, caller, required(p)?),
        method::AGENT_STOP => manage::stop(s, caller, required(p)?),
        method::AGENT_RESTART => manage::restart(s, caller, required(p)?),
        method::AGENT_REMOVE => manage::remove(s, caller, required(p)?),
        method::AGENT_RENAME => manage::rename(s, required(p)?),
        method::AGENT_STATUS => manage::status(s, required(p)?),
        method::AGENT_TRACK => manage::track(s, required(p)?),
        method::AGENT_WAIT => manage::wait(s, required(p)?),
        method::QUEUE_ADD => match &caller.extension {
            Some(ext) => crate::actions::queue_add(s, ext, required(p)?),
            None => manage::queue_add(s, required(p)?),
        },
        method::AGENT_FOCUS => crate::viewer::focus(s, required(p)?),
        method::QUEUE_LIST => manage::queue_list(s, params(p)?),
        method::QUEUE_REMOVE => manage::queue_remove(s, required(p)?),
        method::QUEUE_SEND => manage::queue_send(s, required(p)?),
        method::PROJECT_LIST => manage::project_list(s),
        method::WORKTREE_LIST => ok(pitwall_core::worktrees::list(&s.engine)),
        method::PROJECT_ADD => manage::project_add(s, required(p)?),
        method::PROJECT_REMOVE => manage::project_remove(s, required(p)?),
        method::ACCOUNTS_LIST => crate::profiles::list(s),
        method::PROFILE_SAVE => crate::profiles::save_profile(s, required(p)?),
        method::PROFILE_SET => crate::profiles::set(s, caller, required(p)?),
        method::PROFILE_DELETE => crate::profiles::delete(s, caller, required(p)?),
        method::PROJECT_ACCOUNTS => crate::profiles::project(s, required(p)?),
        method::PROJECT_PICK => crate::profiles::pick(s, caller, required(p)?),
        method::PROJECT_PROFILE => crate::profiles::assign(s, caller, required(p)?),
        method::RULES_LIST => manage::rules_list(s),
        method::RULES_SETS => manage::rules_sets(s),
        method::RULES_APPLY => manage::rules_apply(s, caller, required(p)?),
        method::RULES_DEFAULT => manage::rules_default(s, required(p)?),
        method::RULES_SHOW => manage::rules_show(s, required(p)?),
        method::RULES_LIBRARY_NEW => manage::library_new(s, caller, required(p)?),
        method::RULES_PROJECT_STATUS => manage::project_status(s, required(p)?),
        method::RULES_PROJECT_UPDATES => manage::project_updates(s, required(p)?),
        method::RULES_PROJECT_ASSIGN => manage::project_assign(s, caller, required(p)?),
        method::RULES_PROJECT_UNASSIGN => manage::project_unassign(s, caller, required(p)?),
        method::RULES_PROJECT_UPDATE => manage::project_update(s, caller, required(p)?),
        method::RULES_PROJECT_IMPORT => manage::project_import(s, caller, required(p)?),
        method::RULES_PROJECT_GENERATE => manage::project_generate(s, caller, required(p)?),
        method::RULES_PROJECT_MOVE => manage::project_move(s, caller, required(p)?),
        method::RULES_REAPPLY => manage::rules_reapply(s, params(p)?),
        method::RULES_SET_SAVE => manage::rules_set_save(s, required(p)?),
        method::RULES_SET_DELETE => manage::rules_set_delete(s, caller, required(p)?),
        method::RULES_IMPORT => manage::rules_import(s, caller, required(p)?),
        method::REVIEW_CHANGES => manage::review_changes(s, required(p)?),
        method::SPACE_LIST => workspace::list(s),
        method::SPACE_CREATE => workspace::create(s, required(p)?),
        method::SPACE_RENAME => workspace::rename(s, required(p)?),
        method::SPACE_MOVE => workspace::move_to_window(s, required(p)?),
        method::AGENT_MOVE => workspace::move_agent(s, required(p)?),
        method::VIEWER_OPEN => {
            let r = crate::viewer::params(p)?;
            beside_only(caller, r.place)?;
            crate::viewer::open(s, caller, r)
        }
        method::MCP_TOOLS | method::MCP_CALL if s.viewer.is_none() => Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall has no windows to show files in")),
        method::MCP_TOOLS => crate::viewer::mcp_tools(s),
        method::WEB_OPEN => {
            let r = crate::viewer::web_params(p)?;
            beside_only(caller, r.place)?;
            crate::viewer::web_open(s, caller, r)
        }
        method::SETTINGS_OPEN => {
            let r = crate::viewer::settings_params(p)?;
            crate::viewer::settings_open(s, caller, r)
        }
        method::MCP_CALL => crate::viewer::mcp_call(s, caller, required(p)?),
        method::EXTENSION_LIST => crate::extension_setup::list(s),
        method::EXTENSION_STATUS => crate::extension_setup::status(s, required(p)?),
        method::EXTENSION_LOG => crate::extension_setup::log(s, required(p)?),
        method::EXTENSION_CHECK_UPDATES => crate::extension_setup::check_updates(s, required(p)?),
        method::EXTENSION_SECRET => crate::extension_setup::secret(s, caller, required(p)?),
        method::EXTENSION_ACT => crate::extension_setup::act(s, caller, required(p)?),
        // Needs the connection: server.rs answers it before dispatching.
        method::EVENTS_SUBSCRIBE => Err(ErrorBody::new(code::UNSUPPORTED, "events.subscribe needs a connection of its own")),
        _ => Err(ErrorBody::new(code::UNKNOWN_METHOD, format!("unknown method \"{name}\""))),
    }
}

/// An extension shows things next to an agent's terminal: never in a
/// window of its own nor in the terminal's place.
fn beside_only(caller: &Caller, place: Option<pitwall_proto::ViewerPlace>) -> Result<(), ErrorBody> {
    use pitwall_proto::ViewerPlace;
    match (&caller.extension, place) {
        (Some(_), Some(ViewerPlace::Window | ViewerPlace::Replace)) => Err(ErrorBody::new(code::BAD_PARAMS, "An extension shows things next to an agent's terminal (right, left, above or below): not in a window of its own or in the terminal's place.")),
        _ => Ok(()),
    }
}

fn agent_create(s: &Server, caller: &Caller, mut req: AgentCreate) -> Res {
    if caller.extension.is_some() {
        return crate::actions::start(s, caller, req);
    }
    // Its first instruction: checked before anything is made.
    let prompt = req.prompt.take();
    if prompt.as_deref().is_some_and(|p| p.trim().is_empty()) {
        return Err(ErrorBody::new(code::BAD_PARAMS, "the first instruction (prompt) is empty: nothing was created"));
    }
    if req.provider.is_some() || req.machine.is_some() {
        let form = s.engine.create_form(req.provider.as_deref(), req.machine.as_deref()).map_err(other)?;
        if !form.folder {
            return create_on_platform(s, caller, req, form, prompt);
        }
    }
    if req.kind.trim().is_empty() || req.project.trim().is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "kind and project are required"));
    }
    // An account by name or id (`--account work`).
    let account = match req.account.as_deref() {
        None => None,
        Some(key) => Some(
            pitwall_core::accounts::Accounts::load(s.engine.paths())
                .find(&req.kind, key)
                .ok_or_else(|| ErrorBody::new(code::BAD_PARAMS, format!("no {} account called {key}", req.kind)))?,
        ),
    };
    let view = lifecycle::create(
        &s.engine,
        CreateAgentRequest {
            name: req.name.unwrap_or_default(),
            kind: req.kind,
            project_path: req.project,
            provider: req.provider,
            machine: req.machine,
            cols: req.cols,
            rows: req.rows,
            resume_session_id: req.resume,
            account,
            ..Default::default()
        },
    )
    .map_err(other)?;
    onboarding::add_agent_project(&s.engine, &view);
    ok(first_instruction(view, prompt, |id, text| s.engine.queue_add(id, text)))
}

/// The new agent, with its first instruction (`prompt`) put in its Next up
/// by `queue` as `queue.add` does: the user's item, sent by Auto-send's
/// usual rules. When that fails the agent stays and the answer says why.
pub(crate) fn first_instruction(view: AgentView, prompt: Option<String>, queue: impl FnOnce(&str, String) -> Result<AgentView, String>) -> AgentCreated {
    let Some(text) = prompt else { return AgentCreated { agent: view, queued: None, queue_error: None } };
    match queue(&view.id, text) {
        Ok(after) => {
            // queue.add puts it last.
            let queued = after.queue.last().map(|q| Queued { id: q.id.clone(), position: after.queue.len() });
            AgentCreated { agent: after, queued, queue_error: None }
        }
        Err(e) => AgentCreated { agent: view, queued: None, queue_error: Some(e) },
    }
}

/// A new session on a platform's machine (agw): it changes that machine,
/// so the user approves what the form's summary says first — every time
/// when it also makes a workspace or an agent user there.
fn create_on_platform(s: &Server, caller: &Caller, req: AgentCreate, form: CreateForm, prompt: Option<String>) -> Res {
    let name = req.name.as_deref().map(str::trim).unwrap_or_default().to_string();
    if name.is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, format!("a name is required: the session's name on {}", form.machine_label)));
    }
    let values = form.values(&req.options).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e))?;
    let summary = form.summarize(&name, &values);
    let place = format!("{} ({})", form.machine_label, form.provider);
    let mut details = vec![summary.text.clone()];
    details.extend(summary.creates.iter().map(|c| format!("Also creates {c} on {}.", form.machine_label)));
    details.push("It keeps running there after Pitwall quits; removing it from Pitwall leaves it there.".into());
    if let Some(p) = &prompt {
        details.push(format!("Its first instruction, put in its Next up:\n{p}"));
    }
    let risk = if summary.creates.is_empty() { Risk::Low } else { Risk::High };
    let decision = s.approvals.ask(Ask {
        action: method::AGENT_CREATE.into(),
        summary: format!("create the session \"{name}\" on {place}"),
        details,
        requester: caller.requester(),
        caller_key: caller.key(),
        risk,
    });
    match decision {
        Decision::Allowed | Decision::Remembered => {}
        Decision::Denied => return Err(ErrorBody::new(code::DENIED, format!("The user didn't allow creating \"{name}\" on {place}. Nothing was created."))),
        Decision::TimedOut => {
            return Err(ErrorBody::new(
                code::APPROVAL_TIMEOUT,
                format!("Nobody answered the approval to create \"{name}\" on {place} in time. Nothing was created."),
            ))
        }
    }
    let view = lifecycle::create(
        &s.engine,
        CreateAgentRequest {
            name,
            kind: req.kind,
            provider: Some(form.provider),
            machine: Some(form.machine),
            options: req.options,
            cols: req.cols,
            rows: req.rows,
            ..Default::default()
        },
    )
    .map_err(other)?;
    ok(first_instruction(view, prompt, |id, text| s.engine.queue_add(id, text)))
}

/// Adopt a session (only tracks it: nothing changes there), then — when
/// asked and it is stopped — start it there after the user approves.
fn session_add(s: &Server, caller: &Caller, req: SessionAdd) -> Res {
    let known: Vec<String> = s.engine.views().into_iter().map(|v| v.id).collect();
    let view = lifecycle::adopt(
        &s.engine,
        AdoptSessionRequest {
            provider: req.provider.clone(),
            machine: req.machine.clone(),
            native: req.native.clone(),
            cols: req.cols,
            rows: req.rows,
        },
    )
    .map_err(other)?;
    let already = known.contains(&view.id);
    if !req.start || view.running {
        return ok(SessionAdded { agent: view, already, started: false });
    }
    if !view.caps.restart {
        return Err(ErrorBody::new(code::UNSUPPORTED, format!("\"{}\" was added, but Pitwall can't start it", req.native)));
    }
    let loc = Locator::new(&ProviderId::new(&req.provider), &MachineId::new(&req.machine), &req.native);
    let place = format!("{} ({})", s.engine.providers().machine_label(&loc), view.machine.provider);
    let decision = s.approvals.ask(Ask {
        action: method::SESSION_ADD.into(),
        summary: format!("start the session \"{}\" on {place}", req.native),
        details: vec![
            "It is stopped there. Starting it runs it on that machine, where it keeps running after Pitwall quits.".into(),
            "It has already been added to Pitwall (that needs no approval).".into(),
        ],
        requester: caller.requester(),
        caller_key: caller.key(),
        risk: Risk::Low,
    });
    match decision {
        Decision::Allowed | Decision::Remembered => {
            let size = pitwall_core::model::size_of(req.cols, req.rows);
            let agent = lifecycle::restart(&s.engine, &view.id, size).map_err(other)?;
            ok(SessionAdded { agent, already, started: true })
        }
        Decision::Denied => Err(ErrorBody::new(
            code::DENIED,
            format!("The user didn't allow starting \"{}\". It was added to Pitwall and is still stopped.", req.native),
        )),
        Decision::TimedOut => Err(ErrorBody::new(
            code::APPROVAL_TIMEOUT,
            format!("Nobody answered the approval to start \"{}\" in time. It was added to Pitwall and is still stopped.", req.native),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What an extension gets from a method it may call.
    fn what(name: &str) -> &'static str {
        match name {
            method::AGENT_LIST => "Every agent Pitwall shows, with its state, folder, machine and branch.",
            method::AGENT_STATUS => "What one agent is doing and why (`agentId`: its id).",
            method::PROJECT_LIST => "The user's projects: name and folder.",
            method::WORKTREE_LIST => "Each repository with its worktrees: folder, branch, and the agents working there.",
            method::ACCOUNTS_LIST => "The profiles, only the extension's own connections (`ext:<id>`); git identities are left out.",
            method::PROJECT_ACCOUNTS => "Which of the extension's own connections a project uses.",
            method::EVENTS_SUBSCRIBE => "Events as they happen (`topics`: `agents`, `attention`, `blocked`), each as an `event` line. Calling it again replaces the topics.",
            method::VIEWER_OPEN => "Shows files (`files`, `line`) or a folder in an agent's file viewer (`agentId` required), next to its terminal; never a window of its own, never brings Pitwall forward. Off while the user turned off \"Agents can show you files\".",
            method::WEB_OPEN => "Shows a web page (`url`, http or https) in an agent's page tile (`agentId` required), next to its terminal; never brings Pitwall forward. Off while the user turned off agents' web pages.",
            method::AGENT_FOCUS => "Moves the focus to an agent's tile (`agentId`) while Pitwall is in front; otherwise does nothing and says so (`focused: false`, `note`).",
            method::QUEUE_ADD => "Puts `text` in an agent's Next up (`agentId`), marked with the extension's name. It waits for the user's Send unless the user lets the extension send without asking. At most 4000 characters, one item per agent every 10 seconds, three of its items waiting per agent: see [actions](actions.md).",
            method::AGENT_CREATE => "Asks the user to start an agent (`kind`, `project`: one of Pitwall's projects; maybe `name` and a first `prompt`, put in its Next up). Asked every time, showing all of it; no answer in time is a no. One request at a time, three in ten minutes: see [actions](actions.md).",
            _ => "",
        }
    }

    /// `docs/extensions/methods.md`, from [`METHODS`].
    fn methods_markdown() -> String {
        let mut md = String::from(
            "# Methods an extension may call\n\n\
             <!-- Generated from crates/pitwall-daemon/src/methods.rs (METHODS): run\n\
             `PITWALL_WRITE_DOCS=1 cargo test -p pitwall-daemon methods_doc`. -->\n\n\
             An adapter speaking [protocol 2](protocol-2.md) calls these with a `call`\n\
             line; a program it started can call them on Pitwall's socket\n\
             (`pitwall` or `pitwall-client`) and is the extension there too. Each needs\n\
             a [permission](permissions.md) the user granted. Params and results are\n\
             those of the command-line API (`pitwall --json`).\n\n\
             | Method | Needs | What it gives |\n|---|---|---|\n",
        );
        for (m, _, e) in METHODS {
            if let Ext::Grant(p) = e {
                md.push_str(&format!("| `{m}` | `{p}` | {} |\n", what(m)));
            }
        }
        let later: Vec<String> = METHODS.iter().filter_map(|(m, _, e)| if let Ext::Later(p) = e { Some(format!("| `{m}` | `{p}` |\n")) } else { None }).collect();
        if !later.is_empty() {
            md.push_str("\n## Not yet\n\nThese are refused with `unsupported` in this Pitwall. A later one will\noffer them with the permission named.\n\n| Method | Will need |\n|---|---|\n");
            md.push_str(&later.concat());
        }
        let never: Vec<String> = METHODS.iter().filter(|(_, _, e)| *e == Ext::Never).map(|(m, _, _)| format!("`{m}`")).collect();
        md.push_str(&format!(
            "\n## Never\n\nEverything else is the user's own, refused with `unsupported` whatever is\ngranted: {}.\n\n\
             ## Errors\n\n\
             - `denied`: the permission isn't granted (or was revoked: it applies to the\n  next call). The message names it and where the user grants it.\n\
             - `unsupported`: not an extension's method, or not in this Pitwall yet.\n\
             - `busy`: past {} calls a second, socket and adapter together, or past\n  an action's own limit ([actions](actions.md)). Wait and call again.\n\
             - `approval_timeout`: nobody answered a request to start an agent in\n  time; nothing was started.\n\
             - `unknown_method`: no such method.\n",
            never.join(", "),
            crate::extensions::MAX_CALLS
        ));
        md
    }

    #[test]
    fn methods_doc_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/extensions/methods.md");
        let md = methods_markdown();
        if std::env::var_os("PITWALL_WRITE_DOCS").is_some() {
            std::fs::write(&path, &md).unwrap();
            return;
        }
        assert!(std::fs::read_to_string(&path).unwrap_or_default() == md, "docs/extensions/methods.md is out of date with METHODS: run `PITWALL_WRITE_DOCS=1 cargo test -p pitwall-daemon methods_doc` and commit it");
    }

    #[test]
    fn every_method_has_one_row() {
        for (i, (m, _, _)) in METHODS.iter().enumerate() {
            assert!(METHODS[i + 1..].iter().all(|(n, _, _)| n != m), "{m} twice");
        }
        assert_eq!(access(method::AGENT_LIST), Some(Access::Open));
        assert_eq!(access(method::MACHINE_FORM), Some(Access::Open));
        assert_eq!(access(method::AGENT_REFRESH), Some(Access::Open));
        assert!(matches!(access(method::AGENT_CREATE), Some(Access::AskWhen(_, Risk::Low))));
        assert!(matches!(access(method::SESSION_ADD), Some(Access::AskWhen(_, Risk::Low))));
        assert_eq!(access(method::APPROVAL_ANSWER), Some(Access::Ui));
        assert_eq!(access(method::SETTINGS_LIST), Some(Access::Open));
        assert!(matches!(access(method::SETTINGS_SET), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::SETTINGS_RESET), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::AGENT_REMOVE), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::AGENT_STOP), Some(Access::AskWhen(..))));
        assert!(matches!(access(method::AGENT_RESTART), Some(Access::AskWhen(..))));
        assert!(matches!(access(method::RULES_APPLY), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::RULES_SET_DELETE), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::RULES_IMPORT), Some(Access::AskWhen(_, Risk::High))));
        assert_eq!(access(method::RULES_SET_SAVE), Some(Access::Open));
        for m in [method::RULES_PROJECT_ASSIGN, method::RULES_PROJECT_UNASSIGN, method::RULES_PROJECT_UPDATE, method::RULES_PROJECT_IMPORT, method::RULES_PROJECT_GENERATE, method::RULES_PROJECT_MOVE] {
            assert!(matches!(access(m), Some(Access::AskWhen(_, Risk::Low))), "{m}");
        }
        assert_eq!(access(method::RULES_PROJECT_STATUS), Some(Access::Open));
        assert!(matches!(access(method::RULES_LIBRARY_NEW), Some(Access::AskWhen(_, Risk::Low))));
        assert_eq!(access(method::RULES_PROJECT_UPDATES), Some(Access::Open));
        // Read-only methods never ask.
        for m in [method::AGENT_STATUS, method::AGENT_WAIT, method::QUEUE_LIST, method::PROJECT_LIST, method::RULES_LIST, method::RULES_SETS, method::RULES_SHOW, method::REVIEW_CHANGES, method::SPACE_LIST, method::VIEWER_OPEN, method::MCP_TOOLS, method::MCP_CALL, method::WEB_OPEN, method::SETTINGS_OPEN] {
            assert_eq!(access(m), Some(Access::Open), "{m}");
        }
        assert_eq!(access(method::EVENTS_SUBSCRIBE), Some(Access::Open));
        assert_eq!(access(method::AGENT_FOCUS), Some(Access::UiOrExtension));
        assert_eq!(access("agent.prompt"), None);
    }

    /// The permission column: extensions read with a grant, and act only
    /// through the four action permissions (each a method of its own);
    /// sending, approving, settings and the rest are never theirs.
    #[test]
    fn extensions_read_with_a_grant_and_act_only_with_one() {
        assert_eq!(ext_access(method::AGENT_LIST), Some(Ext::Grant(AGENTS_READ)));
        assert_eq!(ext_access(method::WORKTREE_LIST), Some(Ext::Grant(WORKTREES_READ)));
        assert_eq!(ext_access(method::QUEUE_ADD), Some(Ext::Grant(AGENTS_QUEUE)));
        assert_eq!(ext_access(method::AGENT_CREATE), Some(Ext::Grant(AGENTS_START)));
        assert_eq!(ext_access(method::AGENT_FOCUS), Some(Ext::Grant(AGENTS_FOCUS)));
        assert_eq!(ext_access(method::VIEWER_OPEN), Some(Ext::Grant(SHOW)));
        assert_eq!(ext_access(method::WEB_OPEN), Some(Ext::Grant(SHOW)));
        let acting = [method::QUEUE_ADD, method::AGENT_CREATE, method::AGENT_FOCUS, method::VIEWER_OPEN, method::WEB_OPEN];
        // Core's list (the author's checks use it) says the same.
        for (m, p) in pitwall_core::extensions::permissions::ACTION_METHODS {
            assert!(acting.contains(&m), "{m}");
            assert_eq!(ext_access(m), Some(Ext::Grant(p)), "{m}");
        }
        assert_eq!(pitwall_core::extensions::permissions::ACTION_METHODS.len(), acting.len());
        for (m, a, e) in METHODS {
            if let Ext::Grant(p) = e {
                assert!(pitwall_core::extensions::permissions::available(p), "{m}: {p}");
                // Reads are open to anyone; the actions are these five.
                assert!(*a == Access::Open || acting.contains(m), "{m}");
            }
            assert!(!matches!(e, Ext::Later(_)), "{m}: nothing is left for later");
        }
        // Showing Settings isn't the `show` permission's (files and pages).
        assert_eq!(ext_access(method::SETTINGS_OPEN), Some(Ext::Never));
        assert_eq!(ext_allowed(method::SETTINGS_OPEN, &("ci".to_string(), "CI".to_string()), |_| true).unwrap_err().code, code::UNSUPPORTED);
        for m in [method::AGENT_STOP, method::SETTINGS_SET, method::SETTINGS_RESET, method::APPROVAL_ANSWER, method::APPROVAL_LIST, method::RULES_APPLY, method::MCP_CALL, method::PROJECT_ADD, method::QUEUE_SEND, method::QUEUE_REMOVE, method::SESSION_ADD, method::AGENT_RESTART, method::AGENT_REMOVE] {
            assert_eq!(ext_access(m), Some(Ext::Never), "{m}");
        }
        let ext = ("ci".to_string(), "CI".to_string());
        let granted = |p: &str| p == AGENTS_READ;
        assert!(ext_allowed(method::AGENT_LIST, &ext, granted).is_ok());
        let no = ext_allowed(method::PROJECT_LIST, &ext, granted).unwrap_err();
        assert_eq!(no.code, code::DENIED);
        assert!(no.message.contains("isn't granted projects.read"), "{}", no.message);
        for m in acting {
            assert_eq!(ext_allowed(m, &ext, granted).unwrap_err().code, code::DENIED, "{m} without its grant");
        }
        assert!(ext_allowed(method::QUEUE_ADD, &ext, |p| p == AGENTS_QUEUE).is_ok());
        assert_eq!(ext_allowed(method::QUEUE_SEND, &ext, |_| true).unwrap_err().code, code::UNSUPPORTED, "sending stays the user's");
        assert_eq!(ext_allowed(method::AGENT_STOP, &ext, |_| true).unwrap_err().code, code::UNSUPPORTED);
        // Setting up extensions is the user's (and their agents'), never
        // an extension's: not even reading another's state.
        for m in [method::EXTENSION_LIST, method::EXTENSION_STATUS, method::EXTENSION_LOG, method::EXTENSION_CHECK_UPDATES, method::EXTENSION_SECRET, method::EXTENSION_ACT] {
            assert_eq!(ext_access(m), Some(Ext::Never), "{m}");
            assert_eq!(ext_allowed(m, &ext, |_| true).unwrap_err().code, code::UNSUPPORTED, "{m}");
        }
        assert!(matches!(access(method::EXTENSION_ACT), Some(Access::AskWhen(_, Risk::High))));
        for m in [method::EXTENSION_LIST, method::EXTENSION_STATUS, method::EXTENSION_LOG, method::EXTENSION_SECRET] {
            assert_eq!(access(m), Some(Access::Open), "{m}: read-only");
        }
        assert_eq!(ext_allowed("agent.prompt", &ext, |_| true).unwrap_err().code, code::UNKNOWN_METHOD);
    }

    /// `profiles.read`: an extension sees its own connections and the
    /// profiles' names, never other slots or git identities.
    #[test]
    fn an_extension_sees_only_its_own_slots() {
        let v = serde_json::json!({
            "accounts": [{"id": "c1", "slot": "ext:ci"}, {"id": "k1", "slot": "claude"}, {"id": "o1", "slot": "ext:other"}],
            "git": [{"id": "g1", "email": "me@example.com"}],
            "profiles": [{"id": "p1", "name": "Work", "picks": [{"slot": "ext:ci"}, {"slot": "git"}]}]
        });
        let v = own_slots(method::ACCOUNTS_LIST, "ci", v);
        assert_eq!(v["accounts"], serde_json::json!([{"id": "c1", "slot": "ext:ci"}]));
        assert_eq!(v["git"], serde_json::json!([]));
        assert_eq!(v["profiles"][0]["picks"], serde_json::json!([{"slot": "ext:ci"}]));
        assert_eq!(v["profiles"][0]["name"], "Work");
        let p = own_slots(method::PROJECT_ACCOUNTS, "ci", serde_json::json!({"project": "/p", "slots": [{"slot": "git"}, {"slot": "ext:ci"}]}));
        assert_eq!(p["slots"], serde_json::json!([{"slot": "ext:ci"}]));
    }

    fn new_view() -> AgentView {
        let mut v = serde_json::json!({
            "id": "a1", "name": "api", "kind": "shell", "kindName": "Shell", "terminal": true, "sessionId": null,
            "cwd": "/p", "cwdDisplay": "/p", "project": "/p", "projectDisplay": "/p",
            "branch": null, "worktree": false, "worktreePending": false, "location": "local",
            "agentInTerminal": false, "restartAs": null, "status": "unknown",
            "statusSource": "screen", "statusDetail": null, "running": true, "cols": 120, "rows": 40,
            "added": 0, "removed": 0, "filesChanged": 0, "queue": [], "autoSend": true, "lastSent": null,
            "lastSentAt": null, "createdAt": 1, "currentTaskId": null
        });
        v["machine"] = serde_json::json!({"provider": "local", "id": "this-mac", "label": "This Mac", "canCreate": true});
        v["caps"] = serde_json::json!({"input": true, "restart": true, "resume": false, "stop": true, "removeWorktree": false,
            "diff": false, "review": false, "merge": false, "rules": false, "hooks": false, "removeKeepsSession": true});
        serde_json::from_value(v).unwrap()
    }

    /// The first instruction is queued as `queue.add` does; when that
    /// fails the agent is still answered, with why.
    #[test]
    fn a_first_instruction_is_queued_or_its_failure_said() {
        let none = first_instruction(new_view(), None, |_, _| panic!("nothing to queue"));
        assert_eq!((none.queued, none.queue_error), (None, None));
        let made = first_instruction(new_view(), Some("run the tests".into()), |id, text| {
            let mut after = new_view();
            assert_eq!(id, "a1");
            after.queue.push(pitwall_proto::QueueItem { id: "q1".into(), text, origin: None });
            Ok(after)
        });
        assert_eq!(made.queued, Some(Queued { id: "q1".into(), position: 1 }));
        assert_eq!(made.agent.queue[0].text, "run the tests");
        let failed = first_instruction(new_view(), Some("run the tests".into()), |_, _| Err("no agent \"a1\"".into()));
        assert_eq!((failed.agent.id.as_str(), failed.queued, failed.queue_error.as_deref()), ("a1", None, Some("no agent \"a1\"")));
    }

    #[test]
    fn peers_get_only_reads_of_the_agent_list() {
        for (m, _) in PEER_METHODS {
            assert_eq!(access(m), Some(Access::Open), "{m}: a peer method is an open local one");
        }
        assert_eq!(peer_access(method::AGENT_LIST), Some(PeerAccess::View));
        assert_eq!(peer_access(method::EVENTS_SUBSCRIBE), Some(PeerAccess::View));
        for m in [method::AGENT_CREATE, method::AGENT_STOP, method::APPROVAL_ANSWER, method::SETTINGS_SET, method::QUEUE_ADD, method::MACHINE_LIST] {
            assert_eq!(peer_access(m), None, "{m}");
        }
    }
}
