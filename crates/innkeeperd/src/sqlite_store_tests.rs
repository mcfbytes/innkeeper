use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use innkeeper_world::{conformance, BoardId, LandType};

use super::*;

/// A database file in the temp dir that is deleted, journal included, when dropped.
pub(crate) struct TempDatabase(pub(crate) PathBuf);

impl TempDatabase {
    pub(crate) fn new() -> Self {
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

fn version(store: &SqliteStore) -> i32 {
    user_version(&store.connection()).unwrap()
}

#[test]
fn the_highest_account_id_is_the_largest_row() {
    let store = SqliteStore::open_in_memory().unwrap();
    assert_eq!(store.highest_account_id().unwrap(), None);
    let record = AccountRecord::new(EncodedPassword([1; 10]), "a");
    store
        .create_account(AccountId(100_010), record.clone())
        .unwrap();
    store.create_account(AccountId(100_050), record).unwrap();
    assert_eq!(
        store.highest_account_id().unwrap(),
        Some(AccountId(100_050))
    );
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
fn a_new_file_is_created_at_the_baseline_and_reopens_unchanged() {
    let file = TempDatabase::new();
    assert_eq!(version(&file.open()), SCHEMA_VERSION);
    let again = file.open();
    assert_eq!(version(&again), SCHEMA_VERSION);
    assert!(again.check().unwrap().clean());
}

#[test]
fn foreign_keys_are_on_for_every_connection() {
    let file = TempDatabase::new();
    for _ in 0..2 {
        let store = file.open();
        let on: bool = store
            .connection()
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert!(on, "every connection enforces foreign keys");
        assert!(store.check().unwrap().foreign_keys_on);
    }
    let memory = SqliteStore::open_in_memory().unwrap();
    let on: bool = memory
        .connection()
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert!(on);
}

/// One live account (1), its mailbox (1), an identity (key 0x11) in group 1 and an operator (key
/// 0x22): the parents an orphan insert below lacks one of.
fn parents() -> SqliteStore {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .create_account(
            AccountId(1),
            AccountRecord::new(EncodedPassword([1; 10]), "a"),
        )
        .unwrap();
    store.assign_mailbox(AccountId(1)).unwrap();
    let conn = store.connection();
    conn.execute(
        "INSERT INTO identity_groups (id, created_at) VALUES (1, 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO identities (identity_key, key_version, provider, created_at, identity_group)
         VALUES (?1, 'v', 'p', 1, 1)",
        [vec![0x11u8; 32]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO operators (identity_key, first_seen_at) VALUES (?1, 1)",
        [vec![0x22u8; 32]],
    )
    .unwrap();
    drop(conn);
    store
}

#[test]
fn an_orphan_insert_fails_for_every_foreign_key() {
    let store = parents();
    // Each statement names one missing parent (account, mailbox or group 9, a zero identity or
    // operator key) through exactly one column, so each failure is that key.
    let orphans: &[(&str, &str)] = &[
        ("mailboxes.account", "INSERT INTO mailboxes (account, created_at) VALUES (9, 1)"),
        (
            "letters.to_mailbox",
            "INSERT INTO letters (to_mailbox, from_account, body, created_at) VALUES (9, 1, x'', 1)",
        ),
        (
            "letters.from_account",
            "INSERT INTO letters (to_mailbox, from_account, body, created_at) VALUES (1, 9, x'', 1)",
        ),
        (
            "posts.account",
            "INSERT INTO posts (board, account, body, created_at) VALUES (1, 9, x'', 1)",
        ),
        (
            "posts.about_account",
            "INSERT INTO posts (board, account, about_account, body, created_at)
             VALUES (25347, 1, 9, x'', 1)",
        ),
        (
            "posts.handled_by",
            "INSERT INTO posts (board, account, body, created_at, handled_at, handled_by)
             VALUES (25347, 1, x'', 1, 1, zeroblob(32))",
        ),
        ("money.account", "INSERT INTO money (account, amount) VALUES (9, 1)"),
        (
            "saves.account",
            "INSERT INTO saves (account, path, version, hash, data, modified_at, current)
             VALUES (9, 'P', 1, printf('%064d', 0), x'', 1, 1)",
        ),
        (
            "holds.account",
            "INSERT INTO holds (account, placed_at, expires_at) VALUES (9, 1, 2)",
        ),
        (
            "identities.identity_group",
            "INSERT INTO identities (identity_key, key_version, provider, created_at, identity_group)
             VALUES (zeroblob(32), 'v', 'p', 1, 9)",
        ),
        (
            "group_accounts.account",
            "INSERT INTO group_accounts (account, identity_group, created_at) VALUES (9, 1, 1)",
        ),
        (
            "group_accounts.identity_group",
            "INSERT INTO group_accounts (account, identity_group, created_at) VALUES (1, 9, 1)",
        ),
        (
            "sessions.identity_key",
            "INSERT INTO sessions (token_hash, identity_key, created_at, expires_at)
             VALUES (zeroblob(32), zeroblob(32), 1, 2)",
        ),
        (
            "sessions.account",
            "INSERT INTO sessions (token_hash, identity_key, account, created_at, expires_at)
             VALUES (zeroblob(32),
                     x'1111111111111111111111111111111111111111111111111111111111111111', 9, 1, 2)",
        ),
        (
            "audit_sessions.account",
            "INSERT INTO audit_sessions (started_at, account, sealed) VALUES (1, 9, x'00')",
        ),
        (
            "audit_sessions.identity_key",
            "INSERT INTO audit_sessions (started_at, identity_key, sealed)
             VALUES (1, zeroblob(32), x'00')",
        ),
        (
            "staff.identity_key",
            "INSERT INTO staff (identity_key, added_at) VALUES (zeroblob(32), 1)",
        ),
        (
            "admin_sessions.identity_key",
            "INSERT INTO admin_sessions (token_hash, identity_key, created_at, last_seen_at, expires_at)
             VALUES (zeroblob(32), zeroblob(32), 1, 1, 2)",
        ),
        (
            "admin_audit.identity_key",
            "INSERT INTO admin_audit (recorded_at, identity_key, action, target)
             VALUES (1, zeroblob(32), 'a', 't')",
        ),
        (
            "admin_audit.account",
            "INSERT INTO admin_audit (recorded_at, identity_key, action, target, account)
             VALUES (1, x'2222222222222222222222222222222222222222222222222222222222222222', 'a', 't', 9)",
        ),
    ];
    let conn = store.connection();
    for (key, sql) in orphans {
        let error = conn.execute(sql, []).unwrap_err().to_string();
        assert!(
            error.contains("FOREIGN KEY constraint failed"),
            "{key}: {error}"
        );
    }
    // Every foreign key in the schema is in the list above.
    let mut declared = 0;
    for table in [
        "accounts",
        "mailboxes",
        "letters",
        "posts",
        "money",
        "saves",
        "holds",
        "identity_groups",
        "identities",
        "group_accounts",
        "sessions",
        "audit_sessions",
        "operators",
        "staff",
        "admin_sessions",
        "admin_audit",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_list(?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        declared += count;
    }
    assert_eq!(declared, orphans.len() as i64);
    assert!(foreign_key_violations(&conn).unwrap().is_empty());
}

#[test]
fn every_foreign_key_column_has_an_index() {
    let store = SqliteStore::open_in_memory().unwrap();
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT m.name, f.\"from\" FROM sqlite_schema m, pragma_foreign_key_list(m.name) f
             WHERE m.type = 'table'",
        )
        .unwrap();
    let keys: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (table, column) in keys {
        // The leading column of some index, the primary key's included.
        let indexed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_index_list(?1) l, pragma_index_info(l.name) i
                 WHERE i.seqno = 0 AND i.name = ?2",
                [&table, &column],
                |row| row.get(0),
            )
            .unwrap();
        let rowid_key: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2 AND pk = 1
                 AND type = 'INTEGER'",
                [&table, &column],
                |row| row.get(0),
            )
            .unwrap();
        assert!(indexed + rowid_key > 0, "{table}.{column} has no index");
    }
}

#[test]
fn every_table_is_strict_and_checks_its_ranges() {
    let store = parents();
    let conn = store.connection();
    let loose: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_list WHERE schema = 'main' AND type = 'table'
             AND name NOT LIKE 'sqlite_%' AND strict = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(loose, 0);
    let refused = [
        "INSERT INTO money (account, amount) VALUES (1, -1)",
        "INSERT INTO money (account, amount) VALUES (1, 4294967296)",
        "INSERT INTO money (account, amount) VALUES (1, 'lots')",
        "UPDATE accounts SET user_flags = 65536 WHERE id = 1",
        "UPDATE accounts SET rating = -1 WHERE id = 1",
        "UPDATE accounts SET status = 256 WHERE id = 1",
        "UPDATE accounts SET password_expired = 2 WHERE id = 1",
        "UPDATE accounts SET password = x'00' WHERE id = 1",
        "UPDATE accounts SET purged_at = 1 WHERE id = 1",
        "INSERT INTO saves (account, path, version, hash, data, modified_at, current)
         VALUES (1, 'P', 1, printf('%064d', 0), x'', 1, 2)",
        "INSERT INTO posts (board, account, about_account, body, created_at)
         VALUES (1, 1, 1, x'', 1)",
        "INSERT INTO posts (board, account, body, created_at, handled_at) VALUES (25347, 1, x'', 1, 1)",
        "INSERT INTO identities (identity_key, key_version, provider, created_at, attested_age)
         VALUES (zeroblob(32), 'v', 'p', 1, 18)",
        "DELETE FROM accounts WHERE id = 1",
        "UPDATE accounts SET id = 2 WHERE id = 1",
    ];
    for sql in refused {
        assert!(conn.execute(sql, []).is_err(), "{sql}");
    }
    // The legal extremes the client can send are kept.
    for sql in [
        "INSERT INTO money (account, amount) VALUES (1, 4294967295)",
        "UPDATE accounts SET user_flags = 65535, rating = 65535, status = 255 WHERE id = 1",
    ] {
        conn.execute(sql, []).unwrap();
    }
}

#[test]
fn the_kind_column_agrees_with_the_world_crates_post_kind() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .create_account(
            AccountId(1),
            AccountRecord::new(EncodedPassword([1; 10]), "a"),
        )
        .unwrap();
    let board = |land: u8, location: u16, section: u16| {
        BoardId(u32::from(LandType(land).0) << 24 | u32::from(location) << 8 | u32::from(section))
    };
    let boards = [
        board(0, 0x63, 3),
        board(0, 0x63, 2),
        board(1, 0x63, 2),
        board(1, 0x63, 1),
        board(0, 0, 67),
        board(0, 0, 84),
        board(1, 0, 67),
        board(4, 0x59, 1),
        board(1, 0x5A, 1),
    ];
    for board in boards {
        let post = store.post(board, AccountId(1), None, b"x").unwrap();
        let kind: String = store
            .connection()
            .query_row("SELECT kind FROM posts WHERE id = ?1", [post.0], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(kind, post_kind(board).as_str(), "board {:#x}", board.0);
    }
}

/// The first lines of the version 4 schema, as the chain of `add_column` upgrades left it.
const VERSION_4: &str = "
    CREATE TABLE accounts (id INTEGER PRIMARY KEY, password BLOB NOT NULL, persona TEXT NOT NULL,
        user_flags INTEGER NOT NULL, rating INTEGER NOT NULL, status INTEGER NOT NULL,
        password_expired INTEGER NOT NULL, deleted_at INTEGER, suspended_at INTEGER);
    CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, board INTEGER NOT NULL,
        body BLOB NOT NULL, deleted_at INTEGER, created_at INTEGER, account INTEGER);
    INSERT INTO accounts VALUES (100001, x'09090909090909090909', 'guybrush', 0, 0, 0, 0, NULL, NULL);
    PRAGMA user_version = 4;
";

#[test]
fn an_old_database_is_refused_untouched_with_the_remedy() {
    for (old, found) in [
        (VERSION_4, 4),
        ("CREATE TABLE accounts (id INTEGER PRIMARY KEY);", 0),
    ] {
        let file = TempDatabase::new();
        let conn = Connection::open(&file.0).unwrap();
        conn.execute_batch(old).unwrap();
        drop(conn);
        let before = std::fs::read(&file.0).unwrap();
        let error = SqliteStore::open(&file.0).unwrap_err().to_string();
        assert!(
            error.contains(&format!("schema version {found}")),
            "{error}"
        );
        assert!(
            error.contains("innkeeperd db reset --data-dir DIR"),
            "{error}"
        );
        assert!(error.contains(".db.old"), "{error}");
        assert_eq!(std::fs::read(&file.0).unwrap(), before, "not converted");
    }
}

#[test]
fn version_5_moves_each_identity_into_a_group_of_its_own_with_its_accounts() {
    let file = TempDatabase::new();
    let conn = Connection::open(&file.0).unwrap();
    conn.execute_batch(BASELINE).unwrap();
    conn.pragma_update(None, "user_version", BASELINE_VERSION)
        .unwrap();
    conn.execute_batch(
        "INSERT INTO accounts (id, password, persona, user_flags, rating, status,
                               password_expired, created_at)
         VALUES (100001, zeroblob(10), 'a', 0, 0, 0, 0, 1), (100002, zeroblob(10), 'b', 0, 0, 0, 0, 1);
         INSERT INTO identities (identity_key, key_version, provider, created_at)
         VALUES (x'1111111111111111111111111111111111111111111111111111111111111111', 'v', 'p', 5),
                (x'2222222222222222222222222222222222222222222222222222222222222222', 'v', 'q', 6);
         INSERT INTO identity_accounts (account, identity_key, created_at)
         VALUES (100001, x'1111111111111111111111111111111111111111111111111111111111111111', 7),
                (100002, x'2222222222222222222222222222222222222222222222222222222222222222', 8);",
    )
    .unwrap();
    drop(conn);
    let store = file.open();
    assert_eq!(version(&store), SCHEMA_VERSION);
    let conn = store.connection();
    let owners: Vec<(u32, i64, i64)> = conn
        .prepare(
            "SELECT account, group_accounts.created_at, linked_at FROM group_accounts
             JOIN identities USING (identity_group) ORDER BY account",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(owners, [(100_001, 7, 5), (100_002, 8, 6)]);
    let groups: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT identity_group) FROM identities",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(groups, 2);
    let old: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'identity_accounts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old, 0);
    assert!(foreign_key_violations(&conn).unwrap().is_empty());
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let file = TempDatabase::new();
    let conn = Connection::open(&file.0).unwrap();
    conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(conn);
    let error = SqliteStore::open(&file.0).unwrap_err().to_string();
    assert!(error.contains("newer than this build"), "{error}");
}

#[test]
fn a_purged_account_row_is_a_tombstone_that_keeps_its_number() {
    let store = SqliteStore::open_in_memory().unwrap();
    let id = AccountId(100_001);
    store
        .create_account(id, AccountRecord::new(EncodedPassword([1; 10]), "a"))
        .unwrap();
    store.retire_account(id).unwrap();
    store
        .connection()
        .execute(
            "UPDATE accounts SET password = NULL, persona = NULL, user_flags = NULL, rating = NULL,
             status = NULL, password_expired = NULL, purged_at = 5 WHERE id = ?1",
            [id.0],
        )
        .unwrap();
    let row = store.account_any(id).unwrap().unwrap();
    assert!(row.record.is_none() && row.purged_at == Some(5));
    assert!(store.account(id).unwrap().is_none());
    assert!(store.restore_account(id).is_err(), "a tombstone stays one");
    assert!(matches!(
        store.create_account(id, AccountRecord::new(EncodedPassword([2; 10]), "b")),
        Err(StoreError::AccountExists(_))
    ));
    assert_eq!(store.highest_account_id().unwrap(), Some(id));
}
