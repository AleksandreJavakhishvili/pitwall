//! Styles: lengths, Pitwall's theme tokens, text sizes and the [`Styled`]
//! builder methods every element has. The names follow GPUI's (`flex`,
//! `gap_2`, `p_3`, `size_full`, `text_color`, `bg`), so GPUI code ports by
//! search and replace.

use crate::bindings::pitwall::tile::types as wit;

/// A length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// Logical pixels (they scale with the display).
    Px(f32),
    /// Multiples of Pitwall's base text size, so the length follows the
    /// user's font size. The spacing scale (`gap_2`, `p_3`) is in rems.
    Rems(f32),
    /// A fraction of the parent's size, from 0.0 to 1.0.
    Relative(f32),
    /// Whatever the content needs.
    Auto,
}

/// `n` logical pixels.
pub fn px(n: f32) -> Length {
    Length::Px(n)
}

/// `n` times Pitwall's base text size.
pub fn rems(n: f32) -> Length {
    Length::Rems(n)
}

/// A fraction of the parent's size (`relative(0.5)`: half).
pub fn relative(f: f32) -> Length {
    Length::Relative(f)
}

impl From<f32> for Length {
    /// A bare number is pixels, as in GPUI.
    fn from(n: f32) -> Length {
        Length::Px(n)
    }
}

impl Length {
    pub(crate) fn wit(self) -> wit::Length {
        match self {
            Length::Px(n) => wit::Length::Px(n),
            Length::Rems(n) => wit::Length::Rem(n),
            Length::Relative(f) => wit::Length::Fraction(f),
            Length::Auto => wit::Length::Auto,
        }
    }
}

/// A colour of Pitwall's theme, by meaning. Each has a light and a dark
/// value; the user's theme decides which is drawn. There are no raw
/// colours: a tile always looks native.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Token {
    /// The window's background, behind tiles.
    Bg,
    /// A tile's own background.
    Surface,
    /// A raised area inside a tile: a header, a selected row.
    Surface2,
    /// A further raised area: a hovered selected row, a field.
    Surface3,
    /// Floating surfaces: menus, popovers.
    Raised,
    /// Hairlines and borders.
    Line,
    /// Stronger borders: a focused field, a divider that must be seen.
    LineStrong,
    /// Main text.
    Text,
    /// Secondary text: details, labels.
    Text2,
    /// Muted text: hints, timestamps.
    Text3,
    /// Faint text: placeholders, disabled.
    Text4,
    /// The focus ring.
    Focus,
    /// Success: passed, running well.
    Good,
    /// A soft background for good things.
    GoodSoft,
    /// Needs attention.
    Warn,
    /// A soft background for warnings.
    WarnSoft,
    /// Failed, destructive.
    Bad,
    /// A soft background for failures.
    BadSoft,
    /// Finished.
    Done,
    /// A soft background for finished things.
    DoneSoft,
    /// The background of an added line in a diff.
    DiffAddBg,
    /// The background of a removed line in a diff.
    DiffDelBg,
}

impl Token {
    pub(crate) fn wit(self) -> wit::Token {
        match self {
            Token::Bg => wit::Token::Bg,
            Token::Surface => wit::Token::Surface,
            Token::Surface2 => wit::Token::Surface2,
            Token::Surface3 => wit::Token::Surface3,
            Token::Raised => wit::Token::Raised,
            Token::Line => wit::Token::Line,
            Token::LineStrong => wit::Token::LineStrong,
            Token::Text => wit::Token::Text,
            Token::Text2 => wit::Token::Text2,
            Token::Text3 => wit::Token::Text3,
            Token::Text4 => wit::Token::Text4,
            Token::Focus => wit::Token::Focus,
            Token::Good => wit::Token::Good,
            Token::GoodSoft => wit::Token::GoodSoft,
            Token::Warn => wit::Token::Warn,
            Token::WarnSoft => wit::Token::WarnSoft,
            Token::Bad => wit::Token::Bad,
            Token::BadSoft => wit::Token::BadSoft,
            Token::Done => wit::Token::Done,
            Token::DoneSoft => wit::Token::DoneSoft,
            Token::DiffAddBg => wit::Token::DiffAddBg,
            Token::DiffDelBg => wit::Token::DiffDelBg,
        }
    }
}

/// How children line up across a flex box's main direction (and how a node
/// lines up in its parent, for `self_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// At the start.
    Start,
    /// Centred.
    Center,
    /// At the end.
    End,
    /// Stretched to fill.
    Stretch,
    /// Along their text's baseline.
    Baseline,
}

impl Align {
    fn wit(self) -> wit::Align {
        match self {
            Align::Start => wit::Align::Start,
            Align::Center => wit::Align::Center,
            Align::End => wit::Align::End,
            Align::Stretch => wit::Align::Stretch,
            Align::Baseline => wit::Align::Baseline,
        }
    }
}

/// The styles of one node, as the builder methods set them. Padding,
/// margin and border widths are kept per side, so `p_2().px_4()` gives
/// 8 px top and bottom and 16 px left and right.
#[derive(Debug, Clone, Default)]
pub struct StyleSet {
    padding: Sides,
    margin: Sides,
    border: Sides,
    rest: Vec<wit::Style>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Sides {
    top: Option<Length>,
    right: Option<Length>,
    bottom: Option<Length>,
    left: Option<Length>,
}

impl Sides {
    fn is_set(&self) -> bool {
        self.top.is_some() || self.right.is_some() || self.bottom.is_some() || self.left.is_some()
    }

    fn wit(&self) -> wit::Edges {
        let l = |s: Option<Length>| s.unwrap_or(Length::Px(0.0)).wit();
        wit::Edges { top: l(self.top), right: l(self.right), bottom: l(self.bottom), left: l(self.left) }
    }
}

impl StyleSet {
    fn push(&mut self, s: wit::Style) {
        self.rest.push(s);
    }

    pub(crate) fn wit(&self) -> Vec<wit::Style> {
        let mut out = Vec::with_capacity(self.rest.len() + 3);
        if self.padding.is_set() {
            out.push(wit::Style::Padding(self.padding.wit()));
        }
        if self.margin.is_set() {
            out.push(wit::Style::Margin(self.margin.wit()));
        }
        if self.border.is_set() {
            out.push(wit::Style::Border(self.border.wit()));
        }
        out.extend(self.rest.iter().cloned());
        out
    }
}

/// The spacing scale, as GPUI's: `1` is a quarter of the base text size
/// (4 px at 16 px text).
fn step(n: f32) -> Length {
    Length::Rems(n * 0.25)
}

macro_rules! spacing {
    ($($(#[$doc:meta])* $name:ident => $field:ident [$($side:ident),+] $n:expr;)*) => {
        $(
            $(#[$doc])*
            fn $name(mut self) -> Self where Self: Sized {
                let s = &mut self.style().$field;
                $( s.$side = Some(step($n)); )+
                self
            }
        )*
    };
}

/// The style methods every element has. Later calls win over earlier ones
/// of the same kind.
pub trait Styled {
    /// The node's styles (for the methods below).
    fn style(&mut self) -> &mut StyleSet;

    // ---- Layout -------------------------------------------------------

    /// Lay children out in a row (GPUI's `flex()` defaults to a row too).
    fn flex(self) -> Self
    where
        Self: Sized,
    {
        self.flex_row()
    }

    /// Lay children out in a row.
    fn flex_row(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::FlexRow);
        self
    }

    /// Lay children out in a column (a box's default).
    fn flex_col(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::FlexCol);
        self
    }

    /// Wrap children onto more lines when they don't fit.
    fn flex_wrap(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Wrap);
        self
    }

    /// Take a share of the free space and give it up when short (grow and
    /// shrink 1).
    fn flex_1(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Grow(1.0));
        self.style().push(wit::Style::Shrink(1.0));
        self
    }

    /// Take this share of the free space along the parent's direction.
    fn flex_grow_by(mut self, share: f32) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Grow(share));
        self
    }

    /// Take a share (1) of the free space along the parent's direction.
    fn flex_grow(self) -> Self
    where
        Self: Sized,
    {
        self.flex_grow_by(1.0)
    }

    /// Never shrink below its content's size.
    fn flex_shrink_0(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Shrink(0.0));
        self
    }

    /// Neither grow nor shrink.
    fn flex_none(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Grow(0.0));
        self.style().push(wit::Style::Shrink(0.0));
        self
    }

    /// Space between children.
    fn gap(mut self, gap: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Gap(gap.into().wit()));
        self
    }

    /// No space between children.
    fn gap_0(self) -> Self
    where
        Self: Sized,
    {
        self.gap(px(0.0))
    }
    /// 2 px between children.
    fn gap_0p5(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(0.5))
    }
    /// 4 px between children.
    fn gap_1(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(1.0))
    }
    /// 6 px between children.
    fn gap_1p5(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(1.5))
    }
    /// 8 px between children.
    fn gap_2(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(2.0))
    }
    /// 12 px between children.
    fn gap_3(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(3.0))
    }
    /// 16 px between children.
    fn gap_4(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(4.0))
    }
    /// 20 px between children.
    fn gap_5(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(5.0))
    }
    /// 24 px between children.
    fn gap_6(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(6.0))
    }
    /// 32 px between children.
    fn gap_8(self) -> Self
    where
        Self: Sized,
    {
        self.gap(step(8.0))
    }

    /// Children line up across the main direction like this.
    fn items(mut self, align: Align) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::AlignItems(align.wit()));
        self
    }
    /// Children line up at the start, across the main direction.
    fn items_start(self) -> Self
    where
        Self: Sized,
    {
        self.items(Align::Start)
    }
    /// Children are centred across the main direction.
    fn items_center(self) -> Self
    where
        Self: Sized,
    {
        self.items(Align::Center)
    }
    /// Children line up at the end, across the main direction.
    fn items_end(self) -> Self
    where
        Self: Sized,
    {
        self.items(Align::End)
    }
    /// Children line up along their text's baseline.
    fn items_baseline(self) -> Self
    where
        Self: Sized,
    {
        self.items(Align::Baseline)
    }

    /// This node lines up in its parent like this, over the parent's
    /// `items_*`.
    fn self_align(mut self, align: Align) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::SelfAlign(align.wit()));
        self
    }

    /// Children packed at the start of the main direction.
    fn justify_start(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Justify(wit::Justify::Start));
        self
    }
    /// Children packed in the centre of the main direction.
    fn justify_center(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Justify(wit::Justify::Center));
        self
    }
    /// Children packed at the end of the main direction.
    fn justify_end(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Justify(wit::Justify::End));
        self
    }
    /// The first child at the start, the last at the end, equal space
    /// between.
    fn justify_between(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Justify(wit::Justify::SpaceBetween));
        self
    }
    /// Equal space around each child.
    fn justify_around(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Justify(wit::Justify::SpaceAround));
        self
    }

    // ---- Size ---------------------------------------------------------

    /// Width.
    fn w(mut self, w: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Width(w.into().wit()));
        self
    }
    /// Height.
    fn h(mut self, h: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Height(h.into().wit()));
        self
    }
    /// Width and height.
    fn size(self, s: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        let s = s.into();
        self.w(s).h(s)
    }
    /// The parent's whole width.
    fn w_full(self) -> Self
    where
        Self: Sized,
    {
        self.w(relative(1.0))
    }
    /// The parent's whole height.
    fn h_full(self) -> Self
    where
        Self: Sized,
    {
        self.h(relative(1.0))
    }
    /// The parent's whole width and height.
    fn size_full(self) -> Self
    where
        Self: Sized,
    {
        self.w_full().h_full()
    }
    /// Smallest width.
    fn min_w(mut self, w: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::MinWidth(w.into().wit()));
        self
    }
    /// Largest width.
    fn max_w(mut self, w: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::MaxWidth(w.into().wit()));
        self
    }
    /// Smallest height.
    fn min_h(mut self, h: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::MinHeight(h.into().wit()));
        self
    }
    /// Largest height.
    fn max_h(mut self, h: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::MaxHeight(h.into().wit()));
        self
    }

    // ---- Padding and margin ---------------------------------------------

    /// Padding on every side.
    fn p(mut self, l: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        let l = Some(l.into());
        let s = &mut self.style().padding;
        (s.top, s.right, s.bottom, s.left) = (l, l, l, l);
        self
    }
    /// Margin on every side.
    fn m(mut self, l: impl Into<Length>) -> Self
    where
        Self: Sized,
    {
        let l = Some(l.into());
        let s = &mut self.style().margin;
        (s.top, s.right, s.bottom, s.left) = (l, l, l, l);
        self
    }

    spacing! {
        /// No padding.
        p_0 => padding [top, right, bottom, left] 0.0;
        /// 4 px padding on every side.
        p_1 => padding [top, right, bottom, left] 1.0;
        /// 6 px padding on every side.
        p_1p5 => padding [top, right, bottom, left] 1.5;
        /// 8 px padding on every side.
        p_2 => padding [top, right, bottom, left] 2.0;
        /// 12 px padding on every side.
        p_3 => padding [top, right, bottom, left] 3.0;
        /// 16 px padding on every side.
        p_4 => padding [top, right, bottom, left] 4.0;
        /// 24 px padding on every side.
        p_6 => padding [top, right, bottom, left] 6.0;
        /// 4 px padding left and right.
        px_1 => padding [left, right] 1.0;
        /// 8 px padding left and right.
        px_2 => padding [left, right] 2.0;
        /// 12 px padding left and right.
        px_3 => padding [left, right] 3.0;
        /// 16 px padding left and right.
        px_4 => padding [left, right] 4.0;
        /// 4 px padding top and bottom.
        py_1 => padding [top, bottom] 1.0;
        /// 8 px padding top and bottom.
        py_2 => padding [top, bottom] 2.0;
        /// 12 px padding top and bottom.
        py_3 => padding [top, bottom] 3.0;
        /// 4 px padding on top.
        pt_1 => padding [top] 1.0;
        /// 8 px padding on top.
        pt_2 => padding [top] 2.0;
        /// 4 px padding at the bottom.
        pb_1 => padding [bottom] 1.0;
        /// 8 px padding at the bottom.
        pb_2 => padding [bottom] 2.0;
        /// 8 px padding on the left.
        pl_2 => padding [left] 2.0;
        /// 8 px padding on the right.
        pr_2 => padding [right] 2.0;
        /// 4 px margin on every side.
        m_1 => margin [top, right, bottom, left] 1.0;
        /// 8 px margin on every side.
        m_2 => margin [top, right, bottom, left] 2.0;
        /// 8 px margin left and right.
        mx_2 => margin [left, right] 2.0;
        /// 8 px margin top and bottom.
        my_2 => margin [top, bottom] 2.0;
        /// 8 px margin on top.
        mt_2 => margin [top] 2.0;
        /// 8 px margin at the bottom.
        mb_2 => margin [bottom] 2.0;
    }

    // ---- Borders, corners, colours -----------------------------------------

    /// A 1 px border on every side.
    fn border_1(mut self) -> Self
    where
        Self: Sized,
    {
        let l = Some(px(1.0));
        let s = &mut self.style().border;
        (s.top, s.right, s.bottom, s.left) = (l, l, l, l);
        self
    }
    /// A 1 px border on top.
    fn border_t_1(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().border.top = Some(px(1.0));
        self
    }
    /// A 1 px border at the bottom.
    fn border_b_1(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().border.bottom = Some(px(1.0));
        self
    }
    /// A 1 px border on the left.
    fn border_l_1(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().border.left = Some(px(1.0));
        self
    }
    /// A 1 px border on the right.
    fn border_r_1(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().border.right = Some(px(1.0));
        self
    }
    /// The border's colour (`Token::Line` for hairlines).
    fn border_color(mut self, t: Token) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::BorderColor(t.wit()));
        self
    }
    /// Square corners.
    fn rounded_none(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Radius(wit::Radius::Square));
        self
    }
    /// Small corners (4 px).
    fn rounded_sm(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Radius(wit::Radius::Sm));
        self
    }
    /// The usual corners (6 px).
    fn rounded_md(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Radius(wit::Radius::Md));
        self
    }
    /// Large corners (10 px).
    fn rounded_lg(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Radius(wit::Radius::Lg));
        self
    }
    /// Fully round: pills and circles.
    fn rounded_full(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Radius(wit::Radius::Full));
        self
    }
    /// Background colour.
    fn bg(mut self, t: Token) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Background(t.wit()));
        self
    }
    /// Background while the pointer is over the node.
    fn hover_bg(mut self, t: Token) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::HoverBackground(t.wit()));
        self
    }
    /// Text and icon colour, inherited by children.
    fn text_color(mut self, t: Token) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Color(t.wit()));
        self
    }
    /// Opacity, from 0.0 (invisible) to 1.0.
    fn opacity(mut self, o: f32) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Opacity(o));
        self
    }

    // ---- Overflow -----------------------------------------------------

    /// Content larger than the box is cut off at its edges.
    fn overflow_hidden(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::OverflowX(wit::Overflow::Hidden));
        self.style().push(wit::Style::OverflowY(wit::Overflow::Hidden));
        self
    }

    // ---- Text -----------------------------------------------------------

    /// The smallest text: badges, counts.
    fn text_xs(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::TextSize(wit::TextSize::Xs));
        self
    }
    /// Small text: details, labels.
    fn text_sm(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::TextSize(wit::TextSize::Sm));
        self
    }
    /// The body text size.
    fn text_base(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::TextSize(wit::TextSize::Md));
        self
    }
    /// Headings inside a tile.
    fn text_lg(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::TextSize(wit::TextSize::Lg));
        self
    }
    /// Large headings.
    fn text_xl(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::TextSize(wit::TextSize::Xl));
        self
    }
    /// The monospace font, for code and numbers that line up.
    fn font_mono(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Font(wit::Font::Mono));
        self
    }
    /// The interface font (the default).
    fn font_ui(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Font(wit::Font::Ui));
        self
    }
    /// Regular weight.
    fn font_normal(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Weight(wit::Weight::Normal));
        self
    }
    /// Slightly heavier text: labels.
    fn font_medium(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Weight(wit::Weight::Medium));
        self
    }
    /// Heading weight.
    fn font_semibold(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Weight(wit::Weight::Semibold));
        self
    }
    /// Strong emphasis.
    fn font_bold(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Weight(wit::Weight::Bold));
        self
    }
    /// Italic text.
    fn italic(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Italic);
        self
    }
    /// Underlined text.
    fn underline(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Underline);
        self
    }
    /// One line, cut with an ellipsis when too long.
    fn truncate(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Truncate);
        self
    }
    /// At most `lines` lines, the last one cut with an ellipsis.
    fn line_clamp(mut self, lines: u8) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::LineClamp(lines));
        self
    }
    /// The user can select and copy this text.
    fn selectable(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Selectable);
        self
    }

    // ---- Cursor -----------------------------------------------------------

    /// A hand over the node: something clickable.
    fn cursor_pointer(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Cursor(wit::Cursor::Pointer));
        self
    }
    /// A text cursor over the node: selectable text.
    fn cursor_text(mut self) -> Self
    where
        Self: Sized,
    {
        self.style().push(wit::Style::Cursor(wit::Cursor::Text));
        self
    }
}
