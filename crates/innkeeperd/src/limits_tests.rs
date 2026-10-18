use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use innkeeper_session::SessionConfig;
use innkeeper_world::World;
use pad_thai::LineKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tsn_link::{Link, LinkConfig, LinkOutput, Message};

use super::*;
use crate::connection::{serve_connection, ConnectionSettings};
use crate::server::accept_forever;
use crate::server::tests::{hex, JOIN_PLAYER, LOGIN};
use crate::switchboard::Switchboard;

/// A link-layer client that can stop reading and acknowledging whenever the test likes.
struct Caller {
    stream: TcpStream,
    link: Link,
    delivered: Vec<Vec<u8>>,
}

impl Caller {
    async fn call(address: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(b"@D\r\rc SIERRA\r").await.unwrap();
        let mut seen = Vec::new();
        while !seen.windows(9).any(|w| w == b"CONNECTED") {
            let mut chunk = [0; 64];
            let read = stream.read(&mut chunk).await.unwrap();
            seen.extend_from_slice(&chunk[..read]);
        }
        let config = LinkConfig {
            quiet_after_connect: Duration::ZERO,
            ..LinkConfig::default()
        };
        let link = Link::new(config, Instant::now());
        let delivered = Vec::new();
        Caller {
            stream,
            link,
            delivered,
        }
    }

    /// Sends one message and pumps until the host's first reply to it has arrived.
    async fn ask(&mut self, body: Vec<u8>) -> Vec<u8> {
        let message = Message::try_new(body).unwrap();
        self.link.send_message(&message, Instant::now());
        self.link.flush(Instant::now());
        loop {
            self.write_outputs().await;
            if !self.delivered.is_empty() {
                return self.delivered.remove(0);
            }
            self.read_once().await;
        }
    }

    /// Sends one message and waits only until the host has acknowledged it.
    async fn tell(&mut self, body: Vec<u8>) {
        let message = Message::try_new(body).unwrap();
        self.link.send_message(&message, Instant::now());
        self.link.flush(Instant::now());
        self.write_outputs().await;
        while !self.link.is_transmit_idle() {
            self.read_once().await;
            self.write_outputs().await;
        }
    }

    async fn read_once(&mut self) {
        let mut chunk = [0; 512];
        let read = tokio::time::timeout(Duration::from_secs(5), self.stream.read(&mut chunk));
        let read = read.await.expect("host answered in time").unwrap();
        assert!(read > 0, "host hung up");
        let _ = self.link.handle_input(&chunk[..read], Instant::now());
    }

    async fn write_outputs(&mut self) {
        while let Some(output) = self.link.poll_output() {
            match output {
                LinkOutput::ToClient(bytes) => self.stream.write_all(&bytes).await.unwrap(),
                LinkOutput::Delivered(message) => self.delivered.push(message.body().to_vec()),
                LinkOutput::Event(_) => {}
            }
        }
    }

    /// Reads without acknowledging until the host closes the socket.
    async fn expect_hang_up(mut self, within: Duration) {
        let mut chunk = [0; 4096];
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let read = tokio::time::timeout_at(deadline, self.stream.read(&mut chunk));
            match read.await.expect("the host ended the line in time") {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    }
}

async fn serve(session: SessionConfig, limits: Limits) -> SocketAddr {
    let mut settings = ConnectionSettings::new(session, World::stock(), None);
    settings.switchboard = Switchboard::with_queue_limit(limits.line_queue_bytes);
    settings.limits = limits;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(accept_forever(
        listener,
        Arc::new(settings),
        serve_connection,
    ));
    address
}

fn pad_line() -> SessionConfig {
    SessionConfig {
        line: LineKind::Pad,
        ..SessionConfig::default()
    }
}

/// The victim logs on and takes the first SID, 0x0100; the peer logs on beside it.
async fn victim_and_peer(address: SocketAddr) -> (Caller, Caller) {
    let mut victim = Caller::call(address).await;
    victim.ask(hex(LOGIN)).await;
    let granted = victim.ask(hex(JOIN_PLAYER)).await;
    assert_eq!(granted[6..8], [0x00, 0x01]);
    let mut peer = Caller::call(address).await;
    peer.ask(hex(LOGIN)).await;
    (victim, peer)
}

fn send_to_victim(len: usize) -> Vec<u8> {
    let mut send = hex("02 00 00 01 01 01");
    send.resize(send.len() + len, 0x41);
    send
}

#[tokio::test]
async fn a_client_that_stops_acknowledging_is_dropped_at_its_byte_share() {
    let limits = Limits {
        line_queue_bytes: 32 * 1024,
        ..Limits::default()
    };
    let (victim, mut peer) = victim_and_peer(serve(pad_line(), limits).await).await;
    for _ in 0..3 {
        peer.tell(send_to_victim(16 * 1024)).await;
    }
    victim.expect_hang_up(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn a_frame_the_client_never_acknowledges_ends_the_line() {
    let session = SessionConfig {
        link: LinkConfig {
            resend_after: Duration::from_millis(20),
            max_resends: 2,
            ..LinkConfig::default()
        },
        ..pad_line()
    };
    let (victim, mut peer) = victim_and_peer(serve(session, Limits::default()).await).await;
    peer.tell(send_to_victim(8)).await;
    victim.expect_hang_up(Duration::from_secs(5)).await;
}

#[test]
fn a_backlog_past_the_share_is_an_error() {
    let limits = Limits::default();
    assert!(limits.check_backlog(LINE_QUEUE_BYTES_DEFAULT).is_ok());
    let error = limits
        .check_backlog(LINE_QUEUE_BYTES_DEFAULT + 1)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
}

#[tokio::test]
async fn a_stalled_write_times_out() {
    let limits = Limits {
        write_timeout: Duration::from_millis(10),
        ..Limits::default()
    };
    let stalled = std::future::pending::<()>();
    let error = limits.write_within(stalled).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}
