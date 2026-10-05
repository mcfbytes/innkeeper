use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use innkeeper_session::SessionConfig;
use innkeeper_world::World;
use int14h::{
    encode_envelope, Body, Call, ConnectResult, DialString, Envelope, EnvelopeParser, Hello,
    MessageBody, PeerName, Reply, Tag, PROTOCOL_VERSION,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::serve_transport;
use crate::connection::ConnectionSettings;
use crate::server::accept_forever;
use crate::server::tests::hex;

const GOLDEN: &str = include_str!("../../../int14h/tests/golden/envelopes.txt");
const PATIENCE: Duration = Duration::from_secs(5);
const RECEIVE_RETRY: Duration = Duration::from_millis(10);

fn golden(name: &str) -> Vec<u8> {
    let rows = GOLDEN.lines().filter(|line| !line.starts_with('#'));
    let mut named = rows.filter_map(|line| line.split_once('|'));
    let (_, bytes) = named.find(|(row, _)| row.trim() == name).unwrap();
    hex(bytes)
}

/// A transport listener on a free loopback port with the stock world.
async fn listen() -> SocketAddr {
    let settings = ConnectionSettings::new(SessionConfig::default(), World::stock(), None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(accept_forever(
        listener,
        Arc::new(settings),
        serve_transport,
    ));
    address
}

/// A client that implements the exports by calling `innkeeperd`, as ScummVM's executive will.
#[derive(Debug)]
pub(crate) struct TransportClient {
    stream: TcpStream,
    parser: EnvelopeParser,
    next_tag: u16,
}

impl TransportClient {
    /// Says HELLO, reads WELCOME and dials.
    pub(crate) async fn dial(address: SocketAddr) -> Self {
        let stream = TcpStream::connect(address).await.unwrap();
        let mut client = TransportClient {
            stream,
            parser: EnvelopeParser::new(),
            next_tag: 1,
        };
        let hello = Hello {
            version: PROTOCOL_VERSION,
            client: PeerName::try_new("test").unwrap(),
        };
        client.write(Tag(0), Body::Hello(hello)).await;
        let welcome = client.read().await.expect("server sent WELCOME");
        assert!(matches!(welcome.body, Body::Welcome(_)), "{welcome:?}");
        let dial = DialString::try_new("ATDT0").unwrap();
        let connected = client.call(Call::Connect(dial)).await;
        assert_eq!(connected, Reply::Connect(ConnectResult::Connected));
        client
    }

    pub(crate) async fn call(&mut self, call: Call) -> Reply {
        let tag = Tag(self.next_tag);
        self.next_tag = self.next_tag.wrapping_add(1);
        self.write(tag, Body::Call(call)).await;
        let envelope = self.read().await.expect("server replied");
        assert_eq!(envelope.tag, tag);
        let Body::Reply(reply) = envelope.body else {
            panic!("not a REPLY: {envelope:?}");
        };
        reply
    }

    pub(crate) async fn send(&mut self, body: &str) {
        let body = MessageBody::try_new(hex(body)).unwrap();
        let queued = self.call(Call::Send(body)).await;
        assert_eq!(queued, Reply::Send { queued: true });
    }

    /// Calls Receive until a message is waiting and compares it with `body`.
    pub(crate) async fn expect(&mut self, body: &str) {
        let waited = tokio::time::timeout(PATIENCE, async {
            loop {
                match self.call(Call::Receive).await {
                    Reply::Receive(Some(message)) => return message,
                    Reply::Receive(None) => tokio::time::sleep(RECEIVE_RETRY).await,
                    other => panic!("not a Receive reply: {other:?}"),
                }
            }
        });
        let message = waited.await.expect("a message arrived in time");
        assert_eq!(message.as_bytes(), hex(body));
    }

    async fn write(&mut self, tag: Tag, body: Body) {
        let bytes = encode_envelope(&Envelope { tag, body });
        self.stream.write_all(&bytes).await.unwrap();
    }

    async fn read(&mut self) -> Option<Envelope> {
        loop {
            if let Some(envelope) = self.parser.next_envelope() {
                return Some(envelope.unwrap());
            }
            let mut chunk = [0; 256];
            let read = tokio::time::timeout(PATIENCE, self.stream.read(&mut chunk));
            let read = read.await.expect("server answered in time").unwrap();
            if read == 0 {
                return None;
            }
            self.parser.push(&chunk[..read]);
        }
    }
}

async fn exchange_raw(stream: &mut TcpStream, sent: &[u8], expected: &[u8]) {
    stream.write_all(sent).await.unwrap();
    let mut received = vec![0; expected.len()];
    let read = tokio::time::timeout(PATIENCE, stream.read_exact(&mut received));
    read.await.expect("server answered in time").unwrap();
    assert_eq!(received, expected);
}

async fn assert_closed(stream: &mut TcpStream) {
    let mut rest = Vec::new();
    let read = tokio::time::timeout(PATIENCE, stream.read_to_end(&mut rest));
    read.await.expect("server closed in time").unwrap();
    assert!(rest.is_empty(), "{rest:02x?}");
}

#[tokio::test]
async fn golden_envelopes_are_answered_byte_for_byte() {
    let address = listen().await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    exchange_raw(&mut stream, &golden("hello"), &golden("welcome")).await;
    exchange_raw(&mut stream, &golden("connect"), &golden("connected")).await;
    exchange_raw(&mut stream, &golden("send"), &golden("queued")).await;
    exchange_raw(&mut stream, &golden("poll"), &golden("poll-ok")).await;
    let receive = encode_envelope(&Envelope {
        tag: Tag(5),
        body: Body::Call(Call::Receive),
    });
    exchange_raw(&mut stream, &receive, &golden("receive-empty")).await;
    let status = encode_envelope(&Envelope {
        tag: Tag(7),
        body: Body::Call(Call::GetStatus),
    });
    exchange_raw(&mut stream, &status, &golden("status")).await;
}

#[tokio::test]
async fn a_first_envelope_other_than_hello_closes_the_connection() {
    let address = listen().await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(&golden("connect")).await.unwrap();
    assert_closed(&mut stream).await;
}

#[tokio::test]
async fn another_protocol_version_closes_the_connection() {
    let address = listen().await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    let hello = Hello {
        version: PROTOCOL_VERSION + 1,
        client: PeerName::try_new("future").unwrap(),
    };
    let bytes = encode_envelope(&Envelope {
        tag: Tag(0),
        body: Body::Hello(hello),
    });
    stream.write_all(&bytes).await.unwrap();
    assert_closed(&mut stream).await;
}

#[tokio::test]
async fn a_malformed_envelope_closes_the_connection() {
    let address = listen().await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    exchange_raw(&mut stream, &golden("hello"), &golden("welcome")).await;
    stream
        .write_all(&[0x04, 0, 0, 0, 0x10, 0, 0, 17])
        .await
        .unwrap();
    assert_closed(&mut stream).await;
}
