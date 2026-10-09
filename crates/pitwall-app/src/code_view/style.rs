//! The code view's colours: Monaco's vs-dark / vs token and diff colours
//! with Pitwall's surfaces, as `src/components/review/review.css` sets them
//! (`--rv-*`), for the dark and light schemes; and its font metrics.

use gpui::{px, rgba, Font, FontFeatures, FontWeight, Hsla, Pixels};

use super::highlight::Token;
use crate::kit::MONO_FONT;
use crate::theme::{Mode, Theme};

/// Editor text size and row height (review.css: `12.5px/19px`).
pub const FONT_SIZE: Pixels = px(12.5);
pub const LINE_HEIGHT: Pixels = px(19.);

pub fn mono_font() -> Font {
    Font {
        family: MONO_FONT.into(),
        // No ligatures (review.css: "liga" 0, "calt" 0).
        features: FontFeatures::disable_ligatures(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
    }
}

/// `#rrggbbaa`
fn c(hex: u32) -> Hsla {
    rgba(hex).into()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Colors {
    pub bg: Hsla,
    pub text: Hsla,
    pub gutter_text: Hsla,
    pub gutter_active: Hsla,
    pub ins_line: Hsla,
    pub del_line: Hsla,
    pub ins_char: Hsla,
    pub del_char: Hsla,
    pub ins_gutter: Hsla,
    pub del_gutter: Hsla,
    pub fill: Hsla,
    pub ov_ins: Hsla,
    pub ov_del: Hsla,
    pub slider: Hsla,
    pub slider_hover: Hsla,
    pub slider_active: Hsla,
    pub guide: Hsla,
    pub guide_active: Hsla,
    pub cursor: Hsla,
    pub sel: Hsla,
    pub sel_inactive: Hsla,
    pub bracket_bg: Hsla,
    pub bracket_border: Hsla,
    pub find: Hsla,
    pub find_cur: Hsla,
    pub lane_cursor: Hsla,
    pub lane_find: Hsla,
    pub fold_bg: Hsla,
    pub fold_text: Hsla,
    pub commented: Hsla,
    pub widget_fg: Hsla,
    pub widget_bg: Hsla,
    pub widget_border: Hsla,
    pub input_bg: Hsla,
    pub focus: Hsla,
    pub opt_bg: Hsla,
    pub error: Hsla,
    pub menu_bg: Hsla,
    pub menu_fg: Hsla,
    pub menu_sel: Hsla,
    pub menu_sep: Hsla,
    pub glyph_add_bg: Hsla,
    pub glyph_add_fg: Hsla,
    brackets: [Hsla; 3],
    stray: Hsla,
    keyword: Hsla,
    string: Hsla,
    number: Hsla,
    comment: Hsla,
    ty: Hsla,
    delimiter: Hsla,
    regexp: Hsla,
    tag: Hsla,
    attr: Hsla,
    variable: Hsla,
}

impl Colors {
    pub fn for_theme(t: &Theme) -> Colors {
        let dark = t.mode == Mode::Dark;
        let pick = |d: u32, l: u32| if dark { c(d) } else { c(l) };
        Colors {
            bg: t.term_bg,
            text: t.text,
            gutter_text: t.text_4,
            gutter_active: t.text_3,
            ins_line: pick(0x3fd07f14, 0x14a05214),
            del_line: pick(0xff5f5f14, 0xd63a3a12),
            ins_char: pick(0x3fd07f26, 0x14a05224),
            del_char: pick(0xff5f5f26, 0xd63a3a22),
            ins_gutter: pick(0x3fd07f22, 0x14a05222),
            del_gutter: pick(0xff5f5f22, 0xd63a3a20),
            fill: pick(0xcccccc33, 0x22222233),
            ov_ins: pick(0x48b97b59, 0x3ea56d59),
            ov_del: pick(0xdf616459, 0xcd5e5f59),
            slider: pick(0xffffff14, 0x00000014),
            slider_hover: pick(0xffffff24, 0x00000024),
            slider_active: pick(0xbfbfbf66, 0x00000099),
            guide: pick(0x404040ff, 0xd3d3d3ff),
            guide_active: pick(0x707070ff, 0x939393ff),
            cursor: pick(0xaeafadff, 0x000000ff),
            sel: pick(0x264f78ff, 0xadd6ffff),
            sel_inactive: pick(0x3a3d41ff, 0xe5ebf1ff),
            bracket_bg: c(0x0064001a),
            bracket_border: pick(0x888888ff, 0xb9b9b9ff),
            find: c(0xea5c0054),
            find_cur: pick(0x515c6aff, 0xa8ac94ff),
            lane_cursor: c(0xa0a0a0cc),
            lane_find: c(0xd186167e),
            fold_bg: t.surface_2,
            fold_text: t.text_3,
            commented: t.text.opacity(0.06),
            widget_fg: pick(0xccccccff, 0x616161ff),
            widget_bg: t.surface_2,
            widget_border: t.line,
            input_bg: pick(0x3c3c3cff, 0xffffffff),
            focus: pick(0x007fd4ff, 0x0090f1ff),
            opt_bg: pick(0x007fd466, 0x007fd433),
            error: pick(0xf48771ff, 0xa1260dff),
            menu_bg: pick(0x3c3c3cff, 0xffffffff),
            menu_fg: pick(0xf0f0f0ff, 0x616161ff),
            menu_sel: pick(0x2a2d2eff, 0xf0f0f0ff),
            menu_sep: pick(0xcccccc33, 0x61616133),
            glyph_add_bg: t.text_2,
            glyph_add_fg: t.bg,
            brackets: if dark {
                [c(0xffd700ff), c(0xda70d6ff), c(0x179fffff)]
            } else {
                [c(0x0431faff), c(0x319331ff), c(0x7b3814ff)]
            },
            stray: c(0xff1212cc),
            keyword: pick(0x569cd6ff, 0x0000ffff),
            string: pick(0xce9178ff, 0xa31515ff),
            number: pick(0xb5cea8ff, 0x098658ff),
            comment: pick(0x6a9955ff, 0x008000ff),
            ty: pick(0x3dc9b0ff, 0x008080ff),
            delimiter: pick(0xdcdcdcff, 0x000000ff),
            regexp: pick(0xb46695ff, 0x811f3fff),
            tag: pick(0x569cd6ff, 0x800000ff),
            attr: pick(0x9cdcfeff, 0xe50000ff),
            variable: pick(0x74b0dfff, 0x001188ff),
        }
    }

    pub fn token(&self, t: Token) -> Hsla {
        match t {
            Token::Keyword => self.keyword,
            Token::String => self.string,
            Token::Number => self.number,
            Token::Comment => self.comment,
            Token::Type => self.ty,
            Token::Delimiter => self.delimiter,
            Token::Regexp => self.regexp,
            Token::Tag => self.tag,
            Token::Attr => self.attr,
            Token::Bracket(l) => self.brackets[(l % 3) as usize],
            Token::StrayBracket => self.stray,
            Token::Variable => self.variable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_follow_the_scheme() {
        let d = Colors::for_theme(&Theme::dark());
        let l = Colors::for_theme(&Theme::light());
        assert_ne!(d.keyword, l.keyword);
        assert_eq!(d.bg, Theme::dark().term_bg);
        assert!((d.ins_line.a - 0x14 as f32 / 255.0).abs() < 0.01);
        assert_ne!(d.token(Token::Bracket(0)), d.token(Token::Bracket(1)));
        assert_eq!(d.token(Token::Bracket(3)), d.token(Token::Bracket(0)));
    }

    #[test]
    fn the_review_css_still_has_these_colours() {
        let css = include_str!("../../../../src/components/review/review.css");
        for v in [
            "#3fd07f14",
            "#569cd6",
            "#ce9178",
            "#264f78",
            "#da70d6",
            "#0000ff",
            "#74b0df",
            "#001188",
            "#a31515",
        ] {
            assert!(
                css.contains(v),
                "{v} is gone from review.css: update style.rs"
            );
        }
    }
}
