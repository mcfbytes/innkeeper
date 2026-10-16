use super::*;
use innkeeper_world::{AccountRecord, BoardId, EncodedPassword, Money};

use std::time::{SystemTime, UNIX_EPOCH};

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

const ALICE: AccountId = AccountId(100_001);
const BOB: AccountId = AccountId(100_002);
const TAVERN: BoardId = BoardId(1);
const COMPLAINTS: BoardId = BoardId(0x63 << 8 | 3);

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
            BOB,
            &letter_body(bob_box.0, "elaine", "guybrush", "hi", "from bob"),
        )
        .unwrap();
    store
        .append_letter(
            bob_box,
            ALICE,
            &letter_body(alice_box.0, "guybrush", "elaine", "yo", "from alice"),
        )
        .unwrap();
    store
        .append_letter(
            bob_box,
            BOB,
            &letter_body(bob_box.0, "elaine", "elaine", "note", "keep me"),
        )
        .unwrap();
    store
        .post(TAVERN, ALICE, None, b"guybrush|a tavern tale")
        .unwrap();
    store.post(TAVERN, BOB, None, b"elaine|not alice").unwrap();
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
    assert_eq!(document["letters_received"][0]["from_account"], BOB.0);
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

/// Two accounts that share the persona "twin", each with a post, a complaint and a letter.
fn twins() -> (TempDir, SqliteStore) {
    let dir = TempDir::new("twins");
    let store = dir.store();
    for (id, password) in [(ALICE, 7), (BOB, 8)] {
        store
            .create_account(
                id,
                AccountRecord::new(EncodedPassword([password; 10]), "twin"),
            )
            .unwrap();
        store.assign_mailbox(id).unwrap();
    }
    store
        .post(TAVERN, ALICE, None, b"twin|alice's tale")
        .unwrap();
    store.post(TAVERN, BOB, None, b"twin|bob's tale").unwrap();
    store
        .post(COMPLAINTS, ALICE, Some(BOB), b"twin|Complaint: about: twin")
        .unwrap();
    store
        .post(COMPLAINTS, BOB, None, b"twin|Complaint: bob's report")
        .unwrap();
    let bob_box = store.assign_mailbox(BOB).unwrap();
    let alice_box = store.assign_mailbox(ALICE).unwrap();
    // The envelope claims each other's box; only the recorded sender counts.
    store
        .append_letter(
            bob_box,
            ALICE,
            &letter_body(alice_box.0, "twin", "twin", "a", "alice wrote"),
        )
        .unwrap();
    store
        .append_letter(
            alice_box,
            BOB,
            &letter_body(alice_box.0, "twin", "twin", "b", "bob wrote"),
        )
        .unwrap();
    (dir, store)
}

fn texts(document: &Value, list: &str, field: &str) -> Vec<String> {
    let items = document[list].as_array().unwrap();
    items
        .iter()
        .map(|item| item[field].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn two_accounts_with_one_name_never_see_each_others_posts_complaints_or_letters() {
    let (_dir, store) = twins();
    let alice = export_account(&store, ALICE, false).unwrap();
    assert_eq!(
        texts(&alice, "posts", "body"),
        ["twin|alice's tale", "twin|Complaint: about: twin"]
    );
    assert_eq!(texts(&alice, "letters_sent", "text"), ["alice wrote"]);
    assert_eq!(texts(&alice, "letters_received", "text"), ["bob wrote"]);
    let bob = export_account(&store, BOB, false).unwrap();
    assert_eq!(
        texts(&bob, "posts", "body"),
        ["twin|bob's tale", "twin|Complaint: bob's report"]
    );
    assert_eq!(texts(&bob, "letters_sent", "text"), ["bob wrote"]);
    let plan = deletion_plan(&store, ALICE).unwrap();
    assert_eq!(
        (plan.posts, plan.letters_sent, plan.letters_in_box),
        (2, 1, 1)
    );
}

#[test]
fn deleting_one_of_two_same_named_accounts_hides_only_its_own_rows() {
    let (_dir, store) = twins();
    apply_delete(&store, ALICE, RetentionPolicy::default()).unwrap();
    let bodies: Vec<_> = store
        .posts(TAVERN)
        .unwrap()
        .into_iter()
        .map(|post| String::from_utf8(post.body).unwrap())
        .collect();
    assert_eq!(bodies, ["twin|bob's tale"]);
    let complaints = store.posts(COMPLAINTS).unwrap();
    assert_eq!(complaints.len(), 1);
    assert_eq!(complaints[0].account, BOB);
    let bob = export_account(&store, BOB, false).unwrap();
    assert!(bob["posts"][0]["deleted_at"].is_null());
    assert!(
        bob["letters_sent"][0]["deleted_at"].is_number(),
        "it was in alice's box"
    );
    assert!(store.account(BOB).unwrap().is_some());
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
    apply_delete(&store, ALICE, RetentionPolicy::default()).unwrap();
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
    assert_eq!(store.posts(TAVERN).unwrap().len(), 1);
    assert!(store.account(BOB).unwrap().is_some());
}

#[test]
fn a_purged_account_exports_as_a_tombstone_with_nothing_personal() {
    let (_dir, store, _, _) = seeded();
    apply_delete(&store, ALICE, RetentionPolicy::default()).unwrap();
    let now = dates::now_ms();
    retention::purge_now(&store, ALICE.0, now).unwrap();
    let document = export_account(&store, ALICE, true).unwrap();
    let account = document["account"].as_object().unwrap();
    let mut keys: Vec<_> = account.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["created_at", "deleted_at", "id", "purged_at"]);
    assert_eq!(document["mailbox"], Value::Null);
    assert!(document["letters_received"].as_array().unwrap().is_empty());
    assert!(document["letters_sent"].as_array().unwrap().is_empty());
    assert!(document["posts"].as_array().unwrap().is_empty());
    assert_eq!(document["money"], Value::Null);
    assert!(document["saves"].as_array().unwrap().is_empty());
    assert!(!serde_json::to_string(&document)
        .unwrap()
        .contains("guybrush"));
}

/// A complaint filed from the hub (27/0 board 0/99 and 27/9) is a post by the account, so the
/// export lists it and the delete removes it (privacy.md sections 1 and 6).
#[test]
fn complaints_filed_in_a_session_are_in_the_export_and_the_delete() {
    use innkeeper_world::{ClientMessage, ConnectionId, ObjectStore, PlayerSession, World};

    let dir = TempDir::new("complaint");
    let store = std::sync::Arc::new(dir.store());
    let password = [0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00];
    store
        .create_account(
            ALICE,
            AccountRecord::new(EncodedPassword(password), "guybrush"),
        )
        .unwrap();
    let world = World::stored(store.clone());
    let (mut objects, mut player) = (ObjectStore::new(), PlayerSession::new());
    let mut say = |bytes: &[u8]| {
        let message = ClientMessage::parse(bytes).unwrap();
        player.handle(&world, &mut objects, ConnectionId(1), &message)
    };
    let mut login = vec![0x35, 0, 0, 0, 1, 2, 3, 0x12, 0xa1, 0x86, 1, 0, 1];
    login.extend(password);
    login.extend(b"\0guybrush\0");
    assert_eq!(say(&login).len(), 1);
    let mut plain = vec![27, 0, 0, 0, 0, 1, 99, 0];
    plain.extend(b"Complaint|From: guybrush about: Mo\0");
    let mut named = vec![27, 9, 0, 0, 0, 1, 99, 0, 0x34, 0x12];
    named.extend(b"Complaint|From: guybrush about: Mo, rude\0");
    assert!(say(&plain).is_empty() && say(&named).is_empty());

    let document = export_account(&store, ALICE, false).unwrap();
    let posts = document["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0]["body"],
        "guybrush|Complaint: From: guybrush about: Mo"
    );
    assert_eq!(posts[0]["kind"], "complaint");
    let plan = deletion_plan(&store, ALICE).unwrap();
    assert_eq!(plan.posts, 2);
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
