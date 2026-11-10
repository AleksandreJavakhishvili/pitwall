//! Review's list (agents → files, worktrees → files) as a virtual list
//! (docs/spec/perf.md "Review").
//!
//! Drawn as one tall scroll container, every row of it was built and laid
//! out again on each scroll step, off-screen ones too: a worktree with
//! hundreds of changed files made each step slow. Instead the list is flat
//! rows ([`Row`]: an agent's or worktree's heading, a note under it, one
//! folder or file of its tree, gaps, hairlines…) drawn by `gpui::list` in a
//! view of its own ([`ListBody`]): a scroll step notifies that view and
//! builds and lays out only the rows on screen.
//!
//! The rows are made again when Review is drawn (not on a scroll step),
//! then compared with the last ones: rows that came or went are spliced
//! into the list's state, rows whose height may have changed are measured
//! again ([`Entry::sig`]); the rest keep their heights.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use gpui::{div, prelude::*, App, Context, Entity, ListAlignment, ListState, WeakEntity, Window};
use pitwall_core::vcs::git::FileChange;
use pitwall_proto::AgentView;

use super::comments::Comments;
use super::model::{self, TreeRow, WtRef};
use super::render::Tally;
use super::ReviewView;
use crate::kit;
use crate::theme::theme;

/// One row of Review's list. `rail`: under an agent, beside the line that
/// joins its worktrees.
#[derive(Clone)]
pub(super) enum Row {
    /// Room between rows (px).
    Gap { h: f32, rail: bool },
    /// A hairline: between agents, or (`strong`) between projects.
    Rule { strong: bool },
    /// A quiet line: nothing to list, or why some agents aren't.
    Hint(String),
    /// An agent's heading.
    Agent(Box<AgentView>),
    /// Under it: "this task only", an error, "Reading git…", "No changes."
    AgentNotes(Box<AgentView>),
    /// An agent's comments.
    Comments(Box<AgentView>),
    /// One folder or file of `owner`'s tree (an agent's id, a worktree's
    /// key); `n`: comments on the file.
    Tree { owner: String, row: TreeRow, n: usize, rail: bool },
    /// A worktree's heading.
    Worktree { r: Box<WtRef>, rail: bool },
    /// Under it: its folder is gone, an error, "Reading git…", no changes.
    WorktreeNote { r: Box<WtRef>, rail: bool },
    /// "Other worktrees · <project>".
    Others(String),
    /// "Show N worktrees without changes" and the like.
    Folds,
}

/// A row, what it is (`id`, the same while it is shown) and what its
/// height depends on (`sig`).
#[derive(Clone)]
pub(super) struct Entry {
    pub(super) row: Row,
    id: u64,
    sig: u64,
}

/// The list's rows and state.
pub(super) struct RvList {
    pub(super) state: ListState,
    pub(super) rows: Rc<Vec<Entry>>,
    pub(super) body: Entity<ListBody>,
    /// Worktrees folded away, counted when the rows were made.
    pub(super) tally: Tally,
    /// Tree rows built (tests).
    #[cfg(test)]
    pub(super) tree_rows: usize,
}

/// The list in a view of its own: its scroll steps (wheel, scrollbar)
/// notify this view, not [`ReviewView`], so a step draws neither the diff
/// nor the rows off screen.
pub struct ListBody {
    view: WeakEntity<ReviewView>,
    state: ListState,
}

impl Render for ListBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let view = self.view.clone();
        let list = gpui::list(self.state.clone(), move |ix, window, cx| {
            view.update(cx, |v, cx| v.list_row(ix, window, cx)).unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
        kit::vlist_fill("rv-list-bar", &self.state, &t, list).size_full()
    }
}

impl RvList {
    pub(super) fn new(cx: &mut Context<ReviewView>) -> RvList {
        // Rows a little past the edges are measured, not drawn; every row
        // is measured once, so the scrollbar is right from the start.
        let state = ListState::new(0, ListAlignment::Top, gpui::px(120.)).measure_all();
        let view = cx.weak_entity();
        let body = cx.new(|_| ListBody { view, state: state.clone() });
        RvList {
            state,
            rows: Rc::default(),
            body,
            tally: Tally::default(),
            #[cfg(test)]
            tree_rows: 0,
        }
    }

    /// Take the new rows: splice the ones that came or went, measure again
    /// the ones whose height may have changed.
    fn take(&mut self, rows: Vec<Entry>) {
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
        if !gone.is_empty() || came > 0 {
            self.state.splice(gone, came);
            // Spliced rows are measured with the others' (`measure_all`).
            self.state.remeasure_items(pre..pre + came);
        }
        for r in runs(&changed) {
            self.state.remeasure_items(r);
        }
        self.rows = Rc::new(rows);
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
    fn gap(&mut self, h: f32, rail: bool) {
        let above = self.0.last().map(|e| e.id);
        self.push(Row::Gap { h, rail }, ("gap", above), (h.to_bits(), rail));
    }

    /// A hairline: known by the row above it.
    fn rule(&mut self, strong: bool) {
        let above = self.0.last().map(|e| e.id);
        self.push(Row::Rule { strong }, ("rule", above), ());
    }
}

impl ReviewView {
    /// Make the list's rows again and tell the list what changed. A scroll
    /// step changes nothing here (only the rows on screen are built, by
    /// [`Self::list_row`]).
    pub(super) fn list_sync(&mut self, cx: &mut Context<Self>) {
        let (rows, tally) = self.list_rows(cx);
        self.rv.tally = tally;
        self.rv.take(rows);
    }

    /// The list's rows, in the order (and with the room between them) the
    /// list always had, and the worktrees folded away.
    fn list_rows(&self, cx: &App) -> (Vec<Entry>, Tally) {
        let mut rows = Rows::default();
        // The list's own padding (`pt 4`).
        rows.gap(4., false);
        if self.ordered.is_empty() {
            let text = if self.hidden > 0 { "No agents with changes to review." } else { "No agents yet." };
            rows.push(Row::Hint(text.into()), ("hint", text), ());
        }
        let by_agent = self.projects.by_agent();
        let mut tally = Tally::default();
        for (gi, g) in self.groups.iter().enumerate() {
            if gi > 0 {
                rows.rule(true);
            }
            for (ai, a) in g.agents.iter().enumerate() {
                let wts = self.sift(by_agent.get(&a.id).cloned().unwrap_or_default(), &mut tally, cx);
                self.agent_rows(&mut rows, a, ai > 0, &wts, cx);
            }
            let ids: Vec<&str> = g.agents.iter().map(|a| a.id.as_str()).collect();
            // Narrowed to one agent: not the project's other worktrees.
            let others = if self.agent_only {
                Vec::new()
            } else {
                self.sift(model::other_worktrees(&self.projects, &ids), &mut tally, cx)
            };
            if !others.is_empty() {
                rows.rule(false);
                rows.gap(4., false);
                rows.push(Row::Others(g.display.clone()), ("others", &g.display), ());
                for r in &others {
                    self.worktree_rows(&mut rows, r, false);
                }
            }
        }
        if tally.quiet > 0 || tally.old > 0 || tally.gone > 0 {
            rows.push(Row::Folds, "folds", (tally.quiet > 0, tally.old > 0, tally.gone > 0));
        }
        if self.hidden > 0 && !self.ordered.is_empty() {
            let who = if self.hidden == 1 { "1 agent works".to_string() } else { format!("{} agents work", self.hidden) };
            let text = format!("{who} outside a git repository — no changes to review.");
            rows.push(Row::Hint(text.clone()), ("hint", &text), ());
        }
        // `pb 16`.
        rows.gap(16., false);
        (rows.0, tally)
    }

    /// An agent's heading and, open, what is under it: notes, its files,
    /// its comments, its worktrees.
    fn agent_rows(&self, rows: &mut Rows, a: &AgentView, border: bool, wts: &[WtRef], cx: &App) {
        if border {
            rows.rule(false);
        }
        rows.gap(2., false);
        rows.push(Row::Agent(Box::new(a.clone())), ("agent", &a.id), ());
        if !self.collapsed.contains(&a.id) {
            let list = self.files.get(&a.id);
            let err = self.errors.get(&a.id);
            let scoped = self.task_scope.get(&a.id).cloned().flatten().is_some();
            if scoped || err.is_some() || list.is_none_or(|l| l.is_empty()) {
                let sig = (scoped, err, list.map(|l| l.is_empty()));
                rows.push(Row::AgentNotes(Box::new(a.clone())), ("agent-notes", &a.id), sig);
            }
            let comments = Comments::for_agent(cx, &a.id);
            if let Some(l) = list.filter(|l| !l.is_empty()) {
                let counts = comments.iter().fold(HashMap::new(), |mut m, c| {
                    *m.entry(c.path.clone()).or_insert(0usize) += 1;
                    m
                });
                self.tree_rows(rows, &a.id, l, &counts, false);
            }
            if !comments.is_empty() {
                rows.push(Row::Comments(Box::new(a.clone())), ("comments", &a.id), comments.len());
            }
            for r in wts {
                self.worktree_rows(rows, r, true);
            }
        }
        rows.gap(6., false);
    }

    /// A worktree's heading and, open, a note or its files.
    fn worktree_rows(&self, rows: &mut Rows, r: &WtRef, rail: bool) {
        let key = r.key();
        rows.gap(2., rail);
        rows.push(Row::Worktree { r: Box::new(r.clone()), rail }, ("wt", &key), ());
        if self.wt_open.contains(&key) {
            let files = self.wt_files.get(&key);
            let listed = match files {
                Some(Ok(l)) if !l.is_empty() => Some(l),
                _ => None,
            };
            if !r.wt.caps.diff || listed.is_none() {
                let sig = (r.wt.caps.diff, files.map(|f| f.as_ref().map(|l| l.is_empty())));
                rows.push(Row::WorktreeNote { r: Box::new(r.clone()), rail }, ("wt-note", &key), sig);
            }
            if let Some(l) = listed {
                self.tree_rows(rows, &key, l, &HashMap::new(), rail);
            }
        }
        rows.gap(6., rail);
    }

    /// The folders and files of `owner`'s tree that show (folded folders'
    /// don't), one row each.
    fn tree_rows(&self, rows: &mut Rows, owner: &str, files: &[FileChange], counts: &HashMap<String, usize>, rail: bool) {
        let tree = model::build_tree(files);
        let closed = |d: &str| self.closed_dirs.contains(&format!("{owner}\u{1}{d}"));
        for row in model::visible_rows(&tree, &closed) {
            let (path, dir) = match &row {
                TreeRow::Dir { path, .. } => (path.clone(), true),
                TreeRow::File { path, .. } => (path.clone(), false),
            };
            let n = if dir { 0 } else { counts.get(&path).copied().unwrap_or(0) };
            rows.push(Row::Tree { owner: owner.to_string(), row, n, rail }, ("tree", owner, &path, dir), ());
        }
    }
}
