//! Per-tile font auto-shrink (React `PaneView.tsx` `TerminalSlot`,
//! `src/layout/density.ts` `autoTileFont`): each pane's terminal shows the
//! largest whole font, not above the Settings font and not below 10 px,
//! at which it still holds the density's minimum cells. Panes too small
//! even at 10 px fold into chips (`fold_min`), so tiles shrink first.
//! A tile's own ⌘+ / ⌘− size wins over this ([`MainScreen::tile_font`]).

use std::collections::HashMap;

use crate::theme::{clamp_font, Density, PANE_CHROME};

use super::tree::{pane_rects, Node, Rect};

/// The auto font of every agent shown in `tree` laid out in `area` (w, h).
pub fn auto_fonts(
    tree: &Node,
    area: (f64, f64),
    density: Density,
    base: f64,
) -> HashMap<String, f64> {
    let base = clamp_font(base.round() as i32);
    let agents: HashMap<String, String> = tree
        .leaves()
        .into_iter()
        .filter_map(|p| Some((p.id, p.agent_id?)))
        .collect();
    let full = Rect {
        x: 0.,
        y: 0.,
        w: area.0,
        h: area.1,
    };
    pane_rects(tree, full)
        .into_iter()
        .filter_map(|(pane, r)| {
            let agent = agents.get(&pane)?.clone();
            let w = (r.w - PANE_CHROME.0).max(0.) as f32;
            let h = (r.h - PANE_CHROME.1).max(0.) as f32;
            Some((agent, density.auto_tile_font(w, h, base) as f64))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FONT_FLOOR;

    fn pane(id: &str, agent: &str) -> Node {
        Node::Pane {
            id: id.into(),
            agent_id: Some(agent.into()),
        }
    }

    #[test]
    fn tiles_shrink_to_the_floor_before_folding() {
        let one = pane("p1", "a");
        // Roomy: the base font.
        let f = auto_fonts(&one, (2000., 1200.), Density::Compact, 13.);
        assert_eq!(f["a"], 13.);
        // Compact needs 60 columns: 60 · 0.6 · f + 28 px, so 11 px needs 424.
        let f = auto_fonts(&one, (420., 1200.), Density::Compact, 13.);
        assert_eq!(f["a"], 10.);
        let f = auto_fonts(&one, (430., 1200.), Density::Compact, 13.);
        assert_eq!(f["a"], 11.);
        // Never below the floor (the layout folds such panes away).
        let f = auto_fonts(&one, (100., 100.), Density::Compact, 13.);
        assert_eq!(f["a"], FONT_FLOOR as f64);
        // A base font under the floor stays as it is.
        let f = auto_fonts(&one, (100., 100.), Density::Compact, 9.);
        assert_eq!(f["a"], 9.);
    }

    #[test]
    fn empty_panes_have_no_font() {
        let empty = Node::Pane {
            id: "p1".into(),
            agent_id: None,
        };
        assert!(auto_fonts(&empty, (800., 600.), Density::Dense, 13.).is_empty());
    }
}
