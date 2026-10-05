//! Land occupancy, the waiting-room version gate, a full room and the persona name.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::{Table, LOGIN};
use innkeeper_world::{
    ClientMessage, ClientVersion, ConnectionId, Cookie, Delivery, GroupJoin, GroupKey, HostMessage,
    JoinNet, LandCatalog, LandType, ObjectKind, ObjectStore, PlayerSession, Sid, World,
};

const CLUBHOUSE: LandType = LandType(1);
const OCCUPANCY_REQUEST: &str = "2f 01 00 00 00 00";
/// The 47/1 header: two unread words, then the row count 3.
const OCCUPANCY_HEADER: &str = "2f 01 00 00 00 00 00 00 00 00 03 00";
const WAITING_ROOM_SIZE: u64 = 128;
const WAITING_ROOM: GroupKey = GroupKey {
    kind: ObjectKind::LandGroup,
    land_type: CLUBHOUSE,
    parameter: 1,
};

fn hex(text: &str) -> Vec<u8> {
    let pairs = text.split_whitespace();
    pairs
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

/// The 47/1 reply: the three stock lands with maximum 64, the Clubhouse holding `clubhouse` players.
fn occupancy(clubhouse: u8) -> String {
    let rows = format!("07 01 01 40 {clubhouse:02x} 07 02 01 40 00 07 03 01 40 00");
    format!("{OCCUPANCY_HEADER} {rows}")
}

/// `who` asks for the land occupancy and is told `clubhouse` players are in the Clubhouse.
fn occupancy_is(who: char, clubhouse: u8) -> String {
    let reply = occupancy(clubhouse);
    format!("{who} > land occupancy | {OCCUPANCY_REQUEST}\n{who} < {clubhouse} present | {reply}")
}

/// `who` logs in and joins its player object, which gets SID `player` (the room is 0x0101).
fn log_in_and_join_player(who: char, player: u8) -> String {
    format!(
        "
        {who} > login | {{login}}
        {who} < login ack | {{login ack}}
        {who} > join player | 07 00 00 00 8c 09 01 01 ff ff 1e 00
        {who} < object id | 08 00 8c 09 00 00 {player:02x} 01
        {who} > join waiting room | 07 00 00 00 c0 01 05 01 01 00 80 00
        {who} < object id | 08 00 c0 01 00 00 01 01
        "
    )
}

#[test]
fn occupancy_counts_the_clubhouse_waiting_room_and_the_other_lands_stay_empty() {
    let mut table = Table::new();
    table.run(&log_in_and_join_player('A', 0x00));
    table.run(&occupancy_is('A', 0));
    table.run(
        "
        A > add player to waiting room | 0a 00 01 01 00 01 02 03 12
        A < group joined | 0a 00 01 01 00 01
        ",
    );
    table.run(&occupancy_is('A', 1));
    table.run(&log_in_and_join_player('B', 0x02));
    table.run(
        "
        B > add player to waiting room | 0a 00 01 01 02 01 02 03 12
        B < group joined | 0a 00 01 01 02 01
        A < news of B | 0a 00 01 01 02 01
        ",
    );
    table.run(&occupancy_is('A', 2));
    table.run(
        "
        B ! hang up
        A < B left the room | 0b 00 01 01 02 01
        A < B is gone | 09 00 02 01
        ",
    );
    table.run(&occupancy_is('A', 1));
}

#[test]
fn a_client_outside_the_lands_version_range_is_refused_with_code_6_and_not_added() {
    let mut table = Table::new();
    table.world.lands = LandCatalog::stock().limited_to_versions(
        CLUBHOUSE,
        ClientVersion::new(2, 4, 0),
        ClientVersion::new(3, 0, 0),
    );
    table.run(&log_in_and_join_player('A', 0x00));
    table.run(
        "
        A > add player, version 2.3.18 | 0a 00 01 01 00 01 02 03 12
        A < Nak 10, code 6 | 01 00 01 01 0a 06 00 00 00
        A > member list | 0c 00 01 01 00 01
        A < nobody is in the room | 0c 00 01 01 00 00
        A > add player, no version bytes | 0a 00 01 01 00 01
        A < group joined | 0a 00 01 01 00 01
        A > member list | 0c 00 01 01 00 01
        A < one member | 0c 00 01 01 00 00 00 01
        ",
    );
    assert_eq!(table.objects.member_count(WAITING_ROOM), 1);
}

#[test]
fn a_version_inside_the_range_is_admitted() {
    let mut table = Table::new();
    table.world.lands = LandCatalog::stock().limited_to_versions(
        CLUBHOUSE,
        ClientVersion::new(2, 4, 0),
        ClientVersion::new(3, 0, 0),
    );
    table.run(&log_in_and_join_player('A', 0x00));
    table.run(
        "
        A > add player, version 2.4.0 | 0a 00 01 01 00 01 02 04 00
        A < group joined | 0a 00 01 01 00 01
        ",
    );
}

fn join_net(kind: ObjectKind, parameter: u16, size: u16) -> ClientMessage {
    ClientMessage::JoinNet(JoinNet {
        cookie: Cookie(1),
        kind,
        land_type: CLUBHOUSE,
        parameter,
        size,
    })
}

fn granted(deliveries: &[Delivery]) -> Sid {
    match deliveries.first().map(|delivery| &delivery.message) {
        Some(HostMessage::ObjId { sid, .. }) => *sid,
        other => panic!("expected an ObjID, got {other:?}"),
    }
}

/// A new player logs in, joins its object and the waiting room, and asks to be added.
fn enter_clubhouse(world: &World, objects: &mut ObjectStore, who: u64) -> Vec<Delivery> {
    let connection = ConnectionId(who);
    let mut player = PlayerSession::new();
    let login = ClientMessage::parse(&hex(LOGIN)).unwrap();
    let _ = player.handle(world, objects, connection, &login);
    let own = join_net(ObjectKind::Object, 0xFFFF, 30);
    let member = granted(&player.handle(world, objects, connection, &own));
    let room = join_net(ObjectKind::LandGroup, 1, WAITING_ROOM_SIZE as u16);
    let group = granted(&player.handle(world, objects, connection, &room));
    let version = Some(ClientVersion::new(2, 3, 18));
    let add = ClientMessage::GroupJoin(GroupJoin {
        group,
        member,
        version,
    });
    player.handle(world, objects, connection, &add)
}

#[test]
fn the_129th_joiner_of_a_128_member_room_gets_nak_10_code_2() {
    let world = World::stock();
    let mut objects = ObjectStore::new();
    for who in 1..=WAITING_ROOM_SIZE {
        let caused = enter_clubhouse(&world, &mut objects, who);
        assert!(matches!(
            caused.first().map(|delivery| &delivery.message),
            Some(HostMessage::GroupJoined { .. })
        ));
    }
    assert_eq!(objects.member_count(WAITING_ROOM), 128);
    let caused = enter_clubhouse(&world, &mut objects, WAITING_ROOM_SIZE + 1);
    let [Delivery {
        message: HostMessage::Nak(nak),
        ..
    }] = caused.as_slice()
    else {
        panic!("expected one Nak, got {caused:?}");
    };
    assert_eq!((nak.which_cmd, nak.which_sub), (10, 2));
    assert_eq!(objects.member_count(WAITING_ROOM), 128);
}

#[test]
fn a_persona_name_replaces_the_one_in_the_login() {
    let world = World::stock();
    let mut objects = ObjectStore::new();
    let connection = ConnectionId(1);
    let mut player = PlayerSession::new();
    let set = ClientMessage::parse(&hex("28 04 65 6c 61 69 6e 65 00")).unwrap();
    assert!(player
        .handle(&world, &mut objects, connection, &set)
        .is_empty());
    assert_eq!(player.persona(), None);
    let login = ClientMessage::parse(&hex(LOGIN)).unwrap();
    let _ = player.handle(&world, &mut objects, connection, &login);
    assert_eq!(player.persona(), Some("guybrush"));
    assert!(player
        .handle(&world, &mut objects, connection, &set)
        .is_empty());
    assert_eq!(player.persona(), Some("elaine"));
}
