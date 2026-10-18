//! Land occupancy, the waiting-room version gate, a full room and the persona name.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use std::sync::Arc;

use common::{Table, LOGIN};
use innkeeper_world::{
    ClientMessage, ClientVersion, ConnectionId, Cookie, Delivery, GroupJoin, GroupKey, HostMessage,
    JoinNet, LandCatalog, LandType, MemoryStore, ObjectKind, ObjectStore, PlayerSession, Sid,
    Store, World,
};

const CLUBHOUSE: LandType = LandType(1);
const OCCUPANCY_REQUEST: &str = "2f 01 00 00 00 00";
/// The 47/1 header: two unread words, then the row count 7.
const OCCUPANCY_HEADER: &str = "2f 01 00 00 00 00 00 00 00 00 07 00";
const WAITING_ROOM_SIZE: u64 = 128;
const WAITING_ROOM: GroupKey = GroupKey {
    kind: ObjectKind::LandGroup,
    land_type: CLUBHOUSE,
    parameter: 1,
    within: None,
};

fn hex(text: &str) -> Vec<u8> {
    let pairs = text.split_whitespace();
    pairs
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

/// The 47/1 reply: Clubhouse, SierraLand, CasinoLand and the Arena at maximum 64 (the Clubhouse
/// holding `clubhouse` players, the Arena holding `arena`), then the three RPG titles at 100.
fn occupancy(clubhouse: u8, arena: u8) -> String {
    let lsci = format!(
        "07 01 01 40 {clubhouse:02x} 07 02 01 40 00 07 03 01 40 00 07 05 01 40 {arena:02x}"
    );
    let rows = format!("{lsci} 07 04 01 64 00 07 06 01 64 00 07 08 01 64 00");
    format!("{OCCUPANCY_HEADER} {rows}")
}

/// `who` asks for the land occupancy and is told `clubhouse` players are in the Clubhouse.
fn occupancy_is(who: char, clubhouse: u8) -> String {
    let reply = occupancy(clubhouse, 0);
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

/// Arena login is land type 5 at the game object's version 1.4.46, then the kind-5 waiting room.
const ARENA_LOGIN: &str =
    "35 00 00 00 05 01 04 2e a1 86 01 00 01 1d 7a 01 66 16 66 18 73 03 00 00 \
                           67 75 79 62 72 75 73 68 00";

#[test]
fn an_arena_client_at_1_4_46_logs_in_and_enters_the_waiting_room() {
    let mut table = Table::new();
    let present = occupancy(0, 1);
    table.run(&format!(
        "
        A > arena login 1.4.46 | {ARENA_LOGIN}
        A < login ack | {{login ack}}
        A > join player | 07 00 00 00 8c 09 01 05 ff ff 1e 00
        A < object id | 08 00 8c 09 00 00 00 01
        A > join arena waiting room | 07 00 00 00 c0 01 05 05 01 00 80 00
        A < object id | 08 00 c0 01 00 00 01 01
        A > add player, version 1.4.46 | 0a 00 01 01 00 01 01 04 2e
        A < group joined | 0a 00 01 01 00 01
        A > land occupancy | {OCCUPANCY_REQUEST}
        A < one player in the arena | {present}
        "
    ));
    assert_eq!(
        table.objects.member_count(GroupKey {
            kind: ObjectKind::LandGroup,
            land_type: LandType(5),
            parameter: 1,
            within: None,
        }),
        1
    );
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
        B < news of A, who was there first | 0a 00 01 01 00 01
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
fn a_persona_name_is_kept_as_the_accounts_persona_and_a_second_one_replaces_it() {
    let store = Arc::new(MemoryStore::new());
    let world = World::stored(store.clone());
    let mut objects = ObjectStore::new();
    let connection = ConnectionId(1);
    let mut player = PlayerSession::new();
    let login = ClientMessage::parse(&hex(LOGIN)).unwrap();
    let _ = player.handle(&world, &mut objects, connection, &login);
    let id = player.account().unwrap();
    // The Login opened the account under the name it presented (the book enrols).
    assert_eq!(store.account(id).unwrap().unwrap().persona, "guybrush");
    let set = |name: &str| {
        let mut bytes = vec![0x28, 0x04];
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        ClientMessage::parse(&bytes).unwrap()
    };
    let _ = player.handle(&world, &mut objects, connection, &set("Zelda"));
    assert_eq!(store.account(id).unwrap().unwrap().persona, "Zelda");
    assert_eq!(player.persona(), Some("Zelda"));
    let _ = player.handle(&world, &mut objects, connection, &set("Link"));
    assert_eq!(store.account(id).unwrap().unwrap().persona, "Link");
    // A name a board post cannot carry plays the session and is not kept.
    let _ = player.handle(&world, &mut objects, connection, &set("a|b"));
    assert_eq!(player.persona(), Some("a|b"));
    assert_eq!(store.account(id).unwrap().unwrap().persona, "Link");
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

fn land_join(land_type: u8) -> ClientMessage {
    ClientMessage::JoinNet(JoinNet {
        cookie: Cookie(7),
        kind: ObjectKind::LandGroup,
        land_type: LandType(land_type),
        parameter: 1,
        size: 100,
    })
}

fn set_persona(name: &str) -> ClientMessage {
    let mut bytes = vec![0x28, 0x04];
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);
    ClientMessage::parse(&bytes).unwrap()
}

#[test]
fn an_rpg_sessions_character_name_renames_the_session_and_leaves_the_stored_persona() {
    for title in [4, 6, 8] {
        let store = Arc::new(MemoryStore::new());
        let world = World::stored(store.clone());
        let mut objects = ObjectStore::new();
        let connection = ConnectionId(1);
        let mut player = PlayerSession::new();
        let login = ClientMessage::parse(&hex(LOGIN)).unwrap();
        let _ = player.handle(&world, &mut objects, connection, &login);
        let id = player.account().unwrap();
        let _ = player.handle(&world, &mut objects, connection, &land_join(title));
        let _ = player.handle(&world, &mut objects, connection, &set_persona("Jutara"));
        assert_eq!(player.persona(), Some("Jutara"));
        assert_eq!(store.account(id).unwrap().unwrap().persona, "guybrush");
        assert_eq!(objects.online()[0].persona, "Jutara");
    }
}

#[test]
fn a_hub_land_sessions_persona_name_is_still_stored() {
    let store = Arc::new(MemoryStore::new());
    let world = World::stored(store.clone());
    let mut objects = ObjectStore::new();
    let connection = ConnectionId(1);
    let mut player = PlayerSession::new();
    let login = ClientMessage::parse(&hex(LOGIN)).unwrap();
    let _ = player.handle(&world, &mut objects, connection, &login);
    let id = player.account().unwrap();
    for land in [1, 2, 3, 5] {
        let _ = player.handle(&world, &mut objects, connection, &land_join(land));
        let name = format!("Hub{land}");
        let _ = player.handle(&world, &mut objects, connection, &set_persona(&name));
        assert_eq!(store.account(id).unwrap().unwrap().persona, name);
    }
}
