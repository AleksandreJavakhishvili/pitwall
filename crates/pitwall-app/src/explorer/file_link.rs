//! ⌘-click on a file reference in an agent's terminal (`src/a.rs:42:7`):
//! the viewer opens on that file at that line, as ⌘P and "Show diff" do.
//! The terminal finds the reference and checks the file exists
//! (`pitwall_term_view::paths`); here it becomes a path in the agent's
//! folder, or a notice when it is outside it (the explorer reads only
//! there).

use std::path::{Path, PathBuf};

use gpui::{Action, Context, Window};

use super::Explorer;

/// Open `path` (absolute, as the terminal resolved it) in `agent_id`'s
/// viewer. Dispatched from the terminal to the window's root.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = explorer, no_json)]
pub struct OpenFileLink {
    pub agent_id: String,
    pub path: PathBuf,
    /// 1-based.
    pub line: Option<u32>,
    /// 1-based.
    pub column: Option<u32>,
}

/// The notice for a file outside the agent's folder.
pub const OUTSIDE: &str = "Outside this agent's folder";

/// `file` relative to `root` (`a/b.rs`), when it is inside it. Both are
/// resolved already (links followed).
pub fn relative_to(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The viewer's target for a line and column: the line, and the column's
/// character selected.
pub fn target(line: Option<u32>, column: Option<u32>) -> Option<(u32, Option<(usize, usize)>)> {
    let line = line.filter(|l| *l > 0)?;
    let cols = column
        .filter(|c| *c > 0)
        .map(|c| (c as usize - 1, c as usize));
    Some((line, cols))
}

impl Explorer {
    /// [`OpenFileLink`]: the agent's viewer on that file, or a notice.
    pub fn open_file_link(
        &mut self,
        link: &OpenFileLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = link.agent_id.clone();
        let Some(agent) = self.store.read(cx).agent(&id).cloned() else {
            return;
        };
        if !self.can_read(&id, cx) {
            self.notify(
                format!("{}'s files can't be read from here", agent.name),
                cx,
            );
            return;
        }
        let file = link.path.clone();
        let at = target(link.line, link.column);
        let root = PathBuf::from(&agent.cwd);
        let rel = cx
            .background_executor()
            .spawn(async move { relative_to(&std::fs::canonicalize(&root).ok()?, &file) });
        cx.spawn_in(window, async move |this, cx| {
            let rel = rel.await;
            let _ = this.update_in(cx, |this, window, cx| match rel {
                Some(rel) => this.open_viewer(&id, Some(rel), at, None, window, cx),
                None => this.notify(OUTSIDE, cx),
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_the_folder_only() {
        let root = Path::new("/work/shop");
        assert_eq!(
            relative_to(root, Path::new("/work/shop/src/cart/cart.ts")).as_deref(),
            Some("src/cart/cart.ts")
        );
        assert_eq!(
            relative_to(root, Path::new("/work/shop/დოკები/ჩემი ფაილი.md")).as_deref(),
            Some("დოკები/ჩემი ფაილი.md")
        );
        assert_eq!(relative_to(root, Path::new("/work/shopping/a.rs")), None);
        assert_eq!(relative_to(root, Path::new("/etc/hosts")), None);
        assert_eq!(relative_to(root, root), None);
    }

    #[gpui::test]
    async fn opens_files_in_the_agents_folder_only(cx: &mut gpui::TestAppContext) {
        use std::sync::Arc;

        use gpui::AppContext;

        use crate::agents::AgentStore;
        use crate::explorer::source::{fake::FakeSource, Source};

        let dir = std::env::temp_dir().join(format!("pw-ex-link-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("shop/src")).unwrap();
        std::fs::write(dir.join("shop/src/cart.ts"), "x\n").unwrap();
        std::fs::write(dir.join("elsewhere.md"), "y\n").unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        let mut a = crate::agents::tests::agent(
            "a1",
            &root.join("shop").to_string_lossy(),
            "idle",
            0,
            true,
        );
        a.caps.explorer = true;
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
        let store = cx.new(|_| AgentStore::new(vec![a]));
        let source: Arc<dyn Source> = FakeSource::new(&[("src/cart.ts", "x\n")]);
        let (explorer, cx) =
            cx.add_window_view(move |_, cx| Explorer::with_source(Some(source), store, cx));
        let link = |path: PathBuf| OpenFileLink {
            agent_id: "a1".into(),
            path,
            line: Some(2),
            column: Some(1),
        };

        let inside = link(root.join("shop/src/cart.ts"));
        cx.update(|window, cx| explorer.update(cx, |e, cx| e.open_file_link(&inside, window, cx)));
        cx.run_until_parked();
        let viewer = explorer.read_with(cx, |e, _| {
            assert!(e.viewer_open());
            e.viewers["a1"].clone()
        });
        viewer.read_with(cx, |v, _| assert_eq!(v.active(), Some("src/cart.ts")));

        explorer.update(cx, |e, cx| e.close_viewer(cx));
        let outside = link(root.join("elsewhere.md"));
        cx.update(|window, cx| explorer.update(cx, |e, cx| e.open_file_link(&outside, window, cx)));
        cx.run_until_parked();
        explorer.read_with(cx, |e, _| {
            assert!(!e.viewer_open());
            assert_eq!(e.notice(), Some(OUTSIDE));
        });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn line_and_column_become_the_target() {
        assert_eq!(target(Some(42), Some(7)), Some((42, Some((6, 7)))));
        assert_eq!(target(Some(3), None), Some((3, None)));
        assert_eq!(target(None, Some(2)), None);
        assert_eq!(target(Some(0), None), None);
    }
}
