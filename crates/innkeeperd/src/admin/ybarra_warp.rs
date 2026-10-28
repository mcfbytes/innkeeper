//! The character editor's warp page: pick a map, click a tile, and the character's saved position
//! (map, cell, facing) changes through the slot form's own save path (docs/server/ybarra.md 8.3).

use std::fmt::Write as _;

use hyper::StatusCode;

use super::gamedata::map_name;
use super::html::{esc, narrator, window, Flash, Form, Nav, Tone};
use super::map_grid::{arrow, legend, map_grid, GridCells};
use super::views::{account_link, flag, link};
use super::ybarra::{reason_input, Loaded, Opened};
use super::{Admin, Reply, Req};
use crate::ybarra::automap::SIDE;
use crate::ybarra::dungeon::{SIDE_ORDER, SIDE_ORDER_INFERRED};
use crate::ybarra::schema::{schema, Field, Value};
use crate::ybarra::{Game, SLOTS};

/// The record fields a warp writes, by their names in the field map.
const MAP_FIELD: &str = "map_code";
const CELL_FIELD: &str = "position";
const FACING_FIELD: &str = "facing";

/// Where a field sits in the field map, and the field.
fn field(game: Game, name: &str) -> Option<(usize, &'static Field)> {
    schema(game)
        .record
        .iter()
        .enumerate()
        .find(|(_, field)| field.name == name)
}

/// A character's saved position as its record holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Position {
    code: i64,
    cell: usize,
    facing: u8,
}

fn read_number(game: Game, name: &str, record: &[u8]) -> Option<i64> {
    match field(game, name)?.1.read(record, 0) {
        Value::Number(number) => Some(number),
        _ => None,
    }
}

fn position(game: Game, record: &[u8]) -> Option<Position> {
    Some(Position {
        code: read_number(game, MAP_FIELD, record)?,
        cell: usize::try_from(read_number(game, CELL_FIELD, record)?).ok()?,
        facing: u8::try_from(read_number(game, FACING_FIELD, record)?).ok()?,
    })
}

/// The map table index of a stored map code.
fn map_of(game: Game, code: i64) -> Option<usize> {
    schema(game)
        .automap
        .maps
        .iter()
        .position(|entry| i64::from(entry.code) == code)
}

fn facing_name(facing: u8) -> String {
    SIDE_ORDER_INFERRED.get(usize::from(facing)).map_or_else(
        || format!("value {facing}"),
        |side| format!("{} {}", arrow(facing), side.name()),
    )
}

fn where_text(game: Game, at: Position) -> String {
    let map = map_of(game, at.code).map_or_else(
        || format!("unknown map code {:#06x}", at.code),
        |map| map_name(game, map),
    );
    format!(
        "{map}, row {}, column {}, facing {}",
        at.cell / SIDE,
        at.cell % SIDE,
        facing_name(at.facing)
    )
}

fn map_chooser(url: &str, game: Game, chosen: usize) -> String {
    let mut options = String::new();
    for (index, entry) in schema(game).automap.maps.iter().enumerate() {
        let selected = if index == chosen { " selected" } else { "" };
        let _ = write!(
            options,
            "<option value=\"{}\"{selected}>{} ({:#06x})</option>",
            index + 1,
            esc(&entry.name),
            entry.code
        );
    }
    format!(
        "<form method=\"get\" action=\"{}\" class=\"row\"><div><label for=\"warp-map\">Map</label>\
         <select id=\"warp-map\" name=\"map\">{options}</select></div>\
         <button type=\"submit\" class=\"abtn\">Show this map</button></form>",
        esc(url)
    )
}

fn facing_select(name: &str, current: u8) -> String {
    let mut options = String::new();
    for value in 0..4u8 {
        let selected = if value == current { " selected" } else { "" };
        let _ = write!(
            options,
            "<option value=\"{value}\"{selected}>{} ({value})</option>",
            esc(&facing_name(value))
        );
    }
    format!(
        "<div><label for=\"warp-facing\">Facing {}</label><select id=\"warp-facing\" name=\"{name}\">{options}</select></div>",
        flag(SIDE_ORDER.word(), "warn")
    )
}

impl Admin {
    /// `/admin/saves/<account>/ybarra/<game>/<slot>/warp`, with `?map=N` for the map to show.
    pub(crate) fn ybarra_warp(&self, req: &Req, account: &str, game: &str, slot: &str) -> Reply {
        let (Ok(account), Some(game), Some(slot)) = (
            account.parse::<u32>(),
            Game::from_id(game),
            slot.parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .filter(|n| *n < SLOTS),
        ) else {
            return self.not_found(req);
        };
        match self.load_chars(req, account, game) {
            Loaded::Ready(opened) => {
                self.warp_page(req, account, &opened, slot, None, StatusCode::OK)
            }
            Loaded::Page(page) => page,
        }
    }

    pub(super) fn warp_page(
        &self,
        req: &Req,
        account: u32,
        opened: &Opened,
        slot: usize,
        flash: Option<&Flash>,
        status: StatusCode,
    ) -> Reply {
        let game = opened.game;
        let base = Self::editor_base(account, game);
        let title = format!("{}: warp character slot {}", game.title(), slot + 1);
        let fields = (
            field(game, MAP_FIELD),
            field(game, CELL_FIELD),
            field(game, FACING_FIELD),
        );
        let (Some((map_index, _)), Some((cell_index, _)), Some((facing_index, _))) = fields else {
            let body = narrator(Tone::Bad, "This game's field map does not name the position fields, so there is nothing to warp.", None);
            return self.render_status(req, status, Nav::Saves, &title, flash, &body);
        };
        let record = opened
            .chars
            .slots
            .get(slot)
            .map(|r| r.as_slice())
            .unwrap_or(&[]);
        let Some(at) = position(game, record) else {
            return self.not_found(req);
        };
        let maps = &schema(game).automap.maps;
        let chosen = req
            .param("map")
            .parse::<usize>()
            .ok()
            .and_then(|map| map.checked_sub(1))
            .filter(|map| *map < maps.len())
            .or_else(|| map_of(game, at.code))
            .unwrap_or(0);
        let Some(entry) = maps.get(chosen) else {
            let body = narrator(
                Tone::Bad,
                "This game's field map lists no maps, so there is nothing to warp to.",
                None,
            );
            return self.render_status(req, status, Nav::Saves, &title, flash, &body);
        };
        let plan = match self.game_dungeon(game) {
            Ok(dungeon) => dungeon.and_then(|dungeon| dungeon.maps.get(chosen).cloned()),
            Err(error) => return self.failed(req, &error),
        };
        let name = map_name(game, chosen);
        let action = format!("{base}/{}", slot + 1);
        let version = opened.version.to_string();
        let number = (chosen + 1).to_string();
        let code = entry.code.to_string();
        let map_input = format!("f{map_index}_0");
        let hidden = [
            ("version", version.as_str()),
            ("view", "warp"),
            ("map", number.as_str()),
            (map_input.as_str(), code.as_str()),
        ];
        let form = Form {
            action: &action,
            csrf: &req.ctx.csrf,
            hidden: &hidden,
            confirm: None,
            inline: false,
        };
        let here = (map_of(game, at.code) == Some(chosen)).then_some(at.cell);
        let cell_input = format!("f{cell_index}_0");
        let inner = |index: usize| {
            let label = if here == Some(index) {
                arrow(at.facing)
            } else {
                ""
            };
            format!(
                "<button type=\"submit\" name=\"{cell_input}\" value=\"{index}\" aria-label=\"Warp to row {}, column {}\">{label}</button>",
                index / SIDE,
                index % SIDE
            )
        };
        let walls_note = if plan.is_some() {
            String::new()
        } else {
            format!(
                "<p class=\"muted\">This game's maps file is not imported, so the grid has no walls. {}</p>",
                link("/admin/gamedata", "Import it")
            )
        };
        let online = if self.online_for(account).is_empty() {
            String::new()
        } else {
            narrator(
                Tone::Bad,
                "This account is online now, so a warp would be refused.",
                Some("Kick them or wait until they log off."),
            )
        };
        let grid = map_grid(
            plan.as_ref(),
            &name,
            &GridCells {
                inner: &inner,
                marked: here,
            },
        );
        let body = format!(
            "<p>{} &middot; {} &middot; account {} &middot; {} v{}</p>{online}\
             <p>Saved position: <b>{}</b>.</p>{}\
             {}<button type=\"submit\" disabled hidden aria-hidden=\"true\"></button>\
             <p class=\"muted\">Clicking a cell saves at once: the record's map code, cell and facing, as a new \
             version of the file, checked and audited like the slot form.</p>\
             <div class=\"row\">{}{}</div>{walls_note}{}</form>",
            link(&format!("{base}/{}", slot + 1), "Back to the slot form"),
            link(&base, "All characters in this file"),
            account_link(account),
            esc(game.chars_path()),
            opened.version,
            esc(&where_text(game, at)),
            map_chooser(&format!("{base}/{}/warp", slot + 1), game, chosen),
            form.open(),
            facing_select(&format!("f{facing_index}_0"), at.facing),
            reason_input(),
            window(&name, &format!("{grid}{}", legend(Some("the saved position")))),
        );
        self.render_status(req, status, Nav::Saves, &title, flash, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_names_the_position_fields_and_their_maps() {
        for game in Game::ALL {
            for name in [MAP_FIELD, CELL_FIELD, FACING_FIELD] {
                assert!(field(game, name).is_some(), "{} {name}", game.id());
            }
            let mut record = vec![0u8; crate::ybarra::SLOT_BYTES];
            let first = schema(game).automap.maps[2].code;
            record[0xF2..0xF4].copy_from_slice(&first.to_le_bytes());
            record[0xF4] = 0x3A;
            record[0xF5] = 3;
            let at = position(game, &record).unwrap();
            assert_eq!(map_of(game, at.code), Some(2));
            assert!(
                where_text(game, at).ends_with("row 3, column 10, facing \u{2191} north"),
                "{}",
                where_text(game, at)
            );
        }
    }
}
