//! The durable `Store`: one SQLite database with a baseline schema and an ordered list of
//! migrations after it. The tables, their keys and the lifecycle are in docs/server/store.md.

use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use innkeeper_world::saves::{hash_hex, SaveMeta, KEPT_VERSIONS};
use innkeeper_world::{
    not_a_complaint, post_kind, AccountId, AccountRecord, BoardId, EncodedPassword, Letter,
    LetterId, LoginStatus, MailboxNumber, Money, Post, PostId, PostKind, Store, StoreError,
};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::dates;

/// The schema every new database starts at. Earlier numbers (1 to 4) were a chain of in-place
/// `ALTER TABLE` upgrades of test data; a database at one of them is refused with the remedy
/// (`db reset`), not converted.
pub(crate) const BASELINE_VERSION: i32 = 5;

/// Changes after the baseline, in order: entry `n` takes a database from `BASELINE_VERSION + n` to
/// one more. Each runs in the transaction that also writes the new `user_version`. Empty so far.
const MIGRATIONS: &[&str] = &[];

/// `PRAGMA auto_vacuum` value 2: free pages wait for `incremental_vacuum`.
const AUTO_VACUUM_INCREMENTAL: i64 = 2;

/// The version this build writes and reads.
pub(crate) const SCHEMA_VERSION: i32 = BASELINE_VERSION + MIGRATIONS.len() as i32;

/// The baseline. Every time is INTEGER Unix milliseconds UTC in a column named `*_at`. Every
/// column that names another row is a foreign key with an index. The account row is never deleted:
/// the purge turns it into a tombstone (`purged_at` set, personal columns NULL) that keeps its number
/// and stays the parent of what outlives the account (sealed records, complaints, the portal's log).
/// ON DELETE choices and the lifecycle: docs/server/store.md section 3.
const BASELINE: &str = "
    CREATE TABLE accounts (
        id INTEGER PRIMARY KEY CHECK (id BETWEEN 0 AND 4294967295),
        password BLOB CHECK (length(password) = 10),
        persona TEXT,
        user_flags INTEGER CHECK (user_flags BETWEEN 0 AND 65535),
        rating INTEGER CHECK (rating BETWEEN 0 AND 65535),
        status INTEGER CHECK (status BETWEEN 0 AND 255),
        password_expired INTEGER CHECK (password_expired IN (0, 1)),
        created_at INTEGER NOT NULL,
        deleted_at INTEGER,
        suspended_at INTEGER,
        purged_at INTEGER,
        CHECK (purged_at IS NULL OR deleted_at IS NOT NULL),
        CHECK (
            (purged_at IS NULL AND password IS NOT NULL AND persona IS NOT NULL
                AND user_flags IS NOT NULL AND rating IS NOT NULL AND status IS NOT NULL
                AND password_expired IS NOT NULL)
            OR (purged_at IS NOT NULL AND password IS NULL AND persona IS NULL
                AND user_flags IS NULL AND rating IS NULL AND status IS NULL
                AND password_expired IS NULL AND suspended_at IS NULL)
        )
    ) STRICT;
    CREATE TRIGGER accounts_are_never_deleted BEFORE DELETE ON accounts
    BEGIN
        SELECT RAISE(ABORT, 'an account row is never deleted; the purge leaves a tombstone');
    END;
    CREATE TRIGGER account_numbers_never_change BEFORE UPDATE OF id ON accounts
    BEGIN
        SELECT RAISE(ABORT, 'an account number never changes');
    END;
    CREATE TRIGGER a_tombstone_stays_a_tombstone BEFORE UPDATE ON accounts
    WHEN OLD.purged_at IS NOT NULL
    BEGIN
        SELECT RAISE(ABORT, 'a purged account is a tombstone and is never written again');
    END;

    CREATE TABLE mailboxes (
        number INTEGER PRIMARY KEY AUTOINCREMENT CHECK (number BETWEEN 1 AND 4294901759),
        account INTEGER NOT NULL UNIQUE REFERENCES accounts(id) ON DELETE CASCADE,
        created_at INTEGER NOT NULL,
        deleted_at INTEGER
    ) STRICT;

    CREATE TABLE letters (
        id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id BETWEEN 1 AND 4294967295),
        to_mailbox INTEGER NOT NULL REFERENCES mailboxes(number) ON DELETE CASCADE,
        from_account INTEGER NOT NULL REFERENCES accounts(id),
        body BLOB NOT NULL,
        created_at INTEGER NOT NULL,
        deleted_at INTEGER
    ) STRICT;
    CREATE INDEX letters_by_mailbox ON letters(to_mailbox);
    CREATE INDEX letters_by_sender ON letters(from_account);

    CREATE TABLE posts (
        id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id BETWEEN 1 AND 4294967295),
        board INTEGER NOT NULL CHECK (board BETWEEN 0 AND 4294967295),
        kind TEXT NOT NULL GENERATED ALWAYS AS (
            CASE
                WHEN board = 25346 THEN 'member_services'
                WHEN ((board >> 8) & 65535) = 99 THEN 'complaint'
                WHEN board IN (67, 84) THEN 'topic'
                ELSE 'post'
            END
        ) VIRTUAL,
        account INTEGER NOT NULL REFERENCES accounts(id),
        about_account INTEGER REFERENCES accounts(id),
        body BLOB NOT NULL,
        created_at INTEGER NOT NULL,
        deleted_at INTEGER,
        handled_at INTEGER,
        handled_by BLOB REFERENCES operators(identity_key),
        CHECK (about_account IS NULL OR kind = 'complaint'),
        CHECK ((handled_at IS NULL) = (handled_by IS NULL)),
        CHECK (handled_at IS NULL OR kind IN ('complaint', 'member_services'))
    ) STRICT;
    CREATE INDEX posts_by_board ON posts(board);
    CREATE INDEX posts_by_account ON posts(account);
    CREATE INDEX posts_by_about ON posts(about_account);
    CREATE INDEX posts_by_handler ON posts(handled_by);
    CREATE INDEX posts_by_kind ON posts(kind, created_at);

    CREATE TABLE money (
        account INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        amount INTEGER NOT NULL CHECK (amount BETWEEN 0 AND 4294967295),
        deleted_at INTEGER
    ) STRICT;

    CREATE TABLE saves (
        account INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        path TEXT NOT NULL,
        version INTEGER NOT NULL CHECK (version >= 1),
        hash TEXT NOT NULL CHECK (length(hash) = 64),
        data BLOB NOT NULL,
        modified_at INTEGER NOT NULL CHECK (modified_at >= 0),
        current INTEGER NOT NULL CHECK (current IN (0, 1)),
        deleted_at INTEGER,
        PRIMARY KEY (account, path, version)
    ) STRICT;
    CREATE UNIQUE INDEX saves_one_current ON saves(account, path) WHERE current = 1;

    CREATE TABLE holds (
        account INTEGER PRIMARY KEY REFERENCES accounts(id),
        placed_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE identities (
        identity_key BLOB PRIMARY KEY CHECK (length(identity_key) = 32),
        key_version TEXT NOT NULL,
        subject_sealed BLOB,
        provider TEXT NOT NULL,
        adult_attested_at INTEGER,
        attested_age INTEGER CHECK (attested_age >= 0),
        terms_version INTEGER CHECK (terms_version >= 0),
        created_at INTEGER NOT NULL,
        deleted_at INTEGER,
        CHECK ((adult_attested_at IS NULL) = (attested_age IS NULL)),
        CHECK ((attested_age IS NULL) = (terms_version IS NULL))
    ) STRICT;

    CREATE TABLE identity_accounts (
        account INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        identity_key BLOB NOT NULL
            REFERENCES identities(identity_key) ON UPDATE CASCADE ON DELETE CASCADE,
        created_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX identity_accounts_by_identity ON identity_accounts(identity_key, created_at);

    CREATE TABLE sessions (
        token_hash BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
        identity_key BLOB NOT NULL
            REFERENCES identities(identity_key) ON UPDATE CASCADE ON DELETE CASCADE,
        account INTEGER REFERENCES accounts(id),
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX sessions_by_identity ON sessions(identity_key);
    CREATE INDEX sessions_by_account ON sessions(account);

    CREATE TABLE audit_sessions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        started_at INTEGER NOT NULL,
        ended_at INTEGER,
        account INTEGER REFERENCES accounts(id),
        identity_key BLOB
            REFERENCES identities(identity_key) ON UPDATE CASCADE ON DELETE SET NULL,
        sealed BLOB NOT NULL
    ) STRICT;
    CREATE INDEX audit_by_account ON audit_sessions(account, started_at);
    CREATE INDEX audit_by_identity ON audit_sessions(identity_key);
    CREATE INDEX audit_by_start ON audit_sessions(started_at);

    CREATE TABLE operators (
        identity_key BLOB PRIMARY KEY CHECK (length(identity_key) = 32),
        first_seen_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE staff (
        identity_key BLOB PRIMARY KEY REFERENCES operators(identity_key),
        note TEXT,
        added_at INTEGER NOT NULL,
        revoked_at INTEGER
    ) STRICT;

    CREATE TABLE admin_sessions (
        token_hash BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
        identity_key BLOB NOT NULL REFERENCES operators(identity_key) ON DELETE CASCADE,
        created_at INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX admin_sessions_by_operator ON admin_sessions(identity_key);

    CREATE TABLE admin_audit (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        recorded_at INTEGER NOT NULL,
        identity_key BLOB NOT NULL REFERENCES operators(identity_key),
        action TEXT NOT NULL,
        target TEXT NOT NULL,
        reason TEXT,
        account INTEGER REFERENCES accounts(id)
    ) STRICT;
    CREATE INDEX admin_audit_by_time ON admin_audit(recorded_at);
    CREATE INDEX admin_audit_by_operator ON admin_audit(identity_key);
    CREATE INDEX admin_audit_by_account ON admin_audit(account);
";

/// Why a database file cannot be used as it is.
#[derive(Debug)]
pub(crate) enum SchemaError {
    /// Tables from before the baseline, or with no version at all.
    Older { found: i32 },
    /// Written by a newer build.
    Newer { found: i32 },
    /// `PRAGMA foreign_keys` did not stick (an SQLite built without them).
    NoForeignKeys,
    /// `PRAGMA secure_delete` did not stick.
    NoSecureDelete,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Older { found } => write!(
                f,
                "the database has schema version {found}, older than this build's baseline \
                 {BASELINE_VERSION}, and is not converted. Its data is test data: run \
                 `innkeeperd db reset --data-dir DIR`, which keeps the old file as \
                 innkeeper-<timestamp>.db.old and creates a fresh database"
            ),
            SchemaError::Newer { found } => write!(
                f,
                "the database has schema version {found}, newer than this build's \
                 {SCHEMA_VERSION}; run the build that wrote it"
            ),
            SchemaError::NoForeignKeys => {
                write!(f, "this SQLite does not enforce foreign keys")
            }
            SchemaError::NoSecureDelete => {
                write!(f, "this SQLite does not overwrite deleted content")
            }
        }
    }
}

impl std::error::Error for SchemaError {}

/// `Store` over a SQLite file or an in-memory database; one connection behind a mutex.
#[derive(Debug)]
pub(crate) struct SqliteStore {
    connection: Mutex<Connection>,
}

impl SqliteStore {
    /// Opens the file, creating the baseline in an empty one and running any later migrations.
    /// A file from before the baseline is refused with the remedy (`SchemaError::Older`).
    pub(crate) fn open(path: &Path) -> Result<Self, StoreError> {
        Self::with_connection(Connection::open(path))
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Result<Self, StoreError> {
        Self::with_connection(Connection::open_in_memory())
    }

    fn with_connection(opened: rusqlite::Result<Connection>) -> Result<Self, StoreError> {
        let connection = opened.map_err(backend)?;
        enforce_foreign_keys(&connection)?;
        overwrite_deleted_content(&connection)?;
        prepare_schema(&connection)?;
        vacuum_incrementally(&connection)?;
        Ok(SqliteStore {
            connection: Mutex::new(connection),
        })
    }

    /// Every statement is a single atomic call, so a panic elsewhere leaves nothing half done.
    pub(crate) fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `PRAGMA integrity_check`, `PRAGMA foreign_key_check` and the schema version: what
    /// `innkeeperd db check` prints.
    pub(crate) fn check(&self) -> Result<CheckReport, StoreError> {
        let conn = self.connection();
        let version = user_version(&conn)?;
        let mut integrity = Vec::new();
        {
            let mut statement = conn.prepare("PRAGMA integrity_check").map_err(backend)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(backend)?;
            for row in rows {
                let line = row.map_err(backend)?;
                if line != "ok" {
                    integrity.push(line);
                }
            }
        }
        let foreign_keys = foreign_key_violations(&conn)?;
        let foreign_keys_on: bool = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .map_err(backend)?;
        Ok(CheckReport {
            version,
            integrity,
            foreign_keys,
            foreign_keys_on,
        })
    }
}

/// What `db check` found. Clean when both lists are empty and the version is this build's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CheckReport {
    pub(crate) version: i32,
    pub(crate) integrity: Vec<String>,
    /// `table rowid -> parent` for each row whose parent is missing.
    pub(crate) foreign_keys: Vec<String>,
    pub(crate) foreign_keys_on: bool,
}

impl CheckReport {
    pub(crate) fn clean(&self) -> bool {
        self.integrity.is_empty()
            && self.foreign_keys.is_empty()
            && self.foreign_keys_on
            && self.version == SCHEMA_VERSION
    }
}

/// Each row `PRAGMA foreign_key_check` reports, as `table rowid N -> parent`.
pub(crate) fn foreign_key_violations(conn: &Connection) -> Result<Vec<String>, StoreError> {
    let mut statement = conn.prepare("PRAGMA foreign_key_check").map_err(backend)?;
    let rows = statement
        .query_map([], |row| {
            let table: String = row.get(0)?;
            let rowid: Option<i64> = row.get(1)?;
            let parent: String = row.get(2)?;
            Ok(format!(
                "{table} rowid {} -> {parent}",
                rowid.map_or_else(|| "?".to_owned(), |id| id.to_string())
            ))
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

/// Foreign keys are off by default in SQLite and per connection, so every connection turns them on
/// and reads the setting back.
fn enforce_foreign_keys(connection: &Connection) -> Result<(), StoreError> {
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(backend)?;
    let on: bool = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(backend)?;
    match on {
        true => Ok(()),
        false => Err(StoreError::backend(SchemaError::NoForeignKeys)),
    }
}

/// A deleted row's bytes are zeroed in the file at once, not left in a free page for the file and
/// its backups to carry (docs/server/privacy.md section 6). Per connection, so set on each one.
fn overwrite_deleted_content(connection: &Connection) -> Result<(), StoreError> {
    connection
        .pragma_update(None, "secure_delete", true)
        .map_err(backend)?;
    let on: i64 = connection
        .query_row("PRAGMA secure_delete", [], |row| row.get(0))
        .map_err(backend)?;
    match on {
        1 => Ok(()),
        _ => Err(StoreError::backend(SchemaError::NoSecureDelete)),
    }
}

/// `auto_vacuum = INCREMENTAL`, so a purge can hand its free pages back. An empty file takes it
/// before the baseline is written; an older file is converted once by a full `VACUUM`.
fn vacuum_incrementally(connection: &Connection) -> Result<(), StoreError> {
    let mode: i64 = connection
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .map_err(backend)?;
    if mode == AUTO_VACUUM_INCREMENTAL {
        return Ok(());
    }
    connection
        .pragma_update(None, "auto_vacuum", AUTO_VACUUM_INCREMENTAL)
        .map_err(backend)?;
    connection.execute_batch("VACUUM").map_err(backend)
}

/// Truncates the free pages a purge left off the end of the file. Their content is already zero
/// (`secure_delete`); this only shrinks the file, under the store's lock for a few milliseconds.
pub(crate) fn release_free_pages(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch("PRAGMA incremental_vacuum")
        .map_err(backend)
}

fn user_version(connection: &Connection) -> Result<i32, StoreError> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(backend)
}

/// An empty file gets the baseline; a baseline or later file gets the migrations it lacks; any
/// other file is refused, untouched.
fn prepare_schema(connection: &Connection) -> Result<(), StoreError> {
    let version = user_version(connection)?;
    let tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(backend)?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::backend(SchemaError::Newer { found: version }));
    }
    if version < BASELINE_VERSION {
        if tables > 0 {
            return Err(StoreError::backend(SchemaError::Older { found: version }));
        }
        let create = connection.unchecked_transaction().map_err(backend)?;
        create.execute_batch(BASELINE).map_err(backend)?;
        create
            .pragma_update(None, "user_version", BASELINE_VERSION)
            .map_err(backend)?;
        create.commit().map_err(backend)?;
    }
    migrate(connection)
}

/// Runs each migration after the stored version, each with its version bump in one transaction.
fn migrate(connection: &Connection) -> Result<(), StoreError> {
    loop {
        let version = user_version(connection)?;
        let index = usize::try_from(version - BASELINE_VERSION).unwrap_or(usize::MAX);
        let Some(sql) = MIGRATIONS.get(index) else {
            return Ok(());
        };
        let step = connection.unchecked_transaction().map_err(backend)?;
        step.execute_batch(sql).map_err(backend)?;
        step.pragma_update(None, "user_version", version + 1)
            .map_err(backend)?;
        step.commit().map_err(backend)?;
    }
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::backend(error)
}

/// An account row, including one the player can no longer see.
#[derive(Clone, Debug)]
pub(crate) struct AccountRow {
    /// None for a tombstone: the purge has cleared everything personal.
    pub(crate) record: Option<AccountRecord>,
    pub(crate) created_at: i64,
    pub(crate) deleted_at: Option<i64>,
    pub(crate) purged_at: Option<i64>,
}

fn account_record(row: &Row<'_>) -> rusqlite::Result<Option<AccountRecord>> {
    let Some(password) = row.get::<_, Option<Vec<u8>>>("password")? else {
        return Ok(None);
    };
    let password = <[u8; 10]>::try_from(password.as_slice()).map_err(|_| {
        let reason = "an encoded password is 10 bytes";
        rusqlite::Error::FromSqlConversionFailure(0, Type::Blob, reason.into())
    })?;
    Ok(Some(AccountRecord {
        password: EncodedPassword(password),
        persona: row.get("persona")?,
        user_flags: row.get("user_flags")?,
        rating: row.get("rating")?,
        status: LoginStatus(row.get("status")?),
        password_expired: row.get("password_expired")?,
    }))
}

fn account_row(row: &Row<'_>) -> rusqlite::Result<AccountRow> {
    Ok(AccountRow {
        record: account_record(row)?,
        created_at: row.get("created_at")?,
        deleted_at: row.get("deleted_at")?,
        purged_at: row.get("purged_at")?,
    })
}

/// True when the account exists and is not deleted.
fn live(conn: &Connection, id: AccountId) -> Result<bool, StoreError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
            [id.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(found.is_some())
}

fn require_live(conn: &Connection, id: AccountId) -> Result<(), StoreError> {
    match live(conn, id)? {
        true => Ok(()),
        false => Err(StoreError::UnknownAccount(id)),
    }
}

impl SqliteStore {
    /// The account row even when it has been retired or purged. Export uses this; players use
    /// `account`.
    pub(crate) fn account_any(&self, id: AccountId) -> Result<Option<AccountRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached("SELECT * FROM accounts WHERE id = ?1")
            .map_err(backend)?;
        let mut rows = statement.query_map([id.0], account_row).map_err(backend)?;
        rows.next().transpose().map_err(backend)
    }
}

impl Store for SqliteStore {
    fn account(&self, id: AccountId) -> Result<Option<AccountRecord>, StoreError> {
        Ok(self
            .account_any(id)?
            .filter(|row| row.deleted_at.is_none())
            .and_then(|row| row.record))
    }

    fn create_account(&self, id: AccountId, record: AccountRecord) -> Result<(), StoreError> {
        let inserted = self
            .connection()
            .execute(
                "INSERT INTO accounts
                 (id, password, persona, user_flags, rating, status, password_expired, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT (id) DO NOTHING",
                params![
                    id.0,
                    record.password.0,
                    record.persona,
                    record.user_flags,
                    record.rating,
                    record.status.0,
                    record.password_expired,
                    dates::now_ms(),
                ],
            )
            .map_err(backend)?;
        match inserted {
            0 => Err(StoreError::AccountExists(id)),
            _ => Ok(()),
        }
    }

    fn highest_account_id(&self) -> Result<Option<AccountId>, StoreError> {
        let max: Option<i64> = self
            .connection()
            .query_row("SELECT MAX(id) FROM accounts", [], |row| row.get(0))
            .map_err(backend)?;
        match max {
            None => Ok(None),
            Some(id) => u32::try_from(id)
                .map(|id| Some(AccountId(id)))
                .map_err(StoreError::backend),
        }
    }

    fn update_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), StoreError> {
        let updated = self
            .connection()
            .execute(
                "UPDATE accounts SET password = ?2, password_expired = 0
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![id.0, password.0],
            )
            .map_err(backend)?;
        match updated {
            0 => Err(StoreError::UnknownAccount(id)),
            _ => Ok(()),
        }
    }

    fn set_persona(&self, id: AccountId, persona: &str) -> Result<(), StoreError> {
        let updated = self
            .connection()
            .execute(
                "UPDATE accounts SET persona = ?2 WHERE id = ?1 AND deleted_at IS NULL",
                params![id.0, persona],
            )
            .map_err(backend)?;
        match updated {
            0 => Err(StoreError::UnknownAccount(id)),
            _ => Ok(()),
        }
    }

    fn money(&self, id: AccountId) -> Result<Option<Money>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached("SELECT amount FROM money WHERE account = ?1 AND deleted_at IS NULL")
            .map_err(backend)?;
        let mut rows = statement
            .query_map([id.0], |row| Ok(Money(row.get(0)?)))
            .map_err(backend)?;
        rows.next().transpose().map_err(backend)
    }

    fn set_money(&self, id: AccountId, amount: Money) -> Result<(), StoreError> {
        let written = self
            .connection()
            .execute(
                "INSERT INTO money (account, amount)
                 SELECT id, ?2 FROM accounts WHERE id = ?1 AND deleted_at IS NULL
                 ON CONFLICT (account) DO UPDATE SET amount = excluded.amount, deleted_at = NULL",
                params![id.0, amount.0],
            )
            .map_err(backend)?;
        match written {
            0 => Err(StoreError::UnknownAccount(id)),
            _ => Ok(()),
        }
    }

    fn assign_mailbox(&self, id: AccountId) -> Result<MailboxNumber, StoreError> {
        let connection = self.connection();
        require_live(&connection, id)?;
        connection
            .execute(
                // Not `ON CONFLICT DO NOTHING`: a refused AUTOINCREMENT insert still spends a
                // number, and a player sees the box number.
                "INSERT INTO mailboxes (account, created_at) SELECT ?1, ?2
                 WHERE NOT EXISTS (SELECT 1 FROM mailboxes WHERE account = ?1)",
                params![id.0, dates::now_ms()],
            )
            .map_err(backend)?;
        let number = connection
            .query_row(
                "SELECT number FROM mailboxes WHERE account = ?1",
                [id.0],
                |row| row.get(0),
            )
            .map_err(backend)?;
        Ok(MailboxNumber(number))
    }

    fn append_letter(
        &self,
        mailbox: MailboxNumber,
        from: AccountId,
        body: &[u8],
    ) -> Result<LetterId, StoreError> {
        let connection = self.connection();
        let open: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM mailboxes JOIN accounts ON accounts.id = mailboxes.account
                 WHERE mailboxes.number = ?1 AND accounts.deleted_at IS NULL",
                [mailbox.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        if open.is_none() {
            return Err(StoreError::UnknownMailbox(mailbox));
        }
        require_live(&connection, from)?;
        connection
            .execute(
                "INSERT INTO letters (to_mailbox, from_account, body, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![mailbox.0, from.0, body, dates::now_ms()],
            )
            .map_err(backend)?;
        last_id(&connection).map(LetterId)
    }

    fn letters(&self, mailbox: MailboxNumber) -> Result<Vec<Letter>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached(
                "SELECT id, from_account, body FROM letters
                 WHERE to_mailbox = ?1 AND deleted_at IS NULL ORDER BY id",
            )
            .map_err(backend)?;
        let rows = statement
            .query_map([mailbox.0], |row| {
                Ok(Letter {
                    id: LetterId(row.get(0)?),
                    from: AccountId(row.get(1)?),
                    body: row.get(2)?,
                })
            })
            .map_err(backend)?;
        rows.collect::<Result<_, _>>().map_err(backend)
    }

    fn delete_letter(&self, mailbox: MailboxNumber, letter: LetterId) -> Result<bool, StoreError> {
        let deleted = self
            .connection()
            .execute(
                "UPDATE letters SET deleted_at = ?3
                 WHERE id = ?1 AND to_mailbox = ?2 AND deleted_at IS NULL",
                params![letter.0, mailbox.0, dates::now_ms()],
            )
            .map_err(backend)?;
        Ok(deleted > 0)
    }

    fn post(
        &self,
        board: BoardId,
        author: AccountId,
        about: Option<AccountId>,
        body: &[u8],
    ) -> Result<PostId, StoreError> {
        let connection = self.connection();
        require_live(&connection, author)?;
        if let Some(about) = about {
            if post_kind(board) != PostKind::Complaint {
                return Err(not_a_complaint(board));
            }
            let known: Option<i64> = connection
                .query_row("SELECT 1 FROM accounts WHERE id = ?1", [about.0], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(backend)?;
            if known.is_none() {
                return Err(StoreError::UnknownAccount(about));
            }
        }
        connection
            .execute(
                "INSERT INTO posts (board, account, about_account, body, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    board.0,
                    author.0,
                    about.map(|about| about.0),
                    body,
                    dates::now_ms()
                ],
            )
            .map_err(backend)?;
        last_id(&connection).map(PostId)
    }

    fn posts(&self, board: BoardId) -> Result<Vec<Post>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached(
                "SELECT id, body, account, about_account FROM posts
                 WHERE board = ?1 AND deleted_at IS NULL ORDER BY id",
            )
            .map_err(backend)?;
        let rows = statement
            .query_map([board.0], |row| {
                Ok(Post {
                    id: PostId(row.get(0)?),
                    body: row.get(1)?,
                    account: AccountId(row.get(2)?),
                    about: row.get::<_, Option<u32>>(3)?.map(AccountId),
                })
            })
            .map_err(backend)?;
        rows.collect::<Result<_, _>>().map_err(backend)
    }

    fn delete_post(&self, board: BoardId, post: PostId) -> Result<bool, StoreError> {
        let deleted = self
            .connection()
            .execute(
                "UPDATE posts SET deleted_at = ?3
                 WHERE id = ?1 AND board = ?2 AND deleted_at IS NULL",
                params![post.0, board.0, dates::now_ms()],
            )
            .map_err(backend)?;
        Ok(deleted > 0)
    }

    fn retire_account(&self, id: AccountId) -> Result<(), StoreError> {
        let connection = self.connection();
        let updated = connection
            .execute(
                "UPDATE accounts SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
                params![id.0, dates::now_ms()],
            )
            .map_err(backend)?;
        if updated > 0 {
            return Ok(());
        }
        let exists: bool = connection
            .query_row("SELECT 1 FROM accounts WHERE id = ?1", [id.0], |_| Ok(true))
            .optional()
            .map_err(backend)?
            .unwrap_or(false);
        if exists {
            Ok(())
        } else {
            Err(StoreError::UnknownAccount(id))
        }
    }

    fn suspended(&self, id: AccountId) -> Result<bool, StoreError> {
        let stamp: Option<Option<i64>> = self
            .connection()
            .query_row(
                "SELECT suspended_at FROM accounts WHERE id = ?1",
                [id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        Ok(stamp.flatten().is_some())
    }

    fn restore_account(&self, id: AccountId) -> Result<(), StoreError> {
        let updated = self
            .connection()
            .execute(
                "UPDATE accounts SET deleted_at = NULL
                 WHERE id = ?1 AND deleted_at IS NOT NULL AND purged_at IS NULL",
                [id.0],
            )
            .map_err(backend)?;
        if updated > 0 {
            Ok(())
        } else {
            Err(StoreError::UnknownAccount(id))
        }
    }

    fn saves(&self, id: AccountId) -> Result<Vec<SaveMeta>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached(&format!(
                "SELECT {SAVE_COLUMNS} FROM saves WHERE account = ?1 AND current = 1 \
                 AND deleted_at IS NULL ORDER BY path"
            ))
            .map_err(backend)?;
        let rows = statement.query_map([id.0], save_meta).map_err(backend)?;
        rows.collect::<Result<_, _>>().map_err(backend)
    }

    fn save_versions(&self, id: AccountId, path: &str) -> Result<Vec<SaveMeta>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached(&format!(
                "SELECT {SAVE_COLUMNS} FROM saves WHERE account = ?1 AND path = ?2 \
                 AND deleted_at IS NULL ORDER BY version DESC"
            ))
            .map_err(backend)?;
        let rows = statement
            .query_map(params![id.0, path], save_meta)
            .map_err(backend)?;
        rows.collect::<Result<_, _>>().map_err(backend)
    }

    fn save_read(
        &self,
        id: AccountId,
        path: &str,
        version: Option<u64>,
    ) -> Result<Option<(SaveMeta, Vec<u8>)>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached(&format!(
                "SELECT {SAVE_COLUMNS}, data FROM saves WHERE account = ?1 AND path = ?2 \
                 AND deleted_at IS NULL AND (version = ?3 OR (?3 IS NULL AND current = 1))"
            ))
            .map_err(backend)?;
        let mut rows = statement
            .query_map(params![id.0, path, version.map(signed)], |row| {
                Ok((save_meta(row)?, row.get(6)?))
            })
            .map_err(backend)?;
        rows.next().transpose().map_err(backend)
    }

    fn save_write(
        &self,
        id: AccountId,
        path: &str,
        data: &[u8],
        modified: u64,
        current: bool,
    ) -> Result<SaveMeta, StoreError> {
        let connection = self.connection();
        let write = connection.unchecked_transaction().map_err(backend)?;
        require_live(&write, id)?;
        let (newest, held): (i64, i64) = write
            .query_row(
                "SELECT COALESCE(MAX(version), 0), COUNT(*) FROM saves WHERE account = ?1 AND path = ?2",
                params![id.0, path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(backend)?;
        let current = current || held == 0;
        if current {
            write
                .execute(
                    "UPDATE saves SET current = 0 WHERE account = ?1 AND path = ?2",
                    params![id.0, path],
                )
                .map_err(backend)?;
        }
        let meta = SaveMeta {
            path: path.to_owned(),
            version: unsigned(newest) + 1,
            hash: hash_hex(data),
            size: data.len(),
            modified,
            current,
        };
        write
            .execute(
                "INSERT INTO saves (account, path, version, hash, data, modified_at, current) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.0,
                    path,
                    signed(meta.version),
                    meta.hash,
                    data,
                    signed(modified),
                    current
                ],
            )
            .map_err(backend)?;
        // The current version stays; the newest others fill the rest of the allowance.
        let spare = KEPT_VERSIONS.saturating_sub(1);
        write
            .execute(
                "DELETE FROM saves WHERE account = ?1 AND path = ?2 AND current = 0 AND version NOT IN \
                 (SELECT version FROM saves WHERE account = ?1 AND path = ?2 AND current = 0 \
                  ORDER BY version DESC LIMIT ?3)",
                params![id.0, path, signed(spare as u64)],
            )
            .map_err(backend)?;
        write.commit().map_err(backend)?;
        Ok(meta)
    }
}

/// The columns `save_meta` reads, in its order; `length(data)` keeps the content out of listings.
const SAVE_COLUMNS: &str = "path, version, hash, length(data), modified_at, current";

fn save_meta(row: &Row<'_>) -> rusqlite::Result<SaveMeta> {
    Ok(SaveMeta {
        path: row.get(0)?,
        version: unsigned(row.get(1)?),
        hash: row.get(2)?,
        size: usize::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
        modified: unsigned(row.get(4)?),
        current: row.get(5)?,
    })
}

/// SQLite integers are signed 64-bit; versions and times are far below the sign bit.
fn unsigned(stored: i64) -> u64 {
    u64::try_from(stored).unwrap_or(0)
}

fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The row id the last insert on this connection produced, narrowed to the store's 32 bits.
fn last_id(connection: &Connection) -> Result<u32, StoreError> {
    u32::try_from(connection.last_insert_rowid()).map_err(StoreError::backend)
}

#[cfg(test)]
#[path = "sqlite_store_tests.rs"]
pub(crate) mod tests;
