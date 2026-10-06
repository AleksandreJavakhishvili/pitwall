//! An agent's uncommitted work as the sidebar shows it.

use super::Engine;
use crate::vcs::git::{self, FileChange};

type Res<T> = Result<T, String>;

/// Everything that differs from where the agent started; also refreshes the
/// agent's +/- counts. Blocking (git).
pub fn changes(engine: &Engine, agent_id: &str) -> Res<Vec<FileChange>> {
    let (cwd, base) = engine.with(agent_id, |a| (a.rec.cwd.clone(), a.rec.base_commit.clone()))?;
    let files = git::changes(&cwd, base.as_deref())?;
    let counts = (
        files.iter().map(|f| f.added).sum(),
        files.iter().map(|f| f.removed).sum(),
        files.len() as u32,
    );
    let differs = engine.with(agent_id, |a| {
        let d = (a.added, a.removed, a.files_changed) != counts;
        (a.added, a.removed, a.files_changed) = counts;
        d
    })?;
    if differs {
        engine.changed(false);
    }
    Ok(files)
}

/// One file's diff against where the agent started. Blocking (git).
pub fn file_diff(engine: &Engine, agent_id: &str, path: &str, untracked: bool) -> Res<String> {
    let (cwd, base) = engine.with(agent_id, |a| (a.rec.cwd.clone(), a.rec.base_commit.clone()))?;
    if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err("invalid path".into());
    }
    git::file_diff(&cwd, base.as_deref(), path, untracked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{record, Harness};
    use crate::vcs::snapshot::TempRepo;

    #[test]
    fn counts_follow_the_working_tree() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let h = Harness::new(vec![record("a", r.path())]);
        r.write("a.txt", "1\n2\n");
        r.write("new.txt", "x\n");
        let files = changes(&h.engine, "a").unwrap();
        assert_eq!(files.len(), 2);
        let v = &h.engine.views()[0];
        assert_eq!((v.added, v.removed, v.files_changed), (2, 0, 2));
        assert!(file_diff(&h.engine, "a", "a.txt", false).unwrap().contains("+2"));
        assert_eq!(file_diff(&h.engine, "a", "../x", false).unwrap_err(), "invalid path");
    }
}
