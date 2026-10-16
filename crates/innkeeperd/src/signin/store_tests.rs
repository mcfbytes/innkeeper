//! The sign-in tables against an in-memory store.

use super::*;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_world::{EncodedPassword, Store};

use crate::signin::keys::HmacKey;

fn ring(versions: &[(&str, u8)]) -> KeyRing {
    let keys = versions
        .iter()
        .map(|(name, byte)| HmacKey::from_base64(name, &BASE64.encode([*byte; 32])).unwrap())
        .collect();
    KeyRing::new(keys).unwrap()
}

fn record(persona: &str) -> AccountRecord {
    AccountRecord::new(EncodedPassword([3; 10]), persona)
}

#[test]
fn a_second_sign_in_finds_the_same_identity() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keys = ring(&[("v1", 1)]);
    let first = sign_in(&store, &keys, "iss", "sub", "p", || None, 10).unwrap();
    let again = sign_in(&store, &keys, "iss", "sub", "p", || None, 20).unwrap();
    assert!(first.created);
    assert!(!again.created);
    assert_eq!(first.key, again.key);
    let other = sign_in(&store, &keys, "iss", "other", "p", || None, 20).unwrap();
    assert_ne!(other.key, first.key);
}

#[test]
fn rotation_moves_an_old_row_and_its_links_to_the_current_key() {
    let store = SqliteStore::open_in_memory().unwrap();
    let old = ring(&[("v1", 1)]);
    let first = sign_in(&store, &old, "iss", "sub", "p", || None, 10).unwrap();
    let Created::Account(id) =
        create_owned_account(&store, &first.key, &record("Elaine"), 10).unwrap()
    else {
        panic!("created");
    };
    create_session(&store, &first.key, &[9; 32], 10, 100).unwrap();
    let rotated = ring(&[("v2", 2), ("v1", 1)]);
    let moved = sign_in(&store, &rotated, "iss", "sub", "p", || None, 20).unwrap();
    assert!(!moved.created);
    assert_ne!(moved.key, first.key);
    assert!(owns(&store, &moved.key, id).unwrap());
    assert_eq!(
        session_identity(&store, &[9; 32], 50).unwrap(),
        Some(moved.key)
    );
    let version: String = store
        .connection()
        .query_row("SELECT key_version FROM identities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "v2");
}

#[test]
fn sessions_expire_and_revoke() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keys = ring(&[("v1", 1)]);
    let key = sign_in(&store, &keys, "iss", "sub", "p", || None, 10)
        .unwrap()
        .key;
    create_session(&store, &key, &[1; 32], 10, 100).unwrap();
    assert_eq!(session_identity(&store, &[1; 32], 99).unwrap(), Some(key));
    assert_eq!(session_identity(&store, &[1; 32], 100).unwrap(), None);
    assert_eq!(session_identity(&store, &[2; 32], 50).unwrap(), None);
    revoke_session(&store, &[1; 32], 50).unwrap();
    assert_eq!(session_identity(&store, &[1; 32], 51).unwrap(), None);
}

#[test]
fn web_accounts_are_numbered_after_the_highest_and_personas_are_unique() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keys = ring(&[("v1", 1)]);
    let key = sign_in(&store, &keys, "iss", "sub", "p", || None, 10)
        .unwrap()
        .key;
    store
        .create_account(AccountId(100_005), record("guybrush"))
        .unwrap();
    let created = create_owned_account(&store, &key, &record("Elaine"), 10).unwrap();
    assert_eq!(created, Created::Account(AccountId(100_006)));
    let clash = create_owned_account(&store, &key, &record("GUYBRUSH"), 10).unwrap();
    assert_eq!(clash, Created::PersonaTaken);
    assert_eq!(
        owned_accounts(&store, &key).unwrap(),
        vec![(100_006, "Elaine".to_owned())]
    );
    assert!(!owns(&store, &key, AccountId(100_005)).unwrap());
    assert_eq!(accounts_created_since(&store, &key, 9).unwrap(), 1);
    assert_eq!(accounts_created_since(&store, &key, 10).unwrap(), 0);
    store.retire_account(AccountId(100_006)).unwrap();
    assert!(owned_accounts(&store, &key).unwrap().is_empty());
    assert!(!owns(&store, &key, AccountId(100_006)).unwrap());
}

#[test]
fn an_empty_store_starts_web_accounts_at_100001() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keys = ring(&[("v1", 1)]);
    let key = sign_in(&store, &keys, "iss", "sub", "p", || None, 10)
        .unwrap()
        .key;
    let created = create_owned_account(&store, &key, &record("Elaine"), 10).unwrap();
    assert_eq!(created, Created::Account(AccountId(100_001)));
}

#[test]
fn attestation_is_kept_on_the_identity() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keys = ring(&[("v1", 1)]);
    let key = sign_in(&store, &keys, "iss", "sub", "p", || None, 10)
        .unwrap()
        .key;
    assert_eq!(identity(&store, &key).unwrap().unwrap().attested, None);
    let attested = Attested {
        age: 18,
        terms: 2,
        on: "2026-10-16".into(),
    };
    record_attestation(&store, &key, &attested).unwrap();
    let row = identity(&store, &key).unwrap().unwrap();
    assert_eq!(row.attested, Some(attested));
    assert_eq!(row.provider, "p");
}

#[test]
fn export_names_the_identity_without_its_subject_or_tokens() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keys = ring(&[("v1", 1)]);
    let sealed = || Some(b"{\"v\":1}".to_vec());
    let key = sign_in(&store, &keys, "iss", "sub", "dev", sealed, 10)
        .unwrap()
        .key;
    let Created::Account(id) = create_owned_account(&store, &key, &record("Elaine"), 10).unwrap()
    else {
        panic!("created");
    };
    create_session(&store, &key, &[7; 32], 10, 100).unwrap();
    let exported = export_identity(&store, id).unwrap();
    assert_eq!(exported["provider"], "dev");
    assert_eq!(exported["accounts"], json!([id.0]));
    assert_eq!(exported["sessions"][0]["expiry"], 100);
    assert_eq!(exported["subject"], SUBJECT_SEALED);
    let text = exported.to_string();
    assert!(!text.contains("sub\""), "{text}");
    assert!(!text.contains("0707070707"), "{text}");
    assert_eq!(export_identity(&store, AccountId(9)).unwrap(), Value::Null);
}
