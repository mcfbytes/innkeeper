//! `innkeeperd db reset` and `innkeeperd db check`: the account database file as a whole
//! (docs/server/innkeeperd.md section 5, docs/server/store.md section 3).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::config::DbAction;
use crate::dates;
use crate::server::DATABASE_FILE;
use crate::sqlite_store::{SqliteStore, SCHEMA_VERSION};
use crate::subject::running_pid;

/// The files SQLite keeps beside a database while it is open or after a crash. A reset moves them
/// with the database so the kept copy stays whole.
const COMPANIONS: [&str; 3] = ["-journal", "-wal", "-shm"];

pub(crate) fn run(action: DbAction) -> ExitCode {
    let result = match action {
        DbAction::Reset { data_dir } => reset(&data_dir),
        DbAction::Check { data_dir } => check(&data_dir),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Renames the database to `innkeeper-<timestamp>.db.old`, never deleting it, then creates a fresh
/// one at this build's schema.
fn reset(data_dir: &Path) -> Result<ExitCode, String> {
    if let Some(pid) = running_pid(data_dir) {
        return Err(format!(
            "innkeeperd is running (pid {pid}); stop the server first"
        ));
    }
    if !data_dir.is_dir() {
        return Err(format!("{} is not a directory", data_dir.display()));
    }
    let database = data_dir.join(DATABASE_FILE);
    let kept = match database.exists() {
        true => Some(keep_old(data_dir, &database)?),
        false => None,
    };
    SqliteStore::open(&database)
        .map_err(|error| format!("cannot create {}: {error}", database.display()))?;
    match kept {
        Some(old) => println!(
            "Kept the old database as {}. Created a fresh {} at schema version {SCHEMA_VERSION}.",
            old.display(),
            database.display()
        ),
        None => println!(
            "There was no database. Created {} at schema version {SCHEMA_VERSION}.",
            database.display()
        ),
    }
    Ok(ExitCode::SUCCESS)
}

/// The free name `innkeeper-<stamp>.db.old` (with `-2`, `-3` before `.db.old` when a reset in the
/// same second already took it), and the database and its companions moved there.
fn keep_old(data_dir: &Path, database: &Path) -> Result<PathBuf, String> {
    let stamp = dates::format_stamp(dates::now_ms());
    let mut kept = data_dir.join(format!("innkeeper-{stamp}.db.old"));
    let mut n = 2;
    while kept.exists() || companion(&kept, "-journal").exists() {
        kept = data_dir.join(format!("innkeeper-{stamp}-{n}.db.old"));
        n += 1;
    }
    // Companions first, so a failure leaves the database where SQLite looks for its journal.
    for suffix in COMPANIONS {
        let from = companion(database, suffix);
        if from.exists() {
            let to = companion(&kept, suffix);
            std::fs::rename(&from, &to)
                .map_err(|error| format!("cannot move {}: {error}", from.display()))?;
        }
    }
    std::fs::rename(database, &kept)
        .map_err(|error| format!("cannot move {}: {error}", database.display()))?;
    Ok(kept)
}

fn companion(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn check(data_dir: &Path) -> Result<ExitCode, String> {
    let database = data_dir.join(DATABASE_FILE);
    if !database.exists() {
        return Err(format!("{} has no {DATABASE_FILE}", data_dir.display()));
    }
    let store = SqliteStore::open(&database)
        .map_err(|error| format!("cannot open {}: {error}", database.display()))?;
    let report = store
        .check()
        .map_err(|error| format!("the check could not run: {error}"))?;
    println!(
        "schema version {} (this build {SCHEMA_VERSION})",
        report.version
    );
    println!(
        "foreign keys: {}",
        if report.foreign_keys_on { "on" } else { "OFF" }
    );
    match report.integrity.as_slice() {
        [] => println!("integrity_check: ok"),
        lines => {
            println!("integrity_check: {} problems", lines.len());
            for line in lines {
                println!("  {line}");
            }
        }
    }
    match report.foreign_keys.as_slice() {
        [] => println!("foreign_key_check: ok"),
        rows => {
            println!(
                "foreign_key_check: {} rows without their parent",
                rows.len()
            );
            for row in rows {
                println!("  {row}");
            }
        }
    }
    Ok(match report.clean() {
        true => ExitCode::SUCCESS,
        false => ExitCode::FAILURE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use rusqlite::Connection;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            let path =
                std::env::temp_dir().join(format!("innkeeperd-db-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn old_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".db.old"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_version_4_database_is_refused_and_reset_keeps_it_beside_a_fresh_one() {
        let dir = TempDir::new();
        let database = dir.0.join(DATABASE_FILE);
        let old = Connection::open(&database).unwrap();
        old.execute_batch(
            "CREATE TABLE accounts (id INTEGER PRIMARY KEY, persona TEXT NOT NULL);
             INSERT INTO accounts VALUES (100001, 'guybrush');
             PRAGMA user_version = 4;",
        )
        .unwrap();
        drop(old);
        let before = std::fs::read(&database).unwrap();
        let refused = SqliteStore::open(&database).unwrap_err().to_string();
        assert!(refused.contains("db reset --data-dir"), "{refused}");
        assert!(check(&dir.0).is_err(), "check refuses it too");

        assert_eq!(reset(&dir.0).unwrap(), ExitCode::SUCCESS);
        let kept = old_names(&dir.0);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].starts_with("innkeeper-") && kept[0].ends_with("Z.db.old"));
        assert_eq!(std::fs::read(dir.0.join(&kept[0])).unwrap(), before);
        let fresh = SqliteStore::open(&database).unwrap();
        assert!(fresh.check().unwrap().clean());
        assert_eq!(check(&dir.0).unwrap(), ExitCode::SUCCESS);

        // A second reset in the same second keeps both old files.
        drop(fresh);
        reset(&dir.0).unwrap();
        assert_eq!(old_names(&dir.0).len(), 2);
    }

    #[test]
    fn reset_refuses_while_the_server_runs() {
        let dir = TempDir::new();
        std::fs::write(
            dir.0.join(crate::server::LOCK_FILE),
            format!("{}\n", std::process::id()),
        )
        .unwrap();
        assert!(reset(&dir.0).unwrap_err().contains("running"));
        assert!(!dir.0.join(DATABASE_FILE).exists());
    }

    #[test]
    fn check_reports_a_row_without_its_parent() {
        let dir = TempDir::new();
        let database = dir.0.join(DATABASE_FILE);
        drop(SqliteStore::open(&database).unwrap());
        let raw = Connection::open(&database).unwrap();
        // A connection with foreign keys off can still write an orphan.
        raw.pragma_update(None, "foreign_keys", false).unwrap();
        raw.execute("INSERT INTO money (account, amount) VALUES (9, 1)", [])
            .unwrap();
        drop(raw);
        let report = SqliteStore::open(&database).unwrap().check().unwrap();
        assert_eq!(report.foreign_keys.len(), 1, "{:?}", report.foreign_keys);
        assert!(!report.clean());
        assert_eq!(check(&dir.0).unwrap(), ExitCode::FAILURE);
    }
}
