//! Settings → Profiles (docs/spec/profiles.md), a page of its own, in the
//! nav once there is something to bundle (a second login, a git identity,
//! a browser account, a second connection) or a profile exists:
//!
//! - what a profile is, in a line, and "Add profile";
//! - each profile: its name and the projects that use it; folded, what it
//!   picks as small labelled chips; unfolded (a new one is, at once), each
//!   slot's choices as chips: "Not set" (each project decides, else the
//!   default), the default, each account of that kind, for an extension
//!   None, and "+ New …": a login, browser account or connection is made
//!   in place (a login then opens its login), a git identity on Accounts'
//!   form, back here once saved. Rename; delete (asks; its projects keep
//!   what they run under). A change to a profile projects use applies as
//!   a pick in their Profile… dialog does (`accounts::PicksChanged`).
//!
//! Settings → Accounts keeps the accounts to choose from; its read-only
//! **Extension connections** (only once an extension has two) are here
//! too, and a line at its top points to Profiles when that page shows.

use gpui::{div, prelude::*, px, App, Context, Div, Entity, FontWeight, SharedString};
use pitwall_proto::settings::Page;

use super::view::SettingsView;
use super::widgets::Ui;
use crate::accounts::{self as acc, SlotRow};
use crate::kit::Ellipsis as _;
use crate::kit::{BtnKind, InputEvent, TextInput};
use pitwall_core::accounts::{self as core_acc, Profile};

/// The page's own state.
#[derive(Default)]
pub struct Section {
    /// A profile being named: the one renamed (`None`: a new one) and its
    /// field.
    naming: Option<(Option<String>, Entity<TextInput>)>,
    /// Asking before deleting this profile.
    deleting: Option<String>,
    /// Profiles unfolded (a new one is, until folded).
    open: std::collections::HashSet<String>,
    /// "+ New …" in a profile's slot: (profile, slot) and the new one's
    /// name.
    creating: Option<(String, String, Entity<TextInput>)>,
    /// A git identity being added on Accounts for (profile): it is given
    /// that identity, and Profiles shows again, once it is saved.
    pub(super) git_for: Option<String>,
    error: Option<String>,
    _subs: Vec<gpui::Subscription>,
}

/// Settings opens next on Profiles with this profile unfolded (the
/// project dialog's "Edit profile …").
struct OpenProfile(Option<String>);

impl gpui::Global for OpenProfile {}

/// Settings next opens on Profiles, profile `id` unfolded (`None`: just the
/// page).
pub fn show_profile(id: Option<&str>, cx: &mut App) {
    cx.set_global(OpenProfile(id.map(String::from)));
    super::data::set_page(cx, Page::Profiles);
}

/// A project folder's last part.
fn short(p: &str) -> String {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or(p).to_string()
}

/// What "+ New …" makes in `slot` (`None`: nothing can be made there).
fn new_label(slot: &str, cx: &App) -> Option<&'static str> {
    if slot == core_acc::GIT {
        Some("+ New git identity")
    } else if slot == core_acc::WEB {
        crate::web::profiles::separate(cx).then_some("+ New browser account")
    } else if core_acc::ext_of(slot).is_some() {
        Some("+ New connection")
    } else {
        Some("+ New account")
    }
}

impl SettingsView {
    /// An extension has two connections or more: their section shows (one
    /// is just the extension's settings, on its card).
    pub(super) fn has_connections(cx: &gpui::App) -> bool {
        acc::list(cx).is_some_and(|l| {
            let mut seen = std::collections::HashSet::new();
            l.accounts.iter().filter_map(|a| core_acc::ext_of(&a.kind)).any(|e| !seen.insert(e.to_string()))
        })
    }

    /// Profiles show once there is a choice to bundle: a second login, a
    /// git identity, a browser account or a second connection of an
    /// extension (or once one exists). One login and one connection: no
    /// page, nothing to explain.
    pub(super) fn profiles_useful(cx: &gpui::App) -> bool {
        let has_profiles = acc::list(cx).is_some_and(|l| !l.profiles.is_empty());
        has_profiles || acc::slot_rows(cx).iter().any(|r| if core_acc::ext_of(&r.slot).is_some() { r.choices.len() > 3 } else { r.choices.len() > 1 })
    }

    /// Accounts' first line once Profiles shows: where they are put
    /// together.
    pub(super) fn profiles_pointer(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().flex_1().min_w_0().child(crate::kit::muted("The accounts a profile picks from. Projects get them through their profile.", t)))
            .child(div().flex_none().child(crate::kit::text_button("acct-to-profiles", "Open Profiles", BtnKind::Small, false, t).on_click(cx.listener(|v, _, w, cx| v.go_page(Page::Profiles, w, cx)))))
    }

    /// Each extension's connections, read-only, with where they are used.
    pub(super) fn connections_section(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let list = acc::list(cx).cloned().unwrap_or_default();
        let mut sec = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().pt(px(4.)).min_h(px(26.)).flex().items_center().child(div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child("Extension connections")))
            .child(crate::kit::muted("An extension's server and key, one set per connection. A project uses the default unless it or its profile picks another. They are set up on the extension's card.", t));
        for m in crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).cloned() {
            let conns: Vec<_> = list.connections(&m.id).cloned().collect();
            if conns.is_empty() {
                continue;
            }
            let default = list.default_connection(&m.id).map(|c| c.id.clone());
            let id = m.id.clone();
            sec = sec.child(
                div()
                    .pt(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(11.)).font_weight(crate::kit::WEIGHT_550).text_color(t.text_3).child(m.name.to_uppercase()))
                    .child(div().flex_1())
                    .child(
                        crate::kit::text_button(SharedString::from(format!("acct-conn-edit-{}", m.id)), "Edit in Extensions", BtnKind::Link, false, t).on_click(cx.listener(move |v, _, w, cx| {
                            v.exts_open(&id);
                            v.go_page(pitwall_proto::settings::Page::Extensions, w, cx);
                        })),
                    ),
            );
            for c in conns {
                let mut used: Vec<String> = list.profiles_of(&c.id).into_iter().map(|p| format!("profile {p}")).collect();
                used.extend(list.projects_of(&c.id).iter().map(|p| short(p)));
                let address = c.values.values().next().cloned().unwrap_or_else(|| "Not set up yet".into());
                sec = sec.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py(px(7.))
                        .rounded(crate::theme::RADIUS)
                        .border_1()
                        .border_color(t.line)
                        .child(div().flex_none().child(crate::kit::icon(crate::extensions::icon_of(&m), 14., t.text_3)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(1.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child(c.name.clone()))
                                        .when(default.as_deref() == Some(&c.id), |d| d.child(div().text_size(px(12.)).text_color(t.text_3).child("default"))),
                                )
                                .child(div().min_w_0().ellipsis().text_size(px(11.5)).text_color(t.text_3).font_family(crate::kit::MONO_FONT).child(crate::kit::one_line(address)))
                                .when(!used.is_empty(), |d| d.child(div().min_w_0().ellipsis().text_size(px(11.5)).text_color(t.text_3).child(crate::kit::one_line(format!("Used by {}", used.join(", ")))))),
                        ),
                );
            }
        }
        sec
    }

    /// Bench and debug builds: profile `id` unfolded.
    pub fn debug_profile_open(&mut self, id: &str, cx: &mut Context<Self>) {
        self.profiles.open.insert(id.to_string());
        cx.notify();
    }

    fn start_profile_naming(&mut self, id: Option<String>, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let text = id.as_deref().and_then(|i| acc::list(cx).and_then(|l| l.profile(i)).map(|p| p.name.clone())).unwrap_or_default();
        let input = cx.new(|cx| TextInput::new(cx, &text, "Work"));
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all_text(cx);
        });
        let sub = cx.subscribe(&input, |v, _, e: &InputEvent, cx| match e {
            InputEvent::Submit => v.finish_profile_naming(cx),
            InputEvent::Cancel => {
                v.profiles.naming = None;
                cx.notify();
            }
            InputEvent::Changed => {
                v.profiles.error = None;
                cx.notify();
            }
            _ => {}
        });
        self.profiles._subs.push(sub);
        self.profiles.naming = Some((id, input));
        self.profiles.deleting = None;
        self.profiles.error = None;
        cx.notify();
    }

    fn finish_profile_naming(&mut self, cx: &mut Context<Self>) {
        let Some((id, input)) = self.profiles.naming.clone() else { return };
        let name = input.read(cx).text().to_string();
        let model = acc::model(cx);
        let res = match &id {
            Some(i) => model.update(cx, |m, cx| m.rename_profile(i, &name, cx)).map(|_| i.clone()),
            None => model.update(cx, |m, cx| m.add_profile(&name, cx)).map(|p| p.id),
        };
        match res {
            Ok(pid) => {
                self.profiles.naming = None;
                if id.is_none() {
                    // A new one opens to be filled in.
                    self.profiles.open.insert(pid);
                }
            }
            Err(e) => self.profiles.error = Some(e),
        }
        cx.notify();
    }

    /// "+ New …" in profile `pid`'s `slot`: a git identity is added on
    /// Accounts (its form needs more than a name); the rest are named here.
    fn start_creating(&mut self, pid: &str, slot: &str, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if slot == core_acc::GIT {
            self.open_git_form(None, cx);
            self.profiles.git_for = Some(pid.to_string());
            self.go_page(Page::Accounts, window, cx);
            return;
        }
        let input = cx.new(|cx| TextInput::new(cx, "", "Work"));
        input.update(cx, |i, cx| i.focus(window, cx));
        let sub = cx.subscribe(&input, |v, _, e: &InputEvent, cx| match e {
            InputEvent::Submit => v.finish_creating(cx),
            InputEvent::Cancel => {
                v.profiles.creating = None;
                cx.notify();
            }
            InputEvent::Changed => {
                v.profiles.error = None;
                cx.notify();
            }
            _ => {}
        });
        self.profiles._subs.push(sub);
        self.profiles.creating = Some((pid.to_string(), slot.to_string(), input));
        self.profiles.error = None;
        cx.notify();
    }

    /// Make the account named in "+ New …" and give it to the profile: a
    /// login then starts its login, a connection is then set up on its
    /// extension's card (said under the slot).
    fn finish_creating(&mut self, cx: &mut Context<Self>) {
        let Some((pid, slot, input)) = self.profiles.creating.clone() else { return };
        let name = input.read(cx).text().trim().to_string();
        let model = acc::model(cx);
        let made = if slot == core_acc::WEB {
            crate::web::profiles::add(&name, cx).map(|a| a.id)
        } else if let Some(ext) = core_acc::ext_of(&slot) {
            let ext = ext.to_string();
            model.update(cx, |m, cx| m.add_connection(&ext, &name, cx)).map(|a| a.id)
        } else {
            model.update(cx, |m, cx| m.add(&slot, &name, None, cx)).map(|a| a.id)
        };
        match made.and_then(|id| model.update(cx, |m, cx| m.set_profile_pick(&pid, &slot, Some(&id), cx)).map(|_| id)) {
            Ok(id) => {
                self.profiles.creating = None;
                if acc::is_tool_kind(&slot) {
                    self.log_in(&slot, &id, cx);
                }
            }
            Err(e) => self.profiles.error = Some(e),
        }
        cx.notify();
    }

    /// A git identity was just added on Accounts: the profile that asked
    /// for it gets it, and Profiles shows again with it unfolded.
    pub(super) fn git_added(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(pid) = self.profiles.git_for.take() else { return };
        let model = acc::model(cx);
        if let Err(e) = model.update(cx, |m, cx| m.set_profile_pick(&pid, core_acc::GIT, Some(id), cx)) {
            self.profiles.error = Some(e);
        }
        self.profiles.open.insert(pid);
        super::data::set_page(cx, Page::Profiles);
        cx.notify();
    }

    /// Profiles: what one is, Add profile, each profile's card.
    pub(super) fn profiles_section(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        // Opened from a project's dialog: that profile unfolded.
        if let Some(id) = cx.try_global::<OpenProfile>().and_then(|o| o.0.clone()) {
            cx.set_global(OpenProfile(None));
            self.profiles.open.insert(id);
        }
        let model = acc::model(cx);
        let live = model.read(cx).live();
        let list = model.read(cx).list.clone();
        let adding = matches!(self.profiles.naming, Some((None, _)));
        let mut sec = div().flex().flex_col().gap(px(10.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(4.))
                .min_h(px(26.))
                .child(div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child("Profiles"))
                .child(div().flex_1())
                .when(live && !adding, |d| d.child(crate::kit::text_button("profile-add", "Add profile", BtnKind::Small, false, t).on_click(cx.listener(|v, _, w, cx| v.start_profile_naming(None, w, cx))))),
        );
        sec = sec.child(crate::kit::muted(
            "A profile is a set: which login, git identity, browser account and connections go together. Give a project a profile and it uses all of them.",
            t,
        ));
        if !live {
            return sec.child(crate::kit::hint("Profiles need Pitwall's engine, which isn't running.", t));
        }
        let rows = acc::profile_slot_rows(cx);
        if let Some((None, input)) = self.profiles.naming.clone() {
            let empty = input.read(cx).text().trim().is_empty();
            sec = sec.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(crate::theme::RADIUS)
                    .border_1()
                    .border_color(t.line_strong)
                    .bg(t.surface_2)
                    .child(crate::kit::muted("Name", t))
                    .child(div().w(px(240.)).child(input))
                    .child(div().flex_1())
                    .child(crate::kit::text_button("profile-add-cancel", "Cancel", BtnKind::Ghost, false, t).on_click(cx.listener(|v, _, _, cx| {
                        v.profiles.naming = None;
                        cx.notify();
                    })))
                    .child(crate::kit::text_button("profile-add-save", "Add", BtnKind::Primary, empty, t).on_click(cx.listener(|v, _, _, cx| v.finish_profile_naming(cx)))),
            );
        }
        if list.profiles.is_empty() && !adding {
            sec = sec.child(
                div()
                    .p(px(12.))
                    .rounded(px(10.))
                    .border_1()
                    .border_dashed()
                    .border_color(t.line)
                    .child(crate::kit::muted("No profiles yet. Add one, like Work, pick its accounts, then give it to projects from their Profile… menu item.", t)),
            );
        }
        for p in &list.profiles {
            sec = sec.child(self.profile_card(ui, p, &rows, cx));
        }
        if let Some(e) = &self.profiles.error {
            sec = sec.child(crate::kit::dialog::form_error(e.clone(), t));
        }
        sec
    }

    /// One profile: its name and who uses it; folded, its picks as small
    /// labelled chips; unfolded, each slot's chips.
    fn profile_card(&mut self, ui: &Ui, p: &Profile, rows: &[SlotRow], cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let list = acc::list(cx).cloned().unwrap_or_default();
        let projects: Vec<String> = list.profile_projects(&p.id).iter().map(|f| short(f)).collect();
        let open = self.profiles.open.contains(&p.id);
        let renaming = self.profiles.naming.as_ref().filter(|(i, _)| i.as_deref() == Some(&p.id)).map(|(_, i)| i.clone());
        let pid = p.id.clone();
        let chevron = div()
            .id(SharedString::from(format!("profile-fold-{}", p.id)))
            .cursor_pointer()
            .p(px(4.))
            .rounded(px(4.))
            .hover(|d| d.bg(t.surface_2))
            .child(crate::kit::chevron_at(open, 12., t.text_3))
            .on_click(cx.listener(move |v, _, _, cx| {
                if !v.profiles.open.remove(&pid) {
                    v.profiles.open.insert(pid.clone());
                }
                cx.notify();
            }));
        // What it picks, for the folded card: "Claude Code  Work".
        let picks: Vec<(String, String)> = rows
            .iter()
            .filter_map(|r| {
                let v = p.picks.get(&r.slot)?;
                let name = r.choices.iter().find(|(c, _)| c == v).map(|(_, n)| n.clone())?;
                Some((r.label.clone(), name))
            })
            .collect();
        let mut actions = div().flex().items_center().gap(px(6.)).flex_none();
        if self.profiles.deleting.as_deref() == Some(&p.id) {
            let id = p.id.clone();
            actions = actions
                .child(crate::kit::text_button(SharedString::from(format!("profile-del-no-{}", p.id)), "Keep", BtnKind::Ghost, false, t).on_click(cx.listener(|v, _, _, cx| {
                    v.profiles.deleting = None;
                    cx.notify();
                })))
                .child(crate::kit::text_button(SharedString::from(format!("profile-del-yes-{}", p.id)), "Delete", BtnKind::Danger, false, t).on_click(cx.listener(move |v, _, _, cx| {
                    let model = acc::model(cx);
                    if let Err(e) = model.update(cx, |m, cx| m.remove_profile(&id, cx)) {
                        v.profiles.error = Some(e);
                    }
                    v.profiles.deleting = None;
                    cx.notify();
                })));
        } else if renaming.is_some() {
            actions = actions.child(crate::kit::text_button(SharedString::from(format!("profile-rename-ok-{}", p.id)), "Save", BtnKind::Small, false, t).on_click(cx.listener(|v, _, _, cx| v.finish_profile_naming(cx))));
        } else {
            let (i1, i2) = (p.id.clone(), p.id.clone());
            actions = actions
                .child(crate::kit::text_button(SharedString::from(format!("profile-rename-{}", p.id)), "Rename", BtnKind::Small, false, t).on_click(cx.listener(move |v, _, w, cx| v.start_profile_naming(Some(i1.clone()), w, cx))))
                .child(crate::kit::text_button(SharedString::from(format!("profile-del-{}", p.id)), "Delete", BtnKind::Small, false, t).on_click(cx.listener(move |v, _, _, cx| {
                    v.profiles.deleting = Some(i2.clone());
                    v.profiles.naming = None;
                    cx.notify();
                })));
        }
        let sub = if self.profiles.deleting.as_deref() == Some(&p.id) {
            if projects.is_empty() { "Delete it? Its accounts stay.".to_string() } else { format!("Delete it? Its accounts stay; {} keep what they run under as their own picks.", projects.join(", ")) }
        } else if !projects.is_empty() {
            format!("Used by {}", projects.join(", "))
        } else {
            "No project uses it yet: give it to one from the project's Profile… menu item.".into()
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(chevron)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(match renaming {
                        Some(input) => div().w(px(220.)).child(input).into_any_element(),
                        None => div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child(p.name.clone()).into_any_element(),
                    })
                    .child(div().text_size(px(11.5)).text_color(t.text_3).child(sub)),
            )
            .child(actions);
        let mut card = div().flex().flex_col().gap(px(10.)).px(px(10.)).py(px(8.)).rounded(crate::theme::RADIUS).border_1().border_color(t.line).child(head);
        if !open {
            // Small labelled chips, wrapping: nothing cut mid-word.
            let chips = if picks.is_empty() {
                vec![div().text_size(px(11.5)).text_color(t.text_3).child("Picks nothing yet: unfold it to choose its accounts.").into_any_element()]
            } else {
                picks
                    .into_iter()
                    .map(|(label, name)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .h(px(22.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .bg(t.surface_2)
                            .text_size(px(11.5))
                            .child(div().text_color(t.text_3).child(label))
                            .child(div().text_color(t.text).child(name))
                            .into_any_element()
                    })
                    .collect()
            };
            card = card.child(div().pl(px(32.)).pb(px(2.)).flex().flex_wrap().gap(px(6.)).children(chips));
        } else {
            let mut grid = div().flex().flex_col().gap(px(10.)).pl(px(32.)).pb(px(4.));
            for (n, r) in rows.iter().enumerate() {
                grid = grid.child(self.profile_slot(ui, p, r, n, cx));
            }
            card = card.child(grid);
        }
        card
    }

    /// A profile's slot: its label, then "Not set", each choice and
    /// "+ New …" as chips, its pick ticked; the new one's name field while
    /// it is made; a connection picked that isn't set up yet says so.
    fn profile_slot(&mut self, ui: &Ui, p: &Profile, r: &SlotRow, n: usize, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let current = p.picks.get(&r.slot).cloned();
        let mut chips = div().flex().flex_wrap().gap(px(6.));
        let choices: Vec<(Option<String>, String)> = std::iter::once((None, "Not set".to_string())).chain(r.choices.iter().map(|(v, name)| (Some(v.clone()), name.clone()))).collect();
        for (i, (val, name)) in choices.into_iter().enumerate() {
            let on = current == val;
            let (pid, slot) = (p.id.clone(), r.slot.clone());
            let tip = if val.is_none() { Some("Each project using it picks this itself (else the default)") } else { None };
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("profile-{}-{n}-{i}", p.id)))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .h(px(26.))
                    .px(px(10.))
                    .rounded_full()
                    .border_1()
                    .border_color(if on { t.text_2 } else { t.line_strong })
                    .when(on, |d| d.bg(t.surface_2))
                    .when(val.is_none() && !on, |d| d.border_dashed())
                    .cursor_pointer()
                    .hover(|d| d.bg(t.surface_2))
                    .when_some(tip, |d, tip| d.tooltip(crate::kit::tooltip(tip)))
                    .when(on, |d| d.child(crate::kit::icon("check", 11., t.text_2)))
                    .child(div().text_size(px(12.)).text_color(if val.is_none() && !on { t.text_3 } else { t.text }).when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(name))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        let model = acc::model(cx);
                        if let Err(e) = model.update(cx, |m, cx| m.set_profile_pick(&pid, &slot, val.as_deref(), cx)) {
                            v.profiles.error = Some(e);
                        }
                        cx.notify();
                    })),
            );
        }
        let creating = self.profiles.creating.as_ref().filter(|(pid, slot, _)| *pid == p.id && *slot == r.slot).map(|(_, _, i)| i.clone());
        if let (None, Some(label)) = (&creating, new_label(&r.slot, cx)) {
            let (pid, slot) = (p.id.clone(), r.slot.clone());
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("profile-new-{}-{n}", p.id)))
                    .flex()
                    .items_center()
                    .h(px(26.))
                    .px(px(10.))
                    .rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(t.line_strong)
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .cursor_pointer()
                    .hover(|d| d.bg(t.surface_2))
                    .child(label)
                    .on_click(cx.listener(move |v, _, w, cx| v.start_creating(&pid, &slot, w, cx))),
            );
        }
        let mut col = div().flex_1().min_w_0().flex().flex_col().gap(px(6.)).child(chips);
        if let Some(input) = creating {
            let what = if acc::is_tool_kind(&r.slot) { "Create and log in" } else { "Create" };
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(200.)).child(input))
                    .child(crate::kit::text_button(SharedString::from(format!("profile-new-cancel-{n}")), "Cancel", BtnKind::Ghost, false, t).on_click(cx.listener(|v, _, _, cx| {
                        v.profiles.creating = None;
                        cx.notify();
                    })))
                    .child(crate::kit::text_button(SharedString::from(format!("profile-new-ok-{n}")), what, BtnKind::Primary, false, t).on_click(cx.listener(|v, _, _, cx| v.finish_creating(cx)))),
            );
        }
        // A connection picked that isn't set up yet: where to.
        if let (Some(ext), Some(cid)) = (core_acc::ext_of(&r.slot), current.as_deref().filter(|c| *c != core_acc::DEFAULT && *c != core_acc::NONE)) {
            let unset = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == ext).cloned().zip(acc::list(cx).and_then(|l| l.get(cid)).cloned()).filter(|(m, c)| {
                m.settings.iter().filter(|s| s.required).any(|s| match s.kind {
                    pitwall_core::extensions::SettingKind::Secret => !c.secrets_set.contains(&s.key),
                    _ => c.values.get(&s.key).is_none_or(|v| v.trim().is_empty()),
                })
            });
            if let Some((m, c)) = unset {
                let (ext, cid) = (ext.to_string(), c.id.clone());
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(crate::kit::hint(format!("{} isn't set up yet.", c.name), t))
                        .child(crate::kit::text_button(SharedString::from(format!("profile-setup-{n}")), format!("Set it up on {}'s card", m.name), BtnKind::Link, false, t).on_click(cx.listener(move |v, _, w, cx| {
                            v.exts_show_conn(&ext, &cid);
                            v.go_page(Page::Extensions, w, cx);
                        }))),
                );
            }
        }
        div().flex().items_start().gap(px(10.)).child(div().w(px(130.)).flex_none().pt(px(5.)).text_size(px(12.)).text_color(t.text_2).child(r.label.clone())).child(col)
    }
}
