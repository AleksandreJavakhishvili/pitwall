//! The palette itself (`CommandPalette.tsx` in a `Modal variant="palette"`,
//! `overlays.css` `.palette*`) and the per-window host that opens it.

use crate::kit::Ellipsis as _;
use std::rc::Rc;

use gpui::{
    actions, div, prelude::*, px, uniform_list, AnyElement, App, Context, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, MouseButton, Render, ScrollStrategy, SharedString, Subscription,
    UniformListScrollHandle, WeakEntity, Window,
};

use crate::explorer::Explorer;
use crate::kit::{self, icon, kbd, InputEvent, TextInput};
use crate::main_screen::{CommandState, MainScreen, OpenPalette, ScreenEvent};
use crate::theme::{self, Theme, RADIUS};

use super::matching::{parse_queue, visible};
use super::registry::{self, Command, Hint, Lead, PaletteContext, Run, RunFn, Span};

actions!(pw_palette, [Prev, Next, Pick, Close]);

/// The key context while the palette has the keyboard.
pub const CONTEXT: &str = "PwPalette";

/// `.palette-item`: 8 px padding around a 13 px × 1.45 line (18 px in
/// WebKit, which truncates line heights).
pub const ROW_H: f32 = 34.;
/// `.modal` width for the palette variant.
pub const WIDTH: f32 = 600.;

pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(CONTEXT);
    vec![
        KeyBinding::new("up", Prev, c),
        KeyBinding::new("down", Next, c),
        KeyBinding::new("enter", Pick, c),
        KeyBinding::new("escape", Close, c),
    ]
}

pub enum PaletteEvent {
    /// Close, then run this.
    Run(RunFn),
    Dismiss,
}

pub struct Palette {
    input: Entity<TextInput>,
    state: CommandState,
    screen: Option<WeakEntity<MainScreen>>,
    explorer: Option<WeakEntity<Explorer>>,
    rows: Rc<[Command]>,
    /// The query the rows were listed for.
    listed: Option<String>,
    active: usize,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// Names this opening's animation (it plays once per opening).
    serial: usize,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}

/// What the palette's host tells its window.
#[derive(Debug, Clone)]
pub enum PaletteHostEvent {
    /// The palette opened (the window shows one modal at a time).
    Opened,
}

impl EventEmitter<PaletteHostEvent> for PaletteHost {}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

const PLACEHOLDER: &str = "Jump to an agent, or type  name: prompt  to queue it";

impl Palette {
    pub fn new(
        screen: Option<Entity<MainScreen>>,
        explorer: Option<WeakEntity<Explorer>>,
        serial: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Palette {
        let input = cx.new(|cx| TextInput::new(cx, "", PLACEHOLDER).bare());
        let mut subs = vec![cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
            if *e == InputEvent::Changed {
                this.refresh(cx);
            }
        })];
        // Agents come and go while it is open: the rows follow.
        if let Some(s) = &screen {
            subs.push(cx.observe(s, |this, s, cx| {
                this.state = s.read(cx).command_state(cx);
                this.refresh(cx);
            }));
        }
        input.update(cx, |i, cx| i.focus_all(window, cx));
        let state = screen
            .as_ref()
            .map(|s| s.read(cx).command_state(cx))
            .unwrap_or_default();
        let mut p = Palette {
            input,
            state,
            screen: screen.map(|s| s.downgrade()),
            explorer,
            rows: Rc::from(vec![]),
            listed: None,
            active: 0,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            serial,
            _subs: subs,
        };
        p.refresh(cx);
        p
    }

    /// For tests and demos: the palette over a given state.
    pub fn with_state(&mut self, state: CommandState, cx: &mut Context<Self>) {
        self.state = state;
        self.refresh(cx);
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.replace_text(text, cx));
        self.refresh(cx);
    }

    pub fn rows(&self) -> &[Command] {
        &self.rows
    }

    pub fn active(&self) -> usize {
        self.active.min(self.rows.len().saturating_sub(1))
    }

    /// List the rows again; a new query selects the first one.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let q = self.query(cx);
        if self.listed.as_deref() != Some(q.as_str()) {
            self.active = 0;
        }
        self.rows = Rc::from(self.list(&q, cx));
        self.listed = Some(q);
        self.active = self.active();
        cx.notify();
    }

    /// The rows for `q`: one "Queue for" row for "name: text", else every
    /// provider's rows filtered.
    fn list(&self, q: &str, cx: &App) -> Vec<Command> {
        if let Some((agent, text)) = parse_queue(q, &self.state.agents) {
            let (id, screen) = (agent.id.clone(), self.screen.clone());
            let t = text.clone();
            return vec![Command::new("queue", "", "", move |_, cx| {
                if let Some(s) = screen.as_ref().and_then(|s| s.upgrade()) {
                    s.update(cx, |s, cx| s.cmd_queue(&id, t.clone(), cx));
                }
            })
            .label(vec![
                Span::Text("Queue for ".into()),
                Span::Strong(agent.name.clone().into()),
                Span::Text(": ".into()),
                Span::Quote(text.into()),
            ])
            .glyph("↳")
            .keys("↵")];
        }
        let pc = PaletteContext {
            query: q,
            state: &self.state,
            screen: self.screen.clone(),
            explorer: self.explorer.clone(),
        };
        visible(registry::collect(&pc, cx), q)
    }

    /// Select row `ix` (clamped), scrolled into view.
    pub fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let ix = ix.min(self.rows.len().saturating_sub(1));
        self.move_to(ix, cx);
    }

    fn move_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        let down = ix > self.active;
        self.active = ix;
        // Only scrolls when the row is out of view (`scrollIntoView` nearest).
        let to = if down {
            ScrollStrategy::Bottom
        } else {
            ScrollStrategy::Top
        };
        self.scroll.scroll_to_item(ix, to);
        cx.notify();
    }

    fn prev(&mut self, _: &Prev, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active().saturating_sub(1);
        self.move_to(ix, cx);
    }

    fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        let ix = (self.active() + 1).min(self.rows.len().saturating_sub(1));
        self.move_to(ix, cx);
    }

    fn pick(&mut self, _: &Pick, window: &mut Window, cx: &mut Context<Self>) {
        self.run(self.active(), window, cx);
    }

    fn run(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else { return };
        match row.run.clone() {
            Run::Do(f) => cx.emit(PaletteEvent::Run(f)),
            Run::Fill(text) => {
                // The cursor ends up after "name: ", ready for the prompt.
                self.set_query(&text, cx);
                window.focus(&self.input.focus_handle(cx));
            }
        }
    }

    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismiss);
    }
}

/// `.pal-lead`: an 18 px column holding the row's icon.
fn lead(l: &Option<Lead>, ix: usize, t: &Theme, motion: bool) -> AnyElement {
    let inner: AnyElement = match l {
        None => div().into_any_element(),
        // The web view's fallback face draws ✕ about 6 px across; GPUI's
        // draws it at 9, so it is the kit's x at that ink.
        Some(Lead::Glyph(g)) if g.as_ref() == "✕" => icon("x", 12., t.text_3).into_any_element(),
        Some(Lead::Glyph(g)) => glyph(g.clone(), t.text_3).into_any_element(),
        // GPUI's fallback face draws ▲ larger than the web view's: sized
        // to the same 9 px width.
        Some(Lead::Warn(g)) => glyph(g.clone(), t.amber)
            .text_size(px(9.5))
            .into_any_element(),
        Some(Lead::Icon(name)) => icon(name, 14., t.text_3).into_any_element(),
        Some(Lead::Status(s)) => kit::status_glyph_el(
            *s,
            t,
            // The web view draws `.glyph-sm`'s ● ⚑ ■ at about the kit's
            // medium size (its font fallback), so match what shows.
            kit::GlyphSize::Md,
            motion.then(|| ElementId::NamedInteger("pal-pulse".into(), ix as u64)),
        )
        .into_any_element(),
    };
    div()
        .w(px(18.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(inner)
        .into_any_element()
}

/// `.pal-icon`: 13 px, dim.
fn glyph(g: SharedString, color: gpui::Hsla) -> gpui::Div {
    div().text_size(px(13.)).text_color(color).child(g)
}

/// `.pal-label`: the spans on one line, cut with an ellipsis.
fn label(spans: &[Span], t: &Theme) -> impl IntoElement {
    let piece = |d: gpui::Div| d.flex_shrink().min_w_0().ellipsis();
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .flex()
        .items_baseline()
        .children(spans.iter().map(|s| {
            match s {
                Span::Text(x) => piece(div()).child(x.clone()),
                Span::Strong(x) => piece(div()).font_weight(FontWeight::BOLD).child(x.clone()),
                // A space and `margin-left: 6px`.
                Span::Sub(x) => piece(div())
                    .ml(px(9.5))
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child(x.clone()),
                Span::Quote(x) => piece(div()).text_color(t.text_2).child(x.clone()),
                Span::Mono(x) => piece(div()).font_family(kit::MONO_FONT).child(x.clone()),
            }
        }))
}

fn hint(h: &Hint, t: &Theme) -> AnyElement {
    match h {
        Hint::Keys(k) => kbd(k, t).into_any_element(),
        Hint::Sub(x) => div()
            .ml(px(6.))
            .text_size(px(12.))
            .text_color(t.text_3)
            .child(x.clone())
            .into_any_element(),
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).float();
        let motion = theme::motion_on(cx);
        let vh = f32::from(window.viewport_size().height);
        let rows = self.rows.clone();
        let active = self.active();
        let n = rows.len();
        let list_max = (vh * 0.52 - 12.).max(ROW_H);
        let list = uniform_list(
            "palette-list",
            n,
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                let t = theme::theme(cx).float();
                range
                    .map(|ix| {
                        let row = &rows[ix];
                        crate::kit::hover::probed(div().id(ix))
                            .w_full()
                            .h(px(ROW_H))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .rounded(RADIUS)
                            .cursor_pointer()
                            .when(ix == active, |d| d.bg(t.surface_3))
                            .on_mouse_move(cx.listener(move |this: &mut Palette, _, _, cx| {
                                if this.active != ix {
                                    this.active = ix;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this: &mut Palette, _, window, cx| {
                                this.run(ix, window, cx)
                            }))
                            .child(lead(&row.lead, ix, &t, motion))
                            .child(label(&row.label, &t))
                            .children(row.hint.as_ref().map(|h| hint(h, &t)))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .h(px((n as f32 * ROW_H).min(list_max)));

        let panel = div()
            .id("palette")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::prev))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::pick))
            .on_action(cx.listener(Self::close))
            // ⌘K again closes it (React: the shortcut toggles).
            .on_action(cx.listener(|_, _: &OpenPalette, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .occlude()
            .w(px(WIDTH))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(t.raised)
            .shadow(t.float_shadow())
            .overflow_hidden()
            .font_family(kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(
                div()
                    .flex_none()
                    .px(px(18.))
                    .py(px(16.))
                    .border_b_1()
                    .border_color(t.line)
                    .text_size(px(15.))
                    // 15 px × the body's 1.45.
                    .line_height(px(21.))
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .p(px(6.))
                    .when(n == 0, |d| {
                        d.child(
                            div()
                                .p(px(14.))
                                .text_color(t.text_3)
                                .child("No matches. Try “name: your prompt” to queue."),
                        )
                    })
                    .when(n > 0, |d| {
                        // The bar sits in the list's right padding's place,
                        // as WebKit puts it outside the padding box.
                        d.pr_0().child(kit::vscroll_list("palette-bar", &self.scroll,
                            &t,
                            list.mr(px(6.)),
                        ))
                    }),
            );
        let panel: AnyElement = if motion {
            kit::motion::enter(
                ElementId::NamedInteger("palette-open".into(), self.serial as u64),
                kit::motion::Fx::PALETTE,
                panel,
            )
            .into_any_element()
        } else {
            panel.into_any_element()
        };
        let scrim = t.scrim();
        let backdrop = div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .p(px(24.))
            .pt(px(vh * 0.12))
            .flex()
            .justify_center()
            .items_start()
            .bg(scrim)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Dismiss)),
            )
            .child(panel);
        if motion {
            kit::scrim_in(
                ElementId::NamedInteger("palette-fade".into(), self.serial as u64),
                backdrop,
            )
        } else {
            backdrop.into_any_element()
        }
    }
}

/// One per window: opens the palette on [`ScreenEvent::OpenPalette`] (⌘K,
/// the top bar's Search, the strip's "⌘K commands"), closes it, gives the
/// keyboard back to where it was, then runs the chosen row.
pub struct PaletteHost {
    screen: Option<Entity<MainScreen>>,
    explorer: Option<WeakEntity<Explorer>>,
    palette: Option<Entity<Palette>>,
    restore: Option<FocusHandle>,
    serial: usize,
    _subs: Vec<Subscription>,
}

impl PaletteHost {
    pub fn new(
        screen: Option<Entity<MainScreen>>,
        explorer: Option<&Entity<Explorer>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaletteHost {
        let mut subs = vec![];
        if let Some(s) = &screen {
            subs.push(
                cx.subscribe_in(s, window, |this, _, e: &ScreenEvent, window, cx| {
                    if *e == ScreenEvent::OpenPalette {
                        this.toggle(window, cx);
                    }
                }),
            );
        }
        PaletteHost {
            screen,
            explorer: explorer.map(|e| e.downgrade()),
            palette: None,
            restore: None,
            serial: 0,
            _subs: subs,
        }
    }

    pub fn is_open(&self) -> bool {
        self.palette.is_some()
    }

    pub fn palette(&self) -> Option<&Entity<Palette>> {
        self.palette.as_ref()
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_open() {
            self.close(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_open() {
            return;
        }
        self.restore = window.focused(cx);
        self.serial += 1;
        let (screen, explorer, serial) = (self.screen.clone(), self.explorer.clone(), self.serial);
        let palette = cx.new(|cx| Palette::new(screen, explorer, serial, window, cx));
        self._subs.push(cx.subscribe_in(
            &palette,
            window,
            |this, _, e: &PaletteEvent, window, cx| match e {
                PaletteEvent::Dismiss => this.close(window, cx),
                PaletteEvent::Run(f) => {
                    let f = f.clone();
                    this.close(window, cx);
                    f(window, cx);
                }
            },
        ));
        self.palette = Some(palette);
        cx.emit(PaletteHostEvent::Opened);
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_none() {
            return;
        }
        // The palette's own subscription goes with it.
        self._subs.truncate(usize::from(self.screen.is_some()));
        if let Some(f) = self.restore.take() {
            window.focus(&f);
        }
        cx.notify();
    }
}

impl Render for PaletteHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.palette {
            Some(p) => p.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }
}

/// ⌘K wherever the keyboard is outside the main screen (the explorer's
/// overlays, Settings): handled on the window's root element.
pub fn on_actions<E: InteractiveElement>(el: E, host: &Entity<PaletteHost>) -> E {
    let host = host.clone();
    el.on_action(move |_: &OpenPalette, window, cx| host.update(cx, |h, cx| h.toggle(window, cx)))
}
