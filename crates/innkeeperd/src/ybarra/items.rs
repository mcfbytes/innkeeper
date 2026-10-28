//! The item database of the Ybarra games (`RESOURCE.038`): 256 records of 46 bytes, the same layout
//! in all three games. Which bytes are known and how is in docs/server/ybarra.md section 8.1.

use super::game_file::unrotate;

pub(crate) const RECORDS: usize = 256;
pub(crate) const RECORD_BYTES: usize = 0x2E;
pub(crate) const FILE_BYTES: usize = RECORDS * RECORD_BYTES;
/// The games read the file with load mode 7, a left rotation of each word by 6.
const ROTATION: u32 = 6;

/// How sure the docs are of a fact the code relies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Confidence {
    Confirmed,
    Inferred,
}

impl Confidence {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Confidence::Confirmed => "confirmed",
            Confidence::Inferred => "inferred",
        }
    }
}

/// Offsets in a decoded record.
const NAME: usize = 0x00;
const NAME_BYTES: usize = 0x18;
const UNKNOWN_WORD: usize = 0x18;
const TYPE: usize = 0x1A;
const CHARGES: usize = 0x1C;
const BARRED_GUILDS: usize = 0x1D;
const WORN_STATUS: usize = 0x1E;
const UNKNOWN_PAIR: usize = 0x1F;
const SPELL: usize = 0x25;
const SPELL_LEVEL: usize = 0x26;
const SKILL: usize = 0x27;
const SKILL_LEVEL: usize = 0x28;
const CONSUMABLE: usize = 0x2B;
const TRADEABLE: usize = 0x2C;
const ICON_INFERRED: usize = 0x2D;
/// No spell or skill; as a skill level, the wearer's own rank in the skill.
const NONE: u8 = 0xFF;
/// The bits of `WORN_STATUS` the games OR into the wearer's status while it is worn.
const WORN_STATUS_BITS: u8 = 0xE0;

/// A worn bonus: the record byte the equip routine adds to a character byte or word.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bonus {
    pub(crate) offset: usize,
    pub(crate) label: &'static str,
    /// Added to a word with the byte zero-extended; otherwise a byte add, which wraps like -128..127.
    pub(crate) to_word: bool,
}

/// The six bonuses in record order. Where they go is CONFIRMED; the labels are INFERRED from the
/// attribute each target sits beside (docs/server/ybarra.md section 8.1).
pub(crate) const BONUSES: [Bonus; 6] = [
    Bonus {
        offset: 0x21,
        label: "Strength",
        to_word: false,
    },
    Bonus {
        offset: 0x22,
        label: "Dexterity",
        to_word: true,
    },
    Bonus {
        offset: 0x23,
        label: "Defense",
        to_word: false,
    },
    Bonus {
        offset: 0x24,
        label: "Unnamed (+0x12)",
        to_word: true,
    },
    Bonus {
        offset: 0x29,
        label: "Agility",
        to_word: false,
    },
    Bonus {
        offset: 0x2A,
        label: "Initiative",
        to_word: false,
    },
];
pub(crate) const BONUS_LABELS: Confidence = Confidence::Inferred;

/// Points per unit of each bonus in the power score, in `BONUSES` order: about 3,200 points for an
/// attribute's whole clamp range (99, 3,200, 99, unknown, 50, 50). A judgement, not the game's.
pub(crate) const POWER_WEIGHTS_INFERRED: [i32; 6] = [32, 1, 32, 1, 64, 64];
/// Points per level of an item's spell or skill power.
pub(crate) const POWER_PER_LEVEL_INFERRED: i32 = 100;
/// The level counted for a skill that works at the wearer's own rank: the middle of 0 to 12.
pub(crate) const OWN_RANK_LEVEL_ASSUMED: i32 = 6;

/// An item type: what it is and where it is worn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ItemType {
    pub(crate) code: u16,
    pub(crate) name: &'static str,
    /// The worn slot the equip routine accepts it in; `None` for an item that is only carried.
    pub(crate) worn: Option<&'static str>,
}

/// The types the games use. The slots are CONFIRMED (the equip routine's switch); the names are
/// INFERRED from the items of each type.
pub(crate) const ITEM_TYPES: [ItemType; 11] = [
    ItemType {
        code: 0,
        name: "None",
        worn: None,
    },
    ItemType {
        code: 1,
        name: "Blade, axe or club",
        worn: Some("Weapon hand"),
    },
    ItemType {
        code: 2,
        name: "Staff, spear or polearm",
        worn: Some("Weapon hand"),
    },
    ItemType {
        code: 3,
        name: "Bow or sling",
        worn: Some("Weapon hand"),
    },
    ItemType {
        code: 4,
        name: "Weapon (no item has it)",
        worn: Some("Weapon hand"),
    },
    ItemType {
        code: 6,
        name: "Shield or gauntlets",
        worn: Some("Shield hand"),
    },
    ItemType {
        code: 7,
        name: "Head armour",
        worn: Some("Head"),
    },
    ItemType {
        code: 8,
        name: "Body armour",
        worn: Some("Body"),
    },
    ItemType {
        code: 9,
        name: "Accessory",
        worn: Some("Accessory 1 or 2"),
    },
    ItemType {
        code: 11,
        name: "Usable (scroll, potion, wand)",
        worn: None,
    },
    ItemType {
        code: 12,
        name: "Quest item, key or other",
        worn: None,
    },
];
pub(crate) const ITEM_TYPE_NAMES: Confidence = Confidence::Inferred;

pub(crate) fn item_type(code: u16) -> Option<&'static ItemType> {
    ITEM_TYPES.iter().find(|row| row.code == code)
}

/// The level a skill item works at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SkillLevel {
    Fixed(u8),
    /// The wearer's own rank in the skill.
    OwnRank,
}

/// One item record, decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) id: u16,
    pub(crate) name: String,
    pub(crate) type_code: u16,
    /// What a new one holds; 0 for an item without charges.
    pub(crate) charges: u8,
    /// Bit `0x80 >> guild` set: that guild may not use it.
    pub(crate) barred_guilds: u8,
    pub(crate) worn_status: u8,
    pub(crate) spell: Option<(u8, u8)>,
    pub(crate) skill: Option<(u8, SkillLevel)>,
    /// In `BONUSES` order, as the games add them.
    pub(crate) bonuses: [i16; 6],
    pub(crate) consumable: bool,
    pub(crate) tradeable: bool,
    pub(crate) icon: u8,
    /// The bytes nobody has explained: the word at 0x18, the low bits of 0x1E, 0x1F and 0x20.
    pub(crate) unknown: [u16; 4],
}

impl Item {
    pub(crate) fn usable_by(&self, guild: u8) -> bool {
        guild < 8 && self.barred_guilds & (0x80 >> guild) == 0
    }

    pub(crate) fn worn_status_bits(&self) -> u8 {
        self.worn_status & WORN_STATUS_BITS
    }

    /// The picker's sort key: weighted worn bonuses plus the spell and skill levels
    /// (docs/server/ybarra.md section 8.4). Higher is stronger.
    pub(crate) fn power(&self) -> i32 {
        let bonuses: i32 = self
            .bonuses
            .iter()
            .zip(POWER_WEIGHTS_INFERRED)
            .map(|(bonus, weight)| i32::from(*bonus) * weight)
            .sum();
        let spell = self.spell.map_or(0, |(_, level)| i32::from(level));
        let skill = self.skill.map_or(0, |(_, level)| match level {
            SkillLevel::Fixed(level) => i32::from(level),
            SkillLevel::OwnRank => OWN_RANK_LEVEL_ASSUMED,
        });
        bonuses + (spell + skill) * POWER_PER_LEVEL_INFERRED
    }
}

/// Why a file is not an item database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ItemsError {
    Size(usize),
    /// An item's name is not printable text ending in a NUL, so the decoding does not fit.
    Name(u16),
}

impl std::fmt::Display for ItemsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemsError::Size(size) => write!(f, "{size} bytes, not {FILE_BYTES}"),
            ItemsError::Name(id) => write!(f, "item {id} has no readable name"),
        }
    }
}

/// Every item of a game, by id.
#[derive(Clone, Debug)]
pub(crate) struct ItemTable {
    pub(crate) items: Vec<Item>,
}

fn byte(record: &[u8], at: usize) -> u8 {
    record.get(at).copied().unwrap_or(0)
}

fn word(record: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([byte(record, at), byte(record, at + 1)])
}

fn name(record: &[u8]) -> Option<String> {
    let field = record.get(NAME..NAME + NAME_BYTES)?;
    let end = field.iter().position(|b| *b == 0)?;
    let text = field.get(..end)?;
    text.iter()
        .all(|b| (0x20..0x7f).contains(b))
        .then(|| text.iter().map(|b| char::from(*b)).collect())
}

fn parse_item(id: u16, record: &[u8]) -> Result<Item, ItemsError> {
    let name = name(record).ok_or(ItemsError::Name(id))?;
    let spell =
        (byte(record, SPELL) != NONE).then(|| (byte(record, SPELL), byte(record, SPELL_LEVEL)));
    let skill = (byte(record, SKILL) != NONE).then(|| {
        let level = match byte(record, SKILL_LEVEL) {
            NONE => SkillLevel::OwnRank,
            level => SkillLevel::Fixed(level),
        };
        (byte(record, SKILL), level)
    });
    let bonuses = BONUSES.map(|bonus| {
        let raw = byte(record, bonus.offset);
        if bonus.to_word {
            i16::from(raw)
        } else {
            i16::from(i8::from_le_bytes([raw]))
        }
    });
    Ok(Item {
        id,
        name,
        type_code: word(record, TYPE),
        charges: byte(record, CHARGES),
        barred_guilds: byte(record, BARRED_GUILDS),
        worn_status: byte(record, WORN_STATUS),
        spell,
        skill,
        bonuses,
        consumable: byte(record, CONSUMABLE) != 0,
        tradeable: byte(record, TRADEABLE) != 0,
        icon: byte(record, ICON_INFERRED),
        unknown: [
            word(record, UNKNOWN_WORD),
            u16::from(byte(record, WORN_STATUS) & !WORN_STATUS_BITS),
            u16::from(byte(record, UNKNOWN_PAIR)),
            u16::from(byte(record, UNKNOWN_PAIR + 1)),
        ],
    })
}

impl ItemTable {
    /// Parses the file as it lies in the game's folder.
    pub(crate) fn parse(stored: &[u8]) -> Result<ItemTable, ItemsError> {
        if stored.len() != FILE_BYTES {
            return Err(ItemsError::Size(stored.len()));
        }
        let plain = unrotate(stored, ROTATION);
        let items = plain
            .chunks_exact(RECORD_BYTES)
            .zip(0u16..)
            .map(|(record, id)| parse_item(id, record))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ItemTable { items })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ybarra::game_file::tests::rotate;

    /// A made-up record: a name, a type and the given bytes at their offsets.
    pub(crate) fn record(name: &str, type_code: u16, bytes: &[(usize, u8)]) -> [u8; RECORD_BYTES] {
        let mut out = [0u8; RECORD_BYTES];
        out[..name.len()].copy_from_slice(name.as_bytes());
        out[TYPE..TYPE + 2].copy_from_slice(&type_code.to_le_bytes());
        out[SPELL] = NONE;
        out[SKILL] = NONE;
        for (at, value) in bytes {
            out[*at] = *value;
        }
        out
    }

    /// A whole file as the games store it, from made-up records; the rest are blank.
    pub(crate) fn stored_file(records: &[[u8; RECORD_BYTES]]) -> Vec<u8> {
        let mut plain = Vec::with_capacity(FILE_BYTES);
        for id in 0..RECORDS {
            let blank = record("", 0, &[]);
            plain.extend_from_slice(records.get(id).unwrap_or(&blank));
        }
        rotate(&plain, ROTATION)
    }

    #[test]
    fn a_record_decodes_its_names_bonuses_and_powers() {
        let wand = record(
            "Wand <of> Sparks",
            11,
            &[
                (CHARGES, 5),
                (BARRED_GUILDS, 0x80 >> 1),
                (WORN_STATUS, 0x21),
                (0x21, 0xFD),
                (0x22, 0xF0),
                (SPELL, 6),
                (SPELL_LEVEL, 9),
                (SKILL, 3),
                (SKILL_LEVEL, NONE),
                (CONSUMABLE, 1),
                (ICON_INFERRED, 31),
                (UNKNOWN_PAIR, 4),
            ],
        );
        let table = ItemTable::parse(&stored_file(&[record("Nothing", 0, &[]), wand])).unwrap();
        assert_eq!(table.items.len(), RECORDS);
        let item = &table.items[1];
        assert_eq!(
            (item.id, item.name.as_str(), item.type_code),
            (1, "Wand <of> Sparks", 11)
        );
        assert_eq!(
            (item.charges, item.spell, item.skill),
            (5, Some((6, 9)), Some((3, SkillLevel::OwnRank)))
        );
        assert_eq!(item.bonuses, [-3, 240, 0, 0, 0, 0]);
        assert!(item.usable_by(0) && !item.usable_by(1) && item.usable_by(5));
        assert_eq!(
            (item.worn_status_bits(), item.unknown),
            (0x20, [0, 1, 4, 0])
        );
        assert!(item.consumable && !item.tradeable);
        assert_eq!(item.icon, 31);
        assert_eq!(table.items[2].spell, None);
    }

    #[test]
    fn a_file_of_the_wrong_size_or_with_unreadable_names_is_refused() {
        assert_eq!(
            ItemTable::parse(&[0; 10]).unwrap_err(),
            ItemsError::Size(10)
        );
        let mut long = record("", 1, &[]);
        long[..NAME_BYTES].fill(b'x');
        assert_eq!(
            ItemTable::parse(&stored_file(&[long])).unwrap_err(),
            ItemsError::Name(0)
        );
        assert!(ItemTable::parse(&vec![0xAA; FILE_BYTES]).is_err());
    }

    #[test]
    fn power_weighs_bonuses_and_levels() {
        let table = ItemTable::parse(&stored_file(&[
            record("Plain", 1, &[]),
            record("Heavy", 1, &[(0x21, 2), (0x23, 0xFE), (0x22, 40)]),
            record(
                "Wand",
                11,
                &[
                    (SPELL, 1),
                    (SPELL_LEVEL, 3),
                    (SKILL, 2),
                    (SKILL_LEVEL, NONE),
                ],
            ),
        ]))
        .unwrap();
        let power: Vec<i32> = table.items.iter().take(3).map(Item::power).collect();
        assert_eq!(power, [0, 40, (3 + OWN_RANK_LEVEL_ASSUMED) * 100]);
    }

    #[test]
    fn every_type_has_one_row() {
        let mut codes: Vec<u16> = ITEM_TYPES.iter().map(|row| row.code).collect();
        codes.dedup();
        assert_eq!(codes.len(), ITEM_TYPES.len());
        assert_eq!(item_type(6).and_then(|row| row.worn), Some("Shield hand"));
        assert!(item_type(5).is_none());
    }
}
