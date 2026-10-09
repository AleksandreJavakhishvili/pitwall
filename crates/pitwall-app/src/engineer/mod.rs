//! The Race Engineer preset (docs/spec/engineer.md): an optional assistant
//! agent for Pitwall itself. The top bar's headset button and "Race
//! Engineer" in ⌘K open it (or focus the one there is); what it runs on is
//! `engineer.agent` (Settings → Agents). The launch itself (its folder,
//! instruction files, flags and PATH) is `pitwall_core::engineer`; this
//! module finds the files the app ships for it and adds the palette row.
//! Nothing starts until the user opens it.

use std::path::{Path, PathBuf};

use gpui::App;
use pitwall_core::engineer::{Kit, PERSONA};
use pitwall_core::Shared;

use crate::palette::{self, rank, Command};

/// Once at start: give the engine the engineer's files and add the
/// palette row.
pub fn register(cx: &mut App, engine: Option<&Shared>) {
    if let Some(engine) = engine {
        engine.set_engineer_kit(kit(engine.paths().root()));
    }
    palette::register(cx, |pc, _| {
        let screen = pc.screen.clone();
        vec![Command::new(
            "race-engineer",
            "Race Engineer",
            "race engineer assistant preset help setup pitwall agw settings spaces rules",
            move |window, cx| {
                if let Some(s) = screen.as_ref().and_then(|s| s.upgrade()) {
                    s.update(cx, |s, cx| s.open_engineer(window, cx));
                }
            },
        )
        .icon(ICON)
        .rank(rank::NEW + 50)]
    });
}

/// The top bar's and the palette's icon.
pub const ICON: &str = "headset";

/// What the app ships for the engineer: the skills folder and the CLI.
/// `None` when the skills aren't there (a build without them).
pub fn kit(data_root: &Path) -> Option<Kit> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let skills = find_skills(std::env::var_os("PITWALL_SKILLS_DIR").map(PathBuf::from), exe_dir, DEV_SKILLS)?;
    let cli = crate::platform::cli_install::cli_bin(data_root).ok();
    Some(Kit { skills, cli })
}

/// The repo's `skills/` (workspace builds and tests).
const DEV_SKILLS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../skills");

/// The skills folder, like the other things the app ships
/// (`host::holder_bin`): `$PITWALL_SKILLS_DIR`; in a macOS bundle
/// `Contents/Resources/skills`; next to the executable (Windows, a
/// workspace build); `../lib/<package>/skills` (Linux packages); else the
/// repo's (`dev`).
fn find_skills(env: Option<PathBuf>, exe_dir: Option<PathBuf>, dev: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = env.into_iter().collect();
    if let Some(dir) = exe_dir {
        if let Some(up) = dir.parent() {
            candidates.push(up.join("Resources").join("skills"));
            for pkg in ["pitwall", "pitwall-preview"] {
                candidates.push(up.join("lib").join(pkg).join("skills"));
            }
        }
        candidates.push(dir.join("skills"));
    }
    candidates.push(PathBuf::from(dev));
    candidates.into_iter().find(|d| d.join(PERSONA).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skills_are_found_like_the_other_shipped_files() {
        let dir = std::env::temp_dir().join(format!("pw-eng-{}", std::process::id()));
        let bundle = dir.join("Pitwall.app/Contents");
        let res = bundle.join("Resources/skills");
        std::fs::create_dir_all(res.join("race-engineer")).unwrap();
        std::fs::write(res.join(PERSONA), "p").unwrap();
        let exe = Some(bundle.join("MacOS"));
        assert_eq!(find_skills(None, exe.clone(), "/nowhere"), Some(res.clone()));
        // The override wins; a folder without the persona doesn't count.
        let other = dir.join("other");
        std::fs::create_dir_all(other.join("race-engineer")).unwrap();
        assert_eq!(find_skills(Some(other.clone()), exe.clone(), "/nowhere"), Some(res.clone()));
        std::fs::write(other.join(PERSONA), "p").unwrap();
        assert_eq!(find_skills(Some(other.clone()), exe, "/nowhere"), Some(other));
        // A workspace build: the repo's skills.
        assert!(find_skills(None, None, DEV_SKILLS).is_some(), "skills/race-engineer/ENGINEER.md is in the repo");
        assert_eq!(find_skills(None, None, "/nowhere"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
