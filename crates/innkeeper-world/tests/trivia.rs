//! NTN Trivia: the waiting-room topics and one player's join through the handle.
//! Layouts are in the NTN survey; the bytes below are this host's replies.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use std::sync::Arc;

use common::Table;
use innkeeper_world::{
    AccountId, AccountRecord, BbsReply, BbsRequest, BoardRef, CheckedFailure, ClientMessage,
    Cookie, EncodedPassword, HostMessage, JoinNet, LandType, MemoryStore, MessagePosition,
    NtnMessage, ObjectKind, RawQuestion, ReadStep, Sid, Store, TextFraming, TriviaHeader,
    TriviaPools, TriviaReply, TriviaRequest, World, NTN_SERVER,
};

const LOGIN_PASSWORD: [u8; 10] = [0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00];
const TOPICS: &[u8] =
    b"\x01Choose a room\x02Adventure games\x02General trivia\x02Games and screens\x02";

fn spaced(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn client(message: ClientMessage) -> String {
    spaced(&message.encode())
}

fn host(message: HostMessage) -> String {
    spaced(&message.encode())
}

fn join_net(cookie: u16, kind: ObjectKind, land: u8, parameter: u16, size: u16) -> String {
    client(ClientMessage::JoinNet(JoinNet {
        cookie: Cookie(cookie),
        kind,
        land_type: LandType(land),
        parameter,
        size,
    }))
}

fn obj_id(cookie: u16, sid: u16) -> String {
    host(HostMessage::ObjId {
        cookie: Cookie(cookie),
        sid: Sid(sid),
    })
}

fn read_topics(section: u16) -> String {
    client(ClientMessage::Bbs(BbsRequest::Read {
        from: Sid(0x0102),
        board: BoardRef {
            location: 0,
            section,
        },
        step: ReadStep::This,
        message: 1,
    }))
}

fn topic_reply(section: u16, body: &[u8], number: u16, privileged: bool) -> String {
    host(HostMessage::Bbs(BbsReply::Message {
        to: Sid(0x0102),
        board: BoardRef {
            location: 0,
            section,
        },
        number,
        position: MessagePosition {
            first: true,
            last: true,
        },
        privileged,
        body: body.to_vec(),
    }))
}

fn routing(number: u16, sid: u16) -> TriviaHeader {
    TriviaHeader {
        server: NTN_SERVER,
        number,
        host: 7,
        remote: 0,
        sid,
    }
}

fn trivia_text(number: u16, sid: u16, text: &[u8]) -> String {
    client(ClientMessage::Trivia(TriviaRequest::Text {
        header: routing(number, sid),
        framing: TextFraming {
            zero: 0,
            three: 3,
            number,
            two: 2,
        },
        text: text.to_vec(),
    }))
}

fn game(number: u16, sid: u16, message: NtnMessage) -> String {
    host(HostMessage::Trivia(TriviaReply::Game {
        header: routing(number, sid),
        ignored: 0,
        pad: [0; 7],
        message,
    }))
}

fn joined(number: u16, sid: u16) -> String {
    host(HostMessage::Trivia(TriviaReply::Joined(routing(
        number, sid,
    ))))
}

fn logon_burst(number: u16, sid: u16) -> String {
    format!(
        "
        A < whichCmd 17 | {joined}
        A < type 0 | {version}
        A < type 1 | {logon}
        ",
        joined = joined(number, sid),
        version = game(number, sid, NtnMessage::Version),
        logon = game(number, sid, NtnMessage::Logon),
    )
}

/// A pool of one question in every room, so a JOIN is seated rather than told the show is down.
fn one_question_each() -> TriviaPools {
    let item = innkeeper_world::fit(&RawQuestion {
        question: "Who founded Sierra?".into(),
        correct: "Ken Williams".into(),
        incorrect: vec!["Al Lowe".into()],
        boolean: false,
    })
    .unwrap();
    TriviaPools {
        adventure: vec![item.clone()],
        general: vec![item.clone()],
        screens: vec![item],
        screens_ours: Vec::new(),
    }
}

#[test]
fn a_room_with_no_questions_answers_the_join_with_the_show_down_word() {
    let mut table = Table::new();
    let pms = 0x0301;
    let join = client(ClientMessage::Trivia(TriviaRequest::Join {
        header: routing(0, pms),
        extra: vec![0, 0],
    }));
    let down = host(HostMessage::Trivia(TriviaReply::Checked {
        which: CheckedFailure::Join,
        header: TriviaHeader {
            host: 7,
            ..routing(0, pms)
        },
        word: 42,
    }));
    let steps = format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > trivia JOIN, no questions loaded | {join}
        A < NTN Trivia has gone down | {down}
        "
    );
    table.run(&steps);
}

#[test]
fn one_player_joins_a_room_and_registers_a_handle() {
    let mut table = Table::new();
    table.world.install_trivia(one_question_each());
    let pms = 0x0103;
    let steps = format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > joinNet land 4 place 1 | 07 03 00 00 00 01 05 04 01 00 64 00
        A < land group SID | 08 00 00 01 00 00 00 01
        A > hub trivia topics 27/1 class 85 | {hub}
        A < three rooms | {hub_text}
        A > SL trivia topics 27/1 class 84 | {sl}
        A < the same three rooms | {sl_text}
        A > player object | {player}
        A < player SID | {player_sid}
        A > trivia group topic 1 | {group}
        A < group SID | {group_sid}
        A > GrpJoin the player | 0a 00 02 01 01 01
        A < echo, who is the player | 0a 00 02 01 01 01
        A > PMS object | {pms_join}
        A < PMS SID | {pms_sid}
        A > JOIN | {join}
        {burst}
        A > version line | {version}
        A > handle | {handle}
        A < type 39 | {registered}
        A < type 2 help 0 | {template}
        A > EXIT | {exit}
        A > JOIN again | {join}
        {burst2}
        ",
        hub = read_topics(85),
        hub_text = topic_reply(85, TOPICS, 1, false),
        sl = read_topics(84),
        sl_text = topic_reply(84, TOPICS, 1, false),
        player = join_net(1, ObjectKind::Object, 1, 0xFFFF, 1),
        player_sid = obj_id(1, 0x0101),
        group = join_net(2, ObjectKind::Group, 1, 16, 20),
        group_sid = obj_id(2, 0x0102),
        pms_join = join_net(3, ObjectKind::Object, 1, 0xFFFF, 13),
        pms_sid = obj_id(3, pms),
        join = client(ClientMessage::Trivia(TriviaRequest::Join {
            header: TriviaHeader {
                server: NTN_SERVER,
                number: 0,
                host: 0,
                remote: 0,
                sid: pms,
            },
            extra: vec![0, 0],
        })),
        burst = logon_burst(1, pms),
        version = trivia_text(1, pms, b"TRIVIA%V01.200\r\n"),
        handle = trivia_text(1, pms, b"Guy\r\n"),
        registered = game(
            1,
            pms,
            NtnMessage::HandleRegistered {
                handle: b"GUY".to_vec(),
            },
        ),
        template = game(
            1,
            pms,
            NtnMessage::ScoreTemplate {
                mode: 0,
                cnf: 0,
                help: 0,
                game_id: 0,
                scope: 0,
            },
        ),
        exit = client(ClientMessage::Trivia(TriviaRequest::Exit(routing(1, pms)))),
        burst2 = logon_burst(2, pms),
    );
    table.run(&steps);
}

#[test]
fn a_moderator_topic_replaces_the_default_and_conference_stays_empty() {
    let store = Arc::new(MemoryStore::new());
    let mut record = AccountRecord::new(EncodedPassword(LOGIN_PASSWORD), "guybrush");
    record.user_flags = 0x0204;
    store.create_account(AccountId(100_001), record).unwrap();
    let mut table = Table::new();
    table.world = World::stored(store);
    let custom = b"\x01kept\x02Only this\x02";
    let steps = format!(
        "
        A > login | {{login}}
        A < login ack | 00 00 00 00 16 04 02 00 bf 00
        A > joinNet land 4 place 1 | 07 03 00 00 00 01 05 04 01 00 64 00
        A < land group SID | 08 00 00 01 00 00 00 01
        A > store SL topics | {post}
        A > read them | {read_sl}
        A < the stored text | {stored}
        A > read hub topics | {read_hub}
        A < the default, privileged layout | {default_text}
        A > read conference | {read_conf}
        A < still empty | {empty}
        ",
        post = client(ClientMessage::Bbs(BbsRequest::Post {
            from: Sid(0x0102),
            board: BoardRef {
                location: 0,
                section: 84,
            },
            body: custom.to_vec(),
        })),
        read_sl = read_topics(84),
        stored = topic_reply(84, custom, 1, true),
        read_hub = read_topics(85),
        default_text = topic_reply(85, TOPICS, 1, true),
        read_conf = read_topics(67),
        empty = topic_reply(67, b"", 0, true),
    );
    table.run(&steps);
}
