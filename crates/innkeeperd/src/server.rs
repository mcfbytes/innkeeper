use std::future::{self, Future};
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use innkeeper_world::{StoreError, World};
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

use crate::config::Config;
use crate::connection::{serve_connection, ConnectionSettings};
use crate::int14h_listener::serve_transport;
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
    let world = open_world(config.data_dir.as_deref())?;
    let legacy = listen(config.bind).await?;
    let transport = match config.int14h_bind() {
        Some(bind) => Some(listen(bind).await?),
        None => None,
    };
    let settings = ConnectionSettings::new(config.session_config(), world, config.capture_dir());
    let settings = Arc::new(settings);
    info!(bind = %config.bind, captures = ?settings.capture_dir, "{STARTUP_LINE}");
    let transport = async {
        match transport {
            Some(listener) => {
                info!(bind = ?listener.local_addr().ok(), "serving the INT 14h transport");
                accept_forever(listener, Arc::clone(&settings), serve_transport).await;
            }
            None => future::pending().await,
        }
    };
    tokio::select! {
        () = accept_forever(legacy, Arc::clone(&settings), serve_connection) => {}
        () = transport => {}
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
    }
    Ok(())
}

async fn listen(bind: SocketAddr) -> Result<TcpListener, ServerError> {
    TcpListener::bind(bind)
        .await
        .map_err(|source| ServerError::Bind { bind, source })
}

/// Hands every accepted socket to `serve` on a task of its own.
pub(crate) async fn accept_forever<F, S>(
    listener: TcpListener,
    settings: Arc<ConnectionSettings>,
    serve: S,
) where
    S: Fn(TcpStream, SocketAddr, Arc<ConnectionSettings>) -> F,
    F: Future<Output = ()> + Send + 'static,
{
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tokio::spawn(serve(stream, peer, Arc::clone(&settings)));
            }
            Err(error) => {
                warn!(%error, "accept failed");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Instant;

    use innkeeper_session::SessionConfig;
    use innkeeper_world::{AccountId, AccountRecord, EncodedPassword};
    use int14h::{Call, ConnectResult, DialString, Reply, SwitchAddress, SwitchHostResult};
    use pad_thai::{HayesConfig, LineKind};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tsn_link::{Link, LinkConfig, LinkOutput, Message};

    use crate::int14h_listener::tests::TransportClient;

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
        tokio::spawn(accept_forever(
            listener,
            Arc::new(settings),
            serve_connection,
        ));

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
            LinkedClient {
                stream,
                link: client_link(),
                received: VecDeque::new(),
            }
        }

        /// MODEM.DRV's reconnect: escape to the PAD, clear the call to `old`, call `new`.
        async fn switch_host(&mut self, old: &str, new: &str, answers: bool) {
            self.stream.write_all(b"\r").await.unwrap();
            expect_reply(&mut self.stream, b"\r\n@").await;
            self.stream.write_all(b"SET? 0:0,32:0\r").await.unwrap();
            expect_reply(&mut self.stream, b"\r\n@").await;
            self.stream.write_all(b"D\r").await.unwrap();
            let cleared = format!("\r\n{old} DISCONNECTED\r\n@");
            expect_reply(&mut self.stream, cleared.as_bytes()).await;
            self.call_host(new, answers).await;
        }

        /// A PAD call: a host that answers starts a new link, any other is cleared at once.
        async fn call_host(&mut self, host: &str, answers: bool) {
            let command = format!("c {host}\r");
            self.stream.write_all(command.as_bytes()).await.unwrap();
            let reply = match answers {
                true => format!("\r\n{host} CONNECTED\r\n"),
                false => format!("\r\n{host} DISCONNECTED\r\n@"),
            };
            expect_reply(&mut self.stream, reply.as_bytes()).await;
            self.link = client_link();
            self.received.clear();
        }

        /// Only link control arrives for a while: no message reaches the client.
        async fn expect_silence(&mut self) {
            let deadline = tokio::time::Instant::now() + SILENCE;
            let mut chunk = [0; 256];
            loop {
                let read = tokio::time::timeout_at(deadline, self.stream.read(&mut chunk)).await;
                let Ok(read) = read else {
                    break;
                };
                let read = read.unwrap();
                assert!(read > 0, "server hung up");
                let escaped = self.link.handle_input(&chunk[..read], Instant::now());
                assert_eq!(escaped, None);
                self.write_outputs().await;
            }
            assert!(self.received.is_empty(), "arrived: {:02x?}", self.received);
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

    /// The client's end of a fresh link; the host's quiet start is the server's concern.
    fn client_link() -> Link {
        let config = LinkConfig {
            quiet_after_connect: Duration::ZERO,
            ..LinkConfig::default()
        };
        Link::new(config, Instant::now())
    }

    pub(crate) fn hex(text: &str) -> Vec<u8> {
        let pairs = text.split_whitespace();
        pairs
            .map(|pair| u8::from_str_radix(pair, 16).unwrap())
            .collect()
    }

    pub(crate) const LOGIN: &str =
        "35 00 00 00 01 02 03 12 a1 86 01 00 01 1d 7a 01 66 16 66 18 73 03 00 00 \
                         67 75 79 62 72 75 73 68 00";
    pub(crate) const LOGIN_ACK: &str = "00 00 00 00 16 00 00 00 00 00";
    pub(crate) const JOIN_PLAYER: &str = "07 00 00 00 8c 09 01 01 ff ff 1e 00";
    pub(crate) const JOIN_WAITING_ROOM: &str = "07 00 00 00 c0 01 05 01 01 00 80 00";
    const GUARD_TIME: Duration = Duration::from_millis(50);
    const JOIN_GAME_OBJECT: &str = "07 00 00 00 60 01 81 01 00 00 0c 00";
    /// How long a test waits to be sure nothing arrives.
    const SILENCE: Duration = Duration::from_millis(300);
    /// This host's X.25 number without the DNIC, and the number of a host nobody runs.
    const THIS_HOST: &str = "83420207";
    const OTHER_HOST: &str = "83420208";

    async fn serve_on_loopback(session: SessionConfig) -> SocketAddr {
        let settings = ConnectionSettings::new(session, World::stock(), None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(
            listener,
            Arc::new(settings),
            serve_connection,
        ));
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

    /// Both listeners on loopback ports, sharing one world and one object store.
    struct Listeners {
        legacy: SocketAddr,
        transport: SocketAddr,
    }

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        Legacy,
        Transport,
    }

    /// Either kind of client, driven by the same script.
    enum Client {
        Legacy(Box<LinkedClient>),
        Transport(TransportClient),
    }

    impl Listeners {
        async fn start() -> Self {
            let session = SessionConfig {
                line: LineKind::Pad,
                ..SessionConfig::default()
            };
            let settings = Arc::new(ConnectionSettings::new(session, World::stock(), None));
            let legacy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let transport = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let listeners = Listeners {
                legacy: legacy.local_addr().unwrap(),
                transport: transport.local_addr().unwrap(),
            };
            let shared = Arc::clone(&settings);
            tokio::spawn(accept_forever(legacy, shared, serve_connection));
            tokio::spawn(accept_forever(transport, settings, serve_transport));
            listeners
        }

        async fn client(&self, kind: Kind) -> Client {
            match kind {
                Kind::Legacy => Client::Legacy(Box::new(LinkedClient::call(self.legacy).await)),
                Kind::Transport => Client::Transport(TransportClient::dial(self.transport).await),
            }
        }
    }

    impl Client {
        async fn send(&mut self, body: &str) {
            match self {
                Client::Legacy(client) => client.send(body).await,
                Client::Transport(client) => client.send(body).await,
            }
        }

        async fn expect(&mut self, body: &str) {
            match self {
                Client::Legacy(client) => client.expect(body).await,
                Client::Transport(client) => client.expect(body).await,
            }
        }

        /// Switches from the call to `old` to the host at `new`; `answers` says whether it is this one.
        async fn switch_host(&mut self, old: &str, new: &str, answers: bool) {
            match self {
                Client::Legacy(client) => client.switch_host(old, new, answers).await,
                Client::Transport(client) => {
                    let argument = SwitchAddress::try_new(format!("3110{new}")).unwrap();
                    let expected = match answers {
                        true => SwitchHostResult::Switched,
                        false => SwitchHostResult::HostUnreachable,
                    };
                    let switched = client.call(Call::SwitchHost(argument)).await;
                    assert_eq!(switched, Reply::SwitchHost(expected));
                }
            }
        }

        /// After an unreachable host: MODEM.DRV calls its default host, a transport client dials.
        async fn call_again(&mut self) {
            match self {
                Client::Legacy(client) => client.call_host("SIERRA", true).await,
                Client::Transport(client) => {
                    let dial = DialString::try_new("ATDT0").unwrap();
                    let connected = client.call(Call::Connect(dial)).await;
                    assert_eq!(connected, Reply::Connect(ConnectResult::Connected));
                }
            }
        }

        /// Logs on and puts a player object with the given SID low byte into the waiting room.
        async fn enter_waiting_room(&mut self, player: &str) {
            self.send(LOGIN).await;
            self.expect(LOGIN_ACK).await;
            self.send(JOIN_PLAYER).await;
            self.expect(&format!("08 00 8c 09 00 00 {player} 01")).await;
            self.send(JOIN_WAITING_ROOM).await;
            self.expect("08 00 c0 01 00 00 01 01").await;
            self.send(&format!("0a 00 01 01 {player} 01 02 03 12"))
                .await;
            self.expect(&format!("0a 00 01 01 {player} 01")).await;
        }
    }

    #[tokio::test]
    async fn a_transport_client_reads_the_replies_a_legacy_client_reads() {
        for kind in [Kind::Legacy, Kind::Transport] {
            let listeners = Listeners::start().await;
            let mut client = listeners.client(kind).await;
            client.enter_waiting_room("00").await;
        }
    }

    async fn meet(first: Kind, second: Kind) {
        let listeners = Listeners::start().await;
        let mut first = listeners.client(first).await;
        first.enter_waiting_room("00").await;
        let mut second = listeners.client(second).await;
        second.enter_waiting_room("02").await;
        first.expect("0a 00 01 01 02 01").await;
        drop(second);
        first.expect("0b 00 01 01 02 01").await;
        first.expect("09 00 02 01").await;
    }

    #[tokio::test]
    async fn a_legacy_and_a_transport_client_meet_in_one_waiting_room() {
        meet(Kind::Legacy, Kind::Transport).await;
        meet(Kind::Transport, Kind::Legacy).await;
    }

    /// `kind` switches hosts twice while a client of the `peer` kind watches from the waiting room.
    async fn switch_hosts(kind: Kind, peer: Kind) {
        let listeners = Listeners::start().await;
        let mut client = listeners.client(kind).await;
        client.enter_waiting_room("00").await;
        let mut watcher = listeners.client(peer).await;
        watcher.enter_waiting_room("02").await;
        client.expect("0a 00 01 01 02 01").await;

        client.switch_host("SIERRA", THIS_HOST, true).await;
        watcher.expect("0b 00 01 01 00 01").await;
        watcher.expect("09 00 00 01").await;
        client.enter_waiting_room("03").await;
        watcher.expect("0a 00 01 01 03 01").await;

        client.switch_host(THIS_HOST, OTHER_HOST, false).await;
        watcher.expect("0b 00 01 01 03 01").await;
        watcher.expect("09 00 03 01").await;
        client.call_again().await;
        client.enter_waiting_room("04").await;
        watcher.expect("0a 00 01 01 04 01").await;
    }

    #[tokio::test]
    async fn a_host_switch_releases_the_call_alike_on_both_transports() {
        switch_hosts(Kind::Legacy, Kind::Transport).await;
        switch_hosts(Kind::Transport, Kind::Legacy).await;
    }

    #[tokio::test]
    async fn notices_wait_while_the_client_chains_programs() {
        let listeners = Listeners::start().await;
        let Client::Legacy(mut hub) = listeners.client(Kind::Legacy).await else {
            panic!("asked for a legacy client");
        };
        hub.send(LOGIN).await;
        hub.expect(LOGIN_ACK).await;
        hub.send(JOIN_GAME_OBJECT).await;
        hub.expect("08 00 60 01 00 00 00 01").await;
        hub.send(JOIN_PLAYER).await;
        hub.expect("08 00 8c 09 00 00 01 01").await;
        hub.send(JOIN_WAITING_ROOM).await;
        hub.expect("08 00 c0 01 00 00 02 01").await;
        hub.send("0a 00 02 01 01 01 02 03 12").await;
        hub.expect("0a 00 02 01 01 01").await;
        // The land frees its game object and exits; the next program has not spoken yet.
        hub.send("09 00 00 01 00 00").await;
        hub.expect_silence().await;

        let mut peer = listeners.client(Kind::Transport).await;
        peer.send(LOGIN).await;
        peer.expect(LOGIN_ACK).await;
        peer.send(JOIN_PLAYER).await;
        peer.expect("08 00 8c 09 00 00 03 01").await;
        peer.send(JOIN_WAITING_ROOM).await;
        peer.expect("08 00 c0 01 00 00 02 01").await;
        peer.send("0a 00 02 01 03 01 02 03 12").await;
        peer.expect("0a 00 02 01 03 01").await;
        hub.expect_silence().await;

        hub.send("2f 01 00 00 00 00").await;
        hub.expect("0a 00 02 01 03 01").await;
        let occupancy = "2f 01 00 00 00 00 00 00 00 00 03 00 \
                         07 01 01 40 02 07 02 01 40 00 07 03 01 40 00";
        hub.expect(occupancy).await;
    }
}
