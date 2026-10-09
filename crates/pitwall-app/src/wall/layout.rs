//! The Wall's geometry, kept pure so it is tested without a window
//! (`src/styles/wall.css`): sections per project, a grid of tiles
//! `repeat(auto-fill, minmax(min(100%, 440px), 1fr))` with 10 px gaps, and
//! which rows are on (or within 200 px of) the screen, since only those
//! tiles are drawn and watched.

use std::collections::BTreeSet;

use pitwall_proto::{AgentView, Status};

use crate::agents::{status_word, ProjectGroup};

/// `.wall-grid` minimum column width and gap.
pub const TILE_MIN_W: f32 = 440.;
pub const GAP: f32 = 10.;
/// `.wall-scroll` padding: 6 px top, 12 px sides, 16 px bottom.
pub const PAD_TOP: f32 = 6.;
pub const PAD_X: f32 = 12.;
pub const PAD_BOTTOM: f32 = 16.;
/// `.wall-section-head`: 10 px + a 20 px line + 8 px.
pub const SECTION_HEAD_H: f32 = 38.;
/// `.wall-section .machine-head`: 6 px + a 14 px line + 4 px.
pub const MACHINE_HEAD_H: f32 = 24.;
/// `.wall-tile-head`.
pub const TILE_HEAD_H: f32 = 32.;
/// `.wall-tile-body`: 250 px, 300 px on the `xl` breakpoint.
pub const BODY_H: f32 = 250.;
pub const BODY_H_XL: f32 = 300.;
/// Window width of the `xl` breakpoint (`src/lib/useBreakpoint.ts`).
pub const XL_WIDTH: f32 = 2200.;
/// Tiles this far outside the viewport still count as visible.
pub const VISIBLE_MARGIN: f32 = 200.;

/// A tile's body height for a window this wide.
pub fn body_height(window_width: f32) -> f32 {
    if window_width >= XL_WIDTH {
        BODY_H_XL
    } else {
        BODY_H
    }
}

/// A tile's full height (header with its bottom line, body); its ring is
/// drawn outside.
pub fn tile_height(window_width: f32) -> f32 {
    TILE_HEAD_H + body_height(window_width)
}

/// Grid columns for a Wall this wide (its scroll area, padding included).
pub fn columns(width: f32) -> usize {
    let inner = (width - 2. * PAD_X).max(0.);
    (((inner + GAP) / (TILE_MIN_W + GAP)).floor() as usize).max(1)
}

/// One row of the Wall's scroll area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WallRow {
    /// A machine's name above its first project, when agents run on several.
    Machine(String),
    /// A project's heading (index into the groups).
    Head(usize),
    /// Tiles `start..end` of a group, and whether more rows of it follow.
    Tiles {
        group: usize,
        start: usize,
        end: usize,
        last: bool,
    },
}

/// `machineHeading`: the machine's label above its first group, only when
/// agents run on more than one machine.
pub fn machine_heading(groups: &[ProjectGroup], i: usize) -> Option<String> {
    let key = |g: &ProjectGroup| {
        g.machine
            .as_ref()
            .map(|m| format!("{}:{}", m.provider, m.id))
    };
    let keys: BTreeSet<String> = groups.iter().filter_map(key).collect();
    if keys.len() < 2 {
        return None;
    }
    let mine = key(&groups[i])?;
    let first = groups
        .iter()
        .position(|g| key(g).as_deref() == Some(mine.as_str()))?;
    (first == i)
        .then(|| groups[i].machine.as_ref().map(|m| m.label.clone()))
        .flatten()
}

/// "1 needs you · 2 idle", for collapsed sections (`summarize`).
pub fn summarize(agents: &[AgentView]) -> String {
    let order = [
        Status::Blocked,
        Status::Done,
        Status::Working,
        Status::Idle,
        Status::Unknown,
        Status::Exited,
        Status::Stopped,
    ];
    order
        .iter()
        .filter_map(|s| {
            let n = agents.iter().filter(|a| a.status == *s).count();
            (n > 0).then(|| format!("{n} {}", status_word(*s)))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The rows for these groups at `cols` columns, collapsed sections folded.
pub fn rows(groups: &[ProjectGroup], collapsed: &BTreeSet<String>, cols: usize) -> Vec<WallRow> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        if let Some(m) = machine_heading(groups, gi) {
            out.push(WallRow::Machine(m));
        }
        out.push(WallRow::Head(gi));
        if collapsed.contains(&g.key) {
            continue;
        }
        let n = g.agents.len();
        let mut start = 0;
        while start < n {
            let end = (start + cols).min(n);
            out.push(WallRow::Tiles {
                group: gi,
                start,
                end,
                last: end == n,
            });
            start = end;
        }
    }
    out
}

/// A row's height; tile rows carry the grid gap below them unless last.
pub fn row_height(row: &WallRow, tile_h: f32) -> f32 {
    match row {
        WallRow::Machine(_) => MACHINE_HEAD_H,
        WallRow::Head(_) => SECTION_HEAD_H,
        WallRow::Tiles { last, .. } => tile_h + if *last { 0. } else { GAP },
    }
}

/// Each row's top, from the top of the scroll content.
pub fn row_tops(rows: &[WallRow], tile_h: f32) -> Vec<f32> {
    let mut y = PAD_TOP;
    rows.iter()
        .map(|r| {
            let top = y;
            y += row_height(r, tile_h);
            top
        })
        .collect()
}

/// Which rows are within [`VISIBLE_MARGIN`] of a viewport `height` tall,
/// scrolled down by `scrolled`.
pub fn visible(rows: &[WallRow], tile_h: f32, scrolled: f32, height: f32) -> Vec<bool> {
    let lo = scrolled - VISIBLE_MARGIN;
    let hi = scrolled + height + VISIBLE_MARGIN;
    row_tops(rows, tile_h)
        .into_iter()
        .zip(rows)
        .map(|(top, r)| top + row_height(r, tile_h) >= lo && top <= hi)
        .collect()
}

/// A tile's place: (row index, column).
pub type Cell = (usize, usize);

/// Every tile's agent id and place, in reading order.
pub fn tile_cells(rows: &[WallRow], groups: &[ProjectGroup]) -> Vec<(String, Cell)> {
    let mut out = Vec::new();
    for (ri, r) in rows.iter().enumerate() {
        if let WallRow::Tiles {
            group, start, end, ..
        } = r
        {
            for (col, a) in groups[*group].agents[*start..*end].iter().enumerate() {
                out.push((a.id.clone(), (ri, col)));
            }
        }
    }
    out
}

/// Keyboard moves between tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    First,
    Last,
}

/// The tile a key moves to from `from` (or the first tile when none is
/// selected). Up and down keep the column, clamped to shorter rows; left and
/// right go through the tiles in reading order.
pub fn step(cells: &[(String, Cell)], from: Option<&str>, m: Move) -> Option<String> {
    if cells.is_empty() {
        return None;
    }
    let Some(i) = from.and_then(|id| cells.iter().position(|(a, _)| a == id)) else {
        return Some(match m {
            Move::Last | Move::Up | Move::Left => cells[cells.len() - 1].0.clone(),
            _ => cells[0].0.clone(),
        });
    };
    let (row, col) = cells[i].1;
    let pick = |row: usize| {
        let in_row: Vec<_> = cells.iter().filter(|(_, c)| c.0 == row).collect();
        in_row
            .iter()
            .find(|(_, c)| c.1 == col)
            .or(in_row.last())
            .map(|(a, _)| a.clone())
    };
    let rows: Vec<usize> = {
        let mut r: Vec<usize> = cells.iter().map(|(_, c)| c.0).collect();
        r.dedup();
        r
    };
    let at = rows.iter().position(|r| *r == row).unwrap_or(0);
    Some(match m {
        Move::Left => cells[i.saturating_sub(1)].0.clone(),
        Move::Right => cells[(i + 1).min(cells.len() - 1)].0.clone(),
        Move::First => cells[0].0.clone(),
        Move::Last => cells[cells.len() - 1].0.clone(),
        Move::Up => match at.checked_sub(1) {
            Some(prev) => pick(rows[prev])?,
            None => cells[i].0.clone(),
        },
        Move::Down => match rows.get(at + 1) {
            Some(next) => pick(*next)?,
            None => cells[i].0.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::group_by_project;
    use crate::agents::tests::agent;

    fn groups() -> Vec<ProjectGroup> {
        let mut a = Vec::new();
        for i in 0..5 {
            a.push(agent(&format!("a{i}"), "/work/alpha", "idle", i, true));
        }
        a.push(agent("b0", "/work/beta", "blocked", 0, true));
        a.push(agent("b1", "/work/beta", "done", 1, true));
        group_by_project(&a)
    }

    #[test]
    fn columns_follow_auto_fill_minmax() {
        assert_eq!(columns(300.), 1, "narrower than one tile: one column");
        assert_eq!(columns(440. + 24.), 1);
        assert_eq!(columns(889. + 24.), 1);
        assert_eq!(columns(890. + 24.), 2);
        assert_eq!(columns(1900.), 4);
        // The React Wall at the same window widths (sidebar 268 px, no right
        // panel in Wall mode): 1100 → 1, 1400 → 2, 2200 → 4 columns.
        let sidebar = 268.;
        assert_eq!(columns(1100. - sidebar), 1);
        assert_eq!(columns(1400. - sidebar), 2);
        assert_eq!(columns(2200. - sidebar), 4);
        assert_eq!(tile_height(1400.), 282.);
        assert_eq!(tile_height(2400.), 332.);
    }

    #[test]
    fn rows_break_groups_into_grid_rows() {
        let g = groups();
        let r = rows(&g, &BTreeSet::new(), 2);
        assert_eq!(r.len(), 1 + 3 + 1 + 1);
        assert_eq!(r[0], WallRow::Head(0));
        assert_eq!(
            r[3],
            WallRow::Tiles {
                group: 0,
                start: 4,
                end: 5,
                last: true
            }
        );
        assert!(matches!(r[1], WallRow::Tiles { last: false, .. }));
        // Collapsed sections keep their heading only.
        let collapsed = BTreeSet::from([g[0].key.clone()]);
        let r = rows(&g, &collapsed, 2);
        assert_eq!(
            r,
            vec![
                WallRow::Head(0),
                WallRow::Head(1),
                WallRow::Tiles {
                    group: 1,
                    start: 0,
                    end: 2,
                    last: true
                }
            ]
        );
    }

    #[test]
    fn machine_headings_only_with_several_machines() {
        let mut a = vec![
            agent("x", "/work/alpha", "idle", 0, true),
            agent("y", "/work/beta", "idle", 0, true),
        ];
        assert_eq!(machine_heading(&group_by_project(&a), 0), None);
        a.push(agent("vm", "/srv/gamma", "idle", 0, false));
        let g = group_by_project(&a);
        assert_eq!(machine_heading(&g, 0).as_deref(), Some("This Mac"));
        assert_eq!(
            machine_heading(&g, 1),
            None,
            "only above the machine's first group"
        );
        assert_eq!(machine_heading(&g, 2).as_deref(), Some("vm-1"));
        assert_eq!(
            rows(&g, &BTreeSet::new(), 1)[0],
            WallRow::Machine("This Mac".into())
        );
    }

    #[test]
    fn summaries_count_by_status() {
        let g = groups();
        assert_eq!(summarize(&g[1].agents), "1 needs you · 1 done");
        assert_eq!(summarize(&g[0].agents), "5 idle");
    }

    #[test]
    fn only_rows_near_the_viewport_are_visible() {
        let g = groups();
        let r = rows(&g, &BTreeSet::new(), 1);
        let th = tile_height(1000.);
        // Head, 5 tiles of alpha, head, 2 of beta.
        let tops = row_tops(&r, th);
        assert_eq!(tops[0], PAD_TOP);
        assert_eq!(tops[1], PAD_TOP + SECTION_HEAD_H);
        assert_eq!(tops[2], tops[1] + th + GAP);
        let v = visible(&r, th, 0., 600.);
        assert_eq!(
            v.iter().take_while(|x| **x).count(),
            4,
            "600 px + 200 px margin"
        );
        assert!(v[4..].iter().all(|x| !x));
        let v = visible(&r, th, tops[6], 300.);
        assert!(v[..5].iter().all(|x| !x));
        assert!(v[5..].iter().all(|x| *x));
    }

    #[test]
    fn keys_move_through_the_grid() {
        let g = groups();
        let r = rows(&g, &BTreeSet::new(), 2);
        let cells = tile_cells(&r, &g);
        let ids: Vec<_> = cells.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(ids, ["a0", "a1", "a2", "a3", "a4", "b0", "b1"]);
        assert_eq!(step(&cells, None, Move::Right).as_deref(), Some("a0"));
        assert_eq!(step(&cells, None, Move::Up).as_deref(), Some("b1"));
        assert_eq!(step(&cells, Some("a1"), Move::Down).as_deref(), Some("a3"));
        assert_eq!(
            step(&cells, Some("a3"), Move::Down).as_deref(),
            Some("a4"),
            "clamped to a shorter row"
        );
        assert_eq!(
            step(&cells, Some("a4"), Move::Down).as_deref(),
            Some("b0"),
            "into the next section"
        );
        assert_eq!(
            step(&cells, Some("a1"), Move::Up).as_deref(),
            Some("a1"),
            "top row stays"
        );
        assert_eq!(step(&cells, Some("a1"), Move::Right).as_deref(), Some("a2"));
        assert_eq!(step(&cells, Some("a0"), Move::Left).as_deref(), Some("a0"));
        assert_eq!(step(&cells, Some("b1"), Move::Right).as_deref(), Some("b1"));
        assert_eq!(step(&cells, Some("a2"), Move::Last).as_deref(), Some("b1"));
        assert_eq!(
            step(&cells, Some("gone"), Move::Down).as_deref(),
            Some("a0")
        );
        assert_eq!(step(&[], None, Move::Down), None);
    }
}
