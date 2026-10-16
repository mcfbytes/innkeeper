//! Soft delete, legal holds and the hourly hard purge (docs/server/privacy.md section 6). What an
//! account owns is found by its number alone: `letters.from_account`, the mailbox's `account`,
//! `posts.account`. A purged account becomes a tombstone (docs/server/store.md section 3).

use innkeeper_world::{AccountId, RetentionPolicy, StoreError};
use rusqlite::{params, Connection, OptionalExtension};
use tracing::info;

use crate::dates;
use crate::sqlite_store::SqliteStore;

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
    /// Accounts turned into tombstones.
    pub accounts: u64,
    pub mailboxes: u64,
    pub money: u64,
    pub saves: u64,
    pub sessions: u64,
    pub identities: u64,
    pub admin_audit: u64,
    pub admin_sessions: u64,
    pub operators: u64,
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
        admin_audit = counts.admin_audit,
        admin_sessions = counts.admin_sessions,
        operators = counts.operators,
        "retention purge"
    );
}

/// The kinds that keep the complaint clock (365 days from arrival) and outlive an account purge.
const COMPLAINT_KINDS: &str = "('complaint', 'member_services')";

/// True when a hold on account `?` (the SQL text names it) is still running at `?2`.
fn held_sql(account: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM holds WHERE holds.account = {account} AND holds.expires_at > ?2)"
    )
}

/// Marks the account and what it owns: its mailbox and the letters in it, the letters it sent,
/// the posts it wrote, its money and its saves. A second call keeps the original `deleted_at`.
pub(crate) fn disable_account(store: &SqliteStore, account: u32) -> Result<Disable, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    let stamp = match deleted_at(&tx, account)? {
        None => return Ok(Disable::Missing),
        Some(Some(stamp)) => {
            tx.commit().map_err(StoreError::backend)?;
            return Ok(Disable::Already(stamp));
        }
        Some(None) => dates::now_ms(),
    };
    for sql in [
        "UPDATE accounts SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        "UPDATE mailboxes SET deleted_at = ?2 WHERE account = ?1 AND deleted_at IS NULL",
        "UPDATE letters SET deleted_at = ?2 WHERE deleted_at IS NULL
         AND (from_account = ?1 OR to_mailbox IN (SELECT number FROM mailboxes WHERE account = ?1))",
        "UPDATE posts SET deleted_at = ?2 WHERE account = ?1 AND deleted_at IS NULL",
        "UPDATE money SET deleted_at = ?2 WHERE account = ?1 AND deleted_at IS NULL",
        "UPDATE saves SET deleted_at = ?2 WHERE account = ?1 AND deleted_at IS NULL",
    ] {
        tx.execute(sql, params![account, stamp])
            .map_err(StoreError::backend)?;
    }
    disable_identity(&tx, account, stamp)?;
    tx.commit().map_err(StoreError::backend)?;
    Ok(Disable::Fresh(stamp))
}

/// The owning identity is stamped too once it has no live account left, and its sessions end.
fn disable_identity(conn: &Connection, account: u32, stamp: i64) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE identities SET deleted_at = ?2
         WHERE deleted_at IS NULL
         AND identity_key = (SELECT identity_key FROM identity_accounts WHERE account = ?1)
         AND NOT EXISTS (
             SELECT 1 FROM identity_accounts JOIN accounts ON accounts.id = identity_accounts.account
             WHERE identity_accounts.identity_key = identities.identity_key
             AND accounts.deleted_at IS NULL
         )",
        params![account, stamp],
    )
    .map_err(StoreError::backend)?;
    conn.execute(
        "UPDATE sessions SET expires_at = min(expires_at, ?1)
         WHERE identity_key IN (SELECT identity_key FROM identities WHERE deleted_at = ?1)",
        [stamp],
    )
    .map_err(StoreError::backend)?;
    Ok(())
}

/// Clears `deleted_at` only on rows stamped by the account deletion itself. A tombstone is past
/// its window.
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
    if now >= dates::add_days(stamp, policy.account_days) || purged(&tx, account)? {
        return Ok(Restored::WindowClosed { deleted_at: stamp });
    }
    for sql in [
        "UPDATE letters SET deleted_at = NULL WHERE deleted_at = ?2
         AND (from_account = ?1 OR to_mailbox IN (SELECT number FROM mailboxes WHERE account = ?1))",
        "UPDATE posts SET deleted_at = NULL WHERE account = ?1 AND deleted_at = ?2",
        "UPDATE money SET deleted_at = NULL WHERE account = ?1 AND deleted_at = ?2",
        "UPDATE saves SET deleted_at = NULL WHERE account = ?1 AND deleted_at = ?2",
        "UPDATE mailboxes SET deleted_at = NULL WHERE account = ?1 AND deleted_at = ?2",
        "UPDATE accounts SET deleted_at = NULL WHERE id = ?1 AND deleted_at = ?2",
        "UPDATE identities SET deleted_at = NULL WHERE deleted_at = ?2
         AND identity_key = (SELECT identity_key FROM identity_accounts WHERE account = ?1)",
    ] {
        tx.execute(sql, params![account, stamp])
            .map_err(StoreError::backend)?;
    }
    tx.commit().map_err(StoreError::backend)?;
    Ok(Restored::Ok)
}

/// Operator override: purges the account now instead of at the end of its window. Refuses while a
/// hold applies. Its letters (received and sent), its ordinary posts, money, saves, mailbox and
/// identity link go; the row becomes a tombstone. Sealed connection records and the complaints it
/// wrote stay for their own retention.
pub(crate) fn purge_now(
    store: &SqliteStore,
    account: u32,
    now: i64,
) -> Result<PurgeNow, StoreError> {
    let conn = store.connection();
    if held(&conn, account, now)? {
        return Ok(PurgeNow::Held);
    }
    if deleted_at(&conn, account)?.is_none() {
        return Ok(PurgeNow::Missing);
    }
    let tx = conn.unchecked_transaction().map_err(StoreError::backend)?;
    let letters = changed(
        tx.execute(
            "DELETE FROM letters WHERE from_account = ?1
             OR to_mailbox IN (SELECT number FROM mailboxes WHERE account = ?1)",
            [account],
        )
        .map_err(StoreError::backend)?,
    );
    let posts = changed(
        tx.execute(
            &format!("DELETE FROM posts WHERE account = ?1 AND kind NOT IN {COMPLAINT_KINDS}"),
            [account],
        )
        .map_err(StoreError::backend)?,
    );
    let erased = erase_personal_rows(&tx, account)?;
    tx.execute("DELETE FROM mailboxes WHERE account = ?1", [account])
        .map_err(StoreError::backend)?;
    tombstone(&tx, account, now)?;
    tx.commit().map_err(StoreError::backend)?;
    Ok(PurgeNow::Done {
        letters,
        posts,
        saves: erased.saves,
    })
}

struct Erased {
    money: u64,
    saves: u64,
}

/// The rows that have no meaning without the account's data: its bankroll, its save files and
/// the link to the identity that owned it.
fn erase_personal_rows(conn: &Connection, account: u32) -> Result<Erased, StoreError> {
    let money = changed(
        conn.execute("DELETE FROM money WHERE account = ?1", [account])
            .map_err(StoreError::backend)?,
    );
    let saves = changed(
        conn.execute("DELETE FROM saves WHERE account = ?1", [account])
            .map_err(StoreError::backend)?,
    );
    conn.execute(
        "DELETE FROM identity_accounts WHERE account = ?1",
        [account],
    )
    .map_err(StoreError::backend)?;
    Ok(Erased { money, saves })
}

/// The account row keeps its number and nothing personal. Children that outlive the account
/// (sealed records, complaints, the portal's log, letters and posts waiting out their window)
/// still name it.
fn tombstone(conn: &Connection, account: u32, now: i64) -> Result<u64, StoreError> {
    let n = conn
        .execute(
            "UPDATE accounts SET password = NULL, persona = NULL, user_flags = NULL, rating = NULL,
             status = NULL, password_expired = NULL, suspended_at = NULL,
             deleted_at = coalesce(deleted_at, ?2), purged_at = ?2
             WHERE id = ?1 AND purged_at IS NULL",
            params![account, now],
        )
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

/// Extends a hold on an account the store has a row for (live, deleted or a tombstone). A shorter
/// date does not shorten one already stored.
pub(crate) fn place_hold(store: &SqliteStore, account: u32, until: i64) -> Result<i64, StoreError> {
    let conn = store.connection();
    if deleted_at(&conn, account)?.is_none() {
        return Err(StoreError::UnknownAccount(AccountId(account)));
    }
    conn.execute(
        "INSERT INTO holds (account, placed_at, expires_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (account) DO UPDATE SET expires_at = max(expires_at, excluded.expires_at)",
        params![account, dates::now_ms(), until],
    )
    .map_err(StoreError::backend)?;
    conn.query_row(
        "SELECT expires_at FROM holds WHERE account = ?1",
        [account],
        |row| row.get(0),
    )
    .map_err(StoreError::backend)
}

pub(crate) fn clear_hold(store: &SqliteStore, account: u32) -> Result<(), StoreError> {
    store
        .connection()
        .execute("DELETE FROM holds WHERE account = ?1", [account])
        .map_err(StoreError::backend)?;
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
    let mut counts = PurgeCounts {
        audit: purge_audit(&tx, now, policy)?,
        letters: purge_letters(&tx, now, policy)?,
        ..PurgeCounts::default()
    };
    (counts.posts, counts.complaints) = purge_posts(&tx, now, policy)?;
    purge_accounts(&tx, now, policy, &mut counts)?;
    counts.sessions = purge_sessions(&tx, now, policy)?;
    counts.identities = purge_identities(&tx, now)?;
    purge_admin(&tx, now, policy, &mut counts)?;
    tx.commit().map_err(StoreError::backend)?;
    Ok(counts)
}

fn cutoff(now: i64, days: u32) -> i64 {
    now.saturating_sub(RetentionPolicy::millis(days))
}

fn purge_audit(conn: &Connection, now: i64, policy: RetentionPolicy) -> Result<u64, StoreError> {
    let sql = format!(
        "DELETE FROM audit_sessions WHERE started_at < ?1 AND NOT {}",
        held_sql("audit_sessions.account")
    );
    let n = conn
        .execute(&sql, params![cutoff(now, policy.audit_days), now])
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

/// A deleted letter past the mail window goes, unless the mailbox owner or the sender is held.
fn purge_letters(conn: &Connection, now: i64, policy: RetentionPolicy) -> Result<u64, StoreError> {
    let sql = format!(
        "DELETE FROM letters WHERE deleted_at < ?1
         AND NOT {}
         AND NOT {}",
        held_sql("letters.from_account"),
        held_sql("(SELECT account FROM mailboxes WHERE number = letters.to_mailbox)"),
    );
    let n = conn
        .execute(&sql, params![cutoff(now, policy.mail_days), now])
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

/// Ordinary posts and topics go a window after their delete; complaints and Member Services
/// messages a year after arrival, deleted or not. A hold on the writer, or on the account a
/// complaint names, keeps the row.
fn purge_posts(
    conn: &Connection,
    now: i64,
    policy: RetentionPolicy,
) -> Result<(u64, u64), StoreError> {
    let ordinary = format!(
        "DELETE FROM posts WHERE kind NOT IN {COMPLAINT_KINDS} AND deleted_at < ?1 AND NOT {}",
        held_sql("posts.account")
    );
    let complaints = format!(
        "DELETE FROM posts WHERE kind IN {COMPLAINT_KINDS} AND created_at < ?1
         AND NOT {} AND NOT {}",
        held_sql("posts.account"),
        held_sql("posts.about_account"),
    );
    let posts = conn
        .execute(&ordinary, params![cutoff(now, policy.post_days), now])
        .map_err(StoreError::backend)?;
    let complaints = conn
        .execute(
            &complaints,
            params![cutoff(now, policy.complaint_days), now],
        )
        .map_err(StoreError::backend)?;
    Ok((changed(posts), changed(complaints)))
}

/// An account past its window loses its money, saves and identity link and becomes a tombstone.
/// Its mailbox goes once no letter row points at it, in this purge or a later one: the letters keep
/// their own (longer) window.
fn purge_accounts(
    conn: &Connection,
    now: i64,
    policy: RetentionPolicy,
    counts: &mut PurgeCounts,
) -> Result<(), StoreError> {
    let due = {
        let sql = format!(
            "SELECT id FROM accounts WHERE purged_at IS NULL AND deleted_at < ?1 AND NOT {}",
            held_sql("accounts.id")
        );
        let mut statement = conn.prepare(&sql).map_err(StoreError::backend)?;
        let rows = statement
            .query_map(params![cutoff(now, policy.account_days), now], |row| {
                row.get::<_, u32>(0)
            })
            .map_err(StoreError::backend)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)?
    };
    for id in due {
        let erased = erase_personal_rows(conn, id)?;
        counts.money += erased.money;
        counts.saves += erased.saves;
        counts.accounts += tombstone(conn, id, now)?;
    }
    counts.mailboxes += changed(
        conn.execute(
            "DELETE FROM mailboxes
             WHERE account IN (SELECT id FROM accounts WHERE purged_at IS NOT NULL)
             AND NOT EXISTS (SELECT 1 FROM letters WHERE letters.to_mailbox = mailboxes.number)",
            [],
        )
        .map_err(StoreError::backend)?,
    );
    Ok(())
}

/// A hold on the session's account, or on any account its identity owns, keeps the row.
fn purge_sessions(conn: &Connection, now: i64, policy: RetentionPolicy) -> Result<u64, StoreError> {
    let n = conn
        .execute(
            "DELETE FROM sessions
             WHERE expires_at < ?1
             AND NOT EXISTS (
                 SELECT 1 FROM holds
                 WHERE holds.expires_at > ?2
                 AND (holds.account = sessions.account OR holds.account IN (
                     SELECT account FROM identity_accounts
                     WHERE identity_accounts.identity_key = sessions.identity_key
                 ))
             )",
            params![cutoff(now, policy.session_days), now],
        )
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

/// The portal's audit entries past their window (a hold on the account an entry concerns keeps
/// it), its expired cookies, and operators nothing refers to any more.
fn purge_admin(
    conn: &Connection,
    now: i64,
    policy: RetentionPolicy,
    counts: &mut PurgeCounts,
) -> Result<(), StoreError> {
    let audit = format!(
        "DELETE FROM admin_audit WHERE recorded_at < ?1 AND NOT {}",
        held_sql("admin_audit.account")
    );
    counts.admin_audit = changed(
        conn.execute(&audit, params![cutoff(now, policy.admin_audit_days), now])
            .map_err(StoreError::backend)?,
    );
    counts.admin_sessions = changed(
        conn.execute(
            "DELETE FROM admin_sessions WHERE expires_at < ?1",
            [now.saturating_sub(dates::DAY)],
        )
        .map_err(StoreError::backend)?,
    );
    counts.operators = changed(
        conn.execute(
            "DELETE FROM operators
             WHERE NOT EXISTS (SELECT 1 FROM staff WHERE staff.identity_key = operators.identity_key)
             AND NOT EXISTS (SELECT 1 FROM admin_sessions
                             WHERE admin_sessions.identity_key = operators.identity_key)
             AND NOT EXISTS (SELECT 1 FROM admin_audit
                             WHERE admin_audit.identity_key = operators.identity_key)
             AND NOT EXISTS (SELECT 1 FROM posts WHERE posts.handled_by = operators.identity_key)",
            [],
        )
        .map_err(StoreError::backend)?,
    );
    Ok(())
}

/// An identity that owns no account and has no session row left is gone; sealed records that
/// named it keep their row with the link cleared (ON DELETE SET NULL). An hour's grace covers a
/// sign-in between creating the identity and its session.
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
            [now.saturating_sub(dates::HOUR)],
        )
        .map_err(StoreError::backend)?;
    Ok(changed(n))
}

/// `None` when the account number was never issued, `Some(None)` when it is live.
fn deleted_at(conn: &Connection, account: u32) -> Result<Option<Option<i64>>, StoreError> {
    conn.query_row(
        "SELECT deleted_at FROM accounts WHERE id = ?1",
        [account],
        |row| row.get(0),
    )
    .optional()
    .map_err(StoreError::backend)
}

fn purged(conn: &Connection, account: u32) -> Result<bool, StoreError> {
    let at: Option<Option<i64>> = conn
        .query_row(
            "SELECT purged_at FROM accounts WHERE id = ?1",
            [account],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::backend)?;
    Ok(at.flatten().is_some())
}

fn held(conn: &Connection, account: u32, now: i64) -> Result<bool, StoreError> {
    conn.query_row(
        &format!("SELECT {}", held_sql("?1")),
        params![account, now],
        |row| row.get(0),
    )
    .map_err(StoreError::backend)
}

fn changed(rows: usize) -> u64 {
    u64::try_from(rows).unwrap_or(0)
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;
