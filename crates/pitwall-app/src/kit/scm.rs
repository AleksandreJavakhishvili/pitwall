//! Source-control marks of file rows (`review.css`, `panel.css`): the
//! VS Code status letter and the +/− counts of one file (only the sides
//! that changed). The agent-level diffstat is [`super::diffstat`].

use gpui::{div, prelude::*, px, Div, FontWeight};

use pitwall_proto::FileStatus;

use crate::theme::Theme;

use super::fonts::MONO_FONT;

/// The letter VS Code shows for a status.
pub fn status_letter_text(s: FileStatus) -> &'static str {
    match s {
        FileStatus::M => "M",
        FileStatus::A => "A",
        FileStatus::D => "D",
        FileStatus::R => "R",
        FileStatus::U => "U",
    }
}

/// What a changed file's status is (untracked counts as U; no status as
/// modified), as the React app's `fileStatus`.
pub fn file_status(f: &pitwall_core::vcs::git::FileChange) -> FileStatus {
    f.status.unwrap_or(if f.untracked {
        FileStatus::U
    } else {
        FileStatus::M
    })
}

/// The tooltip of a status letter.
pub fn status_title(s: FileStatus) -> &'static str {
    match s {
        FileStatus::M => "Modified",
        FileStatus::A => "Added",
        FileStatus::D => "Deleted",
        FileStatus::R => "Renamed",
        FileStatus::U => "Untracked",
    }
}

/// `.status-letter`: A and U green, D red, the others text-2.
pub fn status_letter(s: FileStatus, t: &Theme) -> Div {
    let color = match s {
        FileStatus::A | FileStatus::U => t.green,
        FileStatus::D => t.red,
        _ => t.text_2,
    };
    div()
        .w(px(10.))
        .flex_none()
        .text_center()
        .font_family(MONO_FONT)
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .line_height(px(11.))
        .text_color(color)
        .child(status_letter_text(s))
}

/// The counts in a file row: only the non-zero sides.
pub fn diffstat_inline(added: u32, removed: u32, t: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(6.))
        .font_family(MONO_FONT)
        .text_size(px(11.))
        .when(added > 0, |d| {
            d.child(div().text_color(t.green).child(format!("+{added}")))
        })
        .when(removed > 0, |d| {
            d.child(div().text_color(t.red).child(format!("−{removed}")))
        })
}

/// `.rv-ccount`: a small count badge (comments on a file).
pub fn count_badge(n: usize, t: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(px(16.))
        .h(px(16.))
        .px(px(4.))
        .rounded(px(8.))
        .font_family(MONO_FONT)
        .text_size(px(10.))
        .text_color(t.text)
        .bg(t.surface_3)
        .border_1()
        .border_color(t.line_strong)
        .child(n.to_string())
}
