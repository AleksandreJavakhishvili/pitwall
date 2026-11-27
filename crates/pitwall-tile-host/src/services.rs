//! The host side of the minimum's services (platform-services.md §14.2):
//! `net`, `sql`, `notify`, `show`, `toolbar` and `service`. Each runs on
//! the tile's worker, inside the guest's call, and checks its grant
//! first: an interface whose permission isn't granted answers `denied`
//! (permissions.md §5.2), with a line in the tile's Log once per call
//! site's kind. What the app must do is queued as a [`UiRequest`].

use std::path::Path;
use std::time::Instant;

use crate::bindings::pitwall::tile::{net, notify, service, show, sql, toolbar};
use crate::host::{cut, HostState, Pending};
use crate::limits::{MAX_FETCHES, MAX_NOTIFY_BODY, MAX_NOTIFY_TITLE, MAX_TOOLBAR_TOOLTIP, NOTIFY_PER_MINUTE, NOTIFY_SAME_WITHIN, TOOLBAR_SHOWN_EVERY};
use crate::net::{FetchError, Fetched};
use crate::ui::{HostError, ServiceInfo, ServiceState};
use crate::worker::Msg;
use crate::{LogLevel, ToolbarRequest, UiRequest, NET_PERMISSION, NOTIFY_PERMISSION, SHOW_PERMISSION};

/// The state `service.info` gives before the app has said: none declared,
/// or not known yet (off).
pub(crate) fn no_service(declared: bool) -> ServiceInfo {
    ServiceInfo { state: if declared { ServiceState::Off } else { ServiceState::None }, address: None, version: None, why: None }
}

impl HostState {
    fn ui(&mut self, r: UiRequest) {
        self.pending.push(Pending::Ui(r));
    }

    /// Refused for want of `permission`, said in the Log.
    fn denied(&mut self, what: &str, permission: &str) {
        if let Some(kind) = self.gated {
            self.host_log(LogLevel::Info, format!("{what} was refused: an agent's {kind} call runs without the tile's acting permissions (agent-tools.md §11.2); only an act call has them"));
            return;
        }
        self.host_log(LogLevel::Info, format!("{what} was refused: it isn't granted “{permission}”"));
    }
}

impl net::Host for HostState {
    fn fetch(&mut self, request: net::Request) -> Result<u32, FetchError> {
        if !self.count_call() {
            return Err(FetchError::TooLarge);
        }
        let reach = self.grants.reach();
        if reach.hosts.is_empty() && reach.own_server.is_none() {
            self.denied("a request", NET_PERMISSION);
            return Err(FetchError::Denied("not granted “net”".into()));
        }
        if self.fetches.len() >= MAX_FETCHES {
            return Err(FetchError::TooLarge);
        }
        let out = match crate::net::check(request, &reach) {
            Ok(o) => o,
            Err(e) => {
                if let FetchError::Denied(host) = &e {
                    self.host_log(LogLevel::Info, format!("a request to {host} was refused: it isn't one of the hosts it may reach"));
                }
                return Err(e);
            }
        };
        let id = self.next_fetch;
        self.next_fetch = self.next_fetch.wrapping_add(1).max(1);
        self.fetches.insert(id);
        let done = self.done.clone();
        self.fetcher.fetch(
            out,
            reach,
            Box::new(move |outcome| {
                let _ = done.send(Msg::Fetched(Fetched { id, outcome }));
            }),
        );
        Ok(id)
    }

    fn cancel(&mut self, id: u32) {
        self.fetches.remove(&id);
    }
}

impl sql::Host for HostState {
    fn execute(&mut self, statement: String, params: Vec<sql::Value>) -> Result<u64, HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        let (deadline, stop) = (self.deadline, self.stop.clone());
        self.db.execute(&statement, &params, deadline, stop)
    }

    fn query(&mut self, statement: String, params: Vec<sql::Value>) -> Result<sql::Rows, HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        let (deadline, stop) = (self.deadline, self.stop.clone());
        self.db.query(&statement, &params, deadline, stop)
    }

    fn migrate(&mut self, steps: Vec<String>) -> Result<u32, HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        let (deadline, stop) = (self.deadline, self.stop.clone());
        self.db.migrate(&steps, deadline, stop)
    }

    fn begin(&mut self) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        self.db.begin()
    }

    fn commit(&mut self) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        self.db.commit()
    }

    fn rollback(&mut self) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        self.db.rollback()
    }

    fn usage(&mut self) -> u64 {
        self.db.usage()
    }

    fn quota(&mut self) -> u64 {
        self.db.quota()
    }
}

impl notify::Host for HostState {
    fn send(&mut self, title: String, body: String) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if !self.grants.has(NOTIFY_PERMISSION) {
            self.denied("a notification", NOTIFY_PERMISSION);
            return Err(HostError::Denied);
        }
        let (title, body) = (cut(title.trim(), MAX_NOTIFY_TITLE), cut(body.trim(), MAX_NOTIFY_BODY));
        if title.is_empty() {
            return Err(HostError::Invalid("a notification needs a title".into()));
        }
        let now = self.clock.now();
        let said = format!("{title}\n{body}");
        self.notified.retain(|(at, _)| now.duration_since(*at) < NOTIFY_SAME_WITHIN);
        if self.notified.iter().any(|(_, s)| *s == said) {
            return Ok(());
        }
        let in_minute = self.notified.iter().filter(|(at, _)| now.duration_since(*at).as_secs() < 60).count();
        if in_minute >= NOTIFY_PER_MINUTE {
            self.host_log(LogLevel::Warn, format!("a notification was dropped: at most {NOTIFY_PER_MINUTE} a minute (“{}”)", cut(&title, 80)));
            return Err(HostError::Busy);
        }
        self.notified.push_back((now, said));
        self.ui(UiRequest::Notify { title, body });
        Ok(())
    }
}

impl show::Host for HostState {
    fn file(&mut self, path: String, line: Option<u32>) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if !self.grants.has(SHOW_PERMISSION) {
            self.denied("opening a file", SHOW_PERMISSION);
            return Err(HostError::Denied);
        }
        if path.len() > 4096 || path.contains('\0') || !Path::new(&path).is_absolute() {
            return Err(HostError::Invalid(format!("“{}” must be a whole path, from the root", cut(&path, 200))));
        }
        self.ui(UiRequest::OpenFile { path, line: line.filter(|l| *l > 0) });
        Ok(())
    }

    fn web(&mut self, url: String) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if !self.grants.has(SHOW_PERMISSION) {
            self.denied("opening a page", SHOW_PERMISSION);
            return Err(HostError::Denied);
        }
        let bad = || HostError::Invalid(format!("“{}” must be an https:// address (http:// only to this computer)", cut(&url, 200)));
        let parsed = url::Url::parse(&url).map_err(|_| bad())?;
        let local = parsed.host_str().is_some_and(|h| pitwall_core::extensions::net::is_loopback(h.trim_start_matches('[').trim_end_matches(']')));
        let ok = match parsed.scheme() {
            "https" => parsed.host_str().is_some(),
            "http" => local,
            _ => false,
        };
        if !ok || !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(bad());
        }
        self.ui(UiRequest::OpenWeb { url: parsed.to_string() });
        Ok(())
    }
}

impl HostState {
    /// Whether its manifest has a toolbar item (else the call is noted
    /// and ignored).
    fn has_toolbar(&mut self, what: &str) -> bool {
        if self.toolbar.is_some() {
            return true;
        }
        self.host_log(LogLevel::Warn, format!("toolbar.{what}: the manifest declares no toolbar item (\"contributes.toolbar\")"));
        false
    }
}

impl toolbar::Host for HostState {
    fn set_badge(&mut self, badge: toolbar::Badge, tone: toolbar::BadgeTone) {
        if self.count_call() && self.has_toolbar("set-badge") {
            self.ui(UiRequest::Toolbar(ToolbarRequest::Badge { project: None, badge, tone }));
        }
    }

    fn set_project_badge(&mut self, project: String, badge: toolbar::Badge, tone: toolbar::BadgeTone) {
        if !self.count_call() || !self.has_toolbar("set-project-badge") {
            return;
        }
        if !self.grants.has(pitwall_core::extensions::permissions::PROJECTS_READ) {
            self.denied("a project's badge", pitwall_core::extensions::permissions::PROJECTS_READ);
            return;
        }
        self.ui(UiRequest::Toolbar(ToolbarRequest::Badge { project: Some(cut(&project, 200)), badge, tone }));
    }

    fn set_tooltip(&mut self, text: String) {
        if self.count_call() && self.has_toolbar("set-tooltip") {
            let t = cut(text.trim(), MAX_TOOLBAR_TOOLTIP);
            if !t.is_empty() {
                self.ui(UiRequest::Toolbar(ToolbarRequest::Tooltip(t)));
            }
        }
    }

    fn set_shown(&mut self, shown: bool) {
        if !self.count_call() || !self.has_toolbar("set-shown") {
            return;
        }
        let now: Instant = self.clock.now();
        if self.shown_at.is_some_and(|at| now.duration_since(at) < TOOLBAR_SHOWN_EVERY) {
            self.host_log(LogLevel::Warn, format!("toolbar.set-shown was ignored: at most one change in {} s", TOOLBAR_SHOWN_EVERY.as_secs()));
            return;
        }
        self.shown_at = Some(now);
        self.ui(UiRequest::Toolbar(ToolbarRequest::Shown(shown)));
    }

    fn close_popover(&mut self) {
        if self.count_call() && self.has_toolbar("close-popover") {
            self.ui(UiRequest::Toolbar(ToolbarRequest::ClosePopover));
        }
    }
}

impl service::Host for HostState {
    fn info(&mut self) -> ServiceInfo {
        self.service.clone()
    }

    fn restart(&mut self) {
        if !self.count_call() {
            return;
        }
        if self.has_service {
            self.ui(UiRequest::RestartService);
        } else {
            self.host_log(LogLevel::Warn, "service.restart: the manifest declares no service".into());
        }
    }

    fn open_card(&mut self) {
        if self.count_call() {
            self.ui(UiRequest::OpenSettings);
        }
    }
}
