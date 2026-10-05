//! Land switches: a program handing the line to the next on the same host, and a new host call.
//! The client's side is docs/protocol/messages.md section 5.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::{Table, LOGIN};
use innkeeper_world::{ClientMessage, ConnectionId, ObjectStore, PlayerSession, Sid, World};

const OCCUPANCY: &str = "2f 01 00 00 00 00 00 00 00 00 03 00 \
                         07 01 01 40 00 07 02 01 40 00 07 03 01 40 00";

/// A program on the same host arrives: `attachScript` frees the previous game object by its SID,
/// asks for the occupancy, joins its own game object (getting `sid`) and asks for the host number.
fn arrive(land_type: u8, old_sid: u8, sid: u8) -> String {
    format!(
        "
        A > free the previous game object | 09 00 {old_sid:02x} 01 00 00
        A > land occupancy | 2f 01 00 00 00 00
        A < occupancy | {OCCUPANCY}
        A > join the game object | 07 00 00 00 60 01 81 {land_type:02x} 00 00 0c 00
        A < object id | 08 00 60 01 00 00 {sid:02x} 01
        A > host number | 24 05
        A < host 7 | 24 05 07 00
        "
    )
}

/// The program leaves: `hub/script.101` export 1 frees its game object before the next one runs.
fn leave(sid: u8) -> String {
    format!("A > free the game object | 09 00 {sid:02x} 01 00 00")
}

/// A logs on and joins its game object of `land_type`, which gets `sid`.
fn log_on(land_type: u8, sid: u8) -> String {
    format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > join the game object | 07 00 00 00 60 01 81 {land_type:02x} 00 00 0c 00
        A < object id | 08 00 60 01 00 00 {sid:02x} 01
        "
    )
}

#[test]
fn a_walk_from_the_hub_to_sierraland_and_back_on_one_host_leaves_nothing_behind() {
    let mut table = Table::new();
    table.run(&log_on(1, 0x00));
    table.run(&leave(0x00));
    assert!(table.objects.is_empty(), "hub left for SierraLand");
    table.run(&arrive(2, 0x00, 0x01));
    table.run(&leave(0x01));
    assert!(table.objects.is_empty(), "SierraLand left for the hub");
    table.run(&arrive(1, 0x01, 0x02));
    table.run("A ! hang up");
    assert!(table.objects.is_empty(), "the hub hung up");
}

#[test]
fn a_walk_across_new_host_calls_logs_in_on_every_leg_and_leaves_nothing_behind() {
    let mut table = Table::new();
    table.run(&log_on(1, 0x00));
    table.run("A ! new call to SierraLand's host");
    assert!(table.objects.is_empty(), "the hub's call was replaced");
    table.run(
        "
        # attachScript frees the old SID, which the new call already released; then it logs in.
        A > free the previous game object | 09 00 00 01 00 00
        A > join before the Login | 07 00 00 00 60 01 81 02 00 00 0c 00
        ",
    );
    table.run(&log_on(2, 0x01));
    table.run("A ! new call back to the hub's host");
    assert!(table.objects.is_empty(), "SierraLand's call was replaced");
    table.run(&log_on(1, 0x02));
    table.run(&leave(0x02));
    assert!(table.objects.is_empty(), "the hub left");
}

#[test]
fn a_new_call_releases_the_inherited_game_object_and_a_returned_game_group() {
    let mut table = Table::new();
    table.run(&log_on(1, 0x00));
    table.run(
        "
        # A game group as GOLF and Red Baron share it, a player in it, and the player's own GrpDel.
        A > join the game group | 07 00 00 00 70 01 02 66 05 00 04 00
        A < object id | 08 00 70 01 00 00 01 01
        A > join the player | 07 00 00 00 8c 09 01 66 ff ff 1e 00
        A < object id | 08 00 8c 09 00 00 02 01
        A > add the player | 0a 00 01 01 02 01
        A < group joined | 0a 00 01 01 02 01
        A > the player leaves the group | 0b 00 01 01 02 01
        A > free the player | 09 00 02 01 00 00
        A ! new call
        # The arriving land frees the returned group by SID; the call already let go of it.
        A > free the returned game group | 09 00 01 01 00 00
        ",
    );
    assert!(table.objects.is_empty());
}

#[test]
fn a_second_login_that_is_refused_leaves_the_player_awaiting_login() {
    let mut table = Table::new();
    table.run(&log_on(1, 0x00));
    let unlisted_land = LOGIN.replacen("01 02 03 12", "09 02 03 12", 1);
    table.run(&format!(
        "
        A > login for a land this host does not run | {unlisted_land}
        A < refused | 01 00 00 00 16 02 00 00 54 68 69 73 20 6c 61 6e 64 20 69 73 20 6e 6f 74 20 6b 6e 6f 77 6e 20 68 65 72 65 2e 00
        A > host number | 24 05
        "
    ));
    assert!(table.objects.is_empty());
}

const ME: ConnectionId = ConnectionId(1);

fn message(text: &str) -> ClientMessage {
    let bytes: Vec<u8> = text
        .split_whitespace()
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect();
    ClientMessage::parse(&bytes).unwrap()
}

fn handle(player: &mut PlayerSession, objects: &mut ObjectStore, text: &str) {
    let _ = player.handle(&World::stock(), objects, ME, &message(text));
}

fn free(sid: Sid) -> ClientMessage {
    ClientMessage::LeaveNet(sid)
}

#[test]
fn only_freeing_a_held_game_object_ends_the_program() {
    let mut objects = ObjectStore::new();
    let mut player = PlayerSession::new();
    let (game_object, person) = (Sid(0x0100), Sid(0x0101));
    let ends = |player: &PlayerSession, objects: &ObjectStore, sid| {
        player.ends_program(objects, ME, &free(sid))
    };
    handle(&mut player, &mut objects, LOGIN);
    handle(
        &mut player,
        &mut objects,
        "07 00 00 00 60 01 81 01 00 00 0c 00",
    );
    handle(
        &mut player,
        &mut objects,
        "07 00 00 00 8c 09 01 01 ff ff 1e 00",
    );
    assert!(ends(&player, &objects, game_object));
    assert!(!ends(&player, &objects, person), "a player object");
    assert!(!ends(&player, &objects, Sid(0x0200)), "a SID nobody holds");
    assert!(!player.ends_program(&objects, ConnectionId(2), &free(game_object)));

    handle(&mut player, &mut objects, "09 00 00 01 00 00");
    assert!(!ends(&player, &objects, game_object), "already freed");
    let _ = player.end_call(&mut objects, ME);
    assert!(!ends(&player, &objects, person), "before the Login");
}
