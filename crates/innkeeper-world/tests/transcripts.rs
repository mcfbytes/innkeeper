//! Waiting-room and group transcripts; the line syntax is in common/mod.rs.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::Table;
use innkeeper_world::{GroupKey, LandType, ObjectKind};

const WAITING_ROOM: GroupKey = GroupKey {
    kind: ObjectKind::LandGroup,
    land_type: LandType(1),
    parameter: 1,
};

#[test]
fn two_players_meet_in_the_clubhouse_waiting_room() {
    let mut table = Table::new();
    table.run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > join game object | 07 00 00 00 60 01 81 01 00 00 0c 00
        A < object id | 08 00 60 01 00 00 00 01
        B > login | {login}
        B < login ack | {login ack}
        B > join game object, same cookie as A | 07 00 00 00 60 01 81 01 00 00 0c 00
        B < a SID of its own | 08 00 60 01 00 00 01 01
        A > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        A < object id | 08 00 8c 09 00 00 02 01
        A > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        A < object id | 08 00 c0 01 00 00 03 01
        A > add player to waiting room | 0a 00 03 01 02 01 02 03 12
        A < group joined | 0a 00 03 01 02 01
        B > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        B < object id | 08 00 8c 09 00 00 04 01
        B > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        B < the same group | 08 00 c0 01 00 00 03 01
        B > add player to waiting room | 0a 00 03 01 04 01 02 03 12
        B < echo to the joiner first | 0a 00 03 01 04 01
        A < then to the members | 0a 00 03 01 04 01
        B > member list | 0c 00 03 01 04 01
        B < members from byte 6, in joining order | 0c 00 03 01 00 00 02 01 04 01
        A > leave the waiting room | 0b 00 03 01 02 01
        B < the remaining member hears it | 0b 00 03 01 02 01
        A > add player again | 0a 00 03 01 02 01 02 03 12
        A < group joined | 0a 00 03 01 02 01
        B < group joined | 0a 00 03 01 02 01
        ",
    );
    assert_eq!(table.objects.member_count(WAITING_ROOM), 2);
    table.run(
        "
        B ! hang up
        A < B's player left the room | 0b 00 03 01 04 01
        A < and is gone | 09 00 04 01
        A > member list | 0c 00 03 01 02 01
        A < only A is left | 0c 00 03 01 00 00 02 01
        ",
    );
    assert_eq!(table.objects.member_count(WAITING_ROOM), 1);
}

#[test]
fn a_refused_group_join_names_the_reason() {
    Table::new().run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        A < object id | 08 00 8c 09 00 00 00 01
        A > join a game group for one | 07 00 00 00 10 00 02 01 07 00 01 00
        A < object id | 08 00 10 00 00 00 01 01
        A > add player to a SID that is no group | 0a 00 00 02 00 01
        A < Nak 10, code 1 | 01 00 00 02 0a 01 00 00 00
        A > add an object nobody holds | 0a 00 01 01 00 03
        A < Nak 10, code 3 | 01 00 01 01 0a 03 00 00 00
        A > add player | 0a 00 01 01 00 01
        A < group joined | 0a 00 01 01 00 01
        B > login | {login}
        B < login ack | {login ack}
        B > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        B < object id | 08 00 8c 09 00 00 02 01
        B > join the same game group | 07 00 00 00 10 00 02 01 07 00 01 00
        B < object id | 08 00 10 00 00 00 01 01
        B > add player to the full group | 0a 00 01 01 02 01
        B < Nak 10, code 2 | 01 00 01 01 0a 02 00 00 00
        B > add A's player | 0a 00 01 01 00 01
        B < Nak 10, code 3: not B's object | 01 00 01 01 0a 03 00 00 00
        ",
    );
}

#[test]
fn the_last_release_of_a_shared_group_destroys_it() {
    Table::new().run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        A < object id | 08 00 8c 09 00 00 00 01
        A > join a game group for one | 07 00 00 00 10 00 02 01 07 00 01 00
        A < object id | 08 00 10 00 00 00 01 01
        A > add player | 0a 00 01 01 00 01
        A < group joined | 0a 00 01 01 00 01
        B > login | {login}
        B < login ack | {login ack}
        B > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        B < object id | 08 00 8c 09 00 00 02 01
        B > join the same game group | 07 00 00 00 10 00 02 01 07 00 01 00
        B < object id | 08 00 10 00 00 00 01 01
        # A releases its reference: its player leaves, the group stays for B.
        A > free the game group | 09 00 01 01 00 00
        B > add player, which fits now | 0a 00 01 01 02 01
        B < group joined | 0a 00 01 01 02 01
        # B releases the last reference: the group is gone, a new join gets a new SID.
        B > free the game group | 09 00 01 01 00 00
        A > join the game group again | 07 00 00 00 10 00 02 01 07 00 01 00
        A < a new group | 08 00 10 00 00 00 03 01
        A > add player | 0a 00 03 01 00 01
        A < group joined | 0a 00 03 01 00 01
        ",
    );
}

#[test]
fn a_rejoined_game_object_replaces_the_old_one_and_peers_hear_of_it() {
    Table::new().run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > join game object | 07 00 00 00 60 01 81 01 00 00 0c 00
        A < object id | 08 00 60 01 00 00 00 01
        A > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        A < object id | 08 00 c0 01 00 00 01 01
        A > add game object to waiting room | 0a 00 01 01 00 01
        A < group joined | 0a 00 01 01 00 01
        B > login | {login}
        B < login ack | {login ack}
        B > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        B < object id | 08 00 8c 09 00 00 02 01
        B > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        B < object id | 08 00 c0 01 00 00 01 01
        B > add player to waiting room | 0a 00 01 01 02 01
        B < group joined | 0a 00 01 01 02 01
        A < group joined | 0a 00 01 01 02 01
        A > join game object again, same cookie | 07 00 00 00 60 01 81 01 00 00 0c 00
        B < the old object left the room | 0b 00 01 01 00 01
        B < and is gone | 09 00 00 01
        A < the new SID | 08 00 60 01 00 00 03 01
        ",
    );
}

#[test]
fn a_personal_party_takes_other_players_and_its_end_tells_them() {
    Table::new().run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > join player | 07 00 00 00 01 00 01 8b ff ff 01 00
        A < object id | 08 00 01 00 00 00 00 01
        A > join personal party | 07 00 00 00 02 00 04 8b fd ff 04 00
        A < object id | 08 00 02 00 00 00 01 01
        A > add player to party | 0a 00 01 01 00 01
        A < group joined | 0a 00 01 01 00 01
        B > login | {login}
        B < login ack | {login ack}
        B > join player | 07 00 00 00 01 00 01 8b ff ff 01 00
        B < object id | 08 00 01 00 00 00 02 01
        B > join personal party | 07 00 00 00 02 00 04 8b fd ff 04 00
        B < a party of its own | 08 00 02 00 00 00 03 01
        B > add player to A's party | 0a 00 01 01 02 01
        B < group joined | 0a 00 01 01 02 01
        A < group joined | 0a 00 01 01 02 01
        A ! hang up
        B < A's player left the party | 0b 00 01 01 00 01
        B < and is gone | 09 00 00 01
        B < the party is gone | 09 00 01 01
        ",
    );
}
