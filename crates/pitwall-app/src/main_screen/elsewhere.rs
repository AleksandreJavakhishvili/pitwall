//! The sidebar's "Elsewhere" group (React `ElsewhereGroup.tsx`,
//! `BringInDialog.tsx`): coding agents running in other terminal apps on
//! this machine, read-only, listed every ~10 s from the engine's scan
//! (`onboarding::elsewhere`), each with "Bring in", which resumes its
//! conversation in a new Pitwall agent. Settings → "Show agents running
//! elsewhere" hides it (`ui.hideElsewhere`).

use std::sync::Arc;
use std::time::Duration;

use gpui::{div, prelude::*, px, AnyElement, Context, IntoElement, SharedString, Window};

use pitwall_core::onboarding::elsewhere::Elsewhere;
use pitwall_core::onboarding::scan::RunningAgent;
use pitwall_core::onboarding::ContinueRequest;

use super::MainScreen;
use crate::kit::{
    chevron, label_fit, muted, small_btn, text_button, tooltip, BtnKind, HoverText, Modal,
    MONO_FONT,
};
use crate::theme::{Theme, RADIUS};

/// Collapse key of the group (shares `ui.collapsed` with projects).
pub const KEY: &str = "\u{0}elsewhere";
/// How often the list is read again.
pub const POLL: Duration = Duration::from_secs(10);

/// What the group needs between frames.
#[derive(Default)]
pub struct ElsewhereState {
    pub rows: Vec<RunningAgent>,
    /// The row "Bring in" was clicked on (the dialog is open).
    pub bring: Option<RunningAgent>,
    pub busy: bool,
}

/// What tells two lists apart (`RunningAgent` has no `PartialEq`).
type Sig<'a> = (u32, Option<&'a str>, Option<&'a str>, Option<&'a str>);

fn sig(rows: &[RunningAgent]) -> Vec<Sig<'_>> {
    rows.iter()
        .map(|r| {
            (
                r.pid,
                r.session_id.as_deref(),
                r.title.as_deref(),
                r.cwd.as_deref(),
            )
        })
        .collect()
}

/// It can be resumed: a known conversation in a known folder (`canBringIn`).
pub fn can_bring_in(r: &RunningAgent) -> bool {
    r.session_id.is_some() && r.cwd.is_some()
}

impl MainScreen {
    /// Read the list now, then every [`POLL`] while the window lives.
    pub(super) fn start_elsewhere(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let scanner = Arc::new(Elsewhere::new());
        self._poll_elsewhere = Some(cx.spawn(async move |this, cx| loop {
            let hidden = this.update(cx, |s, _| s.ui.hide_elsewhere).unwrap_or(true);
            if !hidden {
                let (e, s) = (engine.clone(), scanner.clone());
                let rows = cx
                    .background_executor()
                    .spawn(async move { s.list(&e).unwrap_or_default() })
                    .await;
                if this
                    .update(cx, |s, cx| {
                        if sig(&s.elsewhere.rows) != sig(&rows) {
                            s.elsewhere.rows = rows;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
            cx.background_executor().timer(POLL).await;
        }));
    }

    /// The group's header and rows (nothing when hidden or empty).
    pub(super) fn render_elsewhere(
        &mut self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if self.ui.hide_elsewhere || self.elsewhere.rows.is_empty() {
            return vec![];
        }
        let collapsed = self.ui.collapsed.iter().any(|k| k == KEY);
        let n = self.elsewhere.rows.len();
        let head =
            div()
                .id("elsewhere-head")
                .group("elsewhere-head")
                .flex()
                .items_center()
                .pl(px(8.))
                .pr(px(6.))
                .pt(px(6.))
                .pb(px(2.))
                .child(
                    div()
                        .id("elsewhere-toggle")
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(4.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .tooltip(tooltip("Agents running in other terminal apps"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_ui(|s| s.toggle_collapsed(KEY), cx)
                        }))
                        .child(chevron("chev", !collapsed, 12., t.text_4))
                        .child(label_fit("Elsewhere", t.text_4, 12., 22).group_hover_text(
                            "elsewhere-head",
                            t.text_2,
                            |s| s,
                        ))
                        .child(
                            div()
                                .flex_none()
                                .font_family(MONO_FONT)
                                .text_size(px(11.))
                                .text_color(t.text_3)
                                .child(n.to_string()),
                        ),
                );
        let mut out = vec![head.into_any_element()];
        if collapsed {
            return out;
        }
        for (i, r) in self.elsewhere.rows.clone().into_iter().enumerate() {
            out.push(self.elsewhere_row(i, r, t, cx).into_any_element());
        }
        out
    }

    fn elsewhere_row(
        &mut self,
        i: usize,
        r: RunningAgent,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can = can_bring_in(&r);
        let tip: SharedString = match &r.cwd {
            Some(cwd) => format!("{} in another terminal (pid {})\n{cwd}", r.kind_name, r.pid),
            None => format!("{} in another terminal (pid {})", r.kind_name, r.pid),
        }
        .into();
        let name = r
            .title
            .clone()
            .or(r.cwd_display.clone())
            .unwrap_or(r.kind_name.clone());
        let hover = t.surface_2;
        let group = SharedString::from(format!("elsewhere-row-{i}"));
        let row = r.clone();
        div()
            .id(("elsewhere-row", i))
            .group(group.clone())
            .mx(px(6.))
            .flex()
            .items_center()
            .gap(px(9.))
            .pl(px(9.))
            .pr(px(8.))
            .py(px(6.))
            .rounded(RADIUS)
            .text_color(t.text_2)
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip))
            .child(
                div()
                    .flex_none()
                    .size(px(8.))
                    .rounded_full()
                    .border(px(1.5))
                    .border_dashed()
                    .border_color(t.text_3),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_color(t.text).child(name))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(t.text_3)
                            .child(div().flex_none().child(r.kind_name.clone()))
                            .when_some(r.title.as_ref().and(r.cwd_display.clone()), |d, c| {
                                d.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(MONO_FONT)
                                        .text_size(px(11.))
                                        .child(c),
                                )
                            }),
                    ),
            )
            .child(
                small_btn(("elsewhere-bring", i), "Bring in", t)
                    .opacity(if can { 0.75 } else { 0.45 })
                    .when(!can, |d| d.cursor_default())
                    .tooltip(tooltip(if can {
                        "Resume this conversation in Pitwall"
                    } else {
                        "Pitwall can't tell which conversation this is yet"
                    }))
                    .when(can, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.elsewhere.bring = Some(row.clone());
                            cx.emit(super::ScreenEvent::ModalOpened);
                            cx.notify();
                        }))
                    }),
            )
    }

    /// "Bring into Pitwall".
    pub(super) fn render_bring_in(
        &mut self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let r = self.elsewhere.bring.clone()?;
        let busy = self.elsewhere.busy;
        let close = cx.entity().downgrade();
        let body = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(18.))
                    .pt(px(8.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(muted(
                        match &r.cwd_display {
                            Some(c) => format!(
                                "Resumes this {} conversation in a new Pitwall agent in {c}.",
                                r.kind_name
                            ),
                            None => format!(
                                "Resumes this {} conversation in a new Pitwall agent.",
                                r.kind_name
                            ),
                        },
                        t,
                    ))
                    .when_some(r.title.clone(), |d, title| {
                        d.child(crate::kit::hint(format!("“{title}”"), t))
                    })
                    .child(
                        div()
                            .px(px(14.))
                            .py(px(12.))
                            .rounded(RADIUS)
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.line_strong)
                            .text_size(px(12.5))
                            .text_color(t.text_2)
                            .child(format!(
                                "Close the original first (in the other terminal, pid {}), so two copies don't work on the same conversation. Pitwall never stops it for you.",
                                r.pid
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .px(px(18.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(
                        text_button("bring-cancel", "Cancel", BtnKind::Ghost, false, t).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.elsewhere.bring = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        text_button(
                            "bring-go",
                            if busy { "Starting…" } else { "I closed it, resume here" },
                            BtnKind::Primary,
                            busy,
                            t,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| this.bring_in(window, cx))),
                    ),
            );
        Some(
            Modal::new("bring-in", 460., move |_, cx| {
                let _ = close.update(cx, |s, cx| {
                    s.elsewhere.bring = None;
                    cx.notify();
                });
            })
            .title("Bring into Pitwall")
            .motion(crate::theme::motion_on(cx))
            .render(t, body),
        )
    }

    fn bring_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(engine), Some(r)) = (self.engine.clone(), self.elsewhere.bring.clone()) else {
            return;
        };
        let (Some(session_id), Some(cwd)) = (r.session_id.clone(), r.cwd.clone()) else {
            return;
        };
        if self.elsewhere.busy {
            return;
        }
        self.elsewhere.busy = true;
        let taken: Vec<String> = self.agents(cx).iter().map(|a| a.name.clone()).collect();
        let name = super::model::suggest_name(r.display_project.as_deref().unwrap_or(&cwd), &taken);
        let (cols, rows) = self.new_agent_size();
        let req = ContinueRequest {
            kind: r.kind.clone(),
            session_id,
            project_path: cwd,
            name,
            display_project: r.display_project.clone(),
            cols,
            rows,
        };
        let task = cx
            .background_executor()
            .spawn(async move { pitwall_core::onboarding::continue_conversation(&engine, req) });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update_in(cx, |s, window, cx| {
                s.elsewhere.busy = false;
                s.elsewhere.bring = None;
                match res {
                    Ok(view) => s.created(view, window, cx),
                    Err(e) => s.failed("bring the agent into Pitwall", e, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(session: Option<&str>, cwd: Option<&str>) -> RunningAgent {
        RunningAgent {
            pid: 42,
            kind: "claude".into(),
            kind_name: "Claude Code".into(),
            cwd: cwd.map(String::from),
            cwd_display: cwd.map(String::from),
            session_id: session.map(String::from),
            title: None,
            in_pitwall: false,
            outside_project: false,
            display_project: None,
        }
    }

    #[test]
    fn only_known_conversations_can_be_brought_in() {
        assert!(can_bring_in(&row(Some("s1"), Some("/work/alpha"))));
        assert!(!can_bring_in(&row(None, Some("/work/alpha"))));
        assert!(!can_bring_in(&row(Some("s1"), None)));
    }
}
