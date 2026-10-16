//! Sealed connection records, and the offline tools that export and open them.
//! The format and the escrow vault are in docs/server/privacy.md.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_world::StoreError;
use rsa::RsaPrivateKey;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use thiserror::Error;
use tracing::warn;

use crate::config::AuditAction;
use crate::dates::{self, DateError};
use crate::retention;
use crate::seal::{self, Recipient, SealError};
use crate::server::DATABASE_FILE;
use crate::sqlite_store::SqliteStore;
use crate::subject;

const DAY: i64 = 86_400;

/// The operator public key and the store a connection's sealed row is written to.
pub(crate) struct AuditSeal {
    store: Arc<SqliteStore>,
    recipient: Recipient,
}

impl fmt::Debug for AuditSeal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuditSeal")
            .field("kid", &self.recipient.kid)
            .finish_non_exhaustive()
    }
}

impl AuditSeal {
    pub(crate) fn new(store: Arc<SqliteStore>, recipient: Recipient) -> Self {
        AuditSeal { store, recipient }
    }
}

/// One connection's sealed row. Dropping it records the end time.
pub(crate) struct AuditTicket {
    seal: Arc<AuditSeal>,
    id: i64,
    started_at: i64,
    ended_at: Option<i64>,
    account_id: Option<u32>,
    peer_ip: String,
    peer_port: u16,
    transport: &'static str,
    closed: bool,
}

impl fmt::Debug for AuditTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuditTicket")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Drop for AuditTicket {
    fn drop(&mut self) {
        self.close_now();
    }
}

/// Opens a row when a recipient is configured. Without one, nothing is written.
pub(crate) fn begin(
    seal: Option<&Arc<AuditSeal>>,
    peer: SocketAddr,
    transport: &'static str,
) -> Option<AuditTicket> {
    let seal = seal?;
    match open_session(seal, peer, transport) {
        Ok(ticket) => Some(ticket),
        Err(error) => {
            warn!(%error, "sealed record was not written");
            None
        }
    }
}

fn open_session(
    seal: &Arc<AuditSeal>,
    peer: SocketAddr,
    transport: &'static str,
) -> Result<AuditTicket, AuditError> {
    let started_at = dates::unix_now();
    let mut ticket = AuditTicket {
        seal: Arc::clone(seal),
        id: 0,
        started_at,
        ended_at: None,
        account_id: None,
        peer_ip: peer.ip().to_string(),
        peer_port: peer.port(),
        transport,
        closed: false,
    };
    let sealed = ticket.seal_record()?;
    let conn = seal.store.connection();
    conn.execute(
        "INSERT INTO audit_sessions
         (started_at, ended_at, account_id, identity_key, hold_until, sealed)
         VALUES (?1, NULL, NULL, NULL, NULL, ?2)",
        params![started_at, sealed],
    )
    .map_err(StoreError::backend)?;
    ticket.id = conn.last_insert_rowid();
    Ok(ticket)
}

impl AuditTicket {
    /// The line presented a session token: the row names the identity's HMAC in the clear.
    pub(crate) fn note_identity(&mut self, identity: &[u8; 32]) {
        let result = self.seal.store.connection().execute(
            "UPDATE audit_sessions SET identity_key = ?2 WHERE id = ?1",
            params![self.id, identity.as_slice()],
        );
        if let Err(error) = result {
            warn!(%error, "sealed record was not linked to its identity");
        }
    }

    /// Login was accepted. A repeat of the same account does not reseal.
    pub(crate) fn note_account(&mut self, account: u32) {
        if self.closed || self.account_id == Some(account) {
            return;
        }
        if let Err(error) = self.write_account(account) {
            warn!(%error, "sealed record was not updated");
        }
    }

    pub(crate) fn close_now(&mut self) {
        if let Err(error) = self.close() {
            warn!(%error, "sealed record was not closed");
        }
    }

    fn write_account(&mut self, account: u32) -> Result<(), AuditError> {
        let previous = self.account_id;
        self.account_id = Some(account);
        let sealed = match self.seal_record() {
            Ok(sealed) => sealed,
            Err(error) => {
                self.account_id = previous;
                return Err(error);
            }
        };
        let hold = hold_for(&self.seal.store, account, dates::unix_now())?;
        let result = self.seal.store.connection().execute(
            "UPDATE audit_sessions SET account_id = ?2, hold_until = ?3, sealed = ?4 WHERE id = ?1",
            params![self.id, i64::from(account), hold, sealed],
        );
        if let Err(error) = result {
            self.account_id = previous;
            return Err(StoreError::backend(error).into());
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), AuditError> {
        if self.closed {
            return Ok(());
        }
        let previous = self.ended_at;
        self.ended_at = Some(dates::unix_now());
        let sealed = match self.seal_record() {
            Ok(sealed) => sealed,
            Err(error) => {
                self.ended_at = previous;
                return Err(error);
            }
        };
        let result = self.seal.store.connection().execute(
            "UPDATE audit_sessions SET ended_at = ?2, account_id = ?3, sealed = ?4 WHERE id = ?1",
            params![
                self.id,
                self.ended_at,
                self.account_id.map(i64::from),
                sealed
            ],
        );
        if let Err(error) = result {
            self.ended_at = previous;
            return Err(StoreError::backend(error).into());
        }
        self.closed = true;
        Ok(())
    }

    fn seal_record(&self) -> Result<Vec<u8>, AuditError> {
        let value = json!({
            "peer_ip": self.peer_ip,
            "peer_port": self.peer_port,
            "transport": self.transport,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
            "account_id": self.account_id,
        });
        let bytes = serde_json::to_vec(&value).map_err(|_| SealError::Envelope)?;
        Ok(seal::seal(&self.seal.recipient, &bytes)?)
    }
}

fn hold_for(store: &SqliteStore, account: u32, now: i64) -> Result<Option<i64>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT hold_until FROM holds WHERE account = ?1 AND hold_until > ?2",
            params![account, now],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::backend)
}

#[derive(Debug, Error)]
pub(crate) enum AuditError {
    #[error("innkeeperd is running (pid {pid}); stop the server first")]
    ServerRunning { pid: u32 },
    #[error("cannot open the account database: {0}")]
    Database(#[from] StoreError),
    #[error("{0}")]
    Seal(#[from] SealError),
    #[error("{0}")]
    Date(#[from] DateError),
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("a row in the export is not a sealed record")]
    Row,
    #[error("no data key matches a sealed record")]
    DataKey,
}

pub(crate) fn run(action: AuditAction) -> ExitCode {
    match dispatch(action) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn run_hold(account: u32, until: &str, data_dir: &Path) -> ExitCode {
    finish(hold(account, until, data_dir))
}

pub(crate) fn run_release(account: u32, data_dir: &Path) -> ExitCode {
    finish(release(account, data_dir))
}

fn finish(result: Result<(), AuditError>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(action: AuditAction) -> Result<(), AuditError> {
    match action {
        AuditAction::Export {
            account,
            since,
            until,
            data_dir,
        } => export_cmd(account, since.as_deref(), until.as_deref(), &data_dir),
        AuditAction::Open {
            r#in,
            private_key,
            data_keys,
        } => open_cmd(&r#in, private_key.as_deref(), data_keys.as_deref()),
        AuditAction::Hold {
            account,
            until,
            data_dir,
        } => hold(account, &until, &data_dir),
        AuditAction::Release { account, data_dir } => release(account, &data_dir),
        AuditAction::DevKeygen { out_dir } => dev_keygen(&out_dir),
        AuditAction::Seal {
            public_key,
            key_id,
            record,
        } => seal_record(&public_key, &key_id, &record),
    }
}

fn export_cmd(
    account: u32,
    since: Option<&str>,
    until: Option<&str>,
    data_dir: &Path,
) -> Result<(), AuditError> {
    refuse_if_running(data_dir)?;
    let since = since.map(dates::parse_date).transpose()?;
    let until = until
        .map(|text| {
            dates::parse_date(text)?
                .checked_add(DAY)
                .ok_or(DateError::Format)
        })
        .transpose()?;
    let store = open_store(data_dir)?;
    for line in export_rows(&store, account, since, until)? {
        println!("{}", serde_json::to_string(&line)?);
    }
    Ok(())
}

fn open_cmd(
    input: &Path,
    private_key: Option<&Path>,
    data_keys: Option<&Path>,
) -> Result<(), AuditError> {
    let text = std::fs::read_to_string(input)?;
    let source = match (private_key, data_keys) {
        (Some(path), None) => {
            eprintln!(
                "DEVELOPMENT KEY: --private-key decrypts on this machine. \
                 Production unwraps each data key with the escrow vault."
            );
            KeySource::Private(Box::new(seal::load_private_key(path)?))
        }
        (None, Some(path)) => KeySource::Keys(read_data_keys(path)?),
        _ => {
            return Err(AuditError::Io(io::Error::other(
                "pass --private-key or --data-keys",
            )))
        }
    };
    for plain in decrypt_export(&text, &source)? {
        println!("{plain}");
    }
    Ok(())
}

fn hold(account: u32, until: &str, data_dir: &Path) -> Result<(), AuditError> {
    refuse_if_running(data_dir)?;
    let until = dates::hold_until(until, dates::unix_now())?;
    let store = open_store(data_dir)?;
    let stored = retention::place_hold(&store, account, until)?;
    let through = dates::format_date(stored.saturating_sub(DAY));
    println!("account {account} held through {through}");
    Ok(())
}

fn release(account: u32, data_dir: &Path) -> Result<(), AuditError> {
    refuse_if_running(data_dir)?;
    let store = open_store(data_dir)?;
    retention::clear_hold(&store, account)?;
    println!("cleared the hold on account {account}");
    Ok(())
}

fn dev_keygen(dir: &Path) -> Result<(), AuditError> {
    seal::write_dev_keypair(dir, seal::DEV_KEY_BITS)?;
    eprintln!(
        "DEVELOPMENT KEY: {} is for local development only. \
         Production keys are created inside the escrow vault and never leave it.",
        dir.join("audit-private.pem").display()
    );
    println!("wrote {}", dir.join("audit-public.pem").display());
    Ok(())
}

fn seal_record(public_key: &Path, kid: &str, record: &str) -> Result<(), AuditError> {
    let key = seal::load_public_key(public_key)?;
    let bits = seal::key_bits(&key);
    if bits < seal::RECOMMENDED_KEY_BITS {
        eprintln!("the audit public key is {bits} bits; the escrow key is 3072 or larger");
    }
    let recipient = Recipient {
        key,
        kid: kid.to_owned(),
    };
    let sealed = seal::seal(&recipient, record.as_bytes())?;
    let view = seal::view(&sealed)?;
    let line = json!({
        "id": Value::Null,
        "started_at": Value::Null,
        "ended_at": Value::Null,
        "account_id": Value::Null,
        "kid": view.kid,
        "wrapped_key_b64": BASE64.encode(view.wrapped_key),
        "sealed_b64": BASE64.encode(sealed),
    });
    println!("{}", serde_json::to_string(&line)?);
    Ok(())
}

fn refuse_if_running(data_dir: &Path) -> Result<(), AuditError> {
    match subject::running_pid(data_dir) {
        Some(pid) => Err(AuditError::ServerRunning { pid }),
        None => Ok(()),
    }
}

fn open_store(data_dir: &Path) -> Result<SqliteStore, AuditError> {
    Ok(SqliteStore::open(&data_dir.join(DATABASE_FILE))?)
}

pub(crate) fn export_rows(
    store: &SqliteStore,
    account: u32,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<Vec<Value>, AuditError> {
    let conn = store.connection();
    let mut statement = conn
        .prepare(
            "SELECT id, started_at, ended_at, account_id, sealed FROM audit_sessions
             WHERE account_id = ?1
             AND (?2 IS NULL OR started_at >= ?2)
             AND (?3 IS NULL OR started_at < ?3)
             ORDER BY started_at, id",
        )
        .map_err(StoreError::backend)?;
    let rows = statement
        .query_map(params![account, since, until], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(StoreError::backend)?;
    let mut lines = Vec::new();
    for row in rows {
        let (id, started_at, ended_at, account_id, sealed) = row.map_err(StoreError::backend)?;
        let view = seal::view(&sealed)?;
        lines.push(json!({
            "id": id,
            "started_at": started_at,
            "ended_at": ended_at,
            "account_id": account_id,
            "kid": view.kid,
            "wrapped_key_b64": BASE64.encode(view.wrapped_key),
            "sealed_b64": BASE64.encode(sealed),
        }));
    }
    drop(statement);
    lines.extend(identity_rows(&conn, account)?);
    Ok(lines)
}

/// The sealed provider subject of the identity that owns the account, with `"record":
/// "identity"` and a null `id`; `audit open` matches its data key by the wrapped key.
fn identity_rows(conn: &rusqlite::Connection, account: u32) -> Result<Vec<Value>, AuditError> {
    let found: Option<(Vec<u8>, String, i64)> = conn
        .query_row(
            "SELECT identities.subject_sealed, identities.provider, identities.created_at
             FROM identity_accounts JOIN identities USING (identity_key)
             WHERE identity_accounts.account_id = ?1 AND identities.subject_sealed IS NOT NULL",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(StoreError::backend)?;
    let Some((sealed, provider, created_at)) = found else {
        return Ok(Vec::new());
    };
    let view = seal::view(&sealed)?;
    Ok(vec![json!({
        "id": Value::Null,
        "record": "identity",
        "provider": provider,
        "started_at": created_at,
        "ended_at": Value::Null,
        "account_id": account,
        "kid": view.kid,
        "wrapped_key_b64": BASE64.encode(view.wrapped_key),
        "sealed_b64": BASE64.encode(sealed),
    })])
}

enum KeySource {
    Private(Box<RsaPrivateKey>),
    Keys(Vec<DataKey>),
}

struct DataKey {
    id: Option<i64>,
    wrapped: Option<Vec<u8>>,
    data_key: Vec<u8>,
}

fn decrypt_export(text: &str, source: &KeySource) -> Result<Vec<String>, AuditError> {
    let mut opened = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)?;
        let sealed_b64 = value
            .get("sealed_b64")
            .and_then(Value::as_str)
            .ok_or(AuditError::Row)?;
        let sealed = seal::decode_b64(sealed_b64)?;
        let plain = match source {
            KeySource::Private(key) => seal::open_with_private_key(&sealed, key)?,
            KeySource::Keys(keys) => {
                let id = value.get("id").and_then(Value::as_i64);
                let wrapped = seal::view(&sealed)?.wrapped_key;
                let data_key = find_key(id, &wrapped, keys).ok_or(AuditError::DataKey)?;
                seal::open_with_data_key(&sealed, data_key)?
            }
        };
        opened.push(String::from_utf8(plain).map_err(|_| AuditError::Row)?);
    }
    Ok(opened)
}

fn find_key<'a>(id: Option<i64>, wrapped: &[u8], keys: &'a [DataKey]) -> Option<&'a [u8]> {
    if let Some(id) = id {
        if let Some(found) = keys.iter().find(|key| key.id == Some(id)) {
            return Some(&found.data_key);
        }
    }
    keys.iter()
        .find(|key| key.wrapped.as_deref() == Some(wrapped))
        .map(|key| key.data_key.as_slice())
}

fn read_data_keys(path: &Path) -> Result<Vec<DataKey>, AuditError> {
    let text = std::fs::read_to_string(path)?;
    let mut keys = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)?;
        let data_key = value
            .get("data_key_b64")
            .and_then(Value::as_str)
            .ok_or(AuditError::DataKey)?;
        let wrapped = value
            .get("wrapped_key_b64")
            .and_then(Value::as_str)
            .map(seal::decode_b64)
            .transpose()?;
        keys.push(DataKey {
            id: value.get("id").and_then(Value::as_i64),
            wrapped,
            data_key: seal::decode_b64(data_key)?,
        });
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use rsa::RsaPublicKey;

    use super::*;

    fn private_key() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| RsaPrivateKey::new(&mut aes_gcm::aead::OsRng, 2048).unwrap())
    }

    fn seal_for(store: Arc<SqliteStore>) -> Arc<AuditSeal> {
        let recipient = Recipient {
            key: RsaPublicKey::from(private_key()),
            kid: "test-kid".into(),
        };
        Arc::new(AuditSeal::new(store, recipient))
    }

    fn plain_of(store: &SqliteStore, id: i64) -> Value {
        let sealed: Vec<u8> = store
            .connection()
            .query_row(
                "SELECT sealed FROM audit_sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        let opened = seal::open_with_private_key(&sealed, private_key()).unwrap();
        serde_json::from_slice(&opened).unwrap()
    }

    #[test]
    fn a_session_is_sealed_on_open_then_updated_on_login_and_close() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let sealing = seal_for(Arc::clone(&store));
        let peer: SocketAddr = "203.0.113.9:443".parse().unwrap();
        let mut ticket = begin(Some(&sealing), peer, "tcp").unwrap();
        let id = ticket.id;
        let plain = plain_of(&store, id);
        assert_eq!(plain["peer_ip"], "203.0.113.9");
        assert_eq!(plain["peer_port"], 443);
        assert_eq!(plain["transport"], "tcp");
        assert!(plain["account_id"].is_null());
        assert!(plain["ended_at"].is_null());
        let identity: Option<Vec<u8>> = store
            .connection()
            .query_row(
                "SELECT identity_key FROM audit_sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(identity.is_none());

        retention::place_hold(&store, 100_001, dates::unix_now() + 10 * DAY).unwrap();
        ticket.note_account(100_001);
        let plain = plain_of(&store, id);
        assert_eq!(plain["account_id"], 100_001);
        let hold: Option<i64> = store
            .connection()
            .query_row(
                "SELECT hold_until FROM audit_sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(hold.is_some());

        ticket.close_now();
        let plain = plain_of(&store, id);
        assert!(plain["ended_at"].as_i64().is_some());
        drop(ticket);

        let lines = export_rows(&store, 100_001, None, None).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["kid"], "test-kid");
        assert!(export_rows(&store, 100_002, None, None).unwrap().is_empty());
        let text = serde_json::to_string(&lines[0]).unwrap();
        let from_private =
            decrypt_export(&text, &KeySource::Private(Box::new(private_key().clone()))).unwrap();
        assert_eq!(from_private.len(), 1);
        let wrapped =
            seal::view(&seal::decode_b64(lines[0]["sealed_b64"].as_str().unwrap()).unwrap())
                .unwrap()
                .wrapped_key;
        let data_key = seal::unwrap_key(private_key(), &wrapped).unwrap();
        let keys = vec![DataKey {
            id: None,
            wrapped: Some(wrapped),
            data_key,
        }];
        let from_data = decrypt_export(&text, &KeySource::Keys(keys)).unwrap();
        assert_eq!(from_private, from_data);
    }

    #[test]
    fn without_a_recipient_nothing_is_written() {
        let store = SqliteStore::open_in_memory().unwrap();
        let peer: SocketAddr = "198.51.100.4:25".parse().unwrap();
        assert!(begin(None, peer, "websocket").is_none());
        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM audit_sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn export_can_bound_the_rows_by_date() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let sealing = seal_for(Arc::clone(&store));
        let peer: SocketAddr = "203.0.113.9:1".parse().unwrap();
        let mut early = begin(Some(&sealing), peer, "tcp").unwrap();
        early.note_account(7);
        early.close_now();
        store
            .connection()
            .execute(
                "UPDATE audit_sessions SET started_at = ?1 WHERE id = ?2",
                params![dates::parse_date("2020-01-01").unwrap(), early.id],
            )
            .unwrap();
        drop(early);
        let mut later = begin(Some(&sealing), peer, "tcp").unwrap();
        later.note_account(7);
        later.close_now();
        let since = dates::parse_date("2024-01-01").unwrap();
        let rows = export_rows(&store, 7, Some(since), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0]["started_at"].as_i64().unwrap() >= since);
    }
}
