//! Settings → Accounts: more than one login per agent tool, for the tools
//! that keep their login in a folder they let you choose (Claude Code,
//! Codex, Gemini CLI, Copilot CLI: `pitwall_core::accounts`). Each
//! installed tool (or one with accounts) lists Default (the user's own
//! login) and the accounts added, with who each is logged in as, its usage
//! where the tool records it (`pitwall_core::usage`), and the projects
//! whose new agents use it; "Add account" names one (a fresh
//! folder, or one already logged in) and "Log in" starts an agent on it,
//! where the tool's own login runs.
//!
//! Then, as a section of its own, the git identities (`pitwall_core::accounts::GitIdentity`): who
//! commits, and the GitHub and Azure DevOps logins their CLIs and HTTPS
//! pushes use, for the projects that name one; each login's "Log in" opens
//! a terminal where `gh` or `az` logs in.

use std::rc::Rc;

use gpui::{div, prelude::*, px, Context, Div, Entity, SharedString, Subscription};

use super::view::{SettingsEvent, SettingsView};
use super::widgets::Ui;
use crate::accounts::{self as acc, AccountsModel};
use pitwall_core::accounts::{GitFields, GitIdentity, GitService};
use crate::kit::Ellipsis as _;
use crate::kit::HoverText as _;
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
    /// The git identity form (adding one, or editing `GitForm::id`).
    git_form: Option<GitForm>,
    /// Asking before removing this git identity.
    git_removing: Option<String>,
    _subs: Vec<Subscription>,
}

/// The add / edit form of a git identity.
struct GitForm {
    /// The one being edited; `None`: a new one.
    id: Option<String>,
    name: Entity<TextInput>,
    user_name: Entity<TextInput>,
    email: Entity<TextInput>,
    ssh_key: Entity<TextInput>,
    error: Option<String>,
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

    /// Open the git identity form: empty (the user's own name filled in),
    /// or `g`'s to edit.
    fn open_git_form(&mut self, g: Option<&GitIdentity>, cx: &mut Context<Self>) {
        let own = if g.is_none() { acc::own_git_name() } else { String::new() };
        let name = cx.new(|cx| TextInput::new(cx, g.map(|g| g.name.as_str()).unwrap_or(""), "Work"));
        let user_name = cx.new(|cx| TextInput::new(cx, g.map(|g| g.user_name.as_str()).unwrap_or(&own), "Your Name"));
        let email = cx.new(|cx| TextInput::new(cx, g.map(|g| g.email.as_str()).unwrap_or(""), "you@company.com"));
        let ssh_key = cx.new(|cx| TextInput::new(cx, g.and_then(|g| g.ssh_key.as_deref()).unwrap_or(""), "Optional: ~/.ssh/id_ed25519_work").mono());
        let mut subs = Vec::new();
        for input in [&name, &user_name, &email, &ssh_key] {
            subs.push(cx.subscribe(input, |v, _, e: &InputEvent, cx| match e {
                InputEvent::Submit => v.save_git(cx),
                InputEvent::Cancel => {
                    v.accounts.git_form = None;
                    cx.notify();
                }
                InputEvent::Changed => {
                    if let Some(f) = v.accounts.git_form.as_mut() {
                        f.error = None;
                    }
                    cx.notify();
                }
                _ => {}
            }));
        }
        self.accounts._subs.extend(subs);
        self.accounts.git_form = Some(GitForm { id: g.map(|g| g.id.clone()), name, user_name, email, ssh_key, error: None });
        cx.notify();
    }

    /// Save the form: a new identity (its logins are made from its row),
    /// or the edited one.
    fn save_git(&mut self, cx: &mut Context<Self>) {
        let Some(f) = &self.accounts.git_form else { return };
        let fields = GitFields {
            name: f.name.read(cx).text().to_string(),
            user_name: f.user_name.read(cx).text().to_string(),
            email: f.email.read(cx).text().to_string(),
            ssh_key: f.ssh_key.read(cx).text().to_string(),
        };
        let id = f.id.clone();
        let model = acc::model(cx);
        let res = match &id {
            Some(id) => model.update(cx, |m, cx| m.edit_git(id, &fields, cx)),
            None => model.update(cx, |m, cx| m.add_git(&fields, cx)).map(|_| ()),
        };
        match res {
            Ok(_) => self.accounts.git_form = None,
            Err(e) => {
                if let Some(f) = self.accounts.git_form.as_mut() {
                    f.error = Some(e);
                }
            }
        }
        cx.notify();
    }

    /// Open a terminal where git identity `id` logs in to `service`.
    fn git_log_in(&mut self, id: &str, service: GitService, cx: &mut Context<Self>) {
        let Some(engine) = cx.try_global::<super::SettingsHost>().and_then(|h| h.engine.clone()) else { return };
        let req = acc::git_login_request(id, service);
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

    /// Git identities: header with "Add git identity", the form, the rows.
    pub(super) fn git_section(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let adding = self.accounts.git_form.as_ref().is_some_and(|f| f.id.is_none());
        let mut sec = div().flex().flex_col().gap(px(8.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(4.))
                .min_h(px(26.))
                .child(div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child("Git identities"))
                .child(div().flex_1())
                .when(self.accounts.git_form.is_none(), |d| {
                    d.child(
                        crate::kit::text_button("git-add", "Add git identity", BtnKind::Small, false, t)
                            .on_click(cx.listener(|v, _, _, cx| v.open_git_form(None, cx))),
                    )
                }),
        );
        sec = sec.child(crate::kit::muted(
            "Who a project's agents and terminals are in git: their commits' name and email, their GitHub and Azure DevOps logins (for gh, az and HTTPS pushes) and an SSH key. Projects without one, and services it isn't logged in to, use your own setup. Applies at each start: restart running agents to switch.",
            t,
        ));
        if adding {
            sec = sec.child(self.git_form(ui, cx));
        }
        let list = acc::model(cx).read(cx).git();
        if list.is_empty() && !adding {
            sec = sec.child(crate::kit::hint("None yet: every project uses your own git setup.", t));
        }
        for g in list {
            let editing = self.accounts.git_form.as_ref().is_some_and(|f| f.id.as_deref() == Some(g.id.as_str()));
            sec = sec.child(if editing { self.git_form(ui, cx) } else { self.git_row(ui, &g, cx) });
        }
        sec
    }

    fn git_form(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let Some(f) = &self.accounts.git_form else { return div() };
        let row = |label: &'static str, input: Entity<TextInput>| {
            div().flex().items_center().gap(px(8.)).child(crate::kit::muted(label, t)).child(div().flex_1()).child(div().w(px(260.)).child(input))
        };
        let editing = f.id.is_some();
        let empty = [&f.name, &f.user_name, &f.email].iter().any(|i| i.read(cx).text().trim().is_empty());
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(crate::theme::RADIUS)
            .border_1()
            .border_color(t.line_strong)
            .bg(t.surface_2)
            .child(row("Name", f.name.clone()))
            .child(row("Commit name", f.user_name.clone()))
            .child(row("Commit email", f.email.clone()))
            .child(row("SSH key", f.ssh_key.clone()))
            .child(crate::kit::hint(
                "Agents and terminals get GIT_AUTHOR_* and GIT_COMMITTER_*, and with a key GIT_SSH_COMMAND. Log it in to GitHub or Azure DevOps from its row: then they get that login's GH_CONFIG_DIR or AZURE_CONFIG_DIR too, and HTTPS pushes there use it.",
                t,
            ))
            .when_some(f.error.clone(), |d, e| d.child(crate::kit::dialog::form_error(e, t)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        crate::kit::text_button("git-cancel", "Cancel", BtnKind::Ghost, false, t).on_click(cx.listener(|v, _, _, cx| {
                            v.accounts.git_form = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        crate::kit::text_button("git-save", if editing { "Save" } else { "Add" }, BtnKind::Primary, empty, t)
                            .on_click(cx.listener(|v, _, _, cx| v.save_git(cx))),
                    ),
            )
    }

    /// One git identity: name, who commits, GitHub login, key, projects;
    /// Log in to GitHub, Edit, Remove.
    fn git_row(&mut self, ui: &Ui, g: &GitIdentity, cx: &mut Context<Self>) -> Div {
        let t = &ui.t;
        let model = acc::model(cx);
        let logins: Vec<(GitService, Option<String>)> = GitService::ALL.iter().map(|&s| (s, model.update(cx, |m, _| m.git_login(&g.id, s)))).collect();
        let any_login = logins.iter().any(|(_, l)| l.is_some());
        let projects = model.read(cx).list.git_projects_of(&g.id);
        let key: SharedString = g.id.clone().into();
        let removing = self.accounts.git_removing.as_deref() == Some(g.id.as_str());
        let mut actions = div().flex().items_center().gap(px(6.)).flex_none();
        if removing {
            let id = g.id.clone();
            actions = actions
                .child(crate::kit::hint("Remove? Its GitHub login stays on disk.", t))
                .child(
                    crate::kit::text_button(gpui::ElementId::Name(format!("git-rm-yes-{key}").into()), "Remove", BtnKind::Danger, false, t).on_click(cx.listener(move |v, _, _, cx| {
                        let model = acc::model(cx);
                        if let Err(e) = model.update(cx, |m, cx| m.remove_git(&id, cx)) {
                            v.accounts.error = Some(e);
                        }
                        v.accounts.git_removing = None;
                        cx.notify();
                    })),
                )
                .child(
                    crate::kit::text_button(gpui::ElementId::Name(format!("git-rm-no-{key}").into()), "Keep", BtnKind::Ghost, false, t).on_click(cx.listener(|v, _, _, cx| {
                        v.accounts.git_removing = None;
                        cx.notify();
                    })),
                );
        } else {
            let (i3, g2) = (g.id.clone(), g.clone());
            actions = actions
                .child(
                    crate::kit::text_button(gpui::ElementId::Name(format!("git-edit-{key}").into()), "Edit", BtnKind::Small, false, t)
                        .on_click(cx.listener(move |v, _, _, cx| v.open_git_form(Some(&g2), cx))),
                )
                .child(
                    crate::kit::text_button(gpui::ElementId::Name(format!("git-rm-{key}").into()), "Remove", BtnKind::Small, false, t).on_click(cx.listener(move |v, _, _, cx| {
                        v.accounts.git_removing = Some(i3.clone());
                        cx.notify();
                    })),
                );
        }
        let line = |id: String, text: String, mono: bool| {
            div()
                .id(gpui::ElementId::Name(id.into()))
                .min_w_0()
                .ellipsis()
                .text_size(px(11.5))
                .text_color(t.text_3)
                .when(mono, |d| d.font_family(crate::kit::MONO_FONT))
                .tooltip(crate::kit::tooltip(text.clone()))
                .child(crate::kit::one_line(text))
        };
        let who = format!("{} <{}>", g.user_name, g.email);
        // One line per service: who it's logged in as, and its Log in.
        let busy = self.accounts.busy;
        let login_lines: Vec<Div> = logins
            .into_iter()
            .map(|(service, login)| {
                let text = match &login {
                    Some(u) => format!("{}: {u}", service.label()),
                    None => format!("{}: not logged in", service.label()),
                };
                let id = g.id.clone();
                let label = if login.is_some() { "Log in again" } else { "Log in" };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .child(line(format!("git-{}-{key}", service.label()), text, false))
                    .child(
                        div()
                            .id(gpui::ElementId::Name(format!("git-login-{}-{key}", service.label()).into()))
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(t.text_2)
                            .when(!busy, |d| d.cursor_pointer().hover_text(t.text, |s| s.underline()).on_click(cx.listener(move |v, _, _, cx| v.git_log_in(&id, service, cx))))
                            .child(label),
                    )
            })
            .collect();
        let dot = if any_login { t.green } else { t.text_4 };
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
                            .min_w_0()
                            .child(div().flex_none().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child(g.name.clone()))
                            .child(
                                div()
                                    .id(gpui::ElementId::Name(format!("git-who-{key}").into()))
                                    .min_w_0()
                                    .ellipsis()
                                    .text_size(px(12.))
                                    .text_color(t.text_2)
                                    .tooltip(crate::kit::tooltip(who.clone()))
                                    .child(crate::kit::one_line(who)),
                            ),
                    )
                    .children(login_lines)
                    .when_some(g.ssh_key.clone(), |d, k| d.child(line(format!("git-key-{key}"), format!("SSH key {k}"), true)))
                    .when(!projects.is_empty(), |d| {
                        let names: Vec<String> = projects.iter().map(|p| p.trim_end_matches('/').rsplit('/').next().unwrap_or(p).to_string()).collect();
                        d.child(line(format!("git-proj-{key}"), format!("Used in {}", names.join(", ")), false))
                    }),
            )
            .child(actions)
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
                .child(div().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child("Agent logins"))
                .child(div().flex_1())
                .when(live && !self.accounts.adding, |d| {
                    d.child(
                        crate::kit::text_button("acct-add", "Add account", BtnKind::Small, false, t)
                            .on_click(cx.listener(|v, _, _, cx| v.open_add(cx))),
                    )
                }),
        );
        sec = sec.child(crate::kit::muted(
            "More than one login per tool: each agent runs on one, a project can pick the one its new agents use, and an agent can move to another (its conversation goes with it, for Claude Code and Codex). Usage is read from what each tool records on this Mac: Codex keeps its plan's limits, Claude Code its tokens.",
            t,
        ));
        if !live {
            return sec.child(crate::kit::hint("Accounts need Pitwall's engine, which isn't running.", t));
        }
        if self.accounts.adding {
            sec = sec.child(self.add_form(ui, cx));
        }
        // Every agent whose definition names a login folder.
        for k in model.read(cx).shown_kinds() {
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
            sec = sec.child(crate::kit::hint("Starting a login…", t));
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
        let usage = model.update(cx, |m, cx| m.usage(kind, id.as_deref(), false, cx));
        let readable = model.read(cx).kind(kind).map(|d| pitwall_core::accounts::logged_in(&d, None).is_some()).unwrap_or(false);
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
                            .min_w_0()
                            .child(match renaming {
                                Some(input) => div().w(px(180.)).flex_none().child(input).into_any_element(),
                                None => div().flex_none().font_weight(crate::kit::WEIGHT_550).text_color(t.text).child(name).into_any_element(),
                            })
                            .child({
                                // Long logins end in "…"; the whole one is in the tooltip.
                                let shown: SharedString = match &login {
                                    Some(email) => email.clone().into(),
                                    None if id.is_none() => "your own login".into(),
                                    None if readable => "not logged in".into(),
                                    // Its login may be in the keychain.
                                    None => "log in to use it".into(),
                                };
                                div()
                                    .id(gpui::ElementId::Name(format!("acct-email-{key}").into()))
                                    .min_w_0()
                                    .ellipsis()
                                    .text_size(px(12.))
                                    .text_color(if login.is_some() { t.text_2 } else { t.text_3 })
                                    .tooltip(crate::kit::tooltip(shown.clone()))
                                    .child(crate::kit::one_line(shown))
                            }),
                    )
                    .child(
                        div()
                            .id(gpui::ElementId::Name(format!("acct-dir-{key}").into()))
                            .min_w_0()
                            .ellipsis()
                            .text_size(px(11.5))
                            .text_color(t.text_3)
                            .font_family(crate::kit::MONO_FONT)
                            .tooltip(crate::kit::tooltip(dir.clone()))
                            .child(crate::kit::one_line(dir)),
                    )
                    .when_some(usage, |d, u| d.child(usage_line(&key, &u, t)))
                    .when(!projects.is_empty(), |d| {
                        let names: Vec<String> = projects
                            .iter()
                            .map(|p| p.trim_end_matches('/').rsplit('/').next().unwrap_or(p).to_string())
                            .collect();
                        d.child(
                            div()
                                .min_w_0()
                                .ellipsis()
                                .text_size(px(11.5))
                                .text_color(t.text_3)
                                .child(crate::kit::one_line(format!("New agents in {}", names.join(", ")))),
                        )
                    }),
            )
            .child(actions)
    }
}

/// An account's usage: its plan's limits (Codex: "5h 25% · resets 14:20",
/// "week 40%") as bars, then its tokens in the last 5 hours, day and week.
fn usage_line(key: &SharedString, u: &pitwall_core::usage::Usage, t: &crate::theme::Theme) -> Div {
    use pitwall_core::usage::short;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut col = div().flex().flex_col().gap(px(3.)).pt(px(2.));
    for (i, l) in u.limits.iter().enumerate() {
        let what = match l.window_minutes {
            300 => "5h limit".to_string(),
            10080 => "Weekly limit".to_string(),
            m if m % 1440 == 0 => format!("{}-day limit", m / 1440),
            m if m % 60 == 0 => format!("{}h limit", m / 60),
            m => format!("{m} min limit"),
        };
        let pct = l.used_percent.clamp(0., 100.);
        let color = if pct >= 90. { t.red } else if pct >= 70. { t.amber } else { t.green };
        let resets = l.resets_at.filter(|r| *r > now).map(|r| format!(" · resets in {}", until(r - now))).unwrap_or_default();
        col = col.child(
            div()
                .id(gpui::ElementId::Name(format!("acct-limit-{key}-{i}").into()))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.5))
                .text_color(t.text_3)
                .child(div().w(px(84.)).flex_none().child(what))
                .child(
                    div()
                        .w(px(120.))
                        .h(px(5.))
                        .flex_none()
                        .rounded_full()
                        .bg(t.surface_3)
                        .child(div().h_full().rounded_full().bg(color).w(px(120. * pct as f32 / 100.))),
                )
                .child(div().min_w_0().ellipsis().child(crate::kit::one_line(format!("{pct:.0}% used{resets}")))),
        );
    }
    let tokens = if u.last_7d.total() == 0 {
        "No use in the last week".to_string()
    } else {
        format!("Tokens: {} in 5h · {} today · {} this week", short(u.last_5h.total()), short(u.last_24h.total()), short(u.last_7d.total()))
    };
    let tip = format!(
        "This week: {} input, {} output, {} from the cache, {} written to it.{}",
        short(u.last_7d.input),
        short(u.last_7d.output),
        short(u.last_7d.cache_read),
        short(u.last_7d.cache_write),
        u.plan.as_ref().map(|p| format!(" Plan: {p}.")).unwrap_or_default()
    );
    col.child(
        div()
            .id(gpui::ElementId::Name(format!("acct-tokens-{key}").into()))
            .debug_selector({
                let k = key.clone();
                move || format!("acct-tokens-{k}")
            })
            .min_w_0()
            .ellipsis()
            .text_size(px(11.5))
            .text_color(t.text_3)
            .tooltip(crate::kit::tooltip(tip))
            .child(crate::kit::one_line(tokens)),
    )
}

/// "2h 05m", "3d 4h", "12m".
fn until(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        m if m >= 1440 => format!("{}d {}h", m / 1440, m % 1440 / 60),
        m if m >= 60 => format!("{}h {:02}m", m / 60, m % 60),
        m => format!("{m}m"),
    }
}

/// The section's model, so the page redraws when accounts change.
pub fn observe(cx: &mut Context<SettingsView>) -> Subscription {
    let m: Entity<AccountsModel> = acc::model(cx);
    cx.observe(&m, |_, _, cx| cx.notify())
}
