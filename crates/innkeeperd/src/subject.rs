//! Offline account export, deletion and restore against `--data-dir` (docs/server/privacy.md).

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_world::{
    AccountId, Envelope, HostTime, MailboxNumber, RetentionPolicy, Store, StoreError,
};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use thiserror::Error;

use crate::config::AccountAction;
use crate::dates;
use crate::retention::{self, Disable, PurgeNow, Restored};
use crate::server::{ServerError, DATABASE_FILE, LOCK_FILE};
use crate::sqlite_store::SqliteStore;

#[derive(Debug, Error)]
enum SubjectError {
    #[error("innkeeperd is running (pid {pid}); stop the server first")]
    ServerRunning { pid: u32 },
    #[error("account {0} does not exist")]
    Unknown(u32),
    #[error("cannot open the account database: {0}")]
    Database(#[from] StoreError),
    #[error("account {0} is under a legal hold")]
    Held(u32),
    #[error("account {0} is not deleted")]
    NotDeleted(u32),
    #[error("account {id} was disabled on {deleted} and the retention window has closed")]
    WindowClosed { id: u32, deleted: String },
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// A lock file in the data dir so offline tools can see a live server.
#[derive(Debug)]
pub(crate) struct DataDirLock {
    path: PathBuf,
}

impl DataDirLock {
    pub(crate) fn acquire(dir: &Path) -> Result<Self, ServerError> {
        let path = dir.join(LOCK_FILE);
        if let Some(pid) = running_pid(dir) {
            return Err(ServerError::AlreadyRunning { pid });
        }
        std::fs::write(&path, format!("{}\n", std::process::id())).map_err(|source| {
            ServerError::Lock {
                path: path.display().to_string(),
                source,
            }
        })?;
        Ok(DataDirLock { path })
    }
}

impl Drop for DataDirLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn running_pid(data_dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(data_dir.join(LOCK_FILE)).ok()?;
    let pid: u32 = text.trim().parse().ok()?;
    pid_alive(pid).then_some(pid)
}

fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

pub(crate) fn run(action: AccountAction, policy: RetentionPolicy) -> ExitCode {
    match run_action(action, policy) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run_action(action: AccountAction, policy: RetentionPolicy) -> Result<ExitCode, SubjectError> {
    match action {
        AccountAction::Export {
            id,
            data_dir,
            with_saves,
        } => {
            refuse_if_running(&data_dir)?;
            let store = open_store(&data_dir)?;
            let document = export_account(&store, AccountId(id), with_saves)?;
            println!("{}", serde_json::to_string_pretty(&document)?);
            Ok(ExitCode::SUCCESS)
        }
        AccountAction::Restore { id, data_dir } => restore_cmd(id, &data_dir, policy),
        AccountAction::Delete {
            id,
            data_dir,
            yes,
            purge_now,
        } => delete_cmd(id, &data_dir, yes, purge_now, policy),
    }
}

fn delete_cmd(
    id: u32,
    data_dir: &Path,
    yes: bool,
    purge_now: bool,
    policy: RetentionPolicy,
) -> Result<ExitCode, SubjectError> {
    refuse_if_running(data_dir)?;
    let store = open_store(data_dir)?;
    let row = store
        .account_any(AccountId(id))?
        .ok_or(SubjectError::Unknown(id))?;
    let now = dates::now_ms();
    if purge_now {
        if retention::account_held(&store, id, now)? {
            return Err(SubjectError::Held(id));
        }
        if !yes {
            println!(
                "Would purge account {id} now: its mail, posts, money, saves and identity link."
            );
            println!("The number stays as a tombstone. Sealed connection records and complaints are kept.");
            eprintln!("Pass --yes to purge.");
            return Ok(ExitCode::FAILURE);
        }
        return match retention::purge_now(&store, id, now)? {
            PurgeNow::Held => Err(SubjectError::Held(id)),
            PurgeNow::Missing => Err(SubjectError::Unknown(id)),
            PurgeNow::Done {
                letters,
                posts,
                saves,
            } => {
                println!(
                    "Purged account {id} ({letters} letters, {posts} posts, {saves} save versions). \
                     The number stays as a tombstone; sealed connection records and complaints are kept."
                );
                Ok(ExitCode::SUCCESS)
            }
        };
    }
    if let Some(purged_at) = row.purged_at {
        let on = dates::format_date(purged_at);
        println!("Account {id} was purged on {on} (UTC); only its number remains.");
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(deleted_at) = row.deleted_at {
        let purge = dates::format_date(dates::add_days(deleted_at, policy.account_days));
        println!("Account {id} is already disabled. Purge date {purge} (UTC).");
        return Ok(ExitCode::SUCCESS);
    }
    let plan = deletion_plan(&store, AccountId(id))?;
    if !yes {
        print_plan(&plan, policy);
        eprintln!("Pass --yes to disable the account.");
        return Ok(ExitCode::FAILURE);
    }
    apply_delete(&store, AccountId(id), policy)
}

fn restore_cmd(
    id: u32,
    data_dir: &Path,
    policy: RetentionPolicy,
) -> Result<ExitCode, SubjectError> {
    refuse_if_running(data_dir)?;
    let store = open_store(data_dir)?;
    match retention::restore_account(&store, id, policy, dates::now_ms())? {
        Restored::Ok => {
            println!("Restored account {id}.");
            Ok(ExitCode::SUCCESS)
        }
        Restored::Missing => Err(SubjectError::Unknown(id)),
        Restored::NotDeleted => Err(SubjectError::NotDeleted(id)),
        Restored::WindowClosed { deleted_at } => Err(SubjectError::WindowClosed {
            id,
            deleted: dates::format_date(deleted_at),
        }),
    }
}

fn refuse_if_running(data_dir: &Path) -> Result<(), SubjectError> {
    match running_pid(data_dir) {
        Some(pid) => Err(SubjectError::ServerRunning { pid }),
        None => Ok(()),
    }
}

fn open_store(data_dir: &Path) -> Result<SqliteStore, SubjectError> {
    Ok(SqliteStore::open(&data_dir.join(DATABASE_FILE))?)
}

/// What a delete would remove; sent letters and authored posts are deleted, not anonymised. Every
/// row is found by the account number: the mailbox's `account`, `letters.from_account` and
/// `posts.account`. Nothing is matched by name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeletionPlan {
    id: AccountId,
    persona: String,
    mailbox: Option<MailboxNumber>,
    letters_in_box: usize,
    letters_sent: usize,
    posts: usize,
    money: Option<u32>,
    save_versions: usize,
}

fn export_account(
    store: &SqliteStore,
    id: AccountId,
    with_saves: bool,
) -> Result<Value, SubjectError> {
    let row = store.account_any(id)?.ok_or(SubjectError::Unknown(id.0))?;
    let account = match &row.record {
        Some(record) => json!({
            "id": id.0,
            "persona": record.persona,
            "user_flags": record.user_flags,
            "rating": record.rating,
            "status": record.status.0,
            "password_expired": record.password_expired,
            "created_at": row.created_at,
            "deleted_at": row.deleted_at,
            "purged_at": row.purged_at,
        }),
        // A tombstone holds nothing personal: only its number and its clock.
        None => json!({
            "id": id.0,
            "created_at": row.created_at,
            "deleted_at": row.deleted_at,
            "purged_at": row.purged_at,
        }),
    };
    let mailbox = mailbox_of(store, id)?;
    let received = match mailbox {
        Some(number) => letters_json(store, "to_mailbox = ?1", number.0)?,
        None => Vec::new(),
    };
    let sent = letters_json(
        store,
        "from_account = ?1 AND to_mailbox NOT IN (SELECT number FROM mailboxes WHERE account = ?1)",
        id.0,
    )?;
    let posts = authored_posts_json(store, id)?;
    let (money, money_deleted_at) = money_of(store, id)?;
    let saves = saves_json(store, id, with_saves)?;
    let identities = crate::signin::store::export_identities(store, id)?;
    Ok(json!({
        "encoded_password": "omitted: the encoded password is not exported",
        "account": account,
        "mailbox": mailbox.map(|number| number.0),
        "letters_received": received,
        "letters_sent": sent,
        "posts": posts,
        "money": money,
        "money_deleted_at": money_deleted_at,
        "saves": saves,
        "identities": identities,
    }))
}

fn deletion_plan(store: &SqliteStore, id: AccountId) -> Result<DeletionPlan, SubjectError> {
    let record = store.account(id)?.ok_or(SubjectError::Unknown(id.0))?;
    let mailbox = mailbox_of(store, id)?;
    let letters_in_box = count(
        store,
        "SELECT COUNT(*) FROM letters WHERE deleted_at IS NULL
         AND to_mailbox IN (SELECT number FROM mailboxes WHERE account = ?1)",
        id,
    )?;
    let letters_sent = count(
        store,
        "SELECT COUNT(*) FROM letters WHERE deleted_at IS NULL AND from_account = ?1
         AND to_mailbox NOT IN (SELECT number FROM mailboxes WHERE account = ?1)",
        id,
    )?;
    let posts = count(
        store,
        "SELECT COUNT(*) FROM posts WHERE deleted_at IS NULL AND account = ?1",
        id,
    )?;
    let money = store.money(id)?.map(|amount| amount.0);
    let save_versions = count(store, "SELECT COUNT(*) FROM saves WHERE account = ?1", id)?;
    Ok(DeletionPlan {
        id,
        persona: record.persona,
        mailbox,
        letters_in_box,
        letters_sent,
        posts,
        money,
        save_versions,
    })
}

fn count(store: &SqliteStore, sql: &str, id: AccountId) -> Result<usize, StoreError> {
    let n: i64 = store
        .connection()
        .query_row(sql, [id.0], |row| row.get(0))
        .map_err(StoreError::backend)?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// What deleting an account would touch, for the portal's confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeletePreview {
    pub(crate) letters_in_box: usize,
    pub(crate) letters_sent: usize,
    pub(crate) posts: usize,
    pub(crate) save_versions: usize,
}

/// `None` when the account does not exist or is already disabled.
pub(crate) fn preview_delete(
    store: &SqliteStore,
    id: AccountId,
) -> Result<Option<DeletePreview>, StoreError> {
    match deletion_plan(store, id) {
        Ok(plan) => Ok(Some(DeletePreview {
            letters_in_box: plan.letters_in_box,
            letters_sent: plan.letters_sent,
            posts: plan.posts,
            save_versions: plan.save_versions,
        })),
        Err(SubjectError::Unknown(_)) => Ok(None),
        Err(SubjectError::Database(error)) => Err(error),
        Err(other) => Err(StoreError::backend(io::Error::other(other.to_string()))),
    }
}

/// The same soft delete as `account delete --yes`: the account and what it owns are disabled and
/// wait out the retention window. `Missing` for an account that is not live.
pub(crate) fn disable(store: &SqliteStore, id: AccountId) -> Result<Disable, StoreError> {
    if store.account(id)?.is_none() {
        return Ok(Disable::Missing);
    }
    retention::disable_account(store, id.0)
}

fn apply_delete(
    store: &SqliteStore,
    id: AccountId,
    policy: RetentionPolicy,
) -> Result<ExitCode, SubjectError> {
    let stamp = match retention::disable_account(store, id.0)? {
        Disable::Missing => return Err(SubjectError::Unknown(id.0)),
        Disable::Fresh(stamp) | Disable::Already(stamp) => stamp,
    };
    let purge = dates::format_date(dates::add_days(stamp, policy.account_days));
    println!(
        "Disabled account {}. Purge date {purge} (UTC). Sealed connection records are kept.",
        id.0
    );
    Ok(ExitCode::SUCCESS)
}

fn mailbox_of(store: &SqliteStore, id: AccountId) -> Result<Option<MailboxNumber>, StoreError> {
    store
        .connection()
        .query_row(
            "SELECT number FROM mailboxes WHERE account = ?1",
            [id.0],
            |row| row.get(0),
        )
        .optional()
        .map(|number| number.map(MailboxNumber))
        .map_err(StoreError::backend)
}

/// The letters a `WHERE` clause on `?1` selects, deleted ones included, oldest first.
fn letters_json(store: &SqliteStore, filter: &str, key: u32) -> Result<Vec<Value>, StoreError> {
    let connection = store.connection();
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, to_mailbox, from_account, body, created_at, deleted_at FROM letters
             WHERE {filter} ORDER BY id"
        ))
        .map_err(StoreError::backend)?;
    let rows = statement
        .query_map([key], |row| {
            let body: Vec<u8> = row.get(3)?;
            Ok(letter_json(
                LetterRow {
                    id: row.get(0)?,
                    mailbox: row.get(1)?,
                    from_account: row.get(2)?,
                    created_at: row.get(4)?,
                    deleted_at: row.get(5)?,
                },
                &body,
            ))
        })
        .map_err(StoreError::backend)?;
    rows.collect::<Result<_, _>>().map_err(StoreError::backend)
}

struct LetterRow {
    id: u32,
    mailbox: u32,
    from_account: u32,
    created_at: i64,
    deleted_at: Option<i64>,
}

fn letter_json(letter: LetterRow, body: &[u8]) -> Value {
    let parsed = Envelope::from_stored_body(body);
    json!({
        "id": letter.id,
        "mailbox": letter.mailbox,
        "from_account": letter.from_account,
        "from_mailbox": parsed.as_ref().map(|(_, envelope)| envelope.from.0),
        "sender": parsed.as_ref().map(|(_, envelope)| c_string(&envelope.sender)),
        "addressee": parsed.as_ref().map(|(_, envelope)| c_string(&envelope.addressee)),
        "subject": parsed.as_ref().map(|(_, envelope)| c_string(&envelope.subject)),
        "text": parsed.as_ref().map(|(_, envelope)| c_string(envelope.text.as_slice())),
        "received": parsed.as_ref().map(|(time, _)| time_json(*time)),
        "created_at": letter.created_at,
        "deleted_at": letter.deleted_at,
        "body_base64": BASE64.encode(body),
    })
}

/// Every post the account wrote (`posts.account`), complaints and Member Services messages
/// included, deleted ones too.
fn authored_posts_json(store: &SqliteStore, account: AccountId) -> Result<Vec<Value>, StoreError> {
    let connection = store.connection();
    let mut statement = connection
        .prepare(
            "SELECT id, board, kind, body, created_at, deleted_at FROM posts
             WHERE account = ?1 ORDER BY id",
        )
        .map_err(StoreError::backend)?;
    let rows = statement
        .query_map([account.0], |row| {
            let body: Vec<u8> = row.get(3)?;
            Ok(json!({
                "id": row.get::<_, u32>(0)?,
                "board": row.get::<_, u32>(1)?,
                "kind": row.get::<_, String>(2)?,
                "body": String::from_utf8_lossy(trim_nul(&body)).into_owned(),
                "created_at": row.get::<_, i64>(4)?,
                "deleted_at": row.get::<_, Option<i64>>(5)?,
                "body_base64": BASE64.encode(&body),
            }))
        })
        .map_err(StoreError::backend)?;
    rows.collect::<Result<_, _>>().map_err(StoreError::backend)
}

/// The `author` before the first `|` of a post body: the client's bytes, shown to the operator as
/// written. It attributes nothing; `posts.account` does.
pub(crate) fn post_author(body: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(trim_nul(body)).ok()?;
    let author = text.split('|').next()?.trim();
    (!author.is_empty()).then_some(author)
}

pub(crate) fn trim_nul(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    bytes.get(..end).unwrap_or(bytes)
}

pub(crate) fn c_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(trim_nul(bytes)).into_owned()
}

fn time_json(time: HostTime) -> Value {
    json!({
        "year": u16::from(time.year_1900) + 1900,
        "month": u16::from(time.month0) + 1,
        "day": time.mday,
        "hour": time.hour,
        "minute": time.minute,
        "second": time.second,
    })
}

fn saves_json(
    store: &SqliteStore,
    id: AccountId,
    with_saves: bool,
) -> Result<Vec<Value>, StoreError> {
    let connection = store.connection();
    let sql = if with_saves {
        "SELECT path, version, hash, length(data), modified_at, current, deleted_at, data \
         FROM saves WHERE account = ?1 ORDER BY path, version"
    } else {
        "SELECT path, version, hash, length(data), modified_at, current, deleted_at FROM saves \
         WHERE account = ?1 ORDER BY path, version"
    };
    let mut statement = connection.prepare(sql).map_err(StoreError::backend)?;
    let rows = statement
        .query_map([id.0], |row| {
            let mut value = json!({
                "path": row.get::<_, String>(0)?,
                "version": unsigned(row.get(1)?),
                "hash": row.get::<_, String>(2)?,
                "size": unsigned(row.get(3)?),
                "modified": unsigned(row.get(4)?),
                "current": row.get::<_, bool>(5)?,
                "deleted_at": row.get::<_, Option<i64>>(6)?,
            });
            if with_saves {
                let data: Vec<u8> = row.get(7)?;
                if let Value::Object(map) = &mut value {
                    map.insert("data_base64".into(), Value::String(BASE64.encode(data)));
                }
            }
            Ok(value)
        })
        .map_err(StoreError::backend)?;
    rows.collect::<Result<_, _>>().map_err(StoreError::backend)
}

fn unsigned(stored: i64) -> u64 {
    u64::try_from(stored).unwrap_or(0)
}

fn money_of(store: &SqliteStore, id: AccountId) -> Result<(Option<i64>, Option<i64>), StoreError> {
    let connection = store.connection();
    let mut statement = connection
        .prepare("SELECT amount, deleted_at FROM money WHERE account = ?1")
        .map_err(StoreError::backend)?;
    let mut rows = statement
        .query_map([id.0], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(StoreError::backend)?;
    match rows.next() {
        Some(Ok(pair)) => Ok((Some(pair.0), pair.1)),
        Some(Err(error)) => Err(StoreError::backend(error)),
        None => Ok((None, None)),
    }
}

fn print_plan(plan: &DeletionPlan, policy: RetentionPolicy) {
    let purge = dates::format_date(dates::add_days(dates::now_ms(), policy.account_days));
    println!(
        "Would disable account {} (persona {}) until purge date {purge} (UTC):",
        plan.id.0, plan.persona
    );
    match plan.mailbox {
        Some(number) => println!(
            "  mailbox {} with {} letters",
            number.0, plan.letters_in_box
        ),
        None => println!("  no mailbox"),
    }
    println!(
        "  {} letters sent to other mailboxes would be hidden",
        plan.letters_sent
    );
    println!(
        "  {} board posts and complaints written by the account would be hidden",
        plan.posts
    );
    match plan.money {
        Some(amount) => println!("  bankroll {amount}"),
        None => println!("  no bankroll"),
    }
    println!("  {} save file versions", plan.save_versions);
    println!("  sealed connection records are kept");
}

impl From<serde_json::Error> for SubjectError {
    fn from(error: serde_json::Error) -> Self {
        SubjectError::Io(io::Error::other(error))
    }
}

#[cfg(test)]
#[path = "subject_tests.rs"]
mod tests;
