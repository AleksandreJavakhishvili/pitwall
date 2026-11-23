//! Settings → Extensions for an extension's author (docs/spec/extensions.md,
//! "Dev mode"): "Load from folder…" lists a folder as a development
//! extension, used where it is (`crate::extensions::load_dev`); its card is
//! marked "development" and has Reload (its manifest read again, its
//! adapter started again when on) and "Remove from list" (its folder is
//! never touched). Any card with an adapter has Log: the last protocol
//! lines both ways and its stderr, secrets taken out, cleared when it is
//! turned off (`hub::Hub::log`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use gpui::{div, prelude::*, px, AnyElement, Context, Div, ScrollHandle, SharedString};
use pitwall_core::extensions::log::Dir;
use pitwall_core::extensions::Manifest;

use super::view::SettingsView;
use super::widgets::Ui;
use crate::extensions::hub;
use crate::kit::BtnKind;

/// The page's state for authors.
#[derive(Default)]
pub struct Dev {
    /// Cards whose log is shown, with its scroll and the lines last drawn.
    logs: HashMap<String, (ScrollHandle, usize)>,
    /// Why the last folder couldn't be loaded.
    note: Option<String>,
    /// A folder being picked.
    picking: bool,
    /// Development cards whose Reload just ran (a word under the row).
    reloaded: HashSet<String>,
    /// Open logs redraw every second (stderr comes without a hub change).
    ticking: bool,
}

/// A log line's time of day, as the clock on this computer shows it.
fn time_of(at: std::time::SystemTime) -> String {
    let t: chrono::DateTime<chrono::Local> = at.into();
    t.format("%H:%M:%S").to_string()
}

impl SettingsView {
    /// Extension `id` is loaded from a development folder.
    pub(super) fn is_dev(id: &str, cx: &Context<Self>) -> bool {
        crate::extensions::dev_folder(id, cx).is_some()
    }

    /// "Load from folder…": the folder picker, then the folder loaded.
    pub(super) fn dev_load_button(&mut self, ui: &Ui, cx: &mut Context<Self>) -> AnyElement {
        let picking = self.exts.dev.picking;
        crate::kit::text_button("extensions-load-dev", "Load from folder…", BtnKind::GhostSm, picking, &ui.t)
            .tooltip(crate::kit::tooltip("Develop an extension: use a folder where it is, without installing it"))
            .on_click(cx.listener(|v, _, _, cx| {
                v.exts.dev.picking = true;
                cx.notify();
                let picked = crate::platform::files::pick_folder(cx, "Load extension");
                cx.spawn(async move |this, cx| {
                    let folder = picked.await;
                    let _ = this.update(cx, |v, cx| {
                        v.exts.dev.picking = false;
                        if let Some(f) = folder {
                            v.load_dev_from(f, cx);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }))
            .into_any_element()
    }

    /// Load `folder` as a development extension: its card unfolded, or
    /// why not under the page's intro.
    fn load_dev_from(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        match crate::extensions::load_dev(&folder, cx) {
            Ok(id) => {
                self.exts.dev.note = None;
                self.exts_open(&id);
            }
            Err(why) => self.exts.dev.note = Some(why),
        }
        cx.notify();
    }

    /// Bench and debug builds: `folder` loaded as from "Load from folder…"
    /// (no picker).
    pub fn debug_load_dev(&mut self, folder: &str, cx: &mut Context<Self>) {
        self.load_dev_from(PathBuf::from(folder), cx);
    }

    /// Bench and debug builds: `ext`'s card unfolded with its log shown
    /// (`on`) or hidden.
    pub fn debug_log(&mut self, ext: &str, on: bool, cx: &mut Context<Self>) {
        self.exts_open(ext);
        if on {
            self.exts.dev.logs.entry(ext.to_string()).or_insert_with(|| (ScrollHandle::new(), 0));
            self.tick_logs(cx);
        } else {
            self.exts.dev.logs.remove(ext);
        }
        cx.notify();
    }

    /// Why the last folder couldn't be loaded (under the intro).
    pub(super) fn dev_note(&self, ui: &Ui) -> Option<Div> {
        let why = self.exts.dev.note.clone()?;
        Some(div().text_size(px(12.)).text_color(ui.t.amber).child(format!("That folder wasn't loaded. {why}")))
    }

    /// A development card's mark beside its name, with its folder in the
    /// tooltip.
    pub(super) fn dev_chip(ui: &Ui, m: &Manifest, cx: &Context<Self>) -> Option<AnyElement> {
        let folder = crate::extensions::dev_folder(&m.id, cx)?;
        let tip = format!("Loaded from {} where it is: Pitwall never copies, changes or updates it", pitwall_core::paths::tildify(&folder.to_string_lossy()));
        // In a row of its own, so the chip keeps its width.
        Some(div().flex().pt(px(2.)).child(div().id(SharedString::from(format!("extension-dev-{}", m.id))).tooltip(crate::kit::tooltip(tip)).child(crate::kit::chip_tone("development", crate::kit::Tone::Subtle, &ui.t))).into_any_element())
    }

    /// The footer's buttons for authors: Log (an extension with an
    /// adapter), and for a development one Reload and "Remove from list"
    /// (in place of Remove). Whether it is a development one.
    pub(super) fn dev_buttons(&mut self, ui: &Ui, m: &Manifest, mut row: Div, cx: &mut Context<Self>) -> (Div, bool) {
        let dev = Self::is_dev(&m.id, cx);
        if m.adapter.is_some() {
            let shown = self.exts.dev.logs.contains_key(&m.id);
            let id = m.id.clone();
            row = row.child(
                crate::kit::text_button(SharedString::from(format!("extension-log-{}", m.id)), if shown { "Hide log" } else { "Log" }, BtnKind::GhostSm, false, &ui.t)
                    .tooltip(crate::kit::tooltip("What Pitwall and its adapter said to each other, and what it printed"))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        if v.exts.dev.logs.remove(&id).is_none() {
                            v.exts.dev.logs.insert(id.clone(), (ScrollHandle::new(), 0));
                            v.tick_logs(cx);
                        }
                        cx.notify();
                    })),
            );
        }
        if dev {
            let id = m.id.clone();
            row = row.child(
                crate::kit::text_button(SharedString::from(format!("extension-reload-{}", m.id)), "Reload", BtnKind::GhostSm, false, &ui.t)
                    .tooltip(crate::kit::tooltip("Read its manifest again and, while it is on, start its adapter again"))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        crate::extensions::reload_dev(&id, cx);
                        v.exts.dev.reloaded.insert(id.clone());
                        cx.notify();
                    })),
            );
            let id = m.id.clone();
            row = row.child(
                crate::kit::text_button(SharedString::from(format!("extension-forget-{}", m.id)), "Remove from list", BtnKind::GhostSm, false, &ui.t)
                    .tooltip(crate::kit::tooltip("Turn it off and stop listing it; its folder stays as it is"))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        crate::extensions::forget_dev(&id, cx);
                        v.exts.dev.logs.remove(&id);
                        v.exts.dev.reloaded.remove(&id);
                        cx.notify();
                    })),
            );
        }
        (row, dev)
    }

    /// Under a development card's top row: where it is loaded from, and
    /// that Reload ran.
    pub(super) fn dev_line(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<AnyElement> {
        let folder = crate::extensions::dev_folder(&m.id, cx)?;
        let mut text = format!("Development: loaded from {}, never copied or updated.", pitwall_core::paths::tildify(&folder.to_string_lossy()));
        if self.exts.dev.reloaded.contains(&m.id) {
            text.push_str(" Reloaded.");
        }
        Some(div().text_size(px(12.)).text_color(ui.t.text_3).child(text).into_any_element())
    }

    /// Redraw while a log is open, every second.
    fn tick_logs(&mut self, cx: &mut Context<Self>) {
        if self.exts.dev.ticking {
            return;
        }
        self.exts.dev.ticking = true;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let open = this
                .update(cx, |v, cx| {
                    let open = !v.exts.dev.logs.is_empty();
                    if open {
                        cx.notify();
                    } else {
                        v.exts.dev.ticking = false;
                    }
                    open
                })
                .unwrap_or(false);
            if !open {
                return;
            }
        })
        .detach();
    }

    /// The log, when it is shown: its lines, newest at the bottom.
    pub(super) fn log_view(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<Div> {
        let lines = hub::try_hub(cx).map(|h| h.read(cx).log(&m.id)).unwrap_or_default();
        let (handle, seen) = self.exts.dev.logs.get_mut(&m.id)?;
        if lines.len() != *seen {
            *seen = lines.len();
            handle.scroll_to_bottom();
        }
        let handle = handle.clone();
        let on = crate::ui_state::get(cx).extension_on(&m.id);
        let head = match (lines.len(), on) {
            (0, false) => "Nothing yet: turn it on and its lines show here.".to_string(),
            (0, true) => "Nothing yet.".to_string(),
            (n, _) => format!("Its last {n} lines, both ways, and what it printed (at most {}). Secrets are taken out; the log is cleared when it is turned off.", pitwall_core::extensions::log::KEEP),
        };
        let mut top = div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().text_size(px(12.)).text_color(ui.t.text_3).child(head));
        if !lines.is_empty() {
            let text = lines
                .iter()
                .map(|(conn, l)| {
                    let conn = if conn.is_empty() { String::new() } else { format!("[{conn}] ") };
                    format!("{} {} {conn}{}", time_of(l.at), mark(l.dir), l.text)
                })
                .collect::<Vec<_>>()
                .join("\n");
            top = top.child(crate::kit::text_button(SharedString::from(format!("extension-log-copy-{}", m.id)), "Copy", BtnKind::GhostSm, false, &ui.t).on_click(move |_, _, cx| crate::platform::files::copy_text(cx, text.clone())));
        }
        let mut list = div().id(SharedString::from(format!("extension-log-lines-{}", m.id))).track_scroll(&handle).max_h(px(280.)).overflow_y_scroll().flex().flex_col().gap(px(2.)).px(px(10.)).py(px(8.)).rounded(px(8.)).bg(ui.t.surface_2);
        for (conn, l) in &lines {
            let color = match l.dir {
                Dir::ToAdapter => ui.t.text_2,
                Dir::FromAdapter => ui.t.text,
                Dir::Stderr => ui.t.amber,
                Dir::Note => ui.t.text_3,
            };
            let mut row = div().flex().gap(px(8.)).items_start().font_family(crate::kit::fonts::MONO_FONT).text_size(px(11.5)).line_height(gpui::relative(1.4));
            row = row.child(div().flex_none().text_color(ui.t.text_4).child(time_of(l.at)));
            row = row.child(div().flex_none().w(px(14.)).text_color(ui.t.text_3).child(mark(l.dir)));
            if !conn.is_empty() {
                row = row.child(div().flex_none().text_color(ui.t.text_3).child(conn.clone()));
            }
            row = row.child(div().flex_1().min_w_0().text_color(color).child(l.text.clone()));
            list = list.child(row);
        }
        Some(div().flex().flex_col().gap(px(6.)).child(top).when(!lines.is_empty(), |d| d.child(list)))
    }
}

/// A line's direction as a mark: to the adapter, from it, its stderr, or
/// Pitwall's own word.
fn mark(d: Dir) -> &'static str {
    match d {
        Dir::ToAdapter => "→",
        Dir::FromAdapter => "←",
        Dir::Stderr => "!",
        Dir::Note => "·",
    }
}
