//! `innkeeperd admin allow|revoke|list`: the `staff` table, edited offline or while the server runs
//! (SQLite serialises the writers). Docs: docs/server/innkeeperd.md section 9.

use std::path::Path;
use std::process::ExitCode;

use crate::admin::allowlist::{hex, parse_key};
use crate::admin::store;
use crate::config::AdminAction;
use crate::dates;
use crate::server::DATABASE_FILE;
use crate::signin::keys::handle;
use crate::sqlite_store::SqliteStore;

pub(crate) fn run(action: AdminAction) -> ExitCode {
    match run_action(action) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("{why}");
            ExitCode::FAILURE
        }
    }
}

fn open(data_dir: &Path) -> Result<SqliteStore, String> {
    if !data_dir.join(DATABASE_FILE).exists() {
        return Err(format!(
            "{} has no {DATABASE_FILE}; check --data-dir",
            data_dir.display()
        ));
    }
    SqliteStore::open(&data_dir.join(DATABASE_FILE))
        .map_err(|error| format!("cannot open the account database: {error}"))
}

fn run_action(action: AdminAction) -> Result<(), String> {
    let now = dates::now_ms();
    match action {
        AdminAction::Allow {
            identity_key,
            note,
            data_dir,
        } => {
            let key = parse_key(identity_key.trim())
                .ok_or("an identity key is 64 hex digits, as the refusal page shows it")?;
            let note = note.map(|text| text.chars().take(80).collect::<String>());
            let store = open(&data_dir)?;
            store::staff_allow(&store, &key, note.as_deref(), now)
                .map_err(|error| error.to_string())?;
            println!("{} may enter the portal (staff table).", handle(&key));
            Ok(())
        }
        AdminAction::Revoke {
            identity_key,
            data_dir,
        } => {
            let key = parse_key(identity_key.trim())
                .ok_or("an identity key is 64 hex digits, as the refusal page shows it")?;
            let store = open(&data_dir)?;
            store::staff_revoke(&store, &key, now).map_err(|error| error.to_string())?;
            println!(
                "{} is shut out of the portal. A revocation beats the vault secret and the file; \
                 sessions it holds end on their next request.",
                handle(&key)
            );
            Ok(())
        }
        AdminAction::List { data_dir } => {
            let store = open(&data_dir)?;
            let rows = store::staff_list(&store).map_err(|error| error.to_string())?;
            if rows.is_empty() {
                println!("The staff table is empty.");
            }
            for (key, row) in rows {
                let state = match row.revoked_at {
                    Some(at) => format!("revoked {}", dates::format_date(at)),
                    None => "allowed".to_owned(),
                };
                println!(
                    "{}  {}  {}  added {}  {}",
                    hex(&key),
                    handle(&key),
                    state,
                    dates::format_date(row.added_at),
                    row.note.unwrap_or_default()
                );
            }
            Ok(())
        }
    }
}
