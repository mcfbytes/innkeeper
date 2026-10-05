//! The durable `Store`: one SQLite database, schema created on open. See docs/server/store.md.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use innkeeper_world::{
    AccountId, AccountRecord, BoardId, EncodedPassword, Letter, LetterId, LoginStatus,
    MailboxNumber, Post, PostId, Store, StoreError,
};
use rusqlite::types::Type;
use rusqlite::{params, Connection, Row};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS accounts (
        id INTEGER PRIMARY KEY,
        password BLOB NOT NULL,
        persona TEXT NOT NULL,
        user_flags INTEGER NOT NULL,
        rating INTEGER NOT NULL,
        status INTEGER NOT NULL,
        password_expired INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS mailboxes (
        number INTEGER PRIMARY KEY AUTOINCREMENT,
        account INTEGER NOT NULL UNIQUE REFERENCES accounts(id)
    );
    CREATE TABLE IF NOT EXISTS letters (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        mailbox INTEGER NOT NULL REFERENCES mailboxes(number),
        body BLOB NOT NULL
    );
    CREATE INDEX IF NOT EXISTS letters_by_mailbox ON letters(mailbox);
    CREATE TABLE IF NOT EXISTS posts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        board INTEGER NOT NULL,
        body BLOB NOT NULL
    );
    CREATE INDEX IF NOT EXISTS posts_by_board ON posts(board);
";

/// `Store` over a SQLite file or an in-memory database; one connection behind a mutex.
#[derive(Debug)]
pub(crate) struct SqliteStore {
    connection: Mutex<Connection>,
}

impl SqliteStore {
    pub(crate) fn open(path: &Path) -> Result<Self, StoreError> {
        Self::with_connection(Connection::open(path))
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Result<Self, StoreError> {
        Self::with_connection(Connection::open_in_memory())
    }

    fn with_connection(opened: rusqlite::Result<Connection>) -> Result<Self, StoreError> {
        let connection = opened.map_err(backend)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(backend)?;
        connection.execute_batch(SCHEMA).map_err(backend)?;
        Ok(SqliteStore {
            connection: Mutex::new(connection),
        })
    }

    /// Every statement is a single atomic call, so a panic elsewhere leaves nothing half done.
    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::backend(error)
}

fn account_record(row: &Row<'_>) -> rusqlite::Result<AccountRecord> {
    let password: Vec<u8> = row.get("password")?;
    let password = <[u8; 10]>::try_from(password.as_slice()).map_err(|_| {
        let reason = "an encoded password is 10 bytes";
        rusqlite::Error::FromSqlConversionFailure(0, Type::Blob, reason.into())
    })?;
    Ok(AccountRecord {
        password: EncodedPassword(password),
        persona: row.get("persona")?,
        user_flags: row.get("user_flags")?,
        rating: row.get("rating")?,
        status: LoginStatus(row.get("status")?),
        password_expired: row.get("password_expired")?,
    })
}

impl Store for SqliteStore {
    fn account(&self, id: AccountId) -> Result<Option<AccountRecord>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached("SELECT * FROM accounts WHERE id = ?1")
            .map_err(backend)?;
        let mut rows = statement
            .query_map([id.0], account_record)
            .map_err(backend)?;
        rows.next().transpose().map_err(backend)
    }

    fn create_account(&self, id: AccountId, record: AccountRecord) -> Result<(), StoreError> {
        let inserted = self
            .connection()
            .execute(
                "INSERT OR IGNORE INTO accounts VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.0,
                    record.password.0,
                    record.persona,
                    record.user_flags,
                    record.rating,
                    record.status.0,
                    record.password_expired,
                ],
            )
            .map_err(backend)?;
        match inserted {
            0 => Err(StoreError::AccountExists(id)),
            _ => Ok(()),
        }
    }

    fn update_password(&self, id: AccountId, password: EncodedPassword) -> Result<(), StoreError> {
        let updated = self
            .connection()
            .execute(
                "UPDATE accounts SET password = ?2, password_expired = 0 WHERE id = ?1",
                params![id.0, password.0],
            )
            .map_err(backend)?;
        match updated {
            0 => Err(StoreError::UnknownAccount(id)),
            _ => Ok(()),
        }
    }

    fn assign_mailbox(&self, id: AccountId) -> Result<MailboxNumber, StoreError> {
        let connection = self.connection();
        connection
            .execute(
                "INSERT INTO mailboxes (account) SELECT id FROM accounts WHERE id = ?1
                 AND NOT EXISTS (SELECT 1 FROM mailboxes WHERE account = ?1)",
                [id.0],
            )
            .map_err(backend)?;
        let number = connection.query_row(
            "SELECT number FROM mailboxes WHERE account = ?1",
            [id.0],
            |row| row.get(0),
        );
        match number {
            Ok(number) => Ok(MailboxNumber(number)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(StoreError::UnknownAccount(id)),
            Err(error) => Err(backend(error)),
        }
    }

    fn append_letter(&self, mailbox: MailboxNumber, body: &[u8]) -> Result<LetterId, StoreError> {
        let connection = self.connection();
        let inserted = connection
            .execute(
                "INSERT INTO letters (mailbox, body) SELECT number, ?2 FROM mailboxes WHERE number = ?1",
                params![mailbox.0, body],
            )
            .map_err(backend)?;
        if inserted == 0 {
            return Err(StoreError::UnknownMailbox(mailbox));
        }
        last_id(&connection).map(LetterId)
    }

    fn letters(&self, mailbox: MailboxNumber) -> Result<Vec<Letter>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached("SELECT id, body FROM letters WHERE mailbox = ?1 ORDER BY id")
            .map_err(backend)?;
        let rows = statement
            .query_map([mailbox.0], |row| {
                Ok(Letter {
                    id: LetterId(row.get(0)?),
                    body: row.get(1)?,
                })
            })
            .map_err(backend)?;
        rows.collect::<Result<_, _>>().map_err(backend)
    }

    fn delete_letter(&self, mailbox: MailboxNumber, letter: LetterId) -> Result<bool, StoreError> {
        let deleted = self
            .connection()
            .execute(
                "DELETE FROM letters WHERE id = ?1 AND mailbox = ?2",
                [letter.0, mailbox.0],
            )
            .map_err(backend)?;
        Ok(deleted > 0)
    }

    fn post(&self, board: BoardId, body: &[u8]) -> Result<PostId, StoreError> {
        let connection = self.connection();
        connection
            .execute(
                "INSERT INTO posts (board, body) VALUES (?1, ?2)",
                params![board.0, body],
            )
            .map_err(backend)?;
        last_id(&connection).map(PostId)
    }

    fn posts(&self, board: BoardId) -> Result<Vec<Post>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached("SELECT id, body FROM posts WHERE board = ?1 ORDER BY id")
            .map_err(backend)?;
        let rows = statement
            .query_map([board.0], |row| {
                Ok(Post {
                    id: PostId(row.get(0)?),
                    body: row.get(1)?,
                })
            })
            .map_err(backend)?;
        rows.collect::<Result<_, _>>().map_err(backend)
    }
}

/// The row id the last insert on this connection produced, narrowed to the store's 32 bits.
fn last_id(connection: &Connection) -> Result<u32, StoreError> {
    u32::try_from(connection.last_insert_rowid()).map_err(StoreError::backend)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use innkeeper_world::conformance;

    use super::*;

    /// A database file in the temp dir that is deleted, journal included, when dropped.
    struct TempDatabase(PathBuf);

    impl TempDatabase {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            let name = format!("innkeeperd-store-{}-{nanos}.db", std::process::id());
            TempDatabase(std::env::temp_dir().join(name))
        }

        fn open(&self) -> SqliteStore {
            SqliteStore::open(&self.0).unwrap()
        }
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-journal", "-wal", "-shm"] {
                let mut path = self.0.clone().into_os_string();
                path.push(suffix);
                std::fs::remove_file(path).ok();
            }
        }
    }

    #[test]
    fn in_memory_database_passes_the_conformance_suite() {
        conformance::run_all(|| SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn file_database_passes_the_conformance_suite() {
        let files = RefCell::new(Vec::new());
        conformance::run_all(|| {
            let file = TempDatabase::new();
            let store = file.open();
            files.borrow_mut().push(file);
            store
        });
    }

    #[test]
    fn file_database_keeps_its_contents_across_reopen() {
        let file = TempDatabase::new();
        conformance::run_persistence(|| file.open());
    }

    #[test]
    fn opening_twice_keeps_the_existing_schema() {
        let file = TempDatabase::new();
        file.open();
        file.open();
    }
}
