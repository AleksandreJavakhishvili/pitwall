//! Pitwall's own components, drawn by Pitwall with the kit its screens
//! use (docs/spec/tile-sdk/ui.md §3.10, "kit parity"): form controls,
//! feedback, layout, data display and charts. Anything Pitwall's own UI
//! draws, a tile draws the same way, in the user's theme.
//!
//! ```ignore
//! v_flex().gap_3().p_3()
//!     .child(form_row("Notify me").help("When a run fails")
//!         .child(switch("On failure").checked(self.notify)
//!             .on_change(cx.listener(|this, on: &bool, cx| { this.notify = *on; cx.notify(); }))))
//!     .child(form_row("Retries")
//!         .child(number_field(Some(self.retries as f64)).min(0.).max(10.)
//!             .on_change(cx.listener(|this, v: &Option<f64>, cx| { this.retries = v.unwrap_or(0.) as u32; cx.notify(); }))))
//!     .child(bar_chart(["Mon", "Tue", "Wed"]).series("Runs", [12., 18., 9.]).h(px(160.)))
//! ```
//!
//! **Controls are controlled.** A control shows the value the tile gives
//! it; the user's change comes to its `on_change` listener, and the tile
//! renders again with the new value (call [`Cx::notify`](crate::Cx::notify)).
//! Without a listener a control can't be changed. Text fields are the
//! exception: Pitwall owns what is typed ([`input`](crate::input),
//! [`search_field`]).
//!
//! **Keyboard.** Every control is a Tab stop unless disabled, with
//! Pitwall's focus ring. Space toggles a checkbox or switch; the arrow
//! keys choose in a radio group or a segmented control and move a slider;
//! Up and Down step a number field; Enter or Space opens a disclosure;
//! Backspace or Delete removes a focused chip; Enter confirms a confirm
//! sheet and Esc cancels it or closes a popover.

use crate::bindings::pitwall::tile::types as wit;
use crate::element::{builder, AnyElement, Handler, Icon, Interactive, IntoElement, Kind, Listen, ParentElement};
use crate::events::{Click, Dismissed};
use crate::style::{StyleSet, Styled, Token};
use crate::Tone;

#[derive(Debug, Clone)]
pub(crate) struct WidgetSpec(pub wit::Element);

fn widget(e: wit::Element) -> AnyElement {
    AnyElement::new_kind(Kind::Widget(Box::new(WidgetSpec(e))))
}

/// The component's WIT record, to change it in place.
macro_rules! spec {
    ($self:ident, $case:ident) => {
        match &mut $self.0.kind {
            Kind::Widget(w) => match &mut w.0 {
                wit::Element::$case(s) => s,
                _ => unreachable!(concat!("a ", stringify!($case))),
            },
            _ => unreachable!("a component"),
        }
    };
}

macro_rules! parent {
    ($t:ident) => {
        impl ParentElement for $t {
            fn children_mut(&mut self) -> &mut Vec<AnyElement> {
                &mut self.0.children
            }
        }
    };
}

fn icon(i: Icon) -> wit::Icon {
    wit::Icon::Named(i.name().to_string())
}

// ---- Form controls ----------------------------------------------------------

/// A checkbox or a switch. Made by [`checkbox`] and [`switch`].
#[derive(Debug, Clone)]
pub struct Check(AnyElement);
builder!(Check);

impl Check {
    fn spec(&mut self) -> &mut wit::Check {
        match &mut self.0.kind {
            Kind::Widget(w) => match &mut w.0 {
                wit::Element::Checkbox(c) | wit::Element::Switch(c) => c,
                _ => unreachable!("a check"),
            },
            _ => unreachable!("a component"),
        }
    }

    /// On.
    pub fn checked(mut self, on: bool) -> Check {
        self.spec().checked = on;
        self
    }

    /// A checkbox that stands for a group of which only some are on: drawn
    /// with a dash; a click sends `true`.
    pub fn mixed(mut self, yes: bool) -> Check {
        self.spec().mixed = yes;
        self
    }

    /// Drawn dimmed; takes no clicks and no focus.
    pub fn disabled(mut self, yes: bool) -> Check {
        self.spec().disabled = yes;
        self
    }

    /// Call `handler` with the new state (`&bool`) when the user clicks it
    /// or its label, or presses Space while it has focus.
    pub fn on_change(mut self, handler: Handler<bool>) -> Check {
        self.0.listen(Listen::Value, handler);
        self
    }
}

/// A checkbox, as Pitwall's own, with `label` after it (`""` for none:
/// then give it a [`label`](Interactive::label) for screen readers).
///
/// ```ignore
/// checkbox("Only failures").checked(self.only_failures)
///     .on_change(cx.listener(|this, on: &bool, cx| { this.only_failures = *on; cx.notify(); }))
/// ```
pub fn checkbox(label: impl Into<String>) -> Check {
    Check(widget(wit::Element::Checkbox(wit::Check { label: label.into(), checked: false, mixed: false, disabled: false })))
}

/// A switch (an on/off toggle), as Pitwall's settings draw them, with
/// `label` before it.
///
/// ```ignore
/// switch("Notify on failure").checked(self.notify)
///     .on_change(cx.listener(|this, on: &bool, cx| { this.notify = *on; cx.notify(); }))
/// ```
pub fn switch(label: impl Into<String>) -> Check {
    Check(widget(wit::Element::Switch(wit::Check { label: label.into(), checked: false, mixed: false, disabled: false })))
}

/// A radio group or a segmented control. Made by [`radio_group`] and
/// [`segmented`].
#[derive(Debug, Clone)]
pub struct Choices(AnyElement);
builder!(Choices);

impl Choices {
    fn spec(&mut self) -> &mut wit::ChoiceGroup {
        match &mut self.0.kind {
            Kind::Widget(w) => match &mut w.0 {
                wit::Element::RadioGroup(g) | wit::Element::Segmented(g) => g,
                _ => unreachable!("a choice group"),
            },
            _ => unreachable!("a component"),
        }
    }

    /// Add a choice: `key` (unique in the group, sent when it is chosen)
    /// and what it says. At most 64.
    pub fn option(mut self, key: impl Into<String>, label: impl Into<String>) -> Choices {
        self.spec().items.push(wit::ChoiceItem { key: key.into(), label: label.into(), disabled: false });
        self
    }

    /// The last choice added can't be chosen (drawn dimmed).
    pub fn option_disabled(mut self) -> Choices {
        if let Some(i) = self.spec().items.last_mut() {
            i.disabled = true;
        }
        self
    }

    /// The chosen choice's key.
    pub fn selected(mut self, key: impl Into<String>) -> Choices {
        self.spec().selected = Some(key.into());
        self
    }

    /// A radio group in a row instead of a column.
    pub fn horizontal(mut self) -> Choices {
        self.spec().horizontal = true;
        self
    }

    /// The whole group is dimmed and can't be changed.
    pub fn disabled(mut self, yes: bool) -> Choices {
        self.spec().disabled = yes;
        self
    }

    /// Call `handler` with the chosen key (`&str`) when the user picks a
    /// choice (a click, or the arrow keys while it has focus).
    pub fn on_change(mut self, handler: Handler<str>) -> Choices {
        self.0.listen(Listen::Value, handler);
        self
    }
}

/// A group of radio buttons, one per [`option`](Choices::option), in a
/// column. One Tab stop: the arrow keys move to the next choice and
/// choose it.
///
/// ```ignore
/// radio_group().option("all", "All runs").option("mine", "My runs").selected(&self.scope)
///     .on_change(cx.listener(|this, key: &str, cx| { this.scope = key.to_string(); cx.notify(); }))
/// ```
pub fn radio_group() -> Choices {
    Choices(widget(wit::Element::RadioGroup(wit::ChoiceGroup { items: Vec::new(), selected: None, horizontal: false, disabled: false })))
}

/// A segmented control: a few choices side by side, as Pitwall's own
/// (Review's Side by side / Inline).
///
/// ```ignore
/// segmented().option("day", "Day").option("week", "Week").selected(&self.range)
///     .on_change(cx.listener(|this, key: &str, cx| { this.range = key.into(); cx.notify(); }))
/// ```
pub fn segmented() -> Choices {
    Choices(widget(wit::Element::Segmented(wit::ChoiceGroup { items: Vec::new(), selected: None, horizontal: true, disabled: false })))
}

/// A slider. Made by [`slider`].
#[derive(Debug, Clone)]
pub struct Slider(AnyElement);
builder!(Slider);

impl Slider {
    /// The steps it snaps to (default: none; the keys then move a
    /// hundredth of the range).
    pub fn step(mut self, step: f64) -> Slider {
        spec!(self, Slider).step = step;
        self
    }

    /// Draw the value at its right.
    pub fn show_value(mut self) -> Slider {
        spec!(self, Slider).show_value = true;
        self
    }

    /// Dimmed and can't be changed.
    pub fn disabled(mut self, yes: bool) -> Slider {
        spec!(self, Slider).disabled = yes;
        self
    }

    /// Call `handler` with the new value (`&f64`) while the user drags it
    /// (at most every 50 ms) and when they let go, or press a key.
    pub fn on_change(mut self, handler: Handler<f64>) -> Slider {
        self.0.listen(Listen::Value, handler);
        self
    }
}

/// A slider at `value` between `range`'s ends. Give it a width (it fills
/// its row otherwise).
///
/// ```ignore
/// slider(self.volume, 0.0..=100.0).step(5.0).show_value()
///     .on_change(cx.listener(|this, v: &f64, cx| { this.volume = *v; cx.notify(); }))
/// ```
pub fn slider(value: f64, range: std::ops::RangeInclusive<f64>) -> Slider {
    Slider(widget(wit::Element::Slider(wit::Slider { value, min: *range.start(), max: *range.end(), step: 0., show_value: false, disabled: false })))
}

/// A number field. Made by [`number_field`].
#[derive(Debug, Clone)]
pub struct NumberField(AnyElement);
builder!(NumberField);

impl NumberField {
    /// The lowest value it takes.
    pub fn min(mut self, v: f64) -> NumberField {
        spec!(self, NumberField).min = Some(v);
        self
    }

    /// The highest value it takes.
    pub fn max(mut self, v: f64) -> NumberField {
        spec!(self, NumberField).max = Some(v);
        self
    }

    /// What − and + (and Up and Down) add (default 1).
    pub fn step(mut self, v: f64) -> NumberField {
        spec!(self, NumberField).step = v;
        self
    }

    /// Digits after the point, 0–6 (default 0).
    pub fn decimals(mut self, n: u8) -> NumberField {
        spec!(self, NumberField).decimals = n;
        self
    }

    /// Shown in faint text while it is empty.
    pub fn placeholder(mut self, p: impl Into<String>) -> NumberField {
        spec!(self, NumberField).placeholder = Some(p.into());
        self
    }

    /// Shown, but can't be changed.
    pub fn readonly(mut self, yes: bool) -> NumberField {
        spec!(self, NumberField).readonly = yes;
        self
    }

    /// Dimmed; can't be changed or focused.
    pub fn disabled(mut self, yes: bool) -> NumberField {
        spec!(self, NumberField).disabled = yes;
        self
    }

    /// Call `handler` with the new value (`&Option<f64>`: `None` when the
    /// user emptied it) after Enter, leaving the field, or a step.
    pub fn on_change(mut self, handler: Handler<Option<f64>>) -> NumberField {
        self.0.listen(Listen::Value, handler);
        self
    }
}

/// A number field showing `value` (`None`: empty), with − and + steps.
/// Typed text that isn't a number is put back; numbers are kept within
/// [`min`](NumberField::min) and [`max`](NumberField::max).
///
/// ```ignore
/// number_field(Some(self.retries as f64)).min(0.).max(10.)
///     .on_change(cx.listener(|this, v: &Option<f64>, cx| { this.retries = v.unwrap_or(0.) as u32; cx.notify(); }))
/// ```
pub fn number_field(value: Option<f64>) -> NumberField {
    NumberField(widget(wit::Element::NumberField(wit::NumberField { value, min: None, max: None, step: 1., decimals: 0, placeholder: None, readonly: false, disabled: false })))
}

/// A search field: a text field (`key` is required, as for
/// [`input`](crate::input)) with a magnifier and a clear button. It is an
/// [`Input`](crate::Input): `.value`, `.placeholder`, `.on_change` and
/// `.on_submit` work as there, and the clear button sends `on_change`
/// with `""`.
///
/// ```ignore
/// search_field("filter").placeholder("Filter runs")
///     .on_change(cx.listener(|this, q: &str, cx| { this.query = q.to_string(); cx.notify(); }))
/// ```
pub fn search_field(key: impl Into<String>) -> crate::Input {
    crate::element::search_input(key.into())
}

/// A form row. Made by [`form_row`].
#[derive(Debug, Clone)]
pub struct FormRow(AnyElement);
builder!(FormRow);
parent!(FormRow);

impl FormRow {
    /// Help under the control, in muted text.
    pub fn help(mut self, text: impl Into<String>) -> FormRow {
        spec!(self, FormRow).help = Some(text.into());
        self
    }

    /// A validation message, in red, in place of the help (`None`: none).
    pub fn error(mut self, message: Option<impl Into<String>>) -> FormRow {
        spec!(self, FormRow).error = message.map(Into::into);
        self
    }

    /// A required field: a mark after the label.
    pub fn required(mut self) -> FormRow {
        spec!(self, FormRow).required = true;
        self
    }

    /// The label beside the control, in a fixed column, instead of over it.
    pub fn inline(mut self) -> FormRow {
        spec!(self, FormRow).inline = true;
        self
    }
}

/// A form row: `label` over its children (the control), with help, a
/// validation message and a required mark. The label is the control's
/// accessible name when it has none.
///
/// ```ignore
/// form_row("Branch").required().help("The branch runs start from")
///     .error(self.branch.is_empty().then_some("Name a branch"))
///     .child(input("branch").value(&self.branch))
/// ```
pub fn form_row(label: impl Into<String>) -> FormRow {
    FormRow(widget(wit::Element::FormRow(wit::FormRow { label: label.into(), help: None, error: None, required: false, inline: false })))
}

// ---- Feedback ---------------------------------------------------------------

/// A progress bar. Made by [`progress`].
#[derive(Debug, Clone)]
pub struct Progress(AnyElement);
builder!(Progress);

impl Progress {
    /// Its colour by meaning (default: the text colour).
    pub fn tone(mut self, tone: Tone) -> Progress {
        spec!(self, Progress).tone = Some(tone.wit());
        self
    }
}

/// A progress bar: `Some(share)` done (0.0–1.0), or `None` while it can't
/// be known (it moves, unless Reduce motion is on). Give it a
/// [`label`](Interactive::label) ("Uploading").
///
/// ```ignore
/// progress(Some(self.done as f32 / self.total as f32)).label("Indexing")
/// ```
pub fn progress(value: Option<f32>) -> Progress {
    Progress(widget(wit::Element::Progress(wit::Progress { value, tone: None })))
}

/// A component with nothing to set. Made by [`spinner`], [`skeleton`],
/// [`kbd`], [`divider`] and [`toolbar`].
#[derive(Debug, Clone)]
pub struct Widget(AnyElement);
builder!(Widget);
parent!(Widget);

/// A spinning ring, as Pitwall's own: something is loading. Still with
/// Reduce motion. Give it a [`label`](Interactive::label).
pub fn spinner() -> Widget {
    Widget(widget(wit::Element::Spinner))
}

/// How a [`skeleton`] is shaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkeletonShape {
    /// This many lines of text (1–20), the last one shorter.
    Lines(u8),
    /// A box (give it a size).
    Block,
    /// A circle, as tall as its width.
    Circle,
}

/// A grey placeholder where content is loading; it shimmers unless
/// Reduce motion is on.
///
/// ```ignore
/// v_flex().gap_2().child(skeleton(SkeletonShape::Circle).size(px(28.))).child(skeleton(SkeletonShape::Lines(3)))
/// ```
pub fn skeleton(shape: SkeletonShape) -> Widget {
    let s = match shape {
        SkeletonShape::Lines(n) => wit::Skeleton::Lines(n),
        SkeletonShape::Block => wit::Skeleton::Block,
        SkeletonShape::Circle => wit::Skeleton::Circle,
    };
    Widget(widget(wit::Element::Skeleton(s)))
}

/// An empty state. Made by [`empty_state`].
#[derive(Debug, Clone)]
pub struct EmptyState(AnyElement);
builder!(EmptyState);

impl EmptyState {
    /// An icon over the title.
    pub fn icon(mut self, i: Icon) -> EmptyState {
        spec!(self, EmptyState).icon = Some(icon(i));
        self
    }

    /// What to do about it.
    pub fn text(mut self, text: impl Into<String>) -> EmptyState {
        spec!(self, EmptyState).text = Some(text.into());
        self
    }

    /// An action button, calling `handler` when clicked.
    pub fn action(mut self, label: impl Into<String>, handler: Handler<Click>) -> EmptyState {
        spec!(self, EmptyState).action = Some(label.into());
        self.0.listen(Listen::Click, handler);
        self
    }
}

/// An empty state, centred in its box, as Pitwall's own lists say they
/// are empty.
///
/// ```ignore
/// empty_state("No runs yet").icon(Icon::Inbox).text("Runs show here when CI starts one.")
///     .action("Refresh", cx.listener(|this, _, cx| this.refresh(cx)))
/// ```
pub fn empty_state(title: impl Into<String>) -> EmptyState {
    EmptyState(widget(wit::Element::EmptyState(wit::EmptyState { icon: None, title: title.into(), text: None, action: None })))
}

/// An inline banner. Made by [`banner`].
#[derive(Debug, Clone)]
pub struct Banner(AnyElement);
builder!(Banner);

impl Banner {
    /// A bold first line.
    pub fn title(mut self, title: impl Into<String>) -> Banner {
        spec!(self, Banner).title = Some(title.into());
        self
    }

    /// An action button, calling `handler` when clicked.
    pub fn action(mut self, label: impl Into<String>, handler: Handler<Click>) -> Banner {
        spec!(self, Banner).action = Some(label.into());
        self.0.listen(Listen::Click, handler);
        self
    }

    /// A close button, calling `handler` when clicked (leave the banner
    /// out after it).
    pub fn on_dismiss(mut self, handler: Handler<Dismissed>) -> Banner {
        spec!(self, Banner).dismissible = true;
        self.0.listen(Listen::Dismiss, handler);
        self
    }
}

/// An inline banner in the tile's flow: information ([`Tone::Info`]),
/// success ([`Tone::Good`]), a warning or an error.
///
/// ```ignore
/// banner(Tone::Bad, "The CI token was refused.").title("Can't load runs")
///     .action("Open settings", cx.listener(|_, _, _| ui::open_settings()))
///     .on_dismiss(cx.listener(|this, _, cx| { this.error = None; cx.notify(); }))
/// ```
pub fn banner(tone: Tone, text: impl Into<String>) -> Banner {
    Banner(widget(wit::Element::Banner(wit::Banner { tone: tone.wit(), title: None, text: text.into(), action: None, dismissible: false })))
}

/// A badge. Made by [`badge`].
#[derive(Debug, Clone)]
pub struct Badge(AnyElement);
builder!(Badge);

impl Badge {
    /// Its colour by meaning (default: neutral).
    pub fn tone(mut self, tone: Tone) -> Badge {
        spec!(self, Badge).tone = Some(tone.wit());
        self
    }
}

/// A small count or status label (at most 32 bytes).
///
/// ```ignore
/// h_flex().gap_2().child("Failing").child(badge("3").tone(Tone::Bad))
/// ```
pub fn badge(text: impl Into<String>) -> Badge {
    Badge(widget(wit::Element::Badge(wit::Badge { text: text.into(), tone: None })))
}

/// A chip. Made by [`chip`].
#[derive(Debug, Clone)]
pub struct Chip(AnyElement);
builder!(Chip);

impl Chip {
    /// An icon before the label.
    pub fn icon(mut self, i: Icon) -> Chip {
        spec!(self, Chip).icon = Some(icon(i));
        self
    }

    /// Its colour by meaning (default: neutral).
    pub fn tone(mut self, tone: Tone) -> Chip {
        spec!(self, Chip).tone = Some(tone.wit());
        self
    }

    /// Drawn picked (a filter that is on).
    pub fn selected(mut self, yes: bool) -> Chip {
        spec!(self, Chip).selected = yes;
        self
    }

    /// Dimmed; takes no clicks.
    pub fn disabled(mut self, yes: bool) -> Chip {
        spec!(self, Chip).disabled = yes;
        self
    }

    /// A × that calls `handler` (so do Backspace and Delete while it has
    /// focus).
    pub fn on_remove(mut self, handler: Handler<Dismissed>) -> Chip {
        spec!(self, Chip).removable = true;
        self.0.listen(Listen::Dismiss, handler);
        self
    }
}

/// A chip (a tag). With [`on_click`](Interactive::on_click) it can be
/// picked; with [`on_remove`](Chip::on_remove) removed.
///
/// ```ignore
/// chip("flaky").selected(self.flaky_only)
///     .on_click(cx.listener(|this, _, cx| { this.flaky_only ^= true; cx.notify(); }))
/// ```
pub fn chip(label: impl Into<String>) -> Chip {
    Chip(widget(wit::Element::Chip(wit::Chip { label: label.into(), icon: None, tone: None, selected: false, removable: false, disabled: false })))
}

/// A key or shortcut as Pitwall draws its own: `"cmd-k"` is ⌘K on macOS
/// and Ctrl+K elsewhere.
pub fn kbd(keys: impl Into<String>) -> Widget {
    Widget(widget(wit::Element::Kbd(keys.into())))
}

// ---- Layout -----------------------------------------------------------------

/// Which way something runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

impl Orientation {
    fn wit(self) -> wit::Orientation {
        match self {
            Orientation::Horizontal => wit::Orientation::Horizontal,
            Orientation::Vertical => wit::Orientation::Vertical,
        }
    }
}

/// Two panes with a bar between. Made by [`h_split`] and [`v_split`].
#[derive(Debug, Clone)]
pub struct Split(AnyElement);
builder!(Split);

impl Split {
    /// The first pane's share at first, 0.05–0.95 (default 0.5).
    pub fn initial(mut self, share: f32) -> Split {
        spec!(self, Split).initial = share;
        self
    }

    /// The panes' smallest sizes, in logical pixels (default 80 each).
    pub fn min_sizes(mut self, first: f32, second: f32) -> Split {
        let s = spec!(self, Split);
        s.min_first = first;
        s.min_second = second;
        self
    }

    /// The first pane.
    pub fn first(mut self, pane: impl IntoElement) -> Split {
        self.0.children.insert(0, pane.into_element());
        self
    }

    /// The second pane.
    pub fn second(mut self, pane: impl IntoElement) -> Split {
        self.0.children.push(pane.into_element());
        self
    }

    /// Call `handler` with the first pane's share (`&f64`) when the user
    /// ends a drag. Pitwall keeps their size for the view anyway.
    pub fn on_resize(mut self, handler: Handler<f64>) -> Split {
        self.0.listen(Listen::Value, handler);
        self
    }
}

fn split(o: Orientation) -> Split {
    Split(widget(wit::Element::Split(wit::Split { orientation: o.wit(), initial: 0.5, min_first: 80., min_second: 80. })))
}

/// Two panes side by side with a bar between them the user drags.
/// Pitwall keeps the user's size for the view, across restarts. Give it
/// a size (`.flex_1()`).
///
/// ```ignore
/// h_split().initial(0.35).min_sizes(180., 240.).flex_1()
///     .first(self.list(cx)).second(self.detail(cx))
/// ```
pub fn h_split() -> Split {
    split(Orientation::Horizontal)
}

/// Two panes one above the other with a bar between them.
pub fn v_split() -> Split {
    split(Orientation::Vertical)
}

/// A section that opens and closes. Made by [`disclosure`].
#[derive(Debug, Clone)]
pub struct Disclosure(AnyElement);
builder!(Disclosure);
parent!(Disclosure);

impl Disclosure {
    /// Muted text at the header's right (a count, a summary).
    pub fn detail(mut self, text: impl Into<String>) -> Disclosure {
        spec!(self, Disclosure).detail = Some(text.into());
        self
    }

    /// Call `handler` with the new state (`&bool`: open) when the header
    /// is clicked, or Enter or Space pressed on it.
    pub fn on_toggle(mut self, handler: Handler<bool>) -> Disclosure {
        self.0.listen(Listen::Value, handler);
        self
    }
}

/// A section with a header that opens and closes it; its children are
/// drawn under the header while `open`.
///
/// ```ignore
/// disclosure("Environment", self.env_open).detail("4 variables")
///     .on_toggle(cx.listener(|this, open: &bool, cx| { this.env_open = *open; cx.notify(); }))
///     .child(key_values().entry("CI", "true"))
/// ```
pub fn disclosure(title: impl Into<String>, open: bool) -> Disclosure {
    Disclosure(widget(wit::Element::Disclosure(wit::Disclosure { title: title.into(), detail: None, open })))
}

/// A card. Made by [`card`].
#[derive(Debug, Clone)]
pub struct Card(AnyElement);
builder!(Card);
parent!(Card);

impl Card {
    /// A title at its top.
    pub fn title(mut self, title: impl Into<String>) -> Card {
        spec!(self, Card).title = Some(title.into());
        self
    }

    /// Muted text at the title's right.
    pub fn detail(mut self, text: impl Into<String>) -> Card {
        spec!(self, Card).detail = Some(text.into());
        self
    }
}

/// A card: a bordered box on the raised surface. With
/// [`on_click`](Interactive::on_click) the whole card is clickable.
///
/// ```ignore
/// card().title("Last run").detail("main").child(text("Passed in 3 min"))
/// ```
pub fn card() -> Card {
    Card(widget(wit::Element::Card(wit::Card { title: None, detail: None })))
}

/// A thin line across its parent (between stacked things).
pub fn divider() -> Widget {
    Widget(widget(wit::Element::Divider(wit::Orientation::Horizontal)))
}

/// A thin upright line (between things in a row).
pub fn vertical_divider() -> Widget {
    Widget(widget(wit::Element::Divider(wit::Orientation::Vertical)))
}

/// Breadcrumbs. Made by [`breadcrumbs`].
#[derive(Debug, Clone)]
pub struct Breadcrumbs(AnyElement);
builder!(Breadcrumbs);

impl Breadcrumbs {
    /// Add a level: `key` (sent when it is clicked) and what it says. The
    /// last is where the user is.
    pub fn crumb(mut self, key: impl Into<String>, label: impl Into<String>) -> Breadcrumbs {
        spec!(self, Breadcrumbs).push(wit::Crumb { key: key.into(), label: label.into() });
        self
    }

    /// Call `handler` with the key (`&str`) of the level clicked.
    pub fn on_pick(mut self, handler: Handler<str>) -> Breadcrumbs {
        self.0.listen(Listen::Value, handler);
        self
    }
}

/// Breadcrumbs: where the user is, each level but the last clickable.
///
/// ```ignore
/// breadcrumbs().crumb("org", "acme").crumb("repo", "web").crumb("run", "#412")
///     .on_pick(cx.listener(|this, key: &str, cx| this.go_up(key, cx)))
/// ```
pub fn breadcrumbs() -> Breadcrumbs {
    Breadcrumbs(widget(wit::Element::Breadcrumbs(Vec::new())))
}

/// A toolbar row: Pitwall's bar (its height, its hairline under it) for
/// the buttons and fields among its children, in a row.
///
/// ```ignore
/// toolbar().child(search_field("q")).child(div().flex_1()).child(icon_button(Icon::Refresh, "Refresh"))
/// ```
pub fn toolbar() -> Widget {
    Widget(widget(wit::Element::Toolbar))
}

/// Where a popover opens from its anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// Under it.
    #[default]
    Below,
    /// Over it.
    Above,
    /// At its left.
    Start,
    /// At its right.
    End,
}

/// A popover. Made by [`popover`].
#[derive(Debug, Clone)]
pub struct Popover(AnyElement);
builder!(Popover);
parent!(Popover);

impl Popover {
    /// Where it opens (default: below).
    pub fn placement(mut self, p: Placement) -> Popover {
        spec!(self, Popover).placement = match p {
            Placement::Below => wit::Placement::Below,
            Placement::Above => wit::Placement::Above,
            Placement::Start => wit::Placement::Start,
            Placement::End => wit::Placement::End,
        };
        self
    }

    /// Its width in logical pixels (default: its content's, 160–480).
    pub fn width(mut self, w: f32) -> Popover {
        spec!(self, Popover).width = Some(w);
        self
    }

    /// Call `handler` when the user presses Esc or clicks outside it:
    /// leave it out of the next render.
    pub fn on_dismiss(mut self, handler: Handler<Dismissed>) -> Popover {
        self.0.listen(Listen::Dismiss, handler);
        self
    }
}

/// A popover: its children in a floating panel, anchored to the element
/// it is a child of (a box around the button that opens it) and drawn by
/// Pitwall inside the tile. Open while it is rendered; at most one at a
/// time.
///
/// ```ignore
/// div().child(button("Filters").on_click(cx.listener(|this, _, cx| { this.filters = true; cx.notify(); })))
///     .when(self.filters, |d| d.child(popover().on_dismiss(cx.listener(|this, _, cx| { this.filters = false; cx.notify(); }))
///         .child(checkbox("Only failures"))))
/// ```
pub fn popover() -> Popover {
    Popover(widget(wit::Element::Popover(wit::Popover { placement: wit::Placement::Below, width: None })))
}

/// A confirm sheet. Made by [`confirm_sheet`].
#[derive(Debug, Clone)]
pub struct ConfirmSheet(AnyElement);
builder!(ConfirmSheet);

impl ConfirmSheet {
    /// More about it.
    pub fn text(mut self, text: impl Into<String>) -> ConfirmSheet {
        spec!(self, ConfirmSheet).text = Some(text.into());
        self
    }

    /// The cancel button's label (default "Cancel").
    pub fn cancel_label(mut self, label: impl Into<String>) -> ConfirmSheet {
        spec!(self, ConfirmSheet).cancel = Some(label.into());
        self
    }

    /// A destructive action: the confirm button is red.
    pub fn danger(mut self) -> ConfirmSheet {
        spec!(self, ConfirmSheet).danger = true;
        self
    }

    /// Call `handler` when the user confirms (the button, or Enter).
    pub fn on_confirm(mut self, handler: Handler<Click>) -> ConfirmSheet {
        self.0.listen(Listen::Click, handler);
        self
    }

    /// Call `handler` when the user cancels (Cancel, Esc, or a click on
    /// the dimmed tile).
    pub fn on_cancel(mut self, handler: Handler<Dismissed>) -> ConfirmSheet {
        self.0.listen(Listen::Dismiss, handler);
        self
    }
}

/// A sheet over the tile asking the user to confirm `title`, with a
/// button labelled `confirm`: tiles can't open the system's dialogs, so
/// this is how a tile asks. Shown while it is rendered (put it anywhere
/// in the tree); the rest of the tile is dimmed and takes no input.
///
/// ```ignore
/// confirm_sheet("Delete 3 notes?", "Delete").danger().text("This can't be undone.")
///     .on_confirm(cx.listener(|this, _, cx| this.delete(cx)))
///     .on_cancel(cx.listener(|this, _, cx| { this.asking = false; cx.notify(); }))
/// ```
pub fn confirm_sheet(title: impl Into<String>, confirm: impl Into<String>) -> ConfirmSheet {
    ConfirmSheet(widget(wit::Element::ConfirmSheet(wit::ConfirmSheet { title: title.into(), text: None, confirm: confirm.into(), cancel: None, danger: false })))
}

// ---- Data display -------------------------------------------------------------

/// A key-value list. Made by [`key_values`].
#[derive(Debug, Clone)]
pub struct KeyValues(AnyElement);
builder!(KeyValues);

impl KeyValues {
    /// Add a line: a label and its value.
    pub fn entry(mut self, key: impl Into<String>, value: impl Into<String>) -> KeyValues {
        spec!(self, KeyValues).push(wit::KeyValue { key: key.into(), value: value.into(), mono: false, copyable: false });
        self
    }

    /// The last line's value in the monospace font (ids, hashes, paths).
    pub fn mono(mut self) -> KeyValues {
        if let Some(e) = spec!(self, KeyValues).last_mut() {
            e.mono = true;
        }
        self
    }

    /// The last line's value has a copy button.
    pub fn copyable(mut self) -> KeyValues {
        if let Some(e) = spec!(self, KeyValues).last_mut() {
            e.copyable = true;
        }
        self
    }
}

/// A list of labels and their values, aligned in two columns.
///
/// ```ignore
/// key_values().entry("Branch", "main").entry("Commit", "4f2a9c1").mono().copyable()
/// ```
pub fn key_values() -> KeyValues {
    KeyValues(widget(wit::Element::KeyValues(Vec::new())))
}

/// An avatar. Made by [`avatar`].
#[derive(Debug, Clone)]
pub struct Avatar(AnyElement);
builder!(Avatar);

impl Avatar {
    /// 20 px (default 28).
    pub fn small(mut self) -> Avatar {
        spec!(self, Avatar).size = wit::AvatarSize::Small;
        self
    }

    /// 40 px (default 28).
    pub fn large(mut self) -> Avatar {
        spec!(self, Avatar).size = wit::AvatarSize::Large;
        self
    }
}

/// An avatar: `name`'s initials in a circle, coloured from the theme by
/// the name (the same name, the same colour). `name` is its tooltip and
/// accessible name.
pub fn avatar(name: impl Into<String>) -> Avatar {
    Avatar(widget(wit::Element::Avatar(wit::Avatar { name: name.into(), size: wit::AvatarSize::Medium })))
}

/// A time drawn as "5 min ago", "yesterday" or a date, kept up to date by
/// Pitwall (no timer needed in the tile); its tooltip is the full date.
/// A time before 1970 is drawn as 1970.
///
/// ```ignore
/// h_flex().gap_1().child("Updated").child(relative_time(run.finished))
/// ```
pub fn relative_time(at: std::time::SystemTime) -> Widget {
    let ms = at.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    relative_time_ms(ms)
}

/// [`relative_time`] of a time in milliseconds since 1970-01-01 UTC.
pub fn relative_time_ms(ms: i64) -> Widget {
    Widget(widget(wit::Element::RelativeTime(wit::RelativeTime { at: ms })))
}

/// A copyable text. Made by [`copyable`].
#[derive(Debug, Clone)]
pub struct Copyable(AnyElement);
builder!(Copyable);

impl Copyable {
    /// In the monospace font.
    pub fn mono(mut self) -> Copyable {
        spec!(self, Copyable).mono = true;
        self
    }
}

/// `text` with a copy button: Pitwall writes it to the clipboard only
/// when the user clicks the button (or presses Enter on it), and says
/// "Copied". An [`on_click`](Interactive::on_click) hears it too.
///
/// ```ignore
/// copyable("4f2a9c1e").mono()
/// ```
pub fn copyable(text: impl Into<String>) -> Copyable {
    Copyable(widget(wit::Element::Copyable(wit::Copyable { text: text.into(), mono: false })))
}

/// A link. Made by [`link`].
#[derive(Debug, Clone)]
pub struct Link(AnyElement);
builder!(Link);

impl Link {
    /// The page it opens (https, or http on this computer), in a Pitwall
    /// web tile, when the tile has the `show` permission; without it the
    /// click goes to [`on_click`](Interactive::on_click).
    pub fn url(mut self, url: impl Into<String>) -> Link {
        spec!(self, Link).url = Some(url.into());
        self
    }
}

/// A link saying `label`. With [`url`](Link::url) and the `show`
/// permission it opens the page; otherwise its click is the tile's.
///
/// ```ignore
/// link("Open run #412").url("https://ci.example.com/runs/412")
/// link("Show all").on_click(cx.listener(|this, _, cx| this.show_all(cx)))
/// ```
pub fn link(label: impl Into<String>) -> Link {
    Link(widget(wit::Element::Link(wit::Link { label: label.into(), url: None })))
}

// ---- Charts -----------------------------------------------------------------

fn series(name: String, values: Vec<f64>, color: Option<Token>) -> wit::ChartSeries {
    wit::ChartSeries { name, values, color: color.map(Token::wit) }
}

/// A sparkline. Made by [`sparkline`].
#[derive(Debug, Clone)]
pub struct Sparkline(AnyElement);
builder!(Sparkline);

impl Sparkline {
    /// Its colour (default: the text colour).
    pub fn color(mut self, c: Token) -> Sparkline {
        spec!(self, Sparkline).color = Some(c.wit());
        self
    }

    /// Fill the area under the line, faintly.
    pub fn filled(mut self) -> Sparkline {
        spec!(self, Sparkline).filled = true;
        self
    }
}

/// A sparkline: a small line without axes, oldest value first (at most
/// 1 000), 80 × 20 unless sized.
///
/// ```ignore
/// sparkline(self.durations.clone()).color(Token::Good).filled()
/// ```
pub fn sparkline(values: impl IntoIterator<Item = f64>) -> Sparkline {
    Sparkline(widget(wit::Element::Sparkline(wit::Sparkline { values: values.into_iter().collect(), color: None, filled: false })))
}

/// A bar chart. Made by [`bar_chart`].
#[derive(Debug, Clone)]
pub struct BarChart(AnyElement);
builder!(BarChart);

impl BarChart {
    /// Add a series: its name and one value per label (at most 8 series).
    pub fn series(mut self, name: impl Into<String>, values: impl IntoIterator<Item = f64>) -> BarChart {
        spec!(self, BarChart).series.push(series(name.into(), values.into_iter().collect(), None));
        self
    }

    /// Add a series in colour `color`.
    pub fn series_colored(mut self, name: impl Into<String>, values: impl IntoIterator<Item = f64>, color: Token) -> BarChart {
        spec!(self, BarChart).series.push(series(name.into(), values.into_iter().collect(), Some(color)));
        self
    }

    /// Stack the series in one bar per label.
    pub fn stacked(mut self) -> BarChart {
        spec!(self, BarChart).stacked = true;
        self
    }

    /// The values' unit, in the axis and tooltips (`"ms"`, `"%"`).
    pub fn unit(mut self, unit: impl Into<String>) -> BarChart {
        spec!(self, BarChart).unit = Some(unit.into());
        self
    }
}

/// A bar chart over `labels` (at most 200), with a value axis, a legend
/// for more than one series, and a tooltip per bar. Give it a height.
///
/// ```ignore
/// bar_chart(["Mon", "Tue", "Wed"]).series("Passed", [12., 18., 9.]).series("Failed", [1., 0., 3.])
///     .stacked().h(px(160.))
/// ```
pub fn bar_chart<S: Into<String>>(labels: impl IntoIterator<Item = S>) -> BarChart {
    BarChart(widget(wit::Element::BarChart(wit::BarChart { labels: labels.into_iter().map(Into::into).collect(), series: Vec::new(), stacked: false, unit: None })))
}

/// A line chart. Made by [`line_chart`].
#[derive(Debug, Clone)]
pub struct LineChart(AnyElement);
builder!(LineChart);

impl LineChart {
    /// Add a series: its name and one value per label (at most 8; a NaN
    /// is a gap).
    pub fn series(mut self, name: impl Into<String>, values: impl IntoIterator<Item = f64>) -> LineChart {
        spec!(self, LineChart).series.push(series(name.into(), values.into_iter().collect(), None));
        self
    }

    /// Add a series in colour `color`.
    pub fn series_colored(mut self, name: impl Into<String>, values: impl IntoIterator<Item = f64>, color: Token) -> LineChart {
        spec!(self, LineChart).series.push(series(name.into(), values.into_iter().collect(), Some(color)));
        self
    }

    /// The values' unit, in the axis and tooltips.
    pub fn unit(mut self, unit: impl Into<String>) -> LineChart {
        spec!(self, LineChart).unit = Some(unit.into());
        self
    }

    /// The axis's lowest value (default: from the data, or 0).
    pub fn min(mut self, v: f64) -> LineChart {
        spec!(self, LineChart).min = Some(v);
        self
    }

    /// The axis's highest value (default: from the data).
    pub fn max(mut self, v: f64) -> LineChart {
        spec!(self, LineChart).max = Some(v);
        self
    }
}

/// A line chart over `labels` (at most 1 000), with axes and a tooltip
/// listing each series' value under the pointer. Give it a height.
///
/// ```ignore
/// line_chart(days).series("p50", p50).series("p95", p95).unit("ms").h(px(180.))
/// ```
pub fn line_chart<S: Into<String>>(labels: impl IntoIterator<Item = S>) -> LineChart {
    LineChart(widget(wit::Element::LineChart(wit::LineChart { labels: labels.into_iter().map(Into::into).collect(), series: Vec::new(), unit: None, min: None, max: None })))
}

/// A status timeline. Made by [`status_timeline`].
#[derive(Debug, Clone)]
pub struct StatusTimeline(AnyElement);
builder!(StatusTimeline);

impl StatusTimeline {
    /// Add a segment: `key` (sent when clicked), its tooltip and
    /// accessible name, and its colour by meaning (`None`: neutral).
    pub fn segment(self, key: impl Into<String>, label: impl Into<String>, tone: Option<Tone>) -> StatusTimeline {
        self.segment_weighted(key, label, tone, 1.)
    }

    /// Add a segment `weight` times as wide as one of weight 1 (a run's
    /// duration).
    pub fn segment_weighted(mut self, key: impl Into<String>, label: impl Into<String>, tone: Option<Tone>, weight: f32) -> StatusTimeline {
        spec!(self, StatusTimeline).segments.push(wit::TimelineSegment { key: key.into(), label: label.into(), tone: tone.map(Tone::wit), weight });
        self
    }

    /// Call `handler` with the key (`&str`) of the segment clicked.
    pub fn on_pick(mut self, handler: Handler<str>) -> StatusTimeline {
        self.0.listen(Listen::Value, handler);
        self
    }
}

/// A status timeline: a row of coloured segments, oldest first, each with
/// a tooltip (at most 500), as a CI tile shows its last runs.
///
/// ```ignore
/// status_timeline().segment("410", "#410 passed", Some(Tone::Good)).segment("411", "#411 failed", Some(Tone::Bad))
///     .on_pick(cx.listener(|this, run: &str, cx| this.open_run(run, cx)))
/// ```
pub fn status_timeline() -> StatusTimeline {
    StatusTimeline(widget(wit::Element::StatusTimeline(wit::StatusTimeline { segments: Vec::new() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(e: impl IntoElement) -> wit::Element {
        match e.into_element().kind {
            Kind::Widget(w) => w.0,
            _ => panic!("not a component"),
        }
    }

    #[test]
    fn builders_fill_their_records() {
        let wit::Element::Switch(c) = element(switch("On").checked(true).disabled(true)) else { panic!() };
        assert!(c.checked && c.disabled && c.label == "On");
        let wit::Element::Segmented(g) = element(segmented().option("a", "A").option("b", "B").option_disabled().selected("a")) else { panic!() };
        assert_eq!((g.items.len(), g.items[1].disabled, g.selected.as_deref()), (2, true, Some("a")));
        let wit::Element::Slider(s) = element(slider(5., 0.0..=10.0).step(0.5)) else { panic!() };
        assert_eq!((s.min, s.max, s.step), (0., 10., 0.5));
        let wit::Element::KeyValues(kv) = element(key_values().entry("a", "1").mono().entry("b", "2")) else { panic!() };
        assert!(kv[0].mono && !kv[1].mono);
        let wit::Element::BarChart(b) = element(bar_chart(["x", "y"]).series("s", [1., 2.]).stacked()) else { panic!() };
        assert!(b.stacked && b.series[0].values == [1., 2.]);
        let wit::Element::RelativeTime(t) = element(relative_time(std::time::UNIX_EPOCH + std::time::Duration::from_secs(2))) else { panic!() };
        assert_eq!(t.at, 2000);
    }
}
