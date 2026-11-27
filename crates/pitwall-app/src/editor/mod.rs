//! The text editor (docs/spec/editor.md): files anywhere on this computer,
//! by full path, in tabs, edited with the code view the files viewer uses
//! ([`CodeView`] in its editor's way: several cursors, comments, bracket
//! pairs, indenting Enter; its highlighting, Find and Replace, undo and
//! input methods). Drawn in a tile of the layout or a window of its own
//! (`crate::main_screen::editor_tile`).
//!
//! Saving keeps the viewer's rules (`pitwall_core::explorer::write_local_file`):
//! never over a file that changed since it was read unless the user said so.
//! A file changed on disk (an agent's edit) is shown at once when it has no
//! unsaved edits; with some, the bar asks: Keep mine, Take theirs, Show
//! difference, as in the viewer. Open files are looked at every two seconds.
//! The margin marks lines changed since git's HEAD, and the tab shows the
//! file's git status.
//!
//! Limits, as the viewer's: up to 2 MiB a file is edited; up to 10 MiB it is
//! shown read-only; larger files, binary files and pictures aren't opened.
//! Lines longer than 10,000 bytes are cut where they are shown (the file
//! keeps them).
//!
//! Not here: completion and other language-server help, debugging, editor
//! extensions, a terminal. A language server would plug in beside [`Tab`]
//! (the file's path, its text by version, the cursors), drawing through
//! the code view's marks the way git's do.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gpui::{
    actions, div, prelude::*, px, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton, PathPromptOptions, SharedString,
    Subscription, Task, Window,
};
use pitwall_core::explorer::{ContentKind, FileView, GitFile, Saved};
use pitwall_proto::FileStatus;

use crate::code_view::rows::Side;
use crate::code_view::{CodeView, CodeViewEvent, LineMark};
use crate::explorer::source::{LARGE_CAP, TEXT_CAP};
use crate::explorer::viewer::{ask_unsaved, Unsaved, Viewer};
use crate::kit::file_icons::file_icon;
use crate::kit::{icon, icon_btn, small_btn, tooltip_view, InputEvent, TextInput, MONO_FONT};
use crate::theme::{theme, Theme};

#[cfg(test)]
mod tests;

actions!(
    pw_editor,
    [
        /// ⇧⌘S: save the file under a new name.
        SaveAs,
        /// ⌥⌘S: save every file with unsaved edits.
        SaveAll,
        /// ⌘W in the editor: close the file (asks about unsaved edits).
        CloseFile,
        /// ⌃G (Ctrl+L elsewhere): go to a line.
        GoToLine,
        /// ⌃Tab / ⌃⇧Tab: the next or previous file.
        NextFile,
        PrevFile,
    ]
);

/// The editor's keys (in the `Editor` context, over the code view's).
pub fn bindings() -> Vec<KeyBinding> {
    let c = Some("Editor");
    if cfg!(target_os = "macos") {
        vec![
            KeyBinding::new("cmd-shift-s", SaveAs, c),
            KeyBinding::new("alt-cmd-s", SaveAll, c),
            KeyBinding::new("cmd-w", CloseFile, c),
            KeyBinding::new("ctrl-g", GoToLine, c),
            KeyBinding::new("ctrl-tab", NextFile, c),
            KeyBinding::new("ctrl-shift-tab", PrevFile, c),
        ]
    } else {
        vec![
            KeyBinding::new("ctrl-shift-s", SaveAs, c),
            KeyBinding::new("ctrl-alt-s", SaveAll, c),
            KeyBinding::new("ctrl-w", CloseFile, c),
            KeyBinding::new("ctrl-l", GoToLine, c),
            KeyBinding::new("ctrl-tab", NextFile, c),
            KeyBinding::new("ctrl-shift-tab", PrevFile, c),
        ]
    }
}

/// A file's modification time and size, as last seen.
type Stamp = (Option<SystemTime>, u64);

/// How often open files are looked at for changes on disk.
const POLL: Duration = Duration::from_secs(2);

/// What the editor tells the tile around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorEvent {
    /// Its open files changed (kept in `ui.json`).
    Files,
}

impl EventEmitter<EditorEvent> for Editor {}

/// One open file.
pub struct Tab {
    id: u64,
    /// Its full path (`None`: a new file not saved yet).
    path: Option<String>,
    /// A new file's name ("Untitled-2").
    untitled: String,
    code: Entity<CodeView>,
    _sub: Subscription,
    /// The file as read or last saved: what a save expects on disk.
    disk: Option<String>,
    /// Read again and different while there are unsaved edits.
    newer: Option<String>,
    /// Its modification time and size when last looked at.
    stamp: Option<Stamp>,
    loading: bool,
    saving: bool,
    /// A save found the file changed on disk: the bar asks.
    save_blocked: bool,
    save_error: Option<String>,
    /// Why it isn't shown (it couldn't be read, or isn't text).
    error: Option<String>,
    /// Why it isn't edited (too large, not UTF-8).
    read_only: Option<&'static str>,
    /// "Show difference": the file on disk against the edits.
    diff: Option<Entity<CodeView>>,
    git: GitFile,
    _marks: Option<Task<()>>,
}

impl Tab {
    /// Its name in the tab.
    pub fn name(&self) -> String {
        match &self.path {
            Some(p) => Path::new(p).file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned()),
            None => self.untitled.clone(),
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        let c = self.code.read(cx);
        c.is_dirty() || (self.path.is_none() && c.text().is_some_and(|t| !t.is_empty()))
    }
}

pub struct Editor {
    tabs: Vec<Tab>,
    active: Option<u64>,
    next_id: u64,
    focus: FocusHandle,
    /// "Go to line": its field, open.
    goto: Option<(Entity<TextInput>, Subscription)>,
    _poll: Task<()>,
}

impl Focusable for Editor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.active_tab() {
            Some(t) if t.error.is_none() => t.code.read(cx).focus_handle(cx),
            _ => self.focus.clone(),
        }
    }
}

impl Editor {
    /// An editor with nothing open (`restore`: the files `ui.json` keeps,
    /// opened again).
    pub fn new(restore: bool, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
        });
        let mut e = Editor { tabs: Vec::new(), active: None, next_id: 1, focus: cx.focus_handle(), goto: None, _poll: poll };
        if restore {
            let kept = crate::ui_state::get(cx).editor;
            for p in &kept.tabs {
                e.open(p, cx);
            }
            if let Some(i) = kept.active.and_then(|a| e.tabs.iter().find(|t| t.path.as_deref() == Some(a.as_str()))).map(|t| t.id) {
                e.active = Some(i);
            }
        }
        e
    }

    // ── tabs ────────────────────────────────────────────────────────────

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| Some(t.id) == self.active)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// The file shown's code view.
    pub fn active_code(&self) -> Option<Entity<CodeView>> {
        self.active_tab().map(|t| t.code.clone())
    }

    /// The file shown's full path (`None`: a new file, or none).
    pub fn active_path(&self) -> Option<&str> {
        self.active_tab().and_then(|t| t.path.as_deref())
    }

    fn new_tab(&mut self, path: Option<String>, cx: &mut Context<Self>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let code = cx.new(CodeView::new);
        code.update(cx, |c, cx| {
            c.set_editor(true, cx);
            c.set_editable(true, cx);
        });
        let sub = cx.subscribe(&code, move |this, _, e: &CodeViewEvent, cx| match e {
            CodeViewEvent::Save => this.save(id, cx).detach(),
            CodeViewEvent::Edited => {
                this.marks_later(id, cx);
                cx.notify();
            }
            CodeViewEvent::Loaded => {
                this.marks_later(id, cx);
                cx.notify();
            }
            CodeViewEvent::Comment { .. } | CodeViewEvent::LineClicked { .. } => {}
        });
        let n = (1..).find(|n| !self.tabs.iter().any(|t| t.path.is_none() && t.untitled == format!("Untitled-{n}"))).unwrap_or(1);
        self.tabs.push(Tab {
            id,
            path,
            untitled: format!("Untitled-{n}"),
            code,
            _sub: sub,
            disk: None,
            newer: None,
            stamp: None,
            loading: false,
            saving: false,
            save_blocked: false,
            save_error: None,
            error: None,
            read_only: None,
            diff: None,
            git: GitFile::default(),
            _marks: None,
        });
        id
    }

    /// A new, empty file (named when it is first saved).
    pub fn new_file(&mut self, cx: &mut Context<Self>) -> u64 {
        let id = self.new_tab(None, cx);
        let name = self.tab(id).map(|t| t.untitled.clone()).unwrap_or_default();
        if let Some(t) = self.tab(id) {
            t.code.update(cx, |c, cx| c.set_file(&name, String::new(), cx));
        }
        self.activate(id, cx);
        id
    }

    /// Open the file at full path `path` (shown if it is open already).
    pub fn open(&mut self, path: &str, cx: &mut Context<Self>) -> u64 {
        if let Some(t) = self.tabs.iter().find(|t| t.path.as_deref() == Some(path)) {
            let id = t.id;
            self.activate(id, cx);
            return id;
        }
        let id = self.new_tab(Some(path.to_string()), cx);
        if let Some(t) = self.tab_mut(id) {
            t.loading = true;
        }
        self.read(id, false, cx);
        self.activate(id, cx);
        id
    }

    /// Open `path` and put the cursor on `line` (1-based).
    pub fn open_at(&mut self, path: &str, line: Option<u32>, cx: &mut Context<Self>) {
        let id = self.open(path, cx);
        if let (Some(l), Some(t)) = (line, self.tab(id)) {
            t.code.update(cx, |c, cx| c.reveal(Side::New, l.max(1) as usize - 1, None, cx));
        }
    }

    pub fn activate(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.active != Some(id) {
            self.active = Some(id);
            self.goto = None;
        }
        self.changed_files(cx);
    }

    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.tabs.iter().position(|t| Some(t.id) == self.active) else { return };
        let n = self.tabs.len() as isize;
        let next = self.tabs[((i as isize + by).rem_euclid(n)) as usize].id;
        self.activate(next, cx);
        self.focus_active(window, cx);
    }

    /// The file shown takes the keyboard.
    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        let f = self.focus_handle(cx);
        window.focus(&f, cx);
    }

    /// Close file `id` (its edits dropped: [`Self::request_close`] asks).
    pub fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else { return };
        self.tabs.remove(i);
        if self.active == Some(id) {
            self.active = self.tabs.get(i).or(self.tabs.last()).map(|t| t.id);
            self.goto = None;
        }
        self.changed_files(cx);
    }

    /// The tab's close button, ⌘W: asks first about unsaved edits.
    pub fn request_close(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.guard(Some(vec![id]), window, cx, move |this, _, cx| this.close_tab(id, cx));
    }

    /// The open files changed: kept for the next start, drawn again.
    fn changed_files(&mut self, cx: &mut Context<Self>) {
        let tabs: Vec<String> = self.tabs.iter().filter_map(|t| t.path.clone()).collect();
        let active = self.active_path().map(String::from);
        let prefs = crate::main_screen::workspace::EditorPrefs { tabs, active };
        if crate::ui_state::get(cx).editor != prefs {
            crate::ui_state::update(cx, |u| u.editor = prefs);
        }
        cx.emit(EditorEvent::Files);
        cx.notify();
    }

    // ── reading ─────────────────────────────────────────────────────────

    /// Read file `id` (`quiet`: from the poll or after a blocked save;
    /// what is shown stays unless it can be replaced safely).
    fn read(&mut self, id: u64, quiet: bool, cx: &mut Context<Self>) {
        let Some(path) = self.tab(id).and_then(|t| t.path.clone()) else { return };
        let job = cx.background_executor().spawn(async move {
            let stamp = stamp_of(&path);
            let res = pitwall_core::explorer::read_local_file(&path, true);
            let git = pitwall_core::explorer::local_git_file(&path);
            (res, stamp, git)
        });
        cx.spawn(async move |this, cx| {
            let (res, stamp, git) = job.await;
            let _ = this.update(cx, |this, cx| this.read_done(id, quiet, res, stamp, git, cx));
        })
        .detach();
    }

    fn read_done(&mut self, id: u64, quiet: bool, res: Result<FileView, String>, stamp: Option<Stamp>, git: GitFile, cx: &mut Context<Self>) {
        let Some(t) = self.tab_mut(id) else { return };
        let path = t.path.clone().unwrap_or_default();
        t.loading = false;
        t.stamp = stamp;
        t.git = git;
        let file = res.and_then(|v| text_of(&v));
        match file {
            Err(e) => {
                if !quiet || t.disk.is_none() {
                    t.error = Some(e);
                }
            }
            Ok((text, read_only)) => {
                t.error = None;
                let dirty = t.is_dirty(cx);
                if quiet && t.disk.is_some() {
                    let on_disk = t.newer.as_ref().or(t.disk.as_ref());
                    if on_disk == Some(&text) && !(t.save_blocked && t.newer.is_none()) {
                        // Nothing new.
                    } else if dirty || t.save_blocked {
                        // Unsaved edits: never replaced; the bar asks.
                        let differs = t.disk.as_ref() != Some(&text);
                        t.newer = differs.then_some(text.clone());
                        if let Some(diff) = t.diff.clone() {
                            let mine = t.code.read(cx).text().unwrap_or_default();
                            diff.update(cx, |c, cx| c.set_diff(&path, text, mine, cx));
                        }
                    } else {
                        // Nothing unsaved: the new text, quietly.
                        t.disk = Some(text.clone());
                        t.newer = None;
                        t.read_only = read_only;
                        t.code.update(cx, |c, cx| {
                            c.set_editable(read_only.is_none(), cx);
                            c.set_file(&path, text, cx);
                        });
                    }
                } else {
                    t.disk = Some(text.clone());
                    t.newer = None;
                    t.read_only = read_only;
                    let hint: SharedString = match read_only {
                        Some(why) => format!("Read-only: {why}").into(),
                        None => "Read-only".into(),
                    };
                    t.code.update(cx, |c, cx| {
                        c.set_readonly_text(hint, cx);
                        c.set_editable(read_only.is_none(), cx);
                        c.set_file(&path, text, cx);
                    });
                }
            }
        }
        self.marks_later(id, cx);
        cx.notify();
    }

    /// Every two seconds: open files whose time or size changed on disk
    /// are read again (and their git status with them).
    fn poll(&mut self, cx: &mut Context<Self>) {
        let files: Vec<(u64, String, Option<Stamp>)> =
            self.tabs.iter().filter(|t| !t.loading && !t.saving).filter_map(|t| Some((t.id, t.path.clone()?, t.stamp))).collect();
        if files.is_empty() {
            return;
        }
        let job = cx.background_executor().spawn(async move { files.into_iter().filter(|(_, p, was)| stamp_of(p) != *was).map(|(id, ..)| id).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| {
            let changed = job.await;
            let _ = this.update(cx, |this, cx| {
                for id in changed {
                    this.read(id, true, cx);
                }
            });
        })
        .detach();
    }

    /// ↻ and agents' changes: the open files read again now.
    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.tabs.iter().filter(|t| t.path.is_some() && !t.saving).map(|t| t.id).collect();
        for id in ids {
            self.read(id, true, cx);
        }
    }

    // ── git's marks ─────────────────────────────────────────────────────

    /// The margin's marks for file `id`, worked out again shortly (edits
    /// come in runs).
    fn marks_later(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(t) = self.tab_mut(id) else { return };
        let (head, git) = (t.git.head.clone(), t.git.clone());
        let code = t.code.clone();
        t._marks = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(250)).await;
            let Some(text) = cx.update(|cx| code.read(cx).text()) else { return };
            let marks = cx.background_executor().spawn(async move { line_marks(head.as_deref(), &git, &text) }).await;
            cx.update(|cx| code.update(cx, |c, cx| c.set_line_marks(marks, cx)));
        }));
    }

    // ── saving ──────────────────────────────────────────────────────────

    /// The files with unsaved edits (their names).
    pub fn unsaved(&self, cx: &App) -> Vec<String> {
        self.tabs.iter().filter(|t| t.is_dirty(cx)).map(|t| t.path.clone().unwrap_or_else(|| t.untitled.clone())).collect()
    }

    fn unsaved_ids(&self, cx: &App) -> Vec<u64> {
        self.tabs.iter().filter(|t| t.is_dirty(cx)).map(|t| t.id).collect()
    }

    /// Save file `id` (a new file: asks where first); whether it was
    /// written.
    pub fn save(&mut self, id: u64, cx: &mut Context<Self>) -> Task<bool> {
        let Some(t) = self.tab_mut(id) else { return Task::ready(false) };
        let Some(path) = t.path.clone() else { return self.save_as(id, cx) };
        let Some(text) = t.code.read(cx).text() else { return Task::ready(false) };
        if t.saving || t.read_only.is_some() {
            return Task::ready(false);
        }
        // What it was read as ("Keep mine": what is there now).
        let base = t.disk.clone();
        t.saving = true;
        t.save_error = None;
        let t2 = text.clone();
        let job = cx.background_executor().spawn(async move { pitwall_core::explorer::write_local_file(&path, &t2, base.as_deref()) });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = job.await;
            this.update(cx, |this, cx| this.saved(id, text, res, cx)).unwrap_or(false)
        })
    }

    fn saved(&mut self, id: u64, text: String, res: Result<Saved, String>, cx: &mut Context<Self>) -> bool {
        let Some(t) = self.tab_mut(id) else { return false };
        t.saving = false;
        let ok = match res {
            Ok(Saved::Written) => {
                t.disk = Some(text.clone());
                t.newer = None;
                t.save_blocked = false;
                t.diff = None;
                t.stamp = t.path.as_deref().and_then(stamp_of);
                t.code.update(cx, |c, cx| c.mark_saved(&text, cx));
                self.refresh_git(id, cx);
                true
            }
            Ok(Saved::ChangedOnDisk) => {
                t.save_blocked = true;
                self.read(id, true, cx);
                false
            }
            Err(e) => {
                t.save_error = Some(e);
                false
            }
        };
        cx.notify();
        ok
    }

    /// Its git status and HEAD's text again (after a save).
    fn refresh_git(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(path) = self.tab(id).and_then(|t| t.path.clone()) else { return };
        let job = cx.background_executor().spawn(async move { pitwall_core::explorer::local_git_file(&path) });
        cx.spawn(async move |this, cx| {
            let git = job.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(t) = this.tab_mut(id) {
                    t.git = git;
                }
                this.marks_later(id, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Save As (and a new file's first save): where, asked in the system's
    /// save panel (which asks before replacing a file).
    pub fn save_as(&mut self, id: u64, cx: &mut Context<Self>) -> Task<bool> {
        let Some(t) = self.tab(id) else { return Task::ready(false) };
        let (dir, name) = match &t.path {
            Some(p) => {
                let p = Path::new(p);
                (p.parent().map(Path::to_path_buf).unwrap_or_else(pitwall_core::paths::home), p.file_name().map(|n| n.to_string_lossy().into_owned()))
            }
            None => (pitwall_core::paths::home(), None),
        };
        let picked = cx.prompt_for_new_path(&dir, name.as_deref());
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(to))) = picked.await else { return false };
            let Ok(task) = this.update(cx, |this, cx| this.save_to(id, &to, cx)) else { return false };
            task.await
        })
    }

    /// Save file `id` as `to` (the user chose it: a file there is
    /// replaced); the tab shows it from now on.
    pub fn save_to(&mut self, id: u64, to: &Path, cx: &mut Context<Self>) -> Task<bool> {
        let Some(t) = self.tab(id) else { return Task::ready(false) };
        let Some(text) = t.code.read(cx).text() else { return Task::ready(false) };
        let to = to.to_string_lossy().into_owned();
        let (p, t2) = (to.clone(), text.clone());
        let job = cx.background_executor().spawn(async move {
            if Path::new(&p).exists() {
                pitwall_core::explorer::write_local_file(&p, &t2, None).map(|_| ())
            } else {
                pitwall_core::explorer::create_local_file(&p, &t2)
            }
        });
        cx.spawn(async move |this, cx| {
            let res = job.await;
            this.update(cx, |this, cx| {
                if let Err(e) = res {
                    if let Some(t) = this.tab_mut(id) {
                        t.save_error = Some(e);
                    }
                    cx.notify();
                    return false;
                }
                // Another tab showing that file shows an older copy: closed.
                this.tabs.retain(|t| t.id == id || t.path.as_deref() != Some(to.as_str()));
                let Some(t) = this.tab_mut(id) else { return false };
                t.path = Some(to.clone());
                t.disk = Some(text.clone());
                t.newer = None;
                t.save_blocked = false;
                t.save_error = None;
                t.diff = None;
                t.read_only = None;
                t.stamp = stamp_of(&to);
                // Shown as the new name (its language with it).
                t.code.update(cx, |c, cx| {
                    c.set_file(&to, text, cx);
                    c.set_editable(true, cx);
                });
                this.refresh_git(id, cx);
                this.changed_files(cx);
                true
            })
            .unwrap_or(false)
        })
    }

    /// Save every file with unsaved edits; whether all were written.
    pub fn save_all(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        let saves: Vec<Task<bool>> = self.unsaved_ids(cx).into_iter().map(|id| self.save(id, cx)).collect();
        cx.spawn(async move |_, _| {
            let mut ok = true;
            for s in saves {
                ok &= s.await;
            }
            ok
        })
    }

    /// Before dropping files `ids` (every file when `None`): ask about
    /// unsaved edits, then `go` (after saving them when asked and it
    /// worked; not at all on Cancel).
    pub fn guard(&mut self, ids: Option<Vec<u64>>, window: &mut Window, cx: &mut Context<Self>, go: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        let dirty: Vec<u64> = match ids {
            Some(ids) => ids.into_iter().filter(|id| self.tab(*id).is_some_and(|t| t.is_dirty(cx))).collect(),
            None => self.unsaved_ids(cx),
        };
        if dirty.is_empty() {
            return go(self, window, cx);
        }
        let names: Vec<String> = dirty.iter().filter_map(|id| self.tab(*id)).map(|t| t.path.clone().unwrap_or_else(|| t.untitled.clone())).collect();
        let answer = ask_unsaved(&names, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            match answer.await {
                Unsaved::Cancel => return,
                Unsaved::Discard => {}
                Unsaved::Save => {
                    let Ok(saves) = this.update(cx, |this, cx| dirty.iter().map(|id| this.save(*id, cx)).collect::<Vec<_>>()) else { return };
                    let mut ok = true;
                    for s in saves {
                        ok &= s.await;
                    }
                    if !ok {
                        return;
                    }
                }
            }
            let _ = this.update_in(cx, |this, window, cx| go(this, window, cx));
        })
        .detach();
    }

    // ── an agent's change while editing ─────────────────────────────────

    /// "Keep mine": the edits stay; a save writes over what is on disk now
    /// (it was seen), and goes on when a save was waiting.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active else { return };
        let Some(t) = self.tab_mut(id) else { return };
        let was_blocked = std::mem::take(&mut t.save_blocked);
        if let Some(n) = t.newer.take() {
            t.disk = Some(n);
        }
        t.diff = None;
        if was_blocked {
            self.save(id, cx).detach();
        }
        cx.notify();
    }

    /// "Take theirs": the file on disk replaces the edits.
    pub fn take_theirs(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active else { return };
        let Some(t) = self.tab_mut(id) else { return };
        t.save_blocked = false;
        t.diff = None;
        match t.newer.take() {
            Some(n) => {
                let path = t.path.clone().unwrap_or_default();
                t.disk = Some(n.clone());
                let ro = t.read_only.is_none();
                t.code.update(cx, |c, cx| {
                    c.set_editable(false, cx);
                    c.set_file(&path, n, cx);
                    c.set_editable(ro, cx);
                });
            }
            None => self.read(id, false, cx),
        }
        cx.notify();
    }

    /// "Show difference": the file on disk against the edits, in place of
    /// the editor (again: back to editing).
    pub fn toggle_difference(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active else { return };
        let Some(t) = self.tab_mut(id) else { return };
        if t.diff.take().is_some() {
            cx.notify();
            return;
        }
        let theirs = t.newer.clone().or(t.disk.clone());
        let mine = t.code.read(cx).text();
        let (Some(theirs), Some(mine)) = (theirs, mine) else { return };
        let path = t.path.clone().unwrap_or_else(|| t.untitled.clone());
        let view = cx.new(CodeView::new);
        view.update(cx, |c, cx| {
            c.set_layout(crate::code_view::Layout::Unified, cx);
            c.set_diff(&path, theirs, mine, cx);
        });
        t.diff = Some(view);
        cx.notify();
    }

    /// Whether the file shown changed on disk while it has unsaved edits
    /// (or a save found it so).
    pub fn conflict(&self) -> bool {
        self.active_tab().is_some_and(|t| t.newer.is_some() || t.save_blocked)
    }

    // ── go to line ──────────────────────────────────────────────────────

    pub fn open_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.active_tab().filter(|t| t.error.is_none()) else { return };
        let lines = t.code.read(cx).file_facts().map_or(0, |f| f.2);
        let field = cx.new(|cx| TextInput::new(cx, "", &format!("Go to line (1–{lines}), or line:column")).plain());
        let sub = cx.subscribe_in(&field, window, |this, field, e: &InputEvent, window, cx| match e {
            InputEvent::Submit => {
                let q = field.read(cx).text().to_string();
                this.goto = None;
                this.go_to(&q, cx);
                this.focus_active(window, cx);
                cx.notify();
            }
            InputEvent::Cancel => {
                this.goto = None;
                this.focus_active(window, cx);
                cx.notify();
            }
            _ => {}
        });
        field.update(cx, |f, cx| f.focus(window, cx));
        self.goto = Some((field, sub));
        cx.notify();
    }

    /// Go to `q`: "line" or "line:column" (1-based).
    pub fn go_to(&mut self, q: &str, cx: &mut Context<Self>) {
        let Some(t) = self.active_tab() else { return };
        let mut parts = q.trim().splitn(2, [':', ',']);
        let Some(line) = parts.next().and_then(|l| l.trim().parse::<usize>().ok()) else { return };
        let col = parts.next().and_then(|c| c.trim().parse::<usize>().ok());
        let code = t.code.clone();
        code.update(cx, |c, cx| {
            let lines = c.file_facts().map_or(1, |f| f.2).max(1);
            let l = line.clamp(1, lines) - 1;
            let cols = col.and_then(|n| {
                let text = c.text()?;
                let src = text.split('\n').nth(l)?;
                let b = src.char_indices().nth(n.max(1) - 1).map_or(src.trim_end_matches('\r').len(), |(i, _)| i);
                Some(b..b)
            });
            c.reveal(Side::New, l, cols, cx);
        });
    }

    // ── drawing ─────────────────────────────────────────────────────────

    fn tabs_row(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div().id("editor-tabs").flex_none().h(px(32.)).flex().items_end().overflow_x_scroll().border_b_1().border_color(t.line).bg(t.surface);
        for (i, tab) in self.tabs.iter().enumerate() {
            let on = Some(tab.id) == self.active;
            let id = tab.id;
            let dirty = tab.is_dirty(cx);
            let name = tab.name();
            let tip: SharedString = tab.path.clone().map(|p| pitwall_core::paths::tildify(&p)).unwrap_or_else(|| "A new file, not saved yet".into()).into();
            let icon_path = tab.path.clone().unwrap_or_else(|| tab.untitled.clone());
            let status = tab.git.status;
            row = row.child(
                div()
                    .id(("editor-tab", i))
                    .debug_selector(move || format!("editor-tab-{i}"))
                    .group("editor-tab")
                    .flex_none()
                    .h_full()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_r_1()
                    .border_color(t.line)
                    .when(on, |d| d.bg(t.bg))
                    .text_size(px(12.5))
                    .text_color(if on { t.text } else { t.text_2 })
                    .cursor_pointer()
                    .tooltip(move |_, cx| tooltip_view(tip.clone(), cx))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                        this.activate(id, cx);
                        this.focus_active(window, cx);
                    }))
                    .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.request_close(id, window, cx)))
                    .child(file_icon(&icon_path, 14.))
                    .child(div().max_w(px(220.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                    .children(status.map(|s| crate::explorer::widgets::status_mark(t, ("editor-tab-status", i), s)))
                    .child(
                        div()
                            .id(("editor-tab-close", i))
                            .debug_selector(move || format!("editor-tab-close-{i}"))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .hover(|d| d.bg(t.surface_3))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, window, cx| this.request_close(id, window, cx)))
                            .map(|d| {
                                if dirty {
                                    d.child(div().size(px(7.)).rounded_full().bg(t.text_2)).tooltip(|_, cx| tooltip_view("Unsaved changes: close", cx))
                                } else {
                                    d.child(icon("x", 11., t.text_3))
                                }
                            }),
                    ),
            );
        }
        row.into_any_element()
    }

    fn notices(&self, tab: &Tab, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if tab.newer.is_some() || tab.save_blocked {
            let text = if tab.save_blocked { "Changed on disk since it was read: saving would overwrite it." } else { "Changed on disk. You have unsaved changes." };
            let diffing = tab.diff.is_some();
            out.push(
                Viewer::notice("editor-changed-on-disk", true, text.into(), t)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(6.))
                            .child(small_btn("editor-keep-mine", if tab.save_blocked { "Save mine" } else { "Keep mine" }, t).debug_selector(|| "editor-keep-mine".into()).tooltip(|_, cx| tooltip_view("Keep your changes; saving writes them over the file on disk", cx)).on_click(cx.listener(|this, _, _, cx| this.keep_mine(cx))))
                            .child(small_btn("editor-take-theirs", "Take theirs", t).debug_selector(|| "editor-take-theirs".into()).tooltip(|_, cx| tooltip_view("Drop your changes and show the file on disk", cx)).on_click(cx.listener(|this, _, _, cx| this.take_theirs(cx))))
                            .child(small_btn("editor-show-difference", if diffing { "Hide difference" } else { "Show difference" }, t).debug_selector(|| "editor-show-difference".into()).tooltip(|_, cx| tooltip_view("The file on disk against your changes", cx)).on_click(cx.listener(|this, _, _, cx| this.toggle_difference(cx)))),
                    )
                    .into_any_element(),
            );
        }
        if let Some(e) = tab.save_error.clone() {
            out.push(Viewer::notice("editor-save-error", false, format!("Couldn't save: {}", e.lines().next().unwrap_or_default()).into(), t).into_any_element());
        }
        out
    }

    fn status_line(&self, tab: &Tab, t: &Theme, cx: &App) -> AnyElement {
        let c = tab.code.read(cx);
        let mut parts: Vec<String> = Vec::new();
        if let Some((l, col)) = c.caret() {
            parts.push(format!("Ln {}, Col {}", l + 1, col + 1));
        }
        let n = c.cursor_count();
        if n > 1 {
            parts.push(format!("{n} cursors"));
        }
        if let Some((indent, crlf, _)) = c.file_facts() {
            parts.push(match indent {
                crate::code_view::text::Indent::Tabs => "Tabs".to_string(),
                crate::code_view::text::Indent::Spaces(n) => format!("Spaces: {n}"),
            });
            parts.push(if crlf { "CRLF" } else { "LF" }.to_string());
        }
        if let Some(why) = tab.read_only {
            parts.push(format!("Read-only: {why}"));
        }
        if tab.saving {
            parts.push("Saving…".into());
        }
        div()
            .id("editor-status")
            .debug_selector(|| "editor-status".into())
            .flex_none()
            .h(px(22.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(14.))
            .border_t_1()
            .border_color(t.line)
            .font_family(MONO_FONT)
            .text_size(px(11.))
            .text_color(t.text_3)
            .children(parts)
            .into_any_element()
    }

    fn empty(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("editor-empty")
            .debug_selector(|| "editor-empty".into())
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .text_color(t.text_3)
            .child("No files open")
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(small_btn("editor-empty-new", "New file", t).on_click(cx.listener(|this, _, window, cx| {
                        this.new_file(cx);
                        this.focus_active(window, cx);
                    })))
                    .child(small_btn("editor-empty-open", "Open file…", t).on_click(cx.listener(|this, _, _, cx| this.pick_files(cx)))),
            )
            .into_any_element()
    }

    /// "Open file…": the system's open panel, then the files picked.
    pub fn pick_files(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Open in Editor".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let _ = this.update(cx, |this, cx| {
                for p in paths {
                    this.open(&p.to_string_lossy(), cx);
                }
            });
        })
        .detach();
    }

    /// The buttons over the tabs: new, open, save.
    pub fn file_buttons(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .child(icon_btn("editor-new", "plus", "New file", true, t).debug_selector(|| "editor-new".into()).on_click(cx.listener(|this, _, window, cx| {
                this.new_file(cx);
                this.focus_active(window, cx);
            })))
            .child(icon_btn("editor-open", "folder", "Open file…", true, t).debug_selector(|| "editor-open".into()).on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))))
            .child(icon_btn("editor-save", "check", crate::kit::keys("Save (⌘S)", false), true, t).debug_selector(|| "editor-save".into()).on_click(cx.listener(|this, _, _, cx| {
                if let Some(id) = this.active {
                    this.save(id, cx).detach();
                }
            })))
            .into_any_element()
    }

    /// The file shown's id.
    pub fn active_id(&self) -> Option<u64> {
        self.active
    }

    /// The bench's `editor key …`: the editor's keys and the code view's
    /// (nothing typed on the screen).
    pub fn debug_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active else { return };
        match key {
            "save" => self.save(id, cx).detach(),
            "save-all" => self.save_all(cx).detach(),
            "close" => self.request_close(id, window, cx),
            "goto" => self.open_goto(window, cx),
            k => {
                if let Some(code) = self.active_code() {
                    code.update(cx, |c, cx| c.debug_key(k, window, cx));
                }
            }
        }
        cx.notify();
    }

    /// What the bench's `editor state` prints.
    pub fn debug_state(&self, cx: &App) -> String {
        let tabs: Vec<String> = self.tabs.iter().map(|t| format!("{}{}{}", t.name(), if t.is_dirty(cx) { "*" } else { "" }, t.git.status.map(|s| format!("[{s:?}]")).unwrap_or_default())).collect();
        let a = self.active_tab();
        let code = a.map(|t| t.code.read(cx));
        format!(
            "tabs={tabs:?} active={:?} cursors={} caret={:?} conflict={} blocked={} save_error={:?} error={:?} read_only={:?} marks={:?} text={:?}",
            a.map(|t| t.name()),
            code.map_or(0, |c| c.cursor_count()),
            code.and_then(|c| c.caret()),
            self.conflict(),
            a.is_some_and(|t| t.save_blocked),
            a.and_then(|t| t.save_error.clone()),
            a.and_then(|t| t.error.clone()),
            a.and_then(|t| t.read_only),
            code.map(|c| c.line_marks().len()),
            code.and_then(|c| c.text()).map(|t| t.chars().take(120).collect::<String>()),
        )
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let body = match self.active_tab() {
            None => self.empty(&t, cx),
            Some(tab) => {
                let notices = self.notices(tab, &t, cx);
                let main = match (&tab.error, &tab.diff) {
                    (Some(e), _) => div().flex_1().flex().items_center().justify_center().p(px(24.)).text_color(t.text_3).child(e.clone()).into_any_element(),
                    (None, Some(d)) => div().flex_1().min_h_0().flex().child(d.clone()).into_any_element(),
                    (None, None) if tab.loading => div().flex_1().into_any_element(),
                    (None, None) => div().flex_1().min_h_0().flex().child(tab.code.clone()).into_any_element(),
                };
                let status = (tab.error.is_none()).then(|| self.status_line(tab, &t, cx));
                let goto = self.goto.as_ref().map(|(f, _)| {
                    div()
                        .id("editor-goto")
                        .debug_selector(|| "editor-goto".into())
                        .absolute()
                        .top(px(8.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(div().w(px(320.)).h(px(30.)).px(px(8.)).flex().items_center().rounded(crate::theme::RADIUS).bg(t.raised).border_1().border_color(t.line_strong).shadow_md().text_size(px(12.5)).child(f.clone()))
                });
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .relative()
                    .children(notices)
                    .child(main)
                    .children(status)
                    .children(goto)
                    .into_any_element()
            }
        };
        div()
            .id("editor")
            .key_context("Editor")
            .track_focus(&self.focus)
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.bg)
            .on_action(cx.listener(|this, _: &SaveAs, _, cx| {
                if let Some(id) = this.active {
                    this.save_as(id, cx).detach();
                }
            }))
            .on_action(cx.listener(|this, _: &SaveAll, _, cx| this.save_all(cx).detach()))
            .on_action(cx.listener(|this, _: &CloseFile, window, cx| {
                if let Some(id) = this.active {
                    this.request_close(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &GoToLine, window, cx| this.open_goto(window, cx)))
            .on_action(cx.listener(|this, _: &NextFile, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevFile, window, cx| this.step(-1, window, cx)))
            .when(!self.tabs.is_empty(), |d| d.child(self.tabs_row(&t, cx)))
            .child(body)
    }
}

/// A file's modification time and size (`None`: it isn't there).
fn stamp_of(path: &str) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok(), m.len()))
}

/// The text the editor shows of a file as read, and why it is read-only
/// (if it is); an error for what it doesn't open.
fn text_of(v: &FileView) -> Result<(String, Option<&'static str>), String> {
    let mb = |n: u64| n / (1024 * 1024);
    match (&v.kind, &v.text) {
        (ContentKind::TooLarge, _) => Err(format!("Too large to open in the editor (over {} MiB).", mb(LARGE_CAP))),
        (ContentKind::Binary, _) => Err("A binary file: the editor opens text files.".into()),
        (ContentKind::Image, None) => Err("A picture: open it in the files viewer.".into()),
        (_, None) => Err("It couldn't be read as text.".into()),
        (_, Some(text)) => {
            let ro = if v.size > TEXT_CAP {
                Some("too large to edit (over 2 MiB)")
            } else if v.lossy {
                Some("not UTF-8 text")
            } else {
                None
            };
            Ok((text.clone(), ro))
        }
    }
}

/// The margin's marks: lines added, changed, or deleted (marked on the
/// line after) since git's HEAD. A file git doesn't know yet is all new.
pub fn line_marks(head: Option<&str>, git: &GitFile, text: &str) -> Vec<(Range<usize>, LineMark)> {
    let Some(head) = head else {
        if git.in_repo && matches!(git.status, Some(FileStatus::U) | Some(FileStatus::A)) {
            let n = text.split('\n').count() - usize::from(text.ends_with('\n'));
            return vec![(0..n.max(1), LineMark::Added)];
        }
        return Vec::new();
    };
    if head == text {
        return Vec::new();
    }
    let d = crate::code_view::diff::diff(head, text);
    d.hunks
        .iter()
        .map(|h| {
            if h.old.is_empty() {
                (h.new.clone(), LineMark::Added)
            } else if h.new.is_empty() {
                (h.new.start..h.new.start + 1, LineMark::Deleted)
            } else {
                (h.new.clone(), LineMark::Changed)
            }
        })
        .collect()
}

/// A path the editor is asked to open, made full (`~` expanded).
pub fn full_path(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => pitwall_core::paths::home().join(rest),
        None => PathBuf::from(p),
    }
}
