//! Logon transcripts against a stored account book; the reply bytes are in golden/logon_policy.txt.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use std::sync::Arc;

use innkeeper_world::{
    AccountBook, AccountId, AccountRecord, ClientMessage, ConnectionId, EncodedPassword, Enrolment,
    LoginStatus, MemoryStore, ObjectStore, PlayerSession, Store, World,
};

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

const LOGIN_OK: &str = "35 00 00 00 01 02 03 12 a1 86 01 00 01 07 07 07 07 07 07 07 07 07 07 00 67 75 79 62 72 75 73 68 00";
const LOGIN_WRONG: &str = "35 00 00 00 01 02 03 12 a1 86 01 00 01 08 08 08 08 08 08 08 08 08 08 00 67 75 79 62 72 75 73 68 00";
const LOGIN_LAND_9: &str = "35 00 00 00 09 02 03 12 a1 86 01 00 01 07 07 07 07 07 07 07 07 07 07 00 67 75 79 62 72 75 73 68 00";
const ACK_FLAGGED: &str = "00 00 00 00 16 86 00 0b 04 00";
const ACK_PLAIN: &str = "00 00 00 00 16 00 00 00 00 00";
const ACK_EXPIRED: &str = "00 00 00 00 16 00 00 0b 00 00";
const NAK_WRONG: &str = "01 00 00 00 16 09 00 03 54 68 61 74 20 70 61 73 73 77 6f 72 64 20 69 73 20 6e 6f 74 20 72 69 67 68 74 2e 00";
const NAK_LAND: &str = "01 00 00 00 16 02 00 00 54 68 69 73 20 6c 61 6e 64 20 69 73 20 6e 6f 74 20 6b 6e 6f 77 6e 20 68 65 72 65 2e 00";
const CHANGE_TO_5: &str = "2c 02 05 01 05 05 05 05 05 05 05 05 05 05";
const CHANGED_ACK: &str = "00 00 05 01 2c 01";
const PASSWORD_7: &str = "07 07 07 07 07 07 07 07 07 07";
const PASSWORD_5: &str = "05 05 05 05 05 05 05 05 05 05";

const ACCOUNT: AccountId = AccountId(100_001);

fn host(record: AccountRecord, enrolment: Enrolment) -> (World, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    store.create_account(ACCOUNT, record).unwrap();
    let mut world = World::stored(store.clone());
    world.accounts = AccountBook::stored(store.clone(), enrolment);
    (world, store)
}

fn member() -> AccountRecord {
    AccountRecord::new(EncodedPassword([7; 10]), "guybrush")
}

/// Sends each client message in turn and returns the encoded replies, in order.
fn transcript(world: &World, session: &mut PlayerSession, sends: &[&str]) -> Vec<Vec<u8>> {
    let mut objects = ObjectStore::new();
    let mut replies = Vec::new();
    for send in sends {
        let message = ClientMessage::parse(&hex(send)).unwrap();
        let deliveries = session.handle(world, &mut objects, ConnectionId(1), &message);
        replies.extend(deliveries.iter().map(|delivery| delivery.message.encode()));
    }
    replies
}

#[test]
fn a_wrong_password_then_the_right_one_logs_in_with_the_stored_flags() {
    let record = AccountRecord {
        user_flags: 0x0086,
        rating: 4,
        status: LoginStatus(0x0b),
        ..member()
    };
    let (world, _) = host(record, Enrolment::Closed);
    let replies = transcript(&world, &mut PlayerSession::new(), &[LOGIN_WRONG, LOGIN_OK]);
    assert_eq!(replies, [hex(NAK_WRONG), hex(ACK_FLAGGED)]);
}

#[test]
fn an_unlisted_land_type_is_refused_before_the_password_is_looked_at() {
    let (world, _) = host(member(), Enrolment::Closed);
    let replies = transcript(&world, &mut PlayerSession::new(), &[LOGIN_LAND_9]);
    assert_eq!(replies, [hex(NAK_LAND)]);
}

#[test]
fn command_44_stores_the_password_and_the_next_login_needs_it() {
    let (world, store) = host(member(), Enrolment::Closed);
    let mut session = PlayerSession::new();
    let replies = transcript(&world, &mut session, &[LOGIN_OK, CHANGE_TO_5]);
    assert_eq!(replies, [hex(ACK_PLAIN), hex(CHANGED_ACK)]);
    let kept = store.account(ACCOUNT).unwrap().unwrap();
    assert_eq!(kept.password, EncodedPassword([5; 10]));
    let again = transcript(&world, &mut PlayerSession::new(), &[LOGIN_OK]);
    assert_eq!(again, [hex(NAK_WRONG)]);
}

#[test]
fn an_expired_password_logs_in_with_status_11_until_command_44_replaces_it() {
    let record = AccountRecord {
        password_expired: true,
        ..member()
    };
    let (world, _) = host(record, Enrolment::Closed);
    let mut session = PlayerSession::new();
    let replies = transcript(&world, &mut session, &[LOGIN_OK]);
    assert_eq!(replies, [hex(ACK_EXPIRED)]);
    let _ = transcript(&world, &mut session, &[CHANGE_TO_5]);
    let relogin = LOGIN_OK.replace(PASSWORD_7, PASSWORD_5);
    let replies = transcript(&world, &mut PlayerSession::new(), &[&relogin]);
    assert_eq!(replies, [hex(ACK_PLAIN)]);
}

#[test]
fn an_unknown_account_is_enrolled_only_by_an_enrolling_book() {
    let store = Arc::new(MemoryStore::new());
    let world = World::stored(store.clone());
    let replies = transcript(&world, &mut PlayerSession::new(), &[LOGIN_OK]);
    assert_eq!(replies, [hex(ACK_PLAIN)]);
    assert!(store.account(ACCOUNT).unwrap().is_some());

    let (closed, _) = host(member(), Enrolment::Closed);
    let stranger = LOGIN_OK.replace("a1 86", "a2 86");
    let replies = transcript(&closed, &mut PlayerSession::new(), &[&stranger]);
    assert_eq!(replies[0][5], 1, "reason 1: unknown account");
}
