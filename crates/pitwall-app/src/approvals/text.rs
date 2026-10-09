//! What the approval dialog says (a port of the helpers in
//! `src/components/ApprovalDialog.tsx`, with its tests).

use pitwall_proto::{ApprovalView, RequesterKind};

/// "1:54" until the request is denied for lack of an answer.
pub fn time_left(expires_at: u64, now: u64) -> String {
    let s = (expires_at.saturating_sub(now) as f64 / 1000.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Who asked, as Pitwall established it from the calling process.
pub fn requester_line(a: &ApprovalView) -> String {
    let r = &a.requester;
    let who = match r.kind {
        RequesterKind::Agent => format!("agent “{}” in Pitwall", r.name),
        RequesterKind::Outside => "a process outside Pitwall's agents".to_string(),
    };
    let proc: Vec<String> = [r.process.clone(), r.pid.map(|p| format!("pid {p}"))]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if proc.is_empty() {
        who
    } else {
        format!("{who} ({})", proc.join(", "))
    }
}

/// The "remember" checkbox's label.
pub fn remember_label(a: &ApprovalView) -> String {
    let who = match a.requester.kind {
        RequesterKind::Agent => a.requester.name.as_str(),
        RequesterKind::Outside => "processes outside Pitwall",
    };
    format!("Allow {who} to do this again without asking")
}

/// "Denied in 1:54 · 2 more waiting".
pub fn footer(a: &ApprovalView, waiting: usize, now: u64) -> String {
    let more = if waiting > 1 {
        format!(" · {} more waiting", waiting - 1)
    } else {
        String::new()
    };
    format!("Denied in {}{more}", time_left(a.expires_at, now))
}

/// Unix ms now (the countdown's clock).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pitwall_proto::{Requester, Risk};

    /// A made-up request.
    pub fn view(id: &str, kind: RequesterKind) -> ApprovalView {
        ApprovalView {
            id: id.into(),
            action: "session.add".into(),
            summary: "start the session \"work\" on vm-1 (agw)".into(),
            details: vec![],
            requester: Requester {
                kind,
                agent_id: Some("a1".into()),
                name: "Race Engineer".into(),
                pid: Some(42),
                process: Some("pitwall".into()),
            },
            risk: Risk::Low,
            rememberable: true,
            created_at: 0,
            expires_at: 120_000,
        }
    }

    #[test]
    fn counts_down_to_the_automatic_denial() {
        assert_eq!(time_left(120_000, 0), "2:00");
        assert_eq!(time_left(120_000, 114_500), "0:06");
        assert_eq!(time_left(120_000, 200_000), "0:00");
    }

    #[test]
    fn names_who_asked_as_pitwall_established_it() {
        let a = view("a", RequesterKind::Agent);
        assert_eq!(
            requester_line(&a),
            "agent “Race Engineer” in Pitwall (pitwall, pid 42)"
        );
        let mut outside = view("b", RequesterKind::Outside);
        outside.requester.pid = None;
        outside.requester.process = None;
        assert_eq!(
            requester_line(&outside),
            "a process outside Pitwall's agents"
        );
        assert_eq!(
            remember_label(&outside),
            "Allow processes outside Pitwall to do this again without asking"
        );
        assert_eq!(
            remember_label(&a),
            "Allow Race Engineer to do this again without asking"
        );
    }

    #[test]
    fn the_footer_counts_the_others_waiting() {
        let a = view("a", RequesterKind::Agent);
        assert_eq!(footer(&a, 1, 0), "Denied in 2:00");
        assert_eq!(footer(&a, 3, 60_000), "Denied in 1:00 · 2 more waiting");
    }
}
