//! Liquid Glass regions: chrome (sidebar, top bar, right panel) that gets
//! its own native glass piece under macOS 26's Liquid Glass, following the
//! region's bounds each frame (`platform::glass::sync_region`). Elsewhere,
//! and with Flat, these are just their child.

use gpui::{canvas, div, prelude::*, px, AnyElement, App, Bounds, IntoElement, Pixels};

use crate::theme::{appearance, glass_regions, GlassTier};

/// Corner radius of floating glass panels (macOS 26 source lists).
pub const PANEL_RADIUS: f32 = 14.;

/// A chrome region with its own glass piece under Liquid Glass. Draw the
/// region translucent (`Theme::chrome`); size the returned box like the
/// region (`size_full`, `w_full`, …).
pub fn glass_region(id: &'static str, radius: f32, child: impl IntoElement) -> gpui::Div {
    div()
        .relative()
        .child(
            canvas(
                move |bounds: Bounds<Pixels>, window, cx| {
                    if appearance(cx).glass == Some(GlassTier::Liquid) {
                        crate::platform::glass::sync_region(window, id, bounds, radius);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(child)
}

/// A chrome panel (the sidebar, the right panel): under Liquid Glass with
/// regions, a floating rounded glass slab inset from the window edges, like
/// a macOS 26 source list; otherwise just `child`. Draw `child` with
/// [`crate::theme::panel`]'s theme so its own background stays clear.
pub fn chrome_panel(id: &'static str, child: impl IntoElement, cx: &App) -> AnyElement {
    if !glass_regions(cx) {
        return child.into_any_element();
    }
    let line = appearance(cx).glass_tokens.line_strong;
    div()
        .h_full()
        .flex_none()
        .p(px(8.))
        .child(
            glass_region(
                id,
                PANEL_RADIUS,
                div()
                    .h_full()
                    .rounded(px(PANEL_RADIUS))
                    .border_1()
                    .border_color(line)
                    .overflow_hidden()
                    .child(child),
            )
            .h_full(),
        )
        .into_any_element()
}

/// Drops the native glass pieces no region placed this frame. Paint it
/// last in the window (after every region).
pub fn region_sweeper() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| crate::platform::glass::sweep_regions(window),
    )
    .absolute()
    .size_0()
}
