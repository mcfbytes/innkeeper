//! Game save files kept per account for the browser client: which paths may be stored, how large
//! they may be, and what the store remembers about each one. Design in docs/server/saves.md.

use sha2::{Digest, Sha256};

/// The most bytes one stored file may have. `AUTOMAPS.DAT` is 168,000 bytes.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// The most bytes the current versions of one account's files may add up to.
pub const MAX_ACCOUNT_BYTES: usize = 8 * 1024 * 1024;
/// The most files one account may keep.
pub const MAX_ACCOUNT_FILES: usize = 64;
/// Versions kept per file, the current one included.
pub const KEPT_VERSIONS: usize = 3;
/// Longest path text the server looks at.
const MAX_PATH_CHARS: usize = 64;

/// Drive-relative paths (upper case, `\` separators) the server stores besides the personas
/// (`*.DTA` in `PERSONA_DIRS`) and the CasinoLand bankroll (`INN\LL$$<digits>.DAT`).
const FIXED_PATHS: [&str; 7] = [
    "INN\\YSERBIUS\\MYCHARS.DAT",
    "INN\\YSERBIUS\\AUTOMAPS.DAT",
    "INN\\TWINION\\TWINCHAR.DAT",
    "INN\\TWINION\\AUTOMAPS.DAT",
    "INN\\PLAYER.DIR",
    "INN\\LL\\PLAYER.DIR",
    "INN\\SL\\PLAYER.DIR",
];
/// The password file is never stored.
const PASSWORD_FILE_STEM: &str = "PASS_SET";
/// Where personas live: the hub's, CasinoLand's (`LL`) and SierraLand's (`SL`).
const PERSONA_DIRS: [&str; 3] = ["INN\\", "INN\\LL\\", "INN\\SL\\"];
/// CasinoLand keeps the bankroll in `INN\LL$$<digits>.DAT`, named from the account number.
const BANKROLL_PREFIX: &str = "INN\\LL$$";
const BANKROLL_SUFFIX: &str = ".DAT";
const PERSONA_SUFFIX: &str = ".DTA";
const DOS_STEM_MAX: usize = 8;

/// The metadata of one stored version of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveMeta {
    /// Canonical path: upper case, `\` separators, relative to the drive root.
    pub path: String,
    /// Counts from 1 per file and is never reused.
    pub version: u64,
    /// Lower-case hex SHA-256 of the content.
    pub hash: String,
    pub size: usize,
    /// When the content was last modified, in milliseconds since the Unix epoch, as the uploader
    /// said (the host clamps it to its own clock).
    pub modified: u64,
    /// True for the version a download returns; the others are kept history.
    pub current: bool,
}

/// Why a path is not stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRefusal {
    /// Empty, too long, with a character DOS does not allow here, or `..`.
    Malformed,
    /// A well-formed path that is not one of the save files.
    NotASaveFile,
    /// The password file.
    PasswordFile,
}

/// The lower-case hex SHA-256 of `data`, the hash in a `SaveMeta` and in the manifest.
pub fn hash_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The canonical form of a save file path, or why it is refused. Case and the kind of slash do not
/// matter; a leading `\` is dropped.
pub fn canonical_path(raw: &str) -> Result<String, PathRefusal> {
    if raw.is_empty() || raw.chars().count() > MAX_PATH_CHARS {
        return Err(PathRefusal::Malformed);
    }
    let path = raw.replace('/', "\\").to_ascii_uppercase();
    let path = path.strip_prefix('\\').unwrap_or(&path);
    let well_formed = path.split('\\').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"_$-.".contains(&b))
    });
    if !well_formed {
        return Err(PathRefusal::Malformed);
    }
    if FIXED_PATHS.contains(&path) {
        return Ok(path.to_owned());
    }
    let bankroll = path
        .strip_prefix(BANKROLL_PREFIX)
        .and_then(|name| name.strip_suffix(BANKROLL_SUFFIX))
        .is_some_and(|digits| {
            !digits.is_empty()
                && digits.len() + 4 <= DOS_STEM_MAX
                && digits.bytes().all(|b| b.is_ascii_digit())
        });
    if bankroll {
        return Ok(path.to_owned());
    }
    let stem = PERSONA_DIRS.iter().find_map(|dir| {
        path.strip_prefix(dir)
            .and_then(|name| name.strip_suffix(PERSONA_SUFFIX))
            .filter(|stem| !stem.is_empty() && stem.len() <= DOS_STEM_MAX && !stem.contains('\\'))
    });
    match stem {
        Some(PASSWORD_FILE_STEM) => Err(PathRefusal::PasswordFile),
        Some(_) => Ok(path.to_owned()),
        _ => Err(PathRefusal::NotASaveFile),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_listed_files_are_stored_whatever_their_spelling() {
        for (raw, canonical) in [
            ("INN\\YSERBIUS\\MYCHARS.DAT", "INN\\YSERBIUS\\MYCHARS.DAT"),
            ("inn/yserbius/automaps.dat", "INN\\YSERBIUS\\AUTOMAPS.DAT"),
            ("\\Inn\\Twinion\\TwinChar.dat", "INN\\TWINION\\TWINCHAR.DAT"),
            ("INN\\TWINION\\AUTOMAPS.DAT", "INN\\TWINION\\AUTOMAPS.DAT"),
            ("inn\\player.dir", "INN\\PLAYER.DIR"),
            ("Inn\\Guybrush.dta", "INN\\GUYBRUSH.DTA"),
            ("INN\\LL\\LUCKY.DTA", "INN\\LL\\LUCKY.DTA"),
            ("inn/ll/player.dir", "INN\\LL\\PLAYER.DIR"),
            ("INN\\SL\\SUNNY.DTA", "INN\\SL\\SUNNY.DTA"),
            ("INN\\SL\\PLAYER.DIR", "INN\\SL\\PLAYER.DIR"),
            ("inn\\ll$$3446.dat", "INN\\LL$$3446.DAT"),
        ] {
            assert_eq!(canonical_path(raw).unwrap(), canonical, "{raw}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        let refused = |raw| canonical_path(raw).unwrap_err();
        assert_eq!(refused("INN\\PASS_SET.DTA"), PathRefusal::PasswordFile);
        assert_eq!(refused("inn/pass_set.dta"), PathRefusal::PasswordFile);
        assert_eq!(refused("INN\\TSNEXEC.EXE"), PathRefusal::NotASaveFile);
        assert_eq!(
            refused("INN\\YSERBIUS\\MYCHARS.NEW"),
            PathRefusal::NotASaveFile
        );
        assert_eq!(refused("INN\\SUB\\X.DTA"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\LL\\PASS_SET.DTA"), PathRefusal::PasswordFile);
        // CasinoLand's access password setting stays with the installation, as PASS_SET.DTA does.
        assert_eq!(refused("INN\\LLSYSTEM.DAT"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\LL\\SUB\\X.DTA"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\LL$$.DAT"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\LL$$12345.DAT"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\LL$$12A.DAT"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\LL\\LL$$3446.DAT"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\TOOLONGNAME.DTA"), PathRefusal::NotASaveFile);
        assert_eq!(refused("INN\\.DTA"), PathRefusal::NotASaveFile);
        assert_eq!(refused(""), PathRefusal::Malformed);
        assert_eq!(refused("INN\\..\\INN\\PLAYER.DIR"), PathRefusal::Malformed);
        assert_eq!(refused("INN\\\\PLAYER.DIR"), PathRefusal::Malformed);
        assert_eq!(refused("C:\\INN\\PLAYER.DIR"), PathRefusal::Malformed);
        assert_eq!(refused("INN\\PLAYER.DIR\0"), PathRefusal::Malformed);
        assert_eq!(refused(&"A".repeat(200)), PathRefusal::Malformed);
    }

    #[test]
    fn the_hash_is_sha256_in_hex() {
        assert_eq!(
            hash_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
