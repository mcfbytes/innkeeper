use innkeeper_world::{
    AccountId, AccountRecord, BoardId, EncodedPassword, MailboxNumber, Money, Store,
};
use rusqlite::params;

use super::*;
use crate::dates::{DAY, HOUR};
use crate::sqlite_store::foreign_key_violations;

const TAVERN: BoardId = BoardId(1 << 24 | 0x59 << 8 | 1);
const COMPLAINTS: BoardId = BoardId(0x63 << 8 | 3);

fn memory() -> SqliteStore {
    SqliteStore::open_in_memory().unwrap()
}

fn policy() -> RetentionPolicy {
    RetentionPolicy::default()
}

fn add_account(store: &SqliteStore, id: u32, persona: &str) -> MailboxNumber {
    store
        .create_account(
            AccountId(id),
            AccountRecord::new(EncodedPassword([1; 10]), persona),
        )
        .unwrap();
    store.assign_mailbox(AccountId(id)).unwrap()
}

fn count(store: &SqliteStore, sql: &str) -> i64 {
    store
        .connection()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

/// `PRAGMA foreign_key_check` finds nothing: no row lost its parent.
fn consistent(store: &SqliteStore, step: &str) {
    let violations = foreign_key_violations(&store.connection()).unwrap();
    assert!(violations.is_empty(), "after {step}: {violations:?}");
}

fn add_identity(store: &SqliteStore, key: u8, account: Option<u32>, now: i64) {
    let conn = store.connection();
    conn.execute(
        "INSERT INTO identities (identity_key, key_version, provider, created_at)
         VALUES (?1, 'v', 'p', ?2)",
        params![vec![key; 32], now - 7 * DAY],
    )
    .unwrap();
    if let Some(account) = account {
        conn.execute(
            "INSERT INTO identity_accounts (account, identity_key, created_at) VALUES (?1, ?2, ?3)",
            params![account, vec![key; 32], now],
        )
        .unwrap();
    }
}

fn add_session(store: &SqliteStore, token: u8, key: u8, account: Option<u32>, expiry: i64) {
    store
        .connection()
        .execute(
            "INSERT INTO sessions (token_hash, identity_key, account, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![vec![token; 32], vec![key; 32], account, expiry],
        )
        .unwrap();
}

fn add_operator(store: &SqliteStore, key: u8) {
    store
        .connection()
        .execute(
            "INSERT INTO operators (identity_key, first_seen_at) VALUES (?1, 1)",
            [vec![key; 32]],
        )
        .unwrap();
}

/// Every table through create, soft delete, a purge that a hold blocks, the release and the purge
/// that follows, with no orphan after any step. Alice is the account under test; Bob shares her
/// persona and must lose nothing.
#[test]
fn every_table_lives_through_delete_hold_release_and_purge_without_an_orphan() {
    let store = memory();
    let t0 = dates::now_ms();
    let alice_box = add_account(&store, 100_001, "twin");
    let bob_box = add_account(&store, 100_002, "twin");
    let (alice, bob) = (AccountId(100_001), AccountId(100_002));
    store.append_letter(alice_box, bob, b"to alice").unwrap();
    store.append_letter(bob_box, alice, b"to bob").unwrap();
    store.append_letter(bob_box, bob, b"bob's own").unwrap();
    store.post(TAVERN, alice, None, b"twin|alice").unwrap();
    store.post(TAVERN, bob, None, b"twin|bob").unwrap();
    store
        .post(COMPLAINTS, alice, Some(bob), b"twin|Complaint: about: twin")
        .unwrap();
    store.set_money(alice, Money(9)).unwrap();
    store.save_write(alice, "INN\\A", b"save", 1, true).unwrap();
    add_identity(&store, 0xA1, Some(100_001), t0);
    add_session(&store, 0xA2, 0xA1, Some(100_001), t0 + HOUR);
    add_operator(&store, 0x0F);
    {
        let conn = store.connection();
        conn.execute(
            "INSERT INTO audit_sessions (started_at, account, identity_key, sealed)
             VALUES (?1, 100001, ?2, x'00')",
            params![t0, vec![0xA1u8; 32]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO admin_audit (recorded_at, identity_key, action, target, account)
             VALUES (?1, ?2, 'account.delete', 'account 100001', 100001)",
            params![t0, vec![0x0Fu8; 32]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO admin_sessions (token_hash, identity_key, created_at, last_seen_at, expires_at)
             VALUES (?1, ?2, ?3, ?3, ?4)",
            params![vec![0x0Eu8; 32], vec![0x0Fu8; 32], t0, t0 + 8 * HOUR],
        )
        .unwrap();
    }
    consistent(&store, "create");

    assert!(matches!(
        disable_account(&store, alice.0).unwrap(),
        Disable::Fresh(_)
    ));
    consistent(&store, "soft delete");
    assert!(store.account(alice).unwrap().is_none());
    assert_eq!(store.posts(TAVERN).unwrap().len(), 1, "bob's stays");

    // Inside every window nothing goes.
    let soon = purge_expired(&store, policy(), t0 + DAY).unwrap();
    assert_eq!((soon.accounts, soon.letters, soon.posts), (0, 0, 0));
    consistent(&store, "an early purge");

    // A year and more later, a hold on alice keeps everything of hers.
    let late = t0 + 400 * DAY;
    place_hold(&store, alice.0, late + DAY).unwrap();
    let held = purge_expired(&store, policy(), late).unwrap();
    consistent(&store, "a held purge");
    assert_eq!((held.accounts, held.letters, held.complaints), (0, 0, 0));
    assert_eq!(
        held.admin_sessions, 1,
        "a portal cookie is not the account's to hold"
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM letters"), 3);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM audit_sessions"), 1);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM admin_audit"), 1);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM accounts WHERE purged_at IS NULL"
        ),
        2
    );

    clear_hold(&store, alice.0).unwrap();
    consistent(&store, "the release");
    let purged = purge_expired(&store, policy(), late).unwrap();
    consistent(&store, "the purge");
    assert_eq!(purged.accounts, 1);
    assert_eq!((purged.money, purged.saves, purged.mailboxes), (1, 1, 1));
    assert_eq!(purged.letters, 2, "the one in her box and the one she sent");
    assert_eq!((purged.posts, purged.complaints), (1, 1));
    assert_eq!(
        (purged.audit, purged.admin_audit, purged.sessions),
        (1, 1, 1)
    );
    assert_eq!((purged.identities, purged.operators), (1, 1));

    // Alice is a tombstone: her number and clocks, nothing personal, and the number is not reused.
    let row = store.account_any(alice).unwrap().unwrap();
    assert!(row.record.is_none());
    assert_eq!(row.purged_at, Some(late));
    assert!(matches!(
        store.create_account(alice, AccountRecord::new(EncodedPassword([2; 10]), "new")),
        Err(StoreError::AccountExists(_))
    ));
    // Bob, who shares her name, kept his account, box, letter and post.
    assert!(store.account(bob).unwrap().is_some());
    assert_eq!(store.letters(bob_box).unwrap().len(), 1);
    assert_eq!(store.posts(TAVERN).unwrap()[0].account, bob);
    // A second purge has nothing left to do and still leaves no orphan.
    let again = purge_expired(&store, policy(), late).unwrap();
    assert_eq!(again, PurgeCounts::default());
    consistent(&store, "a second purge");
}

#[test]
fn the_mailbox_of_a_purged_account_waits_for_its_letters() {
    let store = memory();
    let t0 = dates::now_ms();
    let mailbox = add_account(&store, 5, "cara");
    add_account(&store, 6, "dan");
    store.append_letter(mailbox, AccountId(6), b"note").unwrap();
    disable_account(&store, 5).unwrap();
    let first = purge_expired(&store, policy(), t0 + 31 * DAY).unwrap();
    consistent(&store, "the account purge");
    assert_eq!((first.accounts, first.mailboxes, first.letters), (1, 0, 0));
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM mailboxes WHERE account = 5"),
        1
    );
    let second = purge_expired(&store, policy(), t0 + 91 * DAY).unwrap();
    consistent(&store, "the letter purge");
    assert_eq!((second.letters, second.mailboxes), (1, 1));
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM accounts WHERE id = 5"),
        1
    );
}

#[test]
fn purge_drops_old_audit_rows_unless_they_are_held() {
    let store = memory();
    let now = dates::now_ms();
    add_account(&store, 1, "one");
    add_account(&store, 2, "two");
    let old = now - 91 * DAY;
    store
        .connection()
        .execute(
            "INSERT INTO audit_sessions
             (started_at, account, sealed) VALUES (?1, 1, x'00'), (?2, 1, x'00'), (?1, 2, x'00')",
            params![old, now],
        )
        .unwrap();
    place_hold(&store, 2, now + DAY).unwrap();
    let counts = purge_expired(&store, policy(), now).unwrap();
    assert_eq!(counts.audit, 1);
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM audit_sessions WHERE account = 1"
        ),
        1
    );
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM audit_sessions WHERE account = 2"
        ),
        1
    );
}

#[test]
fn a_deleted_letter_is_purged_after_the_window_unless_its_sender_is_held() {
    let store = memory();
    let now = dates::now_ms();
    add_account(&store, 1, "alice");
    let to = add_account(&store, 2, "bob");
    let id = store.append_letter(to, AccountId(1), b"hi").unwrap();
    assert!(store.delete_letter(to, id).unwrap());
    let later = now + 91 * DAY;
    place_hold(&store, 1, later + 10 * DAY).unwrap();
    let held = purge_expired(&store, policy(), later).unwrap();
    assert_eq!(held.letters, 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM letters"), 1);
    clear_hold(&store, 1).unwrap();
    let purged = purge_expired(&store, policy(), later).unwrap();
    assert_eq!(purged.letters, 1);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM letters"), 0);
}

#[test]
fn a_hold_covers_the_posts_its_account_wrote_and_not_a_same_named_accounts() {
    let store = memory();
    let now = dates::now_ms();
    add_account(&store, 8, "twin");
    add_account(&store, 9, "twin");
    let held = store
        .post(TAVERN, AccountId(8), None, b"twin|held")
        .unwrap();
    let free = store
        .post(TAVERN, AccountId(9), None, b"twin|free")
        .unwrap();
    store.delete_post(TAVERN, held).unwrap();
    store.delete_post(TAVERN, free).unwrap();
    let later = now + 91 * DAY;
    place_hold(&store, 8, later + DAY).unwrap();
    let counts = purge_expired(&store, policy(), later).unwrap();
    assert_eq!(counts.posts, 1);
    let left: Vec<u32> = store
        .connection()
        .prepare("SELECT id FROM posts ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(left, [held.0]);
}

#[test]
fn complaints_purge_from_arrival_and_a_hold_on_either_account_blocks_them() {
    let store = memory();
    let now = dates::now_ms();
    add_account(&store, 8, "mod");
    add_account(&store, 9, "rude");
    let ordinary = store
        .post(TAVERN, AccountId(8), None, b"mod|tavern")
        .unwrap();
    store.delete_post(TAVERN, ordinary).unwrap();
    store
        .post(COMPLAINTS, AccountId(8), Some(AccountId(9)), b"mod|abuse")
        .unwrap();
    let counts = purge_expired(&store, policy(), now + 91 * DAY).unwrap();
    assert_eq!((counts.posts, counts.complaints), (1, 0));
    let late = now + 366 * DAY;
    for held in [8, 9] {
        place_hold(&store, held, late + DAY).unwrap();
        let kept = purge_expired(&store, policy(), late).unwrap();
        assert_eq!(kept.complaints, 0, "a hold on {held}");
        clear_hold(&store, held).unwrap();
    }
    let gone = purge_expired(&store, policy(), late).unwrap();
    assert_eq!(gone.complaints, 1);
    consistent(&store, "the complaint purge");
}

#[test]
fn restore_undoes_only_the_account_deletion() {
    let store = memory();
    let now = dates::now_ms();
    let mailbox = add_account(&store, 4, "dee");
    let early = store
        .append_letter(mailbox, AccountId(4), b"early")
        .unwrap();
    let later = store
        .append_letter(mailbox, AccountId(4), b"later")
        .unwrap();
    store.delete_letter(mailbox, early).unwrap();
    store
        .connection()
        .execute("UPDATE letters SET deleted_at = 1 WHERE id = ?1", [early.0])
        .unwrap();
    disable_account(&store, 4).unwrap();
    assert!(matches!(
        restore_account(&store, 4, policy(), now).unwrap(),
        Restored::Ok
    ));
    let visible = store.letters(mailbox).unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, later);
    assert!(store.account(AccountId(4)).unwrap().is_some());
    consistent(&store, "the restore");
}

#[test]
fn purge_now_leaves_a_tombstone_and_keeps_sealed_records_and_complaints() {
    let store = memory();
    let now = dates::now_ms();
    let mailbox = add_account(&store, 4, "dee");
    add_account(&store, 5, "eve");
    store.append_letter(mailbox, AccountId(5), b"in").unwrap();
    store.post(TAVERN, AccountId(4), None, b"dee|post").unwrap();
    store
        .post(
            COMPLAINTS,
            AccountId(4),
            Some(AccountId(5)),
            b"dee|Complaint: x",
        )
        .unwrap();
    store
        .post(
            COMPLAINTS,
            AccountId(5),
            Some(AccountId(4)),
            b"eve|Complaint: about dee",
        )
        .unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO audit_sessions (started_at, account, sealed) VALUES (?1, 4, x'00')",
            [now],
        )
        .unwrap();
    place_hold(&store, 4, now + DAY).unwrap();
    assert!(matches!(purge_now(&store, 4, now).unwrap(), PurgeNow::Held));
    clear_hold(&store, 4).unwrap();
    assert!(matches!(
        purge_now(&store, 4, now).unwrap(),
        PurgeNow::Done {
            letters: 1,
            posts: 1,
            saves: 0
        }
    ));
    consistent(&store, "purge now");
    assert!(store.account(AccountId(4)).unwrap().is_none());
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM accounts WHERE id = 4 AND purged_at IS NOT NULL"
        ),
        1
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM audit_sessions"), 1);
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM posts WHERE kind = 'complaint'"
        ),
        2
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM mailboxes WHERE account = 4"),
        0
    );
    // A tombstone takes a hold, and a second purge-now has nothing to do.
    place_hold(&store, 4, now + DAY).unwrap();
    clear_hold(&store, 4).unwrap();
    assert!(matches!(
        purge_now(&store, 4, now).unwrap(),
        PurgeNow::Done { letters: 0, .. }
    ));
    assert!(matches!(
        purge_now(&store, 77, now).unwrap(),
        PurgeNow::Missing
    ));
}

#[test]
fn a_hold_needs_an_account_number_that_was_issued() {
    let store = memory();
    assert!(matches!(
        place_hold(&store, 42, dates::now_ms() + DAY),
        Err(StoreError::UnknownAccount(AccountId(42)))
    ));
    add_account(&store, 42, "x");
    let first = place_hold(&store, 42, 2_000).unwrap();
    assert_eq!(first, 2_000);
    assert_eq!(
        place_hold(&store, 42, 1_000).unwrap(),
        2_000,
        "never shortened"
    );
}

#[test]
fn the_highest_number_is_not_issued_again_after_it_is_purged() {
    use innkeeper_world::{
        ClientMessage, ConnectionId, HostMessage, ObjectStore, PlayerSession, Signup, World,
    };

    let store = std::sync::Arc::new(memory());
    add_account(&store, 100_001, "a");
    add_account(&store, 100_002, "b");
    disable_account(&store, 100_002).unwrap();
    purge_now(&store, 100_002, dates::now_ms()).unwrap();
    assert_eq!(
        store.highest_account_id().unwrap(),
        Some(AccountId(100_002))
    );
    let world = World::stored(store.clone());
    let mut player = PlayerSession::new();
    let signup = ClientMessage::Signup(Signup {
        cookie: 1,
        form: Vec::new(),
    });
    let replies = player.handle(&world, &mut ObjectStore::new(), ConnectionId(1), &signup);
    let Some(HostMessage::Ack(ack)) = replies.first().map(|delivery| &delivery.message) else {
        panic!("no signup ack: {replies:?}");
    };
    let issued = u32::from_le_bytes(ack.tail[..4].try_into().unwrap());
    assert_eq!(issued, 100_003);
}

#[test]
fn expired_session_rows_are_purged_unless_a_hold_covers_the_identity() {
    let store = memory();
    let now = 1_700_000_000_000;
    add_account(&store, 1, "one");
    add_account(&store, 2, "two");
    add_identity(&store, 1, Some(1), now);
    add_identity(&store, 2, Some(2), now);
    let old = now - 40 * DAY;
    add_session(&store, 1, 1, Some(1), old);
    add_session(&store, 2, 2, Some(2), old);
    add_session(&store, 3, 1, Some(1), now);
    add_session(&store, 4, 2, None, old);
    place_hold(&store, 2, now + DAY).unwrap();
    let counts = purge_expired(&store, policy(), now).unwrap();
    assert_eq!(counts.sessions, 1);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM sessions"), 3);
}

#[test]
fn an_identity_with_no_account_and_no_session_is_purged_and_sealed_rows_keep_theirs() {
    let store = memory();
    let now = 1_700_000_000_000;
    add_account(&store, 1, "one");
    add_identity(&store, 1, Some(1), now);
    add_identity(&store, 2, None, now);
    add_identity(&store, 3, None, now);
    add_session(&store, 1, 3, None, now + HOUR);
    store
        .connection()
        .execute(
            "INSERT INTO audit_sessions (started_at, identity_key, sealed) VALUES (?1, ?2, x'00')",
            params![now, vec![2u8; 32]],
        )
        .unwrap();
    let counts = purge_expired(&store, policy(), now).unwrap();
    assert_eq!(counts.identities, 1);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM identities"), 2);
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM audit_sessions WHERE identity_key IS NULL"
        ),
        1,
        "the sealed row stays and forgets the purged identity"
    );
    consistent(&store, "the identity purge");
}

#[test]
fn deleting_the_last_account_retires_the_identity_and_ends_its_sessions() {
    let store = memory();
    let now = dates::now_ms();
    add_account(&store, 1, "one");
    add_account(&store, 2, "two");
    add_identity(&store, 1, Some(1), now);
    store
        .connection()
        .execute(
            "INSERT INTO identity_accounts (account, identity_key, created_at) VALUES (2, ?1, 0)",
            [vec![1u8; 32]],
        )
        .unwrap();
    add_session(&store, 1, 1, None, now + DAY);
    disable_account(&store, 1).unwrap();
    let live = "SELECT COUNT(*) FROM identities WHERE deleted_at IS NULL";
    assert_eq!(count(&store, live), 1, "one account is still live");
    disable_account(&store, 2).unwrap();
    assert_eq!(count(&store, live), 0);
    let open = format!(
        "SELECT COUNT(*) FROM sessions WHERE expires_at > {}",
        now + HOUR
    );
    assert_eq!(count(&store, &open), 0);
    restore_account(&store, 2, policy(), dates::now_ms()).unwrap();
    assert_eq!(count(&store, live), 1);
}

#[test]
fn the_admin_audit_is_kept_a_year_unless_a_hold_covers_the_account() {
    let store = memory();
    let now = 1_800_000_000_000;
    add_account(&store, 1, "one");
    add_account(&store, 2, "two");
    add_operator(&store, 0);
    let old = now - 366 * DAY;
    let recent = now - 100 * DAY;
    store
        .connection()
        .execute(
            "INSERT INTO admin_audit (recorded_at, identity_key, action, target, account)
             VALUES (?1, zeroblob(32), 'a', 't', NULL), (?1, zeroblob(32), 'a', 't', 1),
                    (?1, zeroblob(32), 'a', 't', 2), (?2, zeroblob(32), 'a', 't', NULL)",
            params![old, recent],
        )
        .unwrap();
    place_hold(&store, 2, now + DAY).unwrap();
    let counts = purge_expired(&store, policy(), now).unwrap();
    assert_eq!(
        counts.admin_audit, 2,
        "the old entries of accounts 1 and none"
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM admin_audit WHERE account = 2"),
        1,
        "held"
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM admin_audit"), 2);
    clear_hold(&store, 2).unwrap();
    purge_expired(&store, policy(), now).unwrap();
    assert_eq!(count(&store, "SELECT COUNT(*) FROM admin_audit"), 1);
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM operators"),
        1,
        "still named"
    );
}

#[test]
fn expired_portal_cookies_and_unreferenced_operators_are_purged() {
    let store = memory();
    let now = 1_800_000_000_000;
    add_operator(&store, 0);
    add_operator(&store, 1);
    store
        .connection()
        .execute(
            "INSERT INTO admin_sessions (token_hash, identity_key, created_at, last_seen_at, expires_at)
             VALUES (?1, zeroblob(32), 1, 1, ?2), (?3, zeroblob(32), 1, 1, ?4)",
            params![vec![1u8; 32], now - 2 * DAY, vec![2u8; 32], now + 60_000],
        )
        .unwrap();
    let counts = purge_expired(&store, policy(), now).unwrap();
    assert_eq!(counts.admin_sessions, 1);
    assert_eq!(counts.operators, 1, "the one nothing names");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM admin_sessions"), 1);
    consistent(&store, "the portal purge");
}
