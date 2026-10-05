//! The INT 14h transport listener: TSNEXEC's exports as envelopes over TCP (int14h-transport.md).

mod executive;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use innkeeper_world::ConnectionId;
use int14h::{
    encode_envelope, Body, Envelope, EnvelopeParser, Int14hError, LineRate, PeerName, Welcome,
    PROTOCOL_VERSION,
};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, info_span, warn, Instrument};

use crate::capture::{Capture, Direction};
use crate::connection::{
    open_capture, record, report, ConnectionSettings, FileCapture, READ_CHUNK,
};
use crate::host::{Exchange, Host};
use executive::Executive;

const SERVER_NAME: &str = "innkeeperd";

/// Why a transport connection was closed; every one of them ends the call.
#[derive(Debug, Error)]
enum TransportError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("malformed envelope: {0}")]
    Malformed(#[from] Int14hError),
    #[error("client closed before HELLO")]
    NoHello,
    #[error("first envelope is not HELLO")]
    NotHello,
    #[error("client speaks transport version {0}")]
    Version(u8),
    #[error("envelope other than CALL after HELLO")]
    NotCall,
}

/// Runs one transport client to its end, logging instead of failing.
pub(crate) async fn serve_transport(
    stream: TcpStream,
    peer: SocketAddr,
    settings: Arc<ConnectionSettings>,
) {
    let id = settings.new_connection();
    let span = info_span!("transport", id = id.0, %peer);
    async move {
        info!("client connected");
        match drive(stream, peer, id, &settings).await {
            Ok(()) => info!("client closed the transport"),
            Err(error) => warn!(%error, "transport closed"),
        }
    }
    .instrument(span)
    .await;
}

async fn drive(
    stream: TcpStream,
    peer: SocketAddr,
    id: ConnectionId,
    settings: &ConnectionSettings,
) -> Result<(), TransportError> {
    stream.set_nodelay(true)?;
    let capture = open_capture(settings, id, peer, Instant::now());
    let mut wire = Wire {
        stream,
        parser: EnvelopeParser::new(),
        capture,
    };
    wire.greet().await?;
    let mut executive = Executive::new(Host::new(settings, id), line_rate(settings));
    let result = serve_calls(&mut wire, &mut executive).await;
    executive.hang_up();
    record(&mut wire.capture, Capture::finish_run);
    result
}

/// GetLineRate answers the rate the emulated modem announces to legacy clients.
fn line_rate(settings: &ConnectionSettings) -> LineRate {
    LineRate(u16::try_from(settings.session.hayes.connect_rate).unwrap_or(u16::MAX))
}

async fn serve_calls(wire: &mut Wire, executive: &mut Executive<'_>) -> Result<(), TransportError> {
    loop {
        tokio::select! {
            envelope = wire.next_envelope() => {
                let Some(Envelope { tag, body }) = envelope? else {
                    return Ok(());
                };
                let Body::Call(call) = body else {
                    return Err(TransportError::NotCall);
                };
                debug!(?call, "call");
                let handled = executive.handle(call);
                wire.report(handled.exchange);
                debug!(reply = ?handled.reply, "reply");
                wire.send(&Envelope { tag, body: Body::Reply(handled.reply) }).await?;
            }
            Some(notice) = executive.next_notice() => wire.report(executive.pass_on(notice)),
        }
    }
}

/// One transport socket: envelopes in and out, every byte and step in the capture.
#[derive(Debug)]
struct Wire {
    stream: TcpStream,
    parser: EnvelopeParser,
    capture: Option<FileCapture>,
}

impl Wire {
    async fn greet(&mut self) -> Result<(), TransportError> {
        let Envelope { tag, body } = self.next_envelope().await?.ok_or(TransportError::NoHello)?;
        let Body::Hello(hello) = body else {
            return Err(TransportError::NotHello);
        };
        if hello.version != PROTOCOL_VERSION {
            return Err(TransportError::Version(hello.version));
        }
        info!(client = hello.client.as_str(), "transport opened");
        let welcome = Welcome {
            version: PROTOCOL_VERSION,
            server: PeerName::try_new(SERVER_NAME)?,
        };
        let body = Body::Welcome(welcome);
        self.send(&Envelope { tag, body }).await
    }

    /// The next whole envelope, or `None` once the client closes; cancelling loses no bytes.
    async fn next_envelope(&mut self) -> Result<Option<Envelope>, TransportError> {
        let mut chunk = Vec::with_capacity(READ_CHUNK);
        loop {
            if let Some(envelope) = self.parser.next_envelope() {
                return Ok(Some(envelope?));
            }
            chunk.clear();
            if self.stream.read_buf(&mut chunk).await? == 0 {
                return Ok(None);
            }
            let now = Instant::now();
            record(&mut self.capture, |c| {
                c.record_bytes(Direction::FromClient, now, &chunk)
            });
            self.parser.push(&chunk);
        }
    }

    async fn send(&mut self, envelope: &Envelope) -> Result<(), TransportError> {
        let bytes = encode_envelope(envelope);
        let now = Instant::now();
        record(&mut self.capture, |c| {
            c.record_bytes(Direction::ToClient, now, &bytes)
        });
        self.stream.write_all(&bytes).await?;
        Ok(())
    }

    fn report(&mut self, steps: Vec<Exchange>) {
        report(&mut self.capture, Instant::now(), steps);
    }
}

#[cfg(test)]
pub(crate) mod tests;
