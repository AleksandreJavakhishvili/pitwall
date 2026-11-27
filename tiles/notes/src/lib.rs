//! Notes: private plain-text notes beside the agents. This is the preview
//! on SDK phase 1 (step 0 of docs/spec/2026-11-27-notes-tile/plan.md):
//! notes in the tile's own storage, edited in Pitwall's multi-line text
//! field, saved about a second after typing stops and when the tile is
//! hidden or loses focus. An agent (or the user) can open the tile with a
//! draft note (input.schema.json); it stays unsaved until the user keeps it.

mod note;

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use note::{Note, Stored};
use pitwall_extension::prelude::*;
// Not in the prelude, though helpers that build part of a tree return it.
use pitwall_extension::Div;
use serde::{Deserialize, Serialize};

/// Saved this long after typing stops (frd.md FR-6).
const SAVE_AFTER: Duration = Duration::from_secs(1);
/// Notes read from storage per second: under the 200 host calls a second a
/// tile may make, with room for saving.
const LOAD_BATCH: usize = 150;
/// From this width (logical pixels) a view shows the list and the open note
/// side by side.
const SPLIT_AT: f32 = 560.0;
/// The height of a pane's header, as Pitwall's own tiles' bars.
const BAR: f32 = 36.0;

/// What the tile is opened with (input.schema.json).
#[derive(Deserialize)]
struct DraftInput {
    title: Option<String>,
    text: String,
}

/// What a view's editor shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Open {
    Note(u64),
    Draft,
}

/// One view's own state: each place the tile is open has its own note,
/// search and draft.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ViewState {
    open: Option<Open>,
    filter: String,
    /// A draft it was opened with, not saved until the user keeps it.
    draft: Option<String>,
    #[serde(skip)]
    confirm_delete: bool,
    #[serde(skip)]
    width: f32,
    #[serde(skip)]
    focus_editor: bool,
}

/// What `save_state` keeps across a drop or a reload: unsaved text and
/// each view's place.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    unsaved: Vec<(u64, String)>,
    views: BTreeMap<String, ViewState>,
}

struct Notes {
    /// Every note read so far, newest change first.
    notes: Vec<Note>,
    /// Edits not saved yet, by note.
    pending: BTreeMap<u64, String>,
    views: BTreeMap<String, ViewState>,
    /// Views' state from before a reload, taken when each opens again.
    restored: BTreeMap<String, ViewState>,
    next_id: u64,
    saver: Option<Timer>,
    last_edit: Instant,
    /// Notes in storage not read yet (more than one batch).
    to_load: Vec<u64>,
    /// A save failed and the user was told; not told again until one works.
    save_failed: bool,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Tile for Notes {
    fn new(cx: &mut Cx<Self>) -> Self {
        let mut ids: Vec<u64> = match storage::list(note::PREFIX) {
            Ok(keys) => keys.iter().filter_map(|k| note::id_of(k)).collect(),
            Err(e) => {
                log::error(format!("the notes couldn't be listed: {e}"));
                Vec::new()
            }
        };
        ids.sort_unstable_by(|a, b| b.cmp(a));
        let next_id = ids.first().map_or(1, |m| m + 1);
        let mut this = Notes {
            notes: Vec::new(),
            pending: BTreeMap::new(),
            views: BTreeMap::new(),
            restored: BTreeMap::new(),
            next_id,
            saver: None,
            last_edit: Instant::now(),
            to_load: ids,
            save_failed: false,
        };
        // Unsaved text from before a drop or a reload (FR-7): kept as
        // edits, saved a second from now.
        if let Some(saved) = cx.restore::<Saved>() {
            for (id, text) in saved.unsaved {
                this.next_id = this.next_id.max(id + 1);
                this.pending.insert(id, text);
            }
            this.restored = saved.views;
            if !this.pending.is_empty() {
                this.edited(cx);
            }
        }
        this.load_some(cx);
        this
    }

    fn on_view_opened(&mut self, view: &ViewInfo, _cx: &mut Cx<Self>) {
        let mut vs = self.restored.remove(&view.id).unwrap_or_default();
        if let Some(Open::Note(id)) = vs.open {
            if !self.has(id) {
                vs.open = None;
            }
        }
        if vs.open == Some(Open::Draft) && vs.draft.is_none() {
            vs.open = None;
        }
        self.views.insert(view.id.clone(), vs);
    }

    fn on_content(&mut self, view: &ViewInfo, cx: &mut Cx<Self>) {
        let Some(content) = &view.content else { return };
        match content.parse::<DraftInput>() {
            Ok(d) => {
                self.leave(&view.id, cx);
                let vs = self.view_mut(&view.id);
                vs.draft = Some(note::draft_text(d.title.as_deref(), &d.text));
                vs.open = Some(Open::Draft);
            }
            Err(e) => {
                log::warn(e.to_string());
                ui::toast("That draft couldn't be shown", Tone::Warn);
            }
        }
    }

    fn on_view_closed(&mut self, view: &str, cx: &mut Cx<Self>) {
        self.leave(view, cx);
        self.views.remove(view);
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        match event {
            // Hidden or left: save now (FR-6). There is no blur event per
            // text field, so the tile losing focus stands in for it.
            Event::Visibility(false) | Event::Focus(false) => {
                if !self.pending.is_empty() {
                    self.flush(cx);
                    cx.notify();
                }
            }
            Event::Resize { view, width, .. } => {
                let vs = self.view_mut(view);
                let was = vs.width >= SPLIT_AT;
                vs.width = *width;
                if was != (*width >= SPLIT_AT) {
                    cx.notify();
                }
            }
            Event::Theme(_) | Event::SettingsChanged | Event::GrantsChanged(_) => cx.notify(),
            _ => {}
        }
    }

    fn save_state(&self) -> Option<SavedState> {
        let unsaved = self.pending.iter().map(|(id, t)| (*id, t.clone())).collect();
        SavedState::json(&Saved { unsaved, views: self.views.clone() })
    }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        let vs = self.views.get(view).cloned().unwrap_or_default();
        let split = vs.width >= SPLIT_AT;
        let v = view.to_string();
        let new = cx.listener(move |this: &mut Notes, _: &KeyPress, cx| this.new_note(&v, cx));
        cx.bind_key("cmd-n", new);
        if vs.focus_editor {
            self.view_mut(view).focus_editor = false;
            match vs.open {
                Some(Open::Note(id)) => cx.focus(format!("editor/n{id}/text")),
                Some(Open::Draft) => cx.focus("editor/draft/text"),
                None => {}
            }
        }
        let now = now_ms();
        let root = div().size_full().bg(Token::Surface).text_color(Token::Text);
        if split {
            root.flex_row()
                .child(self.list_pane(view, &vs, now, cx).w(px(240.)).flex_none().border_r_1().border_color(Token::Line))
                .child(self.editor_pane(view, &vs, split, cx).flex_1())
        } else if vs.open.is_some() {
            root.child(self.editor_pane(view, &vs, split, cx).flex_1())
        } else {
            root.child(self.list_pane(view, &vs, now, cx).flex_1())
        }
    }
}

// ---- Rendering --------------------------------------------------------------

/// A pane's header bar.
fn bar() -> Div {
    h_flex().id("head").flex_none().h(px(BAR)).px_2().gap_1p5().border_b_1().border_color(Token::Line)
}

impl Notes {
    /// The search field, the New button and the notes, newest first.
    fn list_pane(&self, view: &str, vs: &ViewState, now: u64, cx: &mut Cx<Self>) -> Div {
        let (v1, v2) = (view.to_string(), view.to_string());
        let head = bar()
            .child(
                input("search")
                    .placeholder("Search notes")
                    .value(vs.filter.clone())
                    .flex_1()
                    .text_sm()
                    .on_change(cx.listener(move |this: &mut Notes, q: &str, cx| {
                        this.view_mut(&v1).filter = q.to_string();
                        cx.notify();
                    })),
            )
            .child(icon_button(Icon::Plus, "New note").id("new").tooltip("New note (⌘N)").on_click(cx.listener(move |this: &mut Notes, _, cx| this.new_note(&v2, cx))));

        let query = vs.filter.trim().to_lowercase();
        let shown: Vec<&Note> = self.notes.iter().filter(|n| note::matches(self.text_of(n), &query)).collect();
        let mut rows = div().id("rows").flex_1().min_h(px(0.)).overflow_y_scroll().py_1();
        if let Some(draft) = &vs.draft {
            let v = view.to_string();
            rows = rows.child(
                v_flex()
                    .id("draft")
                    .px_3()
                    .py_2()
                    .gap_0p5()
                    .cursor_pointer()
                    .focusable()
                    .bg(Token::WarnSoft)
                    .child(text(note::title_or_new(draft)).text_sm().font_medium().truncate())
                    .child(text("Draft, not saved").text_xs().text_color(Token::Warn))
                    .on_click(cx.listener(move |this: &mut Notes, _, cx| this.open(&v, Open::Draft, cx))),
            );
        }
        if self.notes.is_empty() && vs.draft.is_none() {
            rows = rows.child(
                v_flex()
                    .id("empty")
                    .px_3()
                    .py_3()
                    .gap_1()
                    .child(text("No notes yet").text_sm().text_color(Token::Text2))
                    .child(text("Press ⌘N or + to write one. Notes stay in this Pitwall and are saved as you type.").text_xs().text_color(Token::Text3)),
            );
        } else if shown.is_empty() && !query.is_empty() {
            rows = rows.child(text(format!("No notes match “{}”", vs.filter.trim())).id("none").px_3().py_3().text_sm().text_color(Token::Text3));
        }
        for n in shown {
            rows = rows.child(self.row(view, vs, n, now, cx));
        }
        v_flex().id("list").min_w(px(0.)).child(head).child(rows)
    }

    /// One note in the list: its title, when it changed, and its next line.
    fn row(&self, view: &str, vs: &ViewState, n: &Note, now: u64, cx: &mut Cx<Self>) -> AnyElement {
        let text_now = self.text_of(n);
        let title = note::title(text_now);
        let selected = vs.open == Some(Open::Note(n.id));
        let (v, id) = (view.to_string(), n.id);
        let when = if self.pending.contains_key(&n.id) { "editing".to_string() } else { note::ago(now, n.updated) };
        v_flex()
            .id(format!("n{}", n.id))
            .px_3()
            .py_2()
            .gap_0p5()
            .cursor_pointer()
            .focusable()
            .hover_bg(Token::Surface2)
            .when(selected, |r| r.bg(Token::Surface2))
            .child(
                h_flex()
                    .gap_2()
                    .child(text(title.clone().unwrap_or_else(|| "New note".into())).text_sm().font_medium().truncate().flex_1().when(title.is_none(), |t| t.text_color(Token::Text3)))
                    .child(text(when).text_xs().text_color(Token::Text3).flex_none()),
            )
            .when_some(note::preview(text_now), |r, p| r.child(text(p).text_xs().text_color(Token::Text3).truncate()))
            .on_click(cx.listener(move |this: &mut Notes, _, cx| this.open(&v, Open::Note(id), cx)))
            .into_any()
    }

    /// The open note (or draft) in a multi-line field, under a bar with its
    /// title, whether it is saved, and Delete.
    fn editor_pane(&self, view: &str, vs: &ViewState, split: bool, cx: &mut Cx<Self>) -> Div {
        let pane = v_flex().id("editor").min_w(px(0.));
        let back = |cx: &mut Cx<Self>| {
            let v = view.to_string();
            icon_button(Icon::Named("back".into()), "All notes").id("back").on_click(cx.listener(move |this: &mut Notes, _, cx| this.back(&v, cx)))
        };
        match vs.open {
            None => pane.items_center().justify_center().p_4().child(text("Select a note, or press ⌘N for a new one").text_sm().text_color(Token::Text3)),
            Some(Open::Draft) => {
                let draft = vs.draft.clone().unwrap_or_default();
                let (v1, v2, v3, v4) = (view.to_string(), view.to_string(), view.to_string(), view.to_string());
                let head = bar()
                    .when(!split, |h| h.child(back(cx)))
                    .child(text(note::title_or_new(&draft)).text_sm().font_medium().truncate().flex_1())
                    .child(text("Draft, not saved").text_xs().text_color(Token::Warn).flex_none())
                    .child(button("Discard").id("discard").on_click(cx.listener(move |this: &mut Notes, _, cx| this.discard_draft(&v1, cx))))
                    .child(button("Keep").primary().id("keep").tooltip("Save it as a note (⌘⏎)").on_click(cx.listener(move |this: &mut Notes, _, cx| this.keep_draft(&v2, cx))));
                let field = input("text")
                    .multiline()
                    .value(draft)
                    .placeholder("Title on the first line, then anything")
                    .size_full()
                    .on_change(cx.listener(move |this: &mut Notes, t: &str, cx| {
                        let vs = this.view_mut(&v3);
                        let retitled = vs.draft.as_deref().map(note::title) != Some(note::title(t));
                        vs.draft = Some(t.to_string());
                        if retitled {
                            cx.notify();
                        }
                    }))
                    .on_submit(cx.listener(move |this: &mut Notes, _, cx| this.keep_draft(&v4, cx)));
                pane.child(head).child(v_flex().id("draft").flex_1().min_h(px(0.)).p_2().child(field))
            }
            Some(Open::Note(id)) => {
                let text_now = self.notes.iter().find(|n| n.id == id).map(|n| self.text_of(n)).unwrap_or_default().to_string();
                let title = note::title_or_new(&text_now);
                let head = if vs.confirm_delete {
                    let (v1, v2) = (view.to_string(), view.to_string());
                    bar()
                        .child(text(format!("Delete “{title}”?")).text_sm().truncate().flex_1())
                        .child(button("Cancel").id("cancel").on_click(cx.listener(move |this: &mut Notes, _, cx| {
                            this.view_mut(&v1).confirm_delete = false;
                            cx.notify();
                        })))
                        .child(button("Delete").danger().id("delete").on_click(cx.listener(move |this: &mut Notes, _, cx| this.delete(&v2, cx))))
                } else {
                    let v = view.to_string();
                    let saved = if self.pending.contains_key(&id) { "Editing…" } else { "Saved" };
                    bar()
                        .when(!split, |h| h.child(back(cx)))
                        .child(text(title).text_sm().font_medium().truncate().flex_1())
                        .child(text(saved).text_xs().text_color(Token::Text3).flex_none())
                        .child(icon_button(Icon::Trash, "Delete note").id("trash").on_click(cx.listener(move |this: &mut Notes, _, cx| {
                            this.view_mut(&v).confirm_delete = true;
                            cx.notify();
                        })))
                };
                let v = view.to_string();
                let field = input("text")
                    .multiline()
                    .value(text_now)
                    .placeholder("Title on the first line, then anything")
                    .size_full()
                    .on_change(cx.listener(move |this: &mut Notes, t: &str, cx| this.typed(id, t, cx)))
                    .on_submit(cx.listener(move |this: &mut Notes, _, cx| this.done(&v, cx)));
                pane.child(head).child(v_flex().id(format!("n{id}")).flex_1().min_h(px(0.)).p_2().child(field))
            }
        }
    }
}

// ---- What the user does ---------------------------------------------------

impl Notes {
    fn view_mut(&mut self, view: &str) -> &mut ViewState {
        self.views.entry(view.to_string()).or_default()
    }

    fn has(&self, id: u64) -> bool {
        self.notes.iter().any(|n| n.id == id)
    }

    /// A note's text now: with its unsaved edits.
    fn text_of<'a>(&'a self, n: &'a Note) -> &'a str {
        self.pending.get(&n.id).map_or(n.text.as_str(), String::as_str)
    }

    fn new_note(&mut self, view: &str, cx: &mut Cx<Self>) {
        self.leave(view, cx);
        let id = self.next_id;
        self.next_id += 1;
        self.notes.insert(0, Note { id, text: String::new(), updated: now_ms() });
        let vs = self.view_mut(view);
        vs.open = Some(Open::Note(id));
        vs.filter.clear();
        vs.focus_editor = true;
        cx.notify();
    }

    fn open(&mut self, view: &str, what: Open, cx: &mut Cx<Self>) {
        if self.views.get(view).and_then(|v| v.open) != Some(what) {
            self.leave(view, cx);
        }
        let vs = self.view_mut(view);
        vs.open = Some(what);
        vs.focus_editor = true;
        cx.notify();
    }

    fn back(&mut self, view: &str, cx: &mut Cx<Self>) {
        self.leave(view, cx);
        self.view_mut(view).open = None;
        cx.notify();
    }

    /// ⌘⏎ in the note: save now; in a narrow view, back to the list.
    fn done(&mut self, view: &str, cx: &mut Cx<Self>) {
        if self.views.get(view).is_some_and(|v| v.width >= SPLIT_AT) {
            self.flush(cx);
            cx.notify();
        } else {
            self.back(view, cx);
        }
    }

    /// The user typed in note `id`.
    fn typed(&mut self, id: u64, text: &str, cx: &mut Cx<Self>) {
        let before = self.notes.iter().find(|n| n.id == id).map(|n| note::title(self.text_of(n)));
        let was_pending = self.pending.contains_key(&id);
        self.pending.insert(id, text.to_string());
        self.edited(cx);
        // Render again only when what the bars and the list show changed,
        // not on every key.
        if !was_pending || before != Some(note::title(text)) {
            cx.notify();
        }
    }

    /// The view stops showing its note: save it, and drop it when it has
    /// no text and no other view shows it.
    fn leave(&mut self, view: &str, cx: &mut Cx<Self>) {
        let Some(vs) = self.views.get_mut(view) else { return };
        vs.confirm_delete = false;
        let Some(Open::Note(id)) = vs.open else { return };
        self.flush(cx);
        let empty = self.notes.iter().find(|n| n.id == id).is_some_and(|n| n.text.trim().is_empty());
        let elsewhere = self.views.iter().any(|(v, s)| v != view && s.open == Some(Open::Note(id)));
        if empty && !elsewhere && self.pending.get(&id).is_none_or(|t| t.trim().is_empty()) {
            if let Err(e) = storage::delete(&note::key(id)) {
                log::warn(format!("the empty note {id} wasn't removed: {e}"));
            }
            self.notes.retain(|n| n.id != id);
            self.pending.remove(&id);
        }
    }

    fn delete(&mut self, view: &str, cx: &mut Cx<Self>) {
        let Some(Open::Note(id)) = self.views.get(view).and_then(|v| v.open) else { return };
        if let Err(e) = storage::delete(&note::key(id)) {
            log::warn(format!("note {id} couldn't be deleted: {e}"));
            ui::toast(format!("The note couldn't be deleted: {e}"), Tone::Bad);
            return;
        }
        self.notes.retain(|n| n.id != id);
        self.pending.remove(&id);
        for vs in self.views.values_mut() {
            if vs.open == Some(Open::Note(id)) {
                vs.open = None;
                vs.confirm_delete = false;
            }
        }
        cx.notify();
    }

    fn keep_draft(&mut self, view: &str, cx: &mut Cx<Self>) {
        let Some(text) = self.view_mut(view).draft.take() else { return };
        let id = self.next_id;
        self.next_id += 1;
        self.notes.insert(0, Note { id, text: String::new(), updated: now_ms() });
        self.pending.insert(id, text);
        self.flush(cx);
        self.view_mut(view).open = Some(Open::Note(id));
        cx.notify();
    }

    fn discard_draft(&mut self, view: &str, cx: &mut Cx<Self>) {
        let vs = self.view_mut(view);
        vs.draft = None;
        vs.open = None;
        cx.notify();
    }
}

// ---- Loading and saving ---------------------------------------------------

impl Notes {
    /// Read the next batch of notes from storage; the rest a second later.
    fn load_some(&mut self, cx: &mut Cx<Self>) {
        let mut left = Vec::new();
        let batch: Vec<u64> = self.to_load.drain(..self.to_load.len().min(LOAD_BATCH)).collect();
        for (i, id) in batch.iter().copied().enumerate() {
            if self.has(id) {
                continue;
            }
            match storage::get_json::<Stored>(&note::key(id)) {
                Ok(Some(s)) => self.notes.push(Note { id, text: s.text, updated: s.updated }),
                Ok(None) => {}
                Err(HostError::Busy) => {
                    left.extend_from_slice(&batch[i..]);
                    break;
                }
                Err(e) => log::warn(format!("note {id} couldn't be read: {e}")),
            }
        }
        left.append(&mut self.to_load);
        self.to_load = left;
        // Edits restored for notes not read yet, or never saved.
        let missing: Vec<u64> = self.pending.keys().copied().filter(|id| !self.has(*id) && !self.to_load.contains(id)).collect();
        for id in missing {
            self.notes.push(Note { id, text: String::new(), updated: now_ms() });
        }
        self.sort();
        if !self.to_load.is_empty() {
            cx.after(Duration::from_secs(1), |this: &mut Notes, cx| {
                this.load_some(cx);
                cx.notify();
            });
        }
    }

    fn sort(&mut self) {
        self.notes.sort_by(|a, b| b.updated.cmp(&a.updated).then(b.id.cmp(&a.id)));
    }

    /// Note that the user typed: save once typing has stopped for
    /// [`SAVE_AFTER`]. One timer, moved on when it fires early, rather
    /// than a new one per key.
    fn edited(&mut self, cx: &mut Cx<Self>) {
        self.last_edit = Instant::now();
        if self.saver.is_none() {
            self.saver = cx.after(SAVE_AFTER, |this: &mut Notes, cx| this.saver_fired(cx));
        }
    }

    fn saver_fired(&mut self, cx: &mut Cx<Self>) {
        self.saver = None;
        let since = self.last_edit.elapsed();
        if since < SAVE_AFTER {
            let wait = (SAVE_AFTER - since).max(Duration::from_millis(100));
            self.saver = cx.after(wait, |this: &mut Notes, cx| this.saver_fired(cx));
        } else {
            self.flush(cx);
            cx.notify();
        }
    }

    /// Save every unsaved edit now. A failed save stays unsaved and is
    /// tried again a second later; the user is told once.
    fn flush(&mut self, cx: &mut Cx<Self>) {
        if let Some(t) = self.saver.take() {
            cx.cancel(t);
        }
        let now = now_ms();
        for (id, text) in std::mem::take(&mut self.pending) {
            match storage::set_json(&note::key(id), &Stored { text: text.clone(), updated: now }) {
                Ok(()) => {
                    self.save_failed = false;
                    match self.notes.iter_mut().find(|n| n.id == id) {
                        Some(n) => (n.text, n.updated) = (text, now),
                        None => self.notes.push(Note { id, text, updated: now }),
                    }
                }
                Err(e) => {
                    log::warn(format!("note {id} wasn't saved: {e}"));
                    if !self.save_failed {
                        ui::toast(format!("Notes couldn't save: {e}"), Tone::Bad);
                        self.save_failed = true;
                    }
                    self.pending.insert(id, text);
                }
            }
        }
        self.sort();
        if !self.pending.is_empty() {
            self.edited(cx);
        }
    }
}

pitwall_extension::export!(Notes);
