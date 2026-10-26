//! The automap pages of the character editor: a character slot's 34 maps with how much of each is
//! explored, a grid of one map's cells, and the forms that forget a map, set its explored cells or
//! replace its 800 bytes (docs/server/ybarra.md section 6). Each change is a new save version.

use std::fmt::Write as _;

use hyper::StatusCode;
use innkeeper_world::AccountId;

use super::html::{button, esc, narrator, window, Confirm, Flash, Form, Nav, Tone};
use super::views::{account_link, link, table, tr};
use super::{Admin, Reply, Req};
use crate::dates;
use crate::ybarra::automap::{self, AutomapError, BLOCK_BYTES, CELLS, SIDE, UNEXPLORED};
use crate::ybarra::schema::{schema, Value};
use crate::ybarra::{Game, SLOTS};

/// The stored automap file of a game, with its version.
struct Maps {
    version: u64,
    data: Vec<u8>,
}

struct Target {
    account: u32,
    game: Game,
    slot: usize,
}

impl Target {
    fn parse(account: &str, game: &str, slot: &str) -> Option<Target> {
        let slot: usize = slot.parse().ok()?;
        Some(Target {
            account: account.parse().ok()?,
            game: Game::from_id(game)?,
            slot: slot.checked_sub(1).filter(|slot| *slot < SLOTS)?,
        })
    }

    fn url(&self) -> String {
        format!(
            "{}/{}/map",
            Admin::editor_base(self.account, self.game),
            self.slot + 1
        )
    }

    fn map_name(&self, map: usize) -> String {
        schema(self.game)
            .automap
            .maps
            .get(map)
            .map_or_else(|| format!("Map {}", map + 1), |entry| entry.name.clone())
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (n, byte) in bytes.iter().enumerate() {
        let gap = if n == 0 {
            ""
        } else if n % 16 == 0 {
            "\n"
        } else {
            " "
        };
        let _ = write!(out, "{gap}{byte:02x}");
    }
    out
}

fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    digits
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .filter(|pair| pair.len() == 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}

impl Admin {
    fn load_maps(&self, req: &Req, target: &Target) -> Result<Maps, Box<Reply>> {
        let found = self.settings.world.store.save_read(
            AccountId(target.account),
            target.game.automaps_path(),
            None,
        );
        match found {
            Ok(Some((meta, data))) if data.len() == automap::FILE_BYTES => Ok(Maps {
                version: meta.version,
                data,
            }),
            Ok(Some((_, data))) => {
                let body = narrator(
                    Tone::Bad,
                    &format!(
                        "The stored {} is {} bytes, not {}; it is not an automap file this editor knows.",
                        target.game.automaps_path(),
                        data.len(),
                        automap::FILE_BYTES
                    ),
                    Some("The atlas is the wrong size for the shelf."),
                );
                Err(Box::new(self.render(
                    req,
                    Nav::Saves,
                    target.game.title(),
                    None,
                    &body,
                )))
            }
            Ok(None) => {
                let body = narrator(
                    Tone::Plain,
                    &format!(
                        "This account has no {} stored, so there is no automap to edit.",
                        target.game.automaps_path()
                    ),
                    Some("The cartographer has not been told where anyone went."),
                ) + &format!(
                    "<p>{}</p>",
                    link(
                        &Admin::editor_base(target.account, target.game),
                        "Back to the characters"
                    )
                );
                Err(Box::new(self.render(
                    req,
                    Nav::Saves,
                    target.game.title(),
                    None,
                    &body,
                )))
            }
            Err(error) => Err(Box::new(self.failed(req, &error))),
        }
    }

    /// The character's name, when the character file can be opened, for the page title.
    fn slot_label(&self, target: &Target) -> String {
        let named = super::store::cipher_for(&self.store, target.game)
            .ok()
            .flatten()
            .and_then(|cipher| {
                let (_, data) = self
                    .settings
                    .world
                    .store
                    .save_read(AccountId(target.account), target.game.chars_path(), None)
                    .ok()??;
                let chars = crate::ybarra::CharFile::open(&cipher, &data).ok()?;
                let record = chars.slots.get(target.slot)?;
                let field = schema(target.game)
                    .record
                    .iter()
                    .find(|f| f.name == "name")?;
                match field.read(record, 0) {
                    Value::Text(name) if !name.is_empty() => Some(name),
                    _ => None,
                }
            });
        named.map_or_else(
            || format!("slot {}", target.slot + 1),
            |name| format!("slot {} ({name})", target.slot + 1),
        )
    }

    /// `/admin/saves/<account>/ybarra/<game>/<slot>/map`, with `?map=N` for one map's grid.
    pub(crate) fn ybarra_maps(&self, req: &Req, account: &str, game: &str, slot: &str) -> Reply {
        let Some(target) = Target::parse(account, game, slot) else {
            return self.not_found(req);
        };
        let maps = match self.load_maps(req, &target) {
            Ok(maps) => maps,
            Err(page) => return *page,
        };
        self.maps_page(req, &target, &maps, None, StatusCode::OK)
    }

    fn maps_page(
        &self,
        req: &Req,
        target: &Target,
        maps: &Maps,
        flash: Option<&Flash>,
        status: StatusCode,
    ) -> Reply {
        let chosen = req
            .param("map")
            .parse::<usize>()
            .ok()
            .and_then(|map| map.checked_sub(1))
            .filter(|map| *map < automap::MAPS);
        let url = target.url();
        let version = maps.version.to_string();
        let mut rows = String::new();
        for map in 0..automap::MAPS {
            let Ok(bytes) = automap::map_bytes(&maps.data, target.slot, map) else {
                continue;
            };
            let number = (map + 1).to_string();
            let hidden = [
                ("version", version.as_str()),
                ("map", number.as_str()),
                ("action", "forget"),
            ];
            let name = target.map_name(map);
            let question = format!("Forget everything about {name}?");
            let form = Form {
                action: &url,
                csrf: &req.ctx.csrf,
                hidden: &hidden,
                confirm: Some(Confirm {
                    question: &question,
                    detail: "Every cell and wall of this map becomes unexplored.",
                    yes: "Forget it",
                }),
                inline: true,
            };
            rows.push_str(&tr(&[
                number.clone(),
                esc(&name),
                schema(target.game)
                    .automap
                    .maps
                    .get(map)
                    .map_or_else(String::new, |entry| {
                        format!("<span class=\"mono\">{:#06x}</span>", entry.code)
                    }),
                format!("{} of {CELLS}", automap::explored(bytes)),
                link(&format!("{url}?map={number}"), "Cells"),
                super::html::action_form(&form, "Forget", ""),
            ]));
        }
        let all_hidden = [("version", version.as_str()), ("action", "forget_all")];
        let all_form = Form {
            action: &url,
            csrf: &req.ctx.csrf,
            hidden: &all_hidden,
            confirm: Some(Confirm {
                question: "Forget every map of this character?",
                detail: "All 34 maps become unexplored, as for a new character.",
                yes: "Forget all",
            }),
            inline: false,
        };
        let names_note = match schema(target.game).automap.names_confidence.as_deref() {
            Some("confirmed") | None => "",
            Some(_) => " Map names are matched to blocks by position, which is inferred.",
        };
        let label = self.slot_label(target);
        let grid = match chosen {
            Some(map) => self.grid_html(req, target, maps, map),
            None => String::new(),
        };
        let online = if self.online_for(target.account).is_empty() {
            String::new()
        } else {
            narrator(
                Tone::Bad,
                "This account is online now, so a change would be refused.",
                Some("Kick them or wait until they log off."),
            )
        };
        let body = format!(
            "<p>{} &middot; account {} &middot; {} v{}</p>{online}{grid}{}{}",
            link(
                &Admin::editor_base(target.account, target.game),
                "All characters in this file"
            ),
            account_link(target.account),
            esc(target.game.automaps_path()),
            maps.version,
            window(
                "Maps",
                &format!(
                    "<p class=\"muted\">Explored cells per map.{names_note}</p>{}",
                    table(&["#", "Map", "Code", "Explored", "", ""], &rows)
                )
            ),
            window(
                "All maps",
                &format!(
                    "{}{}{}</form>",
                    all_form.open(),
                    all_form.confirm_box(),
                    button("Forget every map", "danger")
                )
            ),
        );
        let title = format!("{}: automap of {label}", target.game.title());
        self.render_status(req, status, Nav::Saves, &title, flash, &body)
    }

    /// One map: a tickbox per cell (ticked = explored) and its raw bytes.
    fn grid_html(&self, req: &Req, target: &Target, maps: &Maps, map: usize) -> String {
        let Ok(bytes) = automap::map_bytes(&maps.data, target.slot, map) else {
            return String::new();
        };
        let url = target.url();
        let version = maps.version.to_string();
        let number = (map + 1).to_string();
        let cells_hidden = [
            ("version", version.as_str()),
            ("map", number.as_str()),
            ("action", "cells"),
        ];
        let form = Form {
            action: &url,
            csrf: &req.ctx.csrf,
            hidden: &cells_hidden,
            confirm: None,
            inline: false,
        };
        let mut grid = String::from("<table class=\"automap\"><tbody>");
        for row in 0..SIDE {
            grid.push_str("<tr>");
            for col in 0..SIDE {
                let index = row * SIDE + col;
                let value = bytes.get(index).copied().unwrap_or(UNEXPLORED);
                let checked = if value == UNEXPLORED { "" } else { " checked" };
                let special = if value != UNEXPLORED && value != automap::VISITED {
                    format!(" (value {value})")
                } else {
                    String::new()
                };
                let _ = write!(
                    grid,
                    "<td><input type=\"checkbox\" name=\"c{index}\" value=\"1\"{checked} \
                     aria-label=\"row {row}, column {col}{special}\" title=\"row {row}, column {col}{special}\"></td>"
                );
            }
            grid.push_str("</tr>");
        }
        grid.push_str("</tbody></table>");
        let raw_hidden = [
            ("version", version.as_str()),
            ("map", number.as_str()),
            ("action", "raw"),
        ];
        let raw_form = Form {
            action: &url,
            csrf: &req.ctx.csrf,
            hidden: &raw_hidden,
            confirm: None,
            inline: false,
        };
        window(
            &format!("{}: cells", target.map_name(map)),
            &format!(
                "<p class=\"muted\">Ticked cells are explored. A newly ticked cell shows as visited floor; its \
                 walls stay unknown until the player walks there. Rows run top to bottom, north up.</p>\
                 {}{grid}<div class=\"row\">{}</div></form>\
                 <details><summary>Raw bytes of this map ({BLOCK_BYTES})</summary>\
                 <p class=\"muted\">256 cell bytes, two 256-byte wall planes and two 16-byte edge strips; FF is unexplored.</p>\
                 {}<textarea name=\"bytes\" class=\"mono\" rows=\"12\" spellcheck=\"false\">{}</textarea>\
                 <div class=\"row\">{}</div></form></details>",
                form.open(),
                button("Save cells", "primary"),
                raw_form.open(),
                esc(&hex(bytes)),
                button("Save bytes", "primary"),
            ),
        )
    }

    /// `POST /admin/saves/<account>/ybarra/<game>/<slot>/map`.
    pub(crate) fn ybarra_maps_save(
        &self,
        req: &Req,
        account: &str,
        game: &str,
        slot: &str,
    ) -> Reply {
        let Some(target) = Target::parse(account, game, slot) else {
            return self.not_found(req);
        };
        let mut maps = match self.load_maps(req, &target) {
            Ok(maps) => maps,
            Err(page) => return *page,
        };
        let refuse = |maps: &Maps, flash: Flash, status| {
            self.maps_page(req, &target, maps, Some(&flash), status)
        };
        if !self.online_for(target.account).is_empty() {
            let flash = Flash::bad(
                format!(
                    "Account {} is online now, so its automap was not changed.",
                    target.account
                ),
                "You cannot redraw a map while someone is walking it.",
            );
            return refuse(&maps, flash, StatusCode::CONFLICT);
        }
        if req.param("version") != maps.version.to_string() {
            let flash = Flash::bad(
                format!("The file changed since the page was opened (it is now v{}). Nothing was saved.", maps.version),
                "Someone else got to the atlas first.",
            );
            return refuse(&maps, flash, StatusCode::CONFLICT);
        }
        let map = req
            .param("map")
            .parse::<usize>()
            .ok()
            .and_then(|map| map.checked_sub(1));
        let outcome = self.apply_maps(req, &target, &mut maps.data, map);
        let what = match outcome {
            Ok(Some(what)) => what,
            Ok(None) => {
                let flash = Flash::good(
                    "Nothing was different, so nothing was saved.",
                    "The ink stays in the pot.",
                );
                return self.maps_page(req, &target, &maps, Some(&flash), StatusCode::OK);
            }
            Err(why) => {
                let flash = Flash::bad(
                    format!("Nothing was saved: {why}."),
                    "The cartographer frowns.",
                );
                return refuse(&maps, flash, StatusCode::UNPROCESSABLE_ENTITY);
            }
        };
        let now = dates::now_ms();
        let meta = match self.settings.world.store.save_write(
            AccountId(target.account),
            target.game.automaps_path(),
            &maps.data,
            u64::try_from(now).unwrap_or(0),
            true,
        ) {
            Ok(meta) => meta,
            Err(error) => return self.failed(req, &error),
        };
        let audit = format!(
            "account {} {} v{} character slot {}: {what}",
            target.account,
            target.game.automaps_path(),
            meta.version,
            target.slot + 1
        );
        let reason = req.param("reason").trim();
        let reason = (!reason.is_empty()).then(|| reason.chars().take(300).collect::<String>());
        if let Err(error) = self.log_change(
            req,
            "save.edit",
            &audit,
            reason.as_deref(),
            Some(target.account),
        ) {
            return self.failed(req, &error);
        }
        let previous = maps.version;
        maps.version = meta.version;
        let flash = Flash::good(
            format!("Saved as v{} ({what}). The player's game gets it at their next sign-on; v{previous} stays in the history.", meta.version),
            "The cartographer blots the ink.",
        );
        self.maps_page(req, &target, &maps, Some(&flash), StatusCode::OK)
    }

    /// Applies one form to the file; `Ok(None)` when nothing changed, else what changed.
    fn apply_maps(
        &self,
        req: &Req,
        target: &Target,
        data: &mut [u8],
        map: Option<usize>,
    ) -> Result<Option<String>, String> {
        let one = |data: &mut [u8]| -> Result<(usize, String), String> {
            let map = map.ok_or("no map was named")?;
            let name = target.map_name(map);
            automap::map_bytes(data, target.slot, map).map_err(|_| "no such map".to_owned())?;
            Ok((map, name))
        };
        let changed = |before: &[u8], after: &[u8]| before != after;
        match req.param("action") {
            "forget_all" => {
                if !req.confirmed() {
                    return Err("forgetting every map needs a yes".to_owned());
                }
                let mut any = false;
                for map in 0..automap::MAPS {
                    let bytes = automap::map_bytes_mut(data, target.slot, map).map_err(describe)?;
                    let before = bytes.to_vec();
                    automap::forget(bytes);
                    any |= changed(&before, bytes);
                }
                Ok(any.then(|| "every map forgotten".to_owned()))
            }
            "forget" => {
                if !req.confirmed() {
                    return Err("forgetting a map needs a yes".to_owned());
                }
                let (map, name) = one(data)?;
                let bytes = automap::map_bytes_mut(data, target.slot, map).map_err(describe)?;
                let before = bytes.to_vec();
                automap::forget(bytes);
                Ok(changed(&before, bytes).then(|| format!("{name} forgotten")))
            }
            "cells" => {
                let (map, name) = one(data)?;
                let mut wanted = [false; CELLS];
                for (index, want) in wanted.iter_mut().enumerate() {
                    *want = req.param(&format!("c{index}")) == "1";
                }
                let bytes = automap::map_bytes_mut(data, target.slot, map).map_err(describe)?;
                let count = automap::set_explored(bytes, &wanted);
                Ok((count > 0).then(|| format!("{name}: {count} cells")))
            }
            "raw" => {
                let (map, name) = one(data)?;
                let parsed = parse_hex(req.param("bytes"))
                    .filter(|bytes| bytes.len() == BLOCK_BYTES)
                    .ok_or_else(|| {
                        format!("the raw bytes must be exactly {BLOCK_BYTES} bytes of hex")
                    })?;
                let bytes = automap::map_bytes_mut(data, target.slot, map).map_err(describe)?;
                let before = bytes.to_vec();
                bytes.copy_from_slice(&parsed);
                Ok(changed(&before, bytes).then(|| format!("{name}: raw bytes")))
            }
            _ => Err("the form named no action".to_owned()),
        }
    }
}

fn describe(error: AutomapError) -> String {
    match error {
        AutomapError::Size(size) => format!("the file is {size} bytes"),
        AutomapError::NoSuchMap => "no such map".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_bytes_round_trip_through_the_textarea() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(BLOCK_BYTES).collect();
        assert_eq!(parse_hex(&hex(&bytes)).unwrap(), bytes);
        assert_eq!(parse_hex("0"), None);
        assert_eq!(parse_hex("zz"), None);
    }
}
