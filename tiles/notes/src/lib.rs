//! Notes: private plain-text notes beside the agents. This is the preview
//! on SDK phase 1 (step 0 of docs/spec/2026-11-27-notes-tile/plan.md):
//! notes in the tile's own storage, edited in Pitwall's multi-line text
//! field, saved about a second after typing stops, when the note's field
//! loses focus and when the tile is hidden. An agent (or the user) can open
//! the tile with a draft note (input.schema.json, `notes__open` for
//! agents); it stays unsaved until the user keeps it. Once the user types
//! in an agent's draft, Pitwall is told (`tools::user_edited`), so the
//! agent can't replace what the user wrote.

mod note;

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use note::{Note, Stored};
use pitwall_extension::prelude::*;
use serde::{Deserialize, Serialize};

/// Saved this long after typing stops (frd.md FR-6).
const SAVE_AFTER: Duration = Duration::from_secs(1);
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
    /// The agent's call that brought the draft, until the user edits it.
    #[serde(default)]
    draft_call: Option<u64>,
    #[serde(skip)]
    confirm_delete: bool,
    /// A row of the list asks whether to delete its note.
    #[serde(skip)]
    confirm_row: Option<u64>,
    #[serde(skip)]
    width: f32,
    #[serde(skip)]
    focus_editor: bool,
    #[serde(skip)]
    focus_search: bool,
    /// The note is shown formatted (Preview), not in its editor (FR-11).
    #[serde(default)]
    preview: bool,
    /// The Send picker is open (FR-13).
    #[serde(skip)]
    send: Option<Sending>,
    /// The open note's selection in its field, by note (FR-14, gap G2):
    /// what Send offers first. Byte offsets in the note's text.
    #[serde(skip)]
    selection: Option<(u64, std::ops::Range<usize>)>,
}

/// The Send picker: what goes (the selection, the whole note, or
/// section `n`), and why the last try didn't.
#[derive(Debug, Clone, Default)]
struct Sending {
    section: Option<usize>,
    /// The selected text goes (when there is one).
    selected: bool,
    error: Option<String>,
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
    /// A save failed and the user was told; not told again until one works.
    save_failed: bool,
    /// Pitwall's agents, read when the Send picker opens and when they
    /// change; `Err` without `agents.read`.
    agents: Result<Vec<agents::Agent>, HostError>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Tile for Notes {
    fn new(cx: &mut Cx<Self>) -> Self {
        // Every note in two host calls (the keys, then their values).
        let notes: Vec<Note> = match storage::entries_json::<Stored>(note::PREFIX) {
            Ok(all) => all.into_iter().filter_map(|(k, s)| Some(Note { id: note::id_of(&k)?, text: s.text, updated: s.updated })).collect(),
            Err(e) => {
                log::error(format!("the notes couldn't be read: {e}"));
                Vec::new()
            }
        };
        let next_id = notes.iter().map(|n| n.id + 1).max().unwrap_or(1);
        let mut this = Notes { notes, pending: BTreeMap::new(), views: BTreeMap::new(), restored: BTreeMap::new(), next_id, saver: None, last_edit: Instant::now(), save_failed: false, agents: Ok(Vec::new()) };
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
        // Edits restored for notes never saved.
        let missing: Vec<u64> = this.pending.keys().copied().filter(|id| !this.has(*id)).collect();
        for id in missing {
            this.notes.push(Note { id, text: String::new(), updated: now_ms() });
        }
        this.sort();
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
                vs.draft_call = content.call();
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
            // Hidden: save now (FR-6). Leaving the note's field (to the
            // search, or out of the tile) is its blur, in `editor_pane`.
            Event::Visibility(false) => {
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
            // The picker shows them as they are now.
            Event::AgentsChanged | Event::GrantsChanged(_) if self.views.values().any(|v| v.send.is_some()) => {
                self.agents = agents::list();
                cx.notify();
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
        // ⌘N is Pitwall's (a new agent): N, while no field has focus.
        cx.bind_key("n", new);
        // / goes to the search, where the list shows.
        if split || vs.open.is_none() {
            let v = view.to_string();
            let search = cx.listener(move |this: &mut Notes, _: &KeyPress, cx| {
                this.view_mut(&v).focus_search = true;
                cx.notify();
            });
            cx.bind_key("/", search);
            if vs.focus_search {
                cx.focus("list/head/search");
            }
        }
        self.view_mut(view).focus_search = false;
        // ⌘/ switches Edit and Preview (FR-11), also while typing.
        if let Some(Open::Note(_)) = vs.open {
            let v = view.to_string();
            let flip = cx.listener(move |this: &mut Notes, _: &KeyPress, cx| this.set_preview(&v, !this.views.get(&v).is_some_and(|s| s.preview), cx));
            cx.bind_key("cmd-/", flip);
        }
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
            .child(icon_button(Icon::Plus, "New note").id("new").tooltip("New note (N)").on_click(cx.listener(move |this: &mut Notes, _, cx| this.new_note(&v2, cx))));

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
                    .child(text("Press N or + to write one. Notes stay in this Pitwall and are saved as you type.").text_xs().text_color(Token::Text3)),
            );
        } else if shown.is_empty() && !query.is_empty() {
            rows = rows.child(text(format!("No notes match “{}”", vs.filter.trim())).id("none").px_3().py_3().text_sm().text_color(Token::Text3));
        }
        for n in shown {
            rows = rows.child(self.row(view, vs, n, now, cx));
        }
        v_flex().id("list").min_w(px(0.)).child(head).child(rows)
    }

    /// One note in the list: its title, when it changed, its next line,
    /// and a Delete that shows while the pointer is over the row (it asks
    /// in the row first).
    fn row(&self, view: &str, vs: &ViewState, n: &Note, now: u64, cx: &mut Cx<Self>) -> AnyElement {
        let text_now = self.text_of(n);
        let title = note::title(text_now);
        let id = n.id;
        if vs.confirm_row == Some(id) {
            let (v1, v2) = (view.to_string(), view.to_string());
            return h_flex()
                .id(format!("n{id}"))
                .px_3()
                .py_2()
                .gap_1p5()
                .bg(Token::BadSoft)
                .child(text(format!("Delete “{}”?", title.unwrap_or_else(|| "New note".into()))).text_sm().truncate().flex_1())
                .child(button("Cancel").small().id("cancel").on_click(cx.listener(move |this: &mut Notes, _, cx| {
                    this.view_mut(&v1).confirm_row = None;
                    cx.notify();
                })))
                .child(button("Delete").small().danger().id("delete").on_click(cx.listener(move |this: &mut Notes, _, cx| this.delete(&v2, id, cx))))
                .into_any();
        }
        let selected = vs.open == Some(Open::Note(id));
        let (v, v2) = (view.to_string(), view.to_string());
        let when = if self.pending.contains_key(&id) { "editing".to_string() } else { note::ago(now, n.updated) };
        h_flex()
            .id(format!("n{id}"))
            .group("row")
            .pl_3()
            .pr_1()
            .py_2()
            .gap_1()
            .cursor_pointer()
            .focusable()
            .hover_bg(Token::Surface2)
            .when(selected, |r| r.bg(Token::Surface2))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text(title.clone().unwrap_or_else(|| "New note".into())).text_sm().font_medium().truncate().flex_1().when(title.is_none(), |t| t.text_color(Token::Text3)))
                            .child(text(when).text_xs().text_color(Token::Text3).flex_none()),
                    )
                    .when_some(note::preview(text_now), |r, p| r.child(text(p).text_xs().text_color(Token::Text3).truncate())),
            )
            .child(icon_button(Icon::Trash, "Delete note").id("delete").visible_on_group_hover("row").on_click(cx.listener(move |this: &mut Notes, _, cx| {
                this.view_mut(&v2).confirm_row = Some(id);
                cx.notify();
            })))
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
                    .child(button("Discard").small().id("discard").on_click(cx.listener(move |this: &mut Notes, _, cx| this.discard_draft(&v1, cx))))
                    .child(button("Keep").small().primary().id("keep").tooltip("Save it as a note (⌘⏎)").on_click(cx.listener(move |this: &mut Notes, _, cx| this.keep_draft(&v2, cx))));
                let field = input("text")
                    .multiline()
                    .value(draft)
                    .placeholder("Title on the first line, then anything")
                    .size_full()
                    .on_change(cx.listener(move |this: &mut Notes, t: &str, cx| {
                        let vs = this.view_mut(&v3);
                        let retitled = vs.draft.as_deref().map(note::title) != Some(note::title(t));
                        vs.draft = Some(t.to_string());
                        // The user's words now: the agent can't replace them.
                        if let Some(call) = vs.draft_call.take() {
                            pitwall_extension::tools::user_edited(call);
                        }
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
                        .child(button("Cancel").small().id("cancel").on_click(cx.listener(move |this: &mut Notes, _, cx| {
                            this.view_mut(&v1).confirm_delete = false;
                            cx.notify();
                        })))
                        .child(button("Delete").small().danger().id("delete").on_click(cx.listener(move |this: &mut Notes, _, cx| this.delete(&v2, id, cx))))
                } else {
                    let (v, vm) = (view.to_string(), view.to_string());
                    let saved = if self.pending.contains_key(&id) { "Editing…" } else { "Saved" };
                    // Edit and Preview (FR-11).
                    let mode = tabs("mode", [tab("edit", "Edit"), tab("preview", "Preview")])
                        .selected(if vs.preview { "preview" } else { "edit" })
                        .tooltip("Edit or Preview (⌘/)")
                        .on_select(cx.listener(move |this: &mut Notes, key: &str, cx| this.set_preview(&vm, key == "preview", cx)));
                    bar()
                        .when(!split, |h| h.child(back(cx)))
                        .child(text(title).text_sm().font_medium().truncate().flex_1())
                        .child(text(saved).text_xs().text_color(Token::Text3).flex_none())
                        .child(mode)
                        .child(self.send_button(view, vs, &text_now, cx))
                        .child(icon_button(Icon::Trash, "Delete note").id("trash").on_click(cx.listener(move |this: &mut Notes, _, cx| {
                            this.view_mut(&v).confirm_delete = true;
                            cx.notify();
                        })))
                };
                if vs.preview {
                    // Pitwall's inert Markdown (FR-12); a task's box ticks
                    // it in the note's text, links open in the browser.
                    let md = markdown(text_now.clone()).on_toggle(cx.listener(move |this: &mut Notes, e: &Toggled, cx| {
                        let text = this.notes.iter().find(|n| n.id == id).map(|n| this.text_of(n).to_string()).unwrap_or_default();
                        if let Some(new) = note::tick(&text, e.index, e.on) {
                            this.typed(id, &new, cx);
                            this.flush(cx);
                            cx.notify();
                        }
                    }));
                    return pane.child(head).child(v_flex().id(format!("p{id}")).flex_1().min_h(px(0.)).p_3().overflow_y_scroll().child(md));
                }
                let (v, v1, v2) = (view.to_string(), view.to_string(), view.to_string());
                let field = input("text")
                    .multiline()
                    .value(text_now)
                    .placeholder("Title on the first line, then anything")
                    .size_full()
                    .on_change(cx.listener(move |this: &mut Notes, e: &InputChange, cx| {
                        this.view_mut(&v1).selection = Some((id, e.selection.clone()));
                        this.typed(id, &e.value, cx);
                    }))
                    // Kept for Send (FR-14).
                    .on_select(cx.listener(move |this: &mut Notes, s: &pitwall_extension::events::Selection, cx| {
                        // Send's label says whether a selection goes.
                        let was = this.views.get(&v2).and_then(|vs| vs.selection.as_ref()).is_some_and(|(_, r)| !r.is_empty());
                        this.view_mut(&v2).selection = Some((id, s.range.clone()));
                        if was != !s.range.is_empty() {
                            cx.notify();
                        }
                    }))
                    .on_submit(cx.listener(move |this: &mut Notes, _, cx| this.done(&v, cx)))
                    // Leaving the note's field saves it (FR-6).
                    .on_blur(cx.listener(|this: &mut Notes, _: &Blur, cx| {
                        if !this.pending.is_empty() {
                            this.flush(cx);
                            cx.notify();
                        }
                    }));
                pane.child(head).child(v_flex().id(format!("n{id}")).flex_1().min_h(px(0.)).p_2().child(field))
            }
        }
    }
}

// ---- Send to an agent ---------------------------------------------------------

/// The text selected in the open note's field (trimmed; none when the
/// selection is empty or blank), in Edit only.
fn selected_text(vs: &ViewState, text: &str) -> Option<String> {
    if vs.preview {
        return None;
    }
    let (id, range) = vs.selection.as_ref()?;
    if vs.open != Some(Open::Note(*id)) {
        return None;
    }
    let s = text.get(range.clone())?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// "5 210" for 5210, as the frd's messages write counts.
fn count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn state_word(s: agents::State) -> &'static str {
    match s {
        agents::State::Working => "working",
        agents::State::Blocked => "needs you",
        agents::State::Idle => "idle",
        agents::State::Done => "done",
        agents::State::Stopped => "stopped",
        agents::State::Unknown => "",
    }
}

impl Notes {
    /// The note's Send button and, while open, its picker (FR-13–FR-16):
    /// what to send (the whole note or one section), then the agents
    /// that can take it; a click puts it in that agent's Next up.
    fn send_button(&self, view: &str, vs: &ViewState, text_now: &str, cx: &mut Cx<Self>) -> Div {
        let v = view.to_string();
        // The selected text of this note, in Edit (FR-14, gap G2).
        let selected = selected_text(vs, text_now);
        let has_selection = selected.is_some();
        let send = icon_button(Icon::Send, if has_selection { "Send the selection to an agent" } else { "Send to an agent" }).id("send").disabled(text_now.trim().is_empty()).on_click(cx.listener(move |this: &mut Notes, _, cx| {
            let open = this.views.get(&v).is_some_and(|s| s.send.is_some());
            if open {
                this.view_mut(&v).send = None;
            } else {
                this.agents = agents::list();
                this.view_mut(&v).send = Some(Sending { selected: has_selection, ..Sending::default() });
            }
            cx.notify();
        }));
        let wrap = div().id("send-at").child(send);
        let Some(sending) = &vs.send else { return wrap };
        let sections = note::sections(text_now);
        let section = sending.section.filter(|n| *n < sections.len());
        let selected = selected.filter(|_| sending.selected);
        let what = match (&selected, section) {
            (Some(s), _) => s.clone(),
            (None, Some(n)) => sections[n].1.clone(),
            (None, None) => text_now.trim().to_string(),
        };
        let chars = what.chars().count();
        let mut body = v_flex().id("picker").gap_1().p_1().child(text("Send to an agent").text_sm().font_semibold().px_2().pt_1());
        if !sections.is_empty() || has_selection {
            let v = view.to_string();
            let mut choices = Vec::new();
            if has_selection {
                choices.push(choice("sel", "The selection"));
            }
            choices.push(choice("all", "The whole note"));
            choices.extend(sections.iter().enumerate().map(|(n, (name, _))| choice(format!("s{n}"), format!("§ {name}"))));
            let chosen = match (selected.is_some(), section) {
                (true, _) => "sel".to_string(),
                (false, Some(n)) => format!("s{n}"),
                (false, None) => "all".to_string(),
            };
            body = body.child(
                div().px_2().child(
                    select("what", choices)
                        .selected(chosen)
                        .on_select(cx.listener(move |this: &mut Notes, key: &str, cx| {
                            if let Some(s) = this.view_mut(&v).send.as_mut() {
                                s.selected = key == "sel";
                                s.section = key.strip_prefix('s').and_then(|n| n.parse().ok());
                                s.error = None;
                            }
                            cx.notify();
                        })),
                ),
            );
        }
        if let Some(s) = &selected {
            // What goes, so the user sees it is the selection.
            let shown: String = s.chars().take(120).collect();
            let more = if s.chars().count() > 120 { "…" } else { "" };
            body = body.child(text(format!("“{}{more}”", shown.replace('\n', " "))).id("selected").text_xs().text_color(Token::Text2).px_2().truncate());
        }
        let too_long = chars > agents::MAX_TEXT;
        if too_long {
            let hint = if sections.is_empty() { "Make it shorter, or split it under headings." } else { "Pick a section." };
            body = body.child(text(format!("Too long to send: {} of {} characters. {hint}", count(chars), count(agents::MAX_TEXT))).id("too-long").text_xs().text_color(Token::Warn).px_2());
        }
        if let Some(e) = &sending.error {
            body = body.child(text(e.clone()).id("error").text_xs().text_color(Token::Bad).px_2());
        }
        match &self.agents {
            Err(HostError::Denied) => {
                body = body.child(text("Notes can't see your agents yet. Allow “See your agents” and “Put text in an agent's Next up” for Notes in Settings → Extensions.").id("denied").text_xs().text_color(Token::Text2).px_2()).child(
                    div().px_2().pb_1().child(button("Open settings").small().id("settings").on_click(cx.listener(|_: &mut Notes, _, _| settings::open()))),
                );
            }
            Err(e) => body = body.child(text(format!("The agents couldn't be read: {e}")).text_xs().text_color(Token::Bad).px_2()),
            Ok(list) => {
                let list: Vec<&agents::Agent> = list.iter().filter(|a| a.kind != "shell").collect();
                if list.is_empty() {
                    body = body.child(text("No agents to send to. Start one in Pitwall first.").id("none").text_xs().text_color(Token::Text3).px_2().pb_1());
                }
                let mut rows = v_flex().id("agents").max_h(px(280.)).overflow_y_scroll();
                for a in list {
                    let (v, id, name) = (view.to_string(), a.id.clone(), a.name.clone());
                    let detail = [a.kind_name.as_str(), state_word(a.state)].iter().filter(|w| !w.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                    let detail = match &a.branch {
                        Some(b) => format!("{detail} · {b}"),
                        None => detail,
                    };
                    let what = what.clone();
                    rows = rows.child(
                        h_flex()
                            .id(format!("a-{}", a.id))
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .when(!too_long, |r| r.cursor_pointer().focusable().hover_bg(Token::Surface2))
                            .when(too_long, |r| r.opacity(0.5))
                            .child(avatar(a.name.clone()).small())
                            .child(v_flex().flex_1().min_w(px(0.)).child(text(a.name.clone()).text_sm().truncate()).child(text(detail).text_xs().text_color(Token::Text3).truncate()))
                            .when(a.queued > 0, |r| r.child(badge(format!("{} queued", a.queued))))
                            .when(!too_long, |r| r.on_click(cx.listener(move |this: &mut Notes, _, cx| this.send_to(&v, &id, &name, &what, cx)))),
                    );
                }
                body = body.child(rows);
            }
        }
        let v = view.to_string();
        wrap.child(popover().id("send-menu").width(320.).on_dismiss(cx.listener(move |this: &mut Notes, _: &Dismissed, cx| {
            this.view_mut(&v).send = None;
            cx.notify();
        })).child(body))
    }

    /// Put `text` in agent `id`'s Next up, waiting for the user's Send
    /// (FR-15); Pitwall's refusals stay in the picker (FR-16).
    fn send_to(&mut self, view: &str, id: &str, name: &str, text: &str, cx: &mut Cx<Self>) {
        match agents::queue_add(id, text) {
            Ok(()) => {
                self.view_mut(view).send = None;
                ui::toast(format!("In {name}'s Next up, waiting for your Send"), Tone::Good);
            }
            Err(e) => {
                let why = match e {
                    HostError::Busy => format!("{name} already has Notes' items waiting, or one was just sent: send or remove those first, or try again in a few seconds."),
                    HostError::TooLarge => format!("Too long to send: {} of {} characters.", count(text.chars().count()), count(agents::MAX_TEXT)),
                    HostError::Denied => "Notes may not put text in an agent's Next up: allow it for Notes in Settings → Extensions.".to_string(),
                    HostError::Invalid(_) => format!("{name} isn't there any more."),
                    e => format!("It wasn't sent: {e}"),
                };
                if let Some(s) = self.view_mut(view).send.as_mut() {
                    s.error = Some(why);
                }
            }
        }
        cx.notify();
    }

    /// Edit or Preview (FR-11).
    fn set_preview(&mut self, view: &str, on: bool, cx: &mut Cx<Self>) {
        if on && !self.pending.is_empty() {
            self.flush(cx);
        }
        let vs = self.view_mut(view);
        vs.preview = on;
        if !on {
            vs.focus_editor = true;
        }
        cx.notify();
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

    fn delete(&mut self, view: &str, id: u64, cx: &mut Cx<Self>) {
        self.view_mut(view).confirm_row = None;
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
        self.view_mut(view).draft_call = None;
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
        vs.draft_call = None;
        vs.open = None;
        cx.notify();
    }
}

// ---- Saving -----------------------------------------------------------------

impl Notes {
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
