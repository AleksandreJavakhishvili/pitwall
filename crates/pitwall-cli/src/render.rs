//! Readable text (`--human`). JSON is the default and is printed as is.

use std::fmt::Write;

use pitwall_proto::settings::Sensitivity;
use pitwall_proto::{AgentView, CreateForm, FieldInput, ProviderMachines, ScannedPlace, SessionAdded, SettingView};

/// Columns padded to their widest cell (the last one isn't padded).
pub fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols).map(|c| rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for r in rows {
        let line: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, cell)| if i + 1 == r.len() { cell.clone() } else { format!("{cell:<w$}", w = widths[i]) })
            .collect();
        let _ = writeln!(out, "{}", line.join("  ").trim_end());
    }
    out
}

fn status(a: &AgentView) -> String {
    serde_json::to_value(a.status).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

pub fn agents(list: &[AgentView]) -> String {
    if list.is_empty() {
        return "No agents in Pitwall.\n".into();
    }
    let mut rows = vec![vec!["NAME".into(), "KIND".into(), "STATUS".into(), "MACHINE".into(), "FOLDER".into(), "ID".into()]];
    rows.extend(list.iter().map(|a| vec![a.name.clone(), a.kind_name.clone(), status(a), a.machine.label.clone(), a.cwd_display.clone(), a.id.clone()]));
    table(&rows)
}

pub fn agent_new(a: &AgentView) -> String {
    format!("Started {} ({}) in {} — id {}\n", a.name, a.kind_name, a.cwd_display, a.id)
}

pub fn machines(list: &[ProviderMachines]) -> String {
    let mut out = String::new();
    for p in list {
        let version = p.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default();
        let mut can = vec![];
        if p.can_create {
            can.push("new agents");
        }
        if p.can_add_sessions {
            can.push("add sessions");
        }
        let can = if can.is_empty() { String::new() } else { format!(" — {}", can.join(", ")) };
        let _ = writeln!(out, "{} ({}{version}){can}", p.label, p.provider);
        if let Some(e) = &p.error {
            let _ = writeln!(out, "  can't list machines: {e}");
        }
        let rows: Vec<Vec<String>> =
            p.machines.iter().map(|m| vec![format!("  {}", m.id), m.label.clone(), m.detail.clone().unwrap_or_default()]).collect();
        out.push_str(&table(&rows));
    }
    if out.is_empty() {
        out.push_str("No providers.\n");
    }
    out
}

pub fn sessions(places: &[ScannedPlace]) -> String {
    let mut out = String::new();
    for p in places {
        let Some(machines) = &p.machines else {
            let _ = writeln!(out, "{}: couldn't list machines", p.label);
            continue;
        };
        for m in machines {
            let _ = writeln!(out, "{} / {}", p.label, m.label);
            if m.sessions.is_empty() {
                let _ = writeln!(out, "  (no sessions)");
                continue;
            }
            let rows: Vec<Vec<String>> = m
                .sessions
                .iter()
                .map(|s| {
                    vec![
                        format!("  {}", s.native),
                        s.kind_name.clone(),
                        s.status.clone(),
                        if s.in_pitwall { "in Pitwall".into() } else { "-".into() },
                        s.workspace.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            out.push_str(&table(&rows));
        }
    }
    if out.is_empty() {
        out.push_str("No other places with sessions (is agw set up?).\n");
    }
    out
}

pub fn session_added(r: &SessionAdded) -> String {
    let a = &r.agent;
    let what = match (r.already, r.started) {
        (_, true) => "Started and added",
        (true, false) => "Already in Pitwall:",
        (false, false) => "Added",
    };
    let state = if a.running { "running" } else { "stopped" };
    format!("{what} {} on {} ({state}) — id {}\n", a.name, a.machine.label, a.id)
}

/// A machine's form: each field with its choices (`agent new --machine`).
pub fn form(f: &CreateForm) -> String {
    let mut out = String::new();
    if f.folder {
        let _ = writeln!(out, "{} ({}): new agents work in a folder here: agent new --kind <kind> --project <path>", f.machine_label, f.provider);
        return out;
    }
    let _ = writeln!(out, "{} ({}): agent new --machine {} --name <name> ({})", f.machine_label, f.provider, f.machine, f.name.hint);
    if let Some(e) = &f.error {
        let _ = writeln!(out, "  (some choices couldn't be listed: {e})");
    }
    for field in &f.fields {
        let when = field.when.as_ref().map(|w| format!(" (when {}={})", w.field, w.value)).unwrap_or_default();
        let default = field.default.as_ref().map(|d| format!(", default {d}")).unwrap_or_default();
        let _ = writeln!(out, "{} — {}{when}{default}", field.id, field.label);
        match field.input {
            FieldInput::Text => {
                let note = if field.defaults_to_name { "text; empty = the session's name" } else { "text" };
                let _ = writeln!(out, "  {note}");
            }
            FieldInput::Select => {
                let rows: Vec<Vec<String>> = field
                    .choices
                    .iter()
                    .map(|c| vec![format!("  {}", c.value), c.label.clone(), c.detail.clone().unwrap_or_default()])
                    .collect();
                out.push_str(&table(&rows));
            }
        }
    }
    out
}

fn value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "?".into(),
        // Long text (a prompt) cut short so the table stays readable; `get` shows it whole.
        serde_json::Value::String(s) if s.chars().count() > 40 => format!("{}…", s.chars().take(39).collect::<String>()),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `settings list`: one row per setting, its description under it.
/// `live`: read from the running Pitwall (else from ui.json).
pub fn settings(list: &[SettingView], live: bool) -> String {
    let mut rows = vec![vec!["KEY".into(), "VALUE".into(), "ALLOWED".into(), "DEFAULT".into()]];
    for s in list {
        let mut allowed = match (&s.choices, s.min, s.max) {
            (Some(c), _, _) => c.join("|"),
            (_, _, Some(hi)) if s.kind == "text" => format!("text (≤{hi})"),
            (_, Some(lo), Some(hi)) => format!("{lo}–{hi}"),
            _ if s.only_on => "true".into(),
            _ => "true|false".into(),
        };
        if s.sensitivity == Sensitivity::Approval {
            allowed.push_str("  (approval)");
        }
        rows.push(vec![s.key.clone(), value_text(&s.value), allowed, value_text(&s.default)]);
    }
    let table = table(&rows);
    let mut out = String::new();
    for (i, line) in table.lines().enumerate() {
        let _ = writeln!(out, "{line}");
        if i > 0 {
            let _ = writeln!(out, "    {}", list[i - 1].description);
        }
    }
    if !live {
        out.push_str("Pitwall isn't running: values are from ui.json; \"?\" ones need Pitwall.\n");
    }
    out
}

/// `settings get|set|reset`.
pub fn setting(s: &SettingView) -> String {
    let value = match &s.value {
        serde_json::Value::String(t) => t.clone(),
        v => value_text(v),
    };
    format!("{} = {value}\n", s.key)
}
