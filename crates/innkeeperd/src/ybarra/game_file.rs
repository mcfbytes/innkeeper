//! The game data files the portal's viewers read from the operator's copy: the item database and
//! the maps. Both store every 16-bit word rotated; docs/server/ybarra.md section 8.

use super::{dungeon, items, Game};

/// A game data file the portal can import.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Resource {
    /// `RESOURCE.038`: the item records.
    Items,
    /// `RESOURCE.035`: one block per map.
    Maps,
}

impl Resource {
    pub(crate) const ALL: [Resource; 2] = [Resource::Items, Resource::Maps];

    /// The name the store uses.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Resource::Items => "items",
            Resource::Maps => "maps",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Resource> {
        Resource::ALL
            .into_iter()
            .find(|resource| resource.id() == id)
    }

    /// The file in the game's folder.
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Resource::Items => "RESOURCE.038",
            Resource::Maps => "RESOURCE.035",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Resource::Items => "Items",
            Resource::Maps => "Maps",
        }
    }

    /// The size the three games' copies all have.
    pub(crate) fn file_bytes(self) -> usize {
        match self {
            Resource::Items => items::FILE_BYTES,
            Resource::Maps => dungeon::FILE_BYTES,
        }
    }

    /// Which file this is, told by its size (the two never share one).
    pub(crate) fn identify(bytes: &[u8]) -> Option<Resource> {
        Resource::ALL
            .into_iter()
            .find(|resource| resource.file_bytes() == bytes.len())
    }
}

/// Undoes the games' storage of a data file: every little-endian word rotated right by `by` bits,
/// so reading rotates it left. A trailing odd byte is kept as it is.
pub(crate) fn unrotate(stored: &[u8], by: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(stored.len());
    let mut words = stored.chunks_exact(2);
    for pair in words.by_ref() {
        if let [low, high] = *pair {
            let word = u16::from_le_bytes([low, high]);
            out.extend_from_slice(&word.rotate_left(by).to_le_bytes());
        }
    }
    out.extend_from_slice(words.remainder());
    out
}

/// What the import of one file found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Imported {
    pub(crate) game: Game,
    pub(crate) resource: Resource,
}

/// Checks a game data file and keeps it for the game the operator named. The file must parse,
/// so a wrong file is refused here and not when a page is opened.
pub(crate) fn import(
    store: &crate::sqlite_store::SqliteStore,
    game: Game,
    bytes: &[u8],
    now: i64,
) -> Result<Imported, String> {
    let resource = Resource::identify(bytes).ok_or_else(|| {
        format!(
            "a {} file is {} bytes and a {} file {} bytes; this one is {}",
            Resource::Items.file_name(),
            Resource::Items.file_bytes(),
            Resource::Maps.file_name(),
            Resource::Maps.file_bytes(),
            bytes.len()
        )
    })?;
    let parsed = match resource {
        Resource::Items => items::ItemTable::parse(bytes)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        Resource::Maps => dungeon::Dungeon::parse(bytes)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    };
    parsed.map_err(|why| format!("it is not the game's {}: {why}", resource.file_name()))?;
    let hash = innkeeper_world::saves::hash_hex(bytes);
    crate::admin::store::resource_put(store, game, resource, bytes, &hash, now)
        .map_err(|error| error.to_string())?;
    Ok(Imported { game, resource })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Stores bytes the way the games do, for fixtures.
    pub(crate) fn rotate(plain: &[u8], by: u32) -> Vec<u8> {
        unrotate(plain, 16 - by)
    }

    #[test]
    fn words_rotate_back_and_an_odd_byte_stays() {
        let plain = [0x34, 0x12, 0xcd, 0xab, 0x7f];
        let stored = rotate(&plain, 6);
        assert_ne!(stored, plain);
        assert_eq!(unrotate(&stored, 6), plain);
        assert_eq!(unrotate(&[0x01, 0x00], 2), [0x04, 0x00]);
        assert_eq!(stored[4], 0x7f);
    }

    /// The real files, when a copy is at hand: `INNKEEPER_YBARRA_DIR` as for the programs, each
    /// game folder holding `RESOURCE.038` and `RESOURCE.035` (any that exist are tried).
    #[test]
    fn real_files_when_present() {
        let Ok(dir) = std::env::var("INNKEEPER_YBARRA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        for folder in ["YSERBIUS", "TWINION", "CAWDOR"] {
            if let Ok(bytes) = std::fs::read(dir.join(folder).join("RESOURCE.038")) {
                let table =
                    items::ItemTable::parse(&bytes).unwrap_or_else(|e| panic!("{folder}: {e}"));
                let typed = table
                    .items
                    .iter()
                    .filter(|i| items::item_type(i.type_code).is_some());
                assert_eq!(typed.count(), items::RECORDS, "{folder}");
            }
            if let Ok(bytes) = std::fs::read(dir.join(folder).join("RESOURCE.035")) {
                let maps =
                    dungeon::Dungeon::parse(&bytes).unwrap_or_else(|e| panic!("{folder}: {e}"));
                // The outer walls of the top row and the left column face north and west, as the
                // side order assumes; nearly every walled cell there agrees.
                for (cells, side) in [
                    ((0..16).collect::<Vec<_>>(), dungeon::Side::North),
                    ((0..16).map(|row| row * 16).collect(), dungeon::Side::West),
                ] {
                    let walled: Vec<&dungeon::Cell> = maps
                        .maps
                        .iter()
                        .flat_map(|map| cells.iter().filter_map(|at| map.cell(*at)))
                        .filter(|cell| cell.walls.iter().any(|w| *w != 0))
                        .collect();
                    let agree = walled
                        .iter()
                        .filter(|c| c.edge(side) != dungeon::Edge::Open)
                        .count();
                    assert!(
                        agree * 100 >= walled.len() * 95,
                        "{folder} {side:?}: {agree} of {}",
                        walled.len()
                    );
                }
            }
        }
    }

    #[test]
    fn files_are_told_apart_by_size() {
        assert_eq!(Resource::identify(&vec![0; 11_776]), Some(Resource::Items));
        assert_eq!(Resource::identify(&vec![0; 218_144]), Some(Resource::Maps));
        assert_eq!(Resource::identify(&[0; 10]), None);
        for resource in Resource::ALL {
            assert_eq!(Resource::from_id(resource.id()), Some(resource));
        }
    }
}
