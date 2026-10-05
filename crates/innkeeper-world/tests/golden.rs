//! The live logon from tests/golden/logon.txt: byte-exact codecs and the host's replies.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use innkeeper_world::{
    AccountId, ClientMessage, ClientVersion, HostMessage, LandType, PasswordSource, PlayerSession,
    World,
};

const LOGON: &str = include_str!("golden/logon.txt");

struct Row {
    from_client: bool,
    name: String,
    bytes: Vec<u8>,
}

fn rows() -> Vec<Row> {
    let lines = LOGON.lines().filter(|line| !line.starts_with('#'));
    lines
        .map(|line| {
            let (head, hex) = line.split_once('|').unwrap();
            let (direction, name) = head.trim().split_once(' ').unwrap();
            let bytes = hex
                .split_whitespace()
                .map(|pair| u8::from_str_radix(pair, 16).unwrap())
                .collect();
            let from_client = match direction {
                ">" => true,
                "<" => false,
                other => panic!("unknown direction {other}"),
            };
            Row {
                from_client,
                name: name.to_owned(),
                bytes,
            }
        })
        .collect()
}

#[test]
fn every_row_round_trips_byte_for_byte() {
    for row in rows() {
        let encoded = match row.from_client {
            true => ClientMessage::parse(&row.bytes).unwrap().encode(),
            false => HostMessage::parse(&row.bytes).unwrap().encode(),
        };
        assert_eq!(encoded, row.bytes, "{}", row.name);
    }
}

#[test]
fn the_captured_login_decodes_to_the_persona() {
    let Ok(ClientMessage::Login(login)) = ClientMessage::parse(&rows()[0].bytes) else {
        panic!("the first row is a Login");
    };
    assert_eq!(login.name, "guybrush");
    assert_eq!(login.account, AccountId(100_001));
    assert_eq!(login.land_type, LandType(1));
    assert_eq!(login.version, ClientVersion::new(2, 3, 18));
    assert_eq!(login.password_source, PasswordSource::StoredFile);
    assert_eq!(login.prodigy_id, None);
}

#[test]
fn the_player_session_answers_like_the_capture() {
    let world = World::stock();
    let mut player = PlayerSession::new();
    let mut expected: Vec<Vec<u8>> = Vec::new();
    let mut answered: Vec<Vec<u8>> = Vec::new();
    for row in rows() {
        if row.from_client {
            let message = ClientMessage::parse(&row.bytes).unwrap();
            let replies = player.handle(&world, &message);
            answered.extend(replies.iter().map(HostMessage::encode));
        } else {
            expected.push(row.bytes);
        }
    }
    assert_eq!(answered, expected);
}
