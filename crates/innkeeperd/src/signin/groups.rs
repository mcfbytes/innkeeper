//! Identity groups: the sign-ins one person has linked, which own their accounts together
//! (docs/server/innkeeperd.md section 8.7).

use innkeeper_world::StoreError;
use rusqlite::{params, Connection, OptionalExtension};

use super::keys::IdentityKey;
use super::store::{owns_suspended_in, GROUP_OF_KEY};
use crate::sqlite_store::SqliteStore;

/// The most sign-ins one group may hold, so a listing and a link stay small.
pub(crate) const MAX_GROUP_IDENTITIES: usize = 8;
/// Hex digits of the identity key the page names a sign-in method by: unique within a group.
const METHOD_ID_DIGITS: usize = 16;

/// One sign-in of a group, as the page lists it: never the subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Method {
    pub(crate) key: IdentityKey,
    pub(crate) provider: String,
    pub(crate) linked_at: i64,
}

impl Method {
    pub(crate) fn id(&self) -> String {
        method_id(&self.key)
    }
}

pub(crate) fn method_id(key: &IdentityKey) -> String {
    let mut text = crate::admin::allowlist::hex(key);
    text.truncate(METHOD_ID_DIGITS);
    text
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Linked {
    /// The group without accounts joined the other.
    Joined,
    /// Both were in one group already.
    Already,
    /// Both groups own accounts: two people's accounts are never merged.
    InUse,
    TooMany,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unlinked {
    Done,
    /// The group's only sign-in.
    Last,
    /// The sign-in of the session asking.
    Current,
    /// A suspended account of the group keeps every sign-in it has.
    Suspended,
    Missing,
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::backend(error)
}

fn group_of(conn: &Connection, key: &IdentityKey) -> Result<Option<i64>, StoreError> {
    conn.query_row(
        "SELECT identity_group FROM identities WHERE identity_key = ?1",
        [key.as_slice()],
        |row| row.get::<_, Option<i64>>(0),
    )
    .optional()
    .map_err(backend)
    .map(Option::flatten)
}

fn owns_any(conn: &Connection, group: i64) -> Result<bool, StoreError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM group_accounts WHERE identity_group = ?1 LIMIT 1",
            [group],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(found.is_some())
}

fn members(conn: &Connection, group: i64) -> Result<Vec<Method>, StoreError> {
    let mut statement = conn
        .prepare_cached(
            "SELECT identity_key, provider, ifnull(linked_at, created_at) FROM identities
             WHERE identity_group = ?1 ORDER BY 3, identity_key",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map([group], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(backend)?;
    let mut out = Vec::new();
    for row in rows {
        let (key, provider, linked_at) = row.map_err(backend)?;
        if let Ok(key) = IdentityKey::try_from(key.as_slice()) {
            out.push(Method {
                key,
                provider,
                linked_at,
            });
        }
    }
    Ok(out)
}

/// Every sign-in in the identity's group, oldest link first; the identity itself included.
pub(crate) fn methods(store: &SqliteStore, key: &IdentityKey) -> Result<Vec<Method>, StoreError> {
    let conn = store.connection();
    match group_of(&conn, key)? {
        Some(group) => members(&conn, group),
        None => Ok(Vec::new()),
    }
}

/// What `link` would do, without doing it: the callback asks before it hands the page a proof.
pub(crate) fn link_check(
    store: &SqliteStore,
    session: &IdentityKey,
    proven: &IdentityKey,
) -> Result<Linked, StoreError> {
    Ok(plan(&store.connection(), session, proven)?
        .map_or_else(|outcome| outcome, |_| Linked::Joined))
}

/// Puts the two identities in one group. The group without accounts joins the one with them (the
/// session's when neither has any), so both see the same accounts from now on.
pub(crate) fn link(
    store: &SqliteStore,
    session: &IdentityKey,
    proven: &IdentityKey,
    now: i64,
) -> Result<Linked, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(backend)?;
    let (anchor, joining) = match plan(&tx, session, proven)? {
        Ok(groups) => groups,
        Err(outcome) => return Ok(outcome),
    };
    tx.execute(
        "UPDATE identities SET identity_group = ?1, linked_at = ?3, deleted_at = NULL
         WHERE identity_group = ?2",
        params![anchor, joining, now],
    )
    .map_err(backend)?;
    tx.execute("DELETE FROM identity_groups WHERE id = ?1", [joining])
        .map_err(backend)?;
    tx.commit().map_err(backend)?;
    Ok(Linked::Joined)
}

/// The anchor and the joining group, or why the two cannot be linked.
fn plan(
    conn: &Connection,
    session: &IdentityKey,
    proven: &IdentityKey,
) -> Result<Result<(i64, i64), Linked>, StoreError> {
    let (Some(ours), Some(theirs)) = (group_of(conn, session)?, group_of(conn, proven)?) else {
        return Ok(Err(Linked::Missing));
    };
    if ours == theirs {
        return Ok(Err(Linked::Already));
    }
    let groups = match (owns_any(conn, ours)?, owns_any(conn, theirs)?) {
        (true, true) => return Ok(Err(Linked::InUse)),
        (false, true) => (theirs, ours),
        (true, false) | (false, false) => (ours, theirs),
    };
    if members(conn, ours)?.len() + members(conn, theirs)?.len() > MAX_GROUP_IDENTITIES {
        return Ok(Err(Linked::TooMany));
    }
    Ok(Ok(groups))
}

/// Takes the sign-in named `id` out of the session identity's group into a group of its own, with
/// no accounts, and ends its sessions. It then ages out like any identity that owns nothing.
pub(crate) fn unlink(
    store: &SqliteStore,
    session: &IdentityKey,
    id: &str,
    now: i64,
) -> Result<Unlinked, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(backend)?;
    let Some(group) = group_of(&tx, session)? else {
        return Ok(Unlinked::Missing);
    };
    let all = members(&tx, group)?;
    let Some(target) = all.iter().find(|method| method.id() == id) else {
        return Ok(Unlinked::Missing);
    };
    if all.len() == 1 {
        return Ok(Unlinked::Last);
    }
    if target.key == *session {
        return Ok(Unlinked::Current);
    }
    if owns_suspended_in(&tx, session)? {
        return Ok(Unlinked::Suspended);
    }
    tx.execute(
        "INSERT INTO identity_groups (created_at) VALUES (?1)",
        [now],
    )
    .map_err(backend)?;
    let alone = tx.last_insert_rowid();
    tx.execute(
        "UPDATE identities SET identity_group = ?2, linked_at = ?3 WHERE identity_key = ?1",
        params![target.key.as_slice(), alone, now],
    )
    .map_err(backend)?;
    tx.execute(
        "UPDATE sessions SET expires_at = min(expires_at, ?2) WHERE identity_key = ?1",
        params![target.key.as_slice(), now],
    )
    .map_err(backend)?;
    tx.commit().map_err(backend)?;
    Ok(Unlinked::Done)
}

/// The identity keys of the group, for a list that names identities (the invite list).
pub(crate) fn group_keys(
    store: &SqliteStore,
    key: &IdentityKey,
) -> Result<Vec<IdentityKey>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare_cached(&format!(
            "SELECT identity_key FROM identities WHERE identity_group = {GROUP_OF_KEY}"
        ))
        .map_err(backend)?;
    let rows = statement
        .query_map([key.as_slice()], |row| row.get::<_, Vec<u8>>(0))
        .map_err(backend)?;
    let mut keys = Vec::new();
    for row in rows {
        if let Ok(key) = IdentityKey::try_from(row.map_err(backend)?.as_slice()) {
            keys.push(key);
        }
    }
    Ok(keys)
}

#[cfg(test)]
#[path = "groups_tests.rs"]
mod tests;
