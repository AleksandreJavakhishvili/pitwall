//! The sidebar: agents grouped by project, each with its status glyph,
//! colour and name (Tauri: `src/components/Sidebar.tsx`, `AgentRow.tsx`).
//! Phase 0 shows only what proves the bridge; collapsing, context menus,
//! drag and drop and the rest are inventory items for phase 2.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, IntoElement, SharedString};

use pitwall_proto::AgentView;

use super::MainView;
use crate::agents::{status_glyph, status_word, AgentStore};
use crate::theme::{Theme, RADIUS, SIDEBAR_W};

pub fn render(
    store: &AgentStore,
    selected: Option<&str>,
    data_dir: &SharedString,
    t: &Theme,
    cx: &Context<MainView>,
) -> impl IntoElement {
    let groups = store.groups();
    let mut list: Vec<AnyElement> = Vec::new();
    for g in &groups {
        // The folder's name first, its parent dimmed after it (truncated).
        let (name, parent) = split_display(&g.display);
        let machine = g
            .machine
            .as_ref()
            .filter(|_| !g.can_create)
            .map(|m| format!(" · {}", m.label));
        list.push(
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(
                            gpui::StyledText::new(format!(
                                "{name}{}  {parent}",
                                machine.unwrap_or_default()
                            ))
                            .with_highlights([(
                                0..name.len(),
                                gpui::HighlightStyle {
                                    color: Some(t.text_2),
                                    font_weight: Some(FontWeight::SEMIBOLD),
                                    ..Default::default()
                                },
                            )]),
                        )
                        .text_color(t.text_4),
                )
                .when(g.blocked > 0, |d| {
                    d.child(div().text_color(t.amber).child(format!("{} ▲", g.blocked)))
                })
                .into_any_element(),
        );
        for a in &g.agents {
            list.push(row(a, selected == Some(a.id.as_str()), t, cx).into_any_element());
        }
    }
    let empty = groups.is_empty();
    let footer = format!(
        "{} agent{} · {} need{} you{}",
        store.agents.len(),
        if store.agents.len() == 1 { "" } else { "s" },
        store.blocked,
        if store.blocked == 1 { "s" } else { "" },
        match store.approvals.len() {
            0 => String::new(),
            n => format!(" · {n} approval{}", if n == 1 { "" } else { "s" }),
        }
    );
    div()
        .w(SIDEBAR_W)
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(t.surface)
        .border_r_1()
        .border_color(t.line)
        .child(
            div()
                .h(px(36.))
                .px_3()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(t.line)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_2)
                .child("Agents"),
        )
        .child(
            div()
                .id("agent-list")
                .flex_1()
                .overflow_y_scroll()
                .pb_2()
                .children(list)
                .when(empty, |d| {
                    d.child(
                        div()
                            .p_4()
                            .text_sm()
                            .text_color(t.text_3)
                            .child("No agents yet. Start one with the pitwall CLI."),
                    )
                }),
        )
        .child(
            div()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(t.line)
                .flex()
                .flex_col()
                .gap_0p5()
                .text_xs()
                .text_color(t.text_3)
                .child(footer)
                .child(
                    div()
                        .text_color(t.text_4)
                        .truncate()
                        .child(data_dir.clone()),
                )
                .child(
                    div()
                        .text_color(t.text_4)
                        .child(format!("{} updates", store.updates)),
                ),
        )
}

/// `~/work/alpha` → (`alpha`, `~/work`).
fn split_display(display: &str) -> (String, String) {
    let trimmed = display.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(i) if i + 1 < trimmed.len() => {
            (trimmed[i + 1..].to_string(), trimmed[..i].to_string())
        }
        _ => (trimmed.to_string(), String::new()),
    }
}

fn row(a: &AgentView, selected: bool, t: &Theme, cx: &Context<MainView>) -> impl IntoElement {
    let id = a.id.clone();
    let color = t.status_color(a.status);
    div()
        .id(SharedString::from(format!("agent-{}", a.id)))
        .mx_1()
        .px_2()
        .h(px(30.))
        .flex()
        .items_center()
        .gap_2()
        .rounded(RADIUS)
        .cursor_pointer()
        .when(selected, |d| d.bg(t.surface_3))
        .when(!selected, |d| d.hover(|s| s.bg(t.surface_2)))
        .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
        .child(
            div()
                .w(px(16.))
                .flex()
                .justify_center()
                .text_xs()
                .text_color(color)
                .child(status_glyph(a.status)),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_sm()
                .text_color(if a.running { t.text } else { t.text_3 })
                .child(a.name.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(color)
                .child(status_word(a.status)),
        )
}

#[cfg(test)]
mod tests {
    use super::split_display;

    #[test]
    fn project_headings_lead_with_the_folder_name() {
        assert_eq!(
            split_display("~/work/alpha"),
            ("alpha".into(), "~/work".into())
        );
        assert_eq!(
            split_display("~/work/alpha/"),
            ("alpha".into(), "~/work".into())
        );
        assert_eq!(
            split_display(r"C:\work\beta"),
            ("beta".into(), r"C:\work".into())
        );
        assert_eq!(split_display("solo"), ("solo".into(), String::new()));
    }
}
