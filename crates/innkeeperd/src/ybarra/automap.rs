//! The automap file of the Ybarra games: per character slot, 34 maps of 800 bytes, each a 16 by 16
//! grid of cells followed by its wall planes; 0xFF is unexplored. The three games share the layout
//! (docs/server/ybarra.md section 6).

use std::ops::Range;

/// The file's size; the slots use the first `SLOTS * SLOT_BYTES` of it.
pub(crate) const FILE_BYTES: usize = 168_000;
pub(crate) const MAPS: usize = 34;
pub(crate) const BLOCK_BYTES: usize = 800;
pub(crate) const SLOT_BYTES: usize = MAPS * BLOCK_BYTES;
pub(crate) const SIDE: usize = 16;
pub(crate) const CELLS: usize = SIDE * SIDE;
pub(crate) const UNEXPLORED: u8 = 0xFF;
/// What the games store for a visited cell: they turn every other kept value into it on save.
pub(crate) const VISITED: u8 = 0;

/// Why a stored automap file cannot be edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomapError {
    Size(usize),
    NoSuchMap,
}

/// Where one map of one character slot lies in the file.
pub(crate) fn block(slot: usize, map: usize) -> Result<Range<usize>, AutomapError> {
    if slot >= super::SLOTS || map >= MAPS {
        return Err(AutomapError::NoSuchMap);
    }
    let at = slot * SLOT_BYTES + map * BLOCK_BYTES;
    Ok(at..at + BLOCK_BYTES)
}

/// One map's 800 bytes in a file, checked for size.
pub(crate) fn map_bytes(file: &[u8], slot: usize, map: usize) -> Result<&[u8], AutomapError> {
    if file.len() != FILE_BYTES {
        return Err(AutomapError::Size(file.len()));
    }
    file.get(block(slot, map)?).ok_or(AutomapError::NoSuchMap)
}

pub(crate) fn map_bytes_mut(
    file: &mut [u8],
    slot: usize,
    map: usize,
) -> Result<&mut [u8], AutomapError> {
    if file.len() != FILE_BYTES {
        return Err(AutomapError::Size(file.len()));
    }
    let range = block(slot, map)?;
    file.get_mut(range).ok_or(AutomapError::NoSuchMap)
}

/// Cells of a map that are not unexplored.
pub(crate) fn explored(map: &[u8]) -> usize {
    map.iter()
        .take(CELLS)
        .filter(|cell| **cell != UNEXPLORED)
        .count()
}

/// Forgets everything about a map, cells and walls, as a new character starts.
pub(crate) fn forget(map: &mut [u8]) {
    map.fill(UNEXPLORED);
}

/// Sets which cells are explored. A newly explored cell becomes a visited floor and a cell that
/// stays explored keeps its value; the wall planes are left alone (the game fills them in as the
/// player walks). Returns how many cells changed.
pub(crate) fn set_explored(map: &mut [u8], wanted: &[bool; CELLS]) -> usize {
    let mut changed = 0;
    for (cell, want) in map.iter_mut().zip(wanted) {
        let now = *cell != UNEXPLORED;
        if now != *want {
            *cell = if *want { VISITED } else { UNEXPLORED };
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_stride_by_slot_and_map_and_stay_inside_the_file() {
        assert_eq!(block(0, 0).unwrap(), 0..800);
        assert_eq!(block(1, 2).unwrap().start, (34 + 2) * 800);
        assert!(block(5, 33).unwrap().end <= FILE_BYTES);
        assert_eq!(block(6, 0), Err(AutomapError::NoSuchMap));
        assert_eq!(block(0, 34), Err(AutomapError::NoSuchMap));
        assert_eq!(map_bytes(&[0; 10], 0, 0), Err(AutomapError::Size(10)));
    }

    #[test]
    fn exploring_and_forgetting_cells() {
        let mut file = vec![UNEXPLORED; FILE_BYTES];
        let map = map_bytes_mut(&mut file, 2, 5).unwrap();
        map[3] = 12;
        let mut wanted = [false; CELLS];
        wanted[3] = true;
        wanted[17] = true;
        assert_eq!(set_explored(map, &wanted), 1);
        assert_eq!((map[3], map[17]), (12, VISITED));
        assert_eq!(explored(map), 2);
        assert_eq!(set_explored(map, &[false; CELLS]), 2);
        assert_eq!(explored(map), 0);
        map[CELLS + 4] = 1;
        forget(map);
        assert!(map.iter().all(|byte| *byte == UNEXPLORED));
        assert!(file.iter().all(|byte| *byte == UNEXPLORED));
    }
}
