//! The persistence boundary: accounts, bankrolls, mailboxes and boards behind one synchronous trait.
//! Design and the conformance suite are described in docs/server/store.md.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use thiserror::Error;

use crate::bbs::{post_kind, PostKind};
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

/// A letter as the client composed it; the store never looks inside the body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Letter {
    pub id: LetterId,
    /// The account whose session wrote the letter, recorded by the host at delivery. The envelope's
    /// own `from` mailbox is the client's word and attributes nothing.
    pub from: AccountId,
    pub body: Vec<u8>,
}

/// A board post as the client composed it; the store never looks inside the body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub id: PostId,
    pub body: Vec<u8>,
    /// The account that wrote it. Attribution is by this alone: the `name|` at the head of a body is
    /// the client's bytes, and names are not unique (docs/server/privacy.md section 1).
    pub account: AccountId,
    /// For a complaint, the account the complainer named, resolved while that player was online;
    /// none when nobody was named or nobody matched.
    pub about: Option<AccountId>,
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

/// Durable host state. Reads of an unknown key return `None` or an empty list. Every row that
/// belongs to an account needs that account to exist and be live when it is written, so a store
/// never holds an orphan (docs/server/store.md section 2): a write without its parent is an error.
/// An account number is never issued twice; a deleted account keeps its row.
pub trait Store: fmt::Debug + Send + Sync {
    /// A live account; none for an unknown, deleted or purged one.
    fn account(&self, id: AccountId) -> Result<Option<AccountRecord>, StoreError>;
    /// `AccountExists` for any number ever created, a deleted or purged one included.
    fn create_account(&self, id: AccountId, record: AccountRecord) -> Result<(), StoreError>;
    /// The greatest account number ever created, deleted and purged rows included, so a number one
    /// past it has never been issued.
    fn highest_account_id(&self) -> Result<Option<AccountId>, StoreError>;
    /// Replaces the password and clears the password-expired flag.
    fn update_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), StoreError>;
    /// Replaces the account's persona, the name shown for it in lists and the phone book.
    /// A name is not unique: two accounts may hold the same one.
    fn set_persona(&self, id: AccountId, persona: &str) -> Result<(), StoreError>;

    /// The account's CasinoLand bankroll, or `None` when nothing has been stored for it yet.
    fn money(&self, id: AccountId) -> Result<Option<Money>, StoreError>;
    /// Keeps the bankroll of a live account.
    fn set_money(&self, id: AccountId, amount: Money) -> Result<(), StoreError>;

    /// The account's mailbox number; the first call assigns one and later calls return it again.
    fn assign_mailbox(&self, id: AccountId) -> Result<MailboxNumber, StoreError>;
    /// Delivers a letter written by `from` (a live account) into a live account's mailbox.
    fn append_letter(
        &self,
        mailbox: MailboxNumber,
        from: AccountId,
        body: &[u8],
    ) -> Result<LetterId, StoreError>;
    /// Oldest first; empty for a mailbox that does not exist.
    fn letters(&self, mailbox: MailboxNumber) -> Result<Vec<Letter>, StoreError>;
    /// True when the letter was in that mailbox and is now gone.
    fn delete_letter(&self, mailbox: MailboxNumber, letter: LetterId) -> Result<bool, StoreError>;

    /// Adds a post written by `author`, a live account. `about` is the account a complaint names
    /// and must exist; any other kind of post names nobody (`bbs::post_kind`).
    fn post(
        &self,
        board: BoardId,
        author: AccountId,
        about: Option<AccountId>,
        body: &[u8],
    ) -> Result<PostId, StoreError>;
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
    /// the account must be live. `modified` is Unix milliseconds.
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

/// The refusal for a post that names an account although it is not a complaint.
pub fn not_a_complaint(board: BoardId) -> StoreError {
    StoreError::Backend(format!("board {} takes no named account", board.0).into())
}

/// A store that forgets everything at exit: the default for tests and a host without a data dir.
/// It keeps the same rules as the SQLite store: no row without its parent, no number reused.
#[derive(Debug, Default)]
pub struct MemoryStore {
    tables: Mutex<Tables>,
}

#[derive(Debug, Default)]
struct Tables {
    accounts: HashMap<AccountId, AccountRow>,
    money: HashMap<AccountId, Money>,
    mailbox_of: HashMap<AccountId, MailboxNumber>,
    owner_of: HashMap<MailboxNumber, AccountId>,
    letters: BTreeMap<MailboxNumber, Vec<HeldLetter>>,
    boards: HashMap<BoardId, Vec<HeldPost>>,
    saves: BTreeMap<(AccountId, String), SavedFile>,
    mailboxes_assigned: u32,
    letters_written: u32,
    posts_written: u32,
}

impl Tables {
    fn live(&self, id: AccountId) -> bool {
        self.accounts
            .get(&id)
            .is_some_and(|row| row.deleted_at.is_none())
    }

    fn require_live(&self, id: AccountId) -> Result<(), StoreError> {
        match self.live(id) {
            true => Ok(()),
            false => Err(StoreError::UnknownAccount(id)),
        }
    }
}

/// An account row. A retired row stays so its number is never issued again.
#[derive(Clone, Debug)]
struct AccountRow {
    record: AccountRecord,
    deleted_at: Option<i64>,
}

/// A letter kept after the player deletes it, hidden from readers.
#[derive(Clone, Debug)]
struct HeldLetter {
    letter: Letter,
    deleted_at: Option<i64>,
}

/// A board post kept after it is deleted, hidden from readers.
#[derive(Clone, Debug)]
struct HeldPost {
    post: Post,
    deleted_at: Option<i64>,
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

    fn set_persona(&self, id: AccountId, persona: &str) -> Result<(), StoreError> {
        let mut tables = self.tables();
        let row = tables
            .accounts
            .get_mut(&id)
            .filter(|row| row.deleted_at.is_none())
            .ok_or(StoreError::UnknownAccount(id))?;
        persona.clone_into(&mut row.record.persona);
        Ok(())
    }

    fn money(&self, id: AccountId) -> Result<Option<Money>, StoreError> {
        Ok(self.tables().money.get(&id).copied())
    }

    fn set_money(&self, id: AccountId, amount: Money) -> Result<(), StoreError> {
        let mut tables = self.tables();
        tables.require_live(id)?;
        tables.money.insert(id, amount);
        Ok(())
    }

    fn assign_mailbox(&self, id: AccountId) -> Result<MailboxNumber, StoreError> {
        let mut tables = self.tables();
        tables.require_live(id)?;
        if let Some(&number) = tables.mailbox_of.get(&id) {
            return Ok(number);
        }
        let number = MailboxNumber(successor(tables.mailboxes_assigned)?);
        tables.mailboxes_assigned = number.0;
        tables.mailbox_of.insert(id, number);
        tables.owner_of.insert(number, id);
        tables.letters.insert(number, Vec::new());
        Ok(number)
    }

    fn append_letter(
        &self,
        mailbox: MailboxNumber,
        from: AccountId,
        body: &[u8],
    ) -> Result<LetterId, StoreError> {
        let mut tables = self.tables();
        let owner = tables.owner_of.get(&mailbox).copied();
        if !owner.is_some_and(|owner| tables.live(owner)) {
            return Err(StoreError::UnknownMailbox(mailbox));
        }
        tables.require_live(from)?;
        let id = LetterId(successor(tables.letters_written)?);
        let letters = tables
            .letters
            .get_mut(&mailbox)
            .ok_or(StoreError::UnknownMailbox(mailbox))?;
        letters.push(HeldLetter {
            letter: Letter {
                id,
                from,
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
        held.deleted_at = Some(unix_millis());
        Ok(true)
    }

    fn post(
        &self,
        board: BoardId,
        author: AccountId,
        about: Option<AccountId>,
        body: &[u8],
    ) -> Result<PostId, StoreError> {
        let mut tables = self.tables();
        tables.require_live(author)?;
        if let Some(about) = about {
            if post_kind(board) != PostKind::Complaint {
                return Err(not_a_complaint(board));
            }
            if !tables.accounts.contains_key(&about) {
                return Err(StoreError::UnknownAccount(about));
            }
        }
        let id = PostId(successor(tables.posts_written)?);
        tables.posts_written = id.0;
        tables.boards.entry(board).or_default().push(HeldPost {
            post: Post {
                id,
                body: body.to_vec(),
                account: author,
                about,
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
        held.deleted_at = Some(unix_millis());
        Ok(true)
    }

    fn retire_account(&self, id: AccountId) -> Result<(), StoreError> {
        let mut tables = self.tables();
        let row = tables
            .accounts
            .get_mut(&id)
            .ok_or(StoreError::UnknownAccount(id))?;
        if row.deleted_at.is_none() {
            row.deleted_at = Some(unix_millis());
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
        tables.require_live(id)?;
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

/// Unix milliseconds, the one time unit every store column uses.
fn unix_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
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
    const CARA: AccountId = AccountId(100_003);
    const MYCHARS: &str = "INN\\YSERBIUS\\MYCHARS.DAT";
    const AUTOMAPS: &str = "INN\\YSERBIUS\\AUTOMAPS.DAT";
    /// A Yserbius tavern board and the hub's complaint board (land 0, location 0x63, section 3).
    const TAVERN: BoardId = BoardId(1 << 24 | 0x59 << 8 | 1);
    const COMPLAINTS: BoardId = BoardId(0x63 << 8 | 3);

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

    /// Alice and Bob, both live, both called "guybrush".
    fn two_accounts(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        store.create_account(BOB, record(2)).unwrap();
    }

    /// Runs every case, each on a store from `fresh`.
    pub fn run_all<S: Store>(fresh: impl Fn() -> S) {
        let cases: [fn(&dyn Store); 20] = [
            unknown_account_is_absent,
            persona_updates_replace_the_name_and_need_an_account,
            created_account_reads_back,
            duplicate_account_is_refused,
            bankroll_reads_absent_then_per_account,
            a_bankroll_and_a_save_need_a_live_account,
            password_update_clears_expiry,
            mailbox_numbers_are_per_account,
            letters_keep_order_and_isolation,
            letters_need_a_live_sender_and_a_live_box,
            letters_are_opaque_bytes,
            boards_keep_order_and_isolation,
            a_post_records_its_author_and_needs_a_live_one,
            only_a_complaint_names_an_account_and_it_must_exist,
            same_named_accounts_never_share_posts_or_letters,
            account_numbers_are_never_reused,
            a_deleted_account_cannot_be_written_to,
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
            let letter = store.append_letter(mailbox, ALICE, b"hello").unwrap();
            store.append_letter(mailbox, ALICE, b"again").unwrap();
            store.delete_letter(mailbox, letter).unwrap();
            store.post(BoardId(3), ALICE, None, b"first").unwrap();
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
        let letters = store.letters(mailbox).unwrap();
        assert_eq!(bodies(&letters, |l| &l.body), [b"again".to_vec()]);
        assert_eq!(letters[0].from, ALICE);
        let posts = store.posts(BoardId(3)).unwrap();
        assert_eq!(bodies(&posts, |p| &p.body), [b"first".to_vec()]);
        assert_eq!(posts[0].account, ALICE);
        store.create_account(BOB, record(3)).unwrap();
        assert_ne!(store.assign_mailbox(BOB).unwrap(), mailbox);
        assert!(store.append_letter(mailbox, BOB, b"new").unwrap() > letter);
    }

    fn persona_updates_replace_the_name_and_need_an_account(store: &dyn Store) {
        let unknown = store.set_persona(ALICE, "Zelda").unwrap_err();
        assert!(matches!(unknown, StoreError::UnknownAccount(ALICE)));
        two_accounts(store);
        store.set_persona(ALICE, "Zelda").unwrap();
        assert_eq!(store.account(ALICE).unwrap().unwrap().persona, "Zelda");
        store.set_persona(ALICE, "Link").unwrap();
        store.set_persona(BOB, "Link").unwrap();
        assert_eq!(store.account(ALICE).unwrap().unwrap().persona, "Link");
        assert_eq!(store.account(BOB).unwrap().unwrap().persona, "Link");
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
        assert_eq!(store.highest_account_id().unwrap(), None);
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
        two_accounts(store);
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

    fn a_bankroll_and_a_save_need_a_live_account(store: &dyn Store) {
        let orphan = |error: StoreError| matches!(error, StoreError::UnknownAccount(ALICE));
        assert!(orphan(store.set_money(ALICE, Money(1)).unwrap_err()));
        assert!(orphan(
            store.save_write(ALICE, MYCHARS, b"x", 1, true).unwrap_err()
        ));
        assert_eq!(store.money(ALICE).unwrap(), None);
        assert_eq!(store.saves(ALICE).unwrap(), []);
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
        two_accounts(store);
        let alice = store.assign_mailbox(ALICE).unwrap();
        assert_eq!(alice, MailboxNumber(1), "numbers count from 1");
        for _ in 0..3 {
            assert_eq!(store.assign_mailbox(ALICE).unwrap(), alice);
        }
        // Asking again for a box an account has uses up no number.
        assert_eq!(store.assign_mailbox(BOB).unwrap(), MailboxNumber(2));
    }

    fn letters_keep_order_and_isolation(store: &dyn Store) {
        let ghost = MailboxNumber(999);
        assert!(store.letters(ghost).unwrap().is_empty());
        assert!(!store.delete_letter(ghost, LetterId(1)).unwrap());
        two_accounts(store);
        assert!(matches!(
            store.append_letter(ghost, ALICE, b"lost").unwrap_err(),
            StoreError::UnknownMailbox(_)
        ));
        let alice = store.assign_mailbox(ALICE).unwrap();
        let bob = store.assign_mailbox(BOB).unwrap();
        let first = store.append_letter(alice, BOB, b"one").unwrap();
        let second = store.append_letter(alice, BOB, b"two").unwrap();
        let other = store.append_letter(bob, ALICE, b"theirs").unwrap();
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

    fn letters_need_a_live_sender_and_a_live_box(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        let alice = store.assign_mailbox(ALICE).unwrap();
        // BOB has no row: a letter cannot claim an author the store does not hold.
        assert!(matches!(
            store.append_letter(alice, BOB, b"forged").unwrap_err(),
            StoreError::UnknownAccount(BOB)
        ));
        store.create_account(BOB, record(2)).unwrap();
        store.append_letter(alice, BOB, b"real").unwrap();
        store.retire_account(BOB).unwrap();
        assert!(matches!(
            store.append_letter(alice, BOB, b"after").unwrap_err(),
            StoreError::UnknownAccount(BOB)
        ));
        // A deleted account's box takes no new mail.
        store.restore_account(BOB).unwrap();
        store.retire_account(ALICE).unwrap();
        assert!(matches!(
            store.append_letter(alice, BOB, b"late").unwrap_err(),
            StoreError::UnknownMailbox(_)
        ));
        store.restore_account(ALICE).unwrap();
        let kept = store.letters(alice).unwrap();
        assert_eq!(bodies(&kept, |l| &l.body), [b"real".to_vec()]);
        assert_eq!(kept[0].from, BOB);
    }

    fn letters_are_opaque_bytes(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        let mailbox = store.assign_mailbox(ALICE).unwrap();
        let bodies_in: [&[u8]; 2] = [&[], &[0, 255, 0, 10, 13, 0x80]];
        for body in bodies_in {
            store.append_letter(mailbox, ALICE, body).unwrap();
        }
        let kept = store.letters(mailbox).unwrap();
        assert_eq!(bodies(&kept, |l| &l.body), bodies_in.map(<[u8]>::to_vec));
    }

    fn boards_keep_order_and_isolation(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        assert!(store.posts(BoardId(1)).unwrap().is_empty());
        let first = store.post(BoardId(1), ALICE, None, b"a|first").unwrap();
        let second = store.post(BoardId(1), ALICE, None, &[0, 1, 2]).unwrap();
        let other = store.post(BoardId(2), ALICE, None, b"elsewhere").unwrap();
        assert!(first != second && second != other);
        let kept = store.posts(BoardId(1)).unwrap();
        assert_eq!(
            bodies(&kept, |p| &p.body),
            [b"a|first".to_vec(), vec![0, 1, 2]]
        );
        assert_eq!(store.posts(BoardId(2)).unwrap().len(), 1);
    }

    fn a_post_records_its_author_and_needs_a_live_one(store: &dyn Store) {
        assert!(matches!(
            store.post(TAVERN, ALICE, None, b"x|y").unwrap_err(),
            StoreError::UnknownAccount(ALICE)
        ));
        store.create_account(ALICE, record(1)).unwrap();
        let post = store.post(TAVERN, ALICE, None, b"x|y").unwrap();
        let kept = store.posts(TAVERN).unwrap();
        assert_eq!(
            (kept[0].id, kept[0].account, kept[0].about),
            (post, ALICE, None)
        );
        store.retire_account(ALICE).unwrap();
        assert!(store.post(TAVERN, ALICE, None, b"x|z").is_err());
    }

    fn only_a_complaint_names_an_account_and_it_must_exist(store: &dyn Store) {
        two_accounts(store);
        assert!(matches!(
            store
                .post(TAVERN, ALICE, Some(BOB), b"guybrush|hi")
                .unwrap_err(),
            StoreError::Backend(_)
        ));
        assert!(matches!(
            store
                .post(COMPLAINTS, ALICE, Some(CARA), b"x|Complaint: y")
                .unwrap_err(),
            StoreError::UnknownAccount(CARA)
        ));
        store
            .post(
                COMPLAINTS,
                ALICE,
                Some(BOB),
                b"guybrush|Complaint: about: guybrush",
            )
            .unwrap();
        store
            .post(COMPLAINTS, ALICE, None, b"guybrush|Complaint: nobody")
            .unwrap();
        let kept = store.posts(COMPLAINTS).unwrap();
        let named: Vec<_> = kept.iter().map(|post| post.about).collect();
        assert_eq!(named, [Some(BOB), None]);
        // A complaint about a deleted account is still kept: the number names it.
        store.retire_account(BOB).unwrap();
        store
            .post(COMPLAINTS, ALICE, Some(BOB), b"guybrush|Complaint: again")
            .unwrap();
    }

    fn same_named_accounts_never_share_posts_or_letters(store: &dyn Store) {
        two_accounts(store);
        assert_eq!(
            store.account(ALICE).unwrap().unwrap().persona,
            store.account(BOB).unwrap().unwrap().persona
        );
        store.post(TAVERN, ALICE, None, b"guybrush|mine").unwrap();
        store.post(TAVERN, BOB, None, b"guybrush|mine").unwrap();
        let owners: Vec<_> = store
            .posts(TAVERN)
            .unwrap()
            .iter()
            .map(|post| post.account)
            .collect();
        assert_eq!(owners, [ALICE, BOB]);
        let bob = store.assign_mailbox(BOB).unwrap();
        store.append_letter(bob, ALICE, b"same name").unwrap();
        assert_eq!(store.letters(bob).unwrap()[0].from, ALICE);
    }

    fn account_numbers_are_never_reused(store: &dyn Store) {
        two_accounts(store);
        store.retire_account(BOB).unwrap();
        assert_eq!(store.account(BOB).unwrap(), None);
        assert!(matches!(
            store.create_account(BOB, record(3)).unwrap_err(),
            StoreError::AccountExists(BOB)
        ));
        assert_eq!(store.highest_account_id().unwrap(), Some(BOB));
    }

    fn a_deleted_account_cannot_be_written_to(store: &dyn Store) {
        store.create_account(ALICE, record(1)).unwrap();
        store.retire_account(ALICE).unwrap();
        let gone = |error: StoreError| matches!(error, StoreError::UnknownAccount(ALICE));
        assert!(gone(store.set_money(ALICE, Money(1)).unwrap_err()));
        assert!(gone(store.assign_mailbox(ALICE).unwrap_err()));
        assert!(gone(store.set_persona(ALICE, "x").unwrap_err()));
        assert!(gone(
            store.save_write(ALICE, MYCHARS, b"x", 1, true).unwrap_err()
        ));
        store.restore_account(ALICE).unwrap();
        store.set_money(ALICE, Money(1)).unwrap();
    }

    fn saves_read_absent_then_per_account_and_path(store: &dyn Store) {
        two_accounts(store);
        assert_eq!(store.saves(ALICE).unwrap(), []);
        assert_eq!(store.save_read(ALICE, MYCHARS, None).unwrap(), None);
        assert_eq!(store.save_versions(ALICE, MYCHARS).unwrap(), []);
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
        store.create_account(ALICE, record(1)).unwrap();
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
        store.create_account(ALICE, record(1)).unwrap();
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
        let letter = store.append_letter(mailbox, id, b"hi").unwrap();
        assert!(store.delete_letter(mailbox, letter).unwrap());
        assert!(store.letters(mailbox).unwrap().is_empty());
        assert!(!store.delete_letter(mailbox, letter).unwrap());
        let post = store.post(BoardId(1), id, None, b"a|text").unwrap();
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
