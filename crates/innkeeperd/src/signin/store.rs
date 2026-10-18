//! The sign-in tables: identities, the groups that own accounts and session tokens
//! (docs/server/privacy.md section 1). Linking identities into one group is `super::groups`.

use innkeeper_world::{AccountId, AccountRecord, StoreError};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::keys::{self, IdentityKey, KeyRing};
use crate::sqlite_store::SqliteStore;

/// Where signup starts numbering (`SIGNUP_FIRST_ACCOUNT_ASSUMED` in innkeeper-world).
const FIRST_WEB_ACCOUNT: u32 = 100_001;

/// SQL for the identity group of the identity key bound to `?1`.
pub(super) const GROUP_OF_KEY: &str =
    "(SELECT identity_group FROM identities WHERE identity_key = ?1)";

/// The age and terms answer kept on an identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attested {
    pub(crate) age: u32,
    pub(crate) terms: u32,
    /// When the answer was given, in Unix milliseconds.
    pub(crate) at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentityRow {
    pub(crate) key: IdentityKey,
    pub(crate) provider: String,
    pub(crate) attested: Option<Attested>,
}

/// What a sign-in found: the identity row's key, and whether the row was new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SignedIn {
    pub(crate) key: IdentityKey,
    pub(crate) created: bool,
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::backend(error)
}

/// Finds the identity under any enabled key version and moves an old version's row to the
/// current key (which every linked row follows). Creates the row, in a group of its own, on first
/// sign-in.
pub(crate) fn sign_in(
    store: &SqliteStore,
    ring: &KeyRing,
    issuer: &str,
    subject: &str,
    provider: &str,
    sealed: impl FnOnce() -> Option<Vec<u8>>,
    now: i64,
) -> Result<SignedIn, StoreError> {
    let candidates = ring.candidates(issuer, subject);
    let current = ring.current();
    let current_key = current.identity_key(issuer, subject);
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(backend)?;
    for (version, key) in &candidates {
        if !identity_exists(&tx, key)? {
            continue;
        }
        if *key != current_key {
            rekey(&tx, key, &current_key, current.version())?;
        } else if *version != current.version() {
            tx.execute(
                "UPDATE identities SET key_version = ?2 WHERE identity_key = ?1",
                params![key.as_slice(), current.version()],
            )
            .map_err(backend)?;
        }
        tx.execute(
            "UPDATE identities SET deleted_at = NULL WHERE identity_key = ?1",
            [current_key.as_slice()],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        return Ok(SignedIn {
            key: current_key,
            created: false,
        });
    }
    tx.execute(
        "INSERT INTO identity_groups (created_at) VALUES (?1)",
        [now],
    )
    .map_err(backend)?;
    let group = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO identities
         (identity_key, key_version, subject_sealed, provider, created_at, identity_group, linked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5)",
        params![
            current_key.as_slice(),
            current.version(),
            sealed(),
            provider,
            now,
            group
        ],
    )
    .map_err(backend)?;
    tx.commit().map_err(backend)?;
    Ok(SignedIn {
        key: current_key,
        created: true,
    })
}

fn identity_exists(conn: &Connection, key: &IdentityKey) -> Result<bool, StoreError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM identities WHERE identity_key = ?1",
            [key.as_slice()],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(found.is_some())
}

fn rekey(
    conn: &Connection,
    old: &IdentityKey,
    new: &IdentityKey,
    version: &str,
) -> Result<(), StoreError> {
    // Every row that names the identity follows through ON UPDATE CASCADE: its sessions and its
    // sealed connection records. Its accounts belong to its group, which keeps its number.
    conn.execute(
        "UPDATE identities SET identity_key = ?2, key_version = ?3 WHERE identity_key = ?1",
        params![old.as_slice(), new.as_slice(), version],
    )
    .map_err(backend)?;
    Ok(())
}

pub(crate) fn identity(
    store: &SqliteStore,
    key: &IdentityKey,
) -> Result<Option<IdentityRow>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT provider, attested_age, terms_version, adult_attested_at FROM identities
             WHERE identity_key = ?1",
            [key.as_slice()],
            |row| {
                let age: Option<u32> = row.get(1)?;
                let terms: Option<u32> = row.get(2)?;
                let at: Option<i64> = row.get(3)?;
                let attested = match (age, terms, at) {
                    (Some(age), Some(terms), Some(at)) => Some(Attested { age, terms, at }),
                    _ => None,
                };
                Ok(IdentityRow {
                    key: *key,
                    provider: row.get(0)?,
                    attested,
                })
            },
        )
        .optional()
        .map_err(backend)
}

pub(crate) fn record_attestation(
    store: &SqliteStore,
    key: &IdentityKey,
    attested: &Attested,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "UPDATE identities SET attested_age = ?2, terms_version = ?3, adult_attested_at = ?4
             WHERE identity_key = ?1",
            params![key.as_slice(), attested.age, attested.terms, attested.at],
        )
        .map_err(backend)?;
    Ok(())
}

pub(crate) fn create_session(
    store: &SqliteStore,
    key: &IdentityKey,
    token_hash: &[u8; 32],
    now: i64,
    expiry: i64,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "INSERT INTO sessions (token_hash, identity_key, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![token_hash.as_slice(), key.as_slice(), now, expiry],
        )
        .map_err(backend)?;
    Ok(())
}

/// The identity behind a live token: not expired, not revoked.
pub(crate) fn session_identity(
    store: &SqliteStore,
    token_hash: &[u8; 32],
    now: i64,
) -> Result<Option<IdentityKey>, StoreError> {
    let key: Option<Vec<u8>> = store
        .connection()
        .query_row(
            "SELECT identity_key FROM sessions WHERE token_hash = ?1 AND expires_at > ?2",
            params![token_hash.as_slice(), now],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(key.and_then(|bytes| <IdentityKey>::try_from(bytes.as_slice()).ok()))
}

/// The identity behind a live token and when that sign-in happened.
pub(crate) fn session_row(
    store: &SqliteStore,
    token_hash: &[u8; 32],
    now: i64,
) -> Result<Option<(IdentityKey, i64)>, StoreError> {
    let found: Option<(Vec<u8>, i64)> = store
        .connection()
        .query_row(
            "SELECT identity_key, created_at FROM sessions WHERE token_hash = ?1 AND expires_at > ?2",
            params![token_hash.as_slice(), now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(backend)?;
    Ok(found.and_then(|(bytes, at)| Some((<IdentityKey>::try_from(bytes.as_slice()).ok()?, at))))
}

/// Revoking ends the token now; the row stays until the session purge.
pub(crate) fn revoke_session(
    store: &SqliteStore,
    token_hash: &[u8; 32],
    now: i64,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "UPDATE sessions SET expires_at = min(expires_at, ?2) WHERE token_hash = ?1",
            params![token_hash.as_slice(), now],
        )
        .map_err(backend)?;
    Ok(())
}

pub(crate) fn bind_session_account(
    store: &SqliteStore,
    token_hash: &[u8; 32],
    account: AccountId,
) -> Result<(), StoreError> {
    store
        .connection()
        .execute(
            "UPDATE sessions SET account = ?2 WHERE token_hash = ?1",
            params![token_hash.as_slice(), account.0],
        )
        .map_err(backend)?;
    Ok(())
}

/// Live accounts the identity's group owns, by number.
pub(crate) fn owned_accounts(
    store: &SqliteStore,
    key: &IdentityKey,
) -> Result<Vec<(u32, String)>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare_cached(&format!(
            "SELECT accounts.id, accounts.persona FROM group_accounts
             JOIN accounts ON accounts.id = group_accounts.account
             WHERE group_accounts.identity_group = {GROUP_OF_KEY} AND accounts.deleted_at IS NULL
             ORDER BY accounts.id"
        ))
        .map_err(backend)?;
    let rows = statement
        .query_map([key.as_slice()], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

/// True when the identity's group owns the live account.
pub(crate) fn owns(
    store: &SqliteStore,
    key: &IdentityKey,
    account: AccountId,
) -> Result<bool, StoreError> {
    let found: Option<i64> = store
        .connection()
        .query_row(
            &format!(
                "SELECT 1 FROM group_accounts
                 JOIN accounts ON accounts.id = group_accounts.account
                 WHERE group_accounts.identity_group = {GROUP_OF_KEY}
                 AND group_accounts.account = ?2 AND accounts.deleted_at IS NULL"
            ),
            params![key.as_slice(), account.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(found.is_some())
}

/// True when any account the identity's group owns, deleted or not, is suspended: a way back in
/// through a new account, a fresh password or a linked sign-in is closed while it is.
pub(crate) fn owns_suspended(store: &SqliteStore, key: &IdentityKey) -> Result<bool, StoreError> {
    owns_suspended_in(&store.connection(), key)
}

pub(super) fn owns_suspended_in(conn: &Connection, key: &IdentityKey) -> Result<bool, StoreError> {
    let found: Option<i64> = conn
        .query_row(
            &format!(
                "SELECT 1 FROM group_accounts
                 JOIN accounts ON accounts.id = group_accounts.account
                 WHERE group_accounts.identity_group = {GROUP_OF_KEY}
                 AND accounts.suspended_at IS NOT NULL LIMIT 1"
            ),
            [key.as_slice()],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(found.is_some())
}

/// Accounts the identity's group opened after `since`: the daily cap is per group.
pub(crate) fn accounts_created_since(
    store: &SqliteStore,
    key: &IdentityKey,
    since: i64,
) -> Result<u32, StoreError> {
    store
        .connection()
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM group_accounts
                 WHERE identity_group = {GROUP_OF_KEY} AND created_at > ?2"
            ),
            params![key.as_slice(), since],
            |row| row.get(0),
        )
        .map_err(backend)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Created {
    Account(AccountId),
    PersonaTaken,
}

/// A new account owned by `key`'s group, numbered like signup: one past the highest row ever created, from
/// 100001. Tombstones count, so a purged number is never issued again. A given persona is compared
/// without case against every account that still has one (a retired one included, a tombstone has
/// none): the web refuses a name in use. That is a naming rule, not attribution, which is by number
/// only. An empty persona (named later by the client's 40/4) is not compared.
pub(crate) fn create_owned_account(
    store: &SqliteStore,
    key: &IdentityKey,
    record: &AccountRecord,
    now: i64,
) -> Result<Created, StoreError> {
    let conn = store.connection();
    let tx = conn.unchecked_transaction().map_err(backend)?;
    // An account with no persona yet (empty) has no name to clash.
    let taken: Option<i64> = match record.persona.is_empty() {
        true => None,
        false => tx
            .query_row(
                "SELECT 1 FROM accounts WHERE persona = ?1 COLLATE NOCASE",
                [&record.persona],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?,
    };
    if taken.is_some() {
        return Ok(Created::PersonaTaken);
    }
    let highest: Option<i64> = tx
        .query_row("SELECT MAX(id) FROM accounts", [], |row| row.get(0))
        .map_err(backend)?;
    let floor = i64::from(FIRST_WEB_ACCOUNT) - 1;
    let next = highest.unwrap_or(0).max(floor) + 1;
    let id = u32::try_from(next)
        .map_err(|_| StoreError::Backend("the account-number space is used up".into()))?;
    tx.execute(
        "INSERT INTO accounts
         (id, password, persona, user_flags, rating, status, password_expired, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            record.password.0,
            record.persona,
            record.user_flags,
            record.rating,
            record.status.0,
            record.password_expired,
            now,
        ],
    )
    .map_err(backend)?;
    let owned = tx
        .execute(
            "INSERT INTO group_accounts (account, identity_group, created_at)
             SELECT ?2, identity_group, ?3 FROM identities WHERE identity_key = ?1",
            params![key.as_slice(), id, now],
        )
        .map_err(backend)?;
    if owned != 1 {
        return Err(StoreError::Backend("the identity has no group".into()));
    }
    tx.commit().map_err(backend)?;
    Ok(Created::Account(AccountId(id)))
}

/// The sign-in identities behind one account (every one linked into its group), for `account
/// export`. The sealed subject is named, not exported: only the escrow key opens it.
pub(crate) fn export_identities(
    store: &SqliteStore,
    account: AccountId,
) -> Result<Value, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT identities.identity_key, provider, key_version, adult_attested_at, attested_age,
                    terms_version, identities.created_at, deleted_at, subject_sealed IS NOT NULL,
                    linked_at, identities.identity_group
             FROM group_accounts JOIN identities USING (identity_group)
             WHERE group_accounts.account = ?1 ORDER BY linked_at, identities.created_at",
        )
        .map_err(backend)?;
    let found = statement
        .query_map([account.0], |row| {
            let key: Vec<u8> = row.get(0)?;
            let attested = match (row.get::<_, Option<u32>>(4)?, row.get::<_, Option<u32>>(5)?) {
                (Some(age), Some(terms)) => {
                    let at = row.get::<_, Option<i64>>(3)?;
                    json!({"age": age, "terms": terms, "at": at, "on": at.map(crate::dates::format_date)})
                }
                _ => Value::Null,
            };
            let sealed: bool = row.get(8)?;
            let value = json!({
                "handle": IdentityKey::try_from(key.as_slice()).map(|k| keys::handle(&k)).ok(),
                "identity_key_hex": hex(&key),
                "provider": row.get::<_, String>(1)?,
                "key_version": row.get::<_, String>(2)?,
                "attested": attested,
                "created_at": row.get::<_, i64>(6)?,
                "linked_at": row.get::<_, Option<i64>>(9)?,
                "deleted_at": row.get::<_, Option<i64>>(7)?,
                "subject": if sealed { SUBJECT_SEALED } else { SUBJECT_NOT_KEPT },
            });
            Ok((key, row.get::<_, i64>(10)?, value))
        })
        .map_err(backend)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(backend)?;
    drop(statement);
    let mut identities = Vec::with_capacity(found.len());
    for (key, group, mut value) in found {
        let accounts = group_account_ids(&conn, group)?;
        let sessions = identity_sessions(&conn, &key)?;
        if let Value::Object(map) = &mut value {
            map.insert("accounts".into(), json!(accounts));
            map.insert("sessions".into(), Value::Array(sessions));
        }
        identities.push(value);
    }
    Ok(Value::Array(identities))
}

fn group_account_ids(conn: &Connection, group: i64) -> Result<Vec<u32>, StoreError> {
    let mut statement = conn
        .prepare("SELECT account FROM group_accounts WHERE identity_group = ?1 ORDER BY account")
        .map_err(backend)?;
    let ids = statement
        .query_map([group], |row| row.get(0))
        .map_err(backend)?
        .collect::<Result<_, _>>()
        .map_err(backend)?;
    Ok(ids)
}

/// Each session's account and clock; never the token hash.
fn identity_sessions(conn: &Connection, key: &[u8]) -> Result<Vec<Value>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT account, created_at, expires_at FROM sessions WHERE identity_key = ?1
             ORDER BY created_at",
        )
        .map_err(backend)?;
    let sessions = statement
        .query_map([key], |row| {
            Ok(json!({
                "account_id": row.get::<_, Option<u32>>(0)?,
                "created_at": row.get::<_, i64>(1)?,
                "expiry": row.get::<_, i64>(2)?,
            }))
        })
        .map_err(backend)?
        .collect::<Result<_, _>>()
        .map_err(backend)?;
    Ok(sessions)
}

const SUBJECT_SEALED: &str =
    "sealed: the provider's issuer and subject are encrypted to the operator key, not exported";
const SUBJECT_NOT_KEPT: &str = "not kept: no audit key was configured at sign-in";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
