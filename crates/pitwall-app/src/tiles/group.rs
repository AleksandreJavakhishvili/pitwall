//! The agents' questions group in the app (docs/spec/tile-sdk/
//! platform-services.md §14.2): what tiles reach through it and where it
//! shows.
//!
//! - `agents.list` and `agents.queue-add` go to the daemon's method table
//!   as the extension, as its adapter's calls do ([`AppMethods`]): the
//!   same permissions, limits and Activity (marked "tile"). A queued item
//!   lands in the agent's Next up marked "From <extension>", waiting for
//!   Send.
//! - `agents-changed` goes to each running component when what a tile
//!   sees of the agents changes ([`fingerprint`]); the host drops it for
//!   one not granted `agents.read`.
//! - Badges ([`Kept::badges`]) are drawn on sidebar rows beside the
//!   adapters' ([`marks_for_agent`], [`marks_for_project`]), at most four
//!   sets a second.
//! - Questions ([`Kept::asks`]) are cards in the floating control
//!   ([`float_asks`]); an answer goes back as `ask-answered`
//!   ([`super::Tiles::answer`]), and the tile's `reply` says whether it
//!   was taken.
//!
//! All of it is kept with the component while it is suspended or let go,
//! and goes with it when its last view closes or the extension is turned
//! off.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gpui::{App, EventEmitter};
use pitwall_core::extensions::activity::Via;
use pitwall_core::extensions::view::{Open, RowBadge, Target, Tone};
use pitwall_daemon::Server;
use pitwall_proto::AgentView;
use pitwall_tile_host::{ui, MethodError, Methods};

use crate::extensions::RowMark;
use crate::float::model::AskItem;

/// The daemon's method table for tiles, as their extension.
#[derive(Default)]
pub struct AppMethods {
    server: Mutex<Option<Arc<Server>>>,
}

impl AppMethods {
    /// Answer with `server` from now on (none: every call fails).
    pub fn set_server(&self, server: Option<Arc<Server>>) {
        if let Ok(mut s) = self.server.lock() {
            *s = server;
        }
    }
}

impl Methods for AppMethods {
    fn call(&self, extension: &str, method: &str, params: serde_json::Value) -> Result<serde_json::Value, MethodError> {
        let server = self.server.lock().ok().and_then(|s| s.clone());
        crate::extensions::calls::answer_via(server.as_deref(), extension, method, params, Via::Tile).map_err(|e| MethodError { code: e.code, message: e.message })
    }
}

/// What a component's tiles show outside their views.
#[derive(Debug, Default, Clone)]
pub struct Kept {
    /// Its badges on sidebar rows now.
    pub badges: Vec<ui::Badge>,
    /// When its badges were last applied, and a newer set waiting.
    pub(super) badges_at: Option<Instant>,
    pub(super) badges_wait: Option<Vec<ui::Badge>>,
    /// Its open questions.
    pub asks: Vec<ui::Ask>,
    /// Answers on their way to it: question id → request.
    pub busy: HashMap<String, u64>,
    /// Why it refused the last answer to a question.
    pub notes: HashMap<String, String>,
    /// How each answer went: request → taken.
    pub results: HashMap<u64, bool>,
}

impl Kept {
    /// Its questions now: answers on their way and notes kept only for
    /// those still open.
    pub fn set_asks(&mut self, asks: Vec<ui::Ask>) {
        self.busy.retain(|id, _| asks.iter().any(|a| a.id == *id));
        self.notes.retain(|id, _| asks.iter().any(|a| a.id == *id));
        self.asks = asks;
    }

    /// Its reply to the answer to question `id`.
    pub fn replied(&mut self, id: &str, refused: Option<String>) {
        let Some(req) = self.busy.remove(id) else { return };
        self.results.insert(req, refused.is_none());
        match refused {
            Some(why) => {
                self.notes.insert(id.to_string(), why);
            }
            None => {
                self.notes.remove(id);
            }
        }
    }
}

/// Emitted by [`super::Tiles`] when a component's badges change: the
/// sidebar draws them again.
pub struct MarksChanged;

impl EventEmitter<MarksChanged> for super::Tiles {}

/// What tiles see of the agents, as a number: when it changes, they hear
/// `agents-changed`.
pub fn fingerprint(agents: &[AgentView]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for a in agents {
        (&a.id, &a.name, &a.kind, &a.project, &a.branch, a.status as u8, a.queue.len()).hash(&mut h);
    }
    h.finish()
}

fn tone(t: ui::Tone) -> Tone {
    match t {
        ui::Tone::Info => Tone::None,
        ui::Tone::Good => Tone::Good,
        ui::Tone::Warn => Tone::Warn,
        ui::Tone::Bad => Tone::Bad,
    }
}

/// A tile's badge as a sidebar row's mark.
pub fn row_badge(b: &ui::Badge) -> RowBadge {
    RowBadge {
        target: match &b.row {
            ui::Row::Agent(id) => Target::Agent(id.clone()),
            ui::Row::Project(p) => Target::Project(p.clone()),
        },
        count: b.count,
        tone: tone(b.tone),
        icon: None,
        note: b.note.clone().unwrap_or_default(),
        tooltip: b.tooltip.clone().unwrap_or_default(),
        open: b.open.clone().map(|view| Open::View { view, item: None }),
    }
}

/// Each running component's badges that its extension may draw now (on,
/// with the grants its rows need): (its manifest, the badge).
fn marks(cx: &App) -> Vec<RowMark> {
    let Some(t) = super::try_tiles(cx) else { return vec![] };
    let ui_state = crate::ui_state::get(cx);
    let mut out = Vec::new();
    for (id, r) in &t.read(cx).running {
        if r.group.badges.is_empty() || !ui_state.extension_on(id) {
            continue;
        }
        let Some(m) = crate::extensions::manifest_of(id, cx).filter(|m| m.contributes.badges) else { continue };
        let grants = crate::extensions::calls::effective(&m, &ui_state.extension(id).grants);
        for b in &r.group.badges {
            let badge = row_badge(b);
            if crate::extensions::hub::views::allowed(&m, &grants, &badge).is_ok() {
                out.push(RowMark { ext: m.id.clone(), name: m.name.clone(), badge });
            }
        }
    }
    out.sort_by(|a, b| a.ext.cmp(&b.ext));
    out
}

/// Tiles' badges on agent `a`'s row.
pub fn marks_for_agent(a: &AgentView, cx: &App) -> Vec<RowMark> {
    marks(cx).into_iter().filter(|m| matches!(&m.badge.target, Target::Agent(id) if *id == a.id)).collect()
}

/// Tiles' badges on project `path`'s row.
pub fn marks_for_project(path: &str, cx: &App) -> Vec<RowMark> {
    marks(cx).into_iter().filter(|m| matches!(&m.badge.target, Target::Project(p) if p == path)).collect()
}

/// The thread a tile's question is known by in the floating control (its
/// cards are keyed by extension, thread and number).
pub fn thread_of(id: &str) -> String {
    format!("tile:{id}")
}

/// The open questions of the running components whose extensions are on
/// and contribute `asks`, as the floating control's cards: blocking ones
/// first. `agents` names the agent each is from.
pub fn float_asks(agents: &[AgentView], cx: &App) -> Vec<AskItem> {
    use pitwall_core::extensions::protocol::{Ask, Suggestion};
    let Some(t) = super::try_tiles(cx) else { return vec![] };
    let ui_state = crate::ui_state::get(cx);
    let mut ids: Vec<&String> = t.read(cx).running.keys().collect();
    ids.sort();
    let mut out = Vec::new();
    for id in ids {
        let r = &t.read(cx).running[id];
        if r.group.asks.is_empty() || !ui_state.extension_on(id) {
            continue;
        }
        let Some(m) = crate::extensions::manifest_of(id, cx).filter(|m| m.contributes.asks) else { continue };
        let Some(tile) = m.contributes.tiles.first() else { continue };
        let icon = tile.icon.clone().unwrap_or_else(|| crate::extensions::icon_of(&m).to_string());
        for a in &r.group.asks {
            let from = a.from.as_ref().and_then(|aid| agents.iter().find(|x| x.id == *aid)).map(|x| x.name.clone());
            out.push(AskItem {
                ext: m.id.clone(),
                ext_name: m.name.clone(),
                icon: icon.clone(),
                conn: None,
                thread: thread_of(&a.id),
                title: a.title.clone(),
                from,
                ask: Ask {
                    number: 0,
                    question: a.question.clone(),
                    multiple: a.multiple,
                    open: true,
                    suggestions: a.suggestions.iter().map(|s| Suggestion { label: s.label.clone(), description: s.description.clone().unwrap_or_default() }).collect(),
                    answer: None,
                    withdrawn: None,
                    blocking: a.blocking,
                },
                busy: r.group.busy.contains_key(&a.id),
                note: r.group.notes.get(&a.id).cloned(),
                tile: Some((tile.id.clone(), tile.title.clone())),
            });
        }
    }
    out
}

/// Whether the answer `req` to a question of extension `ext`'s tiles was
/// taken (`None`: no reply yet).
pub fn ask_result(ext: &str, req: u64, cx: &App) -> Option<bool> {
    super::try_tiles(cx)?.read(cx).running.get(ext)?.group.results.get(&req).copied()
}

/// The user answered the question of extension `ext`'s tiles known in the
/// floating control as `thread` ([`thread_of`]): the answer's number
/// ([`super::Tiles::answer`]).
pub fn answer(ext: &str, thread: &str, picks: Vec<u32>, text: Option<String>, declined: bool, cx: &mut App) -> Option<u64> {
    let ask = thread.strip_prefix("tile:")?;
    super::try_tiles(cx)?.update(cx, |t, cx| t.answer(ext, ask, picks, text, declined, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_settles_the_answer_and_keeps_a_refusal() {
        let ask = |id: &str| ui::Ask { id: id.into(), from: None, title: String::new(), question: "Q?".into(), suggestions: vec![], multiple: false, blocking: false };
        let mut k = Kept::default();
        k.set_asks(vec![ask("a"), ask("b")]);
        k.busy.insert("a".into(), 1);
        k.replied("a", Some("not yet".into()));
        assert_eq!((k.results.get(&1), k.notes.get("a").map(String::as_str)), (Some(&false), Some("not yet")));
        k.busy.insert("a".into(), 2);
        k.replied("a", None);
        assert_eq!((k.results.get(&2), k.notes.get("a")), (Some(&true), None));
        k.notes.insert("b".into(), "x".into());
        k.set_asks(vec![ask("a")]);
        assert!(k.notes.is_empty(), "a question that went takes its note along");
        // A reply to nothing on its way changes nothing.
        k.replied("zzz", Some("?".into()));
        assert_eq!(k.results.len(), 2);
    }

    #[test]
    fn a_badge_reads_as_a_row_mark() {
        let b = ui::Badge { row: ui::Row::Project("/p".into()), count: Some(3), note: None, tone: ui::Tone::Bad, tooltip: Some("3 failing".into()), open: Some("main".into()) };
        let r = row_badge(&b);
        assert_eq!((r.target, r.count, r.tone, r.note.as_str(), r.tooltip.as_str()), (Target::Project("/p".into()), Some(3), Tone::Bad, "", "3 failing"));
        assert_eq!(r.open, Some(Open::View { view: "main".into(), item: None }));
    }
}
