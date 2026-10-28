//! The maps of the Ybarra games (`RESOURCE.035`): per map a 16 by 16 grid with each cell's walls,
//! the kind of each wall side and the event planes the map scripts hang on them. The same layout
//! in all three games; docs/server/ybarra.md section 8.2.

use super::automap::{CELLS, MAPS};
use super::game_file::unrotate;
use super::items::Confidence;

pub(crate) const BLOCK_BYTES: usize = 0x1910;
pub(crate) const FILE_BYTES: usize = MAPS * BLOCK_BYTES;
/// What the map loader reads of each block, with load mode 3 (each word rotated left by 2).
const READ_BYTES: usize = 0xC10;
const ROTATION: u32 = 2;
/// Offsets in a decoded block; a plane is one byte per cell.
const HEADER_BYTES: usize = 0x10;
const WALLS: usize = 0x10;
const SIDE_KINDS: usize = 0x110;
const SIDE_EVENTS: usize = 0x510;
const ENTER_EVENTS: usize = 0x910;
const SECOND_EVENTS: usize = 0xA10;
const CELL_KINDS: usize = 0xB10;
const PLANE: usize = 0x100;
/// A side kind the loader turns into a way through, even where the wall bits are set.
const PASSABLE_KIND: u8 = 1;

/// A cell's side, in the order of its two-bit field in the wall byte (side `n` at bit `2n`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    West,
    South,
    East,
    North,
}

/// Which compass side each two-bit field is: INFERRED from the outer walls of every map and from
/// the neighbouring cells agreeing (docs/server/ybarra.md section 8.2).
pub(crate) const SIDE_ORDER_INFERRED: [Side; 4] =
    [Side::West, Side::South, Side::East, Side::North];
pub(crate) const SIDE_ORDER: Confidence = Confidence::Inferred;

impl Side {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Side::West => "west",
            Side::South => "south",
            Side::East => "east",
            Side::North => "north",
        }
    }

    /// The side's index in the wall byte, the side planes and (INFERRED) a facing value.
    pub(crate) fn index(self) -> usize {
        SIDE_ORDER_INFERRED
            .iter()
            .position(|side| *side == self)
            .unwrap_or(0)
    }
}

/// What a cell's side is to someone walking through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Open,
    /// The wall bits, 1 to 3.
    Wall(u8),
    /// A wall the loader makes passable (side kind 1): a door or a secret way, which is INFERRED.
    Passable(u8),
}

/// One cell of a map, every plane's byte for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cell {
    /// The two-bit wall value of each side, in `SIDE_ORDER_INFERRED` order.
    pub(crate) walls: [u8; 4],
    pub(crate) side_kinds: [u8; 4],
    /// The script event of each side, plus one; 0 is none.
    pub(crate) side_events: [u8; 4],
    pub(crate) enter_event: u8,
    pub(crate) second_event: u8,
    /// A byte per cell whose meaning is unknown (values 0 to 12 seen).
    pub(crate) kind: u8,
}

impl Cell {
    pub(crate) fn edge(&self, side: Side) -> Edge {
        let index = side.index();
        let wall = self.walls.get(index).copied().unwrap_or(0);
        let kind = self.side_kinds.get(index).copied().unwrap_or(0);
        match (wall, kind) {
            (0, _) => Edge::Open,
            (wall, PASSABLE_KIND) => Edge::Passable(wall),
            (wall, _) => Edge::Wall(wall),
        }
    }

    pub(crate) fn has_event(&self) -> bool {
        self.enter_event != 0 || self.second_event != 0 || self.side_events.iter().any(|e| *e != 0)
    }
}

/// One map: its header bytes and 256 cells, `row * 16 + col`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MapPlan {
    pub(crate) header: [u8; HEADER_BYTES],
    pub(crate) cells: Vec<Cell>,
}

impl MapPlan {
    pub(crate) fn cell(&self, index: usize) -> Option<&Cell> {
        self.cells.get(index)
    }

    /// Cells with at least one wall or event: the part of the grid the map uses.
    pub(crate) fn used_cells(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| cell.walls.iter().any(|w| *w != 0) || cell.has_event())
            .count()
    }
}

/// Why a file is not the maps file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DungeonError {
    Size(usize),
}

impl std::fmt::Display for DungeonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DungeonError::Size(size) => write!(f, "{size} bytes, not {FILE_BYTES}"),
        }
    }
}

/// Every map of a game, in the order of the game's map table (the automap's order).
#[derive(Clone, Debug)]
pub(crate) struct Dungeon {
    pub(crate) maps: Vec<MapPlan>,
}

fn plane(block: &[u8], at: usize, cell: usize) -> u8 {
    block.get(at + cell).copied().unwrap_or(0)
}

fn parse_block(block: &[u8]) -> MapPlan {
    let mut header = [0u8; HEADER_BYTES];
    for (to, from) in header.iter_mut().zip(block) {
        *to = *from;
    }
    let cells = (0..CELLS)
        .map(|cell| {
            let wall_byte = plane(block, WALLS, cell);
            let sides =
                |base: usize| [0, 1, 2, 3].map(|side| plane(block, base + side * PLANE, cell));
            Cell {
                walls: [0, 1, 2, 3].map(|side| (wall_byte >> (2 * side)) & 3),
                side_kinds: sides(SIDE_KINDS),
                side_events: sides(SIDE_EVENTS),
                enter_event: plane(block, ENTER_EVENTS, cell),
                second_event: plane(block, SECOND_EVENTS, cell),
                kind: plane(block, CELL_KINDS, cell),
            }
        })
        .collect();
    MapPlan { header, cells }
}

impl Dungeon {
    /// Parses the file as it lies in the game's folder.
    pub(crate) fn parse(stored: &[u8]) -> Result<Dungeon, DungeonError> {
        if stored.len() != FILE_BYTES {
            return Err(DungeonError::Size(stored.len()));
        }
        let maps = stored
            .chunks_exact(BLOCK_BYTES)
            .map(|block| {
                parse_block(&unrotate(
                    block.get(..READ_BYTES).unwrap_or(block),
                    ROTATION,
                ))
            })
            .collect();
        Ok(Dungeon { maps })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ybarra::game_file::tests::rotate;

    /// A decoded block with a room of `width` by `height` cells walled round, a door on the
    /// north side of cell 0x12 and an entry event on cell 0x22.
    pub(crate) fn room_block(width: usize, height: usize) -> Vec<u8> {
        let mut block = vec![0u8; BLOCK_BYTES];
        block[0] = 1;
        for row in 0..height {
            for col in 0..width {
                let mut walls = 0u8;
                let mut wall = |side: Side, on: bool| {
                    if on {
                        walls |= 1 << (2 * side.index());
                    }
                };
                wall(Side::North, row == 0);
                wall(Side::South, row + 1 == height);
                wall(Side::West, col == 0);
                wall(Side::East, col + 1 == width);
                block[WALLS + row * 16 + col] = walls;
            }
        }
        block[WALLS + 0x12] |= 2 << (2 * Side::North.index());
        block[WALLS + 0x02] |= 2 << (2 * Side::South.index());
        block[SIDE_KINDS + Side::North.index() * PLANE + 0x12] = PASSABLE_KIND;
        block[ENTER_EVENTS + 0x22] = 4;
        block
    }

    /// A whole file as the games store it, every map the same room.
    pub(crate) fn stored_file(width: usize, height: usize) -> Vec<u8> {
        let block = room_block(width, height);
        let mut stored = Vec::with_capacity(FILE_BYTES);
        for _ in 0..MAPS {
            stored.extend(rotate(&block[..READ_BYTES], ROTATION));
            stored.extend(&block[READ_BYTES..]);
        }
        stored
    }

    #[test]
    fn walls_doors_and_events_come_out_by_side() {
        let dungeon = Dungeon::parse(&stored_file(4, 3)).unwrap();
        assert_eq!(dungeon.maps.len(), MAPS);
        let map = &dungeon.maps[5];
        assert_eq!(map.header[0], 1);
        let corner = map.cell(0).unwrap();
        assert_eq!(corner.edge(Side::North), Edge::Wall(1));
        assert_eq!(corner.edge(Side::West), Edge::Wall(1));
        assert_eq!(corner.edge(Side::East), Edge::Open);
        let door = map.cell(0x12).unwrap();
        assert_eq!(door.edge(Side::North), Edge::Passable(2));
        assert_eq!(map.cell(0x02).unwrap().edge(Side::South), Edge::Wall(2));
        assert!(map.cell(0x22).unwrap().has_event());
        assert_eq!(map.cell(0x22).unwrap().enter_event, 4);
        assert_eq!(map.cell(0x23).unwrap().edge(Side::East), Edge::Wall(1));
        assert_eq!(map.used_cells(), 11);
        assert_eq!(Dungeon::parse(&[0; 4]).unwrap_err(), DungeonError::Size(4));
    }

    #[test]
    fn sides_index_their_bit_fields_once_each() {
        for (index, side) in SIDE_ORDER_INFERRED.iter().enumerate() {
            assert_eq!(side.index(), index);
        }
    }
}
