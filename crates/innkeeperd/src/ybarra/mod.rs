//! The save files of the three Ybarra games (The Shadow of Yserbius, The Fates of Twinion, The
//! Ruins of Cawdor) as the operator portal's character editor reads and writes them: the
//! character file's layout and cipher, the field maps and the automaps. docs/server/ybarra.md.

pub(crate) mod automap;
pub(crate) mod cipher;
pub(crate) mod extract;
pub(crate) mod schema;

use cipher::{Cipher, SEALED, SEALED_AT};
use innkeeper_world::saves::hash_hex;

use crate::admin::store::cipher_put;
use crate::sqlite_store::SqliteStore;

/// The biggest program accepted for an import; `RUINS.EXE` is 376,464 bytes.
pub(crate) const MAX_PROGRAM_BYTES: usize = 1024 * 1024;

/// Reads the cipher out of a game program and keeps it for that game. The program itself is not
/// kept; only its hash is, to say which copy the tables came from.
pub(crate) fn import(store: &SqliteStore, program: &[u8], now: i64) -> Result<Game, String> {
    if program.len() > MAX_PROGRAM_BYTES {
        return Err("the file is larger than any of the game programs".to_owned());
    }
    let found = extract::extract(program).map_err(|error| error.to_string())?;
    cipher_put(store, found.game, &found.cipher, &hash_hex(program), now)
        .map_err(|error| error.to_string())?;
    Ok(found.game)
}

/// One of the three games.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Game {
    Yserbius,
    Twinion,
    Cawdor,
}

/// Bytes of a character file: the header and six slots.
pub(crate) const CHARS_FILE_BYTES: usize = HEADER_BYTES + SLOTS * SLOT_BYTES;
pub(crate) const HEADER_BYTES: usize = 0x40;
pub(crate) const SLOTS: usize = 6;
pub(crate) const SLOT_BYTES: usize = 0x1F6;

impl Game {
    pub(crate) const ALL: [Game; 3] = [Game::Yserbius, Game::Twinion, Game::Cawdor];

    /// The name the store and the URLs use.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Game::Yserbius => "yserbius",
            Game::Twinion => "twinion",
            Game::Cawdor => "cawdor",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Game> {
        Game::ALL.into_iter().find(|game| game.id() == id)
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Game::Yserbius => "The Shadow of Yserbius",
            Game::Twinion => "The Fates of Twinion",
            Game::Cawdor => "The Ruins of Cawdor",
        }
    }

    /// The character file, as the save store names it.
    pub(crate) fn chars_path(self) -> &'static str {
        match self {
            Game::Yserbius => "INN\\YSERBIUS\\MYCHARS.DAT",
            Game::Twinion => "INN\\TWINION\\TWINCHAR.DAT",
            Game::Cawdor => "INN\\CAWDOR\\CAWCHAR.DAT",
        }
    }

    pub(crate) fn automaps_path(self) -> &'static str {
        match self {
            Game::Yserbius => "INN\\YSERBIUS\\AUTOMAPS.DAT",
            Game::Twinion => "INN\\TWINION\\AUTOMAPS.DAT",
            Game::Cawdor => "INN\\CAWDOR\\AUTOMAPS.DAT",
        }
    }

    /// The character file's name in lower case without the extension, as the programs spell it.
    pub(crate) fn chars_stem(self) -> &'static [u8] {
        match self {
            Game::Yserbius => b"mychars",
            Game::Twinion => b"twinchar",
            Game::Cawdor => b"cawchar",
        }
    }
}

/// A character file taken apart: the plain header and each slot's record with the sealed part
/// opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CharFile {
    pub(crate) header: [u8; HEADER_BYTES],
    pub(crate) slots: [[u8; SLOT_BYTES]; SLOTS],
}

/// Why a stored character file cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CharFileError {
    /// Not 3076 bytes.
    Size(usize),
}

/// The sealed part of a slot's record.
fn sealed_part(slot: &mut [u8; SLOT_BYTES]) -> Option<&mut [u8; SEALED]> {
    slot.get_mut(SEALED_AT..SEALED_AT + SEALED)?.try_into().ok()
}

impl CharFile {
    pub(crate) fn open(cipher: &Cipher, bytes: &[u8]) -> Result<CharFile, CharFileError> {
        let size = CharFileError::Size(bytes.len());
        if bytes.len() != CHARS_FILE_BYTES {
            return Err(size);
        }
        let (header, rest) = bytes.split_at(HEADER_BYTES);
        let header: [u8; HEADER_BYTES] = header.try_into().map_err(|_| size)?;
        let mut slots = [[0u8; SLOT_BYTES]; SLOTS];
        for (slot, stored) in slots.iter_mut().zip(rest.chunks_exact(SLOT_BYTES)) {
            slot.copy_from_slice(stored);
            if let Some(sealed) = sealed_part(slot) {
                cipher.open(sealed);
            }
        }
        Ok(CharFile { header, slots })
    }

    pub(crate) fn seal(&self, cipher: &Cipher) -> Vec<u8> {
        let mut out = Vec::with_capacity(CHARS_FILE_BYTES);
        out.extend_from_slice(&self.header);
        for slot in &self.slots {
            let mut slot = *slot;
            if let Some(sealed) = sealed_part(&mut slot) {
                cipher.seal(sealed);
            }
            out.extend_from_slice(&slot);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ybarra::cipher::tests::sample;

    #[test]
    fn a_character_file_reads_back_to_the_same_bytes() {
        let cipher = sample();
        let mut bytes: Vec<u8> = (0..CHARS_FILE_BYTES)
            .map(|n| (n * 13 % 256) as u8)
            .collect();
        bytes[0] = 2;
        let file = CharFile::open(&cipher, &bytes).unwrap();
        assert_eq!(file.header[0], 2);
        assert_eq!(file.seal(&cipher), bytes);
        assert_eq!(
            CharFile::open(&cipher, &bytes[1..]),
            Err(CharFileError::Size(CHARS_FILE_BYTES - 1))
        );
    }

    #[test]
    fn games_round_trip_their_ids() {
        for game in Game::ALL {
            assert_eq!(Game::from_id(game.id()), Some(game));
        }
        assert_eq!(Game::from_id("golf"), None);
    }
}
