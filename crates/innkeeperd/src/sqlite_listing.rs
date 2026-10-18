//! The bounded reads of letters and posts: counts without bodies, one page or one row by `LIMIT`,
//! and what an account wrote lately for the quotas (docs/server/store.md section 2).

use innkeeper_world::{
    AccountId, BoardId, Letter, LetterId, MailboxNumber, Post, PostId, StoreError, Written,
};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::dates;

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::backend(error)
}

/// SQLite's `LIMIT` and `OFFSET` are signed; a count past `i64::MAX` means "all".
fn sql_count(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

fn letter_of(row: &Row<'_>) -> rusqlite::Result<Letter> {
    Ok(Letter {
        id: LetterId(row.get(0)?),
        from: AccountId(row.get(1)?),
        body: row.get(2)?,
    })
}

fn post_of(row: &Row<'_>) -> rusqlite::Result<Post> {
    Ok(Post {
        id: PostId(row.get(0)?),
        body: row.get(1)?,
        account: AccountId(row.get(2)?),
        about: row.get::<_, Option<u32>>(3)?.map(AccountId),
    })
}

pub(crate) fn letter_count(conn: &Connection, mailbox: MailboxNumber) -> Result<usize, StoreError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM letters WHERE to_mailbox = ?1 AND deleted_at IS NULL",
            [mailbox.0],
            |row| row.get(0),
        )
        .map_err(backend)?;
    Ok(usize::try_from(count).unwrap_or_default())
}

pub(crate) fn first_letters(
    conn: &Connection,
    mailbox: MailboxNumber,
    limit: usize,
) -> Result<Vec<Letter>, StoreError> {
    let mut statement = conn
        .prepare_cached(
            "SELECT id, from_account, body FROM letters
             WHERE to_mailbox = ?1 AND deleted_at IS NULL ORDER BY id LIMIT ?2",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map(rusqlite::params![mailbox.0, sql_count(limit)], letter_of)
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

pub(crate) fn letter(
    conn: &Connection,
    mailbox: MailboxNumber,
    id: LetterId,
) -> Result<Option<Letter>, StoreError> {
    conn.query_row(
        "SELECT id, from_account, body FROM letters
         WHERE id = ?1 AND to_mailbox = ?2 AND deleted_at IS NULL",
        [id.0, mailbox.0],
        letter_of,
    )
    .optional()
    .map_err(backend)
}

pub(crate) fn post_count(conn: &Connection, board: BoardId) -> Result<usize, StoreError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM posts WHERE board = ?1 AND deleted_at IS NULL",
            [board.0],
            |row| row.get(0),
        )
        .map_err(backend)?;
    Ok(usize::try_from(count).unwrap_or_default())
}

pub(crate) fn post_at(
    conn: &Connection,
    board: BoardId,
    index: usize,
) -> Result<Option<Post>, StoreError> {
    conn.query_row(
        "SELECT id, body, account, about_account FROM posts
         WHERE board = ?1 AND deleted_at IS NULL ORDER BY id LIMIT 1 OFFSET ?2",
        rusqlite::params![board.0, sql_count(index)],
        post_of,
    )
    .optional()
    .map_err(backend)
}

pub(crate) fn written_within(
    conn: &Connection,
    author: AccountId,
    window_ms: i64,
) -> Result<Written, StoreError> {
    let since = dates::now_ms().saturating_sub(window_ms);
    let count = |sql: &str| -> Result<usize, StoreError> {
        let count: i64 = conn
            .query_row(sql, rusqlite::params![author.0, since], |row| row.get(0))
            .map_err(backend)?;
        Ok(usize::try_from(count).unwrap_or_default())
    };
    Ok(Written {
        letters: count("SELECT COUNT(*) FROM letters WHERE from_account = ?1 AND created_at > ?2")?,
        posts: count("SELECT COUNT(*) FROM posts WHERE account = ?1 AND created_at > ?2")?,
    })
}
