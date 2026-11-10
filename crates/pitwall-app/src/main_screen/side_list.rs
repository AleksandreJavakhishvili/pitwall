//! The sidebar's agents as a virtual list (docs/spec/perf.md "GPUI").
//!
//! A scroll step notifies the view the sidebar is drawn in; drawn as one
//! tall scroll container, every row of it was built and laid out again on
//! each step, off-screen ones too. Instead the sidebar is a flat list of
//! rows ([`Row`]: machine, account and project headings, agent rows, quiet
//! summaries, gaps…) drawn by `gpui::list`, which builds and lays out only
//! the rows on screen and keeps the others' heights.
//!
//! The rows are made again from the sidebar's sections when the screen
//! changes (not on a scroll step), then compared with the last ones: rows
//! that came or went are spliced into the list's state, rows whose height
//! may have changed are measured again ([`Entry::sig`]); the rest keep
//! their heights.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use gpui::{div, prelude::*, px, AnyElement, Context, Focusable as _, ListAlignment, ListState, SharedString, Window};
use pitwall_proto::{AgentView, Status};

use super::model::{machine_level, summarize, MachineSection};
use super::MainScreen;
use crate::agents::ProjectGroup;
use crate::kit::{icon, tooltip, HoverText as _};
use crate::theme::{Theme, RADIUS};

/// Where a project group is: its machine, its account section (grouped by
/// account) and its place there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct At {
    m: usize,
    s: Option<usize>,
    g: usize,
}

/// One row of the sidebar's list.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Row {
    /// Room between rows (px).
    Gap(f32),
    /// A machine's heading (2+ machines).
    Machine { m: usize, collapsed: bool },
    /// A quiet line under a heading: what a folded machine or project
    /// holds, or why a machine shows nothing. `faint`: a machine's.
    Note { text: String, faint: bool, indent: bool },
    Account { m: usize, s: usize, collapsed: bool },
    Project { at: At, collapsed: bool },
    /// A project without agents: one dashed "New agent" row.
    Empty(At),
    /// Agent `i` of its group, with its worktrees chip and rows;
    /// `index`: its place among all agents (⌘1–9).
    Agent { at: At, i: usize, index: usize },
    /// A project's "N other worktrees" chip and rows.
    Others(At),
    /// Below the agents: Elsewhere and the other machines' card.
    Tail,
}

/// A row, what it is (`id`, the same while it is shown) and what its
/// height depends on (`sig`).
#[derive(Clone, Debug)]
pub(super) struct Entry {
    row: Row,
    id: u64,
    sig: u64,
}

/// The sidebar list's rows and state.
pub(super) struct SideList {
    state: ListState,
    sections: Rc<Vec<MachineSection>>,
    rows: Rc<Vec<Entry>>,
    /// The screen changed since the rows were made.
    dirty: bool,
    /// Bring this agent's row into view at the next draw.
    reveal: Option<String>,
    /// Made-again count: the tail's `sig` (it is measured each time).
    builds: u64,
    /// The row of the name being edited and its field (kept drawn while
    /// it has focus, scrolled away or not).
    editing: Option<(usize, gpui::EntityId)>,
    /// Agent rows built (tests).
    #[cfg(test)]
    pub(super) agent_rows: usize,
}

impl SideList {
    pub(super) fn new() -> SideList {
        SideList {
            // Rows a little past the edges are measured, not drawn; every
            // row is measured once, so the scrollbar is right from the start.
            state: ListState::new(0, ListAlignment::Top, px(120.)).measure_all(),
            sections: Rc::default(),
            rows: Rc::default(),
            dirty: true,
            reveal: None,
            builds: 0,
            editing: None,
            #[cfg(test)]
            agent_rows: 0,
        }
    }

    /// The screen changed: make the rows again before the next draw.
    pub(super) fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Scroll agent `id`'s row into view at the next draw.
    pub(super) fn reveal(&mut self, id: &str) {
        self.reveal = Some(id.to_string());
    }

    pub(super) fn state(&self) -> &ListState {
        &self.state
    }

    /// Take the new rows: splice the ones that came or went, measure again
    /// the ones whose height may have changed.
    fn take(&mut self, sections: Vec<MachineSection>, rows: Vec<Entry>) {
        let old = &self.rows;
        let pre = old.iter().zip(&rows).take_while(|(a, b)| a.id == b.id).count();
        let most = old.len().min(rows.len()) - pre;
        let suf = old.iter().rev().zip(rows.iter().rev()).take(most).take_while(|(a, b)| a.id == b.id).count();
        let (gone, came) = (pre..old.len() - suf, rows.len() - pre - suf);
        let mut changed: Vec<usize> = (0..pre).filter(|&i| old[i].sig != rows[i].sig).collect();
        changed.extend((0..suf).filter_map(|k| {
            let (o, n) = (old.len() - 1 - k, rows.len() - 1 - k);
            (old[o].sig != rows[n].sig).then_some(n)
        }));
        changed.sort_unstable();
        let spliced = !gone.is_empty() || came > 0;
        if spliced {
            self.state.splice(gone.clone(), came);
            // Spliced rows are measured with the others' (`measure_all`).
            self.state.remeasure_items(pre..pre + came);
            if self.editing.is_some_and(|(i, _)| i >= pre) {
                self.editing = None;
            }
        }
        for r in runs(&changed) {
            self.state.remeasure_items(r);
        }
        self.sections = Rc::new(sections);
        self.rows = Rc::new(rows);
    }

    fn group(&self, at: At) -> &ProjectGroup {
        let m = &self.sections[at.m];
        match at.s {
            None => &m.groups[at.g],
            Some(s) => &m.accounts[s].groups[at.g],
        }
    }

    /// The row of agent `id`.
    fn row_of(&self, id: &str) -> Option<usize> {
        self.rows.iter().position(|e| match e.row {
            Row::Agent { at, i, .. } => self.group(at).agents[i].id == id,
            _ => false,
        })
    }
}

/// Contiguous runs of sorted indices.
fn runs(ix: &[usize]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for &i in ix {
        match out.last_mut() {
            Some(r) if r.end == i => r.end += 1,
            _ => out.push(i..i + 1),
        }
    }
    out
}

fn hash(x: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

/// Rows as they are made.
#[derive(Default)]
struct Rows(Vec<Entry>);

impl Rows {
    fn push(&mut self, row: Row, id: impl Hash, sig: impl Hash) {
        self.0.push(Entry { row, id: hash(id), sig: hash(sig) });
    }

    /// A gap: known by the row above it.
    fn gap(&mut self, h: f32) {
        let above = self.0.last().map(|e| e.id);
        self.push(Row::Gap(h), ("gap", above), h.to_bits());
    }
}

impl MainScreen {
    /// The sidebar's rows from its machine sections, in the order (and with
    /// the room between them) the sidebar always had.
    fn side_rows(&self, sections: &[MachineSection]) -> Vec<Entry> {
        let mut rows = Rows::default();
        // The list's own padding (`pt 6`).
        rows.gap(6.);
        let by_agent = crate::review::model::worktrees_by_agent(&self.wt.projects);
        let mut index = 0usize;
        // This Mac alone: no machine level, as before there were others.
        let level = machine_level(sections);
        for (mi, m) in sections.iter().enumerate() {
            if level {
                let collapsed = self.ui.collapsed.contains(&m.key);
                if mi > 0 {
                    rows.gap(8.);
                }
                rows.push(Row::Machine { m: mi, collapsed }, ("m", &m.key), collapsed);
                let all = m.agents();
                let quiet = if collapsed {
                    index += all.len();
                    (!all.is_empty()).then(|| summarize(&all))
                } else if m.groups.is_empty() && m.accounts.is_empty() {
                    // A paired machine with nothing to show.
                    let connected = m.link.as_ref().is_some_and(|l| l.state == pitwall_proto::peer::LinkState::Connected);
                    Some(if connected { "No agents running" } else { "Its agents show once it's reachable" }.to_string())
                } else {
                    None
                };
                if let Some(q) = quiet {
                    rows.push(Row::Note { text: q.clone(), faint: true, indent: false }, ("nm", &m.key), q);
                }
                if collapsed {
                    continue;
                }
            }
            self.side_groups(&mut rows, &m.groups, mi, None, &mut index, &by_agent);
            for (si, sec) in m.accounts.iter().enumerate() {
                let collapsed = self.ui.collapsed.contains(&sec.key);
                if si > 0 {
                    rows.gap(6.);
                }
                rows.push(Row::Account { m: mi, s: si, collapsed }, ("a", &sec.key), collapsed);
                if collapsed {
                    index += sec.agents;
                } else {
                    // Its projects sit a step in, under the account.
                    self.side_groups(&mut rows, &sec.groups, mi, Some(si), &mut index, &by_agent);
                }
            }
        }
        // Whatever it holds now is measured again each time.
        rows.push(Row::Tail, "tail", self.side.builds);
        // `pb 8`.
        rows.gap(8.);
        rows.0
    }

    /// Project groups and their agents' rows (`index`: agents so far).
    fn side_groups(
        &self,
        rows: &mut Rows,
        groups: &[ProjectGroup],
        m: usize,
        s: Option<usize>,
        index: &mut usize,
        by_agent: &HashMap<String, Vec<crate::review::model::WtRef>>,
    ) {
        for (gi, g) in groups.iter().enumerate() {
            let collapsed = self.ui.collapsed.contains(&g.key);
            let at = At { m, s, g: gi };
            let start = *index;
            *index += g.agents.len();
            // `.project-group + .project-group { margin-top: 4px }`
            if gi > 0 {
                rows.gap(4.);
            }
            rows.push(Row::Project { at, collapsed }, ("p", &g.key), collapsed);
            if g.agents.is_empty() {
                if !collapsed {
                    rows.push(Row::Empty(at), ("e", &g.key), ());
                }
            } else if collapsed {
                let text = summarize(&g.agents);
                rows.push(Row::Note { text: text.clone(), faint: false, indent: s.is_some() }, ("ng", &g.key), text);
            } else {
                for (i, a) in g.agents.iter().enumerate() {
                    let wts = by_agent.get(&a.id).map_or(0, Vec::len);
                    rows.push(Row::Agent { at, i, index: start + i }, ("r", &g.key, &a.id), self.agent_sig(a, wts));
                }
                let ids: Vec<&str> = g.agents.iter().map(|a| a.id.as_str()).collect();
                let others = crate::review::model::other_worktrees(&self.wt.projects, &ids).len();
                rows.push(Row::Others(at), ("o", &g.key), others);
            }
        }
    }

    /// What an agent row's height depends on: a "needs you" line, the name
    /// field and its error, its worktrees.
    fn agent_sig(&self, a: &AgentView, worktrees: usize) -> impl Hash {
        let blocked = a.status == Status::Blocked;
        let rename = self.agent_rename.as_ref().filter(|r| r.id == a.id).map(|r| r.error.clone());
        (blocked.then(|| a.status_detail.clone()), rename, worktrees)
    }

    /// Make the rows again if the screen changed, and scroll a revealed
    /// agent into view.
    fn side_sync(&mut self, cx: &mut Context<Self>) {
        if self.side.dirty || self.side.rows.is_empty() {
            self.side.dirty = false;
            self.side.builds += 1;
            let sections = self.machine_sections(self.agents(cx), cx);
            let rows = self.side_rows(&sections);
            self.side.take(sections, rows);
            self.side_keep_editing(cx);
        }
        if self.side.state.viewport_bounds().size.height <= px(0.) {
            // Not laid out yet: once it is.
            return;
        }
        if let Some(id) = self.side.reveal.take() {
            if let Some(ix) = self.side.row_of(&id) {
                self.side.state.scroll_to_reveal_item(ix);
            }
        }
    }

    /// The row whose name is being edited stays drawn while its field has
    /// focus, scrolled out of view or not (a field not drawn loses it).
    fn side_keep_editing(&mut self, cx: &Context<Self>) {
        let Some(r) = &self.agent_rename else {
            self.side.editing = None;
            return;
        };
        let Some(ix) = self.side.row_of(&r.id) else { return };
        let input = r.input.entity_id();
        if self.side.editing != Some((ix, input)) {
            let handle = r.input.focus_handle(cx);
            self.side.state.splice_focusable(ix..ix + 1, [Some(handle)]);
            self.side.state.remeasure_items(ix..ix + 1);
            self.side.editing = Some((ix, input));
        }
    }

    /// The sidebar's list: only the rows on screen are built.
    pub(super) fn side_list(&mut self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::List {
        self.side_sync(cx);
        let focused = self.focused_agent_id();
        let kb = self.sidebar_focus.is_focused(window);
        let t = t.clone();
        let screen = cx.entity().downgrade();
        gpui::list(self.side.state.clone(), move |ix, _, cx| {
            screen
                .update(cx, |s, cx| s.side_row(ix, &focused, kb, &t, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full()
    }

    /// Row `ix` of the list.
    fn side_row(
        &mut self,
        ix: usize,
        focused: &Option<String>,
        kb: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.side.rows.clone();
        let Some(e) = rows.get(ix) else {
            return div().into_any_element();
        };
        let sections = self.side.sections.clone();
        let (el, indent): (AnyElement, bool) = match &e.row {
            Row::Gap(h) => (div().h(px(*h)).into_any_element(), false),
            Row::Machine { m, collapsed } => (self.machine_head(&sections[*m], *collapsed, t, cx), false),
            Row::Note { text, faint, indent } => (
                div()
                    .pl(px(32.))
                    .pr(px(14.))
                    .pb(px(6.))
                    .text_size(px(12.))
                    .text_color(if *faint { t.text_4 } else { t.text_3 })
                    .child(text.clone())
                    .into_any_element(),
                *indent,
            ),
            Row::Account { m, s, collapsed } => {
                let m = &sections[*m];
                (self.account_head(&m.accounts[*s], *collapsed, m.home, t, cx).into_any_element(), false)
            }
            Row::Project { at, collapsed } => {
                (self.project_head(self.side.group(*at), *collapsed, t, cx).into_any_element(), at.s.is_some())
            }
            Row::Empty(at) => (self.empty_project(self.side.group(*at), t, cx), at.s.is_some()),
            Row::Agent { at, i, index } => {
                #[cfg(test)]
                {
                    self.side.agent_rows += 1;
                }
                let g = self.side.group(*at);
                let a = &g.agents[*i];
                let selected = focused.as_deref() == Some(a.id.as_str());
                let cursor = kb && self.cursor.as_deref() == Some(a.id.as_str());
                let row = self.agent_row(a, &g.key, *index, selected, cursor, g.can_create, t, cx).into_any_element();
                let wts = self.wt_agent(a, t, cx);
                let el = if wts.is_empty() {
                    row
                } else {
                    div().flex().flex_col().child(row).children(wts).into_any_element()
                };
                (el, at.s.is_some())
            }
            Row::Others(at) => {
                let g = self.side.group(*at);
                let el = div().flex().flex_col().children(self.wt_others(&g.key, &g.agents, t, cx)).into_any_element();
                (el, at.s.is_some())
            }
            Row::Tail => {
                let mut tail: Vec<AnyElement> = self.render_elsewhere(t, cx).into_iter().collect();
                tail.extend(self.render_peers(t, cx));
                let any = !sections.is_empty();
                (
                    div()
                        .flex()
                        .flex_col()
                        .when(any && !tail.is_empty(), |d| d.child(div().h(px(4.)).flex_none()))
                        .children(tail)
                        .into_any_element(),
                    false,
                )
            }
        };
        // Each row fills the list's width; a project's rows under an
        // account sit a step in, as in its account's column before.
        div().w_full().flex().flex_col().when(indent, |d| d.pl(px(10.))).child(el).into_any_element()
    }

    /// `.project-empty`: one quiet dashed row where an agent row would be;
    /// the whole row opens New agent.
    fn empty_project(&self, g: &ProjectGroup, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let project = g.project.clone();
        let group = SharedString::from(format!("project-empty-{}", g.key));
        div()
            .id(SharedString::from(format!("empty-{}", g.key)))
            .group(group.clone())
            .mx(px(6.))
            .mb(px(4.))
            .h(px(32.))
            .px(px(9.))
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded(RADIUS)
            .border_1()
            .border_dashed()
            .border_color(t.line_strong)
            .text_size(px(12.5))
            .text_color(t.text_3)
            .cursor_pointer()
            .hover_text(t.text, |s| s.border_color(t.text_4).bg(t.surface_2))
            .tooltip(tooltip("New agent in this project"))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_new_agent(Some(project.clone()), window, cx)
            }))
            .child(icon("plus", 12., t.text_3).group_hover_text(group, t.text, |s| s))
            .child("New agent")
            .child(div().ml_auto().text_size(px(11.5)).text_color(t.text_4).child("No agents yet"))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_join_neighbours() {
        assert_eq!(runs(&[]), Vec::<Range<usize>>::new());
        assert_eq!(runs(&[1, 2, 3, 7, 9, 10]), vec![1..4, 7..8, 9..11]);
    }
}
