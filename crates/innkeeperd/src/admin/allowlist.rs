//! Who may enter the portal: the staff allowlist from the vault secret, a file and the `staff`
//! table (docs/server/innkeeperd.md section 9).
//!
//! The rule, in one place:
//!
//! 1. A `staff` row that is *revoked* shuts the identity out, whatever else lists it.
//! 2. Otherwise an identity is in when the vault secret, the file or an active `staff` row lists it.
//! 3. When no source exists at all (no vault list, no file, no `staff` rows), the door is open to any
//!    signed-in identity: the development fallback. `--admin-dev-open` forces it. `--public`
//!    refuses to start in that state.

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use thiserror::Error;
use tracing::warn;

use super::store;
use crate::signin::keys::IdentityKey;
use crate::sqlite_store::SqliteStore;
use innkeeper_world::StoreError;

#[derive(Debug, Error)]
pub(crate) enum AllowlistError {
    #[error("{source_name} line {line}: {text:?} is not an identity key (64 hex digits)")]
    Malformed {
        source_name: String,
        line: usize,
        text: String,
    },
    #[error("cannot read the admin allowlist {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
}

/// The keys in `text`, and the entries that were not keys. Newlines and commas separate entries;
/// `#` starts a comment that runs to the end of the line.
pub(crate) fn parse(text: &str) -> (HashSet<IdentityKey>, Vec<(usize, String)>) {
    let mut keys = HashSet::new();
    let mut bad = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let content = line.split('#').next().unwrap_or("");
        for entry in content.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            match parse_key(entry) {
                Some(key) => {
                    keys.insert(key);
                }
                None => bad.push((index + 1, entry.to_owned())),
            }
        }
    }
    (keys, bad)
}

/// Strict: any entry that is not a key is an error. For startup, where a typo should be loud.
pub(crate) fn parse_strict(
    source_name: &str,
    text: &str,
) -> Result<HashSet<IdentityKey>, AllowlistError> {
    let (keys, bad) = parse(text);
    match bad.into_iter().next() {
        None => Ok(keys),
        Some((line, text)) => Err(AllowlistError::Malformed {
            source_name: source_name.to_owned(),
            line,
            // Never echo a long line back; a key that is nearly right is still a secret-ish value.
            text: text.chars().take(12).collect(),
        }),
    }
}

/// One identity key from 64 hex digits, either case.
pub(crate) fn parse_key(text: &str) -> Option<IdentityKey> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (slot, pair) in key.iter_mut().zip(text.as_bytes().chunks(2)) {
        let digits = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(digits, 16).ok()?;
    }
    Some(key)
}

pub(crate) fn hex(key: &IdentityKey) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What the check decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Listed; the key that matched (an older HMAC version's, after a rotation, is kept as is).
    Allowed(IdentityKey),
    /// Nobody is configured, so the development fallback lets the identity in.
    Open(IdentityKey),
    Denied,
}

struct Cached {
    stamp: Option<(SystemTime, u64)>,
    keys: HashSet<IdentityKey>,
}

/// The allowlist sources and the rule that merges them.
pub(crate) struct Allowlist {
    vault: Option<HashSet<IdentityKey>>,
    file: Option<PathBuf>,
    cache: Mutex<Cached>,
    forced_open: bool,
}

impl fmt::Debug for Allowlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Allowlist")
            .field("vault_entries", &self.vault.as_ref().map(HashSet::len))
            .field("file", &self.file)
            .field("forced_open", &self.forced_open)
            .finish_non_exhaustive()
    }
}

impl Allowlist {
    /// `vault` is the parsed `admin-identities` secret when the vault has one. The file is read
    /// once here, so a malformed file stops startup.
    pub(crate) fn new(
        vault: Option<HashSet<IdentityKey>>,
        file: Option<PathBuf>,
        forced_open: bool,
    ) -> Result<Self, AllowlistError> {
        let list = Allowlist {
            vault,
            file,
            cache: Mutex::new(Cached {
                stamp: None,
                keys: HashSet::new(),
            }),
            forced_open,
        };
        if let Some(path) = &list.file {
            let text = std::fs::read_to_string(path).map_err(|source| AllowlistError::Read {
                path: path.display().to_string(),
                source,
            })?;
            parse_strict(&path.display().to_string(), &text)?;
        }
        Ok(list)
    }

    /// Whether any source is set up, ignoring the `staff` table.
    pub(crate) fn has_static_source(&self) -> bool {
        self.vault.is_some() || self.file.is_some()
    }

    /// The directory the sandbox must be able to read to follow the file as it changes.
    pub(crate) fn file_dir(&self) -> Option<PathBuf> {
        let path = self.file.as_ref()?;
        let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty());
        Some(parent.map_or_else(|| PathBuf::from("."), PathBuf::from))
    }

    /// True when the door is open to any signed-in identity right now.
    pub(crate) fn is_open(&self, store: &SqliteStore) -> Result<bool, StoreError> {
        if self.forced_open {
            return Ok(true);
        }
        if self.has_static_source() {
            return Ok(false);
        }
        Ok(store::staff_count(store)? == 0)
    }

    /// Decides for an identity known by one key per HMAC version (newest first). Called at sign-in
    /// and again on every request.
    pub(crate) fn decide(
        &self,
        store: &SqliteStore,
        candidates: &[IdentityKey],
    ) -> Result<Decision, StoreError> {
        let Some(current) = candidates.first() else {
            return Ok(Decision::Denied);
        };
        let file = self.file_keys();
        let mut revoked_any = false;
        for key in candidates {
            let row = store::staff_row(store, key)?;
            match row {
                Some(row) if row.revoked_at.is_some() => {
                    revoked_any = true;
                    continue;
                }
                Some(_) => return Ok(Decision::Allowed(*key)),
                None => {}
            }
            let in_vault = self.vault.as_ref().is_some_and(|set| set.contains(key));
            if in_vault || file.contains(key) {
                return Ok(Decision::Allowed(*key));
            }
        }
        if !revoked_any && self.is_open(store)? {
            return Ok(Decision::Open(*current));
        }
        Ok(Decision::Denied)
    }

    /// The file's keys, read again when its size or modification time changes. An unreadable file
    /// lists nobody: access ends rather than lingers.
    fn file_keys(&self) -> HashSet<IdentityKey> {
        let Some(path) = &self.file else {
            return HashSet::new();
        };
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.stamp.is_some() && cache.stamp == stamp {
            return cache.keys.clone();
        }
        let keys = match std::fs::read_to_string(path) {
            Ok(text) => {
                let (keys, bad) = parse(&text);
                if !bad.is_empty() {
                    warn!(
                        entries = bad.len(),
                        "the admin allowlist file has entries that are not identity keys; they are skipped"
                    );
                }
                keys
            }
            Err(error) => {
                warn!(%error, "the admin allowlist file cannot be read; it lists nobody");
                HashSet::new()
            }
        };
        cache.stamp = stamp;
        cache.keys.clone_from(&keys);
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> IdentityKey {
        [byte; 32]
    }

    #[test]
    fn entries_split_on_newlines_and_commas_and_comments_are_dropped() {
        let a = hex(&key(0xab));
        let b = hex(&key(0x01));
        let c = hex(&key(0xCD)).to_uppercase();
        let text = format!("# the staff\n{a}  # alice\n\n{b}, {c}\n   # only a comment\n");
        let (keys, bad) = parse(&text);
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(keys, HashSet::from([key(0xab), key(0x01), key(0xcd)]));
    }

    #[test]
    fn a_short_or_non_hex_entry_is_reported_with_its_line() {
        let good = hex(&key(7));
        let (keys, bad) = parse(&format!("{good}\nabc123\n{good}, zz{}\n", &good[2..]));
        assert_eq!(keys.len(), 1);
        assert_eq!(bad.len(), 2);
        assert_eq!(bad[0].0, 2);
        assert_eq!(bad[1].0, 3);
        let error = parse_strict("the file", "xyz\n").unwrap_err().to_string();
        assert!(error.contains("line 1"), "{error}");
        assert!(!error.contains(&good));
    }

    #[test]
    fn hex_round_trips() {
        let original = key(0x5a);
        assert_eq!(parse_key(&hex(&original)), Some(original));
        assert_eq!(parse_key(&hex(&original)[..63]), None);
    }

    fn store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    #[test]
    fn sources_merge_and_a_revoked_row_beats_them_all() {
        let store = store();
        let list = Allowlist::new(Some(HashSet::from([key(1)])), None, false).unwrap();
        assert_eq!(
            list.decide(&store, &[key(1)]).unwrap(),
            Decision::Allowed(key(1))
        );
        assert_eq!(list.decide(&store, &[key(2)]).unwrap(), Decision::Denied);
        store::staff_allow(&store, &key(2), Some("bob"), 10).unwrap();
        assert_eq!(
            list.decide(&store, &[key(2)]).unwrap(),
            Decision::Allowed(key(2))
        );
        store::staff_revoke(&store, &key(1), 20).unwrap();
        assert_eq!(
            list.decide(&store, &[key(1)]).unwrap(),
            Decision::Denied,
            "listed in the vault, revoked in the table"
        );
        store::staff_allow(&store, &key(1), None, 30).unwrap();
        assert_eq!(
            list.decide(&store, &[key(1)]).unwrap(),
            Decision::Allowed(key(1)),
            "allowing again lifts the revocation"
        );
    }

    #[test]
    fn the_file_is_followed_as_it_changes() {
        let dir = std::env::temp_dir().join(format!("inn-allow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("staff.txt");
        std::fs::write(&path, format!("{}\n", hex(&key(3)))).unwrap();
        let list = Allowlist::new(None, Some(path.clone()), false).unwrap();
        let store = store();
        assert_eq!(
            list.decide(&store, &[key(3)]).unwrap(),
            Decision::Allowed(key(3))
        );
        std::fs::write(
            &path,
            "# nobody now, and a longer comment so the size changes\n",
        )
        .unwrap();
        assert_eq!(list.decide(&store, &[key(3)]).unwrap(), Decision::Denied);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(list.decide(&store, &[key(3)]).unwrap(), Decision::Denied);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn with_no_source_the_door_is_open_until_a_staff_row_exists() {
        let store = store();
        let list = Allowlist::new(None, None, false).unwrap();
        assert_eq!(
            list.decide(&store, &[key(9)]).unwrap(),
            Decision::Open(key(9))
        );
        store::staff_allow(&store, &key(1), None, 1).unwrap();
        assert_eq!(list.decide(&store, &[key(9)]).unwrap(), Decision::Denied);
        let forced = Allowlist::new(None, None, true).unwrap();
        assert_eq!(
            forced.decide(&store, &[key(9)]).unwrap(),
            Decision::Open(key(9))
        );
    }

    #[test]
    fn an_old_key_version_still_matches_and_a_revoked_identity_is_never_open() {
        let store = store();
        let list = Allowlist::new(Some(HashSet::from([key(5)])), None, false).unwrap();
        assert_eq!(
            list.decide(&store, &[key(6), key(5)]).unwrap(),
            Decision::Allowed(key(5))
        );
        let forced = Allowlist::new(None, None, true).unwrap();
        store::staff_revoke(&store, &key(6), 1).unwrap();
        assert_eq!(
            forced.decide(&store, &[key(6)]).unwrap(),
            Decision::Denied,
            "a revocation holds even in the open-door fallback"
        );
    }

    #[test]
    fn an_empty_but_present_list_is_a_closed_door() {
        let store = store();
        let list = Allowlist::new(Some(HashSet::new()), None, false).unwrap();
        assert_eq!(list.decide(&store, &[key(1)]).unwrap(), Decision::Denied);
    }
}
