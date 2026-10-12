//! Settings → Agents → Accounts: more than one Claude Code or Codex login
//! (`pitwall_core::accounts`). Each kind lists Default (the user's own
//! login) and the accounts added, with who each is logged in as and the
//! projects whose new agents use it; "Add account" names one (a fresh
//! folder, or one already logged in) and "Log in" starts an agent on it,
//! where the tool's own login runs.

use std::rc::Rc;

use gpui::{div, prelude::*, px, Context, Div, Entity, SharedString, Subscription};

use super::view::{SettingsEvent, SettingsView};
use super::widgets::Ui;
use crate::accounts::{self as acc, AccountsModel};
use crate::kit::Ellipsis as _;
use crate::kit::{BtnKind, InputEvent, SegItem, TextInput};

/// The section's own state.
#[derive(Default)]
pub struct Section {
    /// The add form is open.
    adding: bool,
    /// Its kind (index into the kinds that can have accounts).
    kind: usize,
    name: Option<Entity<TextInput>>,
    dir: Option<Entity<TextInput>>,
    /// Renaming this account (with its input).
    renaming: Option<(String, Entity<TextInput>)>,
    /// Asking before removing this one.
    removing: Option<String>,
    /// Starting a login agent.
    busy: bool,
    error: Option<String>,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    fn open_add(&mut self, cx: &mut Context<Self>) {
        let name = cx.new(|cx| TextInput::new(cx, "", "Work"));
        let dir = cx.new(|cx| TextInput::new(cx, "", "Leave empty for a new login").mono());
        let sub = cx.subscribe(&name, |v, _, e: &InputEvent, cx| match e {
            InputEvent::Submit => v.add_account(true, cx),
            InputEvent::Cancel => {
                v.accounts.adding = false;
                cx.notify();
            }
            InputEvent::Changed => {
                v.accounts.error = None;
                cx.notify();
            }
            _ => {}
        });
        self.accounts._subs = vec![sub];
        self.accounts.name = Some(name);
        self.accounts.dir = Some(dir);
        self.accounts.adding = true;
        self.accounts.error = None;
        cx.notify();
    }

    /// Add the form's account; `login`: and start an agent on it to log in.
    fn add_account(&mut self, login: bool, cx: &mut Context<Self>) {
        let s = &self.accounts;
        let (Some(name), Some(dir)) = (&s.name, &s.dir) else { return };
        let (name, dir) = (name.read(cx).text().to_string(), dir.read(cx).text().to_string());
        let model = acc::model(cx);
        let Some(kind) = model.read(cx).kinds().get(s.kind).map(|k| k.id.clone()) else { return };
        let dir = (!dir.trim().is_empty()).then_some(dir);
        match model.update(cx, |m, cx| m.add(&kind, &name, dir.as_deref(), cx)) {
            Ok(a) => {
                self.accounts.adding = false;
                // A folder already logged in needs no login.
                if login && dir.is_none() {
                    self.log_in(&kind, &a.id, cx);
                }
            }
            Err(e) => self.accounts.error = Some(e),
        }
        cx.notify();
    }

    /// Start an agent of `kind` on account `id` in the home folder: the
    /// tool asks to log in on its first run. Handed to the window.
    fn log_in(&mut self, kind: &str, id: &str, cx: &mut Context<Self>) {
        let Some(engine) = cx.try_global::<super::SettingsHost>().and_then(|h| h.engine.clone()) else { return };
        let req = acc::login_request(kind, id, &pitwall_core::paths::home().to_string_lossy());
        self.accounts.busy = true;
        let task = cx.background_executor().spawn(async move { pitwall_core::engine::lifecycle::create(&engine, req) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |v, cx| {
                v.accounts.busy = false;
                match res {
                    Ok(view) => cx.emit(SettingsEvent::Started(Box::new(view))),
                    Err(e) => v.accounts.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_rename(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let input = cx.new(|cx| TextInput::new(cx, name, "Name"));
        let id2 = id.to_string();
        let sub = cx.subscribe(&input, move |v, input, e: &InputEvent, cx| match e {
            InputEvent::Submit | InputEvent::Blur => {
                let name = input.read(cx).text().to_string();
                let model = acc::model(cx);
                if let Err(e) = model.update(cx, |m, cx| m.rename(&id2, &name, cx)) {
                    v.accounts.error = Some(e);
                }
                v.accounts.renaming = None;
                cx.notify();
            }
            InputEvent::Cancel => {
                v.accounts.renaming = None;
                cx.notify();
            }
            _ => {}
        });
        self.accounts._subs.push(sub);
        self.accounts.renaming = Some((id.to_string(), input));
        cx.notify();
    }

    pub(super) fn accounts_section(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let model = acc::model(cx);
        let live = model.read(cx).live();
        let mut sec = div().flex().flex_col().gap(px(8.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .min_h(px(26.))
                .child(div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child("Accounts"))
                .child(div().flex_1())
                .when(live && !self.accounts.adding, |d| {
                    d.child(
                        crate::kit::text_button("acct-add", "Add account", BtnKind::Small, false, t)
                            .on_click(cx.listener(|v, _, _, cx| v.open_add(cx))),
                    )
                }),
        );
        sec = sec.child(crate::kit::muted(
            "More than one Claude Code or Codex login: each agent runs on one, a project can pick the one its new agents use, and an agent can move to another (its conversation goes with it).",
            t,
        ));
        if !live {
            return sec.child(crate::kit::hint("Accounts need Pitwall's engine, which isn't running.", t));
        }
        if self.accounts.adding {
            sec = sec.child(self.add_form(ui, cx));
        }
        // Every agent whose definition names a login folder.
        for k in model.read(cx).kinds() {
            sec = sec.child(
                div()
                    .pt(px(4.))
                    .text_size(px(11.))
                    .font_weight(crate::kit::WEIGHT_550)
                    .text_color(t.text_3)
                    .child(k.name.to_uppercase()),
            );
            sec = sec.child(self.account_row(ui, &k.id, None, cx));
            let list = model.read(cx).of_kind(&k.id);
            for a in list {
                sec = sec.child(self.account_row(ui, &k.id, Some(a), cx));
            }
        }
        if self.accounts.busy {
            sec = sec.child(crate::kit::hint("Starting an agent to log in…", t));
        }
        if let Some(e) = &self.accounts.error {
            sec = sec.child(crate::kit::dialog::form_error(e.clone(), t));
        }
        sec
    }

    fn add_form(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let s = &self.accounts;
        let e = cx.entity();
        let defs = acc::model(cx).read(cx).kinds();
        let kinds = crate::kit::seg(
            "acct-kind",
            defs.iter().enumerate().map(|(i, k)| SegItem::new(k.name.clone(), i == s.kind)).collect(),
            t,
            Rc::new(move |i, _, cx| {
                e.update(cx, |v, cx| {
                    v.accounts.kind = i;
                    cx.notify();
                })
            }),
        );
        let var = defs.get(s.kind).and_then(|k| k.account_env.clone()).unwrap_or_default();
        let empty = s.name.as_ref().is_none_or(|n| n.read(cx).text().trim().is_empty());
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(crate::theme::RADIUS)
            .border_1()
            .border_color(t.line_strong)
            .bg(t.surface_2)
            .child(div().flex().items_center().gap(px(8.)).child(crate::kit::muted("Tool", t)).child(div().flex_1()).child(kinds))
            .when_some(s.name.clone(), |d, n| {
                d.child(div().flex().items_center().gap(px(8.)).child(crate::kit::muted("Name", t)).child(div().flex_1()).child(div().w(px(260.)).child(n)))
            })
            .when_some(s.dir.clone(), |d, n| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(crate::kit::muted("Existing folder", t))
                        .child(div().flex_1())
                        .child(div().w(px(260.)).child(n)),
                )
            })
            .child(crate::kit::hint(
                format!("Pitwall runs its agents with {var} set to the account's folder. A new one asks you to log in the first time."),
                t,
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        crate::kit::text_button("acct-cancel", "Cancel", BtnKind::Ghost, false, t).on_click(cx.listener(|v, _, _, cx| {
                            v.accounts.adding = false;
                            cx.notify();
                        })),
                    )
                    .child(
                        crate::kit::text_button("acct-save", "Add and log in", BtnKind::Primary, empty, t)
                            .on_click(cx.listener(|v, _, _, cx| v.add_account(true, cx))),
                    ),
            )
    }

    /// One account (`None`: the kind's Default): name, login, folder,
    /// projects; Log in, Rename, Remove.
    fn account_row(&mut self, ui: &Ui, kind: &str, a: Option<pitwall_core::accounts::Account>, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let model = acc::model(cx);
        let id = a.as_ref().map(|a| a.id.clone());
        let login = model.update(cx, |m, _| m.login(kind, id.as_deref()));
        let dir = acc::folder_of(cx, kind, id.as_deref());
        let projects = id.as_deref().map(|i| model.read(cx).list.projects_of(i)).unwrap_or_default();
        let key: SharedString = format!("{kind}-{}", id.clone().unwrap_or_default()).into();
        let renaming = self.accounts.renaming.as_ref().filter(|(r, _)| Some(r) == id.as_ref()).map(|(_, i)| i.clone());
        let removing = id.is_some() && self.accounts.removing == id;
        let name = a.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "Default".into());
        let mut actions = div().flex().items_center().gap(px(6.)).flex_none();
        if let Some(id) = id.clone() {
            if removing {
                let (i1, i2) = (id.clone(), id.clone());
                actions = actions
                    .child(crate::kit::hint("Remove? Its folder stays.", t))
                    .child(
                        crate::kit::text_button(gpui::ElementId::Name(format!("acct-rm-yes-{key}").into()), "Remove", BtnKind::Danger, false, t).on_click(cx.listener(move |v, _, _, cx| {
                            let model = acc::model(cx);
                            if let Err(e) = model.update(cx, |m, cx| m.remove(&i1, cx)) {
                                v.accounts.error = Some(e);
                            }
                            v.accounts.removing = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        crate::kit::text_button(gpui::ElementId::Name(format!("acct-rm-no-{key}").into()), "Keep", BtnKind::Ghost, false, t).on_click(cx.listener(move |v, _, _, cx| {
                            let _ = &i2;
                            v.accounts.removing = None;
                            cx.notify();
                        })),
                    );
            } else {
                let (i1, i2, i3, n, k) = (id.clone(), id.clone(), id.clone(), name.clone(), kind.to_string());
                actions = actions
                    .child(
                        crate::kit::text_button(gpui::ElementId::Name(format!("acct-login-{key}").into()), if login.is_some() { "Log in again" } else { "Log in" }, BtnKind::Small, self.accounts.busy, t)
                            .on_click(cx.listener(move |v, _, _, cx| v.log_in(&k, &i1, cx))),
                    )
                    .child(
                        crate::kit::text_button(gpui::ElementId::Name(format!("acct-rename-{key}").into()), "Rename", BtnKind::Small, false, t)
                            .on_click(cx.listener(move |v, _, _, cx| v.start_rename(&i2, &n, cx))),
                    )
                    .child(
                        crate::kit::text_button(gpui::ElementId::Name(format!("acct-rm-{key}").into()), "Remove", BtnKind::Small, false, t).on_click(cx.listener(move |v, _, _, cx| {
                            v.accounts.removing = Some(i3.clone());
                            cx.notify();
                        })),
                    );
            }
        }
        let dot = if login.is_some() { t.green } else { t.text_4 };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(10.))
            .py(px(8.))
            .rounded(crate::theme::RADIUS)
            .border_1()
            .border_color(t.line)
            .child(div().size(px(7.)).rounded_full().bg(dot).flex_none())
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
                            .child(match renaming {
                                Some(input) => div().w(px(180.)).child(input).into_any_element(),
                                None => div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child(name).into_any_element(),
                            })
                            .child(div().text_size(px(12.)).text_color(if login.is_some() { t.text_2 } else { t.text_3 }).child(match &login {
                                Some(email) => email.clone(),
                                None if id.is_none() => "your own login".into(),
                                None => "not logged in".into(),
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .text_size(px(11.5))
                            .text_color(t.text_3)
                            .child(div().font_family(crate::kit::MONO_FONT).min_w_0().ellipsis().child(crate::kit::one_line(dir)))
                            .when(!projects.is_empty(), |d| {
                                let names: Vec<String> = projects
                                    .iter()
                                    .map(|p| p.trim_end_matches('/').rsplit('/').next().unwrap_or(p).to_string())
                                    .collect();
                                d.child(div().flex_none().child(format!("· new agents in {}", names.join(", "))))
                            }),
                    ),
            )
            .child(actions)
    }
}

/// The section's model, so the page redraws when accounts change.
pub fn observe(cx: &mut Context<SettingsView>) -> Subscription {
    let m: Entity<AccountsModel> = acc::model(cx);
    cx.observe(&m, |_, _, cx| cx.notify())
}
