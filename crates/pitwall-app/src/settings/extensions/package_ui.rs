//! Settings → Extensions, "Install from file…" (docs/spec/tile-sdk/
//! packaging.md §6.3, §6.4): a tile package (`.pwtile`) picked, read and
//! its signature checked before anything is unpacked; then its trust
//! screen: who signed it (the key, first seen or the installed version's),
//! a key change or a missing signature that the user must agree to, the
//! permissions it asks for (an update: what changes in them, new ones not
//! granted until allowed), and what installing changes on this computer.
//! An installed extension is updated this way too: same key only, unless
//! the user says the new key is to be trusted; the installed copy is kept
//! for Go back.

use gpui::{div, prelude::*, px, AnyElement, Context, Div, SharedString};
use pitwall_core::extensions::package::{self, Key, PermissionChange};
use pitwall_core::extensions::{permissions, Manifest};

use super::super::view::SettingsView;
use super::super::widgets::Ui;
use crate::extensions::catalog::{self, Prepared};
use crate::kit::{BtnKind, Tone};

/// "Install from file…": the file picked, then what installing it
/// changes, or why it can't be.
pub(in crate::settings) struct AddFile {
    path: Option<std::path::PathBuf>,
    busy: bool,
    note: Option<String>,
    pub(super) prepared: Option<Prepared>,
    /// "Trust the new key" ticked (a package signed by another key than
    /// the installed version, or unsigned while it was signed).
    new_key: bool,
}

/// A text button, found by its id in tests too.
fn btn(id: String, text: impl Into<SharedString>, kind: BtnKind, disabled: bool, ui: &Ui) -> gpui::Stateful<Div> {
    let sel = id.clone();
    crate::kit::text_button(SharedString::from(id), text, kind, disabled, &ui.t).debug_selector(move || sel)
}

/// A box with a tone's border and fill (amber for a warning).
fn note_box(sel: String, warn: bool, ui: &Ui) -> Div {
    let c = if warn { ui.t.amber } else { ui.t.line_strong };
    let d = div().debug_selector(move || sel).flex().flex_col().gap(px(6.)).px(px(10.)).py(px(8.)).rounded(px(8.)).border_1();
    if warn {
        d.border_color(c.opacity(0.35)).bg(ui.t.amber.opacity(0.08))
    } else {
        d.border_color(ui.t.line).bg(ui.t.surface)
    }
}

impl SettingsView {
    /// The "Install from file…" button beside "Install from folder…".
    pub(super) fn add_file_button(&mut self, ui: &Ui, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let open = self.exts.add_file.is_some();
        btn("extension-add-file".to_string(), "Install from file…", BtnKind::GhostSm, open, ui)
            .tooltip(crate::kit::tooltip("Install or update a tile extension from a .pwtile package. Its signature is checked first"))
            .on_click(cx.listener(|v, _, _, cx| v.open_add_file(cx)))
    }

    /// "Install from file…" open: the file being checked, why it can't be
    /// installed, or its trust screen.
    pub(super) fn add_file_view(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Option<gpui::Stateful<Div>> {
        let add = self.exts.add_file.as_ref()?;
        let (busy, note, prepared, path, new_key) = (add.busy, add.note.clone(), add.prepared.clone(), add.path.clone(), add.new_key);
        let mut card = div().id("extension-add-file-card").debug_selector(|| "extension-add-file-card".into()).flex().flex_col().gap(px(12.)).p(px(14.)).rounded(px(10.)).border_1().border_color(ui.t.line_strong).bg(ui.t.surface_2);
        let cancel = btn("extension-add-file-cancel".to_string(), "Cancel", BtnKind::GhostSm, busy && prepared.is_some(), ui).on_click(cx.listener(|v, _, _, cx| v.close_add_file(cx)));
        match prepared {
            Some(p) => {
                let m = p.offer.manifest.clone();
                let key = catalog::package_key(&p, cx).unwrap_or(Key::Unsigned);
                let old = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|o| o.id == m.id).cloned().filter(|_| p.replaces.is_some());
                let (icon, text) = Self::ext_title(ui, &m, &m.id);
                let v = m.version.clone().unwrap_or_default();
                let heading = match &p.replaces {
                    Some(was) => format!("Update {} {} → {v}", m.name, was.clone().unwrap_or_default()),
                    None => format!("Install {} {v}", m.name),
                };
                card = card.child(div().text_size(px(14.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(heading));
                card = card.child(div().flex().gap(px(12.)).items_start().child(icon).child(text));
                card = card.child(self.trust_view(ui, &m, &key, new_key, cx));
                match &old {
                    Some(old) => card = card.child(Self::permission_changes_view(ui, old, &m)),
                    None if !m.permissions.is_empty() => {
                        let mut perms = div().flex().flex_col().gap(px(10.)).child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child("It asks for these permissions"));
                        for ask in &m.permissions {
                            let mut row = div().flex().gap(px(12.)).items_start().child(Self::permission_words_of(ui, &m, &ask.name, &ask.reason));
                            if let Some(amount) = permissions::amount(&m, &ask.name) {
                                row = row.child(div().flex_none().mt(px(1.)).child(crate::kit::mono_text(amount, &ui.t).text_color(ui.t.text_2)));
                            }
                            perms = perms.child(row);
                        }
                        card = card.child(perms.child(crate::kit::hint("You answer them when you turn it on.", &ui.t)));
                    }
                    None => {}
                }
                card = card.child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child("What installing it changes on this computer"));
                card = card.child(super::updates::changes_view(ui, &p.changes));
                let hint = match &p.replaces {
                    Some(was) => format!("The installed {} is kept: Go back on its card returns to it. Its settings and data stay.", was.clone().unwrap_or_default()),
                    None => "Installing runs nothing: it stays off until you turn it on.".into(),
                };
                card = card.child(crate::kit::hint(hint, &ui.t));
                let changed = matches!(key, Key::Changed { .. });
                let unsigned = matches!(key, Key::Unsigned | Key::Changed { now: None, .. });
                let label = match (busy, &p.replaces, unsigned) {
                    (true, _, _) => "Installing…",
                    (false, Some(_), true) => "Update unsigned",
                    (false, Some(_), false) => "Update",
                    (false, None, true) => "Install unsigned",
                    (false, None, false) => "Install",
                };
                let kind = if unsigned || changed { BtnKind::Danger } else { BtnKind::Small };
                let yes = btn("extension-add-file-install".to_string(), label, kind, busy || (changed && !new_key), ui).on_click(cx.listener(|v, _, _, cx| v.install_from_package(cx)));
                card = card.child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).child(yes));
            }
            None => {
                let what = match (&path, busy) {
                    (Some(p), true) => format!("Checking {}…", pitwall_core::paths::tildify(&p.to_string_lossy())),
                    (Some(p), false) => format!("From {}", pitwall_core::paths::tildify(&p.to_string_lossy())),
                    (None, _) => "Pick the .pwtile file. Its signature (.minisig) is read from beside it.".to_string(),
                };
                card = card.child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child("Install from file"));
                card = card.child(crate::kit::hint(what, &ui.t));
                let again = btn("extension-add-file-pick".to_string(), "Choose another file…", BtnKind::GhostSm, busy, ui).on_click(cx.listener(|v, _, _, cx| v.open_add_file(cx)));
                card = card.child(div().flex().gap(px(8.)).child(div().flex_1()).child(cancel).when(note.is_some(), |d| d.child(again)));
            }
        }
        if let Some(n) = note {
            card = card.child(div().debug_selector(|| "extension-add-file-note".into()).text_size(px(12.)).text_color(ui.t.amber).child(n));
        }
        Some(card)
    }

    /// Who signed it: the key and its public key, first seen or the
    /// installed version's; another key (with "Trust the new key") or none.
    fn trust_view(&mut self, ui: &Ui, m: &Manifest, key: &Key, new_key: bool, cx: &mut Context<Self>) -> Div {
        let sel = format!("extension-add-file-key-{}", m.id);
        let title = |icon: &str, color: gpui::Hsla, text: String| div().flex().items_center().gap(px(6.)).child(crate::kit::icon(icon, 13., color)).child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(text));
        let key_rows = |label: &str, s: &package::Signer| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().flex().items_center().gap(px(8.)).child(div().text_size(px(12.)).text_color(ui.t.text_3).child(label.to_string())).child(crate::kit::mono_text(s.key_id.clone(), &ui.t).text_color(ui.t.text)))
                .child(crate::kit::mono_text(s.public_key.clone(), &ui.t).text_color(ui.t.text_3))
        };
        let words = |text: String| div().text_size(px(12.)).line_height(gpui::relative(1.4)).text_color(ui.t.text_2).child(text);
        match key {
            Key::New(s) => note_box(sel, false, ui)
                .child(title("lock", ui.t.green, "Signed".into()))
                .child(key_rows("Key", s))
                .child(words(format!("Installing trusts this key for {}: its updates must be signed by it too. Compare the key with the one its author publishes.", m.name))),
            Key::Same(s) => note_box(sel, false, ui).child(title("lock", ui.t.green, "Signed by the same key as the installed version".into())).child(key_rows("Key", s)),
            Key::Unsigned => note_box(sel, true, ui).child(title("alert", ui.t.amber, "Not signed".into())).child(words(package::UNSIGNED_WARNING.into())).child(words("It is marked as a preview wherever it shows.".into())),
            Key::Changed { now, was } => {
                let mut b = note_box(sel, true, ui);
                b = match now {
                    Some(s) => b.child(title("alert", ui.t.amber, "Signed by another key".into())).child(key_rows("New key", s)).child(key_rows("Installed version's", was)).child(words("Someone else may have signed it. Only trust the new key if its author told you the key changed.".into())),
                    None => b.child(title("alert", ui.t.amber, "Not signed, but the installed version is".into())).child(key_rows("Installed version's", was)).child(words("Only install it if its author told you why it isn't signed.".into())),
                };
                let id = m.id.clone();
                let tick = format!("extension-add-file-new-key-{id}");
                let what = if now.is_some() { "Trust the new key for its updates from now on" } else { "Install it without a signature" };
                b.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(crate::kit::toggle(SharedString::from(tick.clone()), new_key, &ui.t).debug_selector(move || tick).on_click(cx.listener(move |v, _, _, cx| v.trust_new_key(!new_key, cx))))
                        .child(div().text_size(px(12.)).text_color(ui.t.text).child(what)),
                )
            }
        }
    }

    /// An update's changes in what it asks for: new permissions (not
    /// granted until allowed), more of one (net hosts, sizes; asked again),
    /// and ones it no longer asks for.
    fn permission_changes_view(ui: &Ui, old: &Manifest, new: &Manifest) -> AnyElement {
        let changes = package::permission_changes(old, new);
        let mut col = div().debug_selector(|| "extension-add-file-permissions".into()).flex().flex_col().gap(px(10.)).child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child("Permission changes"));
        if changes.is_empty() {
            return col.child(crate::kit::hint("It asks for the same permissions as the installed version.", &ui.t)).into_any_element();
        }
        for c in &changes {
            let name = c.name().to_string();
            let sel = format!("extension-add-file-permission-{name}");
            let reason = new.permissions.iter().chain(old.permissions.iter()).find(|p| p.name == name).map(|p| p.reason.clone()).unwrap_or_default();
            let (chip, tone, lines) = match c {
                PermissionChange::Added { .. } => ("new", Tone::Warn, vec![]),
                PermissionChange::Widened { lines, .. } => ("more", Tone::Warn, lines.clone()),
                PermissionChange::Removed { .. } => ("removed", Tone::Subtle, vec![]),
            };
            let mut words = Self::permission_words_of(ui, new, &name, &reason);
            for l in lines {
                words = words.child(crate::kit::mono_text(l, &ui.t).text_color(ui.t.text_2));
            }
            col = col.child(div().debug_selector(move || sel).flex().gap(px(12.)).items_start().child(words).child(div().flex_none().mt(px(1.)).child(crate::kit::chip_tone(chip, tone, &ui.t))));
        }
        let asks = changes.iter().any(|c| !matches!(c, PermissionChange::Removed { .. }));
        if asks {
            col = col.child(crate::kit::hint("New and wider permissions aren't granted by updating: you allow or refuse each on its card.", &ui.t));
        }
        col.into_any_element()
    }

    /// "Install from file…": the file picker, then the file checked.
    fn open_add_file(&mut self, cx: &mut Context<Self>) {
        if self.exts.add_file.as_ref().is_some_and(|a| a.busy) {
            return;
        }
        if let Some(p) = self.exts.add_file.take().and_then(|a| a.prepared) {
            catalog::discard(&p, cx);
        }
        self.exts.add_file = Some(AddFile { path: None, busy: true, note: None, prepared: None, new_key: false });
        cx.notify();
        let picked = crate::platform::files::pick_file(cx, "Install extension");
        cx.spawn(async move |this, cx| {
            let file = picked.await;
            let _ = this.update(cx, |v, cx| match file {
                Some(f) => v.look_at_package(f, cx),
                None => {
                    v.exts.add_file = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Package file `path` read, its signature checked and unpacked
    /// aside: its trust screen, or why it can't be installed. (Also what
    /// bench commands and tests call: no file picker.)
    pub fn look_at_package(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if let Some(p) = self.exts.add_file.take().and_then(|a| a.prepared) {
            catalog::discard(&p, cx);
        }
        self.exts.add_file = Some(AddFile { path: Some(path.clone()), busy: true, note: None, prepared: None, new_key: false });
        cx.notify();
        let me = cx.entity().downgrade();
        let not_package = path.extension().is_none_or(|e| !e.eq_ignore_ascii_case(package::EXTENSION));
        if not_package {
            if let Some(add) = self.exts.add_file.as_mut() {
                add.busy = false;
                add.note = Some(format!("{} isn't a .{} package. To install a folder, use Install from folder….", path.file_name().unwrap_or_default().to_string_lossy(), package::EXTENSION));
            }
            return;
        }
        catalog::prepare_from_package(&path, cx, move |res, cx| {
            let mut res = Some(res);
            let _ = me.update(cx, |v, cx| {
                let Some(add) = v.exts.add_file.as_mut() else { return };
                add.busy = false;
                match res.take() {
                    // One that can't go in whatever the user says (older,
                    // the same file) is refused now, saying why.
                    Some(Ok(p)) => match catalog::package_refusal(&p, package::Accept { unsigned: true, new_key: true }, cx) {
                        Some(why) => {
                            add.note = Some(why);
                            catalog::discard(&p, cx);
                        }
                        None => add.prepared = Some(p),
                    },
                    Some(Err(why)) => add.note = Some(why),
                    None => {}
                }
                cx.notify();
            });
            // Closed meanwhile: the copy goes.
            if let Some(Ok(p)) = res {
                catalog::discard(&p, cx);
            }
        });
    }

    pub(super) fn close_add_file(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.exts.add_file.take().and_then(|a| a.prepared) {
            catalog::discard(&p, cx);
        }
        cx.notify();
    }

    /// "Trust the new key" ticked or not (tests and bench commands too).
    pub fn trust_new_key(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(add) = self.exts.add_file.as_mut() {
            add.new_key = on;
            cx.notify();
        }
    }

    /// Install (or update to) the package looked at, as its button does:
    /// unsigned agreed by pressing "Install unsigned", a new key only when
    /// ticked.
    pub fn install_from_package(&mut self, cx: &mut Context<Self>) {
        let Some(add) = self.exts.add_file.as_ref() else { return };
        let Some(p) = add.prepared.clone() else { return };
        if add.busy {
            return;
        }
        let accept = package::Accept { unsigned: true, new_key: add.new_key };
        if let Some(why) = catalog::package_refusal(&p, accept, cx) {
            if let Some(add) = self.exts.add_file.as_mut() {
                add.note = Some(why);
            }
            cx.notify();
            return;
        }
        if let Some(add) = self.exts.add_file.as_mut() {
            add.busy = true;
            add.note = None;
        }
        cx.notify();
        let me = cx.entity().downgrade();
        let id = p.offer.manifest.id.clone();
        catalog::apply_with(&p, true, accept, cx, move |o, cx| {
            let _ = me.update(cx, |v, cx| {
                match o {
                    catalog::Outcome::Done => {
                        v.exts.add_file = None;
                        v.exts.open.insert(id.clone(), true);
                    }
                    catalog::Outcome::Failed(why) => {
                        if let Some(add) = v.exts.add_file.as_mut() {
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

    /// Tests and bench commands: what "Install from file…" shows now
    /// (checking or installing, why not, the id it would install).
    pub fn add_file_state(&self) -> Option<(bool, Option<String>, Option<String>)> {
        self.exts.add_file.as_ref().map(|a| (a.busy, a.note.clone(), a.prepared.as_ref().map(|p| p.offer.manifest.id.clone())))
    }
}
