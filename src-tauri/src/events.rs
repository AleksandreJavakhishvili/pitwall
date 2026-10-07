//! The app's [`EventSink`]: engine events become webview events (names and
//! payloads as in docs/CONTRACT.md), notifications and the Dock badge.

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use pitwall_core::events::{Event, EventSink};

use crate::attention;

pub struct AppEvents {
    app: AppHandle,
}

impl AppEvents {
    pub fn new(app: AppHandle) -> AppEvents {
        AppEvents { app }
    }
}

/// The webview event for an engine event (`None`: not a webview event).
pub fn webview_event(event: &Event) -> Option<(&'static str, Value)> {
    let to = |v: Result<Value, serde_json::Error>| v.unwrap_or(Value::Null);
    Some(match event {
        Event::AgentsChanged(views) => ("agents-changed", to(serde_json::to_value(views))),
        Event::Attention(item) => ("attention", to(serde_json::to_value(item))),
        Event::ScanProgress(p) => ("scan-progress", to(serde_json::to_value(p))),
        Event::ProjectsChanged(list) => ("projects-changed", to(serde_json::to_value(list))),
        Event::BlockedCount(_) => return None,
    })
}

impl EventSink for AppEvents {
    fn emit(&self, event: Event) {
        if let Some((name, payload)) = webview_event(&event) {
            let _ = self.app.emit(name, payload);
        }
        match event {
            Event::Attention(item) => attention::notify(&self.app, &item),
            Event::BlockedCount(n) => attention::set_badge(&self.app, n),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::model::Attention;
    use pitwall_core::onboarding::project_list::Project;
    use pitwall_core::onboarding::scan::ScanProgress;
    use serde_json::json;

    /// Names and JSON shapes the UI listens for must not change.
    #[test]
    fn webview_events_keep_their_names_and_shapes() {
        let att = Attention { agent_id: "a1".into(), name: "Fixer".into(), reason: "blocked", detail: None };
        assert_eq!(
            webview_event(&Event::Attention(att)),
            Some(("attention", json!({"agentId": "a1", "name": "Fixer", "reason": "blocked"})))
        );
        let p = Project { path: "/p".into(), display: "/p".into(), is_git: true, added_at: 5 };
        assert_eq!(
            webview_event(&Event::ProjectsChanged(vec![p])),
            Some(("projects-changed", json!([{"path": "/p", "display": "/p", "isGit": true, "addedAt": 5}])))
        );
        let step = ScanProgress { step: "agents", status: "running", summary: None };
        assert_eq!(
            webview_event(&Event::ScanProgress(step)),
            Some(("scan-progress", json!({"step": "agents", "status": "running"})))
        );
        assert_eq!(webview_event(&Event::AgentsChanged(vec![])), Some(("agents-changed", json!([]))));
        assert_eq!(webview_event(&Event::BlockedCount(3)), None, "badge only");
    }
}
