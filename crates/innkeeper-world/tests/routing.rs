//! Relay transcripts: Sends and multicasts reach the right connections, in order and untouched.
//! The bytes are built from golf.md 7, redbaron.md 5.1 and 7, and yserbius.md 4.2 and 5; the line
//! syntax is in common/mod.rs.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::Table;

/// GOLF (land type 0x66): both players join the group, then A's and B's player objects follow.
/// The group is SID 0x0100, A's player 0x0101, B's player 0x0102.
const GOLF_ROOM: &str = "
    A > login | {login}
    A < login ack | {login ack}
    A > join the group | 07 00 00 00 c0 01 05 66 01 00 80 00
    A < group SID | 08 00 c0 01 00 00 00 01
    A > join the player object | 07 00 00 00 01 00 01 66 ff ff 01 00
    A < A's SID | 08 00 01 00 00 00 01 01
    A > add the player | 0a 00 00 01 01 01
    A < group joined | 0a 00 00 01 01 01
    B > login | {login}
    B < login ack | {login ack}
    B > join the group | 07 00 00 00 c0 01 05 66 01 00 80 00
    B < the same group | 08 00 c0 01 00 00 00 01
    B > join the player object | 07 00 00 00 01 00 01 66 ff ff 01 00
    B < B's SID | 08 00 01 00 00 00 02 01
    B > add the player | 0a 00 00 01 02 01
    B < echo to the joiner | 0a 00 00 01 02 01
    A < and to the member | 0a 00 00 01 02 01
";

/// Red Baron (land type 0x65, player objects of kind 3), with the same SIDs as `GOLF_ROOM`.
const RED_BARON_ROOM: &str = "
    A > login | {login}
    A < login ack | {login ack}
    A > join the group | 07 00 00 00 c0 01 05 65 01 00 80 00
    A < group SID | 08 00 c0 01 00 00 00 01
    A > join the player object | 07 00 00 00 01 00 03 65 ff ff 01 00
    A < A's SID | 08 00 01 00 00 00 01 01
    A > add the player | 0a 00 00 01 01 01
    A < group joined | 0a 00 00 01 01 01
    B > login | {login}
    B < login ack | {login ack}
    B > join the group | 07 00 00 00 c0 01 05 65 01 00 80 00
    B < the same group | 08 00 c0 01 00 00 00 01
    B > join the player object | 07 00 00 00 01 00 03 65 ff ff 01 00
    B < B's SID | 08 00 01 00 00 00 02 01
    B > add the player | 0a 00 00 01 02 01
    B < echo to the joiner | 0a 00 00 01 02 01
    A < and to the member | 0a 00 00 01 02 01
";

fn script(room: &str, steps: &str) -> String {
    [room, steps].join("\n")
}

#[test]
fn a_golf_group_send_reaches_both_players_of_the_waiting_room() {
    let steps = "
        # CA, the 32-byte player announcement, goes to the group; the sender hears its own copy.
        A > CA to the group | 02 00 00 01 01 01 ca 41 6e 6e 00 00 00 00 00 00 00 00 01 02 03 04 09 09 09 09 34 12 05 06 01 01
        A < own copy | 02 00 00 01 01 01 ca 41 6e 6e 00 00 00 00 00 00 00 00 01 02 03 04 09 09 09 09 34 12 05 06 01 01
        B < B's copy | 02 00 00 01 01 01 ca 41 6e 6e 00 00 00 00 00 00 00 00 01 02 03 04 09 09 09 09 34 12 05 06 01 01
        # CD, chat from B: ascending connection order, so A before B.
        B > CD to the group | 02 00 00 01 02 01 cd 66 6f 72 65 21 00
        A < A's copy | 02 00 00 01 02 01 cd 66 6f 72 65 21 00
        B < own copy | 02 00 00 01 02 01 cd 66 6f 72 65 21 00
        # A Send to one player object reaches its holder only.
        A > CD to B's player | 02 00 02 01 01 01 cd 66 6f 72 65 21 00
        B < only B | 02 00 02 01 01 01 cd 66 6f 72 65 21 00
    ";
    Table::new().run(&script(GOLF_ROOM, steps));
}

#[test]
fn yserbius_game_messages_arrive_byte_for_byte() {
    // Land type 0x8B: a map group of kind 2 (parameter 5) in place of the waiting room.
    let room = GOLF_ROOM
        .replace("05 66 01 00 80 00", "02 8b 05 00 68 00")
        .replace("01 66 ff ff", "01 8b ff ff");
    let steps = "
        # 200 hello (19 bytes), 207 chat (24) and 216 combat state (35), all from A's player.
        A > 200 | 02 00 00 01 01 01 c8 01 00 00 00 00 00 00 05 07 00 00 00
        A < 200 to A | 02 00 00 01 01 01 c8 01 00 00 00 00 00 00 05 07 00 00 00
        B < 200 to B | 02 00 00 01 01 01 c8 01 00 00 00 00 00 00 05 07 00 00 00
        A > 207 | 02 00 00 01 01 01 cf 47 75 79 62 72 75 73 68 00 00 00 48 65 6c 6c 6f 00
        A < 207 to A | 02 00 00 01 01 01 cf 47 75 79 62 72 75 73 68 00 00 00 48 65 6c 6c 6f 00
        B < 207 to B | 02 00 00 01 01 01 cf 47 75 79 62 72 75 73 68 00 00 00 48 65 6c 6c 6f 00
        A > 216 | 02 00 00 01 01 01 d8 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f 20 21 22 23 24 25 26 27 28 29 2a 2b
        A < 216 to A | 02 00 00 01 01 01 d8 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f 20 21 22 23 24 25 26 27 28 29 2a 2b
        B < 216 to B | 02 00 00 01 01 01 d8 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f 20 21 22 23 24 25 26 27 28 29 2a 2b
    ";
    Table::new().run(&script(&room, steps));
}

#[test]
fn a_red_baron_state_multicast_to_the_group_and_the_sender_comes_back_to_both() {
    // 0xC9, 35 bytes, to {group, own player}; the host sends no message of its own.
    let steps = "
        A > C9 multicast | 1c 01 01 02 00 00 01 01 01 c9 01 08 0f 16 1d 24 2b 32 39 40 47 4e 55 5c 63 6a 71 78 7f 86 8d 94 9b a2 a9 b0 b7 be c5 cc d3 da e1 e8
        A < group copy | 02 00 00 01 01 01 c9 01 08 0f 16 1d 24 2b 32 39 40 47 4e 55 5c 63 6a 71 78 7f 86 8d 94 9b a2 a9 b0 b7 be c5 cc d3 da e1 e8
        B < group copy | 02 00 00 01 01 01 c9 01 08 0f 16 1d 24 2b 32 39 40 47 4e 55 5c 63 6a 71 78 7f 86 8d 94 9b a2 a9 b0 b7 be c5 cc d3 da e1 e8
        A < own copy | 02 00 01 01 01 01 c9 01 08 0f 16 1d 24 2b 32 39 40 47 4e 55 5c 63 6a 71 78 7f 86 8d 94 9b a2 a9 b0 b7 be c5 cc d3 da e1 e8
    ";
    Table::new().run(&script(RED_BARON_ROOM, steps));
}

#[test]
fn a_multicast_is_one_send_per_recipient_in_list_order() {
    // Recipients: B's player, the group, A's player.
    let steps = "
        A > multicast | 1c 01 01 03 00 02 01 00 01 01 01 c9 03 08 0d 12 17 1c 21 26 2b 30 35 3a 3f 44 49 4e 53 58 5d 62 67 6c 71 76 7b 80 85 8a 8f 94 99 9e a3 a8
        B < to B's player | 02 00 02 01 01 01 c9 03 08 0d 12 17 1c 21 26 2b 30 35 3a 3f 44 49 4e 53 58 5d 62 67 6c 71 76 7b 80 85 8a 8f 94 99 9e a3 a8
        A < to the group | 02 00 00 01 01 01 c9 03 08 0d 12 17 1c 21 26 2b 30 35 3a 3f 44 49 4e 53 58 5d 62 67 6c 71 76 7b 80 85 8a 8f 94 99 9e a3 a8
        B < to the group | 02 00 00 01 01 01 c9 03 08 0d 12 17 1c 21 26 2b 30 35 3a 3f 44 49 4e 53 58 5d 62 67 6c 71 76 7b 80 85 8a 8f 94 99 9e a3 a8
        A < to A's player | 02 00 01 01 01 01 c9 03 08 0d 12 17 1c 21 26 2b 30 35 3a 3f 44 49 4e 53 58 5d 62 67 6c 71 76 7b 80 85 8a 8f 94 99 9e a3 a8
    ";
    Table::new().run(&script(RED_BARON_ROOM, steps));
}

#[test]
fn a_send_to_a_freed_or_unknown_sid_gives_nothing() {
    Table::new().run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > join an object | 07 00 00 00 01 00 01 66 ff ff 01 00
        A < its SID | 08 00 01 00 00 00 00 01
        A > free it | 09 00 00 01 00 00
        A > Send to the freed SID | 02 00 00 01 00 01 cd 66 6f 72 65 21 00
        A > Send to a SID never given out | 02 00 ff 7f 00 01 cd 66 6f 72 65 21 00
        A > multicast to both | 1c 00 01 02 00 00 01 ff 7f cd 66 6f 72 65 21 00
        ",
    );
}

#[test]
fn a_connection_that_has_not_logged_in_is_ignored() {
    let steps = "
        C > Send before Login | 02 00 00 01 01 01 cd 66 6f 72 65 21 00
        C > multicast before Login | 1c 01 01 01 00 00 01 cd 66 6f 72 65 21 00
    ";
    Table::new().run(&script(GOLF_ROOM, steps));
}
