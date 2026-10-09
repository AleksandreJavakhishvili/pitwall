//! Settings → Agents → Race Engineer (docs/spec/engineer.md): what it runs
//! on (`engineer.agent`: automatic, any agent New agent offers, or a custom
//! command, `engineer.command`) and its first prompt (`engineer.greeting`).

use std::rc::Rc;

use gpui::{div, prelude::*, px, App, Context, Div, Entity, SharedString, Subscription};
use pitwall_core::engineer::AUTO;
use pitwall_core::kind::CUSTOM;
use pitwall_core::model::KindView;
use pitwall_proto::settings::find;
use serde_json::Value;

use super::view::SettingsView;
use super::widgets::Ui;
use super::{data, SettingsHost};
use crate::kit::{InputEvent, TextInput};

/// The section's own state.
#[derive(Default)]
pub struct Section {
    /// What New agent offers (read once, in the background).
    kinds: Option<Vec<KindView>>,
    reading: bool,
    open: bool,
    command: Option<Entity<TextInput>>,
    greeting: Option<Entity<TextInput>>,
    _subs: Vec<Subscription>,
}

fn value(cx: &App, key: &str) -> String {
    find(key)
        .and_then(|def| data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Set `key` in `ui.json` when the registry allows the value.
fn save(cx: &mut App, key: &str, v: &str) {
    let Some(def) = find(key) else { return };
    let Ok(v) = def.validate(&Value::String(v.trim().to_string())) else { return };
    if data::value(&crate::ui_state::get(cx), def).as_ref() != Some(&v) {
        crate::ui_state::update(cx, |s| *s = data::with(s, def, &v));
    }
}

/// The agents it can run on, in New agent's order (no terminals; custom last).
fn choices(kinds: &[KindView]) -> Vec<&KindView> {
    let mut v: Vec<&KindView> = kinds.iter().filter(|k| k.id != CUSTOM && k.id != "shell").collect();
    v.extend(kinds.iter().filter(|k| k.id == CUSTOM));
    v
}

fn label(k: &KindView) -> String {
    if k.installed || k.id == CUSTOM {
        k.name.clone()
    } else {
        format!("{} (not installed)", k.name)
    }
}

impl SettingsView {
    fn engineer_input(&mut self, key: &'static str, placeholder: &str, cx: &mut Context<Self>) -> Entity<TextInput> {
        let text = value(cx, key);
        let input = cx.new(|cx| TextInput::new(cx, &text, placeholder));
        let sub = cx.subscribe(&input, move |_, input, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Changed | InputEvent::Blur | InputEvent::Submit) {
                let text = input.read(cx).text().to_string();
                save(cx, key, &text);
            }
        });
        self.engineer._subs.push(sub);
        input
    }

    fn read_engineer_kinds(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = cx.try_global::<SettingsHost>().and_then(|h| h.engine.clone()) else { return };
        self.engineer.reading = true;
        let read = cx.background_executor().spawn(async move { engine.list_kinds() });
        cx.spawn(async move |this, cx| {
            let kinds = read.await.unwrap_or_default();
            let _ = this.update(cx, |v, cx| {
                v.engineer.kinds = Some(kinds);
                cx.notify();
            });
        })
        .detach();
    }

    /// The Race Engineer section.
    pub(super) fn engineer(&mut self, ui: &Ui, head: Div, row: impl Fn(&str) -> Div, cx: &mut Context<Self>) -> Div {
        if self.engineer.kinds.is_none() && !self.engineer.reading {
            self.read_engineer_kinds(cx);
        }
        let agent = value(cx, "engineer.agent");
        let kinds = self.engineer.kinds.clone().unwrap_or_default();
        let list = choices(&kinds);
        let auto_label = match list.iter().find(|k| k.id == "claude" && k.installed).or(list.iter().find(|k| k.installed && k.id != CUSTOM)) {
            Some(k) => format!("Automatic ({})", k.name),
            None => "Automatic".to_string(),
        };
        let mut items: Vec<(SharedString, bool)> = vec![(auto_label.clone().into(), agent == AUTO || agent.is_empty())];
        items.extend(list.iter().map(|k| (label(k).into(), k.id == agent)));
        let ids: Vec<String> = std::iter::once(AUTO.to_string()).chain(list.iter().map(|k| k.id.clone())).collect();
        let current: SharedString = match items.iter().find(|(_, on)| *on) {
            Some((l, _)) => l.clone(),
            None => agent.clone().into(),
        };
        let this = cx.entity().downgrade();
        let (t1, t2, t3) = (this.clone(), this.clone(), this);
        let select = crate::kit::select(
            "engineer-agent",
            current,
            26.,
            12.,
            false,
            self.engineer.open,
            items,
            &ui.t,
            move |_, cx| {
                let _ = t1.update(cx, |v, cx| {
                    v.engineer.open = !v.engineer.open;
                    cx.notify();
                });
            },
            Rc::new(move |i, _, cx| {
                if let Some(id) = ids.get(i) {
                    save(cx, "engineer.agent", id);
                }
                let _ = t2.update(cx, |v, cx| {
                    v.engineer.open = false;
                    cx.notify();
                });
            }),
            Rc::new(move |_, cx| {
                let _ = t3.update(cx, |v, cx| {
                    v.engineer.open = false;
                    cx.notify();
                });
            }),
        );
        if self.engineer.greeting.is_none() {
            let g = self.engineer_input("engineer.greeting", "None", cx);
            self.engineer.greeting = Some(g);
        }
        let mut sec = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(head.child(crate::kit::chip_tone("optional", crate::kit::Tone::Subtle, &ui.t)))
            .child(crate::kit::muted(
                "An assistant agent for Pitwall itself: agw setup, spaces, agents per project, rules and settings, through the pitwall CLI. Risky steps still wait for your OK here. It opens from the top bar or ⌘K; nothing runs until you open it.",
                &ui.t,
            ))
            .child(row("Runs on").child(select));
        if agent == CUSTOM {
            if self.engineer.command.is_none() {
                let c = self.engineer_input("engineer.command", "my-agent --flag", cx);
                self.engineer.command = Some(c);
            }
            if let Some(c) = &self.engineer.command {
                sec = sec.child(row("Command").child(div().w(px(300.)).font_family(crate::kit::MONO_FONT).child(c.clone())));
            }
        }
        if let Some(g) = &self.engineer.greeting {
            sec = sec.child(row("First prompt").child(div().w(px(380.)).child(g.clone())));
        }
        sec.child(crate::kit::hint("Changes apply the next time it starts. It works in its own folder in Pitwall's data folder.", &ui.t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::model::KindCaps;

    fn k(id: &str, installed: bool) -> KindView {
        KindView { id: id.into(), name: id.to_uppercase(), installed, path: None, worktree: false, caps: KindCaps::default() }
    }

    #[test]
    fn every_agent_but_terminals_custom_last() {
        let kinds = vec![k("claude", true), k("custom", true), k("shell", true), k("gemini", false)];
        let ids: Vec<&str> = choices(&kinds).iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids, ["claude", "gemini", "custom"]);
        assert_eq!(label(&kinds[3]), "GEMINI (not installed)");
        assert_eq!(label(&kinds[1]), "CUSTOM");
    }
}
