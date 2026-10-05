use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use innkeeper_world::{StoreError, World};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::config::Config;
use crate::connection::{serve_connection, ConnectionSettings};
use crate::sqlite_store::SqliteStore;

const DATABASE_FILE: &str = "innkeeper.db";
const STARTUP_LINE: &str = "INT 14h hooked. Please wait while ImagiNation loads...";
/// Pause after a failed accept, so a full descriptor table does not spin the loop.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub(crate) enum ServerError {
    #[error("cannot listen on {bind}: {source}")]
    Bind { bind: SocketAddr, source: io::Error },
    #[error("cannot create the data directory {dir}: {source}")]
    DataDir { dir: String, source: io::Error },
    #[error("cannot open the account database: {0}")]
    Database(#[from] StoreError),
}

/// The stock world, with its accounts kept under `data_dir` when one is given.
fn open_world(data_dir: Option<&Path>) -> Result<World, ServerError> {
    let Some(dir) = data_dir else {
        return Ok(World::stock());
    };
    std::fs::create_dir_all(dir).map_err(|source| ServerError::DataDir {
        dir: dir.display().to_string(),
        source,
    })?;
    let store = SqliteStore::open(&dir.join(DATABASE_FILE))?;
    Ok(World::stored(Arc::new(store)))
}

pub(crate) async fn serve(config: Config) -> Result<(), ServerError> {
    let bind = config.bind;
    let world = open_world(config.data_dir.as_deref())?;
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|source| ServerError::Bind { bind, source })?;
    let settings = ConnectionSettings::new(config.session_config(), world, config.capture_dir());
    info!(%bind, captures = ?settings.capture_dir, "{STARTUP_LINE}");
    tokio::select! {
        () = accept_forever(listener, Arc::new(settings)) => {}
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
    }
    Ok(())
}

async fn accept_forever(listener: TcpListener, settings: Arc<ConnectionSettings>) {
    let mut next_id = 1u64;
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tokio::spawn(serve_connection(
                    stream,
                    peer,
                    next_id,
                    Arc::clone(&settings),
                ));
                next_id += 1;
            }
            Err(error) => {
                warn!(%error, "accept failed");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Instant;

    use innkeeper_session::SessionConfig;
    use innkeeper_world::{AccountId, AccountRecord, EncodedPassword};
    use pad_thai::{HayesConfig, LineKind};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tsn_link::{Link, LinkConfig, LinkOutput, Message};

    /// The stock client's first frame and the host's reply, from docs/protocol/captures.md.
    const CAPTURED_LOGIN_FRAME: [u8; 39] = [
        0x81, 0x58, 0xd8, 0x00, 0x21, 0x35, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x12, 0xa1, 0x86,
        0x01, 0x00, 0x01, 0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00, 0x00, 0x67,
        0x75, 0x79, 0x62, 0x72, 0x75, 0x73, 0x68, 0x00, 0x82,
    ];
    const LOGIN_ACK_FRAME: [u8; 16] = [
        0x81, 0xa1, 0x11, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x82,
    ];

    async fn expect_reply(client: &mut TcpStream, expected: &[u8]) {
        let mut received = vec![0; expected.len()];
        let read = tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut received));
        read.await.expect("server answered in time").unwrap();
        assert_eq!(
            String::from_utf8_lossy(&received),
            String::from_utf8_lossy(expected)
        );
    }

    #[test]
    fn a_data_dir_opens_a_database_that_enrols_and_remembers_accounts() {
        let dir = std::env::temp_dir().join(format!("innkeeperd-data-{}", std::process::id()));
        let id = AccountId(100_001);
        let world = open_world(Some(&dir)).unwrap();
        assert!(world.store.account(id).unwrap().is_none());
        world
            .store
            .create_account(id, AccountRecord::new(EncodedPassword([7; 10]), "kept"))
            .unwrap();
        drop(world);
        let reopened = open_world(Some(&dir)).unwrap();
        let kept = reopened.store.account(id).unwrap().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(kept.persona, "kept");
    }

    #[test]
    fn without_a_data_dir_the_book_stays_open() {
        assert!(open_world(None).is_ok());
    }

    #[tokio::test]
    async fn stock_client_logon_over_tcp_is_answered_and_captured() {
        let capture_dir =
            std::env::temp_dir().join(format!("innkeeperd-test-{}", std::process::id()));
        let session = SessionConfig {
            line: LineKind::Pad,
            ..SessionConfig::default()
        };
        let settings = ConnectionSettings::new(session, World::stock(), Some(capture_dir.clone()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(listener, Arc::new(settings)));

        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"@D\r\r").await.unwrap();
        expect_reply(&mut client, b"\r\nTERMINAL=\r\n@").await;
        client.write_all(b"c SIERRA\r").await.unwrap();
        expect_reply(&mut client, b"\r\nSIERRA CONNECTED\r\n").await;
        client.write_all(&CAPTURED_LOGIN_FRAME).await.unwrap();
        expect_reply(&mut client, &[0x81, 0x49, 0x62, 0x90, 0x82]).await;
        expect_reply(&mut client, &LOGIN_ACK_FRAME).await;
        drop(client);

        let mut captured = String::new();
        for _ in 0..50 {
            let files: Vec<_> = std::fs::read_dir(&capture_dir).unwrap().flatten().collect();
            captured = files
                .iter()
                .map(|f| std::fs::read_to_string(f.path()).unwrap())
                .collect();
            if captured.contains(" rx 81 a1 11 00 0a 00 ") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        std::fs::remove_dir_all(&capture_dir).unwrap();
        assert!(captured.contains("ev message len=33 35 00"), "{captured}");
        assert!(captured.contains("ev client Login("), "{captured}");
        assert!(captured.contains("ev host LoginAccepted"), "{captured}");
        assert!(captured.contains(" rx 81 a1 11 00 0a 00 "), "{captured}");
    }

    /// A client past the PAD call that speaks the link layer through a `Link` of its own.
    struct LinkedClient {
        stream: TcpStream,
        link: Link,
        received: VecDeque<Vec<u8>>,
    }

    impl LinkedClient {
        async fn call(address: SocketAddr) -> Self {
            Self::call_over(TcpStream::connect(address).await.unwrap()).await
        }

        /// Dials the emulated modem first, as MODEM.DRV does on a raw serial line.
        async fn dial(address: SocketAddr) -> Self {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(b"ATDT5551234\r").await.unwrap();
            expect_reply(&mut stream, b"ATDT5551234\r\r\nCONNECT 2400\r\n").await;
            Self::call_over(stream).await
        }

        async fn call_over(mut stream: TcpStream) -> Self {
            stream.write_all(b"@D\r\r").await.unwrap();
            expect_reply(&mut stream, b"\r\nTERMINAL=\r\n@").await;
            stream.write_all(b"c SIERRA\r").await.unwrap();
            expect_reply(&mut stream, b"\r\nSIERRA CONNECTED\r\n").await;
            let config = LinkConfig {
                quiet_after_connect: Duration::ZERO,
                ..LinkConfig::default()
            };
            let link = Link::new(config, Instant::now());
            let received = VecDeque::new();
            LinkedClient {
                stream,
                link,
                received,
            }
        }

        /// MODEM.DRV's hang-up: `+++` after a guard time, then `AT H0`; the socket stays open.
        async fn hang_up_modem(&mut self) {
            tokio::time::sleep(2 * GUARD_TIME).await;
            self.stream.write_all(b"+++").await.unwrap();
            expect_reply(&mut self.stream, b"\r\nOK\r\n").await;
            self.stream.write_all(b"AT H0\r").await.unwrap();
            expect_reply(&mut self.stream, b"AT H0\r\r\nOK\r\n").await;
        }

        async fn send(&mut self, body: &str) {
            let message = Message::try_new(hex(body)).unwrap();
            self.link.send_message(&message, Instant::now());
            self.link.flush(Instant::now());
            self.write_outputs().await;
        }

        async fn expect(&mut self, body: &str) {
            while self.received.is_empty() {
                let mut chunk = [0; 256];
                let read =
                    tokio::time::timeout(Duration::from_secs(5), self.stream.read(&mut chunk));
                let read = read.await.expect("server answered in time").unwrap();
                assert!(read > 0, "server hung up");
                let escaped = self.link.handle_input(&chunk[..read], Instant::now());
                assert_eq!(escaped, None);
                self.write_outputs().await;
            }
            assert_eq!(self.received.pop_front(), Some(hex(body)));
        }

        async fn write_outputs(&mut self) {
            while let Some(output) = self.link.poll_output() {
                match output {
                    LinkOutput::ToClient(bytes) => self.stream.write_all(&bytes).await.unwrap(),
                    LinkOutput::Delivered(message) => {
                        self.received.push_back(message.body().to_vec())
                    }
                    LinkOutput::Event(_) => {}
                }
            }
        }
    }

    fn hex(text: &str) -> Vec<u8> {
        let pairs = text.split_whitespace();
        pairs
            .map(|pair| u8::from_str_radix(pair, 16).unwrap())
            .collect()
    }

    const LOGIN: &str = "35 00 00 00 01 02 03 12 a1 86 01 00 01 1d 7a 01 66 16 66 18 73 03 00 00 \
                         67 75 79 62 72 75 73 68 00";
    const LOGIN_ACK: &str = "00 00 00 00 16 00 00 00 00 00";
    const JOIN_PLAYER: &str = "07 00 00 00 8c 09 01 01 ff ff 1e 00";
    const JOIN_WAITING_ROOM: &str = "07 00 00 00 c0 01 05 01 01 00 80 00";
    const GUARD_TIME: Duration = Duration::from_millis(50);

    async fn serve_on_loopback(session: SessionConfig) -> SocketAddr {
        let settings = ConnectionSettings::new(session, World::stock(), None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(listener, Arc::new(settings)));
        address
    }

    /// Logs on and adds the player object the host hands out to the Clubhouse waiting room.
    async fn enter_waiting_room(client: &mut LinkedClient, player: &str) {
        client.send(LOGIN).await;
        client.expect(LOGIN_ACK).await;
        client.send(JOIN_PLAYER).await;
        client.expect(&format!("08 00 8c 09 00 00 {player}")).await;
        client.send(JOIN_WAITING_ROOM).await;
        client.expect("08 00 c0 01 00 00 01 01").await;
        client.send(&format!("0a 00 01 01 {player} 02 03 12")).await;
        client.expect(&format!("0a 00 01 01 {player}")).await;
    }

    #[tokio::test]
    async fn two_clients_meet_in_one_waiting_room_group() {
        let address = serve_on_loopback(SessionConfig {
            line: LineKind::Pad,
            ..SessionConfig::default()
        })
        .await;
        let mut first = LinkedClient::call(address).await;
        enter_waiting_room(&mut first, "00 01").await;
        let mut second = LinkedClient::call(address).await;
        enter_waiting_room(&mut second, "02 01").await;
        first.expect("0a 00 01 01 02 01").await;

        drop(second);
        first.expect("0b 00 01 01 02 01").await;
        first.expect("09 00 02 01").await;
    }

    #[tokio::test]
    async fn a_modem_hang_up_releases_the_player_while_the_socket_stays_open() {
        let address = serve_on_loopback(SessionConfig {
            hayes: HayesConfig {
                guard_time: GUARD_TIME,
                ..HayesConfig::default()
            },
            ..SessionConfig::default()
        })
        .await;
        let mut first = LinkedClient::call(address).await;
        enter_waiting_room(&mut first, "00 01").await;
        let mut second = LinkedClient::dial(address).await;
        enter_waiting_room(&mut second, "02 01").await;
        first.expect("0a 00 01 01 02 01").await;

        second.hang_up_modem().await;
        first.expect("0b 00 01 01 02 01").await;
        first.expect("09 00 02 01").await;
        second.stream.write_all(b"AT\r").await.unwrap();
        expect_reply(&mut second.stream, b"AT\r\r\nOK\r\n").await;
    }
}
