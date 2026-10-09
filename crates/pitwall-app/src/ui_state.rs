//! The one copy of `ui.json` (docs/spec/gpui/README.md "State model"): the
//! typed [`UiState`] in an entity every window and screen reads, written
//! 150 ms after the last change, atomically, as the Tauri app does. Settings
//! (theme, look, motion, density, terminal font, Elsewhere), the spaces and
//! their layouts, the Wall, collapsed groups and the explorer's "Ignored"
//! all live here; changing the appearance fields re-applies the appearance.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, Global, Task};

use crate::main_screen::workspace;
pub use crate::main_screen::workspace::UiState;

/// The state and its pending write.
pub struct UiStore {
    pub state: UiState,
    root: Option<PathBuf>,
    save: Option<Task<()>>,
    /// The text this app wrote to `ui.json` last (the settings watcher
    /// ignores the app's own writes).
    pub written: Arc<Mutex<Option<String>>>,
}

impl UiStore {
    /// The data folder `ui.json` is in (`None`: never written).
    pub fn root(&self) -> Option<&PathBuf> {
        self.root.as_ref()
    }

    /// Replace the state; save it and re-apply the appearance if it changed.
    pub fn set(&mut self, next: UiState, cx: &mut Context<Self>) {
        if next == self.state {
            return;
        }
        let looks = next.appearance_inputs() != self.state.appearance_inputs();
        self.state = next;
        if looks {
            crate::theme::apply(self.state.appearance_inputs(), cx);
        }
        cx.notify();
        let Some(root) = self.root.clone() else {
            return;
        };
        let state = self.state.clone();
        let written = self.written.clone();
        self.save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let write = cx.background_executor().spawn(async move {
                if let Ok(text) = serde_json::to_string_pretty(&state) {
                    *written.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
                }
                workspace::save(&root, &state)
            });
            if let Err(e) = write.await {
                eprintln!("pitwall: could not save ui.json: {e}");
            }
        }));
    }
}

/// The app's [`UiStore`].
#[derive(Clone)]
pub struct UiHandle(pub Entity<UiStore>);

impl Global for UiHandle {}

/// Load `ui.json` from the data folder (`None`: defaults, never written)
/// and apply its appearance. Call once, before windows open.
pub fn init(root: Option<PathBuf>, cx: &mut App) -> Entity<UiStore> {
    let state = root.as_deref().map(workspace::load).unwrap_or_default();
    crate::theme::apply(state.appearance_inputs(), cx);
    let store = cx.new(|_| UiStore {
        state,
        root,
        save: None,
        written: Arc::default(),
    });
    cx.set_global(UiHandle(store.clone()));
    store
}

/// The store (made with defaults, unsaved, if `init` didn't run: tests,
/// demos).
pub fn store(cx: &mut App) -> Entity<UiStore> {
    if let Some(h) = cx.try_global::<UiHandle>() {
        return h.0.clone();
    }
    let store = cx.new(|_| UiStore {
        state: UiState::default(),
        root: None,
        save: None,
        written: Arc::default(),
    });
    cx.set_global(UiHandle(store.clone()));
    store
}

/// The current state (defaults before `init`).
pub fn get(cx: &App) -> UiState {
    cx.try_global::<UiHandle>()
        .map(|h| h.0.read(cx).state.clone())
        .unwrap_or_default()
}

/// Change the state.
pub fn update(cx: &mut App, f: impl FnOnce(&mut UiState)) {
    let store = store(cx);
    store.update(cx, |s, cx| {
        let mut next = s.state.clone();
        f(&mut next);
        s.set(next, cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn changes_are_kept_and_saved(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("pw-ui-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ui.json"),
            r#"{"v":1,"theme":"light","look":"glass","fontSize":15,"focusRequest":null,"later":7}"#,
        )
        .unwrap();
        let root = dir.clone();
        cx.update(|cx| {
            init(Some(root), cx);
            let s = get(cx);
            assert_eq!(s.theme, crate::theme::ThemePref::Light);
            assert_eq!(crate::theme::appearance(cx).font_size, 15);
            update(cx, |s| s.explorer_show_ignored = true);
            assert!(get(cx).explorer_show_ignored);
        });
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        let back: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("ui.json")).unwrap()).unwrap();
        assert_eq!(back["explorerShowIgnored"], true);
        assert_eq!(back["theme"], "light");
        assert_eq!(back["look"], "glass");
        assert_eq!(back["later"], 7, "unknown keys are written back");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
