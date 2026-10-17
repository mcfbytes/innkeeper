//! The hub's last unanswered messages: the complaint forms (27/0 board 0/99 and 27/9), the bulletin
//! board's erase (27/6), the Hint Board room (38) and the short sysop pages (37/20, 37/21).
//! docs/server/hub-services.md has the layouts and the audit.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use std::sync::Arc;

use common::Table;
use innkeeper_world::{
    AccountId, AccountRecord, BoardId, EncodedPassword, MemoryStore, Store, World,
    COMPLAINT_BOARD_LOCATION,
};

const LOGIN_PASSWORD: [u8; 10] = [0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00];

fn stored_table(flags: u16) -> (Table, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    let mut record = AccountRecord::new(EncodedPassword(LOGIN_PASSWORD), "guybrush");
    record.user_flags = 0x0200 | flags;
    store.create_account(AccountId(100_001), record).unwrap();
    for id in [100_002, 100_003] {
        let other = AccountRecord::new(EncodedPassword(LOGIN_PASSWORD), "Mo");
        store.create_account(AccountId(id), other).unwrap();
    }
    let mut table = Table::new();
    table.world = World::stored(store.clone());
    (table, store)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x} ")).collect()
}

/// The table's Login with another account number and name.
fn login_as(account: u32, name: &str) -> String {
    let mut bytes = vec![0x35, 0, 0, 0, 1, 2, 3, 0x12];
    bytes.extend(account.to_le_bytes());
    bytes.push(1);
    bytes.extend(LOGIN_PASSWORD);
    bytes.push(0);
    bytes.extend(name.as_bytes());
    bytes.push(0);
    hex(&bytes)
}

fn named_accounts(store: &MemoryStore) -> Vec<Option<AccountId>> {
    let board = BoardId(u32::from(COMPLAINT_BOARD_LOCATION) << 8 | 3);
    let posts = store.posts(board).unwrap();
    posts.iter().map(|post| post.about).collect()
}

#[test]
fn a_complaint_names_the_account_of_the_one_player_online_under_that_name() {
    let (mut table, store) = stored_table(0);
    let about_mo = hex(b"Complaint|From: guybrush about: Mo, who said \"boo\"\0");
    let about_nobody = hex(b"Complaint|From: guybrush about: Zed, rude\0");
    let steps = format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        B > login | {}
        B < login ack | {{login ack}}
        A > about Mo, the only Mo online | 1b 09 00 00 00 01 63 00 34 12 {about_mo}
        A > about Zed, who is not online | 1b 09 00 00 00 01 63 00 34 12 {about_nobody}
        C > login | {}
        C < login ack | {{login ack}}
        A > about Mo, now two accounts | 1b 00 00 00 00 01 63 00 {about_mo}
        ",
        login_as(100_002, "Mo"),
        login_as(100_003, "Mo"),
    );
    table.run(&steps);
    assert_eq!(
        named_accounts(&store),
        [Some(AccountId(100_002)), None, None],
        "a name two accounts share online, or nobody's, names no account"
    );
}

fn bodies(store: &MemoryStore, location: u16, section: u32) -> Vec<String> {
    let board = BoardId(u32::from(location) << 8 | section);
    let posts = store.posts(board).unwrap();
    posts
        .into_iter()
        .map(|post| String::from_utf8(post.body).unwrap())
        .collect()
}

#[test]
fn both_complaint_forms_are_kept_on_the_complaint_board_and_nothing_is_answered() {
    let (mut table, store) = stored_table(0);
    let plain =
        hex(b"Complaint|From: guybrush in: the Map; about: Mo, who said \"boo\" Comments: rude\0");
    let named = hex(b"Complaint|From: guybrush about: Mo\0");
    let steps = format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > complaint, nobody named | 1b 00 00 00 00 01 63 00 {plain}
        A > complaint about a player | 1b 09 00 00 00 01 63 00 34 12 {named}
        "
    );
    table.run(&steps);
    assert_eq!(
        bodies(&store, COMPLAINT_BOARD_LOCATION, 3),
        [
            "guybrush|Complaint: From: guybrush in: the Map; about: Mo, who said \"boo\" Comments: rude",
            "guybrush|Complaint: From: guybrush about: Mo",
        ]
    );
}

#[test]
fn a_complaint_is_cut_to_the_limit_and_the_board_is_the_retention_boards_location() {
    let (mut table, store) = stored_table(0);
    let long = hex(&[b'x'; 2500]);
    let steps = format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > long complaint | 1b 09 00 00 00 01 63 00 01 00 {long} 00
        "
    );
    table.run(&steps);
    let kept = bodies(&store, COMPLAINT_BOARD_LOCATION, 3);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].len(), "guybrush|Complaint: ".len() + 2000);
    // The ordinary 365-day clock finds it by the location, as for every complaint.
    let board = BoardId(u32::from(COMPLAINT_BOARD_LOCATION) << 8 | 3);
    assert!(innkeeper_world::is_complaint_board(board));
}

const IN_A_LAND: &str = "
    A > login | {login}
    A < login ack | {login ack}
    A > joinNet land 4 place 1 | 07 03 00 00 00 01 05 04 01 00 64 00
    A < land group SID | 08 00 00 01 00 00 00 01
";

const BOARD_59_1: BoardId = BoardId(0x59 << 8 | 1 | 4 << 24);

#[test]
fn erase_removes_the_players_own_message_and_leaves_other_peoples() {
    let (mut table, store) = stored_table(0);
    // Message 1 is another account's (the name at its head does not matter); message 2 is the player's.
    store
        .post(BOARD_59_1, AccountId(100_002), None, b"Sir|hi\0")
        .unwrap();
    let steps = format!(
        "{IN_A_LAND}
        A > 27/0 post by me | 1b 00 59 00 00 01 01 00 {}
        A > 27/6 erase message 1 | 1b 06 59 00 00 01 01 00 01 00 00
        A > 27/6 erase message 2 | 1b 06 59 00 00 01 01 00 02 00 00
        A > 27/6 erase message 9 | 1b 06 59 00 00 01 01 00 09 00 00
        ",
        hex(b"Guybrush|mine\0"),
    );
    table.run(&steps);
    assert_eq!(bodies(&store, 0x59, 1 | 4 << 24), ["Sir|hi\0"]);
}

#[test]
fn a_player_erases_by_account_not_by_a_name_another_player_shares() {
    let (mut table, store) = stored_table(0);
    // Two other accounts write under the player's own name: neither post is the player's.
    store.set_persona(AccountId(100_002), "Guybrush").unwrap();
    store
        .post(BOARD_59_1, AccountId(100_002), None, b"Guybrush|twin\0")
        .unwrap();
    store
        .post(BOARD_59_1, AccountId(100_003), None, b"guybrush|other\0")
        .unwrap();
    let steps = format!(
        "{IN_A_LAND}
        A > 27/6 erase the twin's message | 1b 06 59 00 00 01 01 00 01 00 00
        A > 27/6 erase the other one | 1b 06 59 00 00 01 01 00 02 00 00
        "
    );
    table.run(&steps);
    assert_eq!(
        bodies(&store, 0x59, 1 | 4 << 24),
        ["Guybrush|twin\0", "guybrush|other\0"]
    );
}

#[test]
fn board_posts_and_complaints_record_the_writing_account() {
    let (mut table, store) = stored_table(0);
    let steps = format!(
        "{IN_A_LAND}
        A > 27/0 post | 1b 00 59 00 00 01 01 00 {}
        A > complaint | 1b 09 00 00 00 01 63 00 {}
        ",
        hex(b"Guybrush|mine\0"),
        hex(b"Complaint|rude\0"),
    );
    table.run(&steps);
    let writer = |board: BoardId| store.posts(board).unwrap()[0].account;
    assert_eq!(writer(BOARD_59_1), AccountId(100_001));
    let complaint = BoardId(u32::from(COMPLAINT_BOARD_LOCATION) << 8 | 3);
    assert_eq!(writer(complaint), AccountId(100_001));
}

#[test]
fn a_privileged_reader_may_erase_anyones_message() {
    let (mut table, store) = stored_table(0x0004);
    let steps = format!(
        "{IN_A_LAND}
        A > 27/0 post by someone else | 1b 00 59 00 00 01 01 00 {}
        A > 27/6 erase message 1 | 1b 06 59 00 00 01 01 00 01 00 00
        ",
        hex(b"Sir|hi\0"),
    );
    let privileged_ack = "00 00 00 00 16 04 02 00 af 00";
    table.run(&steps.replacen("{login ack}", privileged_ack, 1));
    assert!(bodies(&store, 0x59, 1 | 4 << 24).is_empty());
}

#[test]
fn the_hint_board_answers_each_menu_level_then_a_problem_and_its_answer() {
    let (mut table, _store) = stored_table(0);
    let series = hex(b"Getting Around\nMeeting Folks\nGames and Pastimes\nOdds and Ends");
    let games = hex(b"The Map\nThe Post Office\nThe Town Hall");
    let areas = hex(b"Reading Mail\nWriting Mail");
    let problem = hex(b"Is there any mail for me? How would I even know?");
    let steps = format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > first menu 38/0 | 26 00 03 01 00 00 00 00
        A < series | 26 00 03 01 00 {series} 00
        A > games of series 1 38/1 | 26 01 03 01 01 00 00 00
        A < games | 26 01 03 01 01 {games} 00
        A > areas of game 2 38/2 | 26 02 03 01 01 02 00 00
        A < areas | 26 02 03 01 02 {areas} 00
        A > first hint of area 1 38/3 | 26 03 03 01 01 02 01 01
        A < problem | 26 03 03 01 01 {problem} 00
        A < answer | 26 04 03 01 01 {answer} 00
        A > a third hint that is not there | 26 03 03 01 01 02 01 03
        A < Nak 38/6, no more hints | 01 00 03 01 26 06 00 00 00
        A > a series that is not there | 26 01 03 01 09 00 00 00
        A < Nak 38/5, menu unavailable | 01 00 03 01 26 05 00 00 00
        ",
        answer = hex(b"When a letter is waiting, a little mail icon lights up on the map. No light, no letters; the postmaster sends his regrets and a very short list."),
    );
    table.run(&steps);
}

#[test]
fn a_hint_request_before_login_gets_no_reply() {
    let mut table = Table::new();
    table.run("A > first menu 38/0 | 26 00 03 01 00 00 00 00");
}

#[test]
fn the_short_sysop_pages_are_refused_with_no_rights_so_no_wait_is_left() {
    let (mut table, _store) = stored_table(0);
    table.run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > 37/20 page | 25 14 02 01 05 00 00 00 07 00 08 00 30 75 00 00
        A < Nak 37/20, fault 12 | 01 00 02 01 25 14 0c 05 00 00 00
        A > 37/21 page | 25 15 02 01 05 00 00 00 07 00 08 00
        A < Nak 37/21, fault 12 | 01 00 02 01 25 15 0c 05 00 00 00
        ",
    );
}

#[test]
fn a_moderators_topic_text_replaces_the_last_and_is_what_the_conference_list_reads() {
    let (mut table, _store) = stored_table(0x0004);
    let first = hex(b"\x01old info\x02topic one\x02\0");
    let second = hex(b"\x01new info\x02topic one\x02topic two\x02\0");
    let reply_text = hex(b"\x01new info\x02topic one\x02topic two\x02");
    let steps = format!(
        "{IN_A_LAND}
        A > conference topics 27/0 board 0/67 | 1b 00 00 00 02 01 43 00 {first}
        A > conference topics again | 1b 00 00 00 02 01 43 00 {second}
        A > conference list 27/1 board 0/67 | 1b 01 00 00 02 01 43 00 01 00
        A < the current text, a privileged reader's layout | 1b 01 00 00 02 01 43 00 01 00 00 c0 00 00 00 00 00 00 00 00 00 00 {reply_text} 00
        "
    );
    let privileged_ack = "00 00 00 00 16 04 02 00 af 00";
    table.run(&steps.replacen("{login ack}", privileged_ack, 1));
}

#[test]
fn topic_text_from_a_reader_without_moderator_rights_is_dropped_and_the_list_stays_empty() {
    let (mut table, _store) = stored_table(0);
    let text = hex(b"\x01graffiti\x02\0");
    let steps = format!(
        "{IN_A_LAND}
        A > conference topics 27/0 board 0/67 | 1b 00 00 00 02 01 43 00 {text}
        A > conference list 27/1 board 0/67 | 1b 01 00 00 02 01 43 00 01 00
        A < nothing yet | 1b 01 00 00 02 01 43 00 00 00 00 c0 00 00 00 00 00 00 00
        "
    );
    table.run(&steps);
}
