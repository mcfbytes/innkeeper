//! The map grid control: a map's 16 by 16 cells with their walls, passable walls and events, as
//! one table. The game data viewer shows it as it is; the character editor's warp page puts a
//! button in each cell (docs/server/ybarra.md section 8.3).

use std::fmt::Write as _;

use super::html::esc;
use crate::ybarra::automap::SIDE;
use crate::ybarra::dungeon::{Cell, Edge, MapPlan, Side, SIDE_ORDER_INFERRED};

/// The arrow a facing value is drawn as, in `SIDE_ORDER_INFERRED` order.
const ARROWS: [&str; 4] = ["\u{2190}", "\u{2193}", "\u{2192}", "\u{2191}"];

pub(crate) fn arrow(facing: u8) -> &'static str {
    ARROWS.get(usize::from(facing)).copied().unwrap_or("?")
}

/// What the grid draws in a cell besides its walls.
pub(crate) struct GridCells<'a> {
    /// The cell's inner HTML, given its index (`row * 16 + col`).
    pub(crate) inner: &'a dyn Fn(usize) -> String,
    /// A cell to highlight, such as a character's saved position.
    pub(crate) marked: Option<usize>,
}

fn side_letter(side: Side) -> char {
    match side {
        Side::West => 'w',
        Side::South => 's',
        Side::East => 'e',
        Side::North => 'n',
    }
}

fn classes(cell: &Cell) -> String {
    let mut out = String::new();
    for side in SIDE_ORDER_INFERRED {
        let kind = match cell.edge(side) {
            Edge::Open => continue,
            Edge::Wall(_) => 'w',
            Edge::Passable(_) => 'd',
        };
        let _ = write!(out, " {kind}{}", side_letter(side));
    }
    if cell.has_event() {
        out.push_str(" ev");
    }
    if cell.walls.iter().any(|wall| *wall != 0) || cell.has_event() {
        out.push_str(" used");
    }
    out
}

/// The cell's facts in words, for its tooltip and for screen readers.
pub(crate) fn describe(cell: &Cell) -> String {
    let mut parts = Vec::new();
    for side in SIDE_ORDER_INFERRED {
        let index = side.index();
        let kind = cell.side_kinds.get(index).copied().unwrap_or(0);
        let edge = match cell.edge(side) {
            Edge::Open if kind == 0 => continue,
            Edge::Open => format!("{} open (side kind {kind})", side.name()),
            Edge::Wall(wall) if kind == 0 => format!("{} wall {wall}", side.name()),
            Edge::Wall(wall) => format!("{} wall {wall} (side kind {kind})", side.name()),
            Edge::Passable(wall) => format!("{} passable wall {wall}", side.name()),
        };
        parts.push(edge);
    }
    for (side, event) in SIDE_ORDER_INFERRED.iter().zip(cell.side_events) {
        if event != 0 {
            parts.push(format!("{} side event {}", side.name(), event - 1));
        }
    }
    if cell.enter_event != 0 {
        parts.push(format!("entry event {}", cell.enter_event - 1));
    }
    if cell.second_event != 0 {
        parts.push(format!("second event {}", cell.second_event - 1));
    }
    if cell.kind != 0 {
        parts.push(format!("cell byte {}", cell.kind));
    }
    parts.join(", ")
}

/// The grid as a table. Without a plan (the maps file not imported) it is a bare 16 by 16.
pub(crate) fn map_grid(plan: Option<&MapPlan>, caption: &str, cells: &GridCells<'_>) -> String {
    let mut out = format!(
        "<div class=\"tablewrap\"><table class=\"mapgrid\"><caption class=\"sr\">{}</caption><thead><tr><th></th>",
        esc(caption)
    );
    for col in 0..SIDE {
        let _ = write!(out, "<th scope=\"col\">{col}</th>");
    }
    out.push_str("</tr></thead><tbody>");
    for row in 0..SIDE {
        let _ = write!(out, "<tr><th scope=\"row\">{row}</th>");
        for col in 0..SIDE {
            let index = row * SIDE + col;
            let cell = plan.and_then(|plan| plan.cell(index));
            let mut class = cell.map_or_else(String::new, classes);
            if cells.marked == Some(index) {
                class.push_str(" here");
            }
            let facts = cell.map(describe).filter(|text| !text.is_empty());
            let title = match facts {
                Some(facts) => format!("row {row}, column {col}: {facts}"),
                None => format!("row {row}, column {col}"),
            };
            let _ = write!(
                out,
                "<td class=\"{}\" title=\"{}\">{}</td>",
                class.trim_start(),
                esc(&title),
                (cells.inner)(index)
            );
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    out
}

/// The key to the grid's marks; `marked` names the highlighted cell, when there is one.
pub(crate) fn legend(marked: Option<&str>) -> String {
    let marked = marked.map_or_else(String::new, |text| {
        format!(" <span class=\"key here\"></span> {}", esc(text))
    });
    format!(
        "<p class=\"muted maplegend\"><span class=\"key wall\"></span> wall \
         <span class=\"key door\"></span> passable wall (a door or secret way; inferred) \
         <span class=\"key ev\"></span> script event{marked}. North is up (inferred); hover a cell \
         for its bytes.</p>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ybarra::dungeon::tests::stored_file;
    use crate::ybarra::dungeon::Dungeon;

    #[test]
    fn walls_doors_events_and_the_mark_become_classes() {
        let dungeon = Dungeon::parse(&stored_file(4, 3)).unwrap();
        let inner = |index: usize| {
            if index == 0x22 {
                "<b>x</b>".to_owned()
            } else {
                String::new()
            }
        };
        let html = map_grid(
            dungeon.maps.first(),
            "Map <1>",
            &GridCells {
                inner: &inner,
                marked: Some(0x22),
            },
        );
        assert!(
            html.contains("<caption class=\"sr\">Map &lt;1&gt;</caption>"),
            "{html}"
        );
        assert!(html.contains("class=\"ww wn used\""), "{html}");
        assert!(
            html.contains("class=\"dn used\" title=\"row 1, column 2: north passable wall 2\""),
            "{html}"
        );
        assert!(
            html.contains(
                "ev used here\" title=\"row 2, column 2: south wall 1, entry event 3\"><b>x</b>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("<td").count(), 256);
        let bare = map_grid(
            None,
            "m",
            &GridCells {
                inner: &|_| String::new(),
                marked: None,
            },
        );
        assert!(
            bare.contains("<td class=\"\" title=\"row 15, column 15\"></td>"),
            "{bare}"
        );
        assert_eq!(arrow(3), "\u{2191}");
    }
}
