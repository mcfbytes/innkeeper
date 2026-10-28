//! The item viewer: every record of a game's item database with its fields, filtered by type,
//! guild and power, sorted by any column (docs/server/ybarra.md section 8.1). Everything is a GET.

use std::fmt::Write as _;

use super::html::{esc, window, Nav};
use super::views::{flag, link, tr};
use super::{Admin, Reply, Req};
use crate::ybarra::game_file::Resource;
use crate::ybarra::items::{
    item_type, Confidence, Item, ItemTable, SkillLevel, BONUSES, BONUS_LABELS, ITEM_TYPES,
    ITEM_TYPE_NAMES,
};
use crate::ybarra::schema::schema;
use crate::ybarra::Game;

/// Names the field map already has for a game: guilds, spells and skills.
struct Names {
    guilds: Vec<String>,
    spells: Vec<String>,
    skills: Vec<String>,
}

impl Names {
    fn of(game: Game) -> Names {
        let record = &schema(game).record;
        let items = |name: &str| {
            record
                .iter()
                .find(|field| field.name == name)
                .map(|field| field.items.clone())
                .unwrap_or_default()
        };
        let guilds = record
            .iter()
            .find(|field| field.name == "guild")
            .map(|field| field.values.values().cloned().collect())
            .unwrap_or_default();
        Names {
            guilds,
            spells: items("spell_ranks"),
            skills: items("skill_ranks"),
        }
    }

    fn listed(list: &[String], index: u8) -> String {
        list.get(usize::from(index))
            .cloned()
            .unwrap_or_else(|| format!("#{index}"))
    }
}

/// What the filters ask for, as the query string carries it.
#[derive(Clone, Debug, Default)]
struct Filter {
    type_code: Option<u16>,
    guild: Option<u8>,
    has: String,
    sort: String,
    descending: bool,
}

const POWERS: [(&str, &str); 6] = [
    ("charges", "Charges"),
    ("spell", "A spell"),
    ("skill", "A skill"),
    ("bonus", "A worn bonus"),
    ("consumable", "Used up"),
    ("tradeable", "Tradeable"),
];

impl Filter {
    fn from_req(req: &Req) -> Filter {
        Filter {
            type_code: req.param("type").parse().ok(),
            guild: req.param("guild").parse().ok(),
            has: req.param("has").to_owned(),
            sort: req.param("sort").to_owned(),
            descending: req.param("dir") == "desc",
        }
    }

    fn keeps(&self, item: &Item) -> bool {
        let power = match self.has.as_str() {
            "charges" => item.charges > 0,
            "spell" => item.spell.is_some(),
            "skill" => item.skill.is_some(),
            "bonus" => item.bonuses.iter().any(|b| *b != 0),
            "consumable" => item.consumable,
            "tradeable" => item.tradeable,
            _ => true,
        };
        power
            && self.type_code.is_none_or(|code| item.type_code == code)
            && self.guild.is_none_or(|guild| item.usable_by(guild))
    }

    /// The query string of this filter sorted by `sort`, flipping the order when it already is.
    fn query(&self, sort: &str) -> String {
        let mut out = url::form_urlencoded::Serializer::new(String::new());
        if let Some(code) = self.type_code {
            out.append_pair("type", &code.to_string());
        }
        if let Some(guild) = self.guild {
            out.append_pair("guild", &guild.to_string());
        }
        if !self.has.is_empty() {
            out.append_pair("has", &self.has);
        }
        out.append_pair("sort", sort);
        let flip = self.sort == sort && !self.descending;
        out.append_pair("dir", if flip { "desc" } else { "asc" });
        out.finish()
    }
}

/// A sortable value of a cell.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Number(i64),
    Text(String),
}

/// One column: its query name, heading and value.
struct Column {
    id: &'static str,
    head: String,
    key: Box<dyn Fn(&Item) -> Key>,
}

fn number(value: impl Into<i64>) -> Key {
    Key::Number(value.into())
}

fn columns(names: &Names) -> Vec<Column> {
    let spells = names.spells.clone();
    let skills = names.skills.clone();
    let mut out = vec![
        Column {
            id: "id",
            head: "#".into(),
            key: Box::new(|i| number(i.id)),
        },
        Column {
            id: "name",
            head: "Name".into(),
            key: Box::new(|i| Key::Text(i.name.to_lowercase())),
        },
        Column {
            id: "type",
            head: "Type".into(),
            key: Box::new(|i| number(i.type_code)),
        },
        Column {
            id: "guilds",
            head: "Usable by".into(),
            key: Box::new(|i| number(i.barred_guilds.count_zeros())),
        },
        Column {
            id: "charges",
            head: "Charges".into(),
            key: Box::new(|i| number(i.charges)),
        },
        Column {
            id: "spell",
            head: "Spell".into(),
            key: Box::new(move |i| {
                Key::Text(
                    i.spell
                        .map_or_else(String::new, |(s, _)| Names::listed(&spells, s)),
                )
            }),
        },
        Column {
            id: "skill",
            head: "Skill".into(),
            key: Box::new(move |i| {
                Key::Text(
                    i.skill
                        .map_or_else(String::new, |(s, _)| Names::listed(&skills, s)),
                )
            }),
        },
    ];
    for (index, bonus) in BONUSES.iter().enumerate() {
        out.push(Column {
            id: ["str", "dex", "def", "x12", "agi", "ini"]
                .get(index)
                .copied()
                .unwrap_or("bonus"),
            head: bonus.label.to_owned(),
            key: Box::new(move |i| number(i.bonuses.get(index).copied().unwrap_or(0))),
        });
    }
    out.push(Column {
        id: "icon",
        head: "Icon".into(),
        key: Box::new(|i| number(i.icon)),
    });
    out
}

fn type_text(code: u16) -> String {
    match item_type(code) {
        Some(row) => format!("{} ({code})", row.name),
        None => format!("Type {code}"),
    }
}

fn guilds_text(item: &Item, names: &Names) -> String {
    let allowed: Vec<&str> = names
        .guilds
        .iter()
        .zip(0u8..)
        .filter(|(_, guild)| item.usable_by(*guild))
        .map(|(name, _)| name.as_str())
        .collect();
    match allowed.len() {
        0 => "no guild".to_owned(),
        n if n == names.guilds.len() => "every guild".to_owned(),
        _ => allowed.join(", "),
    }
}

fn flags_html(item: &Item) -> String {
    let mut out = String::new();
    if item.consumable {
        out.push_str(&flag("used up", ""));
    }
    if item.tradeable {
        out.push_str(&flag("tradeable", ""));
    }
    if item.worn_status_bits() != 0 {
        out.push_str(&flag(
            &format!("worn status {:#04x}", item.worn_status_bits()),
            "warn",
        ));
    }
    out
}

fn row_html(item: &Item, names: &Names) -> String {
    let worn = item_type(item.type_code).and_then(|row| row.worn);
    let spell = item.spell.map_or_else(String::new, |(spell, level)| {
        format!("{} (level {level})", Names::listed(&names.spells, spell))
    });
    let skill = item.skill.map_or_else(String::new, |(skill, level)| {
        let level = match level {
            SkillLevel::Fixed(level) => format!("level {level}"),
            SkillLevel::OwnRank => "wearer's rank".to_owned(),
        };
        format!("{} ({level})", Names::listed(&names.skills, skill))
    });
    let mut cells = vec![
        item.id.to_string(),
        format!("<b>{}</b>", esc(&item.name)),
        format!(
            "{}{}",
            esc(&type_text(item.type_code)),
            worn.map_or_else(String::new, |slot| format!(
                "<br><span class=\"muted\">{}</span>",
                esc(slot)
            ))
        ),
        esc(&guilds_text(item, names)),
        if item.charges > 0 {
            item.charges.to_string()
        } else {
            String::new()
        },
        esc(&spell),
        esc(&skill),
    ];
    for bonus in item.bonuses {
        cells.push(if bonus == 0 {
            String::new()
        } else {
            format!("{bonus:+}")
        });
    }
    cells.push(item.icon.to_string());
    cells.push(flags_html(item));
    cells.push(format!(
        "<span class=\"mono\">{:04x} {:02x} {:02x} {:02x}</span>",
        item.unknown[0], item.unknown[1], item.unknown[2], item.unknown[3]
    ));
    tr(&cells)
}

fn select(name: &str, label: &str, options: &[(String, String)], chosen: &str) -> String {
    let mut out = format!(
        "<div><label for=\"filter-{name}\">{}</label><select id=\"filter-{name}\" name=\"{name}\">",
        esc(label)
    );
    for (value, text) in options {
        let selected = if value == chosen { " selected" } else { "" };
        let _ = write!(
            out,
            "<option value=\"{}\"{selected}>{}</option>",
            esc(value),
            esc(text)
        );
    }
    out.push_str("</select></div>");
    out
}

fn filter_form(url: &str, filter: &Filter, table: &ItemTable, names: &Names) -> String {
    let mut types = vec![(String::new(), "Every type".to_owned())];
    for row in &ITEM_TYPES {
        let count = table
            .items
            .iter()
            .filter(|item| item.type_code == row.code)
            .count();
        if count > 0 {
            types.push((row.code.to_string(), format!("{} ({count})", row.name)));
        }
    }
    let mut guilds = vec![(String::new(), "Any guild".to_owned())];
    guilds.extend(
        names
            .guilds
            .iter()
            .zip(0u8..)
            .map(|(name, n)| (n.to_string(), name.clone())),
    );
    let mut powers = vec![(String::new(), "Anything".to_owned())];
    powers.extend(
        POWERS
            .iter()
            .map(|(id, text)| ((*id).to_owned(), (*text).to_owned())),
    );
    let opt = |value: Option<String>| value.unwrap_or_default();
    format!(
        "<form method=\"get\" action=\"{}\" class=\"row\">{}{}{}\
         <input type=\"hidden\" name=\"sort\" value=\"{}\"><button type=\"submit\" class=\"abtn\">Show</button></form>",
        esc(url),
        select("type", "Type", &types, &opt(filter.type_code.map(|c| c.to_string()))),
        select("guild", "Usable by", &guilds, &opt(filter.guild.map(|g| g.to_string()))),
        select("has", "Has", &powers, &filter.has),
        esc(&filter.sort),
    )
}

fn head_html(url: &str, filter: &Filter, columns: &[Column]) -> String {
    let mut out = String::from("<thead><tr>");
    for column in columns {
        let mark = match (filter.sort == column.id, filter.descending) {
            (true, false) => " \u{25b2}",
            (true, true) => " \u{25bc}",
            (false, _) => "",
        };
        let sorted = if mark.is_empty() {
            String::new()
        } else {
            format!(
                " aria-sort=\"{}\"",
                if filter.descending {
                    "descending"
                } else {
                    "ascending"
                }
            )
        };
        let _ = write!(
            out,
            "<th scope=\"col\"{sorted}>{}{mark}</th>",
            link(&format!("{url}?{}", filter.query(column.id)), &column.head)
        );
    }
    out.push_str("<th scope=\"col\">Flags</th><th scope=\"col\" title=\"The word at 0x18, the low bits of 0x1E, 0x1F and 0x20\">Unknown bytes</th></tr></thead>");
    out
}

fn by_type_html(url: &str, table: &ItemTable) -> String {
    let mut rows = String::new();
    for row in &ITEM_TYPES {
        let count = table
            .items
            .iter()
            .filter(|item| item.type_code == row.code)
            .count();
        if count == 0 {
            continue;
        }
        rows.push_str(&tr(&[
            link(
                &format!("{url}?type={}", row.code),
                &format!("{} ({})", row.name, row.code),
            ),
            esc(row.worn.unwrap_or("carried")),
            count.to_string(),
        ]));
    }
    super::views::table(&["Type", "Worn at", "Items"], &rows)
}

fn confidence_note() -> String {
    let marker = |c: Confidence| {
        flag(
            c.word(),
            if c == Confidence::Confirmed {
                "ok"
            } else {
                "warn"
            },
        )
    };
    format!(
        "<p class=\"muted\">Type codes and worn slots, charges, guilds, spells, skills, where each bonus is added, \
         used up and tradeable are confirmed from the games' code. Type names {} and bonus names {} are \
         guesses from the items and the attributes beside each target; the icon byte is inferred; the bytes \
         under Unknown are not explained. No record holds a price.</p>",
        marker(ITEM_TYPE_NAMES),
        marker(BONUS_LABELS),
    )
}

impl Admin {
    /// `/admin/gamedata/<game>/items`.
    pub(crate) fn game_items_page(&self, req: &Req, game: &str) -> Reply {
        let Some(game) = Game::from_id(game) else {
            return self.not_found(req);
        };
        let table = match self.game_items(game) {
            Ok(Some(table)) => table,
            Ok(None) => return self.not_imported(req, game, Resource::Items),
            Err(error) => return self.failed(req, &error),
        };
        let url = super::gamedata::resource_url(game, Resource::Items);
        let names = Names::of(game);
        let filter = Filter::from_req(req);
        let columns = columns(&names);
        let mut shown: Vec<&Item> = table
            .items
            .iter()
            .filter(|item| filter.keeps(item))
            .collect();
        if let Some(column) = columns.iter().find(|column| column.id == filter.sort) {
            shown.sort_by(|a, b| {
                let order = (column.key)(a).cmp(&(column.key)(b)).then(a.id.cmp(&b.id));
                if filter.descending {
                    order.reverse()
                } else {
                    order
                }
            });
        }
        let mut rows = String::new();
        for item in &shown {
            rows.push_str(&row_html(item, &names));
        }
        let list = format!(
            "{}<p class=\"muted\">{} of {} items. Click a heading to sort by it.</p>\
             <div class=\"tablewrap\"><table class=\"items\">{}<tbody>{rows}</tbody></table></div>",
            filter_form(&url, &filter, &table, &names),
            shown.len(),
            table.items.len(),
            head_html(&url, &filter, &columns),
        );
        let body = format!(
            "<p>{} &middot; {}</p>{}{}{}",
            link("/admin/gamedata", "All game data"),
            link(
                &super::gamedata::resource_url(game, Resource::Maps),
                "Maps of this game"
            ),
            window("Items", &list),
            window("By type", &by_type_html(&url, &table)),
            window("What is known", &confidence_note()),
        );
        self.render(
            req,
            Nav::GameData,
            &format!("{}: items", game.title()),
            None,
            &body,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ybarra::items::tests::{record, stored_file};

    fn table() -> ItemTable {
        ItemTable::parse(&stored_file(&[
            record("Nothing", 0, &[]),
            record("Zed Axe", 1, &[(0x1D, 0x80), (0x21, 3)]),
            record("Apple", 11, &[(0x1C, 2), (0x2B, 1)]),
        ]))
        .unwrap()
    }

    #[test]
    fn filters_keep_types_guilds_and_powers() {
        let table = table();
        let keep = |filter: Filter| -> Vec<u16> {
            table
                .items
                .iter()
                .take(3)
                .filter(|i| filter.keeps(i))
                .map(|i| i.id)
                .collect()
        };
        assert_eq!(
            keep(Filter {
                type_code: Some(1),
                ..Filter::default()
            }),
            [1]
        );
        assert_eq!(
            keep(Filter {
                guild: Some(0),
                ..Filter::default()
            }),
            [0, 2]
        );
        assert_eq!(
            keep(Filter {
                has: "charges".into(),
                ..Filter::default()
            }),
            [2]
        );
        assert_eq!(
            keep(Filter {
                has: "bonus".into(),
                ..Filter::default()
            }),
            [1]
        );
    }

    #[test]
    fn a_heading_flips_the_order_it_already_sorts_by() {
        let filter = Filter {
            type_code: Some(11),
            sort: "name".into(),
            ..Filter::default()
        };
        assert_eq!(filter.query("name"), "type=11&sort=name&dir=desc");
        assert_eq!(filter.query("id"), "type=11&sort=id&dir=asc");
        let names = Names {
            guilds: vec!["A".into(), "B".into()],
            spells: vec![],
            skills: vec![],
        };
        let item = &table().items[1];
        assert_eq!(guilds_text(item, &names), "B");
        let row = row_html(item, &names);
        assert!(
            row.contains("<b>Zed Axe</b>") && row.contains("+3"),
            "{row}"
        );
    }
}
