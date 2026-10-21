//! Per-agent rules state, polled once for every pane header
//! (`src/rules/useAgentRules.ts`), the header's "Re-apply rules &
//! restart" button (`src/components/rules/RulesStaleButton.tsx`), and the
//! bulk "Re-apply & restart all" ([`affected`], [`reapply_all`]) that
//! Settings → Rules & skills offers.
//!
//! The poller reads `agent_rules` every 20 s while some header shows a
//! button, when a window comes to the front, and after rules change. Views
//! that drew a button are told when the state changes.

use crate::kit::HoverText as _;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::{
    div, prelude::*, px, AnyElement, App, AppContext, Context, Entity, EntityId, FontWeight,
    Global, SharedString,
};

use pitwall_core::rules::AgentRulesView;

use crate::kit;
use crate::theme::{Theme, RADIUS_SM};

/// Library files are edited outside Pitwall: re-check now and then.
pub const POLL: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct AgentRules {
    pub map: HashMap<String, AgentRulesView>,
    inflight: bool,
    /// Agents whose re-apply is running.
    busy: HashSet<String>,
    /// Views that drew a button since the last change.
    watchers: HashSet<EntityId>,
}

struct Poller(Entity<AgentRules>);

impl Global for Poller {}

impl AgentRules {
    /// Read every agent's rules now (one read at a time).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.inflight {
            return;
        }
        let Some(b) = super::backend(cx) else {
            return;
        };
        self.inflight = true;
        let task = cx
            .background_executor()
            .spawn(async move { b.agent_rules() });
        cx.spawn(async move |this, cx| {
            let list = task.await;
            let _ = this.update(cx, |p, cx| {
                p.inflight = false;
                p.set(list, cx);
            });
        })
        .detach();
    }

    fn set(&mut self, list: Vec<AgentRulesView>, cx: &mut Context<Self>) {
        let map: HashMap<String, AgentRulesView> =
            list.into_iter().map(|r| (r.agent_id.clone(), r)).collect();
        let shown =
            |m: &HashMap<String, AgentRulesView>| -> HashMap<String, (bool, Option<String>)> {
                m.iter()
                    .map(|(k, v)| (k.clone(), (v.stale, v.error.clone())))
                    .collect()
            };
        let changed = shown(&map) != shown(&self.map);
        self.map = map;
        if changed {
            self.changed(cx);
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        for id in std::mem::take(&mut self.watchers) {
            App::notify(cx, id);
        }
        cx.notify();
    }
}

/// Once, from [`super::install`]: the poller and its 20 s loop.
pub fn start(cx: &mut App) {
    let poll = cx.new(|_| AgentRules::default());
    cx.set_global(Poller(poll.clone()));
    let weak = poll.downgrade();
    cx.spawn(async move |cx| loop {
        let alive = weak.update(cx, |p, cx| {
            if !p.watchers.is_empty() {
                p.refresh(cx);
            }
        });
        if alive.is_err() {
            break;
        }
        cx.background_executor().timer(POLL).await;
        // Paused while no window shows (`document.hidden`).
        crate::platform::visible::until_any_visible(cx).await;
    })
    .detach();
    poll.update(cx, |p, cx| p.refresh(cx));
}

/// Re-read every agent's rules now (after changes, on window focus).
pub fn refresh(cx: &mut App) {
    if let Some(p) = cx.try_global::<Poller>().map(|p| p.0.clone()) {
        p.update(cx, |p, cx| p.refresh(cx));
    }
}

/// One agent's rules, as last read.
pub fn get(agent_id: &str, cx: &App) -> Option<AgentRulesView> {
    let p = cx.try_global::<Poller>()?;
    p.0.read(cx).map.get(agent_id).cloned()
}

/// Agents "Re-apply & restart all" would take (rules changed or failed
/// since their session started), sorted. The calling view is told when
/// that changes.
pub fn affected<V: 'static>(cx: &mut Context<V>) -> Vec<String> {
    let Some(poll) = cx.try_global::<Poller>().map(|p| p.0.clone()) else {
        return vec![];
    };
    let me = cx.entity_id();
    poll.update(cx, |p, _| {
        p.watchers.insert(me);
        let mut ids: Vec<String> = p
            .map
            .values()
            .filter(|v| v.needs_reapply())
            .map(|v| v.agent_id.clone())
            .collect();
        ids.sort();
        ids
    })
}

/// What "Re-apply & restart all" did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BulkOutcome {
    pub applied: usize,
    pub restarted: usize,
    /// (agent id, what went wrong).
    pub errors: Vec<(String, String)>,
}

/// "Re-apply & restart all": every affected agent's rules again, then a
/// restart of each one that is running (stopped ones use the rules when
/// they start). Their header buttons show "Applying…" meanwhile.
pub fn reapply_all<V: 'static>(
    view: gpui::WeakEntity<V>,
    cx: &mut App,
    on_done: impl FnOnce(&mut V, BulkOutcome, &mut Context<V>) + 'static,
) {
    let (Some(b), Some(poll)) = (
        super::backend(cx),
        cx.try_global::<Poller>().map(|p| p.0.clone()),
    ) else {
        return;
    };
    let ids: Vec<String> = poll.update(cx, |p, cx| {
        let ids: Vec<String> = p
            .map
            .values()
            .filter(|v| v.needs_reapply())
            .map(|v| v.agent_id.clone())
            .collect();
        p.busy.extend(ids.iter().cloned());
        p.changed(cx);
        ids
    });
    let task = cx.background_executor().spawn(async move {
        let mut out = BulkOutcome::default();
        let mut views = vec![];
        for r in b.reapply_stale(None) {
            if let Some(e) = r.error {
                out.errors.push((r.agent_id, e));
                continue;
            }
            out.applied += 1;
            match b.restart(&r.agent_id) {
                Ok(Some(v)) => {
                    out.restarted += 1;
                    views.push(v);
                }
                Ok(None) => {}
                Err(e) => out.errors.push((r.agent_id, format!("restart: {e}"))),
            }
        }
        (out, views)
    });
    cx.spawn(async move |cx| {
        let (out, views) = task.await;
        let _ = cx.update(|cx| {
            for v in views {
                crate::agents::patch(v, cx);
            }
        });
        let _ = poll.update(cx, |p, cx| {
            for id in &ids {
                p.busy.remove(id);
            }
            p.changed(cx);
            p.refresh(cx);
        });
        let _ = view.update(cx, |v, cx| on_done(v, out, cx));
    })
    .detach();
}

/// What to say after "Re-apply & restart all" (`name`: an agent's name
/// from its id); `false` when something failed.
pub fn bulk_message(o: &BulkOutcome, name: impl Fn(&str) -> String) -> (bool, String) {
    if o.applied == 0 && o.errors.is_empty() {
        return (true, "No agent's rules changed.".into());
    }
    let mut lines = vec![];
    if o.applied > 0 {
        let agents = if o.applied == 1 {
            "1 agent".to_string()
        } else {
            format!("{} agents", o.applied)
        };
        let mut line = format!("Re-applied rules for {agents}, restarted {}.", o.restarted);
        if o.restarted < o.applied {
            line.push_str(" Stopped agents use them when they start.");
        }
        lines.push(line);
    }
    for (id, e) in &o.errors {
        lines.push(format!("{}: {e}", name(id)));
    }
    (o.errors.is_empty(), lines.join("\n"))
}

/// The button's tooltip.
pub fn title(info: &AgentRulesView) -> String {
    match &info.error {
        Some(e) => format!("Rules not applied: {e}\nClick to try again and restart."),
        None => "Rules changed since this session started. Rules only reach new sessions: re-apply and restart (the conversation resumes when the agent supports it).".into(),
    }
}

/// The pane header's "Re-apply rules & restart" (`.rules-stale-btn`),
/// shown when the agent's rules changed or failed since its session
/// started. `on_done` gets `apply_rules`' outcome: restart on `Ok`, a toast
/// on `Err`.
pub fn button<V: 'static>(
    agent_id: &str,
    t: &Theme,
    cx: &mut Context<V>,
    on_done: impl Fn(&mut V, Result<(), String>, &mut Context<V>) + 'static,
) -> Option<AnyElement> {
    let poll = cx.try_global::<Poller>()?.0.clone();
    let me = cx.entity_id();
    let (info, busy) = poll.update(cx, |p, _| {
        p.watchers.insert(me);
        (p.map.get(agent_id).cloned(), p.busy.contains(agent_id))
    });
    let info = info.filter(|i| i.stale || i.error.is_some())?;
    let error = info.error.is_some();
    let (fg, bg, border) = if error {
        (t.red, t.red_soft, t.red)
    } else {
        (t.amber, t.amber_soft, t.amber_glow)
    };
    let id = agent_id.to_string();
    let weak = cx.entity().downgrade();
    let on_done = std::rc::Rc::new(on_done);
    Some(
        div()
            .id(SharedString::from(format!("rules-stale-{agent_id}")))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(22.))
            .px(px(8.))
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .when(busy, |d| d.opacity(0.6))
            .when(!busy, |d| {
                let hover = bg.blend(gpui::white().opacity(0.06));
                d.cursor_pointer().hover_probed(move |s| s.bg(hover))
            })
            .tooltip(kit::tooltip(title(&info)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                if busy {
                    return;
                }
                apply(id.clone(), poll.clone(), weak.clone(), on_done.clone(), cx);
            })
            .child(kit::icon("restart", 12., fg))
            .child(if busy {
                "Applying…"
            } else {
                "Re-apply rules & restart"
            })
            .into_any_element(),
    )
}

type Done<V> = std::rc::Rc<dyn Fn(&mut V, Result<(), String>, &mut Context<V>)>;

fn apply<V: 'static>(
    id: String,
    poll: Entity<AgentRules>,
    view: gpui::WeakEntity<V>,
    on_done: Done<V>,
    cx: &mut App,
) {
    let Some(b) = super::backend(cx) else {
        return;
    };
    poll.update(cx, |p, cx| {
        p.busy.insert(id.clone());
        p.changed(cx);
    });
    let agent = id.clone();
    let task = cx
        .background_executor()
        .spawn(async move { b.apply(&agent, false).map(|_| ()) });
    cx.spawn(async move |cx| {
        let r = task.await;
        let _ = poll.update(cx, |p, cx| {
            p.busy.remove(&id);
            p.changed(cx);
            p.refresh(cx);
        });
        let _ = view.update(cx, |v, cx| on_done(v, r, cx));
    })
    .detach();
}
