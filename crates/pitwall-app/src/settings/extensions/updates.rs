//! Settings → Extensions, updates (docs/spec/extensions.md, "Updates from
//! a git source", "Updates from an npm package"): on a card, where the
//! extension came from (Pitwall's copy, a source's tag and commit, or a
//! package's version) and when its source was last checked, with Check
//! for updates; an update offered (from Pitwall or its source), looked at
//! first: "Review…" fetches it and lists what changes on this computer,
//! and only Update installs it; Go back to the copy before the last
//! update. Under Available, "Add from Git or npm…": an https address and
//! the folder in it, or an npm package and its registry, then what its
//! newest release changes, and Install.

use gpui::{div, prelude::*, px, AnyElement, Context, Div, SharedString};
use pitwall_core::extensions::install::{self, Change, Origin};
use pitwall_core::extensions::Manifest;

use super::super::view::SettingsView;
use super::super::widgets::Ui;
use super::{AddGit, Shipped};
use crate::extensions::catalog::{self, Prepared};
use crate::kit::{BtnKind, InputEvent, TextInput, Tone};

/// How long ago `at` was, in a few words ("just now", "5 min ago").
pub fn ago(at: std::time::SystemTime) -> String {
    let s = at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

/// A text button, found by its id in tests too.
fn btn(id: String, text: impl Into<SharedString>, kind: BtnKind, disabled: bool, ui: &Ui) -> gpui::Stateful<Div> {
    let sel = id.clone();
    crate::kit::text_button(SharedString::from(id), text, kind, disabled, &ui.t).debug_selector(move || sel)
}

/// What a change list says: each change's title, then its lines in mono.
pub fn changes_view(ui: &Ui, changes: &[Change]) -> Div {
    let mut col = div().flex().flex_col().gap(px(8.));
    for c in changes {
        let mut b = div().flex().flex_col().gap(px(3.)).child(div().text_size(px(12.)).text_color(ui.t.text_2).child(c.title.clone()));
        for l in &c.lines {
            b = b.child(crate::kit::mono_text(l.clone(), &ui.t));
        }
        col = col.child(b);
    }
    col
}

impl SettingsView {
    /// Where each installed extension came from and the version Go back
    /// returns to (read again when the extensions folder is).
    pub(super) fn origins_now(&mut self, cx: &mut Context<Self>) -> super::Origins {
        let found = crate::extensions::found(cx);
        let key = std::rc::Rc::as_ptr(&found) as usize;
        if self.exts.origins.as_ref().is_none_or(|(k, _)| *k != key) {
            let dir = crate::extensions::dir(cx);
            let map = found
                .iter()
                .filter_map(|f| {
                    let m = f.manifest.as_ref().ok()?;
                    let data = dir.as_ref()?;
                    let back = install::previous(data, &m.id).map(|p| p.version.unwrap_or_else(|| "the earlier copy".into()));
                    Some((m.id.clone(), (install::origin(&data.join(&m.id)), back)))
                })
                .collect();
            self.exts.origins = Some((key, map));
        }
        self.exts.origins.as_ref().map(|(_, o)| o.clone()).unwrap_or_default()
    }

    /// Under a card's top row: where it came from and its last check; an
    /// update offered, with Review…, then what it changes and Update; a
    /// moved tag or a failed check; Go back asked about a changed folder.
    pub(super) fn update_notes(&mut self, ui: &Ui, m: &Manifest, shipped: Option<&Shipped>, changed: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        // A development folder is the author's: no source, no updates
        // (its own line says where it is loaded from).
        if Self::is_dev(&m.id, cx) {
            return out;
        }
        let (origin, _) = self.origins_now(cx).remove(&m.id).unwrap_or_default();
        let u = catalog::updates(cx);
        let (check, checking) = {
            let u = u.read(cx);
            (u.check(&m.id).cloned(), u.checking(&m.id))
        };
        // Where it came from, and its source's last check.
        let mut parts: Vec<String> = Vec::new();
        match &origin {
            Some(o @ (Origin::Git { .. } | Origin::Npm { .. })) => parts.push(format!("From {}", o.describe())),
            Some(Origin::Bundled) => parts.push("Comes with Pitwall".into()),
            None => parts.push("Put here by hand".into()),
        }
        if let Some(src) = &m.source {
            if !origin.as_ref().is_some_and(|o| o.is_from(src)) {
                parts.push(format!("updates from {}", install::source_place(src)));
            }
            parts.push(match (&check, checking) {
                (_, true) => "checking…".into(),
                (Some(c), _) => format!("checked {}", ago(c.at)),
                (None, _) => "not checked yet".into(),
            });
        }
        let mut line = div().flex().flex_wrap().items_center().gap(px(8.)).child(div().flex_1().min_w(px(160.)).text_size(px(12.)).text_color(ui.t.text_3).child(parts.join(" · ")));
        if m.source.is_some() {
            let id = m.id.clone();
            line = line.child(
                btn(format!("extension-check-{}", m.id), if checking { "Checking…" } else { "Check for updates" }, BtnKind::GhostSm, checking, ui)
                    .tooltip(crate::kit::tooltip("Look for a newer release of it now (nothing is installed)"))
                    .on_click(cx.listener(move |_, _, _, cx| catalog::check_one(&id, cx))),
            );
        }
        out.push(line.into_any_element());
        let warn = |text: String| div().text_size(px(12.)).text_color(ui.t.amber).child(text).into_any_element();
        if let Some(c) = &check {
            if let Err(why) = &c.newest {
                out.push(warn(format!("Couldn't check {}: {why}", c.place)));
            }
            if let Some(why) = &c.moved {
                out.push(warn(why.clone()));
            }
        }
        // The update offered: the newest of Pitwall's copy and its source.
        let offer = catalog::update_offer(m, origin.as_ref(), shipped, cx);
        let working = self.exts.working.contains(&m.id);
        if let Some(offer) = offer {
            let v = offer.manifest.version.clone().unwrap_or_default();
            let from = match &offer.origin {
                Origin::Bundled => "with this Pitwall".to_string(),
                o => format!("from {}", o.place()),
            };
            let preparing = self.exts.preparing.contains(&m.id);
            let prepared = self.exts.prepared.get(&m.id).filter(|p| p.offer.manifest.version.as_deref() == Some(v.as_str())).cloned();
            let mut row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(ui.t.surface_2)
                .child(div().flex_1().min_w(px(160.)).text_size(px(12.)).text_color(ui.t.text_2).child(format!("Version {v} is available {from}. Its settings and secrets stay.")));
            if changed {
                row = row.child(div().id(SharedString::from(format!("extension-changed-{}", m.id))).tooltip(crate::kit::tooltip("Its folder isn't as Pitwall installed it: Update asks before replacing it.")).child(crate::kit::chip_tone("changed locally", Tone::Warn, &ui.t)));
            }
            if prepared.is_none() {
                let (id, o) = (m.id.clone(), offer.clone());
                let text = if preparing { "Fetching…" } else { "Review…" };
                row = row.child(
                    btn(format!("extension-update-{}", m.id), text, BtnKind::Small, preparing || working, ui)
                        .tooltip(crate::kit::tooltip("See what it changes on this computer first; nothing is installed until you press Update"))
                        .on_click(cx.listener(move |v, _, _, cx| v.review_update(&id, o.clone(), cx))),
                );
            }
            out.push(row.into_any_element());
            if let Some(p) = prepared {
                out.push(self.review_card(ui, m, &p, working, cx).into_any_element());
            }
        }
        if self.exts.confirm_back.as_deref() == Some(&m.id) {
            let text = format!("{}'s folder was changed since Pitwall installed it. Going back replaces it, and those changes are lost. Its settings and secrets stay.", m.name);
            let id = m.id.clone();
            let cancel = self.cancel_button(ui, &m.id, cx);
            let yes = btn(format!("extension-back-yes-{}", m.id), "Go back", BtnKind::Small, false, ui).on_click(cx.listener(move |v, _, _, cx| v.start_back(&id, true, cx)));
            out.push(
                div()
                    .id(SharedString::from(format!("extension-ask-back-{}", m.id)))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(ui.t.line_strong)
                    .bg(ui.t.surface_2)
                    .child(div().text_size(px(13.)).text_color(ui.t.text).child(text))
                    .child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).child(yes))
                    .into_any_element(),
            );
        }
        out
    }

    /// Cancel on a question under a card: every question there closes.
    pub(super) fn cancel_button(&mut self, ui: &Ui, id: &str, cx: &mut Context<Self>) -> AnyElement {
        btn(format!("extension-ask-cancel-{id}"), "Cancel", BtnKind::GhostSm, false, ui)
            .on_click(cx.listener(|v, _, _, cx| {
                v.exts.confirm_remove = None;
                v.exts.confirm_replace = None;
                v.exts.confirm_back = None;
                cx.notify();
            }))
            .into_any_element()
    }

    /// The update looked at: what changes on this computer, then Cancel
    /// and Update.
    fn review_card(&mut self, ui: &Ui, m: &Manifest, p: &Prepared, working: bool, cx: &mut Context<Self>) -> Div {
        let v = p.offer.manifest.version.clone().unwrap_or_default();
        let only_files = p.changes.len() <= 1;
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(ui.t.line_strong)
            .bg(ui.t.surface_2)
            .child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(format!("Update {} to {v}: what changes on this computer", m.name)))
            .child(changes_view(ui, &p.changes));
        if only_files {
            card = card.child(crate::kit::hint("Nothing else changes on this computer: only its files.", &ui.t));
        }
        card = card.child(crate::kit::hint("Updating runs nothing by itself. If it is on, its program starts again with the new files.", &ui.t));
        let id = m.id.clone();
        let cancel = btn(format!("extension-review-cancel-{}", m.id), "Cancel", BtnKind::GhostSm, working, ui).on_click(cx.listener(move |v, _, _, cx| {
            if let Some(p) = v.exts.prepared.remove(&id) {
                catalog::discard(&p, cx);
            }
            cx.notify();
        }));
        let id = m.id.clone();
        let yes = btn(format!("extension-review-update-{}", m.id), if working { "Updating…".to_string() } else { format!("Update to {v}") }, BtnKind::Small, working, ui).on_click(cx.listener(move |v, _, _, cx| v.start_update(&id, false, cx)));
        card.child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).child(yes))
    }

    /// Fetch `offer` (a source's release; Pitwall's copy needs no fetch)
    /// and list what it changes.
    pub(super) fn review_update(&mut self, id: &str, offer: pitwall_core::extensions::install::Offer, cx: &mut Context<Self>) {
        if !self.exts.preparing.insert(id.to_string()) {
            return;
        }
        self.exts.failed.remove(id);
        cx.notify();
        let (me, id2) = (cx.entity().downgrade(), id.to_string());
        catalog::prepare(id, offer, cx, move |res, cx| {
            let mut res = Some(res);
            let _ = me.update(cx, |v, cx| {
                v.exts.preparing.remove(&id2);
                match res.take() {
                    Some(Ok(p)) => {
                        if let Some(old) = v.exts.prepared.insert(id2.clone(), p) {
                            catalog::discard(&old, cx);
                        }
                    }
                    Some(Err(why)) => {
                        v.exts.failed.insert(id2.clone(), why);
                    }
                    None => {}
                }
                cx.notify();
            });
            // Settings closed meanwhile: what was fetched goes.
            if let Some(Ok(p)) = res {
                catalog::discard(&p, cx);
            }
        });
    }

    /// "Go back to 1.0.0" in a card's footer, when the last update kept the
    /// copy before it.
    pub(super) fn go_back_button(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<AnyElement> {
        let back = self.origins_now(cx).remove(&m.id).and_then(|(_, b)| b)?;
        let working = self.exts.working.contains(&m.id);
        let id = m.id.clone();
        Some(
            btn(format!("extension-back-{}", m.id), format!("Go back to {back}"), BtnKind::GhostSm, working, ui)
                .tooltip(crate::kit::tooltip("Put back the copy from before the last update; its settings and secrets stay"))
                .on_click(cx.listener(move |v, _, _, cx| v.start_back(&id, false, cx)))
                .into_any_element(),
        )
    }

    pub(super) fn start_back(&mut self, id: &str, replace_changed: bool, cx: &mut Context<Self>) {
        self.exts.working.insert(id.to_string());
        self.exts.failed.remove(id);
        self.exts.confirm_back = None;
        cx.notify();
        let (me, id2) = (cx.entity().downgrade(), id.to_string());
        catalog::rollback_one(id, replace_changed, cx, move |o, cx| {
            let _ = me.update(cx, |v, cx| {
                if o == catalog::Outcome::ChangedLocally {
                    v.exts.working.remove(&id2);
                    v.exts.confirm_back = Some(id2.clone());
                    cx.notify();
                } else {
                    v.ext_done(&id2, o, cx);
                }
            });
        });
    }

    /// The "Available" heading with "Add from Git or npm…", and its form
    /// or what the release it found changes.
    pub(super) fn add_git_view(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let open = self.exts.add_git.is_some();
        let head = div().flex().items_center().pt(px(8.)).child(crate::kit::label("Available", ui.t.text_3, 11.)).child(div().flex_1()).child(
            btn("extension-add-git".to_string(), "Add from Git or npm…", BtnKind::GhostSm, open, ui)
                .tooltip(crate::kit::tooltip("Install an extension from a git repository's release tags or an npm package's versions"))
                .on_click(cx.listener(|v, _, w, cx| v.open_add_git(w, cx))),
        );
        let mut col = div().flex().flex_col().gap(px(8.)).child(head);
        let Some(add) = self.exts.add_git.as_ref() else { return col };
        let (npm, url, sub, package, registry, busy, note, prepared) = (add.npm, add.url.clone(), add.dir.clone(), add.package.clone(), add.registry.clone(), add.busy, add.note.clone(), add.prepared.clone());
        let mut card = div().id("extension-add-git-card").flex().flex_col().gap(px(10.)).p(px(14.)).rounded(px(10.)).border_1().border_color(ui.t.line_strong).bg(ui.t.surface_2);
        if let Some(p) = prepared {
            let m = &p.offer.manifest;
            let (icon, text) = Self::ext_title(ui, m, &m.id);
            card = card.child(div().flex().gap(px(12.)).items_start().child(icon).child(text));
            card = card.child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child("What installing it changes on this computer"));
            card = card.child(changes_view(ui, &p.changes));
            let replacing = p.replaces.is_some();
            let note = match &p.replaces {
                Some(v) => format!("It replaces the installed {} {}; the installed copy is kept for Go back. Its settings and secrets stay.", m.name, v.clone().unwrap_or_default()),
                None => "Installing runs nothing: it stays off until you turn it on.".into(),
            };
            card = card.child(crate::kit::hint(note, &ui.t));
            let cancel = btn("extension-add-git-cancel".to_string(), "Cancel", BtnKind::GhostSm, busy, ui).on_click(cx.listener(|v, _, _, cx| v.close_add_git(cx)));
            let yes = btn("extension-add-git-install".to_string(), if busy { "Installing…" } else if replacing { "Replace" } else { "Install" }, BtnKind::Small, busy, ui).on_click(cx.listener(|v, _, _, cx| v.install_from_git(cx)));
            card = card.child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).child(yes));
        } else {
            let field = |title: &str, hint: &str, input: gpui::Entity<TextInput>| div().flex().flex_col().gap(px(4.)).child(Self::step_title(ui, title.to_string())).child(div().max_w(px(520.)).child(input)).child(crate::kit::hint(hint.to_string(), &ui.t));
            // Where from: a git repository or an npm package.
            let mut kinds = div().flex().flex_wrap().gap(px(6.));
            for (is_npm, label) in [(false, "Git repository"), (true, "npm package")] {
                let on = npm == is_npm;
                let id = format!("extension-add-kind-{}", if is_npm { "npm" } else { "git" });
                let sel = id.clone();
                let b = div().id(SharedString::from(id)).debug_selector(move || sel).h(px(26.)).px(px(10.)).flex().items_center().rounded(px(6.)).border_1().cursor_pointer().text_size(px(12.)).child(label);
                let b = if on { b.bg(ui.t.surface_3).border_color(ui.t.line_strong).text_color(ui.t.text).font_weight(crate::kit::WEIGHT_550) } else { b.border_color(ui.t.line).text_color(ui.t.text_2).hover(|d| d.bg(ui.t.surface_2)) };
                kinds = kinds.child(b.on_click(cx.listener(move |v, _, w, cx| v.pick_add_kind(is_npm, w, cx))));
            }
            card = card.child(div().flex().flex_col().gap(px(5.)).child(Self::step_title(ui, "From")).child(kinds));
            card = if npm {
                card.child(field("Package", "Its name on npm, like my-ext or @me/my-ext.", package))
                    .child(field("Registry (optional)", "An https address. Empty: npm's own (registry.npmjs.org). Pitwall sends no sign-in.", registry))
                    .child(crate::kit::hint("Pitwall takes its newest version, checks the download against the registry's sha512 integrity and shows what it changes before anything is installed. Nothing in the package runs, and its dependencies aren't installed. Not reviewed by Pitwall.", &ui.t))
            } else {
                card.child(field("Repository", "Its https address. A private one works through your git sign-in; Pitwall never asks for a password.", url))
                    .child(field("Folder in it (optional)", "Where its extension.json is, like extensions/my-ext. Empty: the repository's root.", sub))
                    .child(crate::kit::hint("Pitwall takes its newest release tag (like v1.0.0) and shows what it changes before anything is installed. Not reviewed by Pitwall.", &ui.t))
            };
            let cancel = btn("extension-add-git-cancel".to_string(), "Cancel", BtnKind::GhostSm, false, ui).on_click(cx.listener(|v, _, _, cx| v.close_add_git(cx)));
            let look = btn("extension-add-git-look".to_string(), if busy { "Fetching…" } else { "Look" }, BtnKind::Small, busy, ui).on_click(cx.listener(|v, _, _, cx| v.look_add_git(cx)));
            card = card.child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).child(look));
        }
        if let Some(n) = note {
            card = card.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n));
        }
        col = col.child(card);
        col
    }

    fn open_add_git(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let url = cx.new(|cx| TextInput::new(cx, "", "https://github.com/you/your-extension").mono());
        let sub = cx.new(|cx| TextInput::new(cx, "", "extensions/my-ext").mono());
        let package = cx.new(|cx| TextInput::new(cx, "", "@you/your-extension").mono());
        let registry = cx.new(|cx| TextInput::new(cx, "", pitwall_core::extensions::NPM_REGISTRY).mono());
        url.update(cx, |i, cx| i.focus(window, cx));
        for input in [&url, &sub, &package, &registry] {
            let s = cx.subscribe(input, |v, _, e: &InputEvent, cx| match e {
                InputEvent::Submit => v.look_add_git(cx),
                InputEvent::Cancel => v.close_add_git(cx),
                _ => {}
            });
            self.exts._subs.push(s);
        }
        self.exts.add_git = Some(AddGit { npm: false, url, dir: sub, package, registry, busy: false, note: None, prepared: None });
        cx.notify();
    }

    /// The form asks for a git repository, or an npm package (`npm`); the
    /// first field of that kind is focused.
    fn pick_add_kind(&mut self, npm: bool, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(add) = self.exts.add_git.as_mut() else { return };
        if add.busy || add.npm == npm {
            return;
        }
        add.npm = npm;
        add.note = None;
        let first = if npm { add.package.clone() } else { add.url.clone() };
        first.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    fn close_add_git(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.exts.add_git.take().and_then(|a| a.prepared) {
            catalog::discard(&p, cx);
        }
        cx.notify();
    }

    /// The address or package typed: its newest release fetched and looked
    /// at.
    fn look_add_git(&mut self, cx: &mut Context<Self>) {
        let Some(add) = self.exts.add_git.as_mut() else { return };
        if add.busy || add.prepared.is_some() {
            return;
        }
        let npm = add.npm;
        let (a, b) = if npm { (add.package.read(cx).text().to_string(), add.registry.read(cx).text().to_string()) } else { (add.url.read(cx).text().to_string(), add.dir.read(cx).text().to_string()) };
        add.busy = true;
        add.note = None;
        cx.notify();
        let me = cx.entity().downgrade();
        let done = move |res: Result<Prepared, String>, cx: &mut gpui::App| {
            let mut res = Some(res);
            let _ = me.update(cx, |v, cx| {
                let Some(add) = v.exts.add_git.as_mut() else { return };
                add.busy = false;
                match res.take() {
                    Some(Ok(p)) => add.prepared = Some(p),
                    Some(Err(why)) => add.note = Some(why),
                    None => {}
                }
                cx.notify();
            });
            // Closed meanwhile: what was fetched goes.
            if let Some(Ok(p)) = res {
                catalog::discard(&p, cx);
            }
        };
        if npm {
            catalog::prepare_from_npm(&a, &b, cx, done);
        } else {
            catalog::prepare_from_git(&a, &b, cx, done);
        }
    }

    /// Install (or replace with) the release looked at.
    fn install_from_git(&mut self, cx: &mut Context<Self>) {
        let Some(add) = self.exts.add_git.as_mut() else { return };
        let Some(p) = add.prepared.clone() else { return };
        add.busy = true;
        add.note = None;
        cx.notify();
        let me = cx.entity().downgrade();
        let id = p.offer.manifest.id.clone();
        catalog::apply(&p, true, cx, move |o, cx| {
            let _ = me.update(cx, |v, cx| {
                match o {
                    catalog::Outcome::Done => {
                        v.exts.add_git = None;
                        v.exts.open.insert(id.clone(), true);
                    }
                    catalog::Outcome::Failed(why) => {
                        if let Some(add) = v.exts.add_git.as_mut() {
                            add.busy = false;
                            add.prepared = None;
                            add.note = Some(why);
                        }
                    }
                    catalog::Outcome::ChangedLocally => {}
                }
                cx.notify();
            });
        });
    }

    /// Bench and debug builds: extension `id`'s source checked now, as its
    /// button does.
    pub fn debug_check(&mut self, id: &str, cx: &mut Context<Self>) {
        catalog::check_one(id, cx);
    }

    /// Bench and debug builds: extension `id`'s update looked at, as
    /// Review… does (fetched; nothing installed).
    pub fn debug_review(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(m) = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == id).cloned() else { return };
        let (shipped, _) = self.shipped_now(cx);
        let origin = self.origins_now(cx).remove(id).and_then(|(o, _)| o);
        if let Some(offer) = catalog::update_offer(&m, origin.as_ref(), shipped.iter().find(|s| s.offer.manifest.id == id), cx) {
            self.exts.open.insert(id.to_string(), true);
            self.review_update(id, offer, cx);
        }
    }

    /// Bench and debug builds: the update looked at for `id` installed, as
    /// its Update button does.
    pub fn debug_update(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.exts.prepared.contains_key(id) {
            self.start_update(id, false, cx);
        }
    }

    /// Bench and debug builds: the release "Add from Git…" looked at
    /// installed, as its Install button does.
    pub fn debug_add_git_install(&mut self, cx: &mut Context<Self>) {
        self.install_from_git(cx);
    }

    /// Bench and debug builds: "Add from Git…" open with `url` and folder
    /// `sub` typed, and looked at when `look` (fetched; nothing installed).
    pub fn debug_add_git(&mut self, url: &str, sub: &str, look: bool, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.open_add_git(window, cx);
        if let Some(add) = self.exts.add_git.as_ref() {
            let (u, d) = (add.url.clone(), add.dir.clone());
            u.update(cx, |i, cx| i.set_text(url, cx));
            d.update(cx, |i, cx| i.set_text(sub, cx));
        }
        if look {
            self.look_add_git(cx);
        }
    }

    /// Bench and debug builds: "Add from Git or npm…" open on an npm
    /// package, with `package` and `registry` typed, and looked at when
    /// `look` (fetched; nothing installed).
    pub fn debug_add_npm(&mut self, package: &str, registry: &str, look: bool, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.open_add_git(window, cx);
        self.pick_add_kind(true, window, cx);
        if let Some(add) = self.exts.add_git.as_ref() {
            let (p, r) = (add.package.clone(), add.registry.clone());
            p.update(cx, |i, cx| i.set_text(package, cx));
            r.update(cx, |i, cx| i.set_text(registry, cx));
        }
        if look {
            self.look_add_git(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
    use pitwall_core::extensions::install::{self, Origin};
    use pitwall_core::testing::{extension_release, TempDir};
    use pitwall_proto::settings::Page;

    use super::super::super::view::SettingsView;
    use crate::extensions::catalog;

    fn frame(cx: &mut VisualTestContext) {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    fn click(cx: &mut VisualTestContext, id: &'static str) {
        frame(cx);
        let b = cx.debug_bounds(id).unwrap_or_else(|| panic!("{id} isn't drawn"));
        // The pointer moves there first: the last button's tooltip goes.
        cx.simulate_mouse_move(b.center(), None, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_click(b.center(), Modifiers::none());
        cx.run_until_parked();
        frame(cx);
    }

    /// The page scrolled to its end (the window's page area is shorter
    /// than the window).
    fn to_end(view: &Entity<SettingsView>, cx: &mut VisualTestContext) {
        view.update(cx, |v, cx| {
            v.scroll_to_end();
            cx.notify();
        });
        frame(cx);
    }

    fn setup_action(pin: &str) -> String {
        format!(r#"[{{"id": "setup", "setup": true, "label": "Set up", "description": "d", "steps": [{{"run": ["tool", "add", "x#{pin}"]}}]}}]"#)
    }

    /// Settings → Extensions with extension "inbox" 1.0.0 installed from
    /// a local repository standing in for its https source.
    fn open<'a>(cx: &'a mut TestAppContext, t: &TempDir) -> (Entity<SettingsView>, &'a mut VisualTestContext) {
        let (repo, root) = (t.path().join("repo"), t.path().join("data"));
        let c1 = extension_release(&repo, "inbox", "1.0.0", "// 1", &setup_action("v1"), false);
        let data = root.join("extensions");
        let ext = repo.join("ext");
        let manifest = pitwall_core::extensions::parse(&std::fs::read_to_string(ext.join("extension.json")).unwrap()).unwrap();
        let origin = Origin::Git { url: "https://example.com/r".into(), dir: "ext".into(), tag: "ext-v1.0.0".into(), commit: c1 };
        install::install(&install::Offer { manifest, folder: ext, origin }, &data).unwrap();
        cx.update(|cx| {
            cx.set_global(crate::theme::Theme::dark());
            crate::kit::init(cx);
            crate::ui_state::init(None, cx);
            crate::extensions::init(Some(root.clone()), cx);
            catalog::set_bundle(None, cx);
            cx.set_global(catalog::GitRedirect(HashMap::from([("https://example.com/r".to_string(), repo.to_string_lossy().into_owned())])));
            crate::settings::data::set_page(cx, Page::Extensions);
        });
        let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
        cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(1800.)));
        frame(cx);
        (view, cx)
    }

    /// The card says where it came from; a newer release is found by
    /// Check for updates, looked at (what changes) before Update installs
    /// it, and Go back returns the copy before.
    #[gpui::test]
    fn an_update_from_a_source_is_looked_at_before_it_is_installed(cx: &mut TestAppContext) {
        let t = TempDir::new("settings-ext-updates");
        let (view, cx) = open(cx, &t);
        let data = t.path().join("data/extensions");
        assert!(cx.debug_bounds("extension-check-inbox").is_some(), "Check for updates, for one with a source");
        assert!(cx.debug_bounds("extension-update-inbox").is_none(), "nothing to update yet");
        assert!(cx.debug_bounds("extension-add-git").is_some(), "Add from Git…, with nothing shipped");
        extension_release(&t.path().join("repo"), "inbox", "1.1.0", "// 2", &setup_action("v2"), true);
        click(cx, "extension-check-inbox");
        // Review… fetches and lists what changes; nothing is installed.
        click(cx, "extension-update-inbox");
        let p = view.read_with(cx, |v, _| v.exts.prepared.get("inbox").cloned()).expect("looked at");
        assert!(p.changes.iter().any(|c| c.lines == ["tool add x#v2"]));
        assert!(cx.debug_bounds("extension-review-update-inbox").is_some(), "Update, under what changes");
        assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 1");
        to_end(&view, cx);
        click(cx, "extension-review-update-inbox");
        assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 2");
        assert!(!p.offer.folder.exists(), "the fetched folder is gone");
        assert!(cx.debug_bounds("extension-update-inbox").is_none(), "up to date");
        // Go back, in the card's footer (unfolded).
        view.update(cx, |v, cx| {
            v.exts_open("inbox");
            cx.notify();
        });
        frame(cx);
        to_end(&view, cx);
        // The footer's buttons (Log, Go back, Remove) stay one row: beside
        // "What this extension does", or all of them under it when the
        // page is narrow.
        for width in [1400., 760., 560.] {
            cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(1800.)));
            to_end(&view, cx);
            let (what, buttons, back) = (cx.debug_bounds("extension-what-inbox").unwrap(), cx.debug_bounds("extension-footer-inbox").unwrap(), cx.debug_bounds("extension-back-inbox").unwrap());
            assert!(buttons.size.height < gpui::px(40.), "{width}: one row of buttons, {buttons:?}");
            assert!(back.origin.y >= buttons.origin.y && back.bottom() <= buttons.bottom(), "{width}: Go back is in it");
            assert!(what.size.height < gpui::px(24.), "{width}: its words on one line, {what:?}");
            let beside = buttons.origin.y < what.bottom();
            assert!(beside || buttons.origin.y >= what.bottom(), "{width}");
            if width == 1400. {
                assert!(beside, "room for both on one line");
            }
            if width == 560. {
                assert!(!beside, "too narrow: the buttons under it");
            }
        }
        cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(1800.)));
        to_end(&view, cx);
        click(cx, "extension-back-inbox");
        assert_eq!(std::fs::read_to_string(data.join("inbox/adapter/main.mjs")).unwrap(), "// 1");
        assert!(cx.debug_bounds("extension-back-inbox").is_none(), "one step back only");
        assert!(cx.debug_bounds("extension-update-inbox").is_some(), "1.1.0 offered again");
    }

    /// The card's footer buttons stay inside the card however narrow the
    /// page is: they wrap onto more rows rather than run out of it.
    #[gpui::test]
    fn the_footer_buttons_stay_inside_a_narrow_card(cx: &mut TestAppContext) {
        let t = TempDir::new("settings-ext-footer");
        let (view, cx) = open(cx, &t);
        view.update(cx, |v, cx| {
            v.exts_open("inbox");
            cx.notify();
        });
        for width in [1400., 560., 440., 380., 330.] {
            cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(1800.)));
            to_end(&view, cx);
            let (what, buttons) = (cx.debug_bounds("extension-what-inbox").unwrap(), cx.debug_bounds("extension-footer-inbox").unwrap());
            assert!(buttons.origin.x >= what.origin.x - gpui::px(1.), "{width}: the buttons start inside the card, {buttons:?} against {what:?}");
            assert!(buttons.right() <= gpui::px(width), "{width}: and end inside the window, {buttons:?}");
        }
    }

    /// Update of a folder changed by hand asks first (the answer comes
    /// back while the click is still being handled), and Replace updates.
    #[gpui::test]
    fn updating_a_folder_changed_by_hand_asks_first(cx: &mut TestAppContext) {
        let t = TempDir::new("settings-ext-changed");
        let (view, cx) = open(cx, &t);
        let file = t.path().join("data/extensions/inbox/adapter/main.mjs");
        std::fs::write(&file, "// mine").unwrap();
        extension_release(&t.path().join("repo"), "inbox", "1.1.0", "// 2", &setup_action("v2"), true);
        click(cx, "extension-check-inbox");
        click(cx, "extension-update-inbox");
        to_end(&view, cx);
        click(cx, "extension-review-update-inbox");
        assert_eq!(view.read_with(cx, |v, _| v.exts.confirm_replace.clone()).as_deref(), Some("inbox"), "asked before replacing");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "// mine", "nothing replaced yet");
        to_end(&view, cx);
        click(cx, "extension-ask-yes-inbox");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "// 2");
    }

    /// Add from Git: the form, then what the newest release changes, and
    /// Install; nothing before the click.
    #[gpui::test]
    fn add_from_git_shows_what_changes_before_installing(cx: &mut TestAppContext) {
        let t = TempDir::new("settings-ext-add-git");
        let other = t.path().join("other");
        extension_release(&other, "notes", "0.3.0", "// n", "[]", false);
        let (view, cx) = open(cx, &t);
        cx.update(|_, cx| {
            let mut r = cx.global::<catalog::GitRedirect>().0.clone();
            r.insert("https://example.com/notes".into(), other.to_string_lossy().into_owned());
            cx.set_global(catalog::GitRedirect(r));
        });
        click(cx, "extension-add-git");
        assert!(cx.debug_bounds("extension-add-git-look").is_some());
        view.update_in(cx, |v, window, cx| {
            v.close_add_git(cx);
            v.debug_add_git("https://example.com/notes", "ext", false, window, cx);
        });
        frame(cx);
        to_end(&view, cx);
        click(cx, "extension-add-git-look");
        assert!(cx.debug_bounds("extension-add-git-install").is_some(), "Install, under what changes");
        let data = t.path().join("data/extensions");
        assert!(!data.join("notes").exists(), "looking installs nothing");
        to_end(&view, cx);
        click(cx, "extension-add-git-install");
        assert!(data.join("notes/adapter/main.mjs").is_file());
        assert!(view.read_with(cx, |v, _| v.exts.add_git.is_none()), "closed once installed");
        assert!(cx.debug_bounds("extension-check-notes").is_some(), "listed under Installed");
        cx.update(|_, cx| assert!(!crate::ui_state::get(cx).extension_on("notes"), "off"));
        // An address that isn't https: said under the form, nothing fetched.
        view.update_in(cx, |v, window, cx| v.debug_add_git("git@example.com:me/x", "", true, window, cx));
        frame(cx);
        let note = view.read_with(cx, |v, _| v.exts.add_git.as_ref().and_then(|a| a.note.clone()));
        assert!(note.is_some_and(|n| n.contains("https://")), "{:?}", view.read_with(cx, |v, _| v.exts.add_git.as_ref().map(|a| a.note.clone())));
    }

    /// Types `text` into the add form's field `which` (`package`,
    /// `registry`, `url`, `dir`).
    fn type_in(view: &Entity<SettingsView>, cx: &mut VisualTestContext, which: &str, text: &str) {
        view.update(cx, |v, cx| {
            let add = v.exts.add_git.as_ref().expect("the form is open");
            let input = match which {
                "package" => add.package.clone(),
                "registry" => add.registry.clone(),
                "url" => add.url.clone(),
                _ => add.dir.clone(),
            };
            input.update(cx, |i, cx| i.set_text(text, cx));
        });
        frame(cx);
    }

    fn add_note(view: &Entity<SettingsView>, cx: &mut VisualTestContext) -> Option<String> {
        view.read_with(cx, |v, _| v.exts.add_git.as_ref().and_then(|a| a.note.clone()))
    }

    /// Add from npm, by clicks: the form on "npm package", a name and a
    /// registry, Look, what the newest version changes, and Install;
    /// nothing before the click, and nothing of the package runs.
    #[gpui::test]
    fn add_from_npm_shows_what_changes_before_installing(cx: &mut TestAppContext) {
        use pitwall_core::testing::{npm_extension, NpmRegistry};
        const REG: &str = "https://registry.example.com";
        let reg = NpmRegistry::start(REG);
        reg.publish("@me/notes", "0.3.0", &npm_extension("@me/notes", REG, "notes", "0.3.0", "// n", "[]"));
        let t = TempDir::new("settings-ext-add-npm");
        let (view, cx) = open(cx, &t);
        cx.update(|_, cx| cx.set_global(catalog::NpmRedirect(HashMap::from([(REG.to_string(), reg.url.clone())]))));
        click(cx, "extension-add-git");
        assert!(cx.debug_bounds("extension-add-kind-npm").is_some(), "the choice of where from");
        click(cx, "extension-add-kind-npm");
        assert!(view.read_with(cx, |v, _| v.exts.add_git.as_ref().is_some_and(|a| a.npm)));
        type_in(&view, cx, "package", "@me/notes");
        type_in(&view, cx, "registry", REG);
        to_end(&view, cx);
        click(cx, "extension-add-git-look");
        let p = view.read_with(cx, |v, _| v.exts.add_git.as_ref().and_then(|a| a.prepared.clone())).unwrap_or_else(|| panic!("looked at: {:?}", add_note(&view, cx)));
        assert!(p.changes[0].lines[0].starts_with("npm: @me/notes (registry.example.com), version 0.3.0, sha512 "), "{:?}", p.changes[0]);
        assert!(cx.debug_bounds("extension-add-git-install").is_some(), "Install, under what changes");
        let data = t.path().join("data/extensions");
        assert!(!data.join("notes").exists(), "looking installs nothing");
        to_end(&view, cx);
        click(cx, "extension-add-git-install");
        assert!(data.join("notes/adapter/main.mjs").is_file());
        assert!(view.read_with(cx, |v, _| v.exts.add_git.is_none()), "closed once installed");
        assert!(matches!(install::origin(&data.join("notes")), Some(Origin::Npm { package, version, .. }) if package == "@me/notes" && version == "0.3.0"));
        assert!(cx.debug_bounds("extension-check-notes").is_some(), "listed under Installed, with Check for updates");
        cx.update(|_, cx| assert!(!crate::ui_state::get(cx).extension_on("notes"), "off"));
    }

    /// Every refusal the catalog makes before fetching anything reaches
    /// the form under the click that asked (the answer comes after the
    /// click is handled, never inside it): a bad package name, a registry
    /// that isn't https, an address that isn't https.
    #[gpui::test]
    fn add_form_refusals_reach_the_form(cx: &mut TestAppContext) {
        let t = TempDir::new("settings-ext-add-refused");
        let (view, cx) = open(cx, &t);
        click(cx, "extension-add-git");
        click(cx, "extension-add-kind-npm");
        for (package, registry, why) in [("My Ext", "", "isn't an npm package name"), ("@me/x@1.0.0", "", "isn't an npm package name"), ("../x", "", "isn't an npm package name"), ("@me/x", "http://registry.example.com", "https://")] {
            type_in(&view, cx, "package", package);
            type_in(&view, cx, "registry", registry);
            to_end(&view, cx);
            click(cx, "extension-add-git-look");
            let note = add_note(&view, cx);
            assert!(note.as_deref().is_some_and(|n| n.contains(why)), "{package} {registry}: {note:?}");
            assert!(view.read_with(cx, |v, _| v.exts.add_git.as_ref().is_some_and(|a| !a.busy)), "ready again");
        }
        click(cx, "extension-add-kind-git");
        type_in(&view, cx, "url", "git@example.com:me/x");
        to_end(&view, cx);
        click(cx, "extension-add-git-look");
        assert!(add_note(&view, cx).is_some_and(|n| n.contains("https://")));
        assert_eq!(std::fs::read_dir(t.path().join("data/extensions")).unwrap().count(), 1, "nothing fetched or written");
    }

    /// A development folder (here the repository's own, with a source and
    /// a newer release out): no source line, no check, no update, no Go
    /// back, its footer (Log, Reload, Remove from list) on one row; Add
    /// from Git doesn't replace it.
    #[gpui::test]
    fn a_development_folder_gets_no_updates(cx: &mut TestAppContext) {
        let t = TempDir::new("settings-ext-dev-updates");
        let (view, cx) = open(cx, &t);
        let repo = t.path().join("repo");
        extension_release(&repo, "inbox", "1.1.0", "// 2", &setup_action("v2"), true);
        click(cx, "extension-check-inbox");
        assert!(cx.debug_bounds("extension-update-inbox").is_some(), "installed: 1.1.0 offered");
        cx.update(|_, cx| assert_eq!(crate::extensions::load_dev(&repo.join("ext"), cx).unwrap(), "inbox"));
        view.update(cx, |v, cx| {
            v.exts_open("inbox");
            cx.notify();
        });
        frame(cx);
        for gone in ["extension-check-inbox", "extension-update-inbox", "extension-back-inbox"] {
            assert!(cx.debug_bounds(gone).is_none(), "{gone}");
        }
        let (what, buttons) = (cx.debug_bounds("extension-what-inbox").unwrap(), cx.debug_bounds("extension-footer-inbox").unwrap());
        assert!(buttons.size.height < gpui::px(40.) && buttons.origin.y < what.bottom(), "one row beside it: {buttons:?} {what:?}");
        view.update_in(cx, |v, window, cx| v.debug_add_git("https://example.com/r", "ext", true, window, cx));
        frame(cx);
        let note = view.read_with(cx, |v, _| v.exts.add_git.as_ref().and_then(|a| a.note.clone()));
        assert!(note.as_deref().is_some_and(|n| n.contains("development folder")), "{note:?}");
        assert_eq!(std::fs::read_to_string(t.path().join("data/extensions/inbox/adapter/main.mjs")).unwrap(), "// 1", "nothing replaced");
        assert_eq!(std::fs::read_to_string(repo.join("ext/adapter/main.mjs")).unwrap(), "// 2", "the author's folder untouched");
    }
}
