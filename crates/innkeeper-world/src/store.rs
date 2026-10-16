//! The persistence boundary: accounts, bankrolls, mailboxes and boards behind one synchronous trait.
//! Design and the conformance suite are described in docs/server/store.md.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use thiserror::Error;

use crate::saves::{hash_hex, SaveMeta, KEPT_VERSIONS};
use crate::{AccountId, EncodedPassword, LoginStatus, Money};

/// A mailbox number, the value the client keeps in `mail.cfg`; numbering starts at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MailboxNumber(pub u32);

/// A letter's identity inside the store, unique across all mailboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LetterId(pub u32);

/// A bulletin board; a board exists from its first post.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BoardId(pub u32);

/// A post's identity inside the store, unique across all boards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PostId(pub u32);

/// What the host remembers about one account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRecord {
    pub password: EncodedPassword,
    pub persona: String,
    pub user_flags: u16,
    pub rating: u16,
    pub status: LoginStatus,
    pub password_expired: bool,
}

impl AccountRecord {
    /// A plain member: INN Stamps only, no rating, password current.
    pub fn new(password: EncodedPassword, persona: impl Into<String>) -> Self {
        AccountRecord {
            password,
            persona: persona.into(),
            user_flags: crate::assumptions::NEW_ACCOUNT_USER_FLAGS_ASSUMED,
            rating: 0,
            status: LoginStatus::default(),
            password_expired: false,
        }
    }
}

/// A letter as the client composed it; the store never looks inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Letter {
    pub id: LetterId,
    pub body: Vec<u8>,
}

/// A board post as the client composed it; the store never looks inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub id: PostId,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("account {0:?} already exists")]
    AccountExists(AccountId),
    #[error("account {0:?} does not exist")]
    UnknownAccount(AccountId),
    #[error("mailbox {0:?} does not exist")]
    UnknownMailbox(MailboxNumber),
    #[error("the storage backend failed: {0}")]
    Backend(Box<dyn Error + Send + Sync>),
}

impl StoreError {
    pub fn backend(source: impl Error + Send + Sync + 'static) -> Self {
        StoreError::Backend(Box::new(source))
    }
}

/// Durable host state. Reads of an unknown key return `None` or an empty list; only a write that
/// needs a missing parent (an account, a mailbox) is an error.
pub trait Store: fmt::Debug + Send + Sync {
    fn account(&self, id: AccountId) -> Result<Option<AccountRecord>, StoreError>;
    fn create_account(&self, id: AccountId, record: AccountRecord) -> Result<(), StoreError>;
    /// The greatest account number stored, if the store holds any.
    fn highest_account_id(&self) -> Result<Option<AccountId>, StoreError>;
    /// Replaces the password and clears the password-expired flag.
    fn update_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), StoreError>;

    /// The account's CasinoLand bankroll, or `None` when nothing has been stored for it yet.
    fn money(&self, id: AccountId) -> Result<Option<Money>, StoreError>;
    /// Keeps the bankroll. Unlike the mail families this needs no account row: the balance is a value
    /// under an account number (`store.md` section 2), so an open book remembers one too.
    fn set_money(&self, id: AccountId, amount: Money) -> Result<(), StoreError>;

    /// The account's mailbox number; the first call assigns one and later calls return it again.
    fn assign_mailbox(&self, id: AccountId) -> Result<MailboxNumber, StoreError>;
    fn append_letter(&self, mailbox: MailboxNumber, body: &[u8]) -> Result<LetterId, StoreError>;
    /// Oldest first; empty for a mailbox that does not exist.
    fn letters(&self, mailbox: MailboxNumber) -> Result<Vec<Letter>, StoreError>;
    /// True when the letter was in that mailbox and is now gone.
    fn delete_letter(&self, mailbox: MailboxNumber, letter: LetterId) -> Result<bool, StoreError>;

    fn post(&self, board: BoardId, body: &[u8]) -> Result<PostId, StoreError>;
    /// Oldest first; empty for a board nobody has posted to.
    fn posts(&self, board: BoardId) -> Result<Vec<Post>, StoreError>;

    /// The current version of each file the account has stored, ordered by path.
    fn saves(&self, id: AccountId) -> Result<Vec<SaveMeta>, StoreError>;
    /// Every version kept of one file, newest first; empty for a file never stored.
    fn save_versions(&self, id: AccountId, path: &str) -> Result<Vec<SaveMeta>, StoreError>;
    /// One version of a file with its content: the current one when `version` is `None`.
    fn save_read(
        &self,
        id: AccountId,
        path: &str,
        version: Option<u64>,
    ) -> Result<Option<(SaveMeta, Vec<u8>)>, StoreError>;
    /// Keeps `data` as the next version of the file and returns its metadata. With `current` it
    /// becomes the version a download returns and the old current stays as history; without, it is
    /// kept as history beside the current one (it still counts against `KEPT_VERSIONS`). A file with
    /// no versions yet always gets a current one. Paths are canonical (`saves::canonical_path`);
    /// like a bankroll, a save needs no account row.
    fn save_write(
        &self,
        id: AccountId,
        path: &str,
        data: &[u8],
        modified: u64,
        current: bool,
    ) -> Result<SaveMeta, StoreError>;

    /// Hides `post` from readers. The row stays until the retention purge.
    fn delete_post(&self, board: BoardId, post: PostId) -> Result<bool, StoreError>;

    /// Marks the account deleted so it cannot log on and its number is not reused.
    /// Owned mail and posts are retired by the operator command (docs/server/privacy.md).
    fn retire_account(&self, id: AccountId) -> Result<(), StoreError>;

    /// Clears a retirement, so the account can log on again.
    fn restore_account(&self, id: AccountId) -> Result<(), StoreError>;

    /// Whether the operator has suspended the account: a Login for it is refused. A store with no
    /// way to suspend never does.
    fn suspended(&self, _id: AccountId) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// A store that forgets everything at exit: the default for tests and a host without a data dir.
#[derive(Debug, Default)]
pub struct MemoryStore {
    tables: Mutex<Tables>,
}

#[derive(Debug, Default)]
struct Tables {
    accounts: HashMap<AccountId, AccountRow>,
    money: HashMap<AccountId, Money>,
    mailbox_of: HashMap<AccountId, MailboxNumber>,
    letters: BTreeMap<MailboxNumber, Vec<HeldLetter>>,
    boards: HashMap<BoardId, Vec<HeldPost>>,
    saves: BTreeMap<(AccountId, String), SavedFile>,
    mailboxes_assigned: u32,
    letters_written: u32,
    posts_written: u32,
}

/// An account row. A retired row stays so its number is never issued again.
#[derive(Clone, Debug)]
struct AccountRow {
    record: AccountRecord,
    deleted_at: Option<u64>,
}

/// A letter kept after the player deletes it, hidden from readers.
#[derive(Clone, Debug)]
struct HeldLetter {
    letter: Letter,
    deleted_at: Option<u64>,
}

/// A board post kept after it is deleted, hidden from readers.
#[derive(Clone, Debug)]
struct HeldPost {
    post: Post,
    deleted_at: Option<u64>,
}

/// The versions kept of one file, oldest first, and the last version number handed out.
#[derive(Debug, Default)]
struct SavedFile {
    last_version: u64,
    versions: Vec<(SaveMeta, Vec<u8>)>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The tables hold no invariant a panicking writer could break halfway, so poison is ignored.
    fn tables(&self) -> MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Store for MemoryStore {
    fn account(&self, id: AccountId) -> Result<Option<AccountRecord>, StoreError> {
        let tables = self.tables();
        let row = tables
            .accounts
            .get(&id)
            .filter(|row| row.deleted_at.is_none());
        Ok(row.map(|row| row.record.clone()))
    }

    fn create_account(&self, id: AccountId, record: AccountRecord) -> Result<(), StoreError> {
        let mut tables = self.tables();
        if tables.accounts.contains_key(&id) {
            return Err(StoreError::AccountExists(id));
        }
        tables.accounts.insert(
            id,
            AccountRow {
                record,
                deleted_at: None,
            },
        );
        Ok(())
    }

    fn highest_account_id(&self) -> Result<Option<AccountId>, StoreError> {
        Ok(self.tables().accounts.keys().copied().max())
    }

    fn update_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), StoreError> {
        let mut tables = self.tables();
        let row = tables
            .accounts
            .get_mut(&id)
            .filter(|row| row.deleted_at.is_none())
            .ok_or(StoreError::UnknownAccount(id))?;
        row.record.password = password;
        row.record.password_expired = false;
        Ok(())
    }

    fn money(&self, id: AccountId) -> Result<Option<Money>, StoreError> {
        Ok(self.tables().money.get(&id).copied())
    }

    fn set_money(&self, id: AccountId, amount: Money) -> Result<(), StoreError> {
        self.tables().money.insert(id, amount);
        Ok(())
    }

    fn assign_mailbox(&self, id: AccountId) -> Result<MailboxNumber, StoreError> {
        let mut tables = self.tables();
        let live = tables
            .accounts
            .get(&id)
            .is_some_and(|row| row.deleted_at.is_none());
        if !live {
            return Err(StoreError::UnknownAccount(id));
        }
        if let Some(&number) = tables.mailbox_of.get(&id) {
            return Ok(number);
        }
        let number = MailboxNumber(successor(tables.mailboxes_assigned)?);
        tables.mailboxes_assigned = number.0;
        tables.mailbox_of.insert(id, number);
        tables.letters.insert(number, Vec::new());
        Ok(number)
    }

    fn append_letter(&self, mailbox: MailboxNumber, body: &[u8]) -> Result<LetterId, StoreError> {
        let mut tables = self.tables();
        let id = LetterId(successor(tables.letters_written)?);
        let letters = tables
            .letters
            .get_mut(&mailbox)
            .ok_or(StoreError::UnknownMailbox(mailbox))?;
        letters.push(HeldLetter {
            letter: Letter {
                id,
                body: body.to_vec(),
            },
            deleted_at: None,
        });
        tables.letters_written = id.0;
        Ok(id)
    }

    fn letters(&self, mailbox: MailboxNumber) -> Result<Vec<Letter>, StoreError> {
        let tables = self.tables();
        let letters = tables.letters.get(&mailbox).map(Vec::as_slice);
        Ok(letters
            .unwrap_or(&[])
            .iter()
            .filter(|held| held.deleted_at.is_none())
            .map(|held| held.letter.clone())
            .collect())
    }

    fn delete_letter(&self, mailbox: MailboxNumber, letter: LetterId) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let Some(letters) = tables.letters.get_mut(&mailbox) else {
            return Ok(false);
        };
        let Some(held) = letters
            .iter_mut()
            .find(|held| held.letter.id == letter && held.deleted_at.is_none())
        else {
            return Ok(false);
        };
        held.deleted_at = Some(unix_now());
        Ok(true)
    }

    fn post(&self, board: BoardId, body: &[u8]) -> Result<PostId, StoreError> {
        let mut tables = self.tables();
        let id = PostId(successor(tables.posts_written)?);
        tables.posts_written = id.0;
        tables.boards.entry(board).or_default().push(HeldPost {
            post: Post {
                id,
                body: body.to_vec(),
            },
            deleted_at: None,
        });
        Ok(id)
    }

    fn posts(&self, board: BoardId) -> Result<Vec<Post>, StoreError> {
        let tables = self.tables();
        let posts = tables.boards.get(&board).map(Vec::as_slice);
        Ok(posts
            .unwrap_or(&[])
            .iter()
            .filter(|held| held.deleted_at.is_none())
            .map(|held| held.post.clone())
            .collect())
    }

    fn delete_post(&self, board: BoardId, post: PostId) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let Some(posts) = tables.boards.get_mut(&board) else {
            return Ok(false);
        };
        let Some(held) = posts
            .iter_mut()
            .find(|held| held.post.id == post && held.deleted_at.is_none())
        else {
            return Ok(false);
        };
        held.deleted_at = Some(unix_now());
        Ok(true)
    }

    fn retire_account(&self, id: AccountId) -> Result<(), StoreError> {
        let mut tables = self.tables();
        let row = tables
            .accounts
            .get_mut(&id)
            .ok_or(StoreError::UnknownAccount(id))?;
        if row.deleted_at.is_none() {
            row.deleted_at = Some(unix_now());
        }
        Ok(())
    }

    fn restore_account(&self, id: AccountId) -> Result<(), StoreError> {
        let mut tables = self.tables();
        let row = tables
            .accounts
            .get_mut(&id)
            .filter(|row| row.deleted_at.is_some())
            .ok_or(StoreError::UnknownAccount(id))?;
        row.deleted_at = None;
        Ok(())
    }

    fn saves(&self, id: AccountId) -> Result<Vec<SaveMeta>, StoreError> {
        let tables = self.tables();
        let files = tables
            .saves
            .range((id, String::new())..)
            .take_while(|((owner, _), _)| *owner == id);
        Ok(files
            .filter_map(|(_, file)| file.versions.iter().find(|(meta, _)| meta.current))
            .map(|(meta, _)| meta.clone())
            .collect())
    }

    fn save_versions(&self, id: AccountId, path: &str) -> Result<Vec<SaveMeta>, StoreError> {
        let tables = self.tables();
        let file = tables.saves.get(&(id, path.to_owned()));
        let versions = file.map(|file| file.versions.iter().rev());
        Ok(versions
            .into_iter()
            .flatten()
            .map(|(meta, _)| meta.clone())
            .collect())
    }

    fn save_read(
        &self,
        id: AccountId,
        path: &str,
        version: Option<u64>,
    ) -> Result<Option<(SaveMeta, Vec<u8>)>, StoreError> {
        let tables = self.tables();
        let file = tables.saves.get(&(id, path.to_owned()));
        let mut versions = file.into_iter().flat_map(|file| file.versions.iter());
        Ok(versions
            .find(|(meta, _)| version.map_or(meta.current, |wanted| meta.version == wanted))
            .cloned())
    }

    fn save_write(
        &self,
        id: AccountId,
        path: &str,
        data: &[u8],
        modified: u64,
        current: bool,
    ) -> Result<SaveMeta, StoreError> {
        let mut tables = self.tables();
        let file = tables.saves.entry((id, path.to_owned())).or_default();
        let current = current || file.versions.is_empty();
        file.last_version += 1;
        if current {
            for (meta, _) in &mut file.versions {
                meta.current = false;
            }
        }
        let meta = SaveMeta {
            path: path.to_owned(),
            version: file.last_version,
            hash: hash_hex(data),
            size: data.len(),
            modified,
            current,
        };
        file.versions.push((meta.clone(), data.to_vec()));
        // The current version stays; the newest others fill the rest of the allowance.
        let mut spare = KEPT_VERSIONS.saturating_sub(1);
        let mut keep = Vec::new();
        for entry in file.versions.drain(..).rev() {
            if entry.0.current {
                keep.push(entry);
            } else if spare > 0 {
                spare -= 1;
                keep.push(entry);
            }
        }
        keep.reverse();
        file.versions = keep;
        Ok(meta)
    }
}

fn unix_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The number after `last`, counting from 1; a used-up 32-bit space is a backend fault.
fn successor(last: u32) -> Result<u32, StoreError> {
    last.checked_add(1)
        .ok_or_else(|| StoreError::Backend("the 32-bit number space is used up".into()))
}

/// The cases every `Store` must pass; the daemon's tests run them against its own implementations.
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // assertions may abort
pub mod conformance {
    use super::*;

    const ALICE: AccountId = AccountId(100_001);
    const BOB: AccountId = AccountId(100_002);
    const MYCHARS: &str = "INN\\YSERBIUS\\MYCHARS.DAT";
    const AUTOMAPS: &str = "INN\\YSERBIUS\\AUTOMAPS.DAT";

    fn record(password: u8) -> AccountRecord {
        AccountRecord {
            user_flags: 0x0102,
            rating: 7,
            status: LoginStatus::PASSWORD_OUT_OF_DATE,
            password_expired: true,
            ..AccountRecord::new(EncodedPassword([password; 10]), "guybrush")
        }
    }

    fn bodies<T>(items: &[T], body: impl Fn(&T) -> &[u8]) -> Vec<Vec<u8>> {
        items.iter().map(|item| body(item).to_vec()).collect()
    }

    /// Runs every case, each on a store from `fresh`.
    pub fn run_all<S: Store>(fresh: impl Fn() -> S) {
        let cases: [fn(&dyn Store); 12] = [
            unknown_account_is_absent,
            created_account_reads_back,
            duplicate_account_is_refused,
            bankroll_reads_absent_then_per_account,
            password_update_clears_expiry,
            mailbox_numbers_are_per_account,
            letters_keep_order_and_isolation,
            letters_are_opaque_bytes,
            boards_keep_order_and_isolation,
            saves_read_absent_then_per_account_and_path,
            saves_keep_the_newest_versions_and_one_current,
            saves_are_opaque_bytes,
        ];
        for case in cases {
            case(&fresh());
        }
    }

    /// Writes through one handle, drops it, and expects a handle from `reopen` to see everything.
    pub fn run_persistence<S: Store>(reopen: impl Fn() -> S) {
        let (mailbox, letter) = {
            let store = reopen();
            store.create_account(ALICE, record(1)).unwrap();
            store
                .update_password(ALICE, EncodedPassword([2; 10]))
                .unwrap();
            let mailbox = store.assign_mailbox(ALICE).unwrap();
            store.set_money(ALICE, Money(247)).unwrap();
            let letter = store.append_letter(mailbox, b"hello").unwrap();
            store.append_letter(mailbox, b"again").unwrap();
            store.delete_letter(mailbox, letter).unwrap();
            store.post(BoardId(3), b"first").unwrap();
            store.save_write(ALICE, MYCHARS, b"one", 10, true).unwrap();
            store.save_write(ALICE, MYCHARS, b"two", 20, true).unwrap();
            store.save_write(ALICE, MYCHARS, b"old", 5, false).unwrap();
            (mailbox, letter)
        };
        let store = reopen();
        let versions = store.save_versions(ALICE, MYCHARS).unwrap();
        let held = |meta: &SaveMeta| (meta.version, meta.current, meta.modified);
        assert_eq!(
            versions.iter().map(held).collect::<Vec<_>>(),
            [(3, false, 5), (2, true, 20), (1, false, 10)]
        );
        let (meta, data) = store.save_read(ALICE, MYCHARS, None).unwrap().unwrap();
        assert_eq!((meta.version, data.as_slice()), (2, &b"two"[..]));
        let next = store
            .save_write(ALICE, MYCHARS, b"three", 30, true)
            .unwrap();
        assert_eq!(next.version, 4, "numbering carries on");
        let kept = store.account(ALICE).unwrap().unwrap();
        assert_eq!(kept.password, EncodedPassword([2; 10]));
        assert!(!kept.password_expired);
        assert_eq!(store.money(ALICE).unwrap(), Some(Money(247)));
        assert_eq!(store.money(BOB).unwrap(), None);
        assert_eq!(store.assign_mailbox(ALICE).unwrap(), mailbox);
        assert_eq!(
            bodies(&store.letters(mailbox).unwrap(), |l| &l.body),
            [b"again".to_vec()]
        );
        assert_eq!(
            bodies(&store.posts(BoardId(3)).unwrap(), |p| &p.body),
            [b"first".to_vec()]
        );
        store.create_account(BOB, record(3)).unwrap();
        assert_ne!(store.assign_mailbox(BOB).unwrap(), mailbox);
        assert!(store.append_letter(mailbox, b"new").unwrap() > letter);
    }

    fn unknown_account_is_absent(store: &dyn Store) {
        assert_eq!(store.account(ALICE).unwrap(), None);
        let missing = |error: StoreError| matches!(error, StoreError::UnknownAccount(ALICE));
        assert!(missing(
            store
                .update_password(ALICE, EncodedPassword([1; 10]))
                .unwrap_err()
        ));
        assert!(missing(store.assign_mailbox(ALICE).unwrap_err()));
    }

    fn created_account_reads_back(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        assert_eq!(store.account(ALICE).unwrap(), Some(record(1)));
        assert_eq!(store.account(BOB).unwrap(), None);
    }

    fn duplicate_account_is_refused(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        let error = store.create_account(ALICE, record(2)).unwrap_err();
        assert!(matches!(error, StoreError::AccountExists(ALICE)));
        assert_eq!(store.account(ALICE).unwrap(), Some(record(1)));
    }

    fn bankroll_reads_absent_then_per_account(store: &dyn Store) {
        // A bankroll needs no account row, so a kind of account the host never created still has one.
        assert_eq!(store.money(ALICE).unwrap(), None);
        store.set_money(ALICE, Money(247)).unwrap();
        assert_eq!(store.money(ALICE).unwrap(), Some(Money(247)));
        assert_eq!(store.money(BOB).unwrap(), None);
        // A repeated save replaces. An amount above the start, and 0, are stored as given.
        store.set_money(ALICE, Money(1_000_000)).unwrap();
        assert_eq!(store.money(ALICE).unwrap(), Some(Money(1_000_000)));
        store.set_money(ALICE, Money(0)).unwrap();
        assert_eq!(store.money(ALICE).unwrap(), Some(Money(0)));
        // Both words, at the top of the 32-bit range.
        store.set_money(BOB, Money(0xFFFF_FFFF)).unwrap();
        assert_eq!(store.money(BOB).unwrap(), Some(Money(0xFFFF_FFFF)));
    }

    fn password_update_clears_expiry(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        store
            .update_password(ALICE, EncodedPassword([9; 10]))
            .unwrap();
        let expected = AccountRecord {
            password: EncodedPassword([9; 10]),
            password_expired: false,
            ..record(1)
        };
        assert_eq!(store.account(ALICE).unwrap(), Some(expected));
    }

    fn mailbox_numbers_are_per_account(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        store.create_account(BOB, record(1)).unwrap();
        let alice = store.assign_mailbox(ALICE).unwrap();
        let bob = store.assign_mailbox(BOB).unwrap();
        assert_ne!(alice, bob);
        assert_eq!(store.assign_mailbox(ALICE).unwrap(), alice);
    }

    fn letters_keep_order_and_isolation(store: &dyn Store) {
        let ghost = MailboxNumber(999);
        assert!(store.letters(ghost).unwrap().is_empty());
        assert!(!store.delete_letter(ghost, LetterId(1)).unwrap());
        assert!(matches!(
            store.append_letter(ghost, b"lost").unwrap_err(),
            StoreError::UnknownMailbox(_)
        ));

        store.create_account(ALICE, record(1)).unwrap();
        store.create_account(BOB, record(1)).unwrap();
        let alice = store.assign_mailbox(ALICE).unwrap();
        let bob = store.assign_mailbox(BOB).unwrap();
        let first = store.append_letter(alice, b"one").unwrap();
        let second = store.append_letter(alice, b"two").unwrap();
        let other = store.append_letter(bob, b"theirs").unwrap();
        assert!(first != second && second != other);
        assert_eq!(
            bodies(&store.letters(alice).unwrap(), |l| &l.body),
            [b"one".to_vec(), b"two".to_vec()]
        );

        assert!(!store.delete_letter(alice, other).unwrap());
        assert!(store.delete_letter(alice, first).unwrap());
        assert!(!store.delete_letter(alice, first).unwrap());
        assert_eq!(store.letters(alice).unwrap().len(), 1);
        assert_eq!(store.letters(bob).unwrap().len(), 1);
    }

    fn letters_are_opaque_bytes(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        let mailbox = store.assign_mailbox(ALICE).unwrap();
        let bodies_in: [&[u8]; 2] = [&[], &[0, 255, 0, 10, 13, 0x80]];
        for body in bodies_in {
            store.append_letter(mailbox, body).unwrap();
        }
        let kept = store.letters(mailbox).unwrap();
        assert_eq!(bodies(&kept, |l| &l.body), bodies_in.map(<[u8]>::to_vec));
    }

    fn boards_keep_order_and_isolation(store: &dyn Store) {
        assert!(store.posts(BoardId(1)).unwrap().is_empty());
        let first = store.post(BoardId(1), b"a|first").unwrap();
        let second = store.post(BoardId(1), &[0, 1, 2]).unwrap();
        let other = store.post(BoardId(2), b"elsewhere").unwrap();
        assert!(first != second && second != other);
        let kept = store.posts(BoardId(1)).unwrap();
        assert_eq!(
            bodies(&kept, |p| &p.body),
            [b"a|first".to_vec(), vec![0, 1, 2]]
        );
        assert_eq!(store.posts(BoardId(2)).unwrap().len(), 1);
    }

    fn saves_read_absent_then_per_account_and_path(store: &dyn Store) {
        assert_eq!(store.saves(ALICE).unwrap(), []);
        assert_eq!(store.save_read(ALICE, MYCHARS, None).unwrap(), None);
        assert_eq!(store.save_versions(ALICE, MYCHARS).unwrap(), []);
        // A save needs no account row, like a bankroll.
        let first = store
            .save_write(ALICE, MYCHARS, b"hello", 1_000, true)
            .unwrap();
        assert_eq!(first.version, 1);
        assert!(first.current);
        assert_eq!((first.size, first.modified), (5, 1_000));
        assert_eq!(first.hash, crate::saves::hash_hex(b"hello"));
        store
            .save_write(ALICE, AUTOMAPS, b"maps", 2_000, true)
            .unwrap();
        store.save_write(BOB, MYCHARS, b"bob", 3_000, true).unwrap();
        let listed = store.saves(ALICE).unwrap();
        let paths: Vec<_> = listed.iter().map(|meta| meta.path.as_str()).collect();
        assert_eq!(paths, [AUTOMAPS, MYCHARS], "ordered by path");
        assert_eq!(store.saves(BOB).unwrap().len(), 1);
        let (_, data) = store.save_read(BOB, MYCHARS, None).unwrap().unwrap();
        assert_eq!(data, b"bob");
        let (meta, data) = store.save_read(ALICE, MYCHARS, Some(1)).unwrap().unwrap();
        assert_eq!((meta, data.as_slice()), (first, &b"hello"[..]));
        assert_eq!(store.save_read(ALICE, MYCHARS, Some(9)).unwrap(), None);
    }

    fn saves_keep_the_newest_versions_and_one_current(store: &dyn Store) {
        for (n, text) in [b"a", b"b", b"c", b"d"].into_iter().enumerate() {
            let version = store
                .save_write(ALICE, MYCHARS, text, 10 + n as u64, true)
                .unwrap();
            assert_eq!(version.version, n as u64 + 1);
        }
        let held = |store: &dyn Store| -> Vec<(u64, bool)> {
            let versions = store.save_versions(ALICE, MYCHARS).unwrap();
            versions
                .iter()
                .map(|meta| (meta.version, meta.current))
                .collect()
        };
        assert_eq!(
            held(store),
            [(4, true), (3, false), (2, false)],
            "three kept"
        );
        assert_eq!(store.save_read(ALICE, MYCHARS, Some(1)).unwrap(), None);
        // A version kept beside the current one does not replace it, and pushes out the oldest.
        let beside = store.save_write(ALICE, MYCHARS, b"x", 1, false).unwrap();
        assert_eq!((beside.version, beside.current), (5, false));
        assert_eq!(held(store), [(5, false), (4, true), (3, false)]);
        let (meta, data) = store.save_read(ALICE, MYCHARS, None).unwrap().unwrap();
        assert_eq!((meta.version, data.as_slice()), (4, &b"d"[..]));
        // A newer current can arrive while history is full: the old current becomes history.
        store.save_write(ALICE, MYCHARS, b"e", 50, true).unwrap();
        assert_eq!(held(store), [(6, true), (5, false), (4, false)]);
        // The first version of a file is current even if asked otherwise.
        let first = store.save_write(ALICE, AUTOMAPS, b"m", 1, false).unwrap();
        assert!(first.current);
        assert_eq!(store.saves(ALICE).unwrap().len(), 2);
    }

    fn saves_are_opaque_bytes(store: &dyn Store) {
        let odd: Vec<u8> = (0..=255).chain([0, 0, 0xFF]).collect();
        store.save_write(ALICE, MYCHARS, &odd, 1, true).unwrap();
        store.save_write(ALICE, AUTOMAPS, b"", 1, true).unwrap();
        let (_, data) = store.save_read(ALICE, MYCHARS, None).unwrap().unwrap();
        assert_eq!(data, odd);
        let (meta, data) = store.save_read(ALICE, AUTOMAPS, None).unwrap().unwrap();
        assert_eq!((data.len(), meta.size), (0, 0));
        let big = vec![0xA5; 300_000];
        store
            .save_write(ALICE, MYCHARS, &big, u64::from(u32::MAX) + 7, true)
            .unwrap();
        let (meta, data) = store.save_read(ALICE, MYCHARS, None).unwrap().unwrap();
        assert_eq!(
            (data == big, meta.modified),
            (true, u64::from(u32::MAX) + 7)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_passes_the_conformance_suite() {
        conformance::run_all(MemoryStore::new);
    }

    #[test]
    fn a_deleted_letter_and_post_leave_the_readers_view_and_the_number_stays_taken() {
        let store = MemoryStore::new();
        let id = AccountId(7);
        store
            .create_account(id, AccountRecord::new(EncodedPassword([1; 10]), "a"))
            .unwrap();
        let mailbox = store.assign_mailbox(id).unwrap();
        let letter = store.append_letter(mailbox, b"hi").unwrap();
        assert!(store.delete_letter(mailbox, letter).unwrap());
        assert!(store.letters(mailbox).unwrap().is_empty());
        assert!(!store.delete_letter(mailbox, letter).unwrap());
        let post = store.post(BoardId(1), b"a|text").unwrap();
        assert!(store.delete_post(BoardId(1), post).unwrap());
        assert!(store.posts(BoardId(1)).unwrap().is_empty());
        store.retire_account(id).unwrap();
        store.retire_account(id).unwrap();
        assert!(store.account(id).unwrap().is_none());
        assert_eq!(store.highest_account_id().unwrap(), Some(id));
        store.restore_account(id).unwrap();
        assert_eq!(store.account(id).unwrap().unwrap().persona, "a");
    }

    #[test]
    fn a_new_record_is_a_plain_member() {
        let record = AccountRecord::new(EncodedPassword([1; 10]), "elaine");
        let stamps = crate::assumptions::NEW_ACCOUNT_USER_FLAGS_ASSUMED;
        assert_eq!((record.user_flags, record.rating), (stamps, 0));
        assert!(!record.password_expired);
    }
}
