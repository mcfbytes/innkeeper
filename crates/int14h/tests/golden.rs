//! Golden envelopes from docs/protocol/int14h-transport.md, compared byte for byte.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use int14h::{
    encode_envelope, Body, Call, ConnectResult, DialString, Envelope, EnvelopeParser, ExecStatus,
    Hello, LinkStatus, MessageBody, PeerName, Reply, Tag, Welcome, PROTOCOL_VERSION,
};

const ENVELOPES: &str = include_str!("golden/envelopes.txt");

fn expected(name: &str) -> Envelope {
    let call = |tag, call| Envelope {
        tag: Tag(tag),
        body: Body::Call(call),
    };
    let reply = |tag, reply| Envelope {
        tag: Tag(tag),
        body: Body::Reply(reply),
    };
    let peer = |name: &str| PeerName::try_new(name).unwrap();
    let message = |bytes: &[u8]| MessageBody::try_new(bytes.to_vec()).unwrap();
    match name {
        "hello" => Envelope {
            tag: Tag(0),
            body: Body::Hello(Hello {
                version: PROTOCOL_VERSION,
                client: peer("scummvm"),
            }),
        },
        "welcome" => Envelope {
            tag: Tag(0),
            body: Body::Welcome(Welcome {
                version: PROTOCOL_VERSION,
                server: peer("innkeeperd"),
            }),
        },
        "connect" => call(1, Call::Connect(DialString::try_new("ATDT0").unwrap())),
        "connected" => reply(1, Reply::Connect(ConnectResult::Connected)),
        "send" => call(2, Call::Send(message(&[0x22, 0x04, 0x10]))),
        "queued" => reply(2, Reply::Send { queued: true }),
        "poll" => call(3, Call::Poll),
        "poll-ok" => reply(3, Reply::Poll(LinkStatus::Ok)),
        "receive" => call(4, Call::Receive),
        "received" => reply(4, Reply::Receive(Some(message(&[0x07, 0x41])))),
        "receive-empty" => reply(5, Reply::Receive(None)),
        "cancel-next" => call(6, Call::SetNextProgram(None)),
        "status" => reply(
            7,
            Reply::GetStatus(ExecStatus {
                driver_version: 3,
                connected: true,
            }),
        ),
        other => panic!("no expectation for golden example {other}"),
    }
}

fn golden_rows() -> Vec<(String, Vec<u8>)> {
    let rows = ENVELOPES
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
    rows.map(|line| {
        let (name, hex) = line.split_once('|').unwrap();
        let bytes = hex
            .split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).unwrap());
        (name.trim().to_owned(), bytes.collect())
    })
    .collect()
}

#[test]
fn encoder_reproduces_every_golden_envelope() {
    for (name, bytes) in golden_rows() {
        assert_eq!(encode_envelope(&expected(&name)), bytes, "{name}");
    }
}

#[test]
fn parser_reads_the_golden_stream_back() {
    let rows = golden_rows();
    let mut parser = EnvelopeParser::new();
    rows.iter().for_each(|(_, bytes)| parser.push(bytes));
    for (name, _) in &rows {
        assert_eq!(parser.next_envelope(), Some(Ok(expected(name))), "{name}");
    }
    assert_eq!(parser.next_envelope(), None);
}
