//! What the portal reads and writes in the store: the staff table, its sessions, the audit log,
//! complaint marks and the listings the pages show. Changes to accounts, money, posts, letters and
//! holds go through the `Store` trait and the existing retention functions where they exist; the
//! SQL here is for what the trait has no words for (docs/server/store.md).

use innkeeper_world::{Envelope, HostTime, StoreError, COMPLAINT_BOARD_LOCATION};
use rusqlite::{params, OptionalExtension};

use crate::signin::keys::IdentityKey;
use crate::sqlite_store::SqliteStore;
use crate::subject::{c_string, post_author, trim_nul};

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::backend(error)
}

/// Who did something: the staff handle and key, for the audit log.
#[derive(Clone, Debug)]
pub(crate) struct Who {
    pub(crate) handle: String,
    pub(crate) key: IdentityKey,
}

// ---- staff ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StaffRow {
    pub(crate) note: Option<String>,
    pub(crate) added_at: i64,
    pub(crate) revoked_at: Option<i64>,
}

pub(crate) fn staff_row(
    store: &SqliteStore,
    key: &IdentityKey,
) -> Result<Option<StaffRow>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT note, added_at, revoked_at FROM staff WHERE identity_key = ?1",
            [key.as_slice()],
            |row| {
                Ok(StaffRow {
                    note: row.get(0)?,
                    added_at: row.get(1)?,
                    revoked_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(backend)
}

/// Rows of any state: a revoked row still counts as a configured source.
pub(crate) fn staff_count(store: &SqliteStore) -> Result<i64, StoreError> {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM staff", [], |row| row.get(0))
        .map_err(backend)
}

/// Adds the identity, or lifts its revocation.
pub(crate) fn staff_allow(
    store: &SqliteStore,
    key: &IdentityKey,
    note: Option<&str>,
    now: i64,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "INSERT INTO staff (identity_key, note, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(identity_key) DO UPDATE SET revoked_at = NULL,
                 note = coalesce(excluded.note, staff.note)",
            params![key.as_slice(), note, now],
        )
        .map_err(backend)?;
    Ok(())
}

/// Marks the identity revoked, whether or not the table listed it before. Revocation beats the
/// vault and the file.
pub(crate) fn staff_revoke(
    store: &SqliteStore,
    key: &IdentityKey,
    now: i64,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "INSERT INTO staff (identity_key, note, added_at, revoked_at) VALUES (?1, NULL, ?2, ?2)
             ON CONFLICT(identity_key) DO UPDATE SET revoked_at = excluded.revoked_at",
            params![key.as_slice(), now],
        )
        .map_err(backend)?;
    Ok(())
}

pub(crate) fn staff_list(store: &SqliteStore) -> Result<Vec<(IdentityKey, StaffRow)>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare("SELECT identity_key, note, added_at, revoked_at FROM staff ORDER BY added_at")
        .map_err(backend)?;
    let rows = statement
        .query_map([], |row| {
            let key: Vec<u8> = row.get(0)?;
            Ok((
                key,
                StaffRow {
                    note: row.get(1)?,
                    added_at: row.get(2)?,
                    revoked_at: row.get(3)?,
                },
            ))
        })
        .map_err(backend)?;
    let mut out = Vec::new();
    for row in rows {
        let (key, row) = row.map_err(backend)?;
        if let Ok(key) = IdentityKey::try_from(key.as_slice()) {
            out.push((key, row));
        }
    }
    Ok(out)
}

// ---- the portal's own sessions ----

pub(crate) fn create_session(
    store: &SqliteStore,
    token_hash: &[u8; 32],
    key: &IdentityKey,
    now: i64,
    expiry: i64,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "INSERT INTO admin_sessions (token_hash, identity_key, created_at, last_seen, expiry)
             VALUES (?1, ?2, ?3, ?3, ?4)",
            params![token_hash.as_slice(), key.as_slice(), now, expiry],
        )
        .map_err(backend)?;
    Ok(())
}

/// The identity behind a live cookie: not expired, seen within `idle` seconds. A hit renews
/// `last_seen`.
pub(crate) fn live_session(
    store: &SqliteStore,
    token_hash: &[u8; 32],
    now: i64,
    idle: i64,
) -> Result<Option<IdentityKey>, StoreError> {
    let conn = store.connection();
    let key: Option<Vec<u8>> = conn
        .query_row(
            "SELECT identity_key FROM admin_sessions
             WHERE token_hash = ?1 AND expiry > ?2 AND last_seen > ?3",
            params![token_hash.as_slice(), now, now.saturating_sub(idle)],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    let Some(key) = key.and_then(|bytes| IdentityKey::try_from(bytes.as_slice()).ok()) else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE admin_sessions SET last_seen = ?2 WHERE token_hash = ?1",
        params![token_hash.as_slice(), now],
    )
    .map_err(backend)?;
    Ok(Some(key))
}

pub(crate) fn delete_session(store: &SqliteStore, token_hash: &[u8; 32]) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "DELETE FROM admin_sessions WHERE token_hash = ?1",
            [token_hash.as_slice()],
        )
        .map_err(backend)?;
    Ok(())
}

// ---- the audit log ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuditEntry {
    pub(crate) id: i64,
    pub(crate) at: i64,
    pub(crate) handle: String,
    pub(crate) key_hex: String,
    pub(crate) action: String,
    pub(crate) target: String,
    pub(crate) reason: Option<String>,
}

/// Records one change or one letter read. `account` is the account the entry concerns, so a hold on
/// it keeps the entry past its window.
pub(crate) fn record(
    store: &SqliteStore,
    who: &Who,
    action: &str,
    target: &str,
    reason: Option<&str>,
    account: Option<u32>,
    now: i64,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "INSERT INTO admin_audit (at, handle, identity_key, action, target, reason, account)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                now,
                who.handle,
                who.key.as_slice(),
                action,
                target,
                reason,
                account
            ],
        )
        .map_err(backend)?;
    Ok(())
}

pub(crate) fn recent_audit(
    store: &SqliteStore,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<AuditEntry>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT id, at, handle, identity_key, action, target, reason FROM admin_audit
             WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map(params![before.unwrap_or(i64::MAX), limit], |row| {
            let key: Vec<u8> = row.get(3)?;
            Ok(AuditEntry {
                id: row.get(0)?,
                at: row.get(1)?,
                handle: row.get(2)?,
                key_hex: key.iter().map(|byte| format!("{byte:02x}")).collect(),
                action: row.get(4)?,
                target: row.get(5)?,
                reason: row.get(6)?,
            })
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

// ---- the dashboard ----

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) accounts: i64,
    pub(crate) letters: i64,
    pub(crate) posts: i64,
    pub(crate) complaints_waiting: i64,
    pub(crate) holds: i64,
}

fn complaint_filter() -> String {
    format!("((posts.board >> 8) & 65535) = {COMPLAINT_BOARD_LOCATION}")
}

pub(crate) fn counts(store: &SqliteStore, now: i64) -> Result<Counts, StoreError> {
    let conn = store.connection();
    let one = |sql: &str, args: &[&dyn rusqlite::ToSql]| -> Result<i64, StoreError> {
        conn.query_row(sql, args, |row| row.get(0)).map_err(backend)
    };
    let complaints = complaint_filter();
    Ok(Counts {
        accounts: one(
            "SELECT COUNT(*) FROM accounts WHERE deleted_at IS NULL",
            &[],
        )?,
        letters: one("SELECT COUNT(*) FROM letters WHERE deleted_at IS NULL", &[])?,
        posts: one(
            &format!("SELECT COUNT(*) FROM posts WHERE deleted_at IS NULL AND NOT ({complaints})"),
            &[],
        )?,
        complaints_waiting: one(
            &format!(
                "SELECT COUNT(*) FROM posts WHERE deleted_at IS NULL AND ({complaints})
                 AND id NOT IN (SELECT post_id FROM complaint_handled)"
            ),
            &[],
        )?,
        holds: one("SELECT COUNT(*) FROM holds WHERE hold_until > ?1", &[&now])?,
    })
}

// ---- accounts ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountSummary {
    pub(crate) id: u32,
    pub(crate) persona: String,
    pub(crate) deleted_at: Option<i64>,
    pub(crate) suspended_at: Option<i64>,
    pub(crate) user_flags: u16,
}

fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountSummary> {
    Ok(AccountSummary {
        id: row.get(0)?,
        persona: row.get(1)?,
        deleted_at: row.get(2)?,
        suspended_at: row.get(3)?,
        user_flags: row.get(4)?,
    })
}

/// Accounts whose number is `query`, or whose persona contains it (without regard to case). An
/// empty query lists the newest.
pub(crate) fn search_accounts(
    store: &SqliteStore,
    query: &str,
) -> Result<Vec<AccountSummary>, StoreError> {
    let query = query.trim();
    let conn = store.connection();
    let columns = "SELECT id, persona, deleted_at, suspended_at, user_flags FROM accounts";
    let mut statement;
    let rows = if query.is_empty() {
        statement = conn
            .prepare(&format!("{columns} ORDER BY id DESC LIMIT 50"))
            .map_err(backend)?;
        statement.query_map([], summary).map_err(backend)?
    } else {
        let number: i64 = query.parse().unwrap_or(-1);
        let like = format!("%{}%", escape_like(query));
        statement = conn
            .prepare(&format!(
                "{columns} WHERE id = ?1 OR persona LIKE ?2 ESCAPE '\\' ORDER BY id LIMIT 100"
            ))
            .map_err(backend)?;
        statement
            .query_map(params![number, like], summary)
            .map_err(backend)?
    };
    rows.collect::<Result<_, _>>().map_err(backend)
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// The account behind a post or complaint: the one recorded on it, or, for a post from before
/// posts recorded one, the first account whose persona matches the author name (see privacy.md).
pub(crate) fn account_of_post(
    store: &SqliteStore,
    account: Option<u32>,
    author: &str,
) -> Option<u32> {
    account.or_else(|| {
        (!author.is_empty())
            .then(|| account_by_persona(store, author).ok().flatten())
            .flatten()
    })
}

pub(crate) fn account_by_persona(
    store: &SqliteStore,
    persona: &str,
) -> Result<Option<u32>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT id FROM accounts WHERE persona = ?1 COLLATE NOCASE ORDER BY deleted_at IS NOT NULL, id",
            [persona],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkedIdentity {
    pub(crate) key: IdentityKey,
    pub(crate) provider: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WebSession {
    pub(crate) created_at: i64,
    pub(crate) expiry: i64,
    pub(crate) account: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountDetail {
    pub(crate) summary: AccountSummary,
    pub(crate) rating: u16,
    pub(crate) status: u8,
    pub(crate) password_expired: bool,
    pub(crate) mailbox: Option<u32>,
    pub(crate) created_at: Option<i64>,
    pub(crate) last_logon: Option<i64>,
    pub(crate) identities: Vec<LinkedIdentity>,
    pub(crate) sessions: Vec<WebSession>,
    pub(crate) hold_until: Option<i64>,
    pub(crate) save_files: i64,
}

pub(crate) fn account_detail(
    store: &SqliteStore,
    id: u32,
) -> Result<Option<AccountDetail>, StoreError> {
    let conn = store.connection();
    let found = conn
        .query_row(
            "SELECT id, persona, deleted_at, suspended_at, user_flags, rating, status, password_expired
             FROM accounts WHERE id = ?1",
            [id],
            |row| Ok((summary(row)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        )
        .optional()
        .map_err(backend)?;
    let Some((summary, rating, status, password_expired)) = found else {
        return Ok(None);
    };
    let scalar = |sql: &str| -> Result<Option<i64>, StoreError> {
        conn.query_row(sql, [id], |row| row.get(0))
            .optional()
            .map(Option::flatten)
            .map_err(backend)
    };
    let mailbox = conn
        .query_row(
            "SELECT number FROM mailboxes WHERE account = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    let created_at = scalar("SELECT created_at FROM identity_accounts WHERE account_id = ?1")?;
    let last_logon = scalar("SELECT MAX(started_at) FROM audit_sessions WHERE account_id = ?1")?;
    let hold_until = scalar("SELECT hold_until FROM holds WHERE account = ?1")?;
    let save_files =
        scalar("SELECT COUNT(DISTINCT path) FROM saves WHERE account = ?1 AND deleted_at IS NULL")?
            .unwrap_or(0);
    let identities = {
        let mut statement = conn
            .prepare(
                "SELECT identities.identity_key, identities.provider FROM identity_accounts
                 JOIN identities USING (identity_key) WHERE identity_accounts.account_id = ?1",
            )
            .map_err(backend)?;
        let rows = statement
            .query_map([id], |row| {
                let key: Vec<u8> = row.get(0)?;
                Ok((key, row.get::<_, String>(1)?))
            })
            .map_err(backend)?;
        let mut out = Vec::new();
        for row in rows {
            let (key, provider) = row.map_err(backend)?;
            if let Ok(key) = IdentityKey::try_from(key.as_slice()) {
                out.push(LinkedIdentity { key, provider });
            }
        }
        out
    };
    let sessions = {
        let mut statement = conn
            .prepare(
                "SELECT created_at, expiry, account_id FROM sessions
                 WHERE identity_key IN (SELECT identity_key FROM identity_accounts WHERE account_id = ?1)
                 ORDER BY created_at DESC LIMIT 20",
            )
            .map_err(backend)?;
        let rows = statement
            .query_map([id], |row| {
                Ok(WebSession {
                    created_at: row.get(0)?,
                    expiry: row.get(1)?,
                    account: row.get(2)?,
                })
            })
            .map_err(backend)?;
        rows.collect::<Result<_, _>>().map_err(backend)?
    };
    Ok(Some(AccountDetail {
        summary,
        rating,
        status,
        password_expired,
        mailbox,
        created_at,
        last_logon,
        identities,
        sessions,
        hold_until,
        save_files,
    }))
}

/// Suspension is a stamp on the account row; the Login path reads it through `Store::suspended`.
pub(crate) fn set_suspended(
    store: &SqliteStore,
    id: u32,
    suspended: bool,
    now: i64,
) -> Result<bool, StoreError> {
    let stamp = suspended.then_some(now);
    let changed = store
        .connection()
        .execute(
            "UPDATE accounts SET suspended_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, stamp],
        )
        .map_err(backend)?;
    Ok(changed > 0)
}

pub(crate) fn set_flags(
    store: &SqliteStore,
    id: u32,
    user_flags: u16,
    rating: u16,
) -> Result<bool, StoreError> {
    let changed = store
        .connection()
        .execute(
            "UPDATE accounts SET user_flags = ?2, rating = ?3 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, user_flags, rating],
        )
        .map_err(backend)?;
    Ok(changed > 0)
}

/// Ends every web sign-in token of the identities that own the account. Returns how many were live.
pub(crate) fn revoke_web_sessions(
    store: &SqliteStore,
    id: u32,
    now: i64,
) -> Result<usize, StoreError> {
    store
        .connection()
        .execute(
            "UPDATE sessions SET expiry = ?2
             WHERE expiry > ?2
             AND identity_key IN (SELECT identity_key FROM identity_accounts WHERE account_id = ?1)",
            params![id, now],
        )
        .map_err(backend)
}

// ---- complaints ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Complaint {
    pub(crate) post_id: u32,
    pub(crate) section: u32,
    pub(crate) author: String,
    pub(crate) text: String,
    pub(crate) created_at: i64,
    pub(crate) handled: Option<(i64, String)>,
    /// The account that wrote it; none for a complaint from before posts recorded one.
    pub(crate) account: Option<u32>,
}

impl Complaint {
    /// Section 2 of the complaint board is Member Services, section 3 the hub's complaints.
    pub(crate) fn kind(&self) -> &'static str {
        match self.section {
            2 => "Member Services",
            _ => "Complaint",
        }
    }

    /// The persona named after `about:` in the client's complaint text, when there is one.
    pub(crate) fn offender(&self) -> Option<String> {
        let after = self.text.split("about: ").nth(1)?;
        let name = after.split([',', ';']).next()?.trim();
        let valid = !name.is_empty()
            && name.len() <= 10
            && name.bytes().all(|byte| byte.is_ascii_alphanumeric());
        valid.then(|| name.to_owned())
    }
}

fn complaint_from(
    id: u32,
    board: u32,
    body: &[u8],
    created_at: Option<i64>,
    handled: (Option<i64>, Option<String>),
    account: Option<u32>,
) -> Complaint {
    let text = String::from_utf8_lossy(trim_nul(body)).into_owned();
    let (author, rest) = match text.split_once('|') {
        Some((author, rest)) => (author.trim().to_owned(), rest.trim().to_owned()),
        None => (String::new(), text.clone()),
    };
    let rest = rest
        .strip_prefix("Complaint:")
        .or_else(|| rest.strip_prefix("Member Services:"))
        .map_or(rest.clone(), |body| body.trim().to_owned());
    Complaint {
        post_id: id,
        section: board & 0xFF,
        author,
        text: rest,
        created_at: created_at.unwrap_or(0),
        handled: handled.0.zip(handled.1),
        account,
    }
}

const COMPLAINT_COLUMNS: &str = "SELECT posts.id, posts.board, posts.body, posts.created_at,
    complaint_handled.handled_at, complaint_handled.handle, posts.account
    FROM posts LEFT JOIN complaint_handled ON complaint_handled.post_id = posts.id";

pub(crate) fn complaints(store: &SqliteStore, limit: u32) -> Result<Vec<Complaint>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(&format!(
            "{COMPLAINT_COLUMNS} WHERE posts.deleted_at IS NULL AND ({})
             ORDER BY posts.id DESC LIMIT ?1",
            complaint_filter()
        ))
        .map_err(backend)?;
    let rows = statement
        .query_map([limit], |row| {
            let body: Vec<u8> = row.get(2)?;
            Ok(complaint_from(
                row.get(0)?,
                row.get(1)?,
                &body,
                row.get(3)?,
                (row.get(4)?, row.get(5)?),
                row.get(6)?,
            ))
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

pub(crate) fn complaint(
    store: &SqliteStore,
    post_id: u32,
) -> Result<Option<Complaint>, StoreError> {
    let conn = store.connection();
    conn.query_row(
        &format!(
            "{COMPLAINT_COLUMNS} WHERE posts.id = ?1 AND posts.deleted_at IS NULL AND ({})",
            complaint_filter()
        ),
        [post_id],
        |row| {
            let body: Vec<u8> = row.get(2)?;
            Ok(complaint_from(
                row.get(0)?,
                row.get(1)?,
                &body,
                row.get(3)?,
                (row.get(4)?, row.get(5)?),
                row.get(6)?,
            ))
        },
    )
    .optional()
    .map_err(backend)
}

pub(crate) fn set_handled(
    store: &SqliteStore,
    post_id: u32,
    handled: bool,
    handle: &str,
    now: i64,
) -> Result<(), StoreError> {
    let conn = store.connection();
    if handled {
        conn.execute(
            "INSERT OR REPLACE INTO complaint_handled (post_id, handled_at, handle) VALUES (?1, ?2, ?3)",
            params![post_id, now, handle],
        )
        .map_err(backend)?;
    } else {
        conn.execute(
            "DELETE FROM complaint_handled WHERE post_id = ?1",
            [post_id],
        )
        .map_err(backend)?;
    }
    Ok(())
}

// ---- boards and posts ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoardRow {
    pub(crate) board: u32,
    pub(crate) posts: i64,
    pub(crate) newest: i64,
}

impl BoardRow {
    pub(crate) fn label(&self) -> String {
        board_label(self.board)
    }
}

pub(crate) fn board_label(board: u32) -> String {
    format!(
        "land {}, board {}, section {}",
        board >> 24,
        (board >> 8) & 0xFFFF,
        board & 0xFF
    )
}

pub(crate) fn boards(store: &SqliteStore) -> Result<Vec<BoardRow>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(&format!(
            "SELECT board, COUNT(*), MAX(ifnull(created_at, 0)) FROM posts
             WHERE deleted_at IS NULL AND NOT ({}) GROUP BY board ORDER BY board",
            complaint_filter()
        ))
        .map_err(backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok(BoardRow {
                board: row.get(0)?,
                posts: row.get(1)?,
                newest: row.get(2)?,
            })
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PostView {
    pub(crate) id: u32,
    pub(crate) author: Option<String>,
    pub(crate) text: String,
    pub(crate) created_at: i64,
    pub(crate) account: Option<u32>,
}

pub(crate) fn board_posts(
    store: &SqliteStore,
    board: u32,
    limit: u32,
) -> Result<Vec<PostView>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT id, body, ifnull(created_at, 0), account FROM posts
             WHERE board = ?1 AND deleted_at IS NULL ORDER BY id DESC LIMIT ?2",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map(params![board, limit], |row| {
            let body: Vec<u8> = row.get(1)?;
            Ok(post_view(row.get(0)?, &body, row.get(2)?, row.get(3)?))
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

fn post_view(id: u32, body: &[u8], created_at: i64, account: Option<u32>) -> PostView {
    let author = post_author(body).map(str::to_owned);
    let text = String::from_utf8_lossy(trim_nul(body)).into_owned();
    let text = match text.split_once('|') {
        Some((_, rest)) => rest.to_owned(),
        None => text,
    };
    PostView {
        id,
        author,
        text,
        created_at,
        account,
    }
}

/// A live post's board, so a removal names the board it came from.
pub(crate) fn post_board(
    store: &SqliteStore,
    post: u32,
) -> Result<Option<(u32, PostView)>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT board, body, ifnull(created_at, 0), account FROM posts WHERE id = ?1 AND deleted_at IS NULL",
            [post],
            |row| {
                let body: Vec<u8> = row.get(1)?;
                Ok((row.get(0)?, post_view(post, &body, row.get(2)?, row.get(3)?)))
            },
        )
        .optional()
        .map_err(backend)
}

// ---- letters ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MailboxRow {
    pub(crate) number: u32,
    pub(crate) account: u32,
    pub(crate) persona: String,
    pub(crate) letters: i64,
}

pub(crate) fn mailboxes(store: &SqliteStore) -> Result<Vec<MailboxRow>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT mailboxes.number, mailboxes.account, accounts.persona, COUNT(letters.id)
             FROM mailboxes JOIN accounts ON accounts.id = mailboxes.account
             LEFT JOIN letters ON letters.mailbox = mailboxes.number AND letters.deleted_at IS NULL
             WHERE mailboxes.deleted_at IS NULL
             GROUP BY mailboxes.number ORDER BY mailboxes.number",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok(MailboxRow {
                number: row.get(0)?,
                account: row.get(1)?,
                persona: row.get(2)?,
                letters: row.get(3)?,
            })
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

pub(crate) fn mailbox_owner(
    store: &SqliteStore,
    mailbox: u32,
) -> Result<Option<(u32, String)>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT mailboxes.account, accounts.persona FROM mailboxes
             JOIN accounts ON accounts.id = mailboxes.account WHERE mailboxes.number = ?1",
            [mailbox],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(backend)
}

pub(crate) fn mailbox_of_account(
    store: &SqliteStore,
    account: u32,
) -> Result<Option<u32>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT number FROM mailboxes WHERE account = ?1",
            [account],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)
}

/// A letter's envelope fields, with no text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LetterMeta {
    pub(crate) id: u32,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) received: Option<String>,
    pub(crate) size: usize,
}

/// A letter with its words; only the reasoned read path builds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LetterBody {
    pub(crate) meta: LetterMeta,
    pub(crate) subject: String,
    pub(crate) text: String,
}

pub(crate) fn format_time(time: HostTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        u16::from(time.year_1900) + 1900,
        u16::from(time.month0) + 1,
        time.mday,
        time.hour,
        time.minute
    )
}

fn letter_meta(id: u32, body: &[u8]) -> LetterMeta {
    let parsed = Envelope::from_stored_body(body);
    LetterMeta {
        id,
        from: parsed
            .as_ref()
            .map_or_else(String::new, |(_, envelope)| c_string(&envelope.sender)),
        to: parsed
            .as_ref()
            .map_or_else(String::new, |(_, envelope)| c_string(&envelope.addressee)),
        received: parsed.map(|(time, _)| format_time(time)),
        size: body.len(),
    }
}

pub(crate) fn letters_in(
    store: &SqliteStore,
    mailbox: u32,
    limit: u32,
) -> Result<Vec<LetterMeta>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT id, body FROM letters WHERE mailbox = ?1 AND deleted_at IS NULL
             ORDER BY id DESC LIMIT ?2",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map(params![mailbox, limit], |row| {
            let body: Vec<u8> = row.get(1)?;
            Ok(letter_meta(row.get(0)?, &body))
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

/// One letter's metadata, with no text.
pub(crate) fn letter_meta_of(
    store: &SqliteStore,
    mailbox: u32,
    id: u32,
) -> Result<Option<LetterMeta>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT body FROM letters WHERE id = ?1 AND mailbox = ?2 AND deleted_at IS NULL",
            params![id, mailbox],
            |row| {
                let body: Vec<u8> = row.get(0)?;
                Ok(letter_meta(id, &body))
            },
        )
        .optional()
        .map_err(backend)
}

/// The letter with its subject and text. Callers must have recorded the reason first.
pub(crate) fn read_letter(
    store: &SqliteStore,
    mailbox: u32,
    id: u32,
) -> Result<Option<LetterBody>, StoreError> {
    let body: Option<Vec<u8>> = store
        .connection()
        .query_row(
            "SELECT body FROM letters WHERE id = ?1 AND mailbox = ?2 AND deleted_at IS NULL",
            params![id, mailbox],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    let Some(body) = body else {
        return Ok(None);
    };
    let meta = letter_meta(id, &body);
    let (subject, text) = match Envelope::from_stored_body(&body) {
        Some((_, envelope)) => (c_string(&envelope.subject), c_string(&envelope.text)),
        None => (String::new(), String::new()),
    };
    Ok(Some(LetterBody {
        meta,
        subject,
        text,
    }))
}

// ---- legal holds ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HoldRow {
    pub(crate) account: u32,
    pub(crate) persona: Option<String>,
    pub(crate) until: i64,
}

pub(crate) fn holds(store: &SqliteStore) -> Result<Vec<HoldRow>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT holds.account, accounts.persona, holds.hold_until FROM holds
             LEFT JOIN accounts ON accounts.id = holds.account ORDER BY holds.hold_until DESC",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok(HoldRow {
                account: row.get(0)?,
                persona: row.get(1)?,
                until: row.get(2)?,
            })
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

/// What a hold on the account keeps, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub(crate) sealed_records: i64,
    pub(crate) letters: i64,
    pub(crate) save_versions: i64,
    pub(crate) sessions: i64,
}

pub(crate) fn coverage(store: &SqliteStore, account: u32) -> Result<Coverage, StoreError> {
    let conn = store.connection();
    let one = |sql: &str| -> Result<i64, StoreError> {
        conn.query_row(sql, [account], |row| row.get(0))
            .map_err(backend)
    };
    Ok(Coverage {
        sealed_records: one("SELECT COUNT(*) FROM audit_sessions WHERE account_id = ?1")?,
        letters: one(
            "SELECT COUNT(*) FROM letters WHERE mailbox = (SELECT number FROM mailboxes WHERE account = ?1)",
        )?,
        save_versions: one("SELECT COUNT(*) FROM saves WHERE account = ?1")?,
        sessions: one("SELECT COUNT(*) FROM sessions WHERE account_id = ?1")?,
    })
}

// ---- saves ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SaveOwner {
    pub(crate) account: u32,
    pub(crate) persona: Option<String>,
    pub(crate) files: i64,
}

pub(crate) fn save_owners(store: &SqliteStore) -> Result<Vec<SaveOwner>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT saves.account, accounts.persona, COUNT(DISTINCT saves.path) FROM saves
             LEFT JOIN accounts ON accounts.id = saves.account
             WHERE saves.deleted_at IS NULL GROUP BY saves.account ORDER BY saves.account",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok(SaveOwner {
                account: row.get(0)?,
                persona: row.get(1)?,
                files: row.get(2)?,
            })
        })
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offender_is_the_name_after_about() {
        let make = |text: &str| Complaint {
            post_id: 1,
            section: 3,
            author: "a".into(),
            text: text.into(),
            created_at: 0,
            handled: None,
            account: None,
        };
        let named = make("From: Anna in: Tavern; about: Bruno, who said \"x\" Comments: rude");
        assert_eq!(named.offender().as_deref(), Some("Bruno"));
        assert_eq!(make("nobody is named").offender(), None);
        assert_eq!(make("about: <script>, x").offender(), None);
    }

    #[test]
    fn like_wildcards_in_a_search_are_literal() {
        assert_eq!(escape_like("50%_off\\"), "50\\%\\_off\\\\");
    }
}
