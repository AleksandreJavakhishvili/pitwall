//! Settings → Accounts, after the accounts themselves:
//!
//! - **Extension connections** (only once an extension has one): each
//!   extension's connections by name, the default marked, with the
//!   profiles and projects that use each; read-only, "Edit in Extensions"
//!   opens the extension's card, the one place they are set up.
//! - **Profiles** (docs/spec/profiles.md): named bundles ("Work",
//!   "Personal") of one pick per slot that has something to pick (a
//!   tool's login, a git identity, a browser account, a connection per
//!   extension). Add, rename, delete (its projects keep what they run
//!   under); each slot's choices as chips: "Not set" (the project decides,
//!   else the default), the default, each account, and for an extension
//!   None. A change to a profile projects use applies as a pick in their
//!   Accounts… does (`accounts::PicksChanged`).

use gpui::{div, prelude::*, px, Context, Div, Entity, FontWeight, SharedString};

use super::view::SettingsView;
use super::widgets::Ui;
use crate::accounts::{self as acc, SlotRow};
use crate::kit::Ellipsis as _;
use crate::kit::{BtnKind, InputEvent, TextInput};
use pitwall_core::accounts::Profile;

/// The sections' own state.
#[derive(Default)]
pub struct Section {
    /// A profile being named: the one renamed (`None`: a new one) and its
    /// field.
    naming: Option<(Option<String>, Entity<TextInput>)>,
    /// Asking before deleting this profile.
    deleting: Option<String>,
    /// Profiles unfolded (a new one is, until folded).
    open: std::collections::HashSet<String>,
    error: Option<String>,
    _subs: Vec<gpui::Subscription>,
}

/// A project folder's last part.
fn short(p: &str) -> String {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or(p).to_string()
}

impl SettingsView {
    /// An extension has a connection: its section shows.
    pub(super) fn has_connections(cx: &gpui::App) -> bool {
        acc::list(cx).is_some_and(|l| l.accounts.iter().any(|a| pitwall_core::accounts::ext_of(&a.kind).is_some()))
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

    /// Profiles: what one is, each profile's card, Add profile.
    pub(super) fn profiles_section(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let model = acc::model(cx);
        let live = model.read(cx).live();
        let list = model.read(cx).list.clone();
        let adding = matches!(self.profiles.naming, Some((None, _)));
        let mut sec = div().flex().flex_col().gap(px(8.)).child(
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
            "A profile bundles one of each of the accounts above under a name, like Work or Personal. A project picks a profile in its Accounts… instead of each account, and can still change single ones.",
            t,
        ));
        if !live {
            return sec.child(crate::kit::hint("Profiles need Pitwall's engine, which isn't running.", t));
        }
        let rows = acc::slot_rows(cx);
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
            sec = sec.child(div().p(px(12.)).rounded(px(10.)).border_1().border_dashed().border_color(t.line).child(crate::kit::muted(
                if rows.is_empty() { "No profiles. Add a second login, a git identity, a browser account or an extension connection above first: a profile picks among them." } else { "No profiles yet. Without one, each project picks its accounts itself, as now." },
                t,
            )));
        }
        for p in &list.profiles {
            sec = sec.child(self.profile_card(ui, p, &rows, cx));
        }
        if let Some(e) = &self.profiles.error {
            sec = sec.child(crate::kit::dialog::form_error(e.clone(), t));
        }
        sec
    }

    /// One profile: its name and who uses it; folded, what it picks in a
    /// line; unfolded, each slot's chips.
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
        let summary: Vec<String> = rows
            .iter()
            .filter_map(|r| {
                let v = p.picks.get(&r.slot)?;
                let name = r.choices.iter().find(|(c, _)| c == v).map(|(_, n)| n.clone())?;
                Some(format!("{}: {name}", r.label))
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
        let line = |text: String| div().min_w_0().ellipsis().text_size(px(11.5)).text_color(t.text_3).child(crate::kit::one_line(text));
        let mut sub: Vec<String> = Vec::new();
        if self.profiles.deleting.as_deref() == Some(&p.id) {
            sub.push(if projects.is_empty() { "Delete it? Its accounts stay.".into() } else { format!("Delete it? Its accounts stay; {} keep what they run under as their own picks.", projects.join(", ")) });
        } else if !projects.is_empty() {
            sub.push(format!("Used by {}", projects.join(", ")));
        } else {
            sub.push("No project uses it yet: pick it in a project's Accounts…".into());
        }
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
                    .gap(px(1.))
                    .child(match renaming {
                        Some(input) => div().w(px(220.)).child(input).into_any_element(),
                        None => div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child(p.name.clone()).into_any_element(),
                    })
                    .children(sub.into_iter().map(line))
                    .when(!open && !summary.is_empty(), |d| d.child(line(summary.join(" · "))))
                    .when(!open && summary.is_empty(), |d| d.child(line("Picks nothing yet: unfold it to choose.".into()))),
            )
            .child(actions);
        let mut card = div().flex().flex_col().gap(px(10.)).px(px(10.)).py(px(8.)).rounded(crate::theme::RADIUS).border_1().border_color(t.line).child(head);
        if open {
            let mut grid = div().flex().flex_col().gap(px(10.)).pl(px(26.)).pb(px(4.));
            if rows.is_empty() {
                grid = grid.child(crate::kit::hint("There is nothing to pick yet: add accounts above.", t));
            }
            for (n, r) in rows.iter().enumerate() {
                grid = grid.child(self.profile_slot(ui, p, r, n, cx));
            }
            card = card.child(grid);
        }
        card
    }

    /// A profile's slot: its label, then "Not set" and each choice as
    /// chips, its pick ticked.
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
        div().flex().items_start().gap(px(10.)).child(div().w(px(130.)).flex_none().pt(px(5.)).text_size(px(12.)).text_color(t.text_2).child(r.label.clone())).child(div().flex_1().min_w_0().child(chips))
    }
}
