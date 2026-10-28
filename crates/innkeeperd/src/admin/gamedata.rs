//! The portal's game data area: which of the Ybarra games' item and map files the operator has
//! imported from their copy, the import form, and the map viewer (docs/server/ybarra.md section 8).
//! The item viewer is in `item_list.rs`.

use std::fmt::Write as _;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use base64::Engine;

use super::html::{button, esc, narrator, when, window, Flash, Form, Nav, Tone};
use super::map_grid::{legend, map_grid, GridCells};
use super::store;
use super::views::{flag, link, table, tr};
use super::{Admin, Reply, Req};
use crate::dates;
use crate::ybarra::dungeon::Dungeon;
use crate::ybarra::game_file::{self, Resource};
use crate::ybarra::items::ItemTable;
use crate::ybarra::schema::schema;
use crate::ybarra::Game;

pub(crate) const GAME_DATA_IMPORT_PATH: &str = "/admin/gamedata/import";

pub(crate) fn resource_url(game: Game, resource: Resource) -> String {
    format!("/admin/gamedata/{}/{}", game.id(), resource.id())
}

/// The name a map has in the game's field map, or its number.
pub(crate) fn map_name(game: Game, map: usize) -> String {
    schema(game)
        .automap
        .maps
        .get(map)
        .map_or_else(|| format!("Map {}", map + 1), |entry| entry.name.clone())
}

impl Admin {
    /// A game's data file as imported, parsed; `None` when it was never imported.
    fn stored_resource(
        &self,
        game: Game,
        resource: Resource,
    ) -> Result<Option<Vec<u8>>, innkeeper_world::StoreError> {
        store::resource_for(&self.store, game, resource)
    }

    pub(super) fn game_items(
        &self,
        game: Game,
    ) -> Result<Option<ItemTable>, innkeeper_world::StoreError> {
        Ok(self
            .stored_resource(game, Resource::Items)?
            .and_then(|bytes| ItemTable::parse(&bytes).ok()))
    }

    pub(super) fn game_dungeon(
        &self,
        game: Game,
    ) -> Result<Option<Dungeon>, innkeeper_world::StoreError> {
        Ok(self
            .stored_resource(game, Resource::Maps)?
            .and_then(|bytes| Dungeon::parse(&bytes).ok()))
    }

    /// The page a viewer shows while its file is missing.
    pub(super) fn not_imported(&self, req: &Req, game: Game, resource: Resource) -> Reply {
        let body = narrator(
            Tone::Bad,
            &format!(
                "{}'s {} is not imported yet: import it from your copy of the game first.",
                game.title(),
                resource.file_name()
            ),
            Some("The shelf has a label but no book."),
        ) + &format!("<p>{}</p>", link("/admin/gamedata", "Import game files"));
        self.render(req, Nav::GameData, game.title(), None, &body)
    }

    /// `/admin/gamedata`: what is imported, and the import form.
    pub(crate) fn game_data(&self, req: &Req) -> Reply {
        self.game_data_with(req, None)
    }

    fn game_data_with(&self, req: &Req, flash: Option<&Flash>) -> Reply {
        let rows = match store::resources(&self.store) {
            Ok(rows) => rows,
            Err(error) => return self.failed(req, &error),
        };
        let mut lines = String::new();
        for game in Game::ALL {
            let mut cells = vec![esc(game.title())];
            for resource in Resource::ALL {
                let row = rows
                    .iter()
                    .find(|row| row.game == game && row.resource == resource);
                cells.push(match row {
                    Some(row) => format!(
                        "{} {} <span class=\"muted\">{}, sha256 <span class=\"mono\">{}</span></span>",
                        flag("ready", "ok"),
                        link(&resource_url(game, resource), resource.title()),
                        esc(&when(row.imported_at / 1000)),
                        esc(&row.source_sha256.chars().take(16).collect::<String>())
                    ),
                    None => flag("not imported", "warn"),
                });
            }
            lines.push_str(&tr(&cells));
        }
        let body = format!(
            "{}{}",
            window(
                "Game data",
                &format!(
                    "<p>The item and map files of the three Ybarra games, read from your copy. They are \
                     Sierra's data, so they are not shipped with the server; once imported they are kept \
                     in the database and shown here.</p>{}",
                    table(&["Game", "Items (RESOURCE.038)", "Maps (RESOURCE.035)"], &lines)
                )
            ),
            self.import_window(req),
        );
        self.render(req, Nav::GameData, "Game data", flash, &body)
    }

    fn import_window(&self, req: &Req) -> String {
        let form = Form {
            action: GAME_DATA_IMPORT_PATH,
            csrf: &req.ctx.csrf,
            hidden: &[],
            confirm: None,
            inline: false,
        };
        let mut games = String::new();
        for game in Game::ALL {
            let _ = write!(
                games,
                "<option value=\"{}\">{}</option>",
                game.id(),
                esc(game.title())
            );
        }
        window(
            "Import a game file",
            &format!(
                "{}<p>Pick <b>RESOURCE.038</b> (items) or <b>RESOURCE.035</b> (maps) from the game's folder \
                 (<span class=\"mono\">INN\\YSERBIUS</span>, <span class=\"mono\">INN\\TWINION</span> or \
                 <span class=\"mono\">INN\\CAWDOR</span>) and say which game it is from; the two files are \
                 told apart by their size.</p><div class=\"row\">\
                 <div><label for=\"data-game\">Game</label><select id=\"data-game\" name=\"game\">{games}</select></div>\
                 <div><label for=\"data-file\">Game file</label>\
                 <input type=\"file\" id=\"data-file\" data-file-into=\"file\" required></div></div>\
                 <input type=\"hidden\" name=\"file\" value=\"\">\
                 <p class=\"muted\">The file is read in the browser, so this form needs script. Without it, run \
                 <span class=\"mono\">innkeeperd admin ybarra-data --game GAME --data-dir DIR FILE</span> on the server.</p>\
                 {}</form>",
                form.open(),
                button("Import", "primary")
            ),
        )
    }

    /// `POST /admin/gamedata/import`: `game`, and the file as base64url in `file`.
    pub(crate) fn game_data_import(&self, req: &Req) -> Reply {
        let Some(game) = Game::from_id(req.param("game")) else {
            let flash = Flash::bad(
                "Nothing was imported: the form named no game.",
                "Whose book is this?",
            );
            return self.game_data_with(req, Some(&flash));
        };
        let bytes = match BASE64URL.decode(req.param("file").trim()) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            _ => {
                let flash = Flash::bad(
                    "No file came with the form. Pick it again (the form needs script), or use the command line.",
                    "The envelope was empty.",
                );
                return self.game_data_with(req, Some(&flash));
            }
        };
        let flash = match game_file::import(&self.store, game, &bytes, dates::now_ms()) {
            Ok(found) => {
                let hash = innkeeper_world::saves::hash_hex(&bytes);
                let target = format!(
                    "{} {} from sha256 {}",
                    game.id(),
                    found.resource.id(),
                    &hash[..16]
                );
                if let Err(error) = self.log_change(req, "ybarra.data", &target, None, None) {
                    return self.failed(req, &error);
                }
                Flash::good(
                    format!(
                        "{}'s {} are imported.",
                        game.title(),
                        found.resource.title().to_lowercase()
                    ),
                    "The clerk shelves the volume and dusts the spine.",
                )
            }
            Err(why) => Flash::bad(
                format!("Nothing was imported: {why}."),
                "The clerk turns the file over twice and hands it back.",
            ),
        };
        self.game_data_with(req, Some(&flash))
    }

    /// `/admin/gamedata/<game>/maps`, with `?map=N` for one map's grid.
    pub(crate) fn game_maps(&self, req: &Req, game: &str) -> Reply {
        let Some(game) = Game::from_id(game) else {
            return self.not_found(req);
        };
        let dungeon = match self.game_dungeon(game) {
            Ok(Some(dungeon)) => dungeon,
            Ok(None) => return self.not_imported(req, game, Resource::Maps),
            Err(error) => return self.failed(req, &error),
        };
        let url = resource_url(game, Resource::Maps);
        let chosen = req
            .param("map")
            .parse::<usize>()
            .ok()
            .and_then(|map| map.checked_sub(1))
            .filter(|map| *map < dungeon.maps.len())
            .unwrap_or(0);
        let mut rows = String::new();
        for (map, plan) in dungeon.maps.iter().enumerate() {
            let number = (map + 1).to_string();
            let name = map_name(game, map);
            rows.push_str(&tr(&[
                number.clone(),
                if map == chosen {
                    format!("<b>{}</b>", esc(&name))
                } else {
                    link(&format!("{url}?map={number}"), &name)
                },
                schema(game)
                    .automap
                    .maps
                    .get(map)
                    .map_or_else(String::new, |entry| {
                        format!("<span class=\"mono\">{:#06x}</span>", entry.code)
                    }),
                plan.used_cells().to_string(),
            ]));
        }
        let names_note = match schema(game).automap.names_confidence.as_deref() {
            Some("confirmed") | None => "",
            Some(_) => {
                " This game's map names are matched to blocks by position, which is inferred."
            }
        };
        let grid = dungeon.maps.get(chosen).map_or_else(String::new, |plan| {
            let name = map_name(game, chosen);
            let header: Vec<String> = plan.header.iter().map(|b| format!("{b:02x}")).collect();
            window(
                &name,
                &format!(
                    "{}{}<p class=\"muted\">Header bytes (meaning unknown): <span class=\"mono\">{}</span></p>",
                    map_grid(Some(plan), &name, &GridCells { inner: &|_| String::new(), marked: None }),
                    legend(None),
                    header.join(" ")
                ),
            )
        });
        let body = format!(
            "<p>{} &middot; {}</p>{grid}{}",
            link("/admin/gamedata", "All game data"),
            link(&resource_url(game, Resource::Items), "Items of this game"),
            window(
                "Maps",
                &format!(
                    "<p class=\"muted\">Used cells: cells with a wall or an event.{names_note}</p>{}",
                    table(&["#", "Map", "Code", "Used cells"], &rows)
                )
            ),
        );
        self.render(
            req,
            Nav::GameData,
            &format!("{}: maps", game.title()),
            None,
            &body,
        )
    }
}
