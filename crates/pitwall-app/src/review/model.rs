//! Review's pure logic, ported from the React app: which agents it lists
//! (`src/lib/reviewScope.ts`), the compact-folder file tree
//! (`src/lib/fileTree.ts`), worktrees by agent and per project
//! (`src/lib/worktrees.ts`), draft comments and the prompts they become
//! (`src/components/review/comments.ts`), the task picker's labels, the
//! commit/merge plan (`ReviewDialogs.tsx`) and "updated N s ago".

use std::collections::{BTreeMap, HashMap, HashSet};

use pitwall_core::engine::Task;
use pitwall_core::review::MergeStatus;
use pitwall_core::vcs::git::FileChange;
use pitwall_proto::{AgentView, ProjectWorktrees, WorktreeVia, WorktreeView};

// ── scope ──────────────────────────────────────────────────────────────────

/// The space Review is opened in (the shell has no spaces yet: `All`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Space {
    /// "All": every agent.
    All,
    /// A project space: that project's agents plus members and shown agents.
    Project {
        name: String,
        project: String,
        /// Members and agents shown in its panes.
        members: Vec<String>,
    },
    /// A custom space: its members and the agents shown in its panes.
    Custom { name: String, members: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub agents: Vec<AgentView>,
    /// Narrowed: the "All projects" toggle is offered.
    pub can_widen: bool,
    /// What it is narrowed to (the space's or the focused agent's project).
    pub label: String,
}

/// `reviewScope`: the active space's agents (in "All", the focused agent's
/// project; everyone with no focused agent), or everyone with "All
/// projects"; `extra` agents always (the one Review was opened for).
pub fn scope(
    agents: &[AgentView],
    space: &Space,
    all: bool,
    extra: &[String],
    focused: Option<&AgentView>,
) -> Scope {
    let in_all = matches!(space, Space::All);
    let label = match (space, focused) {
        (Space::All, None) => {
            return Scope {
                agents: agents.to_vec(),
                can_widen: false,
                label: String::new(),
            }
        }
        (Space::All, Some(f)) => f.project_display.clone(),
        (Space::Project { name, .. } | Space::Custom { name, .. }, _) => name.clone(),
    };
    if all {
        return Scope {
            agents: agents.to_vec(),
            can_widen: true,
            label,
        };
    }
    let mut ids: HashSet<&str> = agents
        .iter()
        .filter(|a| match space {
            Space::All => in_all && focused.is_some_and(|f| a.project == f.project),
            Space::Project {
                project, members, ..
            } => a.project == *project || members.contains(&a.id),
            Space::Custom { members, .. } => members.contains(&a.id),
        })
        .map(|a| a.id.as_str())
        .collect();
    ids.extend(extra.iter().map(String::as_str));
    Scope {
        agents: agents
            .iter()
            .filter(|a| ids.contains(a.id.as_str()))
            .cloned()
            .collect(),
        can_widen: true,
        label,
    }
}

// ── file tree ──────────────────────────────────────────────────────────────

/// VS Code's SCM letter: the backend's, else U (untracked) or M.
pub use crate::kit::file_status;

pub use crate::kit::status_letter_text as status_letter;

pub use crate::kit::status_title;

/// `src/a/b.rs` → (`src/a/`, `b.rs`).
pub fn split_path(p: &str) -> (&str, &str) {
    match p.rfind('/') {
        Some(i) => (&p[..=i], &p[i + 1..]),
        None => ("", p),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// `path`: the full path (the collapse key); `name`: maybe compact `a/b`.
    Dir {
        path: String,
        name: String,
        children: Vec<Node>,
    },
    File {
        path: String,
        name: String,
        file: FileChange,
    },
}

/// Natural, case-insensitive order (`localeCompare` with `numeric: true`).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

#[derive(Default)]
struct RawDir {
    dirs: BTreeMap<String, RawDir>,
    files: Vec<FileChange>,
}

/// Folders first, then files, by name; a folder whose only child is a
/// folder shares its row (`src/components/review`).
pub fn build_tree(files: &[FileChange]) -> Vec<Node> {
    let mut root = RawDir::default();
    for f in files {
        let parts: Vec<&str> = f.path.split('/').collect();
        let mut d = &mut root;
        for seg in &parts[..parts.len() - 1] {
            d = d.dirs.entry(seg.to_string()).or_default();
        }
        d.files.push(f.clone());
    }
    fn walk(d: &RawDir, prefix: &str) -> Vec<Node> {
        let mut out = Vec::new();
        let mut names: Vec<&String> = d.dirs.keys().collect();
        names.sort_by(|a, b| natural_cmp(a, b));
        for seg in names {
            let mut sub = &d.dirs[seg];
            let mut name = seg.clone();
            while sub.files.is_empty() && sub.dirs.len() == 1 {
                let (k, v) = sub.dirs.iter().next().expect("one");
                name = format!("{name}/{k}");
                sub = v;
            }
            let path = format!("{prefix}{name}");
            out.push(Node::Dir {
                children: walk(sub, &format!("{path}/")),
                path,
                name,
            });
        }
        let mut files = d.files.clone();
        files.sort_by(|a, b| natural_cmp(split_path(&a.path).1, split_path(&b.path).1));
        for f in files {
            out.push(Node::File {
                path: f.path.clone(),
                name: split_path(&f.path).1.to_string(),
                file: f,
            });
        }
        out
    }
    walk(&root, "")
}

/// Files in the order the tree shows them (all folders open).
pub fn tree_order(files: &[FileChange]) -> Vec<FileChange> {
    fn visit(ns: &[Node], out: &mut Vec<FileChange>) {
        for n in ns {
            match n {
                Node::File { file, .. } => out.push(file.clone()),
                Node::Dir { children, .. } => visit(children, out),
            }
        }
    }
    let mut out = Vec::new();
    visit(&build_tree(files), &mut out);
    out
}

/// One visible row of a tree.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Dir {
        path: String,
        name: String,
        depth: usize,
        open: bool,
    },
    File {
        path: String,
        name: String,
        depth: usize,
        file: FileChange,
    },
}

/// The rows shown, with `closed` folders folded.
pub fn visible_rows(tree: &[Node], closed: &dyn Fn(&str) -> bool) -> Vec<TreeRow> {
    fn walk(ns: &[Node], depth: usize, closed: &dyn Fn(&str) -> bool, out: &mut Vec<TreeRow>) {
        for n in ns {
            match n {
                Node::Dir {
                    path,
                    name,
                    children,
                } => {
                    let open = !closed(path);
                    out.push(TreeRow::Dir {
                        path: path.clone(),
                        name: name.clone(),
                        depth,
                        open,
                    });
                    if open {
                        walk(children, depth + 1, closed, out);
                    }
                }
                Node::File { path, name, file } => out.push(TreeRow::File {
                    path: path.clone(),
                    name: name.clone(),
                    depth,
                    file: file.clone(),
                }),
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, 0, closed, &mut out);
    out
}

pub fn diffstat(files: &[FileChange]) -> (u32, u32) {
    files
        .iter()
        .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed))
}

// ── worktrees ──────────────────────────────────────────────────────────────

/// One worktree with its project (what the per-worktree calls take).
#[derive(Debug, Clone, PartialEq)]
pub struct WtRef {
    pub project_id: String,
    pub repo_display: String,
    /// The project's current branch (the merge target).
    pub target: Option<String>,
    pub wt: WorktreeView,
}

impl WtRef {
    pub fn key(&self) -> String {
        wt_key(&self.project_id, &self.wt.path)
    }
}

pub fn wt_key(project_id: &str, path: &str) -> String {
    format!("{project_id}\u{0}{path}")
}

fn wt_ref(p: &ProjectWorktrees, wt: &WorktreeView) -> WtRef {
    WtRef {
        project_id: p.id.clone(),
        repo_display: p.repo_display.clone(),
        target: p.branch.clone(),
        wt: wt.clone(),
    }
}

/// Worktrees an agent has besides its own folder, by agent id.
pub fn worktrees_by_agent(projects: &[ProjectWorktrees]) -> HashMap<String, Vec<WtRef>> {
    let mut out: HashMap<String, Vec<WtRef>> = HashMap::new();
    for p in projects {
        for wt in &p.worktrees {
            let Some(id) = wt.agent_id.as_ref() else {
                continue;
            };
            if matches!(wt.via, WorktreeVia::Own | WorktreeVia::Other) {
                continue;
            }
            out.entry(id.clone()).or_default().push(wt_ref(p, wt));
        }
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| a.wt.name.cmp(&b.wt.name));
    }
    out
}

/// A project's worktrees of no agent, for the group holding any of `ids`.
pub fn other_worktrees(projects: &[ProjectWorktrees], ids: &[&str]) -> Vec<WtRef> {
    let mut out: Vec<WtRef> = projects
        .iter()
        .filter(|p| p.agent_ids.iter().any(|a| ids.contains(&a.as_str())))
        .flat_map(|p| {
            p.worktrees
                .iter()
                .filter(|w| w.via == WorktreeVia::Other)
                .map(move |w| wt_ref(p, w))
        })
        .collect();
    out.sort_by(|a, b| a.wt.name.cmp(&b.wt.name));
    out
}

pub fn find_worktree(projects: &[ProjectWorktrees], project_id: &str, path: &str) -> Option<WtRef> {
    let p = projects.iter().find(|p| p.id == project_id)?;
    let wt = p.worktrees.iter().find(|w| w.path == path)?;
    Some(wt_ref(p, wt))
}

// ── tasks ──────────────────────────────────────────────────────────────────

/// "Task N · HH:MM · <prompt…>" (+ " · running").
pub fn task_label(t: &Task, n: usize, clock: &dyn Fn(u64) -> String) -> String {
    let squashed = t.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let prompt = if squashed.is_empty() {
        "(typed in the terminal)".to_string()
    } else {
        squashed
    };
    let short = if prompt.chars().count() > 70 {
        format!("{}…", prompt.chars().take(69).collect::<String>())
    } else {
        prompt
    };
    format!(
        "Task {n} · {}{} · {short}",
        clock(t.started_at),
        if t.ended_at.is_some() {
            ""
        } else {
            " · running"
        }
    )
}

/// Local "HH:MM" of a millisecond timestamp.
pub fn clock(ms: u64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms as i64) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => {
            t.format("%H:%M").to_string()
        }
        chrono::LocalResult::None => String::new(),
    }
}

/// "updated 12 s ago" (`src/lib/freshness.ts`).
pub fn updated_label(at_ms: u64, now_ms: u64) -> String {
    let s = (now_ms.saturating_sub(at_ms) as f64 / 1000.0).round() as u64;
    crate::kit::updated_label(s)
}

// ── comments ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: u64,
    pub path: String,
    /// 1-based line of the modified file.
    pub line: usize,
    pub text: String,
}

/// File/line order.
pub fn sort_comments(list: &[Comment]) -> Vec<Comment> {
    let mut v = list.to_vec();
    v.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    v
}

/// The one visible template (docs/spec/review.md); text goes in verbatim.
pub fn compose_prompt(list: &[Comment]) -> String {
    let mut out = vec!["Review comments:".to_string()];
    out.extend(
        sort_comments(list)
            .iter()
            .map(|c| format!("- {}:{} — {}", c.path, c.line, c.text)),
    );
    out.join("\n")
}

/// Prefilled prompt after a merge conflict.
pub fn conflict_prompt(branch: Option<&str>) -> String {
    format!(
        "Rebase onto {} and resolve conflicts.",
        branch.unwrap_or("the main branch")
    )
}

// ── commit & merge ─────────────────────────────────────────────────────────

/// What the Commit/Merge dialog will do with a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub need_commit: bool,
    pub can_merge: bool,
    pub will_merge: bool,
    /// Nothing to commit and nothing to merge.
    pub nothing: bool,
}

pub fn plan(st: &MergeStatus, also_merge: bool) -> Plan {
    let need_commit = st.uncommitted > 0;
    let can_merge = st.worktree && st.target.is_some() && st.branch.is_some();
    let will_merge = can_merge
        && (if need_commit {
            also_merge
        } else {
            st.ahead > 0
        })
        && !st.target_dirty;
    Plan {
        need_commit,
        can_merge,
        will_merge,
        nothing: !(need_commit || can_merge && st.ahead > 0),
    }
}

impl Plan {
    /// The primary button's label.
    pub fn primary(&self) -> &'static str {
        if self.need_commit {
            if self.will_merge {
                "Commit & merge"
            } else {
                "Commit"
            }
        } else {
            "Merge"
        }
    }

    pub fn can_go(&self, message: &str) -> bool {
        !self.nothing
            && (if self.need_commit {
                !message.trim().is_empty()
            } else {
                self.will_merge
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::agent;

    fn fc(path: &str) -> FileChange {
        FileChange {
            path: path.into(),
            added: 1,
            removed: 0,
            ..Default::default()
        }
    }

    #[test]
    fn scope_follows_the_space_and_the_focused_agent() {
        let agents = vec![
            agent("a1", "/work/alpha", "idle", 0, true),
            agent("a2", "/work/alpha", "idle", 0, true),
            agent("b1", "/work/beta", "idle", 0, true),
        ];
        // All, nobody focused: everyone, no toggle.
        let s = scope(&agents, &Space::All, false, &[], None);
        assert_eq!(s.agents.len(), 3);
        assert!(!s.can_widen);
        // All, focused agent: its project, with the toggle.
        let s = scope(&agents, &Space::All, false, &[], Some(&agents[2]));
        assert_eq!(
            s.agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["b1"]
        );
        assert!(s.can_widen);
        assert_eq!(s.label, "beta");
        // Widened.
        assert_eq!(
            scope(&agents, &Space::All, true, &[], Some(&agents[2]))
                .agents
                .len(),
            3
        );
        // Extra: the agent Review was opened for.
        let s = scope(
            &agents,
            &Space::All,
            false,
            &["a1".into()],
            Some(&agents[2]),
        );
        assert_eq!(s.agents.len(), 2);
        // A project space with a member from elsewhere; a custom space.
        let p = Space::Project {
            name: "alpha".into(),
            project: "/work/alpha".into(),
            members: vec!["b1".into()],
        };
        assert_eq!(scope(&agents, &p, false, &[], None).agents.len(), 3);
        let c = Space::Custom {
            name: "Space 2".into(),
            members: vec!["a2".into()],
        };
        let s = scope(&agents, &c, false, &[], None);
        assert_eq!((s.agents.len(), s.label.as_str()), (1, "Space 2"));
    }

    #[test]
    fn tree_compacts_single_folders_and_orders_naturally() {
        let files = vec![
            fc("src/components/review/b.ts"),
            fc("src/components/review/a10.ts"),
            fc("src/components/review/a9.ts"),
            fc("README.md"),
            fc("docs/x.md"),
            fc("docs/y/z.md"),
        ];
        let t = build_tree(&files);
        let names: Vec<String> = t
            .iter()
            .map(|n| match n {
                Node::Dir { name, .. } | Node::File { name, .. } => name.clone(),
            })
            .collect();
        assert_eq!(names, ["docs", "src/components/review", "README.md"]);
        let order: Vec<String> = tree_order(&files).into_iter().map(|f| f.path).collect();
        assert_eq!(
            order,
            [
                "docs/y/z.md",
                "docs/x.md",
                "src/components/review/a9.ts",
                "src/components/review/a10.ts",
                "src/components/review/b.ts",
                "README.md"
            ]
        );
        let rows = visible_rows(&t, &|p| p == "docs");
        assert_eq!(rows.len(), 1 + 1 + 3 + 1);
        assert!(matches!(&rows[0], TreeRow::Dir { open: false, .. }));
    }

    #[test]
    fn georgian_paths_split_on_characters() {
        assert_eq!(split_path("დოკები/ფაილი.md"), ("დოკები/", "ფაილი.md"));
        assert_eq!(split_path("a.rs"), ("", "a.rs"));
    }

    #[test]
    fn comments_compose_in_file_and_line_order() {
        let c = |id, path: &str, line, text: &str| Comment {
            id,
            path: path.into(),
            line,
            text: text.into(),
        };
        let p = compose_prompt(&[
            c(1, "b.rs", 3, "rename"),
            c(2, "a.rs", 10, "გადაარქვი"),
            c(3, "a.rs", 2, "x"),
        ]);
        assert_eq!(
            p,
            "Review comments:\n- a.rs:2 — x\n- a.rs:10 — გადაარქვი\n- b.rs:3 — rename"
        );
        assert_eq!(
            conflict_prompt(None),
            "Rebase onto the main branch and resolve conflicts."
        );
    }

    #[test]
    fn task_labels_shorten_and_mark_running() {
        let t = Task {
            id: "t1".into(),
            prompt: "  fix   the\nbug ".into(),
            started_at: 0,
            ended_at: None,
            start_tree: None,
            end_tree: None,
        };
        let l = task_label(&t, 2, &|_| "09:05".into());
        assert_eq!(l, "Task 2 · 09:05 · running · fix the bug");
        let long = Task {
            prompt: "x".repeat(100),
            ended_at: Some(1),
            ..t.clone()
        };
        let l = task_label(&long, 1, &|_| "10:00".into());
        assert!(l.ends_with('…') && !l.contains("running"));
        let empty = Task {
            prompt: String::new(),
            ..t
        };
        assert!(task_label(&empty, 3, &|_| "x".into()).contains("(typed in the terminal)"));
        assert_eq!(clock(0).len(), 5);
    }

    #[test]
    fn plans_for_commit_and_merge() {
        let st = MergeStatus {
            worktree: true,
            branch: Some("feat".into()),
            target: Some("main".into()),
            target_dirty: false,
            uncommitted: 2,
            ahead: 0,
        };
        let p = plan(&st, true);
        assert!(p.need_commit && p.will_merge);
        assert_eq!(p.primary(), "Commit & merge");
        assert!(!p.can_go("  "));
        assert!(p.can_go("msg"));
        assert_eq!(plan(&st, false).primary(), "Commit");
        let dirty = MergeStatus {
            target_dirty: true,
            ..st.clone()
        };
        assert!(!plan(&dirty, true).will_merge);
        let clean = MergeStatus {
            uncommitted: 0,
            ahead: 2,
            ..st.clone()
        };
        let p = plan(&clean, true);
        assert_eq!(p.primary(), "Merge");
        assert!(p.can_go(""));
        let nothing = MergeStatus {
            uncommitted: 0,
            ahead: 0,
            ..st
        };
        assert!(plan(&nothing, true).nothing);
        let main = MergeStatus {
            worktree: false,
            branch: Some("main".into()),
            target: None,
            target_dirty: false,
            uncommitted: 1,
            ahead: 0,
        };
        assert!(!plan(&main, true).can_merge);
    }

    #[test]
    fn freshness_labels() {
        assert_eq!(updated_label(10_000, 12_000), "updated just now");
        assert_eq!(updated_label(0, 12_000), "updated 12 s ago");
        assert_eq!(updated_label(0, 125_000), "updated 2 min ago");
    }
}
