//! The palette's form for a tile's command that needs input
//! (agent-tools.md §11.12): "<Extension>: <title>" over one field per
//! property of the command's input (`tiles::form`), drawn as the tile
//! components' form rows draw (a label with a red mark when needed, the
//! control, the property's description, what is wrong). Run sends the
//! command as the user's own; Esc, Cancel or a click outside closes.

use gpui::{div, prelude::*, px, AnyElement, App, Context, Entity, EventEmitter, Focusable, IntoElement, SharedString, Subscription, Window};
use pitwall_proto::tile_tools::TileRun;

use super::dialogs::DialogEvent;
use crate::kit::pop;
use crate::kit::{self, BtnKind, InputEvent, TextInput};
use crate::theme::{self, Theme};
use crate::tiles::form::{self, Field, Given, Kind};

/// The most options a choice shows side by side (more: a dropdown).
const SIDE_BY_SIDE: usize = 4;

pub struct ToolForm {
    req: TileRun,
    title: String,
    fields: Vec<Field>,
    /// Text and number fields' inputs (by field).
    inputs: Vec<Option<Entity<TextInput>>>,
    /// Booleans' and choices' values (by field).
    given: Vec<Given>,
    /// The choice whose dropdown is open.
    open: Option<usize>,
    problems: Vec<(usize, String)>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<DialogEvent> for ToolForm {}

impl ToolForm {
    pub fn new(req: TileRun, title: String, fields: Vec<Field>, window: &mut Window, cx: &mut Context<Self>) -> ToolForm {
        let mut subs = Vec::new();
        let inputs: Vec<Option<Entity<TextInput>>> = fields
            .iter()
            .map(|f| {
                let max = match f.kind {
                    Kind::Text { max } => max,
                    Kind::Number { .. } => Some(40),
                    _ => return None,
                };
                let placeholder = match f.kind {
                    Kind::Number { integer: true } => "A whole number",
                    Kind::Number { .. } => "A number",
                    _ => "",
                };
                let input = cx.new(|cx| {
                    let i = TextInput::new(cx, "", placeholder);
                    match max {
                        Some(n) => i.max_len(n),
                        None => i,
                    }
                });
                subs.push(cx.subscribe_in(&input, window, |this, _, e: &InputEvent, _, cx| match e {
                    InputEvent::Submit => this.run(cx),
                    InputEvent::Cancel => cx.emit(DialogEvent::Close),
                    _ => {}
                }));
                Some(input)
            })
            .collect();
        if let Some(first) = inputs.iter().flatten().next() {
            window.focus(&first.focus_handle(cx), cx);
        }
        let given = fields.iter().map(form::initial).collect();
        ToolForm { req, title, fields, inputs, given, open: None, problems: Vec::new(), _subs: subs }
    }

    /// What the user gave each field.
    fn values(&self, cx: &App) -> Vec<Given> {
        self.given.iter().zip(&self.inputs).map(|(g, i)| i.as_ref().map_or_else(|| g.clone(), |i| Given::Text(i.read(cx).text().to_string()))).collect()
    }

    /// Run the command with what was given, or say what is wrong.
    pub(crate) fn run(&mut self, cx: &mut Context<Self>) {
        match form::input(&self.fields, &self.values(cx)) {
            Ok(input) => {
                let mut req = self.req.clone();
                req.input = input.to_string();
                cx.emit(DialogEvent::RunTool { req: Box::new(req), title: self.title.clone() });
            }
            Err(p) => {
                self.problems = p;
                cx.notify();
            }
        }
    }

    /// Field `i` given `g` (a boolean or a choice), or its text typed.
    pub(crate) fn set(&mut self, i: usize, g: Given, cx: &mut Context<Self>) {
        match (&g, self.inputs.get(i).cloned().flatten()) {
            (Given::Text(t), Some(input)) => input.update(cx, |x, cx| x.set_text(t.clone(), cx)),
            _ => {
                if let Some(slot) = self.given.get_mut(i) {
                    *slot = g;
                }
            }
        }
        self.open = None;
        self.problems.retain(|(k, _)| *k != i);
        cx.notify();
    }

    fn control(&self, i: usize, f: &Field, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = format!("tool-form-{}", f.name);
        if let Some(input) = self.inputs[i].clone() {
            return div().debug_selector(move || id).child(input).into_any_element();
        }
        match (&f.kind, &self.given[i]) {
            (Kind::Bool, Given::Bool(on)) => {
                let on = *on;
                let sel = id.clone();
                div()
                    .id(SharedString::from(id))
                    .debug_selector(move || sel)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.set(i, Given::Bool(!on), cx)))
                    .child(kit::checkbox_box(on, false, 14., t))
                    .child(div().text_size(px(13.)).text_color(t.text).child(if on { "Yes" } else { "No" }))
                    .into_any_element()
            }
            (Kind::Choice(options), Given::Choice(pick)) => {
                let pick = *pick;
                let me = cx.entity().downgrade();
                let on_pick: kit::OnPick = std::rc::Rc::new(move |k, _, cx| {
                    let _ = me.update(cx, |this, cx| this.set(i, Given::Choice(Some(k)), cx));
                });
                if options.len() <= SIDE_BY_SIDE {
                    let items = options.iter().enumerate().map(|(k, o)| kit::SegItem::new(o.clone(), pick == Some(k))).collect();
                    return div().debug_selector(move || id.clone()).flex().child(kit::seg(SharedString::from(format!("tool-form-seg-{}", f.name)), items, t, on_pick)).into_any_element();
                }
                let open = self.open == Some(i);
                let items = options.iter().enumerate().map(|(k, o)| (SharedString::from(o.clone()), pick == Some(k))).collect();
                let text = pick.and_then(|k| options.get(k)).cloned().unwrap_or_else(|| "Choose…".into());
                let (toggle, close) = (cx.entity().downgrade(), cx.entity().downgrade());
                let sel = kit::select(
                    SharedString::from(id.clone()),
                    text,
                    32.,
                    13.,
                    true,
                    open,
                    items,
                    t,
                    move |_, cx| {
                        let _ = toggle.update(cx, |this, cx| {
                            this.open = if this.open == Some(i) { None } else { Some(i) };
                            cx.notify();
                        });
                    },
                    on_pick,
                    std::rc::Rc::new(move |_, cx| {
                        let _ = close.update(cx, |this, cx| {
                            this.open = None;
                            cx.notify();
                        });
                    }),
                );
                div().debug_selector(move || id.clone()).child(sel).into_any_element()
            }
            _ => div().into_any_element(),
        }
    }
}

impl Render for ToolForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if pop::is_pop(window) { crate::kit::modal::on_glass(theme::theme(cx)) } else { theme::theme(cx).clone() };
        let rows: Vec<AnyElement> = self
            .fields
            .clone()
            .iter()
            .enumerate()
            .map(|(i, f)| {
                // As a tile's form row draws it.
                let head = div()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .text_size(px(12.5))
                    .font_weight(kit::WEIGHT_550)
                    .text_color(t.text_2)
                    .child(f.label.clone())
                    .when(f.required, |d| d.child(div().text_color(t.red).child("*")));
                let problem = self.problems.iter().find(|(k, _)| *k == i).map(|(_, p)| p.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(head)
                    .child(self.control(i, f, &t, cx))
                    .when_some(problem, |d, p| {
                        let sel = format!("tool-form-problem-{}", f.name);
                        d.child(kit::error_text(p, &t).debug_selector(move || sel))
                    })
                    .when_some(f.description.clone(), |d, h| d.child(kit::hint(h, &t)))
                    .into_any_element()
            })
            .collect();
        let close = cx.entity();
        crate::kit::Modal::new("modal", 440., move |_, cx| close.update(cx, |_, cx| cx.emit(DialogEvent::Close)))
            .title(self.title.clone())
            .motion(theme::motion_on(cx))
            .render_in(
                window,
                cx,
                &t,
                div()
                    .debug_selector(|| "tool-form".into())
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .child(kit::dialog::scrolling("tool-form-body", kit::dialog::body().children(rows), &t))
                    .child(
                        kit::dialog::foot(&t)
                            .child(kit::button("tool-form-cancel", BtnKind::Ghost, false, &t).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))).child("Cancel"))
                            .child(
                                kit::button("tool-form-run", BtnKind::Primary, false, &t)
                                    .debug_selector(|| "tool-form-run".into())
                                    .on_click(cx.listener(|this, _, _, cx| this.run(cx)))
                                    .child("Run")
                                    .child(kit::kbd_on_primary("↵", &t)),
                            ),
                    ),
            )
    }
}
