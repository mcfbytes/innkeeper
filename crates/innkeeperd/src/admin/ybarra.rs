//! The character editor for the Ybarra games' saves (docs/server/ybarra.md section 5): the page
//! that imports a game's cipher from the operator's copy, the list of a save file's characters, and
//! the forms that change a character's fields, the file header and the automaps. A change is
//! stored as a new current version of the save file, so the version before it stays as history.

use std::collections::HashMap;
use std::fmt::Write as _;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use base64::Engine;
use hyper::StatusCode;
use innkeeper_world::AccountId;

use super::html::{button, esc, narrator, when, window, Flash, Form, Nav, Tone};
use super::store;
use super::views::{account_link, flag, link, table, tr};
use super::{Admin, Reply, Req};
use crate::dates;
use crate::ybarra::cipher::Cipher;
use crate::ybarra::schema::{schema, Field, Kind, Value};
use crate::ybarra::{self, CharFile, Game, SLOTS};

/// Which part of a character file a form shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Header,
    /// A slot, from 0.
    Slot(usize),
}

impl Part {
    fn parse(text: &str) -> Option<Part> {
        if text == "header" {
            return Some(Part::Header);
        }
        let number: usize = text.parse().ok()?;
        (1..=SLOTS).contains(&number).then(|| Part::Slot(number - 1))
    }

    fn url_part(self) -> String {
        match self {
            Part::Header => "header".to_owned(),
            Part::Slot(slot) => (slot + 1).to_string(),
        }
    }

    fn title(self) -> String {
        match self {
            Part::Header => "File header".to_owned(),
            Part::Slot(slot) => format!("Character slot {}", slot + 1),
        }
    }
}

/// The current version of a game's character file with what it says.
struct Opened {
    game: Game,
    cipher: Cipher,
    version: u64,
    chars: CharFile,
}

/// What came of a request for a character file.
enum Loaded {
    Ready(Box<Opened>),
    Page(Reply),
}

impl Opened {
    fn part(&self, part: Part, game: Game) -> Option<(&'static [Field], &[u8])> {
        let map = schema(game);
        match part {
            Part::Header => Some((&map.header, &self.chars.header)),
            Part::Slot(slot) => Some((&map.record, self.chars.slots.get(slot)?)),
        }
    }

    fn part_mut(&mut self, part: Part) -> Option<(&'static [Field], &mut [u8])> {
        let map = schema(self.game);
        match part {
            Part::Header => Some((&map.header, &mut self.chars.header)),
            Part::Slot(slot) => Some((&map.record, self.chars.slots.get_mut(slot)?)),
        }
    }
}

fn form_name(index: usize, element: usize) -> String {
    format!("f{index}_{element}")
}

fn reason_input() -> &'static str {
    "<div><label for=\"reason\">Reason (kept in the audit log, optional)</label>\
     <input type=\"text\" id=\"reason\" name=\"reason\" maxlength=\"200\"></div>"
}

/// One input for one element of a field. `shown` is the text to put in it: the stored value, or
/// what the operator sent when the form comes back with a complaint.
fn input(field: &Field, name: &str, label: &str, shown: &str) -> String {
    let id = format!("in-{name}");
    let label = format!("<label for=\"{id}\">{}</label>", esc(label));
    if field.kind.is_number() && !field.values.is_empty() {
        let mut options = String::new();
        let mut known = false;
        for (value, text) in &field.values {
            let selected = if value == shown {
                known = true;
                " selected"
            } else {
                ""
            };
            let _ = write!(
                options,
                "<option value=\"{}\"{selected}>{} ({})</option>",
                esc(value),
                esc(text),
                esc(value)
            );
        }
        if !known {
            let _ = write!(
                options,
                "<option value=\"{}\" selected>{} (no name)</option>",
                esc(shown),
                esc(shown)
            );
        }
        return format!("<div>{label}<select id=\"{id}\" name=\"{name}\">{options}</select></div>");
    }
    match field.kind {
        Kind::Str => format!(
            "<div>{label}<input type=\"text\" id=\"{id}\" name=\"{name}\" maxlength=\"{}\" value=\"{}\" spellcheck=\"false\"></div>",
            field.width().saturating_sub(1),
            esc(shown)
        ),
        Kind::Bytes if field.width() > 16 => format!(
            "<div class=\"wide\">{label}<textarea id=\"{id}\" name=\"{name}\" class=\"mono\" rows=\"{}\" spellcheck=\"false\">{}</textarea></div>",
            field.width().div_ceil(16),
            esc(shown)
        ),
        Kind::Bytes => format!(
            "<div>{label}<input type=\"text\" id=\"{id}\" name=\"{name}\" class=\"mono\" size=\"{}\" value=\"{}\" spellcheck=\"false\"></div>",
            field.width() * 3,
            esc(shown)
        ),
        _ => {
            let (low, high) = field.bounds();
            format!(
                "<div>{label}<input type=\"number\" id=\"{id}\" name=\"{name}\" min=\"{low}\" max=\"{high}\" value=\"{}\"></div>",
                esc(shown)
            )
        }
    }
}

/// A marker for a field whose meaning is a guess or unknown, with the evidence on hover.
fn confidence(field: &Field) -> String {
    let word = field.confidence.as_deref().unwrap_or("unknown");
    if word == "confirmed" {
        return String::new();
    }
    let class = if word == "inferred" { "warn" } else { "" };
    let evidence = field.evidence.as_deref().unwrap_or("no evidence recorded");
    format!(
        " <span class=\"flag {class}\" title=\"{}\">{}</span>",
        esc(evidence),
        esc(word)
    )
}

/// The fields of one part as form sections, one per group, in the map's order.
fn fields_html(fields: &[Field], bytes: &[u8], sent: Option<&HashMap<String, String>>) -> String {
    let mut groups: Vec<(&str, String)> = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if field.mirror_of.is_some() {
            continue;
        }
        let group = field.group.as_deref().unwrap_or("Other");
        let mut html = String::new();
        if field.count == 1 {
            let name = form_name(index, 0);
            let shown = sent
                .and_then(|sent| sent.get(&name).cloned())
                .unwrap_or_else(|| field.read(bytes, 0).form_text());
            html.push_str(&input(field, &name, &field.name, &shown));
            if !matches!(field.confidence.as_deref(), Some("confirmed")) {
                html = html.replacen("</label>", &format!("{}</label>", confidence(field)), 1);
            }
        } else {
            let mut cells = String::new();
            for element in 0..field.count {
                let name = form_name(index, element);
                let shown = sent
                    .and_then(|sent| sent.get(&name).cloned())
                    .unwrap_or_else(|| field.read(bytes, element).form_text());
                cells.push_str(&input(field, &name, &field.item_name(element), &shown));
            }
            let _ = write!(
                html,
                "<fieldset class=\"wide\"><legend>{}{}</legend><div class=\"fields\">{cells}</div></fieldset>",
                esc(&field.name),
                confidence(field)
            );
        }
        match groups.iter_mut().find(|(name, _)| *name == group) {
            Some((_, body)) => body.push_str(&html),
            None => groups.push((group, html)),
        }
    }
    let mut out = String::new();
    for (group, body) in groups {
        let _ = write!(
            out,
            "<fieldset class=\"group\"><legend>{}</legend><div class=\"fields\">{body}</div></fieldset>",
            esc(group)
        );
    }
    out
}

/// Applies a form to one part's bytes. Returns the names of the fields that changed, or each
/// complaint with the field it is about; on a complaint the bytes are left as they were.
fn apply(
    fields: &[Field],
    bytes: &mut [u8],
    sent: &HashMap<String, String>,
) -> Result<Vec<String>, Vec<String>> {
    let before = bytes.to_vec();
    let mut problems = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if field.mirror_of.is_some() {
            continue;
        }
        for element in 0..field.count {
            let Some(text) = sent.get(&form_name(index, element)) else {
                continue;
            };
            // An untouched input sends what it showed; writing it back would still clip what
            // follows a short string's NUL, so leave such a field alone.
            if *text == field.read(bytes, element).form_text() {
                continue;
            }
            if let Err(why) = field.write(bytes, element, text) {
                let label = if field.count == 1 {
                    field.name.clone()
                } else {
                    format!("{}: {}", field.name, field.item_name(element))
                };
                problems.push(format!("{label}: {why}"));
            }
        }
    }
    if !problems.is_empty() {
        bytes.copy_from_slice(&before);
        return Err(problems);
    }
    for field in fields {
        let Some(source) = field
            .mirror_of
            .as_ref()
            .and_then(|name| fields.iter().find(|other| &other.name == name))
        else {
            continue;
        };
        for (from, to) in source.span().zip(field.span()).collect::<Vec<_>>() {
            if let Some(byte) = bytes.get(from).copied() {
                if let Some(copy) = bytes.get_mut(to) {
                    *copy = byte;
                }
            }
        }
    }
    let changed = fields
        .iter()
        .filter(|field| field.mirror_of.is_none())
        .filter(|field| field.span().any(|at| before.get(at) != bytes.get(at)))
        .map(|field| field.name.clone())
        .collect();
    Ok(changed)
}

/// A character's line in the list: the map's summary fields.
fn summary(game: Game, record: &[u8]) -> String {
    let fields = &schema(game).record;
    let parts: Vec<String> = fields
        .iter()
        .filter(|field| field.summary)
        .map(|field| {
            let value = field.read(record, 0);
            let text = match (&value, &field.values) {
                (Value::Number(n), values) if values.contains_key(&n.to_string()) => {
                    values[&n.to_string()].clone()
                }
                _ => value.form_text(),
            };
            format!("{}: {}", field.name, text)
        })
        .collect();
    parts.join(", ")
}

impl Admin {
    fn editor_base(account: u32, game: Game) -> String {
        format!("/admin/saves/{account}/ybarra/{}", game.id())
    }

    /// Opens the current character file of a game for an account, or the page that says why not.
    fn load_chars(&self, req: &Req, account: u32, game: Game) -> Loaded {
        let cipher = match store::cipher_for(&self.store, game) {
            Ok(Some(cipher)) => cipher,
            Ok(None) => {
                let body = narrator(
                    Tone::Bad,
                    &format!(
                        "{} saves cannot be opened yet: import the game's tables from your copy first.",
                        game.title()
                    ),
                    Some("The clerk has the strongbox but not the key."),
                ) + &format!("<p>{}</p>", link("/admin/ybarra", "Import the game tables"));
                return Loaded::Page(self.render(req, Nav::Saves, game.title(), None, &body));
            }
            Err(error) => return Loaded::Page(self.failed(req, &error)),
        };
        let found = self.settings.world.store.save_read(
            AccountId(account),
            game.chars_path(),
            None,
        );
        let (meta, data) = match found {
            Ok(Some(found)) => found,
            Ok(None) => return Loaded::Page(self.not_found(req)),
            Err(error) => return Loaded::Page(self.failed(req, &error)),
        };
        match CharFile::open(&cipher, &data) {
            Ok(chars) => Loaded::Ready(Box::new(Opened {
                game,
                cipher,
                version: meta.version,
                chars,
            })),
            Err(ybarra::CharFileError::Size(size)) => {
                let body = narrator(
                    Tone::Bad,
                    &format!(
                        "The stored {} is {size} bytes, not {}; it is not a character file this editor knows.",
                        game.chars_path(),
                        ybarra::CHARS_FILE_BYTES
                    ),
                    Some("The ledger is the wrong shape for the drawer."),
                );
                Loaded::Page(self.render(req, Nav::Saves, game.title(), None, &body))
            }
        }
    }

    /// `/admin/ybarra`: which games can be opened, and the import form.
    pub(crate) fn ybarra_tables(&self, req: &Req) -> Reply {
        self.ybarra_tables_with(req, None)
    }

    fn ybarra_tables_with(&self, req: &Req, flash: Option<&Flash>) -> Reply {
        let rows = match store::ciphers(&self.store) {
            Ok(rows) => rows,
            Err(error) => return self.failed(req, &error),
        };
        let mut lines = String::new();
        for game in Game::ALL {
            let row = rows.iter().find(|row| row.game == game);
            lines.push_str(&tr(&[
                esc(game.title()),
                match row {
                    Some(row) => format!(
                        "{} {}",
                        flag("ready", "ok"),
                        esc(&format!("imported {}", when(row.imported_at / 1000)))
                    ),
                    None => flag("not imported", "warn"),
                },
                row.map_or_else(String::new, |row| {
                    format!(
                        "<span class=\"mono\" title=\"SHA-256 of the program\">{}</span>",
                        esc(&row.source_sha256.chars().take(16).collect::<String>())
                    )
                }),
            ]));
        }
        let form = Form {
            action: "/admin/ybarra/import",
            csrf: &req.ctx.csrf,
            hidden: &[],
            confirm: None,
            inline: false,
        };
        let body = format!(
            "{}{}",
            window(
                "Game tables",
                &format!(
                    "<p>The games keep their characters in a scrambled file. To open one, the editor needs \
                     the game's own tables, read from your copy of one of its programs. Only the tables are \
                     kept (in the database); the program is not.</p>{}",
                    table(&["Game", "State", "From"], &lines)
                )
            ),
            window(
                "Import from a game program",
                &format!(
                    "{}<p>Pick <b>CGENN.EXE</b> or <b>DARKSTRT.EXE</b> from <span class=\"mono\">INN\\YSERBIUS</span>, \
                     <b>TWGENN.EXE</b> or <b>FATES.EXE</b> from <span class=\"mono\">INN\\TWINION</span>, or \
                     <b>TWGENN.EXE</b> or <b>RUINS.EXE</b> from <span class=\"mono\">INN\\CAWDOR</span>.</p>\
                     <div><label for=\"program-file\">Game program</label>\
                     <input type=\"file\" id=\"program-file\" accept=\".exe,.EXE\" data-file-into=\"program\" required></div>\
                     <input type=\"hidden\" name=\"program\" value=\"\">\
                     <p class=\"muted\">The file is read in the browser, so this form needs script. Without it, run \
                     <span class=\"mono\">innkeeperd admin ybarra-import --data-dir DIR FILE</span> on the server.</p>\
                     {}</form>",
                    form.open(),
                    button("Import", "primary")
                )
            ),
        );
        self.render(req, Nav::Saves, "Ybarra game tables", flash, &body)
    }

    /// `POST /admin/ybarra/import`: the program arrives as base64url in `program`.
    pub(crate) fn ybarra_import(&self, req: &Req) -> Reply {
        let flash = match BASE64URL.decode(req.param("program").trim()) {
            Ok(program) if !program.is_empty() => {
                match ybarra::import(&self.store, &program, dates::now_ms()) {
                    Ok(game) => {
                        let hash = innkeeper_world::saves::hash_hex(&program);
                        let target = format!("{} from sha256 {}", game.id(), &hash[..16]);
                        if let Err(error) =
                            self.log_change(req, "ybarra.import", &target, None, None)
                        {
                            return self.failed(req, &error);
                        }
                        Flash::good(
                            format!("{} saves can now be opened and edited.", game.title()),
                            "The clerk files the key on its hook and labels it twice.",
                        )
                    }
                    Err(why) => Flash::bad(
                        format!("Nothing was imported: {why}."),
                        "The clerk turns the file over twice and hands it back.",
                    ),
                }
            }
            _ => Flash::bad(
                "No program came with the form. Pick the file again (the form needs script), or use the command line.",
                "The envelope was empty.",
            ),
        };
        self.ybarra_tables_with(req, Some(&flash))
    }

    /// `/admin/saves/<account>/ybarra/<game>`: the header and the six slots.
    pub(crate) fn ybarra_game(&self, req: &Req, account: &str, game: &str) -> Reply {
        let (Ok(account), Some(game)) = (account.parse::<u32>(), Game::from_id(game)) else {
            return self.not_found(req);
        };
        let opened = match self.load_chars(req, account, game) {
            Loaded::Ready(opened) => opened,
            Loaded::Page(page) => return page,
        };
        let base = Self::editor_base(account, game);
        let mut rows = String::new();
        for (slot, record) in opened.chars.slots.iter().enumerate() {
            rows.push_str(&tr(&[
                format!("{}", slot + 1),
                esc(&summary(game, record)),
                link(&format!("{base}/{}", slot + 1), "Edit"),
                if schema(game).automap.is_some() {
                    link(&format!("{base}/{}/map", slot + 1), "Automap")
                } else {
                    String::new()
                },
            ]));
        }
        let body = format!(
            "<p>{} &middot; account {} &middot; {} v{}</p>{}{}",
            link(&format!("/admin/saves/{account}"), "All saved files"),
            account_link(account),
            esc(game.chars_path()),
            opened.version,
            window(
                "Characters",
                &table(&["Slot", "Character", "", ""], &rows)
            ),
            window(
                "File header",
                &format!(
                    "<p>The 64 bytes before the slots. {}</p>",
                    link(&format!("{base}/header"), "Edit the header")
                )
            ),
        );
        self.render(req, Nav::Saves, game.title(), None, &body)
    }

    /// `/admin/saves/<account>/ybarra/<game>/<part>`: the form for the header or one slot.
    pub(crate) fn ybarra_part(&self, req: &Req, account: &str, game: &str, part: &str) -> Reply {
        let (Ok(account), Some(game), Some(part)) =
            (account.parse::<u32>(), Game::from_id(game), Part::parse(part))
        else {
            return self.not_found(req);
        };
        match self.load_chars(req, account, game) {
            Loaded::Ready(opened) => {
                self.part_page(req, account, &opened, part, None, None, StatusCode::OK)
            }
            Loaded::Page(page) => page,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn part_page(
        &self,
        req: &Req,
        account: u32,
        opened: &Opened,
        part: Part,
        flash: Option<&Flash>,
        sent: Option<&HashMap<String, String>>,
        status: StatusCode,
    ) -> Reply {
        let game = opened.game;
        let map = schema(game);
        let Some((fields, bytes)) = opened.part(part, game) else {
            return self.not_found(req);
        };
        let base = Self::editor_base(account, game);
        let action = format!("{base}/{}", part.url_part());
        let version = opened.version.to_string();
        let hidden = [("version", version.as_str())];
        let form = Form {
            action: &action,
            csrf: &req.ctx.csrf,
            hidden: &hidden,
            confirm: None,
            inline: false,
        };
        let online = !self.online_for(account).is_empty();
        let warning = if online {
            narrator(
                Tone::Bad,
                "This account is online now, so a change would be refused. Its game would overwrite it anyway.",
                Some("Kick them or wait until they log off."),
            )
        } else {
            String::new()
        };
        let invariants = if map.invariants.is_empty() || part == Part::Header {
            String::new()
        } else {
            let mut list = String::from("<ul>");
            for line in &map.invariants {
                let _ = write!(list, "<li>{}</li>", esc(line));
            }
            list.push_str("</ul>");
            window("What the game checks", &list)
        };
        let body = format!(
            "<p>{} &middot; account {} &middot; {} v{}</p>{warning}{invariants}\
             {}<div class=\"editor\">{}</div>{}<div class=\"row\">{}</div></form>",
            link(&base, "All characters in this file"),
            account_link(account),
            esc(game.chars_path()),
            opened.version,
            form.open(),
            fields_html(fields, bytes, sent),
            reason_input(),
            button("Save changes", "primary"),
        );
        let title = format!("{}: {}", game.title(), part.title());
        self.render_status(req, status, Nav::Saves, &title, flash, &body)
    }

    /// `POST /admin/saves/<account>/ybarra/<game>/<part>`: stores the changed file as the new
    /// current version.
    pub(crate) fn ybarra_save(&self, req: &Req, account: &str, game: &str, part: &str) -> Reply {
        let (Ok(account), Some(game), Some(part)) =
            (account.parse::<u32>(), Game::from_id(game), Part::parse(part))
        else {
            return self.not_found(req);
        };
        let mut opened = match self.load_chars(req, account, game) {
            Loaded::Ready(opened) => opened,
            Loaded::Page(page) => return page,
        };
        let sent = &req.params;
        let refuse = |opened: &Opened, flash: Flash, status| {
            self.part_page(req, account, opened, part, Some(&flash), Some(sent), status)
        };
        if !self.online_for(account).is_empty() {
            let flash = Flash::bad(
                format!("Account {account} is online now, so its characters were not changed."),
                "You cannot repaint a ship while it is at sea. Kick them or suspend the account first.",
            );
            return refuse(&opened, flash, StatusCode::CONFLICT);
        }
        if req.param("version") != opened.version.to_string() {
            let flash = Flash::bad(
                format!(
                    "The file changed since this form was opened (it is now v{}). Nothing was saved; \
                     the form below shows what you sent, check it against the new file.",
                    opened.version
                ),
                "Someone else got to the ledger first.",
            );
            return refuse(&opened, flash, StatusCode::CONFLICT);
        }
        let Some((fields, bytes)) = opened.part_mut(part) else {
            return self.not_found(req);
        };
        let outcome = apply(fields, bytes, sent);
        let changed = match outcome {
            Ok(changed) => changed,
            Err(problems) => {
                let flash = Flash::bad(
                    format!("Nothing was saved. {}", problems.join("; ")),
                    "The clerk circles the bad entries in red.",
                );
                return refuse(&opened, flash, StatusCode::UNPROCESSABLE_ENTITY);
            }
        };
        if changed.is_empty() {
            let flash = Flash::good("Nothing was different, so nothing was saved.", "The ink stays in the pot.");
            return self.part_page(req, account, &opened, part, Some(&flash), None, StatusCode::OK);
        }
        let data = opened.chars.seal(&opened.cipher);
        let now = dates::now_ms();
        let meta = match self.settings.world.store.save_write(
            AccountId(account),
            game.chars_path(),
            &data,
            u64::try_from(now).unwrap_or(0),
            true,
        ) {
            Ok(meta) => meta,
            Err(error) => return self.failed(req, &error),
        };
        let target = format!(
            "account {account} {} v{} {}: {}",
            game.chars_path(),
            meta.version,
            part.title().to_lowercase(),
            changed.join(", ")
        );
        let reason = req.param("reason").trim();
        let reason = (!reason.is_empty()).then(|| reason.chars().take(300).collect::<String>());
        if let Err(error) =
            self.log_change(req, "save.edit", &target, reason.as_deref(), Some(account))
        {
            return self.failed(req, &error);
        }
        let previous = opened.version;
        opened.version = meta.version;
        let flash = Flash::good(
            format!(
                "Saved as v{} ({}). The player's game gets it at their next sign-on; v{previous} stays in the history.",
                meta.version,
                changed.join(", ")
            ),
            "Blotted, stamped and back in the drawer.",
        );
        self.part_page(req, account, &opened, part, Some(&flash), None, StatusCode::OK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<Field> {
        serde_json::from_str(
            r#"[
              {"name": "Name", "offset": 0, "type": "str", "len": 6},
              {"name": "Name copy", "offset": 6, "type": "str", "len": 6, "mirror_of": "Name"},
              {"name": "Level", "offset": 12, "type": "u8", "max": 50},
              {"name": "Ranks", "offset": 13, "type": "u8", "count": 2, "items": ["Sword", "Bow"]}
            ]"#,
        )
        .unwrap()
    }

    #[test]
    fn a_form_changes_fields_and_their_mirrors() {
        let fields = fields();
        let mut bytes = *b"Ann\0zzAnn\0zz\x05\x01\x02";
        let mut sent = HashMap::new();
        sent.insert(form_name(0, 0), "Bo".to_owned());
        sent.insert(form_name(2, 0), "5".to_owned());
        sent.insert(form_name(3, 1), "9".to_owned());
        let changed = apply(&fields, &mut bytes, &sent).unwrap();
        assert_eq!(changed, ["Name", "Ranks"]);
        assert_eq!(&bytes[..12], b"Bo\0\0zzBo\0\0zz");
        assert_eq!(bytes[14], 9);
    }

    #[test]
    fn a_bad_value_changes_nothing() {
        let fields = fields();
        let mut bytes = *b"Ann\0zzAnn\0zz\x05\x01\x02";
        let before = bytes;
        let mut sent = HashMap::new();
        sent.insert(form_name(0, 0), "Bo".to_owned());
        sent.insert(form_name(2, 0), "51".to_owned());
        let problems = apply(&fields, &mut bytes, &sent).unwrap_err();
        assert_eq!(problems, ["Level: from 0 to 50"]);
        assert_eq!(bytes, before);
    }

    #[test]
    fn the_form_shows_hostile_text_escaped() {
        let fields = fields();
        let bytes = *b"<b>\0zz<b>\0zz\x05\x01\x02";
        let html = fields_html(&fields, &bytes, None);
        assert!(html.contains("value=\"&lt;b&gt;\""), "{html}");
        assert!(!html.contains("Name copy"), "{html}");
        assert!(html.contains(">Bow</label>"), "{html}");
    }
}
