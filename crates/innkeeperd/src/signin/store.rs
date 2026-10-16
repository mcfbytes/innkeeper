//! The sign-in tables: identities, the accounts they own and session tokens
//! (docs/server/privacy.md section 1).

use innkeeper_world::{AccountId, AccountRecord, StoreError};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::keys::{self, IdentityKey, KeyRing};
use crate::sqlite_store::SqliteStore;

/// Where signup starts numbering (`SIGNUP_FIRST_ACCOUNT_ASSUMED` in innkeeper-world).
const FIRST_WEB_ACCOUNT: u32 = 100_001;

/// The age and terms answer kept on an identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attested {
    pub(crate) age: u32,
    pub(crate) terms: u32,
    /// `YYYY-MM-DD`, a date only.
    pub(crate) on: String,
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
/// current key (which every linked row follows). Creates the row on first sign-in.
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
        "INSERT INTO identities (identity_key, key_version, subject_sealed, provider, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            current_key.as_slice(),
            current.version(),
            sealed(),
            provider,
            now
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
    conn.execute(
        "UPDATE identities SET identity_key = ?2, key_version = ?3 WHERE identity_key = ?1",
        params![old.as_slice(), new.as_slice(), version],
    )
    .map_err(backend)?;
    conn.execute(
        "UPDATE audit_sessions SET identity_key = ?2 WHERE identity_key = ?1",
        params![old.as_slice(), new.as_slice()],
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
                let on: Option<String> = row.get(3)?;
                let attested = match (age, terms, on) {
                    (Some(age), Some(terms), Some(on)) => Some(Attested { age, terms, on }),
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
            params![key.as_slice(), attested.age, attested.terms, attested.on],
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
            "INSERT INTO sessions (token_hash, identity_key, created_at, expiry)
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
            "SELECT identity_key FROM sessions WHERE token_hash = ?1 AND expiry > ?2",
            params![token_hash.as_slice(), now],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(key.and_then(|bytes| <IdentityKey>::try_from(bytes.as_slice()).ok()))
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
            "UPDATE sessions SET expiry = min(expiry, ?2) WHERE token_hash = ?1",
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
            "UPDATE sessions SET account_id = ?2 WHERE token_hash = ?1",
            params![token_hash.as_slice(), account.0],
        )
        .map_err(backend)?;
    Ok(())
}

/// Live accounts the identity owns, by number.
pub(crate) fn owned_accounts(
    store: &SqliteStore,
    key: &IdentityKey,
) -> Result<Vec<(u32, String)>, StoreError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare_cached(
            "SELECT accounts.id, accounts.persona FROM identity_accounts
             JOIN accounts ON accounts.id = identity_accounts.account_id
             WHERE identity_accounts.identity_key = ?1 AND accounts.deleted_at IS NULL
             ORDER BY accounts.id",
        )
        .map_err(backend)?;
    let rows = statement
        .query_map([key.as_slice()], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(backend)?;
    rows.collect::<Result<_, _>>().map_err(backend)
}

pub(crate) fn owns(
    store: &SqliteStore,
    key: &IdentityKey,
    account: AccountId,
) -> Result<bool, StoreError> {
    let found: Option<i64> = store
        .connection()
        .query_row(
            "SELECT 1 FROM identity_accounts
             JOIN accounts ON accounts.id = identity_accounts.account_id
             WHERE identity_accounts.identity_key = ?1 AND identity_accounts.account_id = ?2
             AND accounts.deleted_at IS NULL",
            params![key.as_slice(), account.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(backend)?;
    Ok(found.is_some())
}

pub(crate) fn accounts_created_since(
    store: &SqliteStore,
    key: &IdentityKey,
    since: i64,
) -> Result<u32, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM identity_accounts WHERE identity_key = ?1 AND created_at > ?2",
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

/// A new account owned by `key`, numbered like signup: one past the highest, from 100001.
/// A given persona is compared without case against every row, a retired one included; an empty
/// one (named later by the client's 40/4) is not compared.
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
        "INSERT INTO accounts (id, password, persona, user_flags, rating, status, password_expired)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            record.password.0,
            record.persona,
            record.user_flags,
            record.rating,
            record.status.0,
            record.password_expired,
        ],
    )
    .map_err(backend)?;
    tx.execute(
        "INSERT INTO identity_accounts (account_id, identity_key, created_at) VALUES (?1, ?2, ?3)",
        params![id, key.as_slice(), now],
    )
    .map_err(backend)?;
    tx.commit().map_err(backend)?;
    Ok(Created::Account(AccountId(id)))
}

/// The sign-in data behind one account, for `account export`. The sealed subject is named,
/// not exported: only the escrow key opens it.
pub(crate) fn export_identity(
    store: &SqliteStore,
    account: AccountId,
) -> Result<Value, StoreError> {
    let conn = store.connection();
    let found = conn
        .query_row(
            "SELECT identities.identity_key, provider, key_version, adult_attested_at, attested_age,
                    terms_version, identities.created_at, deleted_at, subject_sealed IS NOT NULL
             FROM identity_accounts JOIN identities USING (identity_key)
             WHERE identity_accounts.account_id = ?1",
            [account.0],
            |row| {
                let key: Vec<u8> = row.get(0)?;
                let attested = match (row.get::<_, Option<u32>>(4)?, row.get::<_, Option<u32>>(5)?) {
                    (Some(age), Some(terms)) => {
                        json!({"age": age, "terms": terms, "on": row.get::<_, Option<String>>(3)?})
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
                    "deleted_at": row.get::<_, Option<i64>>(7)?,
                    "subject": if sealed { SUBJECT_SEALED } else { SUBJECT_NOT_KEPT },
                });
                Ok((key, value))
            },
        )
        .optional()
        .map_err(backend)?;
    let Some((key, mut value)) = found else {
        return Ok(Value::Null);
    };
    let accounts = identity_account_ids(&conn, &key)?;
    let sessions = identity_sessions(&conn, &key)?;
    if let Value::Object(map) = &mut value {
        map.insert("accounts".into(), json!(accounts));
        map.insert("sessions".into(), Value::Array(sessions));
    }
    Ok(value)
}

fn identity_account_ids(conn: &Connection, key: &[u8]) -> Result<Vec<u32>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT account_id FROM identity_accounts WHERE identity_key = ?1 ORDER BY account_id",
        )
        .map_err(backend)?;
    let ids = statement
        .query_map([key], |row| row.get(0))
        .map_err(backend)?
        .collect::<Result<_, _>>()
        .map_err(backend)?;
    Ok(ids)
}

/// Each session's account and clock; never the token hash.
fn identity_sessions(conn: &Connection, key: &[u8]) -> Result<Vec<Value>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT account_id, created_at, expiry FROM sessions WHERE identity_key = ?1
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
