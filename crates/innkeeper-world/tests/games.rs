//! Game groups and invitations: SierraLand's invite rooms with two and three players (invite, lock
//! conflict, accept, decline, cancel, start, relay, one player dropping) and Red Baron's whole path from
//! the invitation to the cockpit. Bytes are the ones the stock clients sent against `innkeeperd` on
//! 2026-10-15 (docs/server/games.md section 2); the line syntax is in common/mod.rs.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use std::collections::BTreeSet;

use common::Table;
use innkeeper_world::{ConnectionId, FixedClock, GamePhase, GameType, HostTime, Sid};

/// Player A is connection 'A' (65), B 'B' (66), C 'C' (67), as in common/mod.rs.
const A: ConnectionId = ConnectionId(65);
const B: ConnectionId = ConnectionId(66);
const C: ConnectionId = ConnectionId(67);

/// A and B in SierraLand's waiting room (land type 2, place 1): player objects 0x0100 and 0x0102,
/// the room 0x0101. B is told of A when it arrives, and A of B (docs/server/games.md section 2).
const TWO_IN_THE_ROOM: &str = "
    A > login | {login}
    A < login ack | {login ack}
    A > joinNet player, kind 1, 32 properties | 07 00 00 00 ec 13 01 02 ff ff 20 00
    A < SID 0x0100 | 08 00 ec 13 00 00 00 01
    A > joinNet waiting room, kind 5, place 1 | 07 00 00 00 b4 01 05 02 01 00 80 00
    A < SID 0x0101 | 08 00 b4 01 00 00 01 01
    A > add the player to the room, v2.3.18 | 0a 00 01 01 00 01 02 03 12
    A < echo | 0a 00 01 01 00 01
    B > login | {login}
    B < login ack | {login ack}
    B > joinNet player | 07 00 00 00 ec 13 01 02 ff ff 20 00
    B < SID 0x0102 | 08 00 ec 13 00 00 02 01
    B > joinNet waiting room | 07 00 00 00 b4 01 05 02 01 00 80 00
    B < the same room | 08 00 b4 01 00 00 01 01
    B > add the player to the room | 0a 00 01 01 02 01 02 03 12
    B < echo | 0a 00 01 01 02 01
    A < news of B | 0a 00 01 01 02 01
    B < news of A, who was there first | 0a 00 01 01 00 01
";

/// A makes one more object of its own (a plain kind 1 object, SID 0x0103) so that the two-player scripts
/// number their SIDs as the three-player ones do, where C's player takes 0x0103.
const A_SPARE_OBJECT: &str = "
    A > joinNet a spare object | 07 00 00 00 10 27 01 02 ff ff 01 00
    A < SID 0x0103 | 08 00 10 27 00 00 03 01
";

/// C arrives as well: player 0x0103; A and B hear of it, C hears of A and B.
const C_JOINS: &str = "
    C > login | {login}
    C < login ack | {login ack}
    C > joinNet player | 07 00 00 00 ec 13 01 02 ff ff 20 00
    C < SID 0x0103 | 08 00 ec 13 00 00 03 01
    C > joinNet waiting room | 07 00 00 00 b4 01 05 02 01 00 80 00
    C < the same room | 08 00 b4 01 00 00 01 01
    C > add the player to the room | 0a 00 01 01 03 01 02 03 12
    C < echo | 0a 00 01 01 03 01
    A < news of C | 0a 00 01 01 03 01
    B < news of C | 0a 00 01 01 03 01
    C < news of A | 0a 00 01 01 00 01
    C < news of B | 0a 00 01 01 02 01
";

/// A invites B (inviting room, steps 1 to 6 of the flow): A's invitation object 0x0104, the lock on
/// both players, the invitation (type 12: game 530, the object, B, A), B's object 0x0105, its type 56.
const A_INVITES_B: &str = "
    A > joinNet invitation object, 25 properties | 07 00 00 00 fc 20 01 02 ff ff 19 00
    A < SID 0x0104 | 08 00 fc 20 00 00 04 01
    A > lock A and B | 04 00 04 01 00 01 02 01
    A < lock granted to the invitation object | 00 00 04 01 04 00
    A > invitation to B, type 12 | 02 00 02 01 00 01 0c 00 12 02 01 00 04 01 02 01 00 01 07 00 2a 00
    B < the invitation | 02 00 02 01 00 01 0c 00 12 02 01 00 04 01 02 01 00 01 07 00 2a 00
    B > joinNet its invitation object | 07 00 00 00 ec 20 01 02 ff ff 19 00
    B < SID 0x0105 | 08 00 ec 20 00 00 05 01
    B > type 56 to A's object: mine is 0x0105 | 02 00 04 01 02 01 38 00 05 01
    A < relayed | 02 00 04 01 02 01 38 00 05 01
    A > type 30 to B's object: no answer yet | 02 00 05 01 00 01 1e 00 03 00 01 00
    B < relayed | 02 00 05 01 00 01 1e 00 03 00 01 00
";

/// B accepts (type 13) and A starts: it tells nobody who has not answered (type 57 to no one), makes
/// the game group 0x0106 (kind 2, game 28, an own group for 4), announces it with type 58 and unlocks.
const B_ACCEPTS_A_STARTS: &str = "
    B > accept, type 13 | 02 00 04 01 02 01 0d 00
    A < relayed | 02 00 04 01 02 01 0d 00
    A > type 30 to B's object: accepted | 02 00 05 01 00 01 1e 00 01 00 01 00
    B < relayed | 02 00 05 01 00 01 1e 00 01 00 01 00
    A > multicast type 57 to nobody | 1c 00 01 00 00 39 00 04 01 12 02 01 00
    A > joinNet game group, kind 2, game 28, own, 4 players | 07 00 00 00 00 23 02 1c fe ff 04 00
    A < SID 0x0106 | 08 00 00 23 00 00 06 01
    A > type 58 to B's object: group 0x0106, 2 players | 02 00 05 01 00 01 3a 00 06 01 02 00
    B < relayed | 02 00 05 01 00 01 3a 00 06 01 02 00
    A > unlock A and B | 06 00 04 01 00 01 02 01
";

/// Both sit in the game group: command 30, GrpJoin, GrpMem. The second one in finds the first.
const BOTH_JOIN_THE_GAME: &str = "
    A > register the group | 1e 06 01
    A > GrpJoin | 0a 00 06 01 00 01
    A < echo | 0a 00 06 01 00 01
    A > GrpMem | 0c 00 06 01 00 01
    A < members | 0c 00 06 01 00 00 00 01
    B > register the group | 1e 06 01
    B > GrpJoin | 0a 00 06 01 02 01
    B < echo | 0a 00 06 01 02 01
    A < B joined | 0a 00 06 01 02 01
    B > GrpMem | 0c 00 06 01 02 01
    B < members | 0c 00 06 01 00 00 00 01 02 01
    B > leaves its invitation object | 09 00 05 01
    A > leaves its invitation object | 09 00 04 01
";

fn script(parts: &[&str]) -> String {
    parts.join("\n")
}

fn table_with(parts: &[&str]) -> Table {
    let mut table = Table::new();
    table.run(&script(parts));
    table
}

#[test]
fn an_invitation_locks_both_players_and_forms_a_game_group_of_two() {
    let mut table = Table::new();
    table.run(&script(&[TWO_IN_THE_ROOM, A_SPARE_OBJECT, A_INVITES_B]));
    let open = table.objects.invitation_of(A).unwrap();
    assert_eq!(open.from, Sid(0x0104));
    assert_eq!(open.locks.len(), 2);
    table.run(B_ACCEPTS_A_STARTS);
    assert!(table.objects.invitation_of(A).is_none(), "unlocked");
    let forming = table.objects.game_session(Sid(0x0106)).unwrap();
    assert_eq!(forming.game, GameType(28));
    assert_eq!((forming.leader, forming.capacity), (A, 4));
    assert_eq!(forming.phase, GamePhase::Forming);
    assert!(
        forming.invitation.is_some(),
        "formed while A's invitation was open"
    );
    table.run(BOTH_JOIN_THE_GAME);
    let playing = table.objects.game_session(Sid(0x0106)).unwrap();
    assert_eq!(playing.phase, GamePhase::Gathered);
    assert_eq!(playing.registered, BTreeSet::from([A, B]));
    assert_eq!(table.objects.game_sessions().count(), 1);
}

#[test]
fn game_messages_reach_every_player_of_the_group_the_sender_included() {
    let mut table = table_with(&[
        TWO_IN_THE_ROOM,
        A_SPARE_OBJECT,
        A_INVITES_B,
        B_ACCEPTS_A_STARTS,
        BOTH_JOIN_THE_GAME,
    ]);
    table.run(
        "
        # Each sets property 9 of its own player (the room number, here 530) for the other's replica.
        A > setInt on A's own player | 0d 00 00 01 ff ff 09 00 12 02
        B < the replica is told | 0d 00 00 01 ff ff 09 00 12 02
        # Graffiti's type 55 to the group, from the player.
        A > Send to the group | 02 00 06 01 00 01 37 00 fd 2a 00
        A < own copy | 02 00 06 01 00 01 37 00 fd 2a 00
        B < copy | 02 00 06 01 00 01 37 00 fd 2a 00
        B > Send to the group | 02 00 06 01 02 01 37 00 f0 07 02 00
        A < copy | 02 00 06 01 02 01 37 00 f0 07 02 00
        B < own copy | 02 00 06 01 02 01 37 00 f0 07 02 00
        # A Send to one player goes only to its holder.
        A > Send to B's player | 02 00 02 01 00 01 64 00 01 00
        B < copy | 02 00 02 01 00 01 64 00 01 00
        ",
    );
}

#[test]
fn a_player_who_drops_mid_game_is_removed_from_the_group_and_the_room() {
    let mut table = table_with(&[
        TWO_IN_THE_ROOM,
        A_SPARE_OBJECT,
        A_INVITES_B,
        B_ACCEPTS_A_STARTS,
        BOTH_JOIN_THE_GAME,
    ]);
    table.run(
        "
        B ! hang up
        A < B leaves the waiting room | 0b 00 01 01 02 01
        A < B leaves the game group | 0b 00 06 01 02 01
        A < B's player is gone | 09 00 02 01
        ",
    );
    let left = table.objects.game_session(Sid(0x0106)).unwrap();
    assert_eq!(left.phase, GamePhase::Forming, "one player is no game");
    // A's own group stays for A; GrpMem now lists A alone.
    table.run(
        "
        A > GrpMem | 0c 00 06 01 00 01
        A < members | 0c 00 06 01 00 00 00 01
        ",
    );
}

#[test]
fn the_group_outlives_its_creator_while_a_registered_player_is_in_it() {
    let mut table = table_with(&[
        TWO_IN_THE_ROOM,
        A_SPARE_OBJECT,
        A_INVITES_B,
        B_ACCEPTS_A_STARTS,
        BOTH_JOIN_THE_GAME,
    ]);
    table.run(
        "
        A > frees the game group, as the land does at a hand-off | 09 00 06 01
        B < A left | 0b 00 06 01 00 01
        B > GrpMem | 0c 00 06 01 02 01
        B < B alone | 0c 00 06 01 00 00 02 01
        ",
    );
    assert!(table.objects.game_session(Sid(0x0106)).is_some());
    table.run(
        "
        B > frees it too | 09 00 06 01
        ",
    );
    assert!(
        table.objects.game_session(Sid(0x0106)).is_none(),
        "the last reference went"
    );
}

#[test]
fn a_hang_up_during_the_invitation_frees_the_locks_for_the_next_inviter() {
    let mut table = table_with(&[TWO_IN_THE_ROOM, C_JOINS, A_INVITES_B]);
    table.run(
        "
        C > joinNet invitation object | 07 00 00 00 fc 20 01 02 ff ff 19 00
        C < SID 0x0106 | 08 00 fc 20 00 00 06 01
        C > lock C and B | 04 00 06 01 03 01 02 01
        C < refused: B is engaged | 01 00 06 01 04 02 01 00
        A ! hang up
        B < A leaves the waiting room | 0b 00 01 01 00 01
        C < A leaves the waiting room | 0b 00 01 01 00 01
        B < A's player is gone | 09 00 00 01
        C < A's player is gone | 09 00 00 01
        C > lock C and B again | 04 00 06 01 03 01 02 01
        C < granted | 00 00 06 01 04 00
        ",
    );
    assert!(table.objects.invitation_of(A).is_none());
    assert_eq!(table.objects.invitation_of(C).unwrap().locks.len(), 2);
}

#[test]
fn a_decline_and_a_cancel_free_the_player_for_someone_else() {
    let mut table = table_with(&[TWO_IN_THE_ROOM, C_JOINS, A_INVITES_B]);
    table.run(
        "
        B > decline, type 14 | 02 00 04 01 02 01 0e 00
        A < relayed | 02 00 04 01 02 01 0e 00
        # A cancels: type 57 to B (a multicast to B's invitation object), then unlock.
        A > multicast type 57 to B's object | 1c 00 01 01 00 05 01 39 00 04 01 12 02 01 00
        B < the cancel | 02 00 05 01 00 01 39 00 04 01 12 02 01 00
        A > unlock A and B | 06 00 04 01 00 01 02 01
        C > joinNet invitation object | 07 00 00 00 fc 20 01 02 ff ff 19 00
        C < SID 0x0106 | 08 00 fc 20 00 00 06 01
        C > lock C and B | 04 00 06 01 03 01 02 01
        C < granted | 00 00 06 01 04 00
        ",
    );
    assert!(table.objects.invitation_of(A).is_none());
    assert!(table.objects.invitation_of(C).is_some());
}

/// Three players: A invites B and C, all three sit in one group for 4.
const THREE_START: &str = "
    A > joinNet invitation object | 07 00 00 00 fc 20 01 02 ff ff 19 00
    A < SID 0x0104 | 08 00 fc 20 00 00 04 01
    A > lock A, B and C | 04 00 04 01 00 01 02 01 03 01
    A < granted | 00 00 04 01 04 00
    A > invitation to B | 02 00 02 01 00 01 0c 00 12 02 02 00 04 01 02 01 03 01 00 01 07 00 2a 00
    B < the invitation | 02 00 02 01 00 01 0c 00 12 02 02 00 04 01 02 01 03 01 00 01 07 00 2a 00
    A > invitation to C | 02 00 03 01 00 01 0c 00 12 02 02 00 04 01 02 01 03 01 00 01 07 00 2a 00
    C < the invitation | 02 00 03 01 00 01 0c 00 12 02 02 00 04 01 02 01 03 01 00 01 07 00 2a 00
    A > joinNet game group for 4 | 07 00 00 00 00 23 02 1c fe ff 04 00
    A < SID 0x0105 | 08 00 00 23 00 00 05 01
    A > unlock | 06 00 04 01 00 01 02 01 03 01
    A > register | 1e 05 01
    A > GrpJoin | 0a 00 05 01 00 01
    A < echo | 0a 00 05 01 00 01
    B > register | 1e 05 01
    B > GrpJoin | 0a 00 05 01 02 01
    B < echo | 0a 00 05 01 02 01
    A < B joined | 0a 00 05 01 02 01
    C > register | 1e 05 01
    C > GrpJoin | 0a 00 05 01 03 01
    C < echo | 0a 00 05 01 03 01
    A < C joined | 0a 00 05 01 03 01
    B < C joined | 0a 00 05 01 03 01
    C > GrpMem | 0c 00 05 01 03 01
    C < members | 0c 00 05 01 00 00 00 01 02 01 03 01
";

#[test]
fn three_players_share_a_group_and_the_last_to_leave_closes_it() {
    let mut table = table_with(&[TWO_IN_THE_ROOM, C_JOINS, THREE_START]);
    let session = table.objects.game_session(Sid(0x0105)).unwrap();
    assert_eq!(session.phase, GamePhase::Gathered);
    assert_eq!(session.registered, BTreeSet::from([A, B, C]));
    table.run(
        "
        B > Send to the group | 02 00 05 01 02 01 37 00 01
        A < copy | 02 00 05 01 02 01 37 00 01
        B < own copy | 02 00 05 01 02 01 37 00 01
        C < copy | 02 00 05 01 02 01 37 00 01
        C ! hang up
        A < C leaves the waiting room | 0b 00 01 01 03 01
        B < C leaves the waiting room | 0b 00 01 01 03 01
        A < C leaves the game group | 0b 00 05 01 03 01
        B < C leaves the game group | 0b 00 05 01 03 01
        A < C's player is gone | 09 00 03 01
        B < C's player is gone | 09 00 03 01
        B > Send to the group | 02 00 05 01 02 01 37 00 02
        A < copy | 02 00 05 01 02 01 37 00 02
        B < own copy | 02 00 05 01 02 01 37 00 02
        ",
    );
    assert_eq!(
        table.objects.game_session(Sid(0x0105)).unwrap().phase,
        GamePhase::Gathered
    );
}

#[test]
fn a_full_group_counts_players_and_refuses_the_next_one_with_code_2() {
    // A game group for two players: A's second member does not take a place, C's join is refused.
    let mut table = table_with(&[TWO_IN_THE_ROOM, C_JOINS]);
    table.run(
        "
        A > joinNet game group for 2 | 07 00 00 00 00 23 02 1c fe ff 02 00
        A < SID 0x0104 | 08 00 00 23 00 00 04 01
        A > a second object of A's | 07 00 00 00 01 00 03 65 ff ff 01 00
        A < SID 0x0105 | 08 00 01 00 00 00 05 01
        A > GrpJoin player | 0a 00 04 01 00 01
        A < echo | 0a 00 04 01 00 01
        A > GrpJoin the second object | 0a 00 04 01 05 01
        A < echo | 0a 00 04 01 05 01
        B > register | 1e 04 01
        B > GrpJoin | 0a 00 04 01 02 01
        B < echo | 0a 00 04 01 02 01
        A < B joined | 0a 00 04 01 02 01
        C > register | 1e 04 01
        C > GrpJoin | 0a 00 04 01 03 01
        C < Nak 10: group full | 01 00 04 01 0a 02 00 00 00
        ",
    );
}

/// Red Baron: A and B have gone through the invitation (not repeated here) and A made the queue group
/// 0x0104 (kind 4, land type 101, own, 4 players); the shared block gives both its SID.
const RB_QUEUE: &str = "
    A > joinNet the queue, kind 4, type 101, 4 players | 07 00 00 00 00 23 04 65 fd ff 04 00
    A < SID 0x0104 | 08 00 00 23 00 00 04 01
";

/// RB.EXE's start-up for one player: 41/2, the two property reads, host time, its game object, GrpJoin,
/// GrpMem. `{who}` is the connection, `{user}` the shared-block user id, `{obj}` the new SID.
fn rb_start(who: &str, user: &str, obj: &str, members: &str) -> String {
    format!(
        "
    {who} > 41/2 service lookup, type 101 | 29 02 00 00 {user} 65 00 00
    {who} < service located: SID 0x0065 | 29 00 {user} 00 00 65 00
    {who} > 17/0 prop 9 | 11 00 65 00 {user} 09 00
    {who} < setInt 9 = 0x0201 | 0d 00 65 00 65 00 09 00 01 02
    {who} > 17/0 prop 0xF | 11 00 65 00 {user} 0f 00
    {who} < setInt 0xF = 10 minutes | 0d 00 65 00 65 00 0f 00 0a 00
    {who} > host time | 24 02
    {who} < 1994-02-15 12:30:45 | 24 02 5e 01 0f 0c 1e 2d
    {who} > joinNet game object, kind 3, type 101 | 07 00 00 00 {user} 03 65 ff ff 01 00
    {who} < ObjID | 08 00 {user} 00 00 {obj}
    {who} > GrpJoin the queue | 0a 00 04 01 {obj}
    {who} > GrpMem | 0c 00 04 01 {obj}
    {who} < members | {members}
    "
    )
}

fn rb_table() -> Table {
    let mut table = table_with(&[TWO_IN_THE_ROOM, A_SPARE_OBJECT, RB_QUEUE]);
    table.world.clock = Box::new(FixedClock {
        time: HostTime {
            year_1900: 94,
            month0: 1,
            mday: 15,
            hour: 12,
            minute: 30,
            second: 45,
        },
    });
    table
}

#[test]
fn red_baron_starts_up_joins_the_queue_and_trades_aircraft_state() {
    let mut table = rb_table();
    // A: user 0x0100, object 0x0105. Its GrpJoin echo comes back to A alone; its GrpMem lists itself.
    let a = rb_start("A", "00 01", "05 01", "0c 00 04 01 00 00 05 01");
    // The echo is the second line of the join; put it in place.
    let a = a.replace(
        "A > GrpMem",
        "A < echo to the joiner | 0a 00 04 01 05 01\n    A > GrpMem",
    );
    table.run(&a);
    // B: user 0x0102, object 0x0106; A hears the join, B hears its own and lists both.
    let b = rb_start("B", "02 01", "06 01", "0c 00 04 01 00 00 05 01 06 01");
    let b = b.replace(
        "B > GrpMem",
        "B < echo to the joiner | 0a 00 04 01 06 01\n    A < B joined | 0a 00 04 01 06 01\n    B > GrpMem",
    );
    table.run(&b);
    table.run(
        "
        # sendPlayerInfo: 0xC8 to the group with the real sender, once from each.
        A > 0xC8 to the group | 02 00 04 01 05 01 c8 01 41 6e 6e 00
        A < own copy, ignored by RB | 02 00 04 01 05 01 c8 01 41 6e 6e 00
        B < copy | 02 00 04 01 05 01 c8 01 41 6e 6e 00
        B > 0xC8 to the group | 02 00 04 01 06 01 c8 00 42 6f 62 00
        A < copy | 02 00 04 01 06 01 c8 00 42 6f 62 00
        B < own copy | 02 00 04 01 06 01 c8 00 42 6f 62 00
        # status 0: playing. Nothing comes back.
        A > 50 playing | 32 00 00 00 00 01
        B > 50 playing | 32 00 00 00 02 01
        # broadcastState: a multicast to {group, own SID}; the update reaches the other and returns to the sender.
        A > 0xC9 state | 1c 05 01 02 00 04 01 05 01 c9 00 00 00 02
        A < to the group, own copy | 02 00 04 01 05 01 c9 00 00 00 02
        B < to the group | 02 00 04 01 05 01 c9 00 00 00 02
        A < to its own SID: the echo that frees the next update | 02 00 05 01 05 01 c9 00 00 00 02
        B > 0xC9 state | 1c 06 01 02 00 04 01 06 01 c9 00 00 00 02
        A < to the group | 02 00 04 01 06 01 c9 00 00 00 02
        B < to the group, own copy | 02 00 04 01 06 01 c9 00 00 00 02
        B < to its own SID | 02 00 06 01 06 01 c9 00 00 00 02
        ",
    );
    let queue = table.objects.game_session(Sid(0x0104)).unwrap();
    assert_eq!(queue.game, GameType::RED_BARON);
    assert_eq!(queue.game.name(), Some("Red Baron"));
    assert_eq!((queue.leader, queue.capacity), (A, 4));
    assert_eq!(queue.phase, GamePhase::Gathered);
}

#[test]
fn a_red_baron_pilot_who_leaves_or_drops_is_removed_for_the_others() {
    let mut table = rb_table();
    let a = rb_start("A", "00 01", "05 01", "0c 00 04 01 00 00 05 01").replace(
        "A > GrpMem",
        "A < echo to the joiner | 0a 00 04 01 05 01\n    A > GrpMem",
    );
    table.run(&a);
    let b = rb_start("B", "02 01", "06 01", "0c 00 04 01 00 00 05 01 06 01").replace(
        "B > GrpMem",
        "B < echo to the joiner | 0a 00 04 01 06 01\n    A < B joined | 0a 00 04 01 06 01\n    B > GrpMem",
    );
    table.run(&b);
    // leaveGame: status 1, GrpDel, ObjFree of its own object. A is told once of each.
    table.run(
        "
        B > 50 leaving | 32 01 00 00 02 01
        B > GrpDel | 0b 00 04 01 06 01
        A < B left | 0b 00 04 01 06 01
        B > ObjFree | 09 00 06 01
        B ! hang up
        A < B leaves the waiting room | 0b 00 01 01 02 01
        A < B's player is gone | 09 00 02 01
        ",
    );
    // A pilot who drops without leaving is taken out the same way.
    let mut dropped = rb_table();
    dropped.run(&a);
    dropped.run(&b);
    dropped.run(
        "
        B ! hang up
        A < B leaves the waiting room | 0b 00 01 01 02 01
        A < B's player is gone | 09 00 02 01
        A < B leaves the queue | 0b 00 04 01 06 01
        A < B's object is gone | 09 00 06 01
        ",
    );
}

#[test]
fn three_red_baron_pilots_fit_a_queue_for_four_though_each_has_two_objects() {
    // Each of A, B and C has its SierraLand player and a game object in the queue: six members, three players.
    let mut table = table_with(&[TWO_IN_THE_ROOM, C_JOINS, RB_QUEUE_3]);
    table.run(
        "
        A > joinNet game object | 07 00 00 00 00 01 03 65 ff ff 01 00
        A < SID 0x0105 | 08 00 00 01 00 00 05 01
        A > GrpJoin the queue | 0a 00 04 01 05 01
        A < echo | 0a 00 04 01 05 01
        A > GrpJoin its player too | 0a 00 04 01 00 01
        A < echo | 0a 00 04 01 00 01
        B > joinNet game object | 07 00 00 00 02 01 03 65 ff ff 01 00
        B < SID 0x0106 | 08 00 02 01 00 00 06 01
        B > GrpJoin the queue | 0a 00 04 01 06 01
        B < echo | 0a 00 04 01 06 01
        A < B joined | 0a 00 04 01 06 01
        B > GrpJoin its player too | 0a 00 04 01 02 01
        B < echo | 0a 00 04 01 02 01
        A < B's player joined | 0a 00 04 01 02 01
        C > joinNet game object | 07 00 00 00 03 01 03 65 ff ff 01 00
        C < SID 0x0107 | 08 00 03 01 00 00 07 01
        C > GrpJoin the queue | 0a 00 04 01 07 01
        C < echo | 0a 00 04 01 07 01
        A < C joined | 0a 00 04 01 07 01
        B < C joined | 0a 00 04 01 07 01
        C > GrpJoin its player too | 0a 00 04 01 03 01
        C < echo | 0a 00 04 01 03 01
        A < C's player joined | 0a 00 04 01 03 01
        B < C's player joined | 0a 00 04 01 03 01
        C > GrpMem | 0c 00 04 01 07 01
        C < all six | 0c 00 04 01 00 00 05 01 00 01 06 01 02 01 07 01 03 01
        ",
    );
    assert_eq!(table.objects.game_session(Sid(0x0104)).unwrap().capacity, 4);
}

const RB_QUEUE_3: &str = RB_QUEUE;

#[test]
fn requests_that_are_not_red_barons_get_no_service_answer() {
    Table::new().run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > 41/2 for land type 7 | 29 02 00 00 00 01 07 00 00
        A > 17/0 on some other service | 11 00 66 00 00 01 09 00
        A > 50 | 32 00 00 00 00 01
        ",
    );
}

#[test]
fn the_connection_that_registers_a_group_nobody_made_gets_nothing() {
    Table::new().run(
        "
        A > login | {login}
        A < login ack | {login ack}
        A > register a SID that is no game group | 1e 99 01
        A > GrpJoin it | 0a 00 99 01 00 01
        A < Nak 10: invalid group | 01 00 99 01 0a 01 00 00 00
        ",
    );
    let _ = (B, C);
}
