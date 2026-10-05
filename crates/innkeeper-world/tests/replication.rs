//! Lock and replication transcripts: locks taken, refused and freed, property updates and remote calls
//! passed to the other replicas, and the mirror answering a late joiner. Bytes from messages.md 3.2 and
//! captures.md 11; the line syntax is in common/mod.rs.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::Table;

/// Two players in the Clubhouse waiting room: A has the game object 0x0100 and the player 0x0101, the
/// room is 0x0102, and B has the game object 0x0103 and the player 0x0104.
const WAITING_ROOM: &str = "
    A > login | {login}
    A < login ack | {login ack}
    A > join game object | 07 00 00 00 60 01 81 01 00 00 0c 00
    A < object id | 08 00 60 01 00 00 00 01
    A > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
    A < object id | 08 00 8c 09 00 00 01 01
    A > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
    A < object id | 08 00 c0 01 00 00 02 01
    A > add player to waiting room | 0a 00 02 01 01 01 02 03 12
    A < group joined | 0a 00 02 01 01 01
    B > login | {login}
    B < login ack | {login ack}
    B > join game object | 07 00 00 00 60 01 81 01 00 00 0c 00
    B < object id | 08 00 60 01 00 00 03 01
    B > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
    B < object id | 08 00 8c 09 00 00 04 01
    B > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
    B < the same room | 08 00 c0 01 00 00 02 01
    B > add player to waiting room | 0a 00 02 01 04 01 02 03 12
    B < echo to the joiner | 0a 00 02 01 04 01
    A < and to the member | 0a 00 02 01 04 01
";

fn run(steps: &str) {
    Table::new().run(&[WAITING_ROOM, steps].join("\n"));
}

#[test]
fn an_invitation_lock_is_granted_refused_with_the_taken_word_and_freed() {
    run("
        # A invites B: the inviter's script has no SID, so fromSID is 0 and the reply goes to SID 0.
        A > lock A's and B's players | 04 00 00 00 01 01 04 01
        A < lock granted | 00 00 00 00 04 00
        # B's own player is taken, which the client shows as someone inviting B.
        B > lock B's and A's players | 04 00 00 00 04 01 01 01
        B < refused, 0x0104 is taken | 01 00 00 00 04 04 01 00
        B > unlock is ignored for locks B lacks | 06 00 00 00 04 01 01 01
        B > still refused | 04 00 00 00 04 01 01 01
        B < refused, 0x0104 is taken | 01 00 00 00 04 04 01 00
        A > unlock, no reply | 06 00 00 00 01 01 04 01
        B > now B may invite A | 04 00 00 00 04 01 01 01
        B < lock granted | 00 00 00 00 04 00
        A > taking a lock again is granted to its holder only | 04 00 00 00 01 01
        A < refused, 0x0101 is B's | 01 00 00 00 04 01 01 00
    ");
}

#[test]
fn the_hang_up_of_the_holder_frees_its_locks() {
    run("
        B > lock both players | 04 00 00 00 04 01 01 01
        B < lock granted | 00 00 00 00 04 00
        A > refused | 04 00 00 00 01 01 04 01
        A < refused, 0x0101 is taken | 01 00 00 00 04 01 01 00
        B ! hang up
        A < B's player left the room | 0b 00 02 01 04 01
        A < and is freed | 09 00 04 01
        A > lock again | 04 00 00 00 01 01
        A < lock granted | 00 00 00 00 04 00
    ");
}

#[test]
fn property_updates_and_remote_calls_reach_the_other_replicas_only() {
    run("
        A > set A's name | 0e 00 01 01 01 01 05 00 67 75 79 00
        B < the name for B's replica | 0e 00 01 01 01 01 05 00 67 75 79 00
        A > set A's game and room | 0d 00 01 01 01 01 09 00 78 00 18 00 12 03
        B < the same integers | 0d 00 01 01 01 01 09 00 78 00 18 00 12 03
        # An invitation room sets a property of the invited player; its holder gets the update.
        B > set A's game | 0d 00 01 01 04 01 09 00 05 00
        A < the update | 0d 00 01 01 04 01 09 00 05 00
        A > call a method on the room | 19 00 02 01 d9 00 01 00
        B < the call | 19 00 02 01 d9 00 01 00
        B > update a SID nobody holds | 0d 00 09 09 04 01 09 00 05 00
    ");
}

#[test]
fn a_late_joiner_reads_the_mirrored_properties() {
    let steps = "
        A > set A's name | 0e 00 01 01 01 01 05 00 67 75 79 00
        B < the name for B's replica | 0e 00 01 01 01 01 05 00 67 75 79 00
        A > set A's game | 0d 00 01 01 01 01 09 00 78 00
        B < the game | 0d 00 01 01 01 01 09 00 78 00
        C > login | {login}
        C < login ack | {login ack}
        C > join game object | 07 00 00 00 60 01 81 01 00 00 0c 00
        C < object id | 08 00 60 01 00 00 05 01
        C > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        C < object id | 08 00 8c 09 00 00 06 01
        C > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        C < the same room | 08 00 c0 01 00 00 02 01
        C > add player to waiting room | 0a 00 02 01 06 01 02 03 12
        C < echo to the joiner | 0a 00 02 01 06 01
        A < to a member | 0a 00 02 01 06 01
        B < to a member | 0a 00 02 01 06 01
        # Nobody set offset 0x30, so it is left out.
        C > getProp of A's player | 20 00 01 01 01 01 05 00 09 00 30 00
        C < SetMsg | 21 00 01 01 00 00 01 05 00 67 75 79 00 00 09 00 78 00
        # B and C set nothing: their name is empty text and their game 0.
        C > member properties of the room | 1f 01 02 01 02 01 05 00 09 00
        C < one row per member | 1f 00 02 01 02 01 03 00 02 00 05 00 01 00 09 00 00 00 \
            01 01 67 75 79 00 78 00 04 01 00 00 00 06 01 00 00 00
        A ! hang up
        B < A's player left the room | 0b 00 02 01 01 01
        C < A's player left the room | 0b 00 02 01 01 01
        B < and is freed | 09 00 01 01
        C < and is freed | 09 00 01 01
        C > getProp of the freed player gets no reply | 20 00 01 01 01 01 05 00
    ";
    run(steps);
}

/// The live Clubhouse entry (captures.md 11): 26, three setStr and two setInt are absorbed, and a second
/// player then reads the persona the first one published.
#[test]
fn the_live_clubhouse_entry_is_absorbed_and_mirrored() {
    let steps = "
        A > login | {login}
        A < login ack | {login ack}
        A > join game object | 07 00 00 00 60 01 81 01 00 00 0c 00
        A < object id | 08 00 60 01 00 00 00 01
        A > join game object again | 07 00 00 00 60 01 81 01 00 00 0c 00
        A < object id | 08 00 60 01 00 00 01 01
        A > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        A < object id | 08 00 8c 09 00 00 02 01
        A > command 26 | 1a 00 02 01 02 01
        A > set name | 0e 00 02 01 02 01 05 00 67 75 79 62 72 75 73 68 00
        A > set looks | 0e 00 02 01 02 01 11 00 03 03 03 03 03 03 03 03 03 03 03 03 03 03 03 03 00
        A > set home | 0e 00 02 01 02 01 17 00 6d 61 69 6e 65 00
        A > set the persona integers | 0d 00 02 01 02 01 0c 00 d4 66 0d 00 66 2e 0e 00 2b 07 0f 00 ff fe \
            10 00 ff fe 13 00 01 00 16 00 1e 00 1b 00 00 00 1c 00 00 00
        A > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        A < object id | 08 00 c0 01 00 00 03 01
        A > add player to waiting room | 0a 00 03 01 02 01 02 03 12
        A < group joined | 0a 00 03 01 02 01
        A > set game and room | 0d 00 02 01 02 01 09 00 78 00 18 00 12 03
        B > login | {login}
        B < login ack | {login ack}
        B > getProp of A's player | 20 00 02 01 02 01 05 00 17 00 0c 00 18 00
        B < SetMsg | 21 00 02 01 00 00 01 05 00 67 75 79 62 72 75 73 68 00 \
            01 17 00 6d 61 69 6e 65 00 00 0c 00 d4 66 00 18 00 12 03
    ";
    Table::new().run(steps);
}
