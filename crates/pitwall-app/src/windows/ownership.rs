//! Which window owns which space: `ui.windowOf` (space → window label,
//! missing means main) and the per-window Wall flag (`ui.wall`), as the
//! React `workspace.ts` keeps them, plus how `windows.json` follows them.

use crate::main_screen::workspace::{UiState, ALL_SPACE};

use super::file::{label_number, WindowEntry, MAIN};

/// `space` now lives in `label` (`moveSpaceToWindow`). "All" stays in main.
pub fn move_space(mut ui: UiState, space: &str, label: &str) -> UiState {
    if space == ALL_SPACE || ui.space(space).is_none() {
        return ui;
    }
    if label == MAIN {
        ui.window_of.remove(space);
    } else {
        ui.window_of.insert(space.to_string(), label.to_string());
    }
    ui
}

/// A window went away: its spaces come home to main and it leaves Wall
/// mode (`reclaimWindow`).
pub fn reclaim(mut ui: UiState, label: &str) -> UiState {
    if label == MAIN {
        return ui;
    }
    ui.window_of.retain(|_, w| w != label);
    ui.wall.retain(|w| w != label);
    ui
}

/// Window labels `ui.json` mentions that no open window has.
pub fn stale_labels(ui: &UiState, open: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in ui.window_of.values().chain(ui.wall.iter()) {
        if l != MAIN && !open.contains(&l.as_str()) && !out.contains(l) {
            out.push(l.clone());
        }
    }
    out
}

/// A saved secondary window to reopen, and the space it claims (the one
/// it was opened for, unless another saved window owns it now).
#[derive(Debug, Clone, PartialEq)]
pub struct Reopen {
    pub entry: WindowEntry,
    pub claim: Option<String>,
}

/// The saved secondary windows worth reopening: those that still own a
/// space in `ui.json`, or whose space still exists (a file written before
/// `ui.json` caught up). A window with nothing to show stays closed.
pub fn reopen_plan(entries: &[WindowEntry], ui: &UiState) -> Vec<Reopen> {
    let saved: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    let mut out = vec![];
    for e in entries {
        if label_number(&e.label).is_none() {
            continue;
        }
        let owns = ui.window_of.values().any(|w| *w == e.label);
        let claim = e
            .space_id
            .as_deref()
            .filter(|s| *s != ALL_SPACE && ui.space(s).is_some())
            .filter(|s| {
                let owner = ui.window_of_space(s);
                owner == MAIN || owner == e.label || !saved.contains(&owner)
            })
            .map(str::to_string);
        if owns || claim.is_some() {
            out.push(Reopen {
                entry: e.clone(),
                claim,
            });
        }
    }
    out
}

/// Keep each saved window's `spaceId` on a space it still owns (so a
/// restart doesn't pull a space that moved on back into it). True when an
/// entry changed.
pub fn follow_spaces(entries: &mut [WindowEntry], ui: &UiState) -> bool {
    let mut changed = false;
    for e in entries.iter_mut().filter(|e| e.label != MAIN) {
        let owned: Vec<&str> = ui
            .spaces_of(&e.label)
            .into_iter()
            .map(|s| s.id.as_str())
            .collect();
        let keeps = e.space_id.as_deref().is_some_and(|s| owned.contains(&s));
        if !keeps {
            if let Some(first) = owned.first() {
                e.space_id = Some(first.to_string());
                changed = true;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui_with(spaces: &[&str]) -> UiState {
        let mut ui = UiState::default();
        for _ in spaces {
            ui = ui.create_custom_space(MAIN).0;
        }
        // create_custom_space makes its own ids; rename them for the test.
        let ids: Vec<String> = ui.spaces.iter().skip(1).map(|s| s.id.clone()).collect();
        for (id, want) in ids.iter().zip(spaces) {
            if let Some(sp) = ui.spaces.iter_mut().find(|s| s.id == *id) {
                sp.id = want.to_string();
            }
            ui.window_of.remove(id);
        }
        ui
    }

    fn entry(label: &str, space: Option<&str>) -> WindowEntry {
        WindowEntry {
            label: label.into(),
            space_id: space.map(str::to_string),
            bounds: None,
        }
    }

    #[test]
    fn spaces_move_out_and_back() {
        let ui = ui_with(&["a", "b"]);
        let ui = move_space(ui, "a", "pitwall-1");
        assert_eq!(ui.window_of_space("a"), "pitwall-1");
        assert_eq!(ui.spaces_of(MAIN).len(), 2, "All and b stay");
        let ui = move_space(ui, "a", "pitwall-2");
        assert_eq!(ui.window_of_space("a"), "pitwall-2");
        let ui = move_space(ui, "a", MAIN);
        assert!(ui.window_of.is_empty(), "main is the default, not a key");
    }

    #[test]
    fn all_and_unknown_spaces_never_move() {
        let ui = ui_with(&["a"]);
        assert_eq!(move_space(ui.clone(), ALL_SPACE, "pitwall-1"), ui);
        assert_eq!(move_space(ui.clone(), "gone", "pitwall-1"), ui);
    }

    #[test]
    fn a_closed_window_gives_its_spaces_and_wall_back() {
        let mut ui = move_space(ui_with(&["a", "b"]), "a", "pitwall-1");
        ui = move_space(ui, "b", "pitwall-2");
        ui.wall = vec![MAIN.into(), "pitwall-1".into()];
        let ui = reclaim(ui, "pitwall-1");
        assert_eq!(ui.window_of_space("a"), MAIN);
        assert_eq!(ui.window_of_space("b"), "pitwall-2");
        assert_eq!(ui.wall, vec![MAIN.to_string()]);
        assert_eq!(reclaim(ui.clone(), MAIN), ui);
    }

    #[test]
    fn labels_of_windows_that_are_gone_are_found() {
        let mut ui = move_space(ui_with(&["a", "b"]), "a", "pitwall-1");
        ui = move_space(ui, "b", "pitwall-2");
        ui.wall = vec!["pitwall-7".into()];
        assert_eq!(
            stale_labels(&ui, &[MAIN, "pitwall-2"]),
            vec!["pitwall-1".to_string(), "pitwall-7".to_string()]
        );
    }

    #[test]
    fn saved_windows_reopen_only_with_something_to_show() {
        let ui = move_space(ui_with(&["a", "b", "c"]), "a", "pitwall-1");
        let ui = move_space(ui, "c", "pitwall-3");
        let entries = vec![
            entry(MAIN, None),
            // Owns a in ui.json.
            entry("pitwall-1", Some("a")),
            // Its space was closed since.
            entry("pitwall-2", Some("gone")),
            // c moved on to pitwall-3 (saved too): no claim, owns nothing.
            entry("pitwall-4", Some("c")),
            // b is in main: claimed (ui.json older than windows.json).
            entry("pitwall-5", Some("b")),
            entry("pitwall-3", Some("c")),
        ];
        let plan = reopen_plan(&entries, &ui);
        let labels: Vec<_> = plan.iter().map(|r| r.entry.label.as_str()).collect();
        assert_eq!(labels, ["pitwall-1", "pitwall-5", "pitwall-3"]);
        assert_eq!(plan[1].claim.as_deref(), Some("b"));
    }

    #[test]
    fn saved_spaces_follow_the_windows() {
        let ui = move_space(ui_with(&["a", "b"]), "b", "pitwall-1");
        let mut entries = vec![entry(MAIN, None), entry("pitwall-1", Some("a"))];
        assert!(follow_spaces(&mut entries, &ui));
        assert_eq!(entries[1].space_id.as_deref(), Some("b"));
        assert!(!follow_spaces(&mut entries, &ui));
        assert_eq!(entries[0].space_id, None);
    }
}
