//! One agent's file tree (a port of `TreeModel` in `src/lib/explorer.ts`):
//! which folders are open and what they hold. Folders are listed when first
//! opened; `refresh` lists the agent's folder and every open folder again
//! (coalesced while one runs). Shared by the Files tab and the viewer, so
//! both show the same folders open.

use std::sync::Arc;

use futures::future::join_all;
use gpui::{Context, Task};

use pitwall_core::explorer::DirListing;

use super::logic::{parent_dirs, TreeState};
use super::source::{Res, Source};

pub struct TreeModel {
    source: Arc<dyn Source>,
    agent_id: String,
    ignored: bool,
    state: TreeState,
    /// Bumped when the way of listing changes: older answers are dropped.
    generation: u64,
    refreshing: Option<Task<()>>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl TreeModel {
    pub fn new(source: Arc<dyn Source>, agent_id: String, ignored: bool) -> TreeModel {
        TreeModel {
            source,
            agent_id,
            ignored,
            state: TreeState::default(),
            generation: 0,
            refreshing: None,
        }
    }

    pub fn state(&self) -> &TreeState {
        &self.state
    }

    pub fn ignored(&self) -> bool {
        self.ignored
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// "Show ignored files" toggled: forget what was read and read again.
    pub fn set_ignored(&mut self, ignored: bool, cx: &mut Context<Self>) {
        if self.ignored == ignored {
            return;
        }
        self.ignored = ignored;
        self.generation += 1;
        self.refreshing = None;
        self.state.listings.clear();
        self.state.errors.clear();
        self.state.loading.clear();
        self.state.refreshing = false;
        cx.notify();
        self.refresh(false, cx);
    }

    pub fn is_open(&self, dir: &str) -> bool {
        self.state.is_open(dir)
    }

    pub fn toggle(&mut self, dir: &str, cx: &mut Context<Self>) {
        if self.is_open(dir) {
            self.collapse(dir, cx)
        } else {
            self.expand(dir, cx)
        }
    }

    pub fn collapse(&mut self, dir: &str, cx: &mut Context<Self>) {
        if self.is_open(dir) {
            self.state.expanded.retain(|d| d != dir);
            cx.notify();
        }
    }

    /// Open a folder; listed now unless it already is.
    pub fn expand(&mut self, dir: &str, cx: &mut Context<Self>) {
        if !self.is_open(dir) {
            self.state.expanded.push(dir.to_string());
            cx.notify();
        }
        if self.state.listings.contains_key(dir) || self.state.loading.iter().any(|d| d == dir) {
            return;
        }
        self.list_one(dir.to_string(), true, cx).detach();
    }

    /// Open every folder above `path` (to show it).
    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        for d in parent_dirs(path) {
            self.expand(&d, cx);
        }
    }

    fn list_one(&mut self, dir: String, first: bool, cx: &mut Context<Self>) -> Task<()> {
        let generation = self.generation;
        if first {
            self.state.loading.push(dir.clone());
            cx.notify();
        }
        let (source, agent, ignored, d) = (
            self.source.clone(),
            self.agent_id.clone(),
            self.ignored,
            dir.clone(),
        );
        let load = cx
            .background_executor()
            .spawn(async move { source.list_files(&agent, &d, ignored) });
        cx.spawn(async move |this, cx| {
            let res = load.await;
            let _ = this.update(cx, |m, cx| m.listed(generation, dir, res, cx));
        })
    }

    fn listed(
        &mut self,
        generation: u64,
        dir: String,
        res: Res<DirListing>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        self.state.loading.retain(|d| *d != dir);
        match res {
            Ok(l) => {
                self.state.errors.remove(&dir);
                if self.state.listings.get(&dir) != Some(&l) {
                    self.state.listings.insert(dir, l);
                }
            }
            Err(e) => {
                self.state.errors.insert(dir, e);
            }
        }
        cx.notify();
    }

    pub fn is_refreshing(&self) -> bool {
        self.refreshing.is_some()
    }

    /// The agent's folder and every open folder, read again (`quiet`: no
    /// "refreshing…"). A refresh already running absorbs this one.
    pub fn refresh(&mut self, quiet: bool, cx: &mut Context<Self>) {
        if self.refreshing.is_some() {
            return;
        }
        let generation = self.generation;
        if !quiet {
            self.state.refreshing = true;
            cx.notify();
        }
        let first = !self.state.listings.contains_key("");
        let mut dirs = vec![String::new()];
        dirs.extend(
            self.state
                .expanded
                .iter()
                .filter(|d| !d.is_empty())
                .cloned(),
        );
        let tasks: Vec<Task<()>> = dirs
            .into_iter()
            .map(|d| {
                let first = first && d.is_empty();
                self.list_one(d, first, cx)
            })
            .collect();
        self.refreshing = Some(cx.spawn(async move |this, cx| {
            join_all(tasks).await;
            let _ = this.update(cx, |m, cx| {
                if generation != m.generation {
                    return;
                }
                m.refreshing = None;
                // Open folders that no longer exist close.
                let gone: Vec<String> = m
                    .state
                    .expanded
                    .iter()
                    .filter(|d| {
                        m.state
                            .errors
                            .get(*d)
                            .is_some_and(|e| e.to_lowercase().contains("not found"))
                    })
                    .cloned()
                    .collect();
                for d in &gone {
                    m.state.errors.remove(d);
                }
                m.state.expanded.retain(|d| !gone.contains(d));
                m.state.updated_at = Some(now_ms());
                m.state.refreshing = false;
                cx.notify();
            });
        }));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use gpui::{AppContext, TestAppContext};

    use super::super::logic::TreeRow;
    use super::super::source::fake::FakeSource;
    use super::*;

    fn names(m: &TreeModel) -> Vec<String> {
        m.state()
            .rows()
            .iter()
            .map(|r| match r {
                TreeRow::Entry { entry, .. } => entry.path.clone(),
                TreeRow::Loading { dir, .. } => format!("loading {dir}"),
                TreeRow::Error { error, .. } => format!("error {error}"),
                TreeRow::Truncated { .. } => "truncated".into(),
            })
            .collect()
    }

    #[gpui::test]
    async fn lists_the_folder_then_folders_as_they_open(cx: &mut TestAppContext) {
        let src = FakeSource::new(&[
            ("src/a.rs", "a"),
            ("src/ქართული.md", "b"),
            ("README.md", "r"),
        ]);
        let tree = cx.new(|_| TreeModel::new(src.clone(), "a1".into(), false));
        tree.update(cx, |m, cx| m.refresh(false, cx));
        tree.read_with(cx, |m, _| {
            assert!(m.state().refreshing);
            assert_eq!(names(m), ["loading "]);
        });
        cx.run_until_parked();
        tree.read_with(cx, |m, _| {
            assert_eq!(names(m), ["src", "README.md"]);
            assert!(m.state().updated_at.is_some());
            assert!(!m.state().refreshing);
        });
        tree.update(cx, |m, cx| m.expand("src", cx));
        tree.read_with(cx, |m, _| {
            assert_eq!(names(m), ["src", "loading src", "README.md"])
        });
        cx.run_until_parked();
        tree.read_with(cx, |m, _| {
            assert_eq!(names(m), ["src", "src/a.rs", "src/ქართული.md", "README.md"])
        });
        tree.update(cx, |m, cx| m.toggle("src", cx));
        tree.read_with(cx, |m, _| assert_eq!(names(m), ["src", "README.md"]));
    }

    #[gpui::test]
    async fn refresh_closes_folders_that_vanished(cx: &mut TestAppContext) {
        let src = FakeSource::new(&[("old/x.rs", "x"), ("keep.rs", "k")]);
        let tree = cx.new(|_| TreeModel::new(src.clone(), "a1".into(), false));
        tree.update(cx, |m, cx| {
            m.refresh(false, cx);
            m.reveal("old/x.rs", cx);
        });
        cx.run_until_parked();
        tree.read_with(cx, |m, _| {
            assert_eq!(names(m), ["old", "old/x.rs", "keep.rs"])
        });
        src.files.lock().unwrap().remove("old/x.rs");
        tree.update(cx, |m, cx| m.refresh(true, cx));
        cx.run_until_parked();
        tree.read_with(cx, |m, _| {
            assert_eq!(names(m), ["keep.rs"]);
            assert!(m.state().expanded.is_empty());
        });
    }

    #[gpui::test]
    async fn show_ignored_reads_everything_again(cx: &mut TestAppContext) {
        let src = FakeSource::new(&[("dist/out.js", "o"), ("main.rs", "m")]);
        src.ignored.lock().unwrap().push("dist".into());
        let tree = cx.new(|_| TreeModel::new(src.clone(), "a1".into(), false));
        tree.update(cx, |m, cx| m.refresh(false, cx));
        cx.run_until_parked();
        tree.read_with(cx, |m, _| assert_eq!(names(m), ["main.rs"]));
        let before = src.lists.load(Ordering::SeqCst);
        tree.update(cx, |m, cx| m.set_ignored(true, cx));
        cx.run_until_parked();
        tree.read_with(cx, |m, _| {
            assert_eq!(names(m), ["dist", "main.rs"]);
            assert!(m.state().find_entry("dist").unwrap().ignored);
        });
        assert!(src.lists.load(Ordering::SeqCst) > before);
    }

    #[gpui::test]
    async fn keeps_an_error_next_to_the_folder(cx: &mut TestAppContext) {
        let src = FakeSource::new(&[("main.rs", "m")]);
        let tree = cx.new(|_| TreeModel::new(src.clone(), "a1".into(), false));
        tree.update(cx, |m, cx| m.refresh(false, cx));
        cx.run_until_parked();
        tree.update(cx, |m, cx| m.expand("missing", cx));
        cx.run_until_parked();
        tree.read_with(cx, |m, _| {
            assert_eq!(
                m.state().errors.get("missing").map(String::as_str),
                Some("missing: not found")
            );
        });
    }
}
