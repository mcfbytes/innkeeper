use std::fs::File;
use std::io::BufWriter;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use innkeeper_session::{Session, SessionConfig, SessionEvent, SessionOutput};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, info_span, warn, Instrument};
use tsn_link::LinkEvent;

use crate::capture::{Capture, Direction};

const READ_CHUNK: usize = 4096;

type FileCapture = Capture<BufWriter<File>>;

/// What every connection shares: the session settings and where captures go.
#[derive(Debug)]
pub(crate) struct ConnectionSettings {
    pub(crate) session: SessionConfig,
    pub(crate) capture_dir: Option<PathBuf>,
}

/// Runs one client connection to its end, logging instead of failing.
pub(crate) async fn serve_connection(
    stream: TcpStream,
    peer: SocketAddr,
    id: u64,
    settings: Arc<ConnectionSettings>,
) {
    let span = info_span!("session", id, %peer);
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
    id: u64,
    settings: &ConnectionSettings,
) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let started = Instant::now();
    let mut capture = open_capture(settings, id, peer, started);
    let result = pump(&mut stream, Session::new(settings.session), &mut capture).await;
    record(&mut capture, Capture::finish_run);
    result
}

async fn pump(
    stream: &mut TcpStream,
    mut session: Session,
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
        }
        deliver_outputs(&mut session, stream, capture).await?;
    }
}

async fn deliver_outputs(
    session: &mut Session,
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
            }
        }
    }
    Ok(())
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
        SessionEvent::Link(_) | SessionEvent::Line(_) => info!(%event),
    }
}

fn open_capture(
    settings: &ConnectionSettings,
    id: u64,
    peer: SocketAddr,
    started: Instant,
) -> Option<FileCapture> {
    let dir = settings.capture_dir.as_ref()?;
    match Capture::create(dir, id, peer, started) {
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
fn record(
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
