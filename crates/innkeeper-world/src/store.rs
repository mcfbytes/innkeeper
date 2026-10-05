//! The persistence boundary: accounts, mailboxes and boards behind one synchronous trait.
//! Design and the conformance suite are described in docs/server/store.md.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use thiserror::Error;

use crate::{AccountId, EncodedPassword, LoginStatus};

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
    /// A plain member: no flags, no rating, password current.
    pub fn new(password: EncodedPassword, persona: impl Into<String>) -> Self {
        AccountRecord {
            password,
            persona: persona.into(),
            user_flags: 0,
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
    /// Replaces the password and clears the password-expired flag.
    fn update_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), StoreError>;

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
}

/// A store that forgets everything at exit: the default for tests and a host without a data dir.
#[derive(Debug, Default)]
pub struct MemoryStore {
    tables: Mutex<Tables>,
}

#[derive(Debug, Default)]
struct Tables {
    accounts: HashMap<AccountId, AccountRecord>,
    mailbox_of: HashMap<AccountId, MailboxNumber>,
    letters: BTreeMap<MailboxNumber, Vec<Letter>>,
    boards: HashMap<BoardId, Vec<Post>>,
    mailboxes_assigned: u32,
    letters_written: u32,
    posts_written: u32,
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
        Ok(self.tables().accounts.get(&id).cloned())
    }

    fn create_account(&self, id: AccountId, record: AccountRecord) -> Result<(), StoreError> {
        let mut tables = self.tables();
        if tables.accounts.contains_key(&id) {
            return Err(StoreError::AccountExists(id));
        }
        tables.accounts.insert(id, record);
        Ok(())
    }

    fn update_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), StoreError> {
        let mut tables = self.tables();
        let record = tables
            .accounts
            .get_mut(&id)
            .ok_or(StoreError::UnknownAccount(id))?;
        record.password = password;
        record.password_expired = false;
        Ok(())
    }

    fn assign_mailbox(&self, id: AccountId) -> Result<MailboxNumber, StoreError> {
        let mut tables = self.tables();
        if !tables.accounts.contains_key(&id) {
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
        letters.push(Letter {
            id,
            body: body.to_vec(),
        });
        tables.letters_written = id.0;
        Ok(id)
    }

    fn letters(&self, mailbox: MailboxNumber) -> Result<Vec<Letter>, StoreError> {
        Ok(self
            .tables()
            .letters
            .get(&mailbox)
            .cloned()
            .unwrap_or_default())
    }

    fn delete_letter(&self, mailbox: MailboxNumber, letter: LetterId) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let Some(letters) = tables.letters.get_mut(&mailbox) else {
            return Ok(false);
        };
        let before = letters.len();
        letters.retain(|held| held.id != letter);
        Ok(letters.len() != before)
    }

    fn post(&self, board: BoardId, body: &[u8]) -> Result<PostId, StoreError> {
        let mut tables = self.tables();
        let id = PostId(successor(tables.posts_written)?);
        tables.posts_written = id.0;
        tables.boards.entry(board).or_default().push(Post {
            id,
            body: body.to_vec(),
        });
        Ok(id)
    }

    fn posts(&self, board: BoardId) -> Result<Vec<Post>, StoreError> {
        Ok(self
            .tables()
            .boards
            .get(&board)
            .cloned()
            .unwrap_or_default())
    }
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
        let cases: [fn(&dyn Store); 8] = [
            unknown_account_is_absent,
            created_account_reads_back,
            duplicate_account_is_refused,
            password_update_clears_expiry,
            mailbox_numbers_are_per_account,
            letters_keep_order_and_isolation,
            letters_are_opaque_bytes,
            boards_keep_order_and_isolation,
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
            let letter = store.append_letter(mailbox, b"hello").unwrap();
            store.append_letter(mailbox, b"again").unwrap();
            store.delete_letter(mailbox, letter).unwrap();
            store.post(BoardId(3), b"first").unwrap();
            (mailbox, letter)
        };
        let store = reopen();
        let kept = store.account(ALICE).unwrap().unwrap();
        assert_eq!(kept.password, EncodedPassword([2; 10]));
        assert!(!kept.password_expired);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_passes_the_conformance_suite() {
        conformance::run_all(MemoryStore::new);
    }

    #[test]
    fn a_new_record_is_a_plain_member() {
        let record = AccountRecord::new(EncodedPassword([1; 10]), "elaine");
        assert_eq!((record.user_flags, record.rating), (0, 0));
        assert!(!record.password_expired);
    }
}
