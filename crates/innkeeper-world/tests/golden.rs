//! Every tests/golden/*.txt file: byte-exact codecs, plus the live logon's replies from logon.txt.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use std::fs;
use std::path::Path;

use innkeeper_world::{
    AccountId, ClientMessage, ClientVersion, ConnectionId, HostMessage, LandType, MessageError,
    ObjExists, ObjectKind, ObjectStore, PasswordSource, PlayerSession, SendMessage, Sid, World,
};

struct Row {
    from_client: bool,
    name: String,
    bytes: Vec<u8>,
}

fn golden_files() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(path).unwrap())
        })
        .collect();
    files.sort();
    files
}

fn rows() -> Vec<Row> {
    golden_files()
        .iter()
        .flat_map(|(_, text)| parse_rows(text))
        .collect()
}

fn logon_rows() -> Vec<Row> {
    let (_, text) = golden_files()
        .into_iter()
        .find(|(name, _)| name == "logon")
        .unwrap();
    parse_rows(&text)
}

fn parse_rows(text: &str) -> Vec<Row> {
    let lines = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
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
fn every_golden_file_has_a_census_header_and_rows() {
    let files = golden_files();
    assert!(files.len() >= 7, "{} golden files", files.len());
    for (name, text) in files {
        let header = text.lines().next().unwrap();
        assert!(header.starts_with('#'), "{name} lacks a header");
        assert!(parse_rows(&text).len() >= 2, "{name} has too few rows");
    }
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

/// The shared-object commands are `w cmd` in LSCI and `b cmd, b 0` in the DOS libraries.
#[test]
fn the_word_and_byte_header_shapes_are_the_same_bytes() {
    for row in rows().iter().filter(|row| row.bytes[1] == 0) {
        let command = row.bytes[0];
        let word_header = u16::from(command).to_le_bytes();
        assert_eq!(word_header, [command, 0], "{}", row.name);
        let with_header = |header: [u8; 2]| [&header[..], &row.bytes[2..]].concat();
        let (as_word, as_bytes) = (with_header(word_header), with_header([command, 0]));
        match row.from_client {
            true => assert_eq!(
                ClientMessage::parse(&as_word),
                ClientMessage::parse(&as_bytes)
            ),
            false => assert_eq!(HostMessage::parse(&as_word), HostMessage::parse(&as_bytes)),
        }
        assert_eq!(as_word, row.bytes, "{}", row.name);
    }
}

#[test]
fn kind_3_parses_and_kind_6_and_unknown_commands_are_errors() {
    let join = |kind| [7, 0, 0, 0, 2, 1, kind, 0x65, 0xFF, 0xFF, 1, 0];
    let Ok(ClientMessage::JoinNet(join_net)) = ClientMessage::parse(&join(3)) else {
        panic!("kind 3 is a joinNet kind");
    };
    assert_eq!(join_net.kind, ObjectKind::SoloObject);
    assert_eq!(
        ClientMessage::parse(&join(6)),
        Err(MessageError::UnknownValue {
            field: "kind",
            value: 6
        })
    );
    for command in [3, 5, 17, 18, 200] {
        let bytes = [command, 0, 2, 1, 2, 1];
        let error = MessageError::UnsupportedCommand(command);
        assert_eq!(ClientMessage::parse(&bytes), Err(error.clone()));
        assert_eq!(HostMessage::parse(&bytes), Err(error));
    }
}

#[test]
fn a_command_only_one_side_sends_is_an_error_on_the_other() {
    assert_eq!(
        ClientMessage::parse(&[0x30, 0, b'x', 0]),
        Err(MessageError::UnsupportedCommand(48))
    );
    assert_eq!(
        HostMessage::parse(&[28, 2, 1, 0, 0]),
        Err(MessageError::UnsupportedCommand(28))
    );
}

#[test]
fn a_send_keeps_its_payload_and_offers_the_word_message_type() {
    let bytes = [2, 0, 3, 1, 2, 1, 1, 0, 0x68, 0x69];
    let Ok(ClientMessage::Send(SendMessage { to, from, payload })) = ClientMessage::parse(&bytes)
    else {
        panic!("a Send decodes");
    };
    assert_eq!((to, from), (Sid(0x0103), Sid(0x0102)));
    assert_eq!(payload, [1, 0, 0x68, 0x69]);
    let send = SendMessage { to, from, payload };
    assert_eq!(send.msg_type_word(), Some(1));
}

#[test]
fn a_lookup_names_the_land_type_only_in_its_dos_form() {
    let service = [0x29, 2, 0, 0, 2, 1, 0x8B, 0, 0];
    assert_eq!(
        ClientMessage::parse(&service),
        Ok(ClientMessage::ObjExists(ObjExists::Service {
            user: Sid(0x0102),
            land_type: LandType(0x8B)
        }))
    );
    assert_eq!(
        ClientMessage::parse(&[0x29, 9, 0, 0, 2, 1, 0, 0, 0]),
        Err(MessageError::UnsupportedSub {
            command: 41,
            sub: 9
        })
    );
}

#[test]
fn truncated_messages_are_errors() {
    for bytes in [
        &[2u8, 0, 3][..],
        &[28, 2, 1, 2, 0, 3, 1],
        &[13, 0, 1, 0, 2, 0, 5, 0, 7],
    ] {
        assert!(matches!(
            ClientMessage::parse(bytes),
            Err(MessageError::Truncated { .. })
        ));
    }
}

#[test]
fn the_captured_login_decodes_to_the_persona() {
    let Ok(ClientMessage::Login(login)) = ClientMessage::parse(&logon_rows()[0].bytes) else {
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
    let mut objects = ObjectStore::new();
    let mut player = PlayerSession::new();
    let mut expected: Vec<Vec<u8>> = Vec::new();
    let mut answered: Vec<Vec<u8>> = Vec::new();
    for row in logon_rows() {
        if row.from_client {
            let message = ClientMessage::parse(&row.bytes).unwrap();
            let replies = player.handle(&world, &mut objects, ConnectionId(1), &message);
            answered.extend(replies.iter().map(|reply| reply.message.encode()));
        } else {
            expected.push(row.bytes);
        }
    }
    assert_eq!(answered, expected);
}
