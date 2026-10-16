use std::fs::File;
use std::io::BufWriter;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use innkeeper_session::{Session, SessionConfig, SessionEvent, SessionOutput};
use innkeeper_world::{ConnectionId, HostMessage, LogPolicy, ObjectStore, World};
use pad_thai::{LineEvent, ModemEvent, PadEvent};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, info_span, warn, Instrument};
use tsn_link::{LinkEvent, Message};

use crate::capture::{Capture, Direction};
use crate::host::{Exchange, Host, LoggedExchange, LogonSignal, ReplyError};
use crate::peer::{PeerLog, PeerTokens};
use crate::switchboard::Switchboard;

pub(crate) const READ_CHUNK: usize = 4096;

pub(crate) type FileCapture = Capture<BufWriter<File>>;

/// What every connection shares: the session settings, the world, the host's objects, the way to
/// reach other connections and where captures go.
#[derive(Debug)]
pub(crate) struct ConnectionSettings {
    pub(crate) session: SessionConfig,
    pub(crate) world: World,
    pub(crate) objects: Mutex<ObjectStore>,
    pub(crate) switchboard: Switchboard,
    pub(crate) capture_dir: Option<PathBuf>,
    pub(crate) peers: PeerTokens,
    pub(crate) audit: Option<Arc<crate::audit::AuditSeal>>,
    next_connection: AtomicU64,
}

impl ConnectionSettings {
    pub(crate) fn new(session: SessionConfig, world: World, capture_dir: Option<PathBuf>) -> Self {
        let objects = ObjectStore::with_log(world.log);
        ConnectionSettings {
            session,
            objects: Mutex::new(objects),
            switchboard: Switchboard::default(),
            capture_dir,
            peers: PeerTokens::new(),
            audit: None,
            next_connection: AtomicU64::new(1),
            world,
        }
    }

    pub(crate) fn peer_log(&self, addr: SocketAddr) -> PeerLog {
        match self.world.log {
            LogPolicy::Verbose => PeerLog::Addr(addr),
            LogPolicy::Public => PeerLog::Token(self.peers.token(addr.ip())),
        }
    }

    /// A connection id no other connection of either listener has had.
    pub(crate) fn new_connection(&self) -> ConnectionId {
        ConnectionId(self.next_connection.fetch_add(1, Ordering::Relaxed))
    }
}

/// Runs one client connection to its end, logging instead of failing.
pub(crate) async fn serve_connection(
    stream: TcpStream,
    addr: SocketAddr,
    settings: Arc<ConnectionSettings>,
) {
    let peer = settings.peer_log(addr);
    if let Err(error) = stream.set_nodelay(true) {
        warn!(%error, %peer, "cannot disable Nagle; replies may lag");
    }
    serve_line(stream, addr, settings, "tcp", None).await;
}

/// Runs a legacy line session over any byte stream: a TCP socket or a WebSocket's bytes.
/// With a `logon` signal, it reports the account the Login opened, and `None` after the hang-up.
pub(crate) async fn serve_line<S>(
    stream: S,
    addr: SocketAddr,
    settings: Arc<ConnectionSettings>,
    transport: &'static str,
    logon: Option<LogonSignal>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let id = settings.new_connection();
    let peer = settings.peer_log(addr);
    let span = info_span!("session", id = id.0, %peer);
    async move {
        info!("client connected");
        match drive(stream, addr, id, &settings, transport, logon).await {
            Ok(()) => info!("client hung up"),
            Err(error) => warn!(%error, "connection failed"),
        }
    }
    .instrument(span)
    .await;
}

async fn drive<S>(
    mut stream: S,
    peer: SocketAddr,
    id: ConnectionId,
    settings: &ConnectionSettings,
    transport: &'static str,
    logon: Option<LogonSignal>,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let started = Instant::now();
    let mut capture = open_capture(settings, id, peer, started);
    let mut host = Host::new(settings, id);
    if let Some(signal) = logon {
        host.announce_logon(signal);
    }
    host.watch_audit(crate::audit::begin(
        settings.audit.as_ref(),
        peer,
        transport,
    ));
    let session = Session::new(settings.session);
    let result = pump(&mut stream, session, &mut host, &mut capture).await;
    host.hang_up();
    record(&mut capture, Capture::finish_run);
    result
}

async fn pump<S>(
    stream: &mut S,
    mut session: Session,
    host: &mut Host<'_>,
    capture: &mut Option<FileCapture>,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut chunk = Vec::with_capacity(READ_CHUNK);
    loop {
        chunk.clear();
        let run_deadline = capture.as_ref().and_then(Capture::run_deadline);
        tokio::select! {
            read = stream.read_buf(&mut chunk) => {
                if read? == 0 {
                    return Ok(());
                }
                let now = Instant::now();
                record(capture, |c| c.record_bytes(Direction::FromClient, now, &chunk));
                session.handle_input(&chunk, now);
            }
            () = sleep_until(session.next_deadline()) => session.handle_timeout(Instant::now()),
            () = sleep_until(run_deadline) => record(capture, Capture::finish_run),
            Some(notice) = host.next_notice() => {
                let now = Instant::now();
                report(
                    capture,
                    now,
                    send_all(&mut session, vec![notice], now),
                    host.log_policy(),
                );
            }
        }
        deliver_outputs(&mut session, host, stream, capture).await?;
    }
}

async fn deliver_outputs<S>(
    session: &mut Session,
    host: &mut Host<'_>,
    stream: &mut S,
    capture: &mut Option<FileCapture>,
) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    while let Some(output) = session.poll_output() {
        let now = Instant::now();
        match output {
            SessionOutput::ToClient(bytes) => {
                record(capture, |c| {
                    c.record_bytes(Direction::ToClient, now, &bytes)
                });
                stream.write_all(&bytes).await?;
            }
            SessionOutput::Event(event) => {
                log_event(&event, host.log_policy());
                record(capture, |c| c.record_event(now, &event));
                match &event {
                    SessionEvent::Message(message) => {
                        let answer = host.answer(message.body());
                        report(capture, now, answer.exchange, host.log_policy());
                        report(
                            capture,
                            now,
                            send_all(session, answer.outgoing, now),
                            host.log_policy(),
                        );
                    }
                    SessionEvent::LinkLost(_) => host.hang_up(),
                    SessionEvent::Line(_) | SessionEvent::Link(_) => {}
                }
            }
        }
    }
    Ok(())
}

fn send_all(session: &mut Session, messages: Vec<HostMessage>, now: Instant) -> Vec<Exchange> {
    let mut exchange = Vec::new();
    for message in messages {
        exchange.push(match send(session, &message, now) {
            Ok(()) => Exchange::Replied(message),
            Err(error) => Exchange::NotSent(error),
        });
    }
    if let Err(error) = session.flush(now) {
        exchange.push(Exchange::NotSent(error.into()));
    }
    exchange
}

fn send(session: &mut Session, reply: &HostMessage, now: Instant) -> Result<(), ReplyError> {
    let message = Message::try_new(reply.encode())?;
    session.send_message(&message, now)?;
    Ok(())
}

pub(crate) fn report(
    capture: &mut Option<FileCapture>,
    now: Instant,
    steps: Vec<Exchange>,
    policy: LogPolicy,
) {
    for step in steps {
        log_exchange(&step, policy);
        record(capture, |c| c.record_event(now, &step));
    }
}

fn log_exchange(step: &Exchange, policy: LogPolicy) {
    let logged = LoggedExchange { step, policy };
    match step {
        Exchange::Received(_) | Exchange::Replied(_) => info!(step = %logged),
        Exchange::Undecodable(_) | Exchange::NotSent(_) => warn!(step = %logged),
    }
}

fn log_event(event: &SessionEvent, policy: LogPolicy) {
    if policy.is_public() {
        log_event_public(event);
        return;
    }
    match event {
        SessionEvent::Message(message) => info!(%message, "client message"),
        SessionEvent::Link(LinkEvent::FrameReceived { .. }) => debug!(%event),
        SessionEvent::Link(
            LinkEvent::BadCrc { .. }
            | LinkEvent::Aborted { .. }
            | LinkEvent::Oversized { .. }
            | LinkEvent::UnknownControl { .. }
            | LinkEvent::GaveUp { .. },
        ) => warn!(%event),
        SessionEvent::Link(_) | SessionEvent::Line(_) | SessionEvent::LinkLost(_) => info!(%event),
    }
}

fn log_event_public(event: &SessionEvent) {
    match event {
        SessionEvent::Message(message) => {
            info!(message = %format!("len={}", message.body().len()), "client message");
        }
        SessionEvent::Link(LinkEvent::FrameReceived { .. }) => debug!(event = %public_event(event)),
        SessionEvent::Link(
            LinkEvent::BadCrc { .. }
            | LinkEvent::Aborted { .. }
            | LinkEvent::Oversized { .. }
            | LinkEvent::UnknownControl { .. }
            | LinkEvent::GaveUp { .. },
        ) => warn!(event = %public_event(event)),
        SessionEvent::Link(_) | SessionEvent::Line(_) | SessionEvent::LinkLost(_) => {
            info!(event = %public_event(event));
        }
    }
}

fn public_event(event: &SessionEvent) -> String {
    match event {
        SessionEvent::Message(message) => format!("message len={}", message.body().len()),
        SessionEvent::Link(link) => format!("link: {link}"),
        SessionEvent::LinkLost(loss) => format!("link lost: {loss}"),
        SessionEvent::Line(LineEvent::Detected(kind)) => format!("detected {kind:?}"),
        SessionEvent::Line(LineEvent::Modem(ModemEvent::Command(_))) => "AT command".to_owned(),
        SessionEvent::Line(LineEvent::Modem(ModemEvent::Dialled { .. })) => "dialled".to_owned(),
        SessionEvent::Line(LineEvent::Modem(event)) => event.to_string(),
        SessionEvent::Line(LineEvent::Pad(PadEvent::Command(_))) => "PAD command".to_owned(),
        SessionEvent::Line(LineEvent::Pad(PadEvent::UnknownCommand(_))) => {
            "unknown PAD command".to_owned()
        }
        SessionEvent::Line(LineEvent::Pad(PadEvent::InvalidAddress(_))) => {
            "invalid call address".to_owned()
        }
        SessionEvent::Line(LineEvent::Pad(PadEvent::HostConnected(_))) => {
            "call connected".to_owned()
        }
        SessionEvent::Line(LineEvent::Pad(PadEvent::HostDisconnected(_))) => {
            "call cleared".to_owned()
        }
        SessionEvent::Line(LineEvent::Pad(PadEvent::Escaped)) => {
            "escaped to the PAD prompt".to_owned()
        }
    }
}

pub(crate) fn open_capture(
    settings: &ConnectionSettings,
    id: ConnectionId,
    peer: SocketAddr,
    started: Instant,
) -> Option<FileCapture> {
    let dir = settings.capture_dir.as_ref()?;
    match Capture::create(dir, id.0, peer, started) {
        Ok((capture, path)) => {
            info!(path = %path.display(), "capturing session");
            Some(capture)
        }
        Err(error) => {
            warn!(%error, dir = %dir.display(), "running without a capture file");
            None
        }
    }
}

/// Writes to the capture; a failing capture is dropped so the session itself carries on.
pub(crate) fn record(
    capture: &mut Option<FileCapture>,
    write: impl FnOnce(&mut FileCapture) -> std::io::Result<()>,
) {
    if let Some(error) = capture.as_mut().and_then(|c| write(c).err()) {
        warn!(%error, "capture stopped");
        *capture = None;
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}
