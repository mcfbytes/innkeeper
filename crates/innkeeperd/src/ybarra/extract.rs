//! Finds a Ybarra game's cipher tables and key in one of its programs, so that they come from the
//! operator's own copy and never from this source (docs/server/ybarra.md section 3). Any of the
//! six programs works: the galleries (`CGENN.EXE`, `TWGENN.EXE` of Twinion or of Cawdor) and the
//! games (`DARKSTRT.EXE`, `FATES.EXE`, `RUINS.EXE`).
//!
//! The program is a DOS MZ executable. In it the cipher routine starts by loading its data segment
//! (`push bp; mov bp,sp; push ds; push es; mov ax,DS; mov ds,ax; push di; push si; mov ax,[bp+8];
//! mov cs:[..],ax`), copies its masks from a code-segment table (`mov ax,cs:[bx+MASKS]; mov
//! cs:[bx+..],ax`), and is called far as `push KEY; push ds; push TABLE; push ds; call far ROUTINE`.
//! The key's four words are stored one after another (`mov word [KEY+n],imm` four times) just
//! before the call.

use super::cipher::{Cipher, KEY_BYTES, MASKS, SHUFFLE_BYTES};
use super::Game;

/// What was found, with where, for the operator's report.
#[derive(Debug)]
pub(crate) struct Found {
    pub(crate) game: Game,
    pub(crate) cipher: Cipher,
}

/// Why a file gave no cipher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExtractError {
    NotAnExe,
    /// None of the three character file names is in it.
    NotAYbarraGame,
    NoRoutine,
    NoCall,
    NoKey,
    OutOfFile,
    BadTables,
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ExtractError::NotAnExe => "the file is not a DOS program",
            ExtractError::NotAYbarraGame => {
                "the program names no Yserbius, Twinion or Cawdor character file"
            }
            ExtractError::NoRoutine => "the program has no character cipher",
            ExtractError::NoCall => "the cipher's call was not found",
            ExtractError::NoKey => "the cipher's key was not found",
            ExtractError::OutOfFile => "a table points outside the program",
            ExtractError::BadTables => "the tables found do not make a working cipher",
        })
    }
}

/// The routine's first bytes; `?` is the data segment word.
const PROLOGUE: [Option<u8>; 17] = [
    Some(0x55),
    Some(0x8b),
    Some(0xec),
    Some(0x1e),
    Some(0x06),
    Some(0xb8),
    None,
    None,
    Some(0x8e),
    Some(0xd8),
    Some(0x57),
    Some(0x56),
    Some(0x8b),
    Some(0x46),
    Some(0x08),
    Some(0x2e),
    Some(0xa3),
];
/// How far into the routine the mask copy is looked for.
const ROUTINE_SPAN: usize = 0x200;
/// How far before the far call the key stores are looked for.
const KEY_SPAN: usize = 0x80;

fn word(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

fn has(bytes: &[u8], at: usize, want: &[u8]) -> bool {
    bytes.get(at..at + want.len()) == Some(want)
}

fn matches(bytes: &[u8], at: usize, pattern: &[Option<u8>]) -> bool {
    pattern
        .iter()
        .enumerate()
        .all(|(i, want)| want.is_none_or(|want| bytes.get(at + i) == Some(&want)))
}

fn game_of(bytes: &[u8]) -> Result<Game, ExtractError> {
    let lower = bytes.to_ascii_lowercase();
    let named = |name: &[u8]| lower.windows(name.len()).any(|window| window == name);
    // Twinion's file name is checked first: no program names two of them.
    for game in [Game::Twinion, Game::Cawdor, Game::Yserbius] {
        if named(game.chars_stem()) {
            return Ok(game);
        }
    }
    Err(ExtractError::NotAYbarraGame)
}

/// The far call to the routine at image offset `routine`, with what it pushes.
struct CallSite {
    at: usize,
    /// The routine's offset in its code segment.
    offset: u16,
    key_at: u16,
    table_at: u16,
}

/// `push KEY; push ds; push TABLE; push ds; call far ROUTINE`, written as
/// `b8 KEY 50 1e b8 TABLE 50 1e 9a OFFSET SEGMENT`.
fn find_call(image: &[u8], routine: usize) -> Option<CallSite> {
    (10..image.len()).find_map(|at| {
        let offset = word(image, at + 1)?;
        let segment = word(image, at + 3)?;
        let pushes = has(image, at, &[0x9a])
            && usize::from(segment) * 16 + usize::from(offset) == routine
            && has(image, at - 10, &[0xb8])
            && has(image, at - 7, &[0x50, 0x1e, 0xb8])
            && has(image, at - 2, &[0x50, 0x1e]);
        pushes.then_some(CallSite {
            at,
            offset,
            key_at: word(image, at - 9)?,
            table_at: word(image, at - 4)?,
        })
    })
}

/// The code-segment offset of the masks: `mov ax,cs:[bx+MASKS]; mov cs:[bx+..],ax` in the routine.
fn find_masks(image: &[u8], routine: usize) -> Option<u16> {
    (routine..routine + ROUTINE_SPAN).find_map(|at| {
        (has(image, at, &[0x2e, 0x8b, 0x87]) && has(image, at + 5, &[0x2e, 0x89, 0x87]))
            .then(|| word(image, at + 3))
            .flatten()
    })
}

/// Reads the cipher out of a game program.
pub(crate) fn extract(exe: &[u8]) -> Result<Found, ExtractError> {
    if exe.len() < 0x40 || !has(exe, 0, b"MZ") {
        return Err(ExtractError::NotAnExe);
    }
    let game = game_of(exe)?;
    let header = usize::from(word(exe, 8).ok_or(ExtractError::NotAnExe)?) * 16;
    let image = exe.get(header..).ok_or(ExtractError::NotAnExe)?;

    // The first routine is the encryption; the decryption follows it with the same tables.
    let routine = (0..image.len())
        .find(|at| matches(image, *at, &PROLOGUE))
        .ok_or(ExtractError::NoRoutine)?;
    let data_segment = usize::from(word(image, routine + 6).ok_or(ExtractError::NoRoutine)?);
    let call = find_call(image, routine).ok_or(ExtractError::NoCall)?;
    let masks_at = find_masks(image, routine).ok_or(ExtractError::NoRoutine)?;
    let key = find_key(image, call.at, call.key_at).ok_or(ExtractError::NoKey)?;

    let shuffle_from = data_segment * 16 + usize::from(call.table_at);
    let masks_from = routine - usize::from(call.offset) + usize::from(masks_at);
    let shuffle = image
        .get(shuffle_from..shuffle_from + SHUFFLE_BYTES)
        .ok_or(ExtractError::OutOfFile)?;
    let masks = image
        .get(masks_from..masks_from + MASKS * 4)
        .ok_or(ExtractError::OutOfFile)?;
    let cipher = Cipher::new(shuffle, masks, &key).map_err(|_| ExtractError::BadTables)?;
    Ok(Found { game, cipher })
}

/// The four `mov word [key+2n], imm` before the call; the stores nearest the call win when the
/// key is set on more than one path.
fn find_key(image: &[u8], call: usize, key_at: u16) -> Option<[u8; KEY_BYTES]> {
    (call.saturating_sub(KEY_SPAN)..call).rev().find_map(|at| {
        let mut key = Vec::with_capacity(KEY_BYTES);
        for n in 0..4u16 {
            let store = at + usize::from(n) * 6;
            let target = key_at.wrapping_add(n * 2);
            if !has(image, store, &[0xc7, 0x06]) || word(image, store + 2) != Some(target) {
                return None;
            }
            key.extend_from_slice(&word(image, store + 4)?.to_le_bytes());
        }
        key.try_into().ok()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ybarra::cipher::tests::sample;

    /// A made-up program laid out the way the games are: the tables, the routine and the call.
    pub(crate) fn fake_exe(game: Game, cipher: &Cipher) -> Vec<u8> {
        let header = 0x20usize; // two paragraphs
        let mut image = vec![0x90u8; 0x3000];
        // Code segment 0x0010 (base 0x100): the routine at offset 0x40, masks at offset 0x400.
        let routine = 0x140;
        let mut code = vec![0x55, 0x8b, 0xec, 0x1e, 0x06, 0xb8, 0x00, 0x02, 0x8e, 0xd8];
        code.extend_from_slice(&[0x57, 0x56, 0x8b, 0x46, 0x08, 0x2e, 0xa3, 0x00, 0x03]);
        code.extend_from_slice(&[
            0xbb, 0x7e, 0x00, 0x2e, 0x8b, 0x87, 0x00, 0x04, 0x2e, 0x89, 0x87, 0x80, 0x04,
        ]);
        image[routine..routine + code.len()].copy_from_slice(&code);
        image[0x500..0x580].copy_from_slice(&cipher.mask_bytes());
        // Data segment 0x0200 (base 0x2000): the key at 0x10, the shuffle table at 0x100.
        image[0x2100..0x2100 + SHUFFLE_BYTES].copy_from_slice(&cipher.shuffle);
        let mut site = Vec::new();
        for n in 0..4u16 {
            site.extend_from_slice(&[0xc7, 0x06]);
            site.extend_from_slice(&(0x10 + n * 2).to_le_bytes());
            let at = usize::from(n) * 2;
            site.extend_from_slice(&[cipher.key[at], cipher.key[at + 1]]);
        }
        site.extend_from_slice(&[0xb8, 0x10, 0x00, 0x50, 0x1e, 0xb8, 0x00, 0x01, 0x50, 0x1e]);
        site.extend_from_slice(&[0x9a, 0x40, 0x00, 0x10, 0x00]);
        image[0x800..0x800 + site.len()].copy_from_slice(&site);
        let name = format!("{}.dat", String::from_utf8_lossy(game.chars_stem()));
        image[0x2800..0x2800 + name.len()].copy_from_slice(name.as_bytes());
        let mut exe = vec![0u8; header];
        exe[..2].copy_from_slice(b"MZ");
        exe[8..10].copy_from_slice(&2u16.to_le_bytes());
        exe.extend_from_slice(&image);
        exe
    }

    #[test]
    fn finds_the_tables_and_the_key_of_a_game_shaped_program() {
        let cipher = sample();
        let found = extract(&fake_exe(Game::Cawdor, &cipher)).unwrap();
        assert_eq!(found.game, Game::Cawdor);
        assert_eq!(found.cipher, cipher);
    }

    #[test]
    fn refuses_other_files() {
        assert_eq!(extract(b"hello").unwrap_err(), ExtractError::NotAnExe);
        let mut exe = fake_exe(Game::Yserbius, &sample());
        let at = exe.windows(7).position(|w| w == b"mychars").unwrap();
        exe[at] = b'x';
        assert_eq!(extract(&exe).unwrap_err(), ExtractError::NotAYbarraGame);
    }

    /// The real programs, when a copy is at hand: `INNKEEPER_YBARRA_DIR` names a directory with
    /// `YSERBIUS`, `TWINION` and `CAWDOR` folders (any that exist are tried).
    #[test]
    fn real_programs_when_present() {
        let Ok(dir) = std::env::var("INNKEEPER_YBARRA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        for (folder, files, game) in [
            ("YSERBIUS", &["CGENN.EXE", "DARKSTRT.EXE"], Game::Yserbius),
            ("TWINION", &["TWGENN.EXE", "FATES.EXE"], Game::Twinion),
            ("CAWDOR", &["TWGENN.EXE", "RUINS.EXE"], Game::Cawdor),
        ] {
            let mut ciphers = Vec::new();
            for file in files {
                let Ok(bytes) = std::fs::read(dir.join(folder).join(file)) else {
                    continue;
                };
                let found = extract(&bytes).unwrap_or_else(|e| panic!("{folder}\\{file}: {e}"));
                assert_eq!(found.game, game, "{folder}\\{file}");
                ciphers.push(found.cipher);
            }
            assert!(
                ciphers.windows(2).all(|pair| pair[0] == pair[1]),
                "{folder}"
            );
            // A save beside them (`saves/<file>`) reads as a named character and seals back.
            let file = game.chars_path().rsplit('\\').next().unwrap();
            let (Some(cipher), Ok(bytes)) =
                (ciphers.first(), std::fs::read(dir.join("saves").join(file)))
            else {
                continue;
            };
            let chars = crate::ybarra::CharFile::open(cipher, &bytes).unwrap();
            let name = &chars.slots[0][8..12];
            assert!(
                name.iter().all(|b| b.is_ascii_graphic() || *b == b' '),
                "{file}: {name:?}"
            );
            assert_eq!(chars.seal(cipher), bytes, "{file}");
        }
    }
}
