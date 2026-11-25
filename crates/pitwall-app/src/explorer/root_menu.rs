//! The file viewer's root menu: the folder in a files tile's or window's
//! bar, clicked. It lists the worktrees of the repository shown (`git
//! worktree list`: folder name and branch, the one shown ticked; typing
//! filters them), "Open folder…" (a path, `~` and relative paths from the
//! root read, checked to be a folder; or the system's folder picker), "Up
//! to parent folder" and, when the root isn't the agent's folder, "Back to
//! the agent's folder". What is picked roots the viewer there
//! ([`super::show::switch_root`]); refused (unsaved edits), it says why
//! and stays open.
//!
//! Only for folders on this computer: another machine's agent has no menu
//! (its viewer reads its whole folder there).

use std::path::{Path, PathBuf};

use gpui::{
    div, prelude::*, px, App, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, SharedString,
    Subscription, Task, Window,
};

use pitwall_core::exec::{Exec as _, LocalExec};
use pitwall_core::show::Worktree;

use crate::kit::{icon, HoverText, InputEvent, TextInput, MONO_FONT};
use crate::theme::{Theme, RADIUS_SM};

/// What the menu opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// The worktrees, the keyboard in their filter.
    Worktrees,
    /// The "Open folder…" field, the keyboard in it.
    OpenFolder,
}

pub enum RootMenuEvent {
    /// Root the viewer at this folder (a full path on this computer).
    Pick(PathBuf),
    /// Root it at the agent's own folder again.
    AgentFolder,
    Dismissed,
}

pub struct RootMenu {
    /// The folder shown now.
    root: PathBuf,
    /// The agent's folder, when the root isn't it.
    agent_root: Option<PathBuf>,
    filter: Entity<TextInput>,
    path: Entity<TextInput>,
    /// The "Open folder…" field is shown.
    typing: bool,
    /// `None`: still asking git.
    worktrees: Option<Vec<Worktree>>,
    /// Why the last pick didn't happen.
    error: Option<String>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    _load: Task<()>,
    _picker: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<RootMenuEvent> for RootMenu {}

impl Focusable for RootMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The worktrees the menu lists for a viewer rooted at `root`: its
/// repository's, else (a folder that isn't one) those of the repositories
/// directly inside it. Blocking (git).
pub fn worktrees_for(root: &str) -> Vec<Worktree> {
    let x = LocalExec;
    let near = pitwall_core::show::worktrees_near(&x, &[root]);
    if near.is_empty() {
        pitwall_core::show::worktrees_inside(&x, root)
    } else {
        near
    }
}

/// `text` as a folder on this computer: `~` expanded, relative to `root`;
/// links followed. Why not, when it isn't one.
pub fn folder_from(text: &str, root: &Path) -> Result<PathBuf, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type a folder's path.".into());
    }
    let home = LocalExec.home().ok();
    let expanded = pitwall_core::paths::expand_tilde_in(home.as_deref(), text);
    let p = Path::new(&expanded);
    let full = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let real = LocalExec.real_path(&full.to_string_lossy()).map_err(|_| format!("{text}: no such folder"))?;
    if !Path::new(&real).is_dir() {
        return Err(format!("{text} is a file, not a folder"));
    }
    Ok(PathBuf::from(real))
}

impl RootMenu {
    pub fn new(root: PathBuf, agent_root: Option<PathBuf>, start: Start, cx: &mut Context<Self>) -> RootMenu {
        let filter = cx.new(|cx| TextInput::new(cx, "", "Filter worktrees"));
        let path = cx.new(|cx| TextInput::new(cx, "", "~/path/to/folder").mono());
        let subs = vec![
            cx.subscribe(&filter, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Changed => {
                    this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                    cx.notify();
                }
                // ↵: the first one listed.
                InputEvent::Submit => {
                    if let Some(w) = this.listed(cx).first() {
                        let p = PathBuf::from(&w.0);
                        this.pick(p, cx);
                    }
                }
                InputEvent::Cancel => cx.emit(RootMenuEvent::Dismissed),
                _ => {}
            }),
            cx.subscribe(&path, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Changed => {
                    this.error = None;
                    cx.notify();
                }
                InputEvent::Submit => this.open_typed(cx),
                InputEvent::Cancel => cx.emit(RootMenuEvent::Dismissed),
                _ => {}
            }),
        ];
        let at = root.to_string_lossy().into_owned();
        let load = cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { worktrees_for(&at) }).await;
            let _ = this.update(cx, |m, cx| {
                m.worktrees = Some(list);
                cx.notify();
            });
        });
        RootMenu {
            root,
            agent_root,
            filter,
            path,
            typing: start == Start::OpenFolder,
            worktrees: None,
            error: None,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            _load: load,
            _picker: None,
            _subs: subs,
        }
    }

    /// The field it opened on (the keyboard goes there).
    pub fn field(&self, cx: &App) -> FocusHandle {
        if self.typing {
            self.path.focus_handle(cx)
        } else {
            self.filter.focus_handle(cx)
        }
    }

    /// A pick was refused: why, shown until the next one.
    pub fn set_error(&mut self, e: String, cx: &mut Context<Self>) {
        self.error = Some(e);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The worktrees listed (git answered).
    #[cfg(test)]
    pub(crate) fn loaded(&self) -> Option<usize> {
        self.worktrees.as_ref().map(Vec::len)
    }

    /// The worktrees the filter lets through.
    fn listed(&self, cx: &App) -> Vec<Worktree> {
        let q = self.filter.read(cx).text().trim().to_lowercase();
        self.worktrees
            .iter()
            .flatten()
            .filter(|(p, name, branch)| {
                q.is_empty() || name.to_lowercase().contains(&q) || branch.as_deref().is_some_and(|b| b.to_lowercase().contains(&q)) || p.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }

    fn pick(&mut self, p: PathBuf, cx: &mut Context<Self>) {
        self.error = None;
        cx.emit(RootMenuEvent::Pick(p));
    }

    fn open_typed(&mut self, cx: &mut Context<Self>) {
        let text = self.path.read(cx).text().to_string();
        match folder_from(&text, &self.root) {
            Ok(p) => self.pick(p, cx),
            Err(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }

    fn show_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.typing = true;
        if self.path.read(cx).text().is_empty() {
            let shown = pitwall_core::paths::tildify(&self.root.to_string_lossy());
            self.path.update(cx, |p, cx| {
                p.set_text(format!("{shown}/"), cx);
            });
        }
        let f = self.path.focus_handle(cx);
        window.focus(&f, cx);
        cx.notify();
    }

    /// The system's folder picker; a folder picked roots the viewer there.
    fn choose(&mut self, cx: &mut Context<Self>) {
        let picked = crate::platform::files::pick_folder(cx, "Open as the root");
        self._picker = Some(cx.spawn(async move |this, cx| {
            let Some(p) = picked.await else { return };
            let _ = this.update(cx, |m, cx| match folder_from(&p.to_string_lossy(), &m.root) {
                Ok(p) => m.pick(p, cx),
                Err(e) => m.set_error(e, cx),
            });
        }));
    }

    fn row(id: impl Into<SharedString>, glyph: &str, text: impl Into<SharedString>, t: &Theme) -> gpui::Stateful<gpui::Div> {
        let id: SharedString = id.into();
        let sel = id.clone();
        crate::kit::menu::menu_item(id, glyph, text, None, false, t).debug_selector(move || sel.to_string())
    }
}

impl Render for RootMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).float();
        let listed = self.listed(cx);
        let total = self.worktrees.as_ref().map(Vec::len);
        let root = self.root.to_string_lossy().into_owned();
        let section = |text: &'static str| div().px(px(8.)).pt(px(6.)).pb(px(2.)).text_size(px(11.)).text_color(t.text_3).child(text);
        let rows = listed.into_iter().enumerate().map(|(i, (path, name, branch))| {
            let on = path == root;
            let p = PathBuf::from(&path);
            div()
                .id(("ex-root-wt", i))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(5.))
                .rounded(RADIUS_SM)
                .cursor_pointer()
                .hover_probed(|s| s.bg(t.surface_3))
                // (After the hover, which marks the row in tests.)
                .debug_selector(move || format!("ex-root-wt-{i}"))
                .tooltip({
                    let tip: SharedString = path.clone().into();
                    move |_, cx| crate::kit::tooltip_view(tip.clone(), cx)
                })
                .child(div().w(px(16.)).flex().when(on, |d| d.child(icon("check", 13., t.text_2))))
                .child(div().flex_none().text_size(px(13.)).text_color(t.text).child(crate::kit::one_line(name)))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .font_family(MONO_FONT)
                        .text_size(px(11.5))
                        .text_color(t.text_3)
                        .child(icon("branch", 11., t.text_4))
                        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(crate::kit::one_line(branch.unwrap_or_else(|| "detached".into())))),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.pick(p.clone(), cx)))
        });
        let parent = self.root.parent().map(Path::to_path_buf);
        let back = self.agent_root.clone();
        crate::kit::menu::menu_panel(&t)
            .id("ex-root-menu")
            .debug_selector(|| "ex-root-menu".into())
            .track_focus(&self.focus)
            .w(px(340.))
            .occlude()
            .border_1()
            .border_color(t.line_strong)
            .child(section("Worktrees"))
            .child(div().px(px(6.)).pb(px(4.)).child(self.filter.clone()))
            .child(
                div()
                    .id("ex-root-wts")
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows)
                    .when(total.is_none(), |d| d.child(div().px(px(8.)).py(px(5.)).text_size(px(12.)).text_color(t.text_3).child("Reading…")))
                    .when(total == Some(0), |d| {
                        d.child(div().px(px(8.)).py(px(5.)).text_size(px(12.)).text_color(t.text_3).child("No git repository here"))
                    }),
            )
            .child(div().my(px(4.)).h(px(1.)).bg(t.line))
            .child(Self::row("ex-root-open-folder", "folder", "Open folder…", &t).on_click(cx.listener(|this, _, window, cx| this.show_field(window, cx))))
            .when(self.typing, |d| {
                d.child(
                    div()
                        .px(px(6.))
                        .py(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().flex_1().min_w_0().child(self.path.clone()))
                        .child(
                            crate::kit::small_btn("ex-root-open", "Open", &t)
                                .debug_selector(|| "ex-root-open".into())
                                .on_click(cx.listener(|this, _, _, cx| this.open_typed(cx))),
                        )
                        .child(
                            crate::kit::icon_btn("ex-root-choose", "folder", "Choose a folder…", true, &t)
                                .debug_selector(|| "ex-root-choose".into())
                                .on_click(cx.listener(|this, _, _, cx| this.choose(cx))),
                        ),
                )
            })
            .when_some(parent, |d, p| {
                d.child(Self::row("ex-root-up", "chevron-up", "Up to parent folder", &t).on_click(cx.listener(move |this, _, _, cx| this.pick(p.clone(), cx))))
            })
            .when_some(back, |d, _| {
                d.child(Self::row("ex-root-back", "back", "Back to the agent's folder", &t).on_click(cx.listener(|_, _, _, cx| cx.emit(RootMenuEvent::AgentFolder))))
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .id("ex-root-error")
                        .debug_selector(|| "ex-root-error".into())
                        .px(px(8.))
                        .py(px(5.))
                        .text_size(px(12.))
                        .text_color(t.red)
                        .child(e),
                )
            })
    }
}
