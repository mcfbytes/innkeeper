//! Chat transcripts: room chat (Send, msgType 1), conference chat (multicast, msgType 50) and
//! the conference lookup (41/0). Layouts: messages.md 3.2.1 and 8; line syntax: common/mod.rs.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::Table;
use innkeeper_world::{lookup_name, LandType, Sid};

/// `who` logs in and joins a player object, which gets SID `player`.
fn log_in_with_player(who: char, player: u8) -> String {
    format!(
        "
        {who} > login | {{login}}
        {who} < login ack | {{login ack}}
        {who} > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        {who} < object id | 08 00 8c 09 00 00 {player:02x} 01
        "
    )
}

/// `who` joins conference `number` (group parameter 9 + number, size 24, land type 1) as `group`.
fn join_conference(who: char, number: u8, group: u8) -> String {
    let parameter = 9 + number;
    format!(
        "
        {who} > join conference | 07 00 00 00 8d 09 02 01 {parameter:02x} 00 18 00
        {who} < group SID | 08 00 8d 09 00 00 {group:02x} 01
        "
    )
}

/// `who` logs in, joins the Clubhouse waiting room (SID 0x0101) and is added; `news` follows.
fn enter_waiting_room(who: char, player: u8, news: &str) -> String {
    let login = log_in_with_player(who, player);
    format!(
        "{login}
        {who} > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        {who} < object id | 08 00 c0 01 00 00 01 01
        {who} > add player | 0a 00 01 01 {player:02x} 01 02 03 12
        {who} < group joined | 0a 00 01 01 {player:02x} 01
        {news}"
    )
}

#[test]
fn two_players_chat_in_the_clubhouse_room() {
    let mut table = Table::new();
    table.run(&enter_waiting_room('A', 0x00, ""));
    table.run(&enter_waiting_room(
        'B',
        0x02,
        "A < news of B | 0a 00 01 01 02 01",
    ));
    // msgType 1: foreground 15, background 0, flag 1, then the text and its NUL.
    table.run(
        "
        A > chat line | 02 00 01 01 00 01 01 00 0f 00 01 68 69 00
        A < A hears itself | 02 00 01 01 00 01 01 00 0f 00 01 68 69 00
        B < B hears A | 02 00 01 01 00 01 01 00 0f 00 01 68 69 00
        B > reply | 02 00 01 01 02 01 01 00 0f 00 01 79 6f 00
        A < A hears B | 02 00 01 01 02 01 01 00 0f 00 01 79 6f 00
        B < B hears itself | 02 00 01 01 02 01 01 00 0f 00 01 79 6f 00
        ",
    );
}

#[test]
fn a_line_to_the_selected_players_goes_to_their_objects_only() {
    let mut table = Table::new();
    table.run(&enter_waiting_room('A', 0x00, ""));
    table.run(&enter_waiting_room(
        'B',
        0x02,
        "A < news of B | 0a 00 01 01 02 01",
    ));
    table.run(&enter_waiting_room(
        'C',
        0x03,
        "A < news of C | 0a 00 01 01 03 01\nB < news of C | 0a 00 01 01 03 01",
    ));
    // The Talk button sends one Send per selected player; the edit line multicasts to the selection.
    table.run(
        "
        A > line to B | 02 00 02 01 00 01 01 00 0f 00 01 73 68 00
        B < only B | 02 00 02 01 00 01 01 00 0f 00 01 73 68 00
        A > line to B and C | 1c 00 01 02 00 02 01 03 01 01 00 0f 00 01 68 69 00
        B < B hears A | 02 00 02 01 00 01 01 00 0f 00 01 68 69 00
        C < C hears A | 02 00 03 01 00 01 01 00 0f 00 01 68 69 00
        ",
    );
}

#[test]
fn a_conference_multicast_reaches_the_members_and_nobody_in_another_conference() {
    let mut table = Table::new();
    table.run(&log_in_with_player('A', 0x00));
    table.run(&join_conference('A', 1, 0x01));
    table.run("A > add player | 0a 00 01 01 00 01\nA < group joined | 0a 00 01 01 00 01");
    table.run(&log_in_with_player('B', 0x02));
    table.run(&join_conference('B', 1, 0x01));
    table.run(
        "
        B > add player | 0a 00 01 01 02 01
        B < group joined | 0a 00 01 01 02 01
        A < news of B | 0a 00 01 01 02 01
        ",
    );
    table.run(&log_in_with_player('C', 0x03));
    table.run(&join_conference('C', 2, 0x04));
    table.run("C > add player | 0a 00 04 01 03 01\nC < group joined | 0a 00 04 01 03 01");
    // The member list is what the client multicasts to: msgType 50, colours 15 and 0, flag 1.
    table.run(
        "
        A > members | 0c 00 01 01 00 01
        A < the two members | 0c 00 01 01 00 00 00 01 02 01
        A > chat | 1c 00 01 02 00 00 01 02 01 32 00 0f 00 01 68 69 00
        A < A hears itself | 02 00 00 01 00 01 32 00 0f 00 01 68 69 00
        B < B hears A | 02 00 02 01 00 01 32 00 0f 00 01 68 69 00
        B > reply | 1c 02 01 02 00 00 01 02 01 32 00 0f 00 01 79 6f 00
        A < A hears B | 02 00 00 01 02 01 32 00 0f 00 01 79 6f 00
        B < B hears itself | 02 00 02 01 02 01 32 00 0f 00 01 79 6f 00
        ",
    );
}

#[test]
fn a_free_conference_name_is_located_at_sid_0_and_a_taken_one_at_its_object() {
    let mut table = Table::new();
    table.run(&log_in_with_player('A', 0x00));
    let lookup = "29 00 00 01 00 01 01 00 0f 00 00 00";
    table.run(&format!(
        "A > lookup | {lookup}\nA < nobody has the name | 29 00 00 01 00 00 00 00"
    ));
    let name = lookup_name(LandType(1));
    table.world.conferences.publish(name, Sid(0x00C0));
    table.run(&format!(
        "A > lookup | {lookup}\nA < the control object | 29 00 00 01 00 00 c0 00"
    ));
    table.run(
        "
        # Another land type's name is still free.
        A > lookup | 29 00 00 01 00 01 02 00 0f 00 00 00
        A < nobody has the name | 29 00 00 01 00 00 00 00
        ",
    );
}

#[test]
fn lookups_before_login_and_the_dos_service_lookup_get_no_reply() {
    let mut table = Table::new();
    table.run("C > lookup before login | 29 00 00 01 00 01 01 00 0f 00 00 00");
    table.run(&log_in_with_player('A', 0x00));
    table.run("A > service lookup | 29 02 00 00 00 01 8b 00 00");
}
