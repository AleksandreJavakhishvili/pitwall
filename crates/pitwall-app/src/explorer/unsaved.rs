//! Unsaved edits across the app's file viewers (the full viewer, the panes
//! beside terminals, the windows of their own): quitting Pitwall asks
//! first (Save / Don't Save / Cancel), as closing a tab, a pane or a
//! window does in the viewer itself.

use gpui::{App, Entity, Global, WeakEntity};

use super::viewer::{ask_unsaved, Unsaved, Viewer};

#[derive(Default)]
struct Viewers(Vec<WeakEntity<Viewer>>);

impl Global for Viewers {}

/// Keep `viewer` in the list (a new viewer does).
pub fn register(viewer: WeakEntity<Viewer>, cx: &mut App) {
    let v = cx.default_global::<Viewers>();
    v.0.retain(|w| w.upgrade().is_some());
    v.0.push(viewer);
}

/// The viewers with unsaved changes, and their files.
pub fn unsaved(cx: &App) -> Vec<(Entity<Viewer>, Vec<String>)> {
    let Some(v) = cx.try_global::<Viewers>() else { return vec![] };
    v.0.iter()
        .filter_map(|w| w.upgrade())
        .map(|v| {
            let files = v.read(cx).unsaved(cx);
            (v, files)
        })
        .filter(|(_, files)| !files.is_empty())
        .collect()
}

/// Run `then` (quitting) unless there are unsaved changes and the user
/// cancels, or asks to save them and a save fails.
pub fn quit_guarded(cx: &mut App, then: impl FnOnce(&mut App) + 'static) {
    // (Run once the window that dispatched Quit is done with it: the
    // prompt needs a window to show in.)
    cx.defer(move |cx| ask_then(cx, then));
}

fn ask_then(cx: &mut App, then: impl FnOnce(&mut App) + 'static) {
    let dirty = unsaved(cx);
    if dirty.is_empty() {
        return then(cx);
    }
    let paths: Vec<String> = dirty.iter().flat_map(|(_, p)| p.clone()).collect();
    let Some(win) = cx.active_window().or_else(|| cx.windows().into_iter().next()) else {
        return then(cx);
    };
    let Ok(answer) = win.update(cx, |_, window, cx| ask_unsaved(&paths, window, cx)) else {
        return then(cx);
    };
    cx.spawn(async move |cx| {
        match answer.await {
            Unsaved::Cancel => return,
            Unsaved::Discard => {}
            Unsaved::Save => {
                let saves = cx.update(|cx| dirty.iter().map(|(v, _)| v.update(cx, |v, cx| v.save_all(cx))).collect::<Vec<_>>());
                let mut ok = true;
                for s in saves {
                    ok &= s.await;
                }
                if !ok {
                    return;
                }
            }
        }
        cx.update(then);
    })
    .detach();
}
