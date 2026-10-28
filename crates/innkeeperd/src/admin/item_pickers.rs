//! The item lists of the character editor's inventory slots: each restricted slot gets its own list
//! of what the game lets the character use there, weakest first (docs/server/ybarra.md section 8.4).
//! The lists only suggest; the save still takes any item.

use super::html::esc;
use crate::ybarra::items::{Item, ItemTable};
use crate::ybarra::schema::Catalog;

/// The id of the list of every item, which unrestricted slots use.
pub(crate) const CATALOG_LIST: &str = "ybarra-items";

/// What the game allows in an inventory slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Area {
    /// Backpack (0 to 9), quest item bag (21 to 30), holding box (31): any item.
    Free,
    /// Slots 10 to 14, which the inventory builder empties every time.
    Unused,
    /// A worn slot, with the item types its position takes.
    Worn(&'static [u16]),
}

/// Slot by slot, the same in all three games: the inventory builder and the equip routine
/// (docs/server/ybarra.md sections 4 and 8.4).
pub(crate) fn area(slot: usize) -> Area {
    match slot {
        10..=14 => Area::Unused,
        15 => Area::Worn(&[7]),
        16 => Area::Worn(&[8]),
        17 => Area::Worn(&[1, 2, 3, 4]),
        18 => Area::Worn(&[6]),
        19 | 20 => Area::Worn(&[9]),
        _ => Area::Free,
    }
}

/// That the guild bar refuses wearing, and not only using, is not traced: the check that prints
/// "That item is not usable by your guild!" is in the item widget's handler.
pub(crate) const GUILD_BAR_GATES_WEARING_INFERRED: bool = true;

/// The per-slot lists of one character's form.
#[derive(Debug, Default)]
pub(crate) struct Pickers {
    /// The list id of each item slot.
    lists: Vec<String>,
    /// The `<datalist>` elements, the full one included.
    pub(crate) html: String,
}

impl Pickers {
    pub(crate) fn list_for(&self, slot: usize) -> &str {
        self.lists.get(slot).map_or(CATALOG_LIST, String::as_str)
    }
}

/// Who the lists are for and what is in the slots now.
pub(crate) struct Wearer<'a> {
    pub(crate) guild: Option<u8>,
    /// The item id in each slot.
    pub(crate) current: &'a [u16],
}

fn allowed(item: &Item, area: Area, guild: Option<u8>) -> bool {
    match area {
        Area::Free => true,
        Area::Unused => item.id == 0,
        Area::Worn(types) => {
            let guild_ok = !GUILD_BAR_GATES_WEARING_INFERRED
                || guild.is_none_or(|guild| item.usable_by(guild));
            item.id == 0 || (types.contains(&item.type_code) && guild_ok)
        }
    }
}

fn option(id: u16, catalog: &Catalog, note: &str) -> String {
    let entry = catalog.items.iter().find(|item| item.id == id);
    let name = entry.map_or_else(
        || {
            if id == 0 {
                "Empty".to_owned()
            } else {
                format!("Item {id}")
            }
        },
        |e| e.name.clone(),
    );
    let charges = entry
        .and_then(|item| item.charges)
        .map_or_else(String::new, |charges| {
            format!(" data-charges=\"{charges}\"")
        });
    let label = if note.is_empty() {
        String::new()
    } else {
        format!(" label=\"{}\"", esc(note))
    };
    format!(
        "<option value=\"{} ({id})\" data-id=\"{id}\"{charges}{label}></option>",
        esc(&name)
    )
}

fn datalist(id: &str, options: &str, catalog: &Catalog) -> String {
    let empty = catalog
        .no_charges
        .map_or_else(String::new, |value| format!(" data-no-charges=\"{value}\""));
    format!("<datalist id=\"{id}\"{empty}>{options}</datalist>")
}

/// The list of every item, by name.
pub(crate) fn full_list(catalog: &Catalog) -> String {
    let mut ids: Vec<(String, u16)> = catalog
        .items
        .iter()
        .map(|item| (item.name.clone(), item.id))
        .collect();
    if !ids.iter().any(|(_, id)| *id == 0) {
        ids.push(("Empty".to_owned(), 0));
    }
    ids.sort();
    let options: String = ids.iter().map(|(_, id)| option(*id, catalog, "")).collect();
    datalist(CATALOG_LIST, &options, catalog)
}

/// One list per restricted slot, weakest usable item first and the slot's current item always
/// in it; free slots share the full list. `None` items (file not imported) restrict nothing.
pub(crate) fn pickers(
    table: Option<&ItemTable>,
    wearer: &Wearer<'_>,
    catalog: &Catalog,
) -> Pickers {
    let mut out = Pickers {
        lists: Vec::new(),
        html: full_list(catalog),
    };
    let Some(table) = table else {
        return out;
    };
    for (slot, current) in wearer.current.iter().enumerate() {
        let area = area(slot);
        if area == Area::Free {
            out.lists.push(CATALOG_LIST.to_owned());
            continue;
        }
        let mut usable: Vec<&Item> = table
            .items
            .iter()
            .filter(|item| allowed(item, area, wearer.guild))
            .collect();
        usable.sort_by(|a, b| {
            (a.id != 0, a.power(), a.name.as_str(), a.id).cmp(&(
                b.id != 0,
                b.power(),
                b.name.as_str(),
                b.id,
            ))
        });
        let mut options: String = usable
            .iter()
            .map(|item| option(item.id, catalog, ""))
            .collect();
        if !usable.iter().any(|item| item.id == *current) {
            options.insert_str(
                0,
                &option(*current, catalog, "in the slot now; not usable here"),
            );
        }
        let id = format!("{CATALOG_LIST}-{slot}");
        out.html.push_str(&datalist(&id, &options, catalog));
        out.lists.push(id);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ybarra::items::tests::{record, stored_file};

    fn catalog() -> Catalog {
        serde_json::from_str(
            r#"{"items": [{"id": 1, "name": "Club"}, {"id": 2, "name": "Big <Sword>"},
               {"id": 3, "name": "Cap"}, {"id": 4, "name": "Knight Blade", "charges": 3}], "no_charges": 255}"#,
        )
        .unwrap()
    }

    fn table() -> ItemTable {
        ItemTable::parse(&stored_file(&[
            record("", 0, &[]),
            record("Club", 1, &[(0x21, 1)]),
            record("Big <Sword>", 1, &[(0x21, 4)]),
            record("Cap", 7, &[]),
            record("Knight Blade", 1, &[(0x1D, 0x80), (0x21, 9)]),
        ]))
        .unwrap()
    }

    #[test]
    fn worn_slots_offer_their_types_for_the_guild_weakest_first_and_keep_what_is_there() {
        let mut current = [0u16; 32];
        current[17] = 3;
        let pickers = pickers(
            Some(&table()),
            &Wearer {
                guild: Some(0),
                current: &current,
            },
            &catalog(),
        );
        let weapon = pickers
            .html
            .split("<datalist id=\"ybarra-items-17\"")
            .nth(1)
            .unwrap();
        let weapon = weapon.split("</datalist>").next().unwrap();
        let order: Vec<&str> = weapon
            .split("data-id=\"")
            .skip(1)
            .map(|s| &s[..1])
            .collect();
        assert_eq!(order, ["3", "0", "1", "2"], "{weapon}");
        assert!(
            weapon.contains("label=\"in the slot now; not usable here\""),
            "{weapon}"
        );
        assert!(
            weapon.contains("Big &lt;Sword&gt; (2)") && !weapon.contains("Knight"),
            "{weapon}"
        );
        assert_eq!(pickers.list_for(17), "ybarra-items-17");
        assert_eq!(pickers.list_for(3), CATALOG_LIST);
        let unused = pickers
            .html
            .split("<datalist id=\"ybarra-items-12\"")
            .nth(1)
            .unwrap();
        assert_eq!(
            unused
                .split("</datalist>")
                .next()
                .unwrap()
                .matches("<option")
                .count(),
            1
        );
        assert!(
            pickers.html.contains("data-id=\"4\" data-charges=\"3\""),
            "{}",
            pickers.html
        );
    }

    #[test]
    fn another_guild_gets_the_barred_item_and_no_table_restricts_nothing() {
        let current = [0u16; 32];
        let knight = pickers(
            Some(&table()),
            &Wearer {
                guild: Some(1),
                current: &current,
            },
            &catalog(),
        );
        assert!(knight.html.contains("Knight Blade (4)\" data-id=\"4\" data-charges=\"3\"></option></datalist><datalist id=\"ybarra-items-18\""), "{}", knight.html);
        let none = pickers(
            None,
            &Wearer {
                guild: Some(1),
                current: &current,
            },
            &catalog(),
        );
        assert_eq!(none.list_for(17), CATALOG_LIST);
        assert_eq!(none.html.matches("<datalist").count(), 1);
        assert_eq!(area(31), Area::Free);
        assert_eq!(area(0), Area::Free);
    }
}
