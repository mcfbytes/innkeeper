//! Soft delete, legal holds and the hourly hard purge (docs/server/privacy.md).

use std::collections::HashMap;

use innkeeper_world::{MailboxNumber, RetentionPolicy, StoreError, COMPLAINT_BOARD_LOCATION};
use rusqlite::{params, Connection, OptionalExtension};
use tracing::info;

use crate::dates;
use crate::sqlite_store::SqliteStore;
use crate::subject::{self, post_author};

/// Rows the account owns, gathered before the write so the transaction stays short.
pub(crate) struct OwnedRows {
    pub mailbox: Option<u32>,
    pub sent: Vec<u32>,
    pub posts: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disable {
    Fresh(i64),
    Already(i64),
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Restored {
    Ok,
    Missing,
    NotDeleted,
    WindowClosed { deleted_at: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PurgeNow {
    Done {
        letters: u64,
        posts: u64,
        saves: u64,
    },
    Missing,
    Held,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PurgeCounts {
    pub audit: u64,
    pub letters: u64,
    pub posts: u64,
    pub complaints: u64,
    pub accounts: u64,
    pub mailboxes: u64,
    pub money: u64,
    pub saves: u64,
    pub sessions: u64,
    pub identities: u64,
}

pub(crate) fn log_counts(counts: &PurgeCounts) {
    info!(
        audit = counts.audit,
        letters = counts.letters,
        posts = counts.posts,
        complaints = counts.complaints,
        accounts = counts.accounts,
        mailboxes = counts.mailboxes,
        money = counts.money,
        saves = counts.saves,
        sessions = counts.sessions,
        identities = counts.identities,
        "retention purge"
    );
}

/// Marks the account and what it owns. A second call keeps the original `deleted_at`.
pub(crate) fn disable_account(
    store: &SqliteStore,
    account: u32,
    owned: &OwnedRows,
) -> Result<Disable, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    let existing = deleted_at(&tx, account)?;
    let stamp = match existing {
        None => return Ok(Disable::Missing),
        Some(Some(stamp)) => {
            tx.commit().map_err(StoreError::backend)?;
            return Ok(Disable::Already(stamp));
        }
        Some(None) => dates::unix_now(),
    };
    tx.execute(
        "UPDATE accounts SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        params![account, stamp],
    )
    .map_err(StoreError::backend)?;
    tx.execute(
        "UPDATE mailboxes SET deleted_at = ?2 WHERE account = ?1 AND deleted_at IS NULL",
        params![account, stamp],
    )
    .map_err(StoreError::backend)?;
    if let Some(mailbox) = owned.mailbox {
        tx.execute(
            "UPDATE letters SET deleted_at = ?2 WHERE mailbox = ?1 AND deleted_at IS NULL",
            params![mailbox, stamp],
        )
        .map_err(StoreError::backend)?;
    }
    for id in &owned.sent {
        tx.execute(
            "UPDATE letters SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, stamp],
        )
        .map_err(StoreError::backend)?;
    }
    for id in &owned.posts {
        tx.execute(
            "UPDATE posts SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, stamp],
        )
        .map_err(StoreError::backend)?;
    }
    tx.execute(
        "UPDATE money SET deleted_at = ?2 WHERE account = ?1 AND deleted_at IS NULL",
        params![account, stamp],
    )
    .map_err(StoreError::backend)?;
    tx.execute(
        "UPDATE saves SET deleted_at = ?2 WHERE account = ?1 AND deleted_at IS NULL",
        params![account, stamp],
    )
    .map_err(StoreError::backend)?;
    disable_identity(&tx, account, stamp)?;
    tx.commit().map_err(StoreError::backend)?;
    Ok(Disable::Fresh(stamp))
}

/// The owning identity is stamped too once it has no live account left, and its sessions end.
fn disable_identity(conn: &Connection, account: u32, stamp: i64) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE identities SET deleted_at = ?2
         WHERE deleted_at IS NULL
         AND identity_key = (SELECT identity_key FROM identity_accounts WHERE account_id = ?1)
         AND NOT EXISTS (
             SELECT 1 FROM identity_accounts JOIN accounts ON accounts.id = identity_accounts.account_id
             WHERE identity_accounts.identity_key = identities.identity_key
             AND accounts.deleted_at IS NULL
         )",
        params![account, stamp],
    )
    .map_err(StoreError::backend)?;
    conn.execute(
        "UPDATE sessions SET expiry = min(expiry, ?1)
         WHERE identity_key IN (SELECT identity_key FROM identities WHERE deleted_at = ?1)",
        [stamp],
    )
    .map_err(StoreError::backend)?;
    Ok(())
}

/// Clears `deleted_at` only on rows stamped by the account deletion itself.
pub(crate) fn restore_account(
    store: &SqliteStore,
    account: u32,
    policy: RetentionPolicy,
    now: i64,
) -> Result<Restored, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    let stamp = match deleted_at(&tx, account)? {
        None => return Ok(Restored::Missing),
        Some(None) => return Ok(Restored::NotDeleted),
        Some(Some(stamp)) => stamp,
    };
    if now >= dates::add_days(stamp, policy.account_days) {
        return Ok(Restored::WindowClosed { deleted_at: stamp });
    }
    let persona: String = tx
        .query_row(
            "SELECT persona FROM accounts WHERE id = ?1",
            [account],
            |row| row.get(0),
        )
        .map_err(StoreError::backend)?;
    let mailbox = mailbox_of(&tx, account)?;
    let letters = letter_rows(&tx)?;
    for letter in &letters {
        let in_box = mailbox.is_some_and(|number| letter.mailbox == number);
        let sent = mailbox.is_some_and(|number| {
            subject::sender_mailbox(&letter.body) == Some(MailboxNumber(number))
        });
        if letter.deleted_at == Some(stamp) && (in_box || sent) {
            tx.execute(
                "UPDATE letters SET deleted_at = NULL WHERE id = ?1 AND deleted_at = ?2",
                params![letter.id, stamp],
            )
            .map_err(StoreError::backend)?;
        }
    }
    for post in post_rows(&tx)? {
        if post.deleted_at == Some(stamp) && subject::post_is_by(&post.body, &persona) {
            tx.execute(
                "UPDATE posts SET deleted_at = NULL WHERE id = ?1 AND deleted_at = ?2",
                params![post.id, stamp],
            )
            .map_err(StoreError::backend)?;
        }
    }
    for sql in [
        "UPDATE money SET deleted_at = NULL WHERE account = ?1 AND deleted_at = ?2",
        "UPDATE saves SET deleted_at = NULL WHERE account = ?1 AND deleted_at = ?2",
        "UPDATE mailboxes SET deleted_at = NULL WHERE account = ?1 AND deleted_at = ?2",
        "UPDATE accounts SET deleted_at = NULL WHERE id = ?1 AND deleted_at = ?2",
        "UPDATE identities SET deleted_at = NULL WHERE deleted_at = ?2
         AND identity_key = (SELECT identity_key FROM identity_accounts WHERE account_id = ?1)",
    ] {
        tx.execute(sql, params![account, stamp])
            .map_err(StoreError::backend)?;
    }
    tx.commit().map_err(StoreError::backend)?;
    Ok(Restored::Ok)
}

/// Operator override. Refuses while a hold applies, and never touches sealed records.
pub(crate) fn purge_now(
    store: &SqliteStore,
    account: u32,
    now: i64,
) -> Result<PurgeNow, StoreError> {
    let conn = store.connection();
    if held(&conn, account, now)? {
        return Ok(PurgeNow::Held);
    }
    let Some((persona, _)) = account_row(&conn, account)? else {
        return Ok(PurgeNow::Missing);
    };
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    let mailbox = mailbox_of(&tx, account)?;
    let mut letters = 0u64;
    if let Some(number) = mailbox {
        letters += changed(
            tx.execute("DELETE FROM letters WHERE mailbox = ?1", [number])
                .map_err(StoreError::backend)?,
        );
    }
    let sent: Vec<u32> = letter_rows(&tx)?
        .into_iter()
        .filter(|letter| {
            mailbox.is_some_and(|number| {
                letter.mailbox != number
                    && subject::sender_mailbox(&letter.body) == Some(MailboxNumber(number))
            })
        })
        .map(|letter| letter.id)
        .collect();
    for id in sent {
        letters += changed(
            tx.execute("DELETE FROM letters WHERE id = ?1", [id])
                .map_err(StoreError::backend)?,
        );
    }
    let mut posts = 0u64;
    for post in post_rows(&tx)? {
        if subject::post_is_by(&post.body, &persona) {
            posts += changed(
                tx.execute("DELETE FROM posts WHERE id = ?1", [post.id])
                    .map_err(StoreError::backend)?,
            );
        }
    }
    let saves = changed(
        tx.execute("DELETE FROM saves WHERE account = ?1", [account])
            .map_err(StoreError::backend)?,
    );
    tx.execute("DELETE FROM money WHERE account = ?1", [account])
        .map_err(StoreError::backend)?;
    tx.execute("DELETE FROM mailboxes WHERE account = ?1", [account])
        .map_err(StoreError::backend)?;
    tx.execute("DELETE FROM accounts WHERE id = ?1", [account])
        .map_err(StoreError::backend)?;
    tx.commit().map_err(StoreError::backend)?;
    Ok(PurgeNow::Done {
        letters,
        posts,
        saves,
    })
}

/// Extends a hold. A shorter date does not shorten one already stored.
pub(crate) fn place_hold(store: &SqliteStore, account: u32, until: i64) -> Result<i64, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    tx.execute(
        "INSERT INTO holds (account, hold_until) VALUES (?1, ?2)
         ON CONFLICT(account) DO UPDATE SET hold_until = max(hold_until, excluded.hold_until)",
        params![account, until],
    )
    .map_err(StoreError::backend)?;
    tx.execute(
        "UPDATE audit_sessions
         SET hold_until = max(ifnull(hold_until, 0), (SELECT hold_until FROM holds WHERE account = ?1))
         WHERE account_id = ?1",
        [account],
    )
    .map_err(StoreError::backend)?;
    let stored: i64 = tx
        .query_row(
            "SELECT hold_until FROM holds WHERE account = ?1",
            [account],
            |row| row.get(0),
        )
        .map_err(StoreError::backend)?;
    tx.commit().map_err(StoreError::backend)?;
    Ok(stored)
}

pub(crate) fn clear_hold(store: &SqliteStore, account: u32) -> Result<(), StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    tx.execute("DELETE FROM holds WHERE account = ?1", [account])
        .map_err(StoreError::backend)?;
    tx.execute(
        "UPDATE audit_sessions SET hold_until = NULL WHERE account_id = ?1",
        [account],
    )
    .map_err(StoreError::backend)?;
    tx.commit().map_err(StoreError::backend)?;
    Ok(())
}

pub(crate) fn account_held(
    store: &SqliteStore,
    account: u32,
    now: i64,
) -> Result<bool, StoreError> {
    held(&store.connection(), account, now)
}

/// Hard-deletes rows whose clock has run out. A hold skips that account's rows.
pub(crate) fn purge_expired(
    store: &SqliteStore,
    policy: RetentionPolicy,
    now: i64,
) -> Result<PurgeCounts, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    let audit = purge_audit(&tx, now, policy)?;
    let letters = purge_letters(&tx, now, policy)?;
    let (posts, complaints) = purge_posts(&tx, now, policy)?;
    let (accounts, mailboxes, money, saves) = purge_accounts(&tx, now, policy)?;
    let sessions = purge_sessions(&tx, now, policy)?;
    let identities = purge_identities(&tx, now)?;
    let counts = PurgeCounts {
        audit,
        letters,
        posts,
        complaints,
        accounts,
        mailboxes,
        money,
        saves,
        sessions,
        identities,
    };
    tx.commit().map_err(StoreError::backend)?;
    Ok(counts)
}

fn purge_audit(conn: &Connection, now: i64, policy: RetentionPolicy) -> Result<u64, StoreError> {
    let cutoff = now.saturating_sub(RetentionPolicy::seconds(policy.audit_days));
    let n = conn
        .execute(
            "DELETE FROM audit_sessions
             WHERE started_at < ?1
             AND (hold_until IS NULL OR hold_until <= ?2)
             AND NOT EXISTS (
                 SELECT 1 FROM holds
                 WHERE holds.account = audit_sessions.account_id AND holds.hold_until > ?2
             )",
            params![cutoff, now],
        )
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

fn purge_letters(conn: &Connection, now: i64, policy: RetentionPolicy) -> Result<u64, StoreError> {
    let cutoff = now.saturating_sub(RetentionPolicy::seconds(policy.mail_days));
    let due = letter_rows(conn)?
        .into_iter()
        .filter(|letter| letter.deleted_at.is_some_and(|stamp| stamp < cutoff))
        .collect::<Vec<_>>();
    let mut owners = HashMap::new();
    let mut held_accounts = HashMap::new();
    let mut removed = 0u64;
    for letter in due {
        if letter_held(conn, &letter, now, &mut owners, &mut held_accounts)? {
            continue;
        }
        removed += changed(
            conn.execute("DELETE FROM letters WHERE id = ?1", [letter.id])
                .map_err(StoreError::backend)?,
        );
    }
    Ok(removed)
}

fn letter_held(
    conn: &Connection,
    letter: &LetterRow,
    now: i64,
    owners: &mut HashMap<u32, Option<u32>>,
    held_accounts: &mut HashMap<u32, bool>,
) -> Result<bool, StoreError> {
    if owner_held(conn, letter.mailbox, now, owners, held_accounts)? {
        return Ok(true);
    }
    let Some(sender) = subject::sender_mailbox(&letter.body) else {
        return Ok(false);
    };
    owner_held(conn, sender.0, now, owners, held_accounts)
}

fn owner_held(
    conn: &Connection,
    mailbox: u32,
    now: i64,
    owners: &mut HashMap<u32, Option<u32>>,
    held_accounts: &mut HashMap<u32, bool>,
) -> Result<bool, StoreError> {
    let account = match owners.get(&mailbox) {
        Some(account) => *account,
        None => {
            let account = mailbox_account(conn, mailbox)?;
            owners.insert(mailbox, account);
            account
        }
    };
    match account {
        Some(account) => account_is_held(conn, account, now, held_accounts),
        None => Ok(false),
    }
}

fn purge_posts(
    conn: &Connection,
    now: i64,
    policy: RetentionPolicy,
) -> Result<(u64, u64), StoreError> {
    let post_cutoff = now.saturating_sub(RetentionPolicy::seconds(policy.post_days));
    let complaint_cutoff = now.saturating_sub(RetentionPolicy::seconds(policy.complaint_days));
    let location = i64::from(COMPLAINT_BOARD_LOCATION);
    let ordinary = {
        let mut statement = conn
            .prepare(
                "SELECT id, body FROM posts
                 WHERE deleted_at IS NOT NULL AND deleted_at < ?1
                 AND ((board >> 8) & 65535) != ?2",
            )
            .map_err(StoreError::backend)?;
        let rows = statement
            .query_map(params![post_cutoff, location], |row| {
                Ok((row.get::<_, u32>(0)?, row.get(1)?))
            })
            .map_err(StoreError::backend)?;
        rows.collect::<Result<Vec<(u32, Vec<u8>)>, _>>()
            .map_err(StoreError::backend)?
    };
    let complaints = {
        let mut statement = conn
            .prepare(
                "SELECT id, body FROM posts
                 WHERE created_at < ?1 AND ((board >> 8) & 65535) = ?2",
            )
            .map_err(StoreError::backend)?;
        let rows = statement
            .query_map(params![complaint_cutoff, location], |row| {
                Ok((row.get::<_, u32>(0)?, row.get(1)?))
            })
            .map_err(StoreError::backend)?;
        rows.collect::<Result<Vec<(u32, Vec<u8>)>, _>>()
            .map_err(StoreError::backend)?
    };
    let mut held_accounts = HashMap::new();
    Ok((
        delete_unheld_posts(conn, &ordinary, now, &mut held_accounts)?,
        delete_unheld_posts(conn, &complaints, now, &mut held_accounts)?,
    ))
}

fn delete_unheld_posts(
    conn: &Connection,
    posts: &[(u32, Vec<u8>)],
    now: i64,
    held_accounts: &mut HashMap<u32, bool>,
) -> Result<u64, StoreError> {
    let mut removed = 0u64;
    for (id, body) in posts {
        if author_held(conn, body, now, held_accounts)? {
            continue;
        }
        removed += changed(
            conn.execute("DELETE FROM posts WHERE id = ?1", [id])
                .map_err(StoreError::backend)?,
        );
    }
    Ok(removed)
}

fn author_held(
    conn: &Connection,
    body: &[u8],
    now: i64,
    held_accounts: &mut HashMap<u32, bool>,
) -> Result<bool, StoreError> {
    let Some(author) = post_author(body) else {
        return Ok(false);
    };
    let mut statement = conn
        .prepare("SELECT id FROM accounts WHERE persona = ?1 COLLATE NOCASE")
        .map_err(StoreError::backend)?;
    let ids = statement
        .query_map([author], |row| row.get::<_, u32>(0))
        .map_err(StoreError::backend)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::backend)?;
    drop(statement);
    for id in ids {
        if account_is_held(conn, id, now, held_accounts)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn purge_accounts(
    conn: &Connection,
    now: i64,
    policy: RetentionPolicy,
) -> Result<(u64, u64, u64, u64), StoreError> {
    let cutoff = now.saturating_sub(RetentionPolicy::seconds(policy.account_days));
    let due = conn
        .prepare("SELECT id FROM accounts WHERE deleted_at IS NOT NULL AND deleted_at < ?1")
        .map_err(StoreError::backend)?
        .query_map([cutoff], |row| row.get::<_, u32>(0))
        .map_err(StoreError::backend)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::backend)?;
    let mut accounts = 0u64;
    let mut mailboxes = 0u64;
    let mut money = 0u64;
    let mut saves = 0u64;
    let mut held_accounts = HashMap::new();
    for id in due {
        if account_is_held(conn, id, now, &mut held_accounts)? {
            continue;
        }
        money += changed(
            conn.execute("DELETE FROM money WHERE account = ?1", [id])
                .map_err(StoreError::backend)?,
        );
        saves += changed(
            conn.execute("DELETE FROM saves WHERE account = ?1", [id])
                .map_err(StoreError::backend)?,
        );
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM letters
                 WHERE mailbox = (SELECT number FROM mailboxes WHERE account = ?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(StoreError::backend)?;
        if remaining > 0 {
            continue;
        }
        mailboxes += changed(
            conn.execute("DELETE FROM mailboxes WHERE account = ?1", [id])
                .map_err(StoreError::backend)?,
        );
        accounts += changed(
            conn.execute("DELETE FROM accounts WHERE id = ?1", [id])
                .map_err(StoreError::backend)?,
        );
    }
    Ok((accounts, mailboxes, money, saves))
}

/// A hold on the session's account, or on any account its identity owns, keeps the row.
fn purge_sessions(conn: &Connection, now: i64, policy: RetentionPolicy) -> Result<u64, StoreError> {
    let cutoff = now.saturating_sub(RetentionPolicy::seconds(policy.session_days));
    let n = conn
        .execute(
            "DELETE FROM sessions
             WHERE expiry < ?1
             AND NOT EXISTS (
                 SELECT 1 FROM holds
                 WHERE holds.hold_until > ?2
                 AND (holds.account = sessions.account_id OR holds.account IN (
                     SELECT account_id FROM identity_accounts
                     WHERE identity_accounts.identity_key = sessions.identity_key
                 ))
             )",
            params![cutoff, now],
        )
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

/// An identity that owns no account and has no session row left is gone. An hour's grace
/// covers a sign-in between creating the identity and its session.
fn purge_identities(conn: &Connection, now: i64) -> Result<u64, StoreError> {
    let n = conn
        .execute(
            "DELETE FROM identities
             WHERE created_at < ?1
             AND NOT EXISTS (
                 SELECT 1 FROM identity_accounts
                 WHERE identity_accounts.identity_key = identities.identity_key
             )
             AND NOT EXISTS (
                 SELECT 1 FROM sessions WHERE sessions.identity_key = identities.identity_key
             )",
            [now.saturating_sub(3600)],
        )
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

/// `None` when the account is gone, `Some(None)` when it is live.
fn deleted_at(conn: &Connection, account: u32) -> Result<Option<Option<i64>>, StoreError> {
    conn.query_row(
        "SELECT deleted_at FROM accounts WHERE id = ?1",
        [account],
        |row| row.get(0),
    )
    .optional()
    .map_err(StoreError::backend)
}

fn account_row(
    conn: &Connection,
    account: u32,
) -> Result<Option<(String, Option<i64>)>, StoreError> {
    conn.query_row(
        "SELECT persona, deleted_at FROM accounts WHERE id = ?1",
        [account],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(StoreError::backend)
}

fn mailbox_of(conn: &Connection, account: u32) -> Result<Option<u32>, StoreError> {
    conn.query_row(
        "SELECT number FROM mailboxes WHERE account = ?1",
        [account],
        |row| row.get(0),
    )
    .optional()
    .map_err(StoreError::backend)
}

fn mailbox_account(conn: &Connection, mailbox: u32) -> Result<Option<u32>, StoreError> {
    conn.query_row(
        "SELECT account FROM mailboxes WHERE number = ?1",
        [mailbox],
        |row| row.get(0),
    )
    .optional()
    .map_err(StoreError::backend)
}

fn held(conn: &Connection, account: u32, now: i64) -> Result<bool, StoreError> {
    let mut cache = HashMap::new();
    account_is_held(conn, account, now, &mut cache)
}

fn account_is_held(
    conn: &Connection,
    account: u32,
    now: i64,
    cache: &mut HashMap<u32, bool>,
) -> Result<bool, StoreError> {
    if let Some(known) = cache.get(&account) {
        return Ok(*known);
    }
    let until: Option<i64> = conn
        .query_row(
            "SELECT hold_until FROM holds WHERE account = ?1",
            [account],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::backend)?;
    let held = until.is_some_and(|until| until > now);
    cache.insert(account, held);
    Ok(held)
}

struct LetterRow {
    id: u32,
    mailbox: u32,
    body: Vec<u8>,
    deleted_at: Option<i64>,
}

struct PostRow {
    id: u32,
    body: Vec<u8>,
    deleted_at: Option<i64>,
}

fn letter_rows(conn: &Connection) -> Result<Vec<LetterRow>, StoreError> {
    let mut statement = conn
        .prepare("SELECT id, mailbox, body, deleted_at FROM letters")
        .map_err(StoreError::backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok(LetterRow {
                id: row.get(0)?,
                mailbox: row.get(1)?,
                body: row.get(2)?,
                deleted_at: row.get(3)?,
            })
        })
        .map_err(StoreError::backend)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::backend)
}

fn post_rows(conn: &Connection) -> Result<Vec<PostRow>, StoreError> {
    let mut statement = conn
        .prepare("SELECT id, body, deleted_at FROM posts")
        .map_err(StoreError::backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok(PostRow {
                id: row.get(0)?,
                body: row.get(1)?,
                deleted_at: row.get(2)?,
            })
        })
        .map_err(StoreError::backend)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::backend)
}

fn changed(rows: usize) -> u64 {
    u64::try_from(rows).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use innkeeper_world::{
        AccountId, AccountRecord, BoardId, EncodedPassword, Money, Store, COMPLAINT_BOARD_LOCATION,
    };
    use rusqlite::params;

    use super::*;

    fn memory() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    fn policy() -> RetentionPolicy {
        RetentionPolicy::default()
    }

    fn add_account(store: &SqliteStore, id: u32, persona: &str) -> u32 {
        store
            .create_account(
                AccountId(id),
                AccountRecord::new(EncodedPassword([1; 10]), persona),
            )
            .unwrap();
        store.assign_mailbox(AccountId(id)).unwrap().0
    }

    fn letter_body(from: u32) -> Vec<u8> {
        let mut body = vec![96, 0, 1, 12, 0, 0];
        body.extend([0u8; 10]);
        body.extend((from as u16).to_le_bytes());
        body.extend(((from >> 16) as u16).to_le_bytes());
        body.extend([0u8; 32]);
        body.push(0);
        body
    }

    fn count(store: &SqliteStore, sql: &str) -> i64 {
        store
            .connection()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn purge_drops_old_audit_rows_unless_they_are_held() {
        let store = memory();
        let now = 1_700_000_000;
        let old = now - 91 * 86_400;
        store
            .connection()
            .execute(
                "INSERT INTO audit_sessions
                 (started_at, account_id, sealed) VALUES (?1, 1, x'00'), (?2, 1, x'00'), (?1, 2, x'00')",
                params![old, now],
            )
            .unwrap();
        place_hold(&store, 2, now + 86_400).unwrap();
        let counts = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(counts.audit, 1);
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM audit_sessions WHERE account_id = 1"
            ),
            1
        );
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM audit_sessions WHERE account_id = 2"
            ),
            1
        );
    }

    #[test]
    fn a_deleted_letter_is_purged_after_the_window_unless_its_sender_is_held() {
        let store = memory();
        let now = dates::unix_now();
        let from = add_account(&store, 1, "alice");
        let to = add_account(&store, 2, "bob");
        let id = store
            .append_letter(innkeeper_world::MailboxNumber(to), &letter_body(from))
            .unwrap();
        assert!(store
            .delete_letter(innkeeper_world::MailboxNumber(to), id)
            .unwrap());
        store
            .connection()
            .execute("UPDATE letters SET deleted_at = ?1", [now - 91 * 86_400])
            .unwrap();
        place_hold(&store, 1, now + 10 * 86_400).unwrap();
        let held = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(held.letters, 0);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM letters"), 1);
        clear_hold(&store, 1).unwrap();
        let purged = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(purged.letters, 1);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM letters"), 0);
    }

    #[test]
    fn a_deleted_account_keeps_a_shell_until_its_letters_age_out() {
        let store = memory();
        let now = dates::unix_now();
        let mailbox = add_account(&store, 5, "cara");
        store
            .append_letter(innkeeper_world::MailboxNumber(mailbox), b"note")
            .unwrap();
        store.set_money(AccountId(5), Money(9)).unwrap();
        store
            .save_write(AccountId(5), "INN\\A", b"save", 1, true)
            .unwrap();
        let owned = OwnedRows {
            mailbox: Some(mailbox),
            sent: Vec::new(),
            posts: Vec::new(),
        };
        assert!(matches!(
            disable_account(&store, 5, &owned).unwrap(),
            Disable::Fresh(_)
        ));
        store
            .connection()
            .execute(
                "UPDATE accounts SET deleted_at = ?1 WHERE id = 5",
                [now - 31 * 86_400],
            )
            .unwrap();
        let first = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(first.accounts, 0);
        assert_eq!(first.money, 1);
        assert_eq!(first.saves, 1);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM accounts"), 1);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM letters"), 1);
        store
            .connection()
            .execute("UPDATE letters SET deleted_at = ?1", [now - 91 * 86_400])
            .unwrap();
        let second = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(second.letters, 1);
        assert_eq!(second.accounts, 1);
        assert_eq!(second.mailboxes, 1);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM accounts"), 0);
    }

    #[test]
    fn complaints_purge_from_arrival_and_a_hold_blocks_them() {
        let store = memory();
        let now = dates::unix_now();
        add_account(&store, 8, "mod");
        let ordinary = store.post(BoardId(1), b"mod|tavern").unwrap();
        let board = BoardId(u32::from(COMPLAINT_BOARD_LOCATION) << 8);
        let complaint = store.post(board, b"mod|abuse").unwrap();
        store
            .connection()
            .execute("UPDATE posts SET deleted_at = ?1", [now - 91 * 86_400])
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE posts SET created_at = ?1 WHERE id = ?2",
                params![now - 10 * 86_400, complaint.0],
            )
            .unwrap();
        let counts = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(counts.posts, 1);
        assert_eq!(counts.complaints, 0);
        assert!(store.posts(BoardId(1)).unwrap().is_empty());
        store
            .connection()
            .execute(
                "UPDATE posts SET created_at = ?1 WHERE id = ?2",
                params![now - 400 * 86_400, complaint.0],
            )
            .unwrap();
        place_hold(&store, 8, now + 86_400).unwrap();
        let held = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(held.complaints, 0);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM posts"), 1);
        clear_hold(&store, 8).unwrap();
        let gone = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(gone.complaints, 1);
        let _ = ordinary;
    }

    #[test]
    fn restore_undoes_only_the_account_deletion_and_purge_now_spares_audit_rows() {
        let store = memory();
        let now = dates::unix_now();
        let mailbox = add_account(&store, 4, "dee");
        let early = store
            .append_letter(innkeeper_world::MailboxNumber(mailbox), b"early")
            .unwrap();
        let later = store
            .append_letter(innkeeper_world::MailboxNumber(mailbox), b"later")
            .unwrap();
        store
            .delete_letter(innkeeper_world::MailboxNumber(mailbox), early)
            .unwrap();
        store
            .connection()
            .execute("UPDATE letters SET deleted_at = 1 WHERE id = ?1", [early.0])
            .unwrap();
        let owned = OwnedRows {
            mailbox: Some(mailbox),
            sent: Vec::new(),
            posts: Vec::new(),
        };
        disable_account(&store, 4, &owned).unwrap();
        assert!(matches!(
            restore_account(&store, 4, policy(), now).unwrap(),
            Restored::Ok
        ));
        let visible = store
            .letters(innkeeper_world::MailboxNumber(mailbox))
            .unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, later);
        assert!(store.account(AccountId(4)).unwrap().is_some());

        store
            .connection()
            .execute(
                "INSERT INTO audit_sessions (started_at, account_id, sealed) VALUES (?1, 4, x'00')",
                [now],
            )
            .unwrap();
        place_hold(&store, 4, now + 86_400).unwrap();
        assert!(matches!(purge_now(&store, 4, now).unwrap(), PurgeNow::Held));
        clear_hold(&store, 4).unwrap();
        assert!(matches!(
            purge_now(&store, 4, now).unwrap(),
            PurgeNow::Done { .. }
        ));
        assert!(store.account(AccountId(4)).unwrap().is_none());
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM accounts WHERE id = 4"),
            0
        );
        assert_eq!(count(&store, "SELECT COUNT(*) FROM audit_sessions"), 1);
    }

    fn add_identity(store: &SqliteStore, key: u8, account: Option<u32>, now: i64) {
        let conn = store.connection();
        conn.execute(
            "INSERT INTO identities (identity_key, key_version, provider, created_at)
             VALUES (?1, 'v', 'p', ?2)",
            params![vec![key; 32], now - 7 * 86_400],
        )
        .unwrap();
        if let Some(account) = account {
            conn.execute(
                "INSERT INTO identity_accounts (account_id, identity_key, created_at)
                 VALUES (?1, ?2, ?3)",
                params![account, vec![key; 32], now],
            )
            .unwrap();
        }
    }

    fn add_session(store: &SqliteStore, token: u8, key: u8, account: Option<u32>, expiry: i64) {
        store
            .connection()
            .execute(
                "INSERT INTO sessions (token_hash, identity_key, account_id, created_at, expiry)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![vec![token; 32], vec![key; 32], account, expiry],
            )
            .unwrap();
    }

    #[test]
    fn expired_session_rows_are_purged_unless_a_hold_covers_the_identity() {
        let store = memory();
        let now = 1_700_000_000;
        add_account(&store, 1, "one");
        add_account(&store, 2, "two");
        add_identity(&store, 1, Some(1), now);
        add_identity(&store, 2, Some(2), now);
        let old = now - 40 * 86_400;
        add_session(&store, 1, 1, Some(1), old);
        add_session(&store, 2, 2, Some(2), old);
        add_session(&store, 3, 1, Some(1), now);
        add_session(&store, 4, 2, None, old);
        place_hold(&store, 2, now + 86_400).unwrap();
        let counts = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(counts.sessions, 1);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM sessions"), 3);
    }

    #[test]
    fn an_identity_with_no_account_and_no_session_is_purged() {
        let store = memory();
        let now = 1_700_000_000;
        add_account(&store, 1, "one");
        add_identity(&store, 1, Some(1), now);
        add_identity(&store, 2, None, now);
        add_identity(&store, 3, None, now);
        add_session(&store, 1, 3, None, now + 3600);
        let counts = purge_expired(&store, policy(), now).unwrap();
        assert_eq!(counts.identities, 1);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM identities"), 2);
    }

    #[test]
    fn deleting_the_last_account_retires_the_identity_and_ends_its_sessions() {
        let store = memory();
        let now = dates::unix_now();
        add_account(&store, 1, "one");
        add_account(&store, 2, "two");
        add_identity(&store, 1, Some(1), now);
        store
            .connection()
            .execute(
                "INSERT INTO identity_accounts (account_id, identity_key, created_at) VALUES (2, ?1, 0)",
                [vec![1u8; 32]],
            )
            .unwrap();
        add_session(&store, 1, 1, None, now + 86_400);
        let owned = OwnedRows {
            mailbox: None,
            sent: Vec::new(),
            posts: Vec::new(),
        };
        disable_account(&store, 1, &owned).unwrap();
        let live = "SELECT COUNT(*) FROM identities WHERE deleted_at IS NULL";
        assert_eq!(count(&store, live), 1, "one account is still live");
        disable_account(&store, 2, &owned).unwrap();
        assert_eq!(count(&store, live), 0);
        let open = format!(
            "SELECT COUNT(*) FROM sessions WHERE expiry > {}",
            now + 3600
        );
        assert_eq!(count(&store, &open), 0);
        restore_account(&store, 2, policy(), dates::unix_now()).unwrap();
        assert_eq!(count(&store, live), 1);
    }
}
