//! Sessions on other machines (agw VMs today): each provider that can adopt
//! sessions, its machines and the sessions there, each with "Add to
//! Pitwall" (Tauri: onboarding's "Running now" `MachineGroup`, the
//! `pitwall session list/add` data). Adding only starts tracking: Pitwall
//! attaches to the session's terminal; nothing changes on the machine.
//!
//! The data comes through a [`Source`]: the hosted engine in the app
//! ([`Source::engine`]: `onboarding::places` and `lifecycle::adopt`, both
//! blocking, so they run off the main thread), made-up places in tests.
//! [`machine_group`] draws one machine; onboarding can reuse it.

use crate::kit::Ellipsis as _;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::kit::HoverText;
use gpui::{
    div, prelude::*, px, AnyElement, Context, FontWeight, IntoElement, Render, SharedString, Window,
};

use pitwall_core::model::AdoptSessionRequest;
use pitwall_core::Shared;
use pitwall_proto::{ScannedMachine, ScannedPlace, ScannedSession};

use crate::kit::{self, Tone, MONO_FONT};
use crate::theme::{self, Theme, RADIUS};

/// Lists every adopting provider's places (blocking).
pub type ListPlaces = Arc<dyn Fn() -> Vec<ScannedPlace> + Send + Sync>;
/// Adds one session to Pitwall (blocking).
pub type AddSession = Arc<dyn Fn(&ScannedSession) -> Result<(), String> + Send + Sync>;

/// Where the panel's data comes from.
#[derive(Clone)]
pub struct Source {
    pub places: ListPlaces,
    pub add: AddSession,
}

impl Source {
    /// The hosted engine's providers (read-only listing; "Add" tracks).
    pub fn engine(engine: Shared) -> Source {
        let list = engine.clone();
        Source {
            places: Arc::new(move || {
                let owned = list.records().iter().map(|r| r.locator()).collect();
                pitwall_core::onboarding::places::places(list.providers(), list.kinds(), &owned)
            }),
            add: Arc::new(move |x: &ScannedSession| {
                pitwall_core::engine::lifecycle::adopt(
                    &engine,
                    AdoptSessionRequest {
                        provider: x.provider.clone(),
                        machine: x.machine.clone(),
                        native: x.native.clone(),
                        cols: None,
                        rows: None,
                    },
                )
                .map(drop)
            }),
        }
    }
}

/// A session's identity (`provider:machine:native`).
pub fn session_key(x: &ScannedSession) -> String {
    format!("{}:{}:{}", x.provider, x.machine, x.native)
}

/// How a session's status is said.
pub fn status_word(status: &str) -> &'static str {
    match status {
        "running" => "running",
        "stopped" => "stopped",
        _ => "status unknown",
    }
}

/// "workspace web · as builder".
pub fn where_line(x: &ScannedSession) -> String {
    [
        x.workspace.as_ref().map(|w| format!("workspace {w}")),
        x.user.as_ref().map(|u| format!("as {u}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// "agw 1.4 is installed, but its machines and sessions couldn't be listed."
pub fn unlisted_note(p: &ScannedPlace) -> String {
    let name = [Some(p.label.clone()), p.version.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    format!("{name} is installed, but its machines and sessions couldn't be listed.")
}

#[derive(Debug, Clone, PartialEq)]
enum Row {
    Idle,
    Adding,
    Added,
    Failed(String),
}

/// The panel: loads on creation and on [`MachineSessions::refresh`].
pub struct MachineSessions {
    source: Source,
    places: Option<Vec<ScannedPlace>>,
    rows: HashMap<String, Row>,
    loading: bool,
}

impl MachineSessions {
    pub fn new(source: Source, cx: &mut Context<Self>) -> Self {
        let mut this = MachineSessions {
            source,
            places: None,
            rows: HashMap::new(),
            loading: false,
        };
        this.refresh(cx);
        this
    }

    /// List again (in the background).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let list = self.source.places.clone();
        let task = cx.background_executor().spawn(async move { list() });
        cx.spawn(async move |this, cx| {
            let places = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.places = Some(places);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// "Add to Pitwall" for one session (in the background).
    pub fn add(&mut self, x: ScannedSession, cx: &mut Context<Self>) {
        let key = session_key(&x);
        if matches!(self.rows.get(&key), Some(Row::Adding | Row::Added)) || x.in_pitwall {
            return;
        }
        self.rows.insert(key.clone(), Row::Adding);
        let add = self.source.add.clone();
        let task = cx.background_executor().spawn(async move { add(&x) });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                this.rows.insert(
                    key,
                    match done {
                        Ok(()) => Row::Added,
                        Err(e) => Row::Failed(e),
                    },
                );
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Sessions that could be added (for a count or a heading).
    pub fn adoptable(&self) -> usize {
        let added: HashSet<&String> = self
            .rows
            .iter()
            .filter(|(_, r)| **r == Row::Added)
            .map(|(k, _)| k)
            .collect();
        self.places
            .iter()
            .flatten()
            .flat_map(|p| p.machines.iter().flatten())
            .flat_map(|m| &m.sessions)
            .filter(|x| !x.in_pitwall && !added.contains(&session_key(x)))
            .count()
    }
}

/// One machine and its sessions; `action` draws each row's right end.
pub fn machine_group(
    place: &str,
    m: &ScannedMachine,
    t: &Theme,
    mut action: impl FnMut(&ScannedSession) -> AnyElement,
) -> impl IntoElement {
    let sub = [Some(place.to_string()), m.detail.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let head = div()
        .mt_1()
        .flex()
        .items_baseline()
        .gap_2()
        .child(
            div()
                .font_family(MONO_FONT)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_2)
                .child(SharedString::from(m.label.clone())),
        )
        .child(div().text_xs().text_color(t.text_3).child(sub));
    let mut list = div()
        .flex()
        .flex_col()
        .rounded(RADIUS)
        .border_1()
        .border_color(t.line)
        .bg(t.surface);
    if m.sessions.is_empty() {
        list = list.child(
            div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(t.text_3)
                .child("No sessions"),
        );
    }
    for (i, x) in m.sessions.iter().enumerate() {
        let running = x.status == "running";
        list = list.child(
            div()
                .px_3()
                .py_2()
                .min_h(px(42.))
                .flex()
                .items_center()
                .gap(px(10.))
                .when(i > 0, |d| d.border_t_1().border_color(t.line))
                .child(
                    chip(&x.kind_name, t, false)
                        .min_w(px(76.))
                        .justify_center(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .when(x.in_pitwall, |d| d.opacity(0.6))
                        .child(
                            div()
                                .font_family(MONO_FONT)
                                .text_size(px(12.))
                                .ellipsis()
                                .text_color(t.text)
                                .child(crate::kit::one_line(SharedString::from(x.name.clone()))),
                        )
                        .child(
                            div()
                                .text_xs()
                                .ellipsis()
                                .text_color(t.text_3)
                                .child(crate::kit::one_line(where_line(x))),
                        ),
                )
                .child(chip(status_word(&x.status), t, running))
                .child(action(x)),
        );
    }
    div().flex().flex_col().gap_1().child(head).child(list)
}

/// `.chip` (`chip-ok` when `ok`, else `chip-subtle`).
fn chip(label: &str, t: &Theme, ok: bool) -> gpui::Div {
    kit::chip_tone(label.to_string(), if ok { Tone::Ok } else { Tone::Subtle }, t)
}

impl Render for MachineSessions {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let mut root = div().flex().flex_col().gap_3().text_color(t.text);
        let Some(places) = self.places.clone() else {
            return root.child(
                div()
                    .text_xs()
                    .text_color(t.text_3)
                    .child("Looking for sessions on other machines…"),
            );
        };
        let mut any = false;
        for p in &places {
            let Some(machines) = &p.machines else {
                root = root.child(div().text_xs().text_color(t.text_3).child(unlisted_note(p)));
                continue;
            };
            for m in machines {
                any = true;
                let group = machine_group(&p.label, m, &t, |x| {
                    let key = session_key(x);
                    let row = self.rows.get(&key).cloned().unwrap_or(Row::Idle);
                    if x.in_pitwall || row == Row::Added {
                        return chip("in Pitwall", &t, true).into_any_element();
                    }
                    let x = x.clone();
                    let busy = row == Row::Adding;
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(px(2.))
                        .child(
                            div()
                                .id(SharedString::from(format!("add-{key}")))
                                .h(px(26.))
                                .px(px(9.))
                                .flex()
                                .items_center()
                                .rounded(RADIUS)
                                .border_1()
                                .border_color(t.line_strong)
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text_2)
                                .when(!busy, |d| {
                                    d.cursor_pointer()
                                        .hover_text(t.text, |s| s.bg(t.surface_3))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.add(x.clone(), cx)
                                        }))
                                })
                                .child(if busy { "Adding…" } else { "Add to Pitwall" }),
                        )
                        .when_some(
                            match row {
                                Row::Failed(e) => Some(e),
                                _ => None,
                            },
                            |d, e| d.child(div().text_xs().text_color(t.red).child(e)),
                        )
                        .into_any_element()
                });
                root = root.child(group);
            }
        }
        if any {
            root = root.child(div().text_xs().text_color(t.text_3).child(
                "Adding a session only starts tracking it: Pitwall attaches to its terminal and \
                 reads its status from the screen. Nothing changes on the machine.",
            ));
        } else if places.iter().all(|p| p.machines.is_some()) {
            root = root.child(
                div()
                    .text_xs()
                    .text_color(t.text_3)
                    .child("No other machines are set up."),
            );
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext, VisualTestContext};
    use std::sync::Mutex;

    /// A made-up agw place: one VM with two sessions (never a real agw).
    fn place() -> ScannedPlace {
        let s = |native: &str, status: &str, in_pitwall: bool| ScannedSession {
            provider: "agw".into(),
            machine: "vm-1".into(),
            native: native.into(),
            name: native.into(),
            kind: "claude".into(),
            kind_name: "Claude Code".into(),
            program: "claude-code".into(),
            workspace: Some("web".into()),
            user: Some("builder".into()),
            cwd: None,
            status: status.into(),
            in_pitwall,
        };
        ScannedPlace {
            provider: "agw".into(),
            label: "agw".into(),
            version: Some("1.4".into()),
            machines: Some(vec![ScannedMachine {
                id: "vm-1".into(),
                label: "vm-1".into(),
                detail: Some("site-a".into()),
                sessions: vec![s("work", "running", false), s("old", "stopped", true)],
            }]),
        }
    }

    fn mock(fail: bool) -> (Source, Arc<Mutex<Vec<String>>>) {
        let added = Arc::new(Mutex::new(Vec::new()));
        let log = added.clone();
        (
            Source {
                places: Arc::new(|| vec![place()]),
                add: Arc::new(move |x: &ScannedSession| {
                    log.lock().unwrap().push(session_key(x));
                    if fail {
                        Err("vm-1 has no session \"work\"".into())
                    } else {
                        Ok(())
                    }
                }),
            },
            added,
        )
    }

    #[test]
    fn rows_say_where_and_how() {
        let p = place();
        let x = &p.machines.as_ref().unwrap()[0].sessions[0];
        assert_eq!(where_line(x), "workspace web · as builder");
        assert_eq!(session_key(x), "agw:vm-1:work");
        assert_eq!(status_word("unknown"), "status unknown");
        let unlisted = ScannedPlace {
            machines: None,
            ..place()
        };
        assert_eq!(
            unlisted_note(&unlisted),
            "agw 1.4 is installed, but its machines and sessions couldn't be listed."
        );
    }

    #[gpui::test]
    fn lists_and_adds_a_session(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(Theme::dark()));
        let (source, added) = mock(false);
        let (panel, vcx) = cx.add_window_view(|_, cx| MachineSessions::new(source, cx));
        let vcx: &mut VisualTestContext = vcx;
        vcx.run_until_parked();
        assert_eq!(panel.read_with(vcx, |p, _| p.adoptable()), 1);

        let work = place().machines.unwrap()[0].sessions[0].clone();
        panel.update(vcx, |p, cx| p.add(work.clone(), cx));
        vcx.run_until_parked();
        assert_eq!(*added.lock().unwrap(), ["agw:vm-1:work"]);
        assert_eq!(panel.read_with(vcx, |p, _| p.adoptable()), 0);
        // Added once; a second click does nothing.
        panel.update(vcx, |p, cx| p.add(work, cx));
        vcx.run_until_parked();
        assert_eq!(added.lock().unwrap().len(), 1);
    }

    #[gpui::test]
    fn a_failed_add_says_why_and_can_be_retried(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(Theme::dark()));
        let (source, added) = mock(true);
        let panel = cx.new(|cx| MachineSessions::new(source, cx));
        cx.run_until_parked();
        let work = place().machines.unwrap()[0].sessions[0].clone();
        panel.update(cx, |p, cx| p.add(work.clone(), cx));
        cx.run_until_parked();
        panel.read_with(cx, |p, _| {
            assert!(matches!(p.rows.get("agw:vm-1:work"), Some(Row::Failed(_))))
        });
        panel.update(cx, |p, cx| p.add(work, cx));
        cx.run_until_parked();
        assert_eq!(added.lock().unwrap().len(), 2);
        // Sessions already in Pitwall are never added again.
        let old = place().machines.unwrap()[0].sessions[1].clone();
        panel.update(cx, |p, cx| p.add(old, cx));
        cx.run_until_parked();
        assert_eq!(added.lock().unwrap().len(), 2);
    }
}
