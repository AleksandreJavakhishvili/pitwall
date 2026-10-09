//! ⇧⌘F: search in the agent's folder (ripgrep, else git grep, on its
//! machine), a port of `SearchPane.tsx`. Searches as you type (300 ms
//! debounce), ↵ runs it again, a newer search cancels the older one (the
//! core does, per agent), leaving the pane cancels. Results are grouped by
//! file (collapsible); a click opens the file at the match.

use crate::kit::Ellipsis as _;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::kit::HoverText;
use gpui::{
    actions, div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, HighlightStyle, KeyBinding, SharedString, StyledText, Subscription,
    Task, Window,
};

use pitwall_core::explorer::{SearchQuery, SearchResult};

use crate::kit::file_icons::file_icon;
use crate::kit::{InputEvent, TextInput};
use super::logic::{
    glob_list, group_matches, match_byte_ranges, match_target, search_summary, split_path,
    MatchGroup,
};
use super::source::{Source, CANCELLED};
use crate::kit::{checkbox, icon_btn, tooltip_view, MONO_FONT};
use crate::theme::{self, RADIUS, RADIUS_SM};

actions!(explorer_search, [Rerun, Leave]);

pub const CONTEXT: &str = "ExplorerSearch";
const DEBOUNCE: Duration = Duration::from_millis(300);
const ROW_H: f32 = 22.;

pub fn bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", Rerun, Some(CONTEXT)),
        KeyBinding::new("escape", Leave, Some(CONTEXT)),
    ]
}

/// Esc in the search fields: focus leaves them (a second Esc leaves the
/// viewer).
pub struct Blurred;

/// Open `path` at `line` with `char` columns `from..to` selected.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenHit {
    pub path: String,
    pub line: u32,
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FormKey {
    query: String,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    include: String,
    exclude: String,
    default_excludes: bool,
}

enum Row {
    File(usize),
    Match(usize, usize),
}

pub struct SearchPane {
    source: Arc<dyn Source>,
    agent_id: String,
    query: Entity<TextInput>,
    include: Entity<TextInput>,
    exclude: Entity<TextInput>,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    default_excludes: bool,
    details: bool,
    scroll: gpui::UniformListScrollHandle,
    result: Option<SearchResult>,
    groups: Rc<Vec<MatchGroup>>,
    rows: Rc<Vec<Row>>,
    collapsed: HashSet<String>,
    error: Option<String>,
    busy: bool,
    /// What the shown result (or running search) is for.
    last: Option<FormKey>,
    seq: u64,
    running: Option<Task<()>>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<OpenHit> for SearchPane {}
impl EventEmitter<Blurred> for SearchPane {}

impl Focusable for SearchPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SearchPane {
    pub fn new(source: Arc<dyn Source>, agent_id: String, cx: &mut Context<Self>) -> SearchPane {
        let query = cx.new(|cx| TextInput::new(cx, "", "Search").bare());
        let include = cx.new(|cx| TextInput::new(cx, "", "e.g. *.ts, src/**").bare());
        let exclude = cx.new(|cx| TextInput::new(cx, "", "e.g. *.test.ts").bare());
        let subs = [&query, &include, &exclude]
            .into_iter()
            .map(|i| {
                cx.subscribe(i, |this, _, e: &InputEvent, cx| {
                    if *e == InputEvent::Changed {
                        this.changed(false, cx)
                    }
                })
            })
            .collect();
        SearchPane {
            source,
            agent_id,
            query,
            include,
            exclude,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            default_excludes: true,
            details: false,
            scroll: gpui::UniformListScrollHandle::new(),
            result: None,
            groups: Rc::default(),
            rows: Rc::default(),
            collapsed: HashSet::new(),
            error: None,
            busy: false,
            last: None,
            seq: 0,
            running: None,
            focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    /// Focus the query with its text selected (⇧⌘F, "Search" picked).
    pub fn focus_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |q, cx| q.focus_all(window, cx));
    }

    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.query
            .update(cx, |q, cx| q.replace_text(text, cx));
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    fn key(&self, cx: &App) -> FormKey {
        FormKey {
            query: self.query.read(cx).text().to_string(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            include: self.include.read(cx).text().to_string(),
            exclude: self.exclude.read(cx).text().to_string(),
            default_excludes: self.default_excludes,
        }
    }

    /// The form changed (or ↵: `again`): search after the debounce.
    fn changed(&mut self, again: bool, cx: &mut Context<Self>) {
        let key = self.key(cx);
        if !again && self.last.as_ref() == Some(&key) {
            return;
        }
        self.last = Some(key.clone());
        self.seq += 1;
        let n = self.seq;
        if key.query.is_empty() {
            self.running = None;
            self.result = None;
            self.error = None;
            self.busy = false;
            self.set_groups();
            self.cancel(cx);
            cx.notify();
            return;
        }
        let q = SearchQuery {
            query: key.query.clone(),
            regex: key.regex,
            case_sensitive: key.case_sensitive,
            whole_word: key.whole_word,
            include: glob_list(&key.include),
            exclude: glob_list(&key.exclude),
            max_results: None,
            hidden: None,
            default_excludes: Some(key.default_excludes),
        };
        let (source, agent) = (self.source.clone(), self.agent_id.clone());
        let delay = if again { Duration::ZERO } else { DEBOUNCE };
        self.running = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = true;
                cx.notify();
            });
            let res = cx
                .background_executor()
                .spawn(async move { source.search(&agent, &q) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if n != this.seq {
                    return;
                }
                this.busy = false;
                match res {
                    Ok(r) => {
                        this.result = Some(r);
                        this.error = None;
                        this.collapsed.clear();
                    }
                    Err(e) if e == CANCELLED => {}
                    Err(e) => {
                        this.error = Some(e);
                        this.result = None;
                    }
                }
                this.set_groups();
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn rerun(&mut self, _: &Rerun, _: &mut Window, cx: &mut Context<Self>) {
        self.changed(true, cx);
    }

    fn leave(&mut self, _: &Leave, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(Blurred);
    }

    /// Stop the running search (the Stop button, leaving the pane).
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.running.is_some() {
            self.seq += 1;
            self.running = None;
            self.busy = false;
            // The form's answer is unknown now: ↵ or a change searches again.
            self.last = None;
            cx.notify();
        }
        let (source, agent) = (self.source.clone(), self.agent_id.clone());
        cx.background_executor()
            .spawn(async move { source.cancel_search(&agent) })
            .detach();
    }

    fn set_groups(&mut self) {
        self.groups = Rc::new(group_matches(
            self.result.as_ref().map(|r| &r.matches[..]).unwrap_or(&[]),
        ));
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            rows.push(Row::File(gi));
            if !self.collapsed.contains(&g.path) {
                rows.extend((0..g.matches.len()).map(|mi| Row::Match(gi, mi)));
            }
        }
        self.rows = Rc::new(rows);
    }

    fn toggle_group(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.rebuild_rows();
        cx.notify();
    }

    fn option(
        &self,
        id: &'static str,
        label: &'static str,
        text: &'static str,
        on: bool,
        set: fn(&mut SearchPane),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let hover = t.surface_3;
        let text_c = t.text;
        div()
            .id(id)
            .w(px(22.))
            .h(px(20.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .font_family(MONO_FONT)
            .text_size(px(11.))
            .text_color(if on { t.text } else { t.text_3 })
            .when(on, |d| d.bg(t.surface_3).border_1().border_color(t.text_4))
            .when(text == "ab", |d| d.underline())
            .cursor_pointer()
            .hover_text(text_c, move |s| s.bg(hover))
            .tooltip(move |_, cx| tooltip_view(label, cx))
            .child(text)
            .on_click(cx.listener(move |this, _, _, cx| {
                set(this);
                this.changed(false, cx);
            }))
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = theme::theme(cx).clone();
        let hover = t.surface_2;
        match self.rows[ix] {
            Row::File(gi) => {
                let g = &self.groups[gi];
                let open = !self.collapsed.contains(&g.path);
                let (dir, base) = split_path(&g.path);
                let path = g.path.clone();
                let title: SharedString = g.path.clone().into();
                div()
                    .id(ix)
                    .w_full()
                    .h(px(ROW_H))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(RADIUS_SM)
                    .cursor_pointer()
                    .hover_probed(move |s| s.bg(hover))
                    .tooltip(move |_, cx| tooltip_view(title.clone(), cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(&path, cx)))
                    .child(crate::kit::chevron("chev", open, 10., t.text_4))
                    .child(file_icon(&g.path, 16.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ellipsis()
                            .child(
                                div()
                                    .flex()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .flex_none()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(t.text)
                                            .child(base.to_string()),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_color(t.text_3)
                                            .child(crate::kit::one_line(dir.to_string())),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(18.))
                            .px(px(6.))
                            .rounded(px(9.))
                            .bg(t.surface_3)
                            .text_size(px(10.5))
                            .text_color(t.text_2)
                            .flex()
                            .justify_center()
                            .child(g.matches.len().to_string()),
                    )
                    .into_any_element()
            }
            Row::Match(gi, mi) => {
                let g = &self.groups[gi];
                let m = &g.matches[mi];
                let (line, from, to) = match_target(m);
                let path = g.path.clone();
                let title: SharedString = format!("{}:{}", g.path, m.line).into();
                let prefix = if m.text_offset > 0 { "…" } else { "" };
                let shift = prefix.len();
                let text = format!("{prefix}{}", m.text.replace('\t', "  "));
                let ranges = if m.text.contains('\t') {
                    // Tabs widened: map ranges on the widened text.
                    let widened =
                        |b: usize| shift + m.text[..b].len() + m.text[..b].matches('\t').count();
                    match_byte_ranges(m)
                        .into_iter()
                        .map(|r| widened(r.start)..widened(r.end))
                        .collect::<Vec<_>>()
                } else {
                    match_byte_ranges(m)
                        .into_iter()
                        .map(|r| r.start + shift..r.end + shift)
                        .collect()
                };
                let styled = StyledText::new(text).with_highlights(ranges.into_iter().map(|r| {
                    (
                        r,
                        HighlightStyle {
                            color: Some(t.text),
                            font_weight: Some(FontWeight::BOLD),
                            background_color: Some(t.amber_soft),
                            ..Default::default()
                        },
                    )
                }));
                div()
                    .id(ix)
                    .w_full()
                    .h(px(ROW_H))
                    .pl(px(28.))
                    .pr(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(RADIUS_SM)
                    .cursor_pointer()
                    .hover_probed(move |s| s.bg(hover))
                    .tooltip(move |_, cx| tooltip_view(title.clone(), cx))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(OpenHit {
                            path: path.clone(),
                            line,
                            from,
                            to,
                        })
                    }))
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(26.))
                            .flex()
                            .justify_end()
                            .text_size(px(10.5))
                            .text_color(t.text_4)
                            .child(m.line.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ellipsis()
                            .text_color(t.text_2)
                            .child(styled),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for SearchPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let total = self.result.as_ref().map(|r| r.matches.len()).unwrap_or(0);
        let summary: Option<SharedString> = if self.busy {
            Some("Searching…".into())
        } else if self.error.is_some() {
            None
        } else {
            self.result
                .as_ref()
                .map(|r| search_summary(total, r.files, r.truncated).into())
        };
        let field = |label: &'static str, input: &Entity<TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().text_size(px(10.5)).text_color(t.text_3).child(label))
                .child(
                    div()
                        .h(px(26.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(RADIUS)
                        .border_1()
                        .border_color(t.line_strong)
                        .bg(t.bg)
                        .text_size(px(12.))
                        .child(input.clone()),
                )
        };
        let count = self.rows.len();
        let list = uniform_list(
            "ex-results",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .size_full()
        .px(px(4.))
        .pb(px(12.));
        let details = self.details;
        div()
            .id("ex-search")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::rerun))
            .on_action(cx.listener(Self::leave))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex_none()
                    .pl(px(6.))
                    .pr(px(10.))
                    .pb(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .child(
                                icon_btn("ex-details", if details { "chevron-down" } else { "chevron" }, "", true, &t)
.on_click(cx.listener(|this, _, _, cx| {
                                        this.details = !this.details;
                                        cx.notify();
                                    }))
                                .tooltip(|_, cx| tooltip_view("Files to include / exclude", cx)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(28.))
                                    .pl(px(8.))
                                    .pr(px(3.))
                                    .flex()
                                    .items_center()
                                    .gap(px(1.))
                                    .rounded(RADIUS)
                                    .border_1()
                                    .border_color(t.line_strong)
                                    .bg(t.bg)
                                    .text_size(px(12.5))
                                    .child(self.query.clone())
                                    .child(self.option(
                                        "ex-case",
                                        "Match case",
                                        "Aa",
                                        self.case_sensitive,
                                        |s| s.case_sensitive = !s.case_sensitive,
                                        cx,
                                    ))
                                    .child(self.option(
                                        "ex-word",
                                        "Match whole word",
                                        "ab",
                                        self.whole_word,
                                        |s| s.whole_word = !s.whole_word,
                                        cx,
                                    ))
                                    .child(self.option(
                                        "ex-regex",
                                        "Use regular expression",
                                        ".*",
                                        self.regex,
                                        |s| s.regex = !s.regex,
                                        cx,
                                    )),
                            ),
                    )
                    .when(details, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(7.))
                                .pt(px(8.))
                                .pl(px(26.))
                                .child(field("files to include", &self.include))
                                .child(field("files to exclude", &self.exclude))
                                .child(checkbox("ex-default-excludes", self.default_excludes, "Leave out node_modules and bower_components", &t, cx.listener(|this, _, _, cx| {
                                        this.default_excludes = !this.default_excludes;
                                        this.changed(false, cx);
                                    })))
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(t.text_4)
                                        .child(".gitignore applies; .git is never searched."),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .min_h(px(18.))
                    .px(px(12.))
                    .pb(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .text_color(t.text_3)
                    .children(summary)
                    .when(self.busy, |d| {
                        d.child(
                            div()
                                .id("ex-search-stop")
                                .text_color(t.text_2)
                                .cursor_pointer()
                                .hover_underline()
                                .child("Stop")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        )
                    }),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .px(px(12.))
                        .pb(px(6.))
                        .text_size(px(11.5))
                        .text_color(t.red)
                        .ellipsis()
                        .child(crate::kit::one_line(e.lines().next().unwrap_or_default().to_string())),
                )
            })
            .child(
                crate::kit::vscroll_list_fill("ex-results-bar", &self.scroll, &t, list)
                    .flex_1()
                    .min_h_0()
                    .font_family(MONO_FONT)
                    .text_size(px(12.)),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use gpui::{TestAppContext, VisualTestContext};

    use super::super::source::fake::FakeSource;
    use super::*;

    #[gpui::test]
    async fn searches_after_typing_and_cancels_when_cleared(cx: &mut TestAppContext) {
        let src = FakeSource::new(&[
            ("a.rs", "let x = 1;\nfn გამარჯობა() {}\n"),
            ("b.rs", "nothing"),
        ]);
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
        let (pane, cx) = cx.add_window_view(|_, cx| SearchPane::new(src.clone(), "a1".into(), cx));
        let cx: &mut VisualTestContext = cx;
        pane.update(cx, |p, cx| {
            p.query.update(cx, |q, cx| q.replace_text("გამარ", cx))
        });
        cx.run_until_parked();
        cx.executor().advance_clock(DEBOUNCE);
        cx.run_until_parked();
        pane.read_with(cx, |p, _| {
            let r = p.result.as_ref().expect("a result");
            assert_eq!(r.matches.len(), 1);
            assert_eq!(match_target(&r.matches[0]), (2, 3, 8));
            assert_eq!(p.groups.len(), 1);
            assert_eq!(p.rows.len(), 2);
            assert!(!p.busy);
        });
        pane.update(cx, |p, cx| p.toggle_group("a.rs", cx));
        pane.read_with(cx, |p, _| assert_eq!(p.rows.len(), 1));
        pane.update(cx, |p, cx| p.query.update(cx, |q, cx| q.replace_text("", cx)));
        cx.run_until_parked();
        pane.read_with(cx, |p, _| assert!(p.result.is_none()));
        assert!(src.cancels.load(Ordering::SeqCst) >= 1);
    }
}
