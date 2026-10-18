//! Linking and unlinking sign-ins against an in-memory store.

use super::*;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_world::{AccountId, AccountRecord, EncodedPassword, Store};

use crate::signin::keys::{HmacKey, KeyRing};
use crate::signin::store::{self, Created};

fn ring() -> KeyRing {
    KeyRing::new(vec![
        HmacKey::from_base64("v1", &BASE64.encode([1; 32])).unwrap()
    ])
    .unwrap()
}

fn identity(store: &SqliteStore, subject: &str, provider: &str) -> IdentityKey {
    store::sign_in(store, &ring(), "iss", subject, provider, || None, 10)
        .unwrap()
        .key
}

fn account(store: &SqliteStore, key: &IdentityKey, now: i64) -> AccountId {
    let record = AccountRecord::new(EncodedPassword([3; 10]), "");
    match store::create_owned_account(store, key, &record, now).unwrap() {
        Created::Account(id) => id,
        Created::PersonaTaken => panic!("no persona was given"),
    }
}

fn groups(store: &SqliteStore) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM identity_groups", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_new_identity_joins_the_group_with_accounts_and_both_see_them() {
    let store = SqliteStore::open_in_memory().unwrap();
    let alice = identity(&store, "alice", "microsoft");
    let first = account(&store, &alice, 10);
    let google = identity(&store, "alice-g", "google");
    assert_eq!(groups(&store), 2);
    assert_eq!(link_check(&store, &alice, &google).unwrap(), Linked::Joined);
    assert_eq!(link(&store, &alice, &google, 20).unwrap(), Linked::Joined);
    assert_eq!(groups(&store), 1, "the empty group is gone");
    assert!(store::owns(&store, &google, first).unwrap());
    // A later account of either is the other's too, and the daily cap counts them together.
    let second = account(&store, &google, 30);
    assert!(store::owns(&store, &alice, second).unwrap());
    assert_eq!(store::accounts_created_since(&store, &alice, 0).unwrap(), 2);
    let listed = methods(&store, &google).unwrap();
    let providers: Vec<&str> = listed.iter().map(|m| m.provider.as_str()).collect();
    assert_eq!(providers, ["microsoft", "google"]);
    assert_eq!(listed[1].linked_at, 20);
    assert_eq!(listed[0].id().len(), 16);
    assert_eq!(link(&store, &google, &alice, 40).unwrap(), Linked::Already);
    let mut keys = group_keys(&store, &alice).unwrap();
    keys.sort();
    let mut both = vec![alice, google];
    both.sort();
    assert_eq!(keys, both);
}

#[test]
fn either_order_works_and_the_session_without_accounts_joins() {
    let store = SqliteStore::open_in_memory().unwrap();
    let old = identity(&store, "old", "github");
    let id = account(&store, &old, 10);
    let fresh = identity(&store, "fresh", "discord");
    assert_eq!(link(&store, &fresh, &old, 20).unwrap(), Linked::Joined);
    assert!(store::owns(&store, &fresh, id).unwrap());
    let none = identity(&store, "a", "dev");
    let other = identity(&store, "b", "dev");
    assert_eq!(link(&store, &none, &other, 30).unwrap(), Linked::Joined);
    assert_eq!(methods(&store, &other).unwrap().len(), 2);
}

#[test]
fn two_groups_with_accounts_are_never_merged() {
    let store = SqliteStore::open_in_memory().unwrap();
    let alice = identity(&store, "alice", "dev");
    let bob = identity(&store, "bob", "dev");
    let hers = account(&store, &alice, 10);
    let his = account(&store, &bob, 10);
    // A soft-deleted account still counts: its link lasts until the purge.
    store.retire_account(his).unwrap();
    assert_eq!(link_check(&store, &alice, &bob).unwrap(), Linked::InUse);
    assert_eq!(link(&store, &alice, &bob, 20).unwrap(), Linked::InUse);
    assert!(!store::owns(&store, &bob, hers).unwrap());
    assert_eq!(groups(&store), 2);
}

#[test]
fn a_group_holds_at_most_the_cap() {
    let store = SqliteStore::open_in_memory().unwrap();
    let first = identity(&store, "s0", "dev");
    for n in 1..MAX_GROUP_IDENTITIES {
        let next = identity(&store, &format!("s{n}"), "dev");
        assert_eq!(link(&store, &first, &next, 20).unwrap(), Linked::Joined);
    }
    let extra = identity(&store, "extra", "dev");
    assert_eq!(link(&store, &first, &extra, 20).unwrap(), Linked::TooMany);
    let unknown = [0x42; 32];
    assert_eq!(link(&store, &first, &unknown, 20).unwrap(), Linked::Missing);
}

#[test]
fn unlink_refuses_the_last_and_the_current_sign_in_and_a_suspended_group() {
    let store = SqliteStore::open_in_memory().unwrap();
    let alice = identity(&store, "alice", "microsoft");
    let id = account(&store, &alice, 10);
    let alone = method_id(&alice);
    assert_eq!(unlink(&store, &alice, &alone, 20).unwrap(), Unlinked::Last);
    let google = identity(&store, "alice-g", "google");
    link(&store, &alice, &google, 20).unwrap();
    assert_eq!(
        unlink(&store, &alice, &alone, 30).unwrap(),
        Unlinked::Current
    );
    assert_eq!(
        unlink(&store, &alice, "0000", 30).unwrap(),
        Unlinked::Missing
    );
    store
        .connection()
        .execute("UPDATE accounts SET suspended_at = 1 WHERE id = ?1", [id.0])
        .unwrap();
    let theirs = method_id(&google);
    assert_eq!(
        unlink(&store, &alice, &theirs, 30).unwrap(),
        Unlinked::Suspended
    );
    assert!(store::owns_suspended(&store, &google).unwrap());
}

#[test]
fn an_unlinked_identity_owns_nothing_and_its_sessions_end() {
    let store = SqliteStore::open_in_memory().unwrap();
    let alice = identity(&store, "alice", "microsoft");
    let id = account(&store, &alice, 10);
    let google = identity(&store, "alice-g", "google");
    link(&store, &alice, &google, 20).unwrap();
    store::create_session(&store, &google, &[4; 32], 20, 1_000).unwrap();
    let done = unlink(&store, &alice, &method_id(&google), 30).unwrap();
    assert_eq!(done, Unlinked::Done);
    assert!(!store::owns(&store, &google, id).unwrap());
    assert!(store::owns(&store, &alice, id).unwrap());
    assert_eq!(store::session_identity(&store, &[4; 32], 31).unwrap(), None);
    assert_eq!(methods(&store, &alice).unwrap().len(), 1);
    assert_eq!(methods(&store, &google).unwrap()[0].linked_at, 30);
}
