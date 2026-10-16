use std::future::{self, Future};
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use innkeeper_world::{
    AccountBook, Enrolment, LogPolicy, RetentionPolicy, Store, StoreError, World,
};
use std::net::TcpListener as StdListener;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

use crate::admin::allowlist::Allowlist;
use crate::admin::{Admin, AdminSetup};
use crate::audit::AuditSeal;
use crate::config::Config;
use crate::connection::{serve_connection, ConnectionSettings};
use crate::dates;
use crate::int14h_listener::serve_transport;
use crate::retention;
use crate::seal::Recipient;
use crate::signin::EnrolmentMode;
use crate::sqlite_store::SqliteStore;
use crate::websocket::serve_websocket;

pub(crate) const DATABASE_FILE: &str = "innkeeper.db";
pub(crate) const LOCK_FILE: &str = "innkeeper.lock";
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
    #[error("innkeeperd is already running (pid {pid}); stop that process first")]
    AlreadyRunning { pid: u32 },
    #[error("cannot write the data-dir lock {path}: {source}")]
    Lock { path: String, source: io::Error },
    #[error("{0}")]
    SignIn(#[from] crate::signin::startup::StartupError),
    #[error("cannot use a listening socket: {0}")]
    Adopt(io::Error),
    #[error("{0}")]
    Admin(String),
    #[error("--azure-metrics: {0}")]
    Metrics(String),
    #[error("{0}")]
    AdminList(#[from] crate::admin::allowlist::AllowlistError),
}

/// Startup finished, sockets bound, sandbox not yet applied.
pub(crate) struct Prepared {
    settings: Arc<ConnectionSettings>,
    legacy: Option<StdListener>,
    transport: Option<StdListener>,
    websocket: Option<StdListener>,
    /// The operator portal's listener and state (`--admin-bind`).
    admin: Option<(StdListener, Arc<Admin>)>,
    lock: Option<crate::subject::DataDirLock>,
    store: Option<Arc<SqliteStore>>,
    policy: RetentionPolicy,
    data_dir: Option<PathBuf>,
    capture_dir: Option<PathBuf>,
    /// The Azure custom-metrics pusher's settings (`--azure-metrics`).
    metrics: Option<crate::metrics::insights::Settings>,
}

impl Prepared {
    pub(crate) fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    pub(crate) fn capture_dir(&self) -> Option<&Path> {
        self.capture_dir.as_deref()
    }

    /// Directories the sandbox keeps readable after startup: the portal's allowlist file lives in one.
    pub(crate) fn read_dirs(&self) -> Vec<PathBuf> {
        self.admin
            .as_ref()
            .and_then(|(_, admin)| admin.allowlist_dir())
            .into_iter()
            .collect()
    }

    /// Ports already bound. Port 0 is not a Landlock rule (it means the ephemeral range).
    pub(crate) fn bound_ports(&self) -> Vec<u16> {
        let listeners = [
            self.legacy.as_ref(),
            self.transport.as_ref(),
            self.websocket.as_ref(),
            self.admin.as_ref().map(|(listener, _)| listener),
        ];
        let mut ports = Vec::new();
        for listener in listeners.into_iter().flatten() {
            let Ok(addr) = listener.local_addr() else {
                continue;
            };
            let port = addr.port();
            if port != 0 && !ports.contains(&port) {
                ports.push(port);
            }
        }
        ports
    }
}

struct Opened {
    world: World,
    store: Option<Arc<SqliteStore>>,
}

/// The stock world, with its accounts kept under `data_dir` when one is given.
fn open_world(data_dir: Option<&Path>) -> Result<Opened, ServerError> {
    let Some(dir) = data_dir else {
        return Ok(Opened {
            world: World::stock(),
            store: None,
        });
    };
    std::fs::create_dir_all(dir).map_err(|source| ServerError::DataDir {
        dir: dir.display().to_string(),
        source,
    })?;
    let store = Arc::new(SqliteStore::open(&dir.join(DATABASE_FILE))?);
    let world = World::stored(Arc::clone(&store) as Arc<dyn Store>);
    Ok(Opened {
        world,
        store: Some(store),
    })
}

/// The stored book under `--enrolment`: only open enrolment creates an account at Login.
pub(crate) fn account_book(store: &Arc<SqliteStore>, enrolment: EnrolmentMode) -> AccountBook {
    let login = match enrolment.login_enrols() {
        true => Enrolment::Open,
        false => Enrolment::Closed,
    };
    AccountBook::stored(Arc::clone(store) as Arc<dyn Store>, login)
}

/// Open the database, take the data-dir lock, read the vault and bind listeners. The runtime
/// starts later.
pub(crate) fn prepare(config: Config) -> Result<Prepared, ServerError> {
    let mut opened = open_world(config.data_dir.as_deref())?;
    opened.world.log = if config.public {
        LogPolicy::Public
    } else {
        LogPolicy::Verbose
    };
    let enrolment = config.enrolment();
    if let Some(store) = &opened.store {
        opened.world.accounts = account_book(store, enrolment);
    }
    let lock = match config.data_dir.as_deref() {
        Some(dir) => Some(crate::subject::DataDirLock::acquire(dir)?),
        None => None,
    };
    let policy = config.retention();
    if let Some(store) = &opened.store {
        run_purge(store, policy);
    }
    let startup = crate::signin::startup::prepare(&config, opened.store.as_ref())?;
    let audit = load_audit(&config, startup.recipient, opened.store.as_ref());
    let has_providers = startup
        .sign_in
        .as_ref()
        .is_some_and(|sign_in| sign_in.has_providers());
    if config.public && !has_providers {
        warn!("{}", crate::connection_gate::NO_PROVIDERS);
    }
    let metrics =
        crate::metrics::insights::Settings::from_config(&config).map_err(ServerError::Metrics)?;
    let plan = config.listen_plan();
    let legacy = listen_std_if(plan.legacy)?;
    let transport = listen_std_if(plan.int14h)?;
    let websocket = listen_std_if(plan.websocket)?;
    let admin_listener = listen_std_if(config.admin_bind)?;
    let capture_dir = config.capture_dir();
    let mut settings =
        ConnectionSettings::new(config.session_config(), opened.world, capture_dir.clone());
    settings.audit = audit;
    settings.sign_in = startup.sign_in;
    settings.require_session = config.public;
    settings.signup_open = enrolment.login_enrols();
    let settings = Arc::new(settings);
    let admin = match (admin_listener, opened.store.as_ref()) {
        (Some(listener), Some(store)) => {
            let admin = build_admin(&config, &settings, store, startup.admin_vault, policy)?;
            Some((listener, admin))
        }
        _ => None,
    };
    Ok(Prepared {
        settings,
        legacy,
        transport,
        websocket,
        admin,
        lock,
        store: opened.store,
        policy,
        data_dir: config.data_dir,
        capture_dir,
        metrics,
    })
}

/// The portal's state and the open-door rule (docs/server/innkeeperd.md section 9): with no vault
/// list, no file and no staff rows it is open to any signed-in identity, which `--public` refuses.
fn build_admin(
    config: &Config,
    settings: &Arc<ConnectionSettings>,
    store: &Arc<SqliteStore>,
    vault: Option<std::collections::HashSet<crate::signin::keys::IdentityKey>>,
    retention: RetentionPolicy,
) -> Result<Arc<Admin>, ServerError> {
    let Some(sign_in) = settings.sign_in.clone() else {
        return Err(ServerError::Admin(
            "--admin-bind needs --auth-config: staff sign in with the same providers".into(),
        ));
    };
    let allowlist = Allowlist::new(vault, config.admin_allowlist.clone(), config.admin_dev_open)?;
    let open = allowlist
        .is_open(store)
        .map_err(|error| ServerError::Admin(error.to_string()))?;
    if open && config.public {
        return Err(ServerError::Admin(
            "--public refuses an open admin portal: give the admin-identities vault secret, an \
             --admin-allowlist file, or run `innkeeperd admin allow` first"
                .into(),
        ));
    }
    if open {
        warn!(
            "DEV: the admin portal is OPEN to any signed-in identity (no admin-identities secret, \
             no --admin-allowlist file, no staff rows). Every page shows a red banner."
        );
    }
    let origin = sign_in.rp().admin_origin().clone();
    Ok(Arc::new(Admin::new(AdminSetup {
        settings: Arc::clone(settings),
        sign_in,
        store: Arc::clone(store),
        allowlist,
        origin,
        retention,
    })))
}

pub(crate) async fn run(prepared: Prepared) -> Result<(), ServerError> {
    let Prepared {
        settings,
        legacy,
        transport,
        websocket,
        admin,
        lock,
        store,
        policy,
        data_dir: _,
        capture_dir: _,
        metrics,
    } = prepared;
    let _lock = lock;
    if let Some(push) = metrics {
        tokio::spawn(crate::metrics::insights::run(push, Arc::clone(&settings)));
    }
    if let Some(interval) = crate::notify::watchdog_interval() {
        tokio::spawn(crate::notify::watchdog(interval, store.clone()));
    }
    if let Some(store) = store.clone() {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                run_purge(&store, policy);
            }
        });
    }
    let legacy = adopt_if(legacy)?;
    let transport = adopt_if(transport)?;
    let websocket = adopt_if(websocket)?;
    let admin = match admin {
        Some((listener, admin)) => Some((adopt(listener)?, admin)),
        None => None,
    };
    match legacy
        .as_ref()
        .and_then(|listener| listener.local_addr().ok())
    {
        Some(bind) => info!(bind = %bind, captures = ?settings.capture_dir, "{STARTUP_LINE}"),
        None => info!(captures = ?settings.capture_dir, "{STARTUP_LINE}"),
    }
    let legacy = async {
        match legacy {
            Some(listener) => {
                accept_forever(listener, Arc::clone(&settings), serve_connection).await
            }
            None => future::pending().await,
        }
    };
    let transport = accept_if(
        transport,
        &settings,
        serve_transport,
        "the INT 14h transport",
    );
    let websocket = accept_if(websocket, &settings, serve_websocket, "WebSocket lines");
    let admin = async {
        match admin {
            Some((listener, admin)) => {
                info!(bind = ?listener.local_addr().ok(), "serving the operator portal under /admin/");
                accept_forever(listener, admin, crate::admin::serve).await;
            }
            None => future::pending().await,
        }
    };
    tokio::select! {
        () = admin => {}
        () = legacy => {}
        () = transport => {}
        () = websocket => {}
        _ = tokio::signal::ctrl_c() => {
            crate::notify::stopping();
            info!("shutting down");
        }
    }
    Ok(())
}

fn listen_std(bind: SocketAddr) -> Result<StdListener, ServerError> {
    StdListener::bind(bind).map_err(|source| ServerError::Bind { bind, source })
}

fn listen_std_if(bind: Option<SocketAddr>) -> Result<Option<StdListener>, ServerError> {
    bind.map(listen_std).transpose()
}

fn adopt(listener: StdListener) -> Result<TcpListener, ServerError> {
    listener.set_nonblocking(true).map_err(ServerError::Adopt)?;
    TcpListener::from_std(listener).map_err(ServerError::Adopt)
}

fn adopt_if(listener: Option<StdListener>) -> Result<Option<TcpListener>, ServerError> {
    listener.map(adopt).transpose()
}

fn run_purge(store: &SqliteStore, policy: RetentionPolicy) {
    match retention::purge_expired(store, policy, dates::now_ms()) {
        Ok(counts) => retention::log_counts(&counts),
        Err(error) => {
            crate::metrics::counters().store_error();
            warn!(%error, "retention purge failed");
        }
    }
}

fn load_audit(
    config: &Config,
    recipient: Option<Recipient>,
    store: Option<&Arc<SqliteStore>>,
) -> Option<Arc<AuditSeal>> {
    let Some(recipient) = recipient else {
        if config.public {
            warn!(
                "no sealed connection records are kept: --public was set without an audit public key"
            );
        }
        return None;
    };
    let Some(store) = store else {
        warn!("no sealed connection records are kept: the audit public key needs --data-dir");
        return None;
    };
    info!(kid = %recipient.kid, "sealed connection records are on");
    Some(Arc::new(AuditSeal::new(Arc::clone(store), recipient)))
}

/// Serves an optional listener; a listener that is switched off never finishes.
async fn accept_if<F, S>(
    listener: Option<TcpListener>,
    settings: &Arc<ConnectionSettings>,
    serve: S,
    what: &str,
) where
    S: Fn(TcpStream, SocketAddr, Arc<ConnectionSettings>) -> F,
    F: Future<Output = ()> + Send + 'static,
{
    match listener {
        Some(listener) => {
            info!(bind = ?listener.local_addr().ok(), "serving {what}");
            accept_forever(listener, Arc::clone(settings), serve).await;
        }
        None => future::pending().await,
    }
}

/// Hands every accepted socket to `serve` on a task of its own.
pub(crate) async fn accept_forever<T, F, S>(listener: TcpListener, settings: Arc<T>, serve: S)
where
    T: Send + Sync + 'static,
    S: Fn(TcpStream, SocketAddr, Arc<T>) -> F,
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
    use pad_thai::{HayesConfig, LineKind};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tsn_link::{Link, LinkConfig, LinkOutput, Message};

    use crate::int14h_listener::tests::TransportClient;

    /// The stock client's first frame, from docs/protocol/captures.md, and the host's reply, whose rating
    /// now grants the five stock lands and whose userFlags carry INN Stamps.
    pub(crate) const CAPTURED_LOGIN_FRAME: [u8; 39] = [
        0x81, 0x58, 0xd8, 0x00, 0x21, 0x35, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x12, 0xa1, 0x86,
        0x01, 0x00, 0x01, 0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00, 0x00, 0x67,
        0x75, 0x79, 0x62, 0x72, 0x75, 0x73, 0x68, 0x00, 0x82,
    ];
    pub(crate) const LOGIN_ACK_FRAME: [u8; 16] = [
        0x81, 0x6f, 0xf7, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x16, 0x00, 0x02, 0x00, 0x8f, 0x00,
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
        let world = open_world(Some(&dir)).unwrap().world;
        assert!(world.store.account(id).unwrap().is_none());
        world
            .store
            .create_account(id, AccountRecord::new(EncodedPassword([7; 10]), "kept"))
            .unwrap();
        drop(world);
        let reopened = open_world(Some(&dir)).unwrap().world;
        let kept = reopened.store.account(id).unwrap().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(kept.persona, "kept");
    }

    #[test]
    fn only_open_enrolment_creates_an_account_at_login() {
        use innkeeper_world::{ClientVersion, LandType, Login, PasswordSource};
        let login = Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(100_009),
            password_source: PasswordSource::StoredFile,
            password: EncodedPassword([4; 10]),
            name: "guybrush".into(),
            prodigy_id: None,
        };
        for (mode, enrols) in [
            (EnrolmentMode::Open, true),
            (EnrolmentMode::WebOnly, false),
            (EnrolmentMode::Closed, false),
        ] {
            let store = Arc::new(SqliteStore::open_in_memory().unwrap());
            let admitted = account_book(&store, mode).admit(&login).is_ok();
            assert_eq!(admitted, enrols, "{mode:?}");
        }
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
            if captured.contains(" rx 81 6f f7 00 0a 00 ") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        std::fs::remove_dir_all(&capture_dir).unwrap();
        assert!(captured.contains("ev message len=33 35 00"), "{captured}");
        assert!(captured.contains("ev client Login("), "{captured}");
        assert!(captured.contains("ev host LoginAccepted"), "{captured}");
        assert!(captured.contains(" rx 81 6f f7 00 0a 00 "), "{captured}");
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

    pub(crate) fn hex(text: &str) -> Vec<u8> {
        let pairs = text.split_whitespace();
        pairs
            .map(|pair| u8::from_str_radix(pair, 16).unwrap())
            .collect()
    }

    pub(crate) const LOGIN: &str =
        "35 00 00 00 01 02 03 12 a1 86 01 00 01 1d 7a 01 66 16 66 18 73 03 00 00 \
                         67 75 79 62 72 75 73 68 00";
    pub(crate) const LOGIN_ACK: &str = "00 00 00 00 16 00 02 00 8f 00";
    pub(crate) const JOIN_PLAYER: &str = "07 00 00 00 8c 09 01 01 ff ff 1e 00";
    pub(crate) const JOIN_WAITING_ROOM: &str = "07 00 00 00 c0 01 05 01 01 00 80 00";
    const GUARD_TIME: Duration = Duration::from_millis(50);

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
}
