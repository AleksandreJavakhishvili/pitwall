//! People (⌘⇧P, collaboration/ui.md §11): three tabs.
//! - **Directory**: shared agents I may use, grouped by owner: name,
//!   description, capability chip, "runs on Claude Code", what's left
//!   today, online dot. A click opens the Ask panel on that agent.
//! - **Teams**: my teams (fingerprint, members, my role, "bound" chip),
//!   then discovered ones ("Request to join"), "Join with a link…" and
//!   "Create team…". A team opens its page: members with devices and
//!   online state, admins (★), version and the last change, requests,
//!   invite, leave; for admins Add member… (also people without Pitwall,
//!   "Claude only") and the setup lines for them.
//! - **People**: everyone in my teams, their devices and shared agents;
//!   Block stops all of a person's messages to my agents.

use gpui::{div, prelude::*, px, App, AppContext, ClipboardItem, Context, Div, Entity, IntoElement, SharedString, Subscription, WeakEntity, Window};
use serde_json::json;

use pitwall_proto::api::collab::{capability_chip, InviteLink, MemberAdded, TeamDetail};
use pitwall_proto::collab::{DirectoryEntry, JoinPolicy, TeamCard, TeamRole};
use pitwall_proto::method;

use crate::kit::dialog::form_error;
use crate::kit::{check, chip, chip_tone, label_t, text_button, BtnKind, SegItem, TextInput, Tone, MONO_FONT};
use crate::main_screen::{MainScreen, Route};
use crate::theme::{self, Theme, RADIUS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Directory,
    Teams,
    People,
}

pub struct PeopleView {
    screen: WeakEntity<MainScreen>,
    pub tab: Tab,
    /// The team whose page is open.
    pub team: Option<TeamDetail>,
    pub invite: Option<InviteLink>,
    pub added: Option<MemberAdded>,
    link: Entity<TextInput>,
    new_team: Entity<TextInput>,
    new_listed: bool,
    member: Entity<TextInput>,
    pub error: Option<String>,
    _subs: Vec<Subscription>,
}

impl PeopleView {
    pub fn new(screen: WeakEntity<MainScreen>, _window: &mut Window, cx: &mut Context<Self>) -> PeopleView {
        let model = super::model_or_new(cx);
        let link = cx.new(|cx| TextInput::new(cx, "", "pitwall://team/join?… or a team's name"));
        let new_team = cx.new(|cx| TextInput::new(cx, "", "New team's name"));
        let member = cx.new(|cx| TextInput::new(cx, "", "Their login, e.g. erin@example.com"));
        let subs = vec![cx.observe(&model, |this, _, cx| {
            this.follow_team(cx);
            cx.notify();
        })];
        PeopleView { screen, tab: Tab::Directory, team: None, invite: None, added: None, link, new_team, new_listed: true, member, error: None, _subs: subs }
    }

    /// The open team's page, read again when the lists change.
    fn follow_team(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = &self.team {
            self.open_team(t.card.team.as_str().to_string(), cx);
        }
    }

    pub fn open_team(&mut self, team: String, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        super::call::<TeamDetail>(method::TEAM_SHOW, json!({"team": team}), cx, move |r, cx| {
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(d) => {
                        if v.team.as_ref() != Some(&d) {
                            v.team = Some(d);
                            cx.notify();
                        }
                    }
                    Err(e) => v.error = Some(e.message),
                }
            });
        });
    }

    fn done<T: 'static>(&self, cx: &mut Context<Self>) -> impl FnOnce(Result<T, pitwall_proto::ErrorBody>, &mut App) + 'static {
        let this = cx.entity().downgrade();
        move |r, cx| {
            let _ = this.update(cx, |v, cx| {
                v.error = r.err().map(|e| e.message);
                cx.notify();
            });
        }
    }

    fn create_team(&mut self, cx: &mut Context<Self>) {
        let name = self.new_team.read(cx).text().trim().to_string();
        if name.is_empty() {
            return;
        }
        self.new_team.update(cx, |i, cx| i.set_text("", cx));
        let this = cx.entity().downgrade();
        super::call::<TeamDetail>(method::TEAM_CREATE, json!({"name": name, "listed": self.new_listed}), cx, move |r, cx| {
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(d) => v.team = Some(d),
                    Err(e) => v.error = Some(e.message),
                }
                cx.notify();
            });
        });
    }

    fn join(&mut self, team: String, cx: &mut Context<Self>) {
        let done = self.done::<TeamCard>(cx);
        super::call::<TeamCard>(method::TEAM_JOIN, json!({"team": team}), cx, done);
    }

    fn add_member(&mut self, cx: &mut Context<Self>) {
        let (Some(team), login) = (self.team.clone(), self.member.read(cx).text().trim().to_string()) else { return };
        if login.is_empty() {
            return;
        }
        self.member.update(cx, |i, cx| i.set_text("", cx));
        let this = cx.entity().downgrade();
        super::call::<MemberAdded>(method::TEAM_ADD_MEMBER, json!({"team": team.card.team.as_str(), "login": login}), cx, move |r, cx| {
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(a) => {
                        v.team = Some(a.team.clone());
                        v.added = Some(a);
                        v.error = None;
                    }
                    Err(e) => v.error = Some(e.message),
                }
                cx.notify();
            });
        });
    }

    fn ask(&self, e: &DirectoryEntry, cx: &mut App) {
        let target = format!("{}/{}", super::short(&e.owner), e.slug);
        if let Some(m) = super::model(cx) {
            m.update(cx, |s, cx| {
                s.ask_target = Some(target);
                cx.notify();
            });
        }
        if let Some(s) = self.screen.upgrade() {
            s.update(cx, |s, cx| s.set_route(Route::Ask, cx));
        }
    }
}

fn hint(text: impl Into<SharedString>, t: &Theme) -> Div {
    div().text_size(px(12.)).text_color(t.text_3).child(text.into())
}

fn section_title(text: &str, t: &Theme) -> Div {
    div().pt(px(10.)).pb(px(4.)).child(label_t(text.to_string(), t.text_3, 11., 0.08))
}

fn card(t: &Theme) -> Div {
    div().flex().flex_col().gap(px(4.)).p(px(10.)).rounded(RADIUS).border_1().border_color(t.line).bg(t.surface)
}

fn dot(online: bool, t: &Theme) -> Div {
    div().flex_none().size(px(7.)).rounded_full().bg(if online { t.green } else { t.text_4 })
}

fn copy_line(id: impl Into<SharedString>, line: String, t: &Theme) -> Div {
    let text = line.clone();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().min_w_0().px(px(8.)).py(px(4.)).rounded(RADIUS).bg(t.surface_2).font_family(MONO_FONT).text_size(px(11.5)).child(line))
        .child(text_button(id.into(), "Copy", BtnKind::Small, false, t).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))))
}

impl PeopleView {
    fn directory(&mut self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let snap = super::snapshot(cx);
        let entries = snap.directory();
        let mut out = div().flex().flex_col().gap(px(6.));
        if entries.is_empty() {
            return out.child(hint("No shared agents yet. Join a team (Teams), or ask a teammate to share an agent with you.", t));
        }
        let mut owner = String::new();
        for (i, e) in entries.into_iter().enumerate() {
            if e.owner != owner {
                owner = e.owner.clone();
                let state = if e.online { String::new() } else { e.last_seen.as_deref().map(|at| format!(" · offline · {}", super::sidebar::ago(at))).unwrap_or(" · offline".into()) };
                out = out.child(div().flex().items_center().gap(px(6.)).pt(px(8.)).child(dot(e.online, t)).child(div().text_color(t.text_2).child(format!("{owner}{state}"))));
            }
            let name = e.card.get("name").and_then(|v| v.as_str()).unwrap_or(&e.slug).to_string();
            let description = e.card.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let left = e.left.map(|n| format!("{n} left today")).unwrap_or_default();
            let kind = super::kind_name(&e.kind, cx);
            let entry = e.clone();
            out = out.child(
                card(t)
                    .id(("dir", i))
                    .debug_selector(move || format!("dir-{i}"))
                    .cursor_pointer()
                    .when(!e.online, |d| d.opacity(0.55))
                    .hover(|s| s.border_color(t.line_strong))
                    .on_click(cx.listener(move |v, _, _, cx| v.ask(&entry, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().text_color(t.text).child(name))
                            .child(chip(capability_chip(e.capability), t))
                            .child(div().flex_1())
                            .child(hint(left, t)),
                    )
                    .when(!description.is_empty(), |d| d.child(hint(description, t)))
                    .child(hint(format!("{}/{} · runs on {kind}", super::short(&e.owner), e.slug), t)),
            );
        }
        out
    }

    fn teams(&mut self, t: &Theme, cx: &mut Context<Self>) -> Div {
        if let Some(d) = self.team.clone() {
            return self.team_page(&d, t, cx);
        }
        let snap = super::snapshot(cx);
        let (mine, found): (Vec<TeamCard>, Vec<TeamCard>) = snap.teams.into_iter().partition(|x| x.mine.is_some());
        let mut out = div().flex().flex_col().gap(px(6.)).child(section_title("MY TEAMS", t));
        if mine.is_empty() {
            out = out.child(hint("None yet.", t));
        }
        for (i, team) in mine.into_iter().enumerate() {
            let id = team.team.as_str().to_string();
            let role = match team.mine {
                Some(TeamRole::Admin) => "★ admin",
                Some(TeamRole::Requested) => "requested",
                _ => "member",
            };
            out = out.child(
                card(t)
                    .id(("team", i))
                    .cursor_pointer()
                    .hover(|s| s.border_color(t.line_strong))
                    .on_click(cx.listener(move |v, _, _, cx| v.open_team(id.clone(), cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().text_color(t.text).child(team.name.clone()))
                            .child(div().font_family(MONO_FONT).text_size(px(11.)).text_color(t.text_3).child(team.fingerprint.clone()))
                            .when(team.bound, |d| d.child(chip_tone("bound to a group", Tone::Subtle, t)))
                            .child(div().flex_1())
                            .child(hint(format!("{} members · {role}", team.members), t)),
                    )
                    .when(!team.description.is_empty(), |d| d.child(hint(team.description.clone(), t))),
            );
        }
        out = out.child(section_title("DISCOVERED", t));
        if found.is_empty() {
            out = out.child(hint("No listed teams found on your networks yet (Settings → Collaboration → Use teams).", t));
        }
        for (i, team) in found.into_iter().enumerate() {
            let id = team.team.as_str().to_string();
            let can = team.join == JoinPolicy::Request;
            out = out.child(
                card(t).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().text_color(t.text).child(team.name.clone()))
                        .child(div().font_family(MONO_FONT).text_size(px(11.)).text_color(t.text_3).child(team.fingerprint.clone()))
                        .child(hint(format!("by {} · {} members", team.admins.join(", "), team.members), t))
                        .child(div().flex_1())
                        .child(if can {
                            text_button(("join", i), "Request to join", BtnKind::Small, false, t).on_click(cx.listener(move |v, _, _, cx| v.join(id.clone(), cx))).into_any_element()
                        } else {
                            hint("closed: invite only", t).into_any_element()
                        }),
                ),
            );
        }
        let listed = self.new_listed;
        out.child(section_title("JOIN WITH A LINK…", t))
            .child(
                div().flex().gap(px(6.)).child(div().flex_1().child(self.link.clone())).child(text_button("join-link", "Join", BtnKind::Small, false, t).on_click(cx.listener(|v, _, _, cx| {
                    let link = v.link.read(cx).text().trim().to_string();
                    if !link.is_empty() {
                        v.link.update(cx, |i, cx| i.set_text("", cx));
                        v.join(link, cx);
                    }
                }))),
            )
            .child(section_title("CREATE TEAM…", t))
            .child(div().flex().gap(px(6.)).child(div().flex_1().child(self.new_team.clone())).child(text_button("create-team", "Create", BtnKind::Small, false, t).on_click(cx.listener(|v, _, _, cx| v.create_team(cx)))))
            .child(
                div()
                    .id("new-listed")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.new_listed = !v.new_listed;
                        cx.notify();
                    }))
                    .child(check(listed, t))
                    .child(hint("Listed: people on my networks can find it and ask to join", t)),
            )
    }

    fn team_page(&mut self, d: &TeamDetail, t: &Theme, cx: &mut Context<Self>) -> Div {
        let admin = d.card.mine == Some(TeamRole::Admin);
        let team = d.card.team.as_str().to_string();
        let mut out = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(text_button("team-back", "‹ Teams", BtnKind::Link, false, t).on_click(cx.listener(|v, _, _, cx| {
                v.team = None;
                v.invite = None;
                v.added = None;
                cx.notify();
            })))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label_t(d.card.name.clone(), t.text, 15., 0.02))
                    .child(div().font_family(MONO_FONT).text_size(px(12.)).text_color(t.text_3).child(d.card.fingerprint.clone()))
                    .when(d.card.bound, |x| x.child(chip_tone("bound to a group", Tone::Subtle, t))),
            )
            .child(hint(
                format!(
                    "version {}{}",
                    d.card.version,
                    d.last_change_by.as_deref().map(|by| format!(" · last change by {}{}", super::short(by), d.last_change_at.as_deref().map(|a| format!(" · {}", super::sidebar::ago(a))).unwrap_or_default())).unwrap_or_default()
                ),
                t,
            ))
            .when_some(d.conflict.clone(), |x, c| x.child(div().text_size(px(12.)).text_color(t.amber).child(c)));
        out = out.child(section_title("MEMBERS", t));
        for m in &d.members {
            let devices = m.devices.iter().map(|x| format!("{}{}", x.label, if x.online { "" } else { " (offline)" })).collect::<Vec<_>>().join(", ");
            let online = m.devices.iter().any(|x| x.online);
            out = out.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(dot(online, t))
                    .child(div().text_color(t.text).child(format!("{}{}", if m.admin { "★ " } else { "" }, m.login)))
                    .when(m.claude_only, |x| x.child(chip_tone("Claude only", Tone::Subtle, t)))
                    .child(div().flex_1())
                    .child(hint(if devices.is_empty() { "no devices".to_string() } else { devices }, t)),
            );
        }
        if admin && !d.requests.is_empty() {
            out = out.child(section_title("REQUESTS", t));
            for (i, r) in d.requests.iter().enumerate() {
                let (p1, p2, t1, t2) = (r.person.clone(), r.person.clone(), team.clone(), team.clone());
                out = out.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().child(format!("{} — {}{}", r.login, r.from, if r.note.is_empty() { String::new() } else { format!(": {}", r.note) })))
                        .child(text_button(("approve", i), "Approve", BtnKind::Small, false, t).on_click(cx.listener(move |v, _, _, cx| {
                            let done = v.done::<TeamDetail>(cx);
                            super::call::<TeamDetail>(method::TEAM_APPROVE, json!({"team": t1, "person": p1, "allow": true}), cx, done)
                        })))
                        .child(text_button(("decline", i), "Decline", BtnKind::Ghost, false, t).on_click(cx.listener(move |v, _, _, cx| {
                            let done = v.done::<TeamDetail>(cx);
                            super::call::<TeamDetail>(method::TEAM_APPROVE, json!({"team": t2, "person": p2, "allow": false}), cx, done)
                        }))),
                );
            }
        }
        if admin {
            out = out
                .child(section_title("ADD MEMBER…", t))
                .child(hint("By login, also people without Pitwall: they use your shared agents from plain Claude Code.", t))
                .child(div().flex().gap(px(6.)).child(div().flex_1().child(self.member.clone())).child(text_button("add-member", "Add", BtnKind::Small, false, t).on_click(cx.listener(|v, _, _, cx| v.add_member(cx)))));
            if let Some(a) = &self.added {
                out = out.child(hint(a.note.clone(), t));
            }
            let t3 = team.clone();
            out = out.child(section_title("INVITE", t)).child(div().flex().gap(px(6.)).child(text_button("invite", "Make an invite link", BtnKind::Small, false, t).on_click(cx.listener(move |_, _, _, cx| {
                let this = cx.entity().downgrade();
                super::call::<InviteLink>(method::TEAM_INVITE, json!({"team": t3}), cx, move |r, cx| {
                    let _ = this.update(cx, |v, cx| {
                        match r {
                            Ok(l) => v.invite = Some(l),
                            Err(e) => v.error = Some(e.message),
                        }
                        cx.notify();
                    });
                });
            }))));
            if let Some(l) = &self.invite {
                out = out.child(copy_line("invite-copy", l.link.clone(), t)).child(hint(format!("One use, until {}.", l.expires_at), t));
            }
        }
        out = out.child(section_title("SETUP FOR PEOPLE WITHOUT PITWALL", t));
        let setup = if d.setup.is_empty() { super::snapshot(cx).settings.map(|s| s.setup).unwrap_or_default() } else { d.setup.clone() };
        if setup.is_empty() {
            out = out.child(hint("Turn on Settings → Collaboration → “Let people on … reach this machine” to get the line to send them.", t));
        } else {
            out = out.child(hint("Send them these lines: their Claude Code then sees the shared agents as tools.", t));
            for (i, l) in setup.iter().enumerate() {
                out = out.child(copy_line(SharedString::from(format!("setup-{i}")), l.line.clone(), t));
            }
        }
        let leave = team.clone();
        out.child(div().pt(px(10.)).child(text_button("leave", "Leave team", BtnKind::Ghost, false, t).on_click(cx.listener(move |_, _, _, cx| {
            let this = cx.entity().downgrade();
            super::call::<serde_json::Value>(method::TEAM_LEAVE, json!({"team": leave}), cx, move |r, cx| {
                let _ = this.update(cx, |v, cx| {
                    match r {
                        Ok(_) => v.team = None,
                        Err(e) => v.error = Some(e.message),
                    }
                    cx.notify();
                });
            });
        }))))
    }

    fn people(&mut self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let snap = super::snapshot(cx);
        let mut out = div().flex().flex_col().gap(px(6.));
        if snap.people.is_empty() {
            return out.child(hint("Nobody yet: people in your teams show here.", t));
        }
        for (i, p) in snap.people.into_iter().enumerate() {
            let person = p.person.clone();
            let blocked = p.blocked;
            let online = p.devices.iter().any(|d| d.online);
            let devices = p.devices.iter().map(|d| d.label.clone()).collect::<Vec<_>>().join(", ");
            let shares = p.shares.iter().map(|s| s.slug.clone()).collect::<Vec<_>>().join(", ");
            out = out.child(
                card(t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(dot(online, t))
                            .child(div().text_color(if blocked { t.text_4 } else { t.text }).child(p.login.clone()))
                            .when(p.claude_only, |x| x.child(chip_tone("Claude only", Tone::Subtle, t)))
                            .when(blocked, |x| x.child(chip_tone("blocked", Tone::Warn, t)))
                            .child(div().flex_1())
                            .child(text_button(("block", i), if blocked { "Unblock" } else { "Block" }, BtnKind::Small, false, t).on_click(cx.listener(move |v, _, _, cx| {
                                let done = v.done::<serde_json::Value>(cx);
                                super::call::<serde_json::Value>(method::SHARE_BLOCK, json!({"person": person, "blocked": !blocked}), cx, done)
                            }))),
                    )
                    .child(hint(
                        format!(
                            "{} team(s){}{}",
                            p.teams.len(),
                            if devices.is_empty() { String::new() } else { format!(" · devices: {devices}") },
                            if shares.is_empty() { String::new() } else { format!(" · shares: {shares}") }
                        ),
                        t,
                    )),
            );
        }
        out
    }
}

impl gpui::Render for PeopleView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let tab = self.tab;
        let this = cx.entity().downgrade();
        let tabs = crate::kit::seg(
            "people-tabs",
            vec![SegItem::new("Directory", tab == Tab::Directory), SegItem::new("Teams", tab == Tab::Teams), SegItem::new("People", tab == Tab::People)],
            &t,
            std::rc::Rc::new(move |i, _, cx| {
                let _ = this.update(cx, |v, cx| {
                    v.tab = [Tab::Directory, Tab::Teams, Tab::People][i];
                    cx.notify();
                });
            }),
        );
        let off = super::snapshot(cx).settings.is_some_and(|s| !s.settings.use_teams);
        let body = match self.tab {
            Tab::Directory => self.directory(&t, cx),
            Tab::Teams => self.teams(&t, cx),
            Tab::People => self.people(&t, cx),
        };
        div()
            .id("people")
            .key_context("PwPeople")
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(12.))
                    .px(px(20.))
                    .h(px(44.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(label_t("PEOPLE", t.text, 13., 0.08))
                    .child(tabs)
                    .child(div().flex_1())
                    .child(text_button("people-refresh", "Refresh", BtnKind::GhostSm, false, &t).on_click(|_, _, cx| super::refresh(cx))),
            )
            .child(
                div()
                    .id("people-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.))
                    .py(px(12.))
                    .child(
                        div()
                            .max_w(px(760.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .when(off, |d| {
                                d.child(
                                    div().px(px(10.)).py(px(8.)).rounded(RADIUS).bg(t.surface_2).text_size(px(12.5)).text_color(t.text_2).child(
                                        "Collaboration is off: Settings → Collaboration → Use teams turns on discovery, joining and asking.",
                                    ),
                                )
                            })
                            .when_some(self.error.clone(), |d, e| d.child(form_error(e, &t)))
                            .child(body),
                    ),
            )
    }
}
