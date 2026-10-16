//! Offline account export, deletion and restore against `--data-dir` (docs/server/privacy.md).

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_world::{
    AccountId, Envelope, HostTime, MailboxNumber, RetentionPolicy, Store, StoreError,
};
use serde_json::{json, Value};
use thiserror::Error;

use crate::config::AccountAction;
use crate::dates;
use crate::retention::{self, Disable, OwnedRows, PurgeNow, Restored};
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
    let now = dates::unix_now();
    if purge_now {
        if retention::account_held(&store, id, now)? {
            return Err(SubjectError::Held(id));
        }
        if !yes {
            println!("Would hard-delete account {id} and everything it owns.");
            println!("Sealed connection records are kept.");
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
                     Sealed connection records are kept."
                );
                Ok(ExitCode::SUCCESS)
            }
        };
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
    apply_delete(&store, AccountId(id), &plan, policy)
}

fn restore_cmd(
    id: u32,
    data_dir: &Path,
    policy: RetentionPolicy,
) -> Result<ExitCode, SubjectError> {
    refuse_if_running(data_dir)?;
    let store = open_store(data_dir)?;
    match retention::restore_account(&store, id, policy, dates::unix_now())? {
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

/// What a delete would remove; sent letters and authored posts are deleted, not anonymised.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeletionPlan {
    id: AccountId,
    persona: String,
    mailbox: Option<MailboxNumber>,
    letters_in_box: usize,
    /// Letter ids in other mailboxes whose envelope `from` is this account's mailbox.
    sent_ids: Vec<u32>,
    letters_sent: usize,
    /// Post ids whose body starts with this account's stored persona.
    post_ids: Vec<u32>,
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
    let record = &row.record;
    let mailbox = mailbox_of(store, id)?;
    let received = match mailbox {
        Some(number) => letters_json(store, number)?,
        None => Vec::new(),
    };
    let sent = sent_letters_json(store, mailbox)?;
    let posts = authored_posts_json(store, &record.persona)?;
    let (money, money_deleted_at) = money_of(store, id)?;
    let saves = saves_json(store, id, with_saves)?;
    Ok(json!({
        "encoded_password": "omitted: the encoded password is not exported",
        "account": {
            "id": id.0,
            "persona": record.persona,
            "user_flags": record.user_flags,
            "rating": record.rating,
            "status": record.status.0,
            "password_expired": record.password_expired,
            "deleted_at": row.deleted_at,
        },
        "mailbox": mailbox.map(|number| number.0),
        "letters_received": received,
        "letters_sent": sent,
        "posts": posts,
        "money": money,
        "money_deleted_at": money_deleted_at,
        "saves": saves,
    }))
}

fn deletion_plan(store: &SqliteStore, id: AccountId) -> Result<DeletionPlan, SubjectError> {
    let record = store.account(id)?.ok_or(SubjectError::Unknown(id.0))?;
    let mailbox = mailbox_of(store, id)?;
    let letters_in_box = match mailbox {
        Some(number) => store.letters(number)?.len(),
        None => 0,
    };
    let sent_ids = sent_letter_ids(store, mailbox)?;
    let post_ids = authored_post_ids(store, &record.persona)?;
    let money = store.money(id)?.map(|amount| amount.0);
    let save_versions = count_saves(store, id)?;
    Ok(DeletionPlan {
        id,
        persona: record.persona,
        mailbox,
        letters_in_box,
        letters_sent: sent_ids.len(),
        sent_ids,
        posts: post_ids.len(),
        post_ids,
        money,
        save_versions,
    })
}

fn apply_delete(
    store: &SqliteStore,
    id: AccountId,
    plan: &DeletionPlan,
    policy: RetentionPolicy,
) -> Result<ExitCode, SubjectError> {
    let owned = OwnedRows {
        mailbox: plan.mailbox.map(|number| number.0),
        sent: plan.sent_ids.clone(),
        posts: plan.post_ids.clone(),
    };
    let stamp = match retention::disable_account(store, id.0, &owned)? {
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
    let connection = store.connection();
    let mut statement = connection
        .prepare("SELECT number FROM mailboxes WHERE account = ?1")
        .map_err(StoreError::backend)?;
    let mut rows = statement
        .query_map([id.0], |row| row.get(0))
        .map_err(StoreError::backend)?;
    match rows.next() {
        Some(Ok(number)) => Ok(Some(MailboxNumber(number))),
        Some(Err(error)) => Err(StoreError::backend(error)),
        None => Ok(None),
    }
}

fn letters_json(store: &SqliteStore, mailbox: MailboxNumber) -> Result<Vec<Value>, StoreError> {
    Ok(all_letters(store)?
        .into_iter()
        .filter(|(_, number, _, _)| *number == mailbox.0)
        .map(|(id, number, body, deleted_at)| letter_json(id, number, &body, deleted_at))
        .collect())
}

fn sent_letters_json(
    store: &SqliteStore,
    own: Option<MailboxNumber>,
) -> Result<Vec<Value>, StoreError> {
    let Some(own) = own else {
        return Ok(Vec::new());
    };
    let rows = all_letters(store)?;
    Ok(rows
        .into_iter()
        .filter(|(_, mailbox, body, _)| *mailbox != own.0 && sender_mailbox(body) == Some(own))
        .map(|(id, mailbox, body, deleted_at)| letter_json(id, mailbox, &body, deleted_at))
        .collect())
}

fn sent_letter_ids(
    store: &SqliteStore,
    own: Option<MailboxNumber>,
) -> Result<Vec<u32>, StoreError> {
    let Some(own) = own else {
        return Ok(Vec::new());
    };
    let rows = all_letters(store)?;
    Ok(rows
        .into_iter()
        .filter(|(_, mailbox, body, deleted_at)| {
            deleted_at.is_none() && *mailbox != own.0 && sender_mailbox(body) == Some(own)
        })
        .map(|(id, _, _, _)| id)
        .collect())
}

/// One stored letter or post: id, mailbox or board, body, and `deleted_at`.
type HeldBody = (u32, u32, Vec<u8>, Option<i64>);

fn all_letters(store: &SqliteStore) -> Result<Vec<HeldBody>, StoreError> {
    let connection = store.connection();
    let mut statement = connection
        .prepare("SELECT id, mailbox, body, deleted_at FROM letters ORDER BY id")
        .map_err(StoreError::backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(StoreError::backend)?;
    rows.collect::<Result<_, _>>().map_err(StoreError::backend)
}

pub(crate) fn sender_mailbox(body: &[u8]) -> Option<MailboxNumber> {
    Envelope::from_stored_body(body).map(|(_, envelope)| envelope.from)
}

fn letter_json(id: u32, mailbox: u32, body: &[u8], deleted_at: Option<i64>) -> Value {
    let parsed = Envelope::from_stored_body(body);
    json!({
        "id": id,
        "mailbox": mailbox,
        "from_mailbox": parsed.as_ref().map(|(_, envelope)| envelope.from.0),
        "sender": parsed.as_ref().map(|(_, envelope)| c_string(&envelope.sender)),
        "addressee": parsed.as_ref().map(|(_, envelope)| c_string(&envelope.addressee)),
        "subject": parsed.as_ref().map(|(_, envelope)| c_string(&envelope.subject)),
        "text": parsed.as_ref().map(|(_, envelope)| c_string(envelope.text.as_slice())),
        "received": parsed.as_ref().map(|(time, _)| time_json(*time)),
        "deleted_at": deleted_at,
        "body_base64": BASE64.encode(body),
    })
}

fn authored_posts_json(store: &SqliteStore, persona: &str) -> Result<Vec<Value>, StoreError> {
    Ok(all_posts(store)?
        .into_iter()
        .filter(|(_, _, body, _)| post_is_by(body, persona))
        .map(|(id, board, body, deleted_at)| {
            json!({
                "id": id,
                "board": board,
                "body": String::from_utf8_lossy(trim_nul(&body)).into_owned(),
                "deleted_at": deleted_at,
                "body_base64": BASE64.encode(&body),
            })
        })
        .collect())
}

fn authored_post_ids(store: &SqliteStore, persona: &str) -> Result<Vec<u32>, StoreError> {
    Ok(all_posts(store)?
        .into_iter()
        .filter(|(_, _, body, deleted_at)| deleted_at.is_none() && post_is_by(body, persona))
        .map(|(id, _, _, _)| id)
        .collect())
}

fn all_posts(store: &SqliteStore) -> Result<Vec<HeldBody>, StoreError> {
    let connection = store.connection();
    let mut statement = connection
        .prepare("SELECT id, board, body, deleted_at FROM posts ORDER BY id")
        .map_err(StoreError::backend)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(StoreError::backend)?;
    rows.collect::<Result<_, _>>().map_err(StoreError::backend)
}

/// Posts are `author|text`. Session 40/4 names are not stored, so only the Login persona matches.
pub(crate) fn post_is_by(body: &[u8], persona: &str) -> bool {
    if persona.is_empty() {
        return false;
    }
    post_author(body).is_some_and(|author| author.eq_ignore_ascii_case(persona))
}

pub(crate) fn post_author(body: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(trim_nul(body)).ok()?;
    let author = text.split('|').next()?.trim();
    (!author.is_empty()).then_some(author)
}

fn trim_nul(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    bytes.get(..end).unwrap_or(bytes)
}

fn c_string(bytes: &[u8]) -> String {
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
        "SELECT path, version, hash, length(data), updated_at, current, deleted_at, data \
         FROM saves WHERE account = ?1 ORDER BY path, version"
    } else {
        "SELECT path, version, hash, length(data), updated_at, current, deleted_at FROM saves \
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

fn count_saves(store: &SqliteStore, id: AccountId) -> Result<usize, StoreError> {
    let connection = store.connection();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM saves WHERE account = ?1",
            [id.0],
            |row| row.get(0),
        )
        .map_err(StoreError::backend)?;
    Ok(usize::try_from(count).unwrap_or(0))
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
    let purge = dates::format_date(dates::add_days(dates::unix_now(), policy.account_days));
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
        "  {} board posts matching the stored persona would be hidden",
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
mod tests {
    use super::*;
    use innkeeper_world::{AccountRecord, BoardId, EncodedPassword, Money};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "innkeeperd-subject-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn store(&self) -> SqliteStore {
            SqliteStore::open(&self.0.join(DATABASE_FILE)).unwrap()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    use std::time::{SystemTime, UNIX_EPOCH};

    const ALICE: AccountId = AccountId(100_001);
    const BOB: AccountId = AccountId(100_002);

    fn seeded() -> (TempDir, SqliteStore, MailboxNumber, MailboxNumber) {
        let dir = TempDir::new("db");
        let store = dir.store();
        store
            .create_account(
                ALICE,
                AccountRecord::new(EncodedPassword([7; 10]), "guybrush"),
            )
            .unwrap();
        store
            .create_account(BOB, AccountRecord::new(EncodedPassword([8; 10]), "elaine"))
            .unwrap();
        let alice_box = store.assign_mailbox(ALICE).unwrap();
        let bob_box = store.assign_mailbox(BOB).unwrap();
        store
            .append_letter(
                alice_box,
                &letter_body(bob_box.0, "elaine", "guybrush", "hi", "from bob"),
            )
            .unwrap();
        store
            .append_letter(
                bob_box,
                &letter_body(alice_box.0, "guybrush", "elaine", "yo", "from alice"),
            )
            .unwrap();
        store
            .append_letter(
                bob_box,
                &letter_body(bob_box.0, "elaine", "elaine", "note", "keep me"),
            )
            .unwrap();
        store.post(BoardId(1), b"guybrush|a tavern tale").unwrap();
        store.post(BoardId(1), b"elaine|not alice").unwrap();
        store.set_money(ALICE, Money(247)).unwrap();
        store
            .save_write(ALICE, "INN\\PLAYER.DIR", b"save-bytes", 9, true)
            .unwrap();
        (dir, store, alice_box, bob_box)
    }

    fn letter_body(from: u32, sender: &str, addressee: &str, subject: &str, text: &str) -> Vec<u8> {
        let mut body = vec![96, 0, 1, 12, 0, 0];
        body.extend([0u8; 10]);
        body.extend((from as u16).to_le_bytes());
        body.extend(((from >> 16) as u16).to_le_bytes());
        body.extend(padded(subject));
        body.extend(0u16.to_le_bytes());
        body.extend(padded(addressee));
        body.extend(padded(sender));
        body.extend(text.as_bytes());
        body.push(0);
        body
    }

    fn padded(text: &str) -> [u8; 10] {
        let mut field = [0u8; 10];
        let bytes = text.as_bytes();
        let n = bytes.len().min(10);
        field[..n].copy_from_slice(&bytes[..n]);
        field
    }

    #[test]
    fn export_omits_the_password_and_lists_mail_posts_money_and_saves() {
        let (_dir, store, alice_box, bob_box) = seeded();
        let document = export_account(&store, ALICE, true).unwrap();
        assert_eq!(
            document["encoded_password"],
            "omitted: the encoded password is not exported"
        );
        assert!(document["account"].get("password").is_none());
        assert_eq!(document["account"]["id"], 100_001);
        assert_eq!(document["account"]["persona"], "guybrush");
        assert_eq!(document["mailbox"], alice_box.0);
        assert_eq!(document["letters_received"].as_array().unwrap().len(), 1);
        assert_eq!(document["letters_sent"].as_array().unwrap().len(), 1);
        assert_eq!(document["letters_sent"][0]["mailbox"], bob_box.0);
        assert_eq!(document["letters_sent"][0]["text"], "from alice");
        assert_eq!(document["posts"].as_array().unwrap().len(), 1);
        assert!(document["posts"][0]["body"]
            .as_str()
            .unwrap()
            .starts_with("guybrush|"));
        assert_eq!(document["money"], 247);
        assert_eq!(document["saves"].as_array().unwrap().len(), 1);
        assert_eq!(document["saves"][0]["size"], 10);
        let data = BASE64
            .decode(document["saves"][0]["data_base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(data, b"save-bytes");
        assert!(!serde_json::to_string(&document).unwrap().contains("070707"));
    }

    #[test]
    fn export_without_saves_lists_sizes_only() {
        let (_dir, store, _, _) = seeded();
        let document = export_account(&store, ALICE, false).unwrap();
        assert!(document["saves"][0].get("data_base64").is_none());
        assert_eq!(document["saves"][0]["size"], 10);
    }

    #[test]
    fn delete_removes_the_account_and_what_belongs_to_it() {
        let (_dir, store, _, bob_box) = seeded();
        let plan = deletion_plan(&store, ALICE).unwrap();
        assert_eq!(plan.letters_in_box, 1);
        assert_eq!(plan.letters_sent, 1);
        assert_eq!(plan.posts, 1);
        assert_eq!(plan.money, Some(247));
        assert_eq!(plan.save_versions, 1);
        apply_delete(&store, ALICE, &plan, RetentionPolicy::default()).unwrap();
        assert!(store.account(ALICE).unwrap().is_none());
        assert!(store
            .account_any(ALICE)
            .unwrap()
            .unwrap()
            .deleted_at
            .is_some());
        let document = export_account(&store, ALICE, false).unwrap();
        assert!(document["account"]["deleted_at"].is_number());
        assert!(document["letters_received"][0]["deleted_at"].is_number());
        assert!(document["posts"][0]["deleted_at"].is_number());
        assert!(document["money_deleted_at"].is_number());
        assert_eq!(document["money"], 247);
        assert_eq!(store.money(ALICE).unwrap(), None);
        assert!(store.saves(ALICE).unwrap().is_empty());
        let bob_letters = store.letters(bob_box).unwrap();
        assert_eq!(bob_letters.len(), 1);
        let kept = Envelope::from_stored_body(&bob_letters[0].body).unwrap();
        assert_eq!(c_string(&kept.1.text), "keep me");
        assert_eq!(store.posts(BoardId(1)).unwrap().len(), 1);
        assert!(store.account(BOB).unwrap().is_some());
    }

    #[test]
    fn export_of_an_unknown_account_fails() {
        let (_dir, store, _, _) = seeded();
        let error = export_account(&store, AccountId(9), false).unwrap_err();
        assert!(matches!(error, SubjectError::Unknown(9)));
    }

    #[test]
    fn a_live_lock_file_is_refused() {
        let dir = TempDir::new("lock");
        std::fs::write(dir.0.join(LOCK_FILE), format!("{}\n", std::process::id())).unwrap();
        assert!(matches!(
            refuse_if_running(&dir.0).unwrap_err(),
            SubjectError::ServerRunning { .. }
        ));
    }
}
