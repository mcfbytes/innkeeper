//! Yserbius and Twinion transcripts: DARKSTRT's join sequence, two players meeting on a map and in a
//! party, the version gate, and the bulletin board. Bytes from docs/protocol/int14h-census/yserbius.md
//! sections 4 and 5 and twinion.md section 4; the line syntax is in common/mod.rs.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use std::sync::Arc;

use common::Table;
use innkeeper_world::{
    AccountId, AccountRecord, EncodedPassword, GroupKey, LandType, MemoryStore, ObjectKind, Place,
    Store, World,
};

/// DARKSTRT for player A (shared-block SID 0x0100): player 0x0100, land group 0x0101, personal
/// party 0x0102, map 5 group 0x0103 (yserbius.md 4.3 steps 1 to 8).
const A_ENTERS_MAP_5: &str = "
    A > login | {login}
    A < login ack | {login ack}
    A > joinNet player | 07 00 00 00 00 01 01 8b ff ff 01 00
    A < player SID | 08 00 00 01 00 00 00 01
    A > 41/2 service lookup | 29 02 00 00 00 01 8b 00 00
    A < service located | 29 00 00 01 00 00 8b 00
    A > joinNet land 4 place 1 | 07 03 00 00 00 01 05 04 01 00 64 00
    A < land group SID | 08 00 00 01 00 00 01 01
    A > GrpJoin land v1.1.7 | 0a 03 01 01 00 01 01 01 07
    A < joined | 0a 00 01 01 00 01
    A > joinNet personal party | 07 00 00 00 00 01 04 8b fd ff 04 00
    A < party SID | 08 00 00 01 00 00 02 01
    A > GrpJoin party | 0a 00 02 01 00 01
    A < joined | 0a 00 02 01 00 01
    A > 40/4 character name | 28 04 53 69 72 00
    A > 217/1 hello to the land | 02 00 01 01 00 01 d9 01 00 01 53 69 72 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
    A > joinNet map 5 | 07 00 00 00 00 01 02 8b 05 00 68 00
    A < map group SID | 08 00 00 01 00 00 03 01
    A > GrpJoin map | 0a 00 03 01 00 01
    A < joined | 0a 00 03 01 00 01
    A > 200/1 roster to the map | 02 00 03 01 00 01 c8 01 00 00 00 00 00 00 05 05 00 00 00
";

/// B (shared-block SID 0x0200) follows A into the same land and map: player 0x0104, the same land
/// group, party 0x0105, the same map group; A hears both group joins.
const B_FOLLOWS: &str = "
    B > login | {login}
    B < login ack | {login ack}
    B > joinNet player | 07 00 00 00 00 02 01 8b ff ff 01 00
    B < player SID | 08 00 00 02 00 00 04 01
    B > 41/2 service lookup | 29 02 00 00 04 01 8b 00 00
    B < service located | 29 00 04 01 00 00 8b 00
    B > joinNet land 4 place 1 | 07 03 00 00 04 01 05 04 01 00 64 00
    B < the same land group | 08 00 04 01 00 00 01 01
    B > GrpJoin land v1.1.7 | 0a 03 01 01 04 01 01 01 07
    B < joined | 0a 00 01 01 04 01
    A < B is in the land | 0a 00 01 01 04 01
    B > joinNet personal party | 07 00 00 00 04 01 04 8b fd ff 04 00
    B < own party | 08 00 04 01 00 00 05 01
    B > GrpJoin party | 0a 00 05 01 04 01
    B < joined | 0a 00 05 01 04 01
    B > joinNet map 5 | 07 00 00 00 04 01 02 8b 05 00 68 00
    B < the same map group | 08 00 04 01 00 00 03 01
    B > GrpJoin map | 0a 00 03 01 04 01
    B < joined | 0a 00 03 01 04 01
    A < B is on the map | 0a 00 03 01 04 01
";

fn script(parts: &[&str]) -> String {
    parts.join("\n")
}

#[test]
fn darkstrt_reaches_its_map_and_the_service_lookup_ends_the_resend_loop() {
    Table::new().run(A_ENTERS_MAP_5);
}

#[test]
fn two_players_share_the_land_and_map_groups_and_see_each_others_messages() {
    let steps = "
        # 200/1 from B: a leader answers hello with 200/2 to the sender (yserbius.md 5).
        B > 200/1 roster to the map | 02 00 03 01 04 01 c8 01 00 00 00 00 00 00 05 05 00 00 00
        A < A's copy | 02 00 03 01 04 01 c8 01 00 00 00 00 00 00 05 05 00 00 00
        A > 200/2 answer to B's player | 02 00 04 01 00 01 c8 02 00 00 00 00 00 00 05 05 00 00 00
        B < only B | 02 00 04 01 00 01 c8 02 00 00 00 00 00 00 05 05 00 00 00
        # 207 chat to the map group: the 11-byte name, then the text.
        B > 207 chat | 02 00 03 01 04 01 cf 53 69 72 00 00 00 00 00 00 00 00 68 69 00
        A < A's copy | 02 00 03 01 04 01 cf 53 69 72 00 00 00 00 00 00 00 00 68 69 00
    ";
    Table::new().run(&script(&[A_ENTERS_MAP_5, B_FOLLOWS, steps]));
}

#[test]
fn a_player_joins_anothers_party_and_its_members_hear_of_it() {
    let steps = "
        # 212 join request to the leader, 211 status 0 with the party SID back (yserbius.md 5).
        B > 212 join request to A | 02 00 00 01 04 01 d4
        A < only A | 02 00 00 01 04 01 d4
        A > 211 accepted, party 0x0102 | 02 00 04 01 00 01 d3 00 02 01
        B < only B | 02 00 04 01 00 01 d3 00 02 01
        B > GrpJoin A's party | 0a 00 02 01 04 01
        B < joined | 0a 00 02 01 04 01
        A < B is in the party | 0a 00 02 01 04 01
        B > 204 joined, to the party | 02 00 02 01 04 01 cc 01 04 01 00 00 00 01 04 01 00 00 00 00
        A < A's copy | 02 00 02 01 04 01 cc 01 04 01 00 00 00 01 04 01 00 00 00 00
        # Leaving: GrpDel tells the remaining member.
        B > GrpDel A's party | 0b 00 02 01 04 01
        A < B left the party | 0b 00 02 01 04 01
    ";
    Table::new().run(&script(&[A_ENTERS_MAP_5, B_FOLLOWS, steps]));
}

#[test]
fn a_dropped_player_leaves_every_group_and_its_objects_are_freed() {
    let steps = "
        B ! hang up
        A < B left the land | 0b 00 01 01 04 01
        A < B left the map | 0b 00 03 01 04 01
        A < B's player freed | 09 00 04 01
    ";
    Table::new().run(&script(&[A_ENTERS_MAP_5, B_FOLLOWS, steps]));
}

#[test]
fn darkstrt_shutdown_releases_everything() {
    let steps = "
        A > GrpDel party | 0b 00 02 01 00 01
        A > ObjFree party | 09 00 02 01
        A > GrpDel map | 0b 00 03 01 00 01
        A > ObjFree map | 09 00 03 01
        A > GrpDel land | 0b 00 01 01 00 01
        A > ObjFree land | 09 00 01 01
        A > ObjFree player | 09 00 00 01
    ";
    let mut table = Table::new();
    table.run(&script(&[A_ENTERS_MAP_5, steps]));
    assert!(table.objects.is_empty());
}

#[test]
fn twinion_maps_of_the_same_number_are_other_groups() {
    let twinion = "
        C > login | {login}
        C < login ack | {login ack}
        C > joinNet player | 07 00 00 00 00 03 01 8b ff ff 01 00
        C < player SID | 08 00 00 03 00 00 04 01
        C > joinNet land 8 place 1 | 07 03 00 00 04 01 05 08 01 00 64 00
        C < Twinion's land group | 08 00 04 01 00 00 05 01
        C > GrpJoin land v1.0.22 | 0a 03 05 01 04 01 01 00 16
        C < joined | 0a 00 05 01 04 01
        C > joinNet map 5 | 07 00 00 00 04 01 02 8b 05 00 50 00
        C < another map group | 08 00 04 01 00 00 06 01
    ";
    let mut table = Table::new();
    table.run(&script(&[A_ENTERS_MAP_5, twinion]));
    let map = |land_type| GroupKey {
        kind: ObjectKind::Group,
        land_type: LandType(0x8B),
        parameter: 5,
        within: Some(Place {
            land_type: LandType(land_type),
            land_number: 1,
        }),
    };
    assert_eq!(table.objects.member_count(map(4)), 1);
    assert_eq!(table.objects.member_count(map(8)), 0);
}

#[test]
fn a_land_join_with_another_titles_version_gets_code_6() {
    let wrong = "
        B > login | {login}
        B < login ack | {login ack}
        B > joinNet player | 07 00 00 00 00 02 01 8b ff ff 01 00
        B < player SID | 08 00 00 02 00 00 04 01
        B > joinNet land 4 place 1 | 07 03 00 00 04 01 05 04 01 00 64 00
        B < the land group | 08 00 04 01 00 00 01 01
        B > GrpJoin land v1.0.22 | 0a 03 01 01 04 01 01 00 16
        B < Nak 10, Incompatible Land Version | 01 00 01 01 0a 06 00 00 00
    ";
    Table::new().run(&script(&[A_ENTERS_MAP_5, wrong]));
}

/// Directory, index and reads for the tavern (location 0x59) after A posts twice; replies are
/// `b 27, b sub, w board, w player, w boardId` and the layouts of yserbius.md 4.4.
const TAVERN: &str = "
    A > 27/13 tavern directory | 1b 0d 00 00 00 01 59 00
    A < one empty board | 1b 0d 01 00 00 01 59 00 01 00 00 59 00 01 00 00 00 00 00 00 00 00 00 00 00 54 61 76 65 72 6e 5f 50 6f 73 74 73 00
    A > 27/0 post | 1b 00 59 00 00 01 01 00 53 69 72 7c 68 69 00
    A > 27/0 second post | 1b 00 59 00 00 01 01 00 53 69 72 7c 62 79 65 00
    A > 27/13 tavern directory | 1b 0d 00 00 00 01 59 00
    A < two messages | 1b 0d 01 00 00 01 59 00 01 02 00 59 00 01 00 02 00 00 00 00 00 00 00 00 00 54 61 76 65 72 6e 5f 50 6f 73 74 73 00
    A > 27/14 index | 1b 0e 59 00 00 01 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
    A < messages 1 and 2 | 1b 0e 59 00 00 01 01 00 00 00 00 00 00 00 00 00 00 00 01 00 02 00
    A > 27/4 first | 1b 04 59 00 00 01 01 00 01 00
    A < message 1, no previous | 1b 01 59 00 00 01 01 00 01 00 00 80 00 00 00 00 00 00 53 69 72 7c 68 69 00
    A > 27/2 next | 1b 02 59 00 00 01 01 00 01 00
    A < message 2, no next | 1b 01 59 00 00 01 01 00 02 00 00 40 00 00 00 00 00 00 53 69 72 7c 62 79 65 00
    A > 27/2 next again | 1b 02 59 00 00 01 01 00 02 00
    A < still message 2 | 1b 01 59 00 00 01 01 00 02 00 00 40 00 00 00 00 00 00 53 69 72 7c 62 79 65 00
    A > 27/5 last | 1b 05 59 00 00 01 01 00 01 00
    A < message 2 | 1b 01 59 00 00 01 01 00 02 00 00 40 00 00 00 00 00 00 53 69 72 7c 62 79 65 00
    A > 27/3 previous | 1b 03 59 00 00 01 01 00 02 00
    A < message 1 | 1b 01 59 00 00 01 01 00 01 00 00 80 00 00 00 00 00 00 53 69 72 7c 68 69 00
";

#[test]
fn the_tavern_board_lists_indexes_and_steps_through_posts() {
    Table::new().run(&script(&[A_ENTERS_MAP_5, TAVERN]));
}

#[test]
fn another_player_reads_the_post_and_twinion_does_not_see_it() {
    let steps = "
        A > 27/0 post | 1b 00 59 00 00 01 01 00 53 69 72 7c 68 69 00
        B > 27/1 message 1 | 1b 01 59 00 04 01 01 00 01 00
        B < A's post | 1b 01 59 00 04 01 01 00 01 00 00 c0 00 00 00 00 00 00 53 69 72 7c 68 69 00
        C > login | {login}
        C < login ack | {login ack}
        C > joinNet land 8 place 1 | 07 03 00 00 00 03 05 08 01 00 64 00
        C < Twinion's land group | 08 00 00 03 00 00 06 01
        C > 27/13 tavern directory | 1b 0d 00 00 00 03 59 00
        C < an empty board | 1b 0d 01 00 00 03 59 00 01 00 00 59 00 01 00 00 00 00 00 00 00 00 00 00 00 54 61 76 65 72 6e 5f 50 6f 73 74 73 00
    ";
    Table::new().run(&script(&[A_ENTERS_MAP_5, B_FOLLOWS, steps]));
}

#[test]
fn an_unknown_location_has_no_boards_and_a_request_outside_a_land_is_offline() {
    let steps = "
        A > 27/13 unknown location | 1b 0d 00 00 00 01 22 00
        A < no boards | 1b 0d 00 00 00 01 22 00
        B > login | {login}
        B < login ack | {login ack}
        B > 27/13 before any land | 1b 0d 00 00 00 02 59 00
        B < Nak 27, BBS Server offline | 01 00 00 02 1b 0d 00 00 00
    ";
    Table::new().run(&script(&[A_ENTERS_MAP_5, steps]));
}

#[test]
fn a_privileged_reader_gets_the_text_four_bytes_later() {
    let store = Arc::new(MemoryStore::new());
    let password = EncodedPassword([0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00]);
    let mut record = AccountRecord::new(password, "guybrush");
    record.user_flags = 0x0004;
    store.create_account(AccountId(100_001), record).unwrap();
    let mut table = Table::new();
    table.world = World::stored(store);
    let steps = "
        A > 27/0 post | 1b 00 59 00 00 01 01 00 53 69 72 7c 68 69 00
        A > 27/1 message 1 | 1b 01 59 00 00 01 01 00 01 00
        A < four more bytes | 1b 01 59 00 00 01 01 00 01 00 00 c0 00 00 00 00 00 00 00 00 00 00 53 69 72 7c 68 69 00
    ";
    let privileged_ack = "00 00 00 00 16 04 00 00 00 00";
    let script = script(&[A_ENTERS_MAP_5, steps]).replacen("{login ack}", privileged_ack, 1);
    table.run(&script);
}
