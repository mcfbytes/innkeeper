use std::fs::File;
use std::io::BufWriter;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use innkeeper_session::{Session, SessionConfig, SessionEvent, SessionOutput};
use innkeeper_world::{ConnectionId, HostMessage, ObjectStore, World};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, info_span, warn, Instrument};
use tsn_link::{LinkEvent, Message};

use crate::capture::{Capture, Direction};
use crate::host::{Exchange, Host, ReplyError};
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
    next_connection: AtomicU64,
}

impl ConnectionSettings {
    pub(crate) fn new(session: SessionConfig, world: World, capture_dir: Option<PathBuf>) -> Self {
        ConnectionSettings {
            session,
            world,
            objects: Mutex::new(ObjectStore::new()),
            switchboard: Switchboard::default(),
            capture_dir,
            next_connection: AtomicU64::new(1),
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
    peer: SocketAddr,
    settings: Arc<ConnectionSettings>,
) {
    let id = settings.new_connection();
    let span = info_span!("session", id = id.0, %peer);
    async move {
        info!("client connected");
        match drive(stream, peer, id, &settings).await {
            Ok(()) => info!("client hung up"),
            Err(error) => warn!(%error, "connection failed"),
        }
    }
    .instrument(span)
    .await;
}

async fn drive(
    mut stream: TcpStream,
    peer: SocketAddr,
    id: ConnectionId,
    settings: &ConnectionSettings,
) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let started = Instant::now();
    let mut capture = open_capture(settings, id, peer, started);
    let mut host = Host::new(settings, id);
    let session = Session::new(settings.session);
    let result = pump(&mut stream, session, &mut host, &mut capture).await;
    host.hang_up();
    record(&mut capture, Capture::finish_run);
    result
}

async fn pump(
    stream: &mut TcpStream,
    mut session: Session,
    host: &mut Host<'_>,
    capture: &mut Option<FileCapture>,
) -> std::io::Result<()> {
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
                report(capture, now, send_all(&mut session, vec![notice], now));
            }
        }
        deliver_outputs(&mut session, host, stream, capture).await?;
    }
}

async fn deliver_outputs(
    session: &mut Session,
    host: &mut Host<'_>,
    stream: &mut TcpStream,
    capture: &mut Option<FileCapture>,
) -> std::io::Result<()> {
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
                log_event(&event);
                record(capture, |c| c.record_event(now, &event));
                match &event {
                    SessionEvent::Message(message) => {
                        let answer = host.answer(message.body());
                        report(capture, now, answer.exchange);
                        report(capture, now, send_all(session, answer.outgoing, now));
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

pub(crate) fn report(capture: &mut Option<FileCapture>, now: Instant, steps: Vec<Exchange>) {
    for step in steps {
        log_exchange(&step);
        record(capture, |c| c.record_event(now, &step));
    }
}

fn log_exchange(step: &Exchange) {
    match step {
        Exchange::Received(_) | Exchange::Replied(_) => info!(%step),
        Exchange::Undecodable(_) | Exchange::NotSent(_) => warn!(%step),
    }
}

fn log_event(event: &SessionEvent) {
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
