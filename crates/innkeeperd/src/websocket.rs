//! WebSocket ingress for the browser client: binary messages carry the raw serial bytes of one
//! legacy line session (docs/server/innkeeperd.md section 2).
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info, warn};

use crate::connection::{serve_line, ConnectionSettings, READ_CHUNK};

/// A browser that opens the socket but never finishes the upgrade is dropped after this.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Serial bytes arrive a few at a time; anything larger is not a modem line.
const MAX_MESSAGE_SIZE: usize = 64 * 1024;
const LINE_BUFFER: usize = 4 * READ_CHUNK;

#[derive(Debug, Error)]
enum RelayError {
    #[error("websocket handshake did not finish in {HANDSHAKE_TIMEOUT:?}")]
    HandshakeTimedOut,
    #[error("websocket: {0}")]
    Socket(#[from] tungstenite::Error),
    #[error("line: {0}")]
    Line(#[from] io::Error),
}

/// Upgrades one accepted socket and runs a line session on its bytes, logging instead of failing.
pub(crate) async fn serve_websocket(
    stream: TcpStream,
    peer: SocketAddr,
    settings: Arc<ConnectionSettings>,
) {
    if let Err(error) = stream.set_nodelay(true) {
        warn!(%error, %peer, "cannot disable Nagle; replies may lag");
    }
    let socket = match upgrade(stream).await {
        Ok(socket) => socket,
        Err(error) => {
            warn!(%error, %peer, "websocket refused");
            return;
        }
    };
    info!(%peer, "websocket upgraded");
    let (line, session_end) = tokio::io::duplex(LINE_BUFFER);
    let (relayed, ()) = tokio::join!(relay(socket, line), serve_line(session_end, peer, settings));
    if let Err(error) = relayed {
        warn!(%error, %peer, "websocket relay stopped");
    }
}

async fn upgrade(stream: TcpStream) -> Result<WebSocketStream<TcpStream>, RelayError> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_MESSAGE_SIZE));
    let handshake = tokio_tungstenite::accept_async_with_config(stream, Some(config));
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
        Ok(upgraded) => Ok(upgraded?),
        Err(_elapsed) => Err(RelayError::HandshakeTimedOut),
    }
}

/// Copies bytes both ways until either side ends; dropping `line` then ends the other side too.
async fn relay(socket: WebSocketStream<TcpStream>, line: DuplexStream) -> Result<(), RelayError> {
    let (mut to_browser, mut from_browser) = socket.split();
    let (mut from_session, mut to_session) = tokio::io::split(line);
    let mut chunk = vec![0; READ_CHUNK];
    loop {
        tokio::select! {
            message = from_browser.next() => {
                let Some(message) = message else { return Ok(()) };
                match message? {
                    Message::Binary(bytes) => to_session.write_all(&bytes).await?,
                    Message::Close(_) => return Ok(()),
                    Message::Text(text) => debug!(len = text.len(), "ignoring a text message"),
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            read = from_session.read(&mut chunk) => {
                let Some(bytes) = chunk.get(..read?).filter(|bytes| !bytes.is_empty()) else {
                    to_browser.send(Message::Close(None)).await?;
                    return Ok(());
                };
                to_browser.send(Message::binary(bytes.to_vec())).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use innkeeper_session::SessionConfig;
    use innkeeper_world::World;
    use pad_thai::LineKind;
    use tokio::net::TcpListener;
    use tokio_tungstenite::client_async;

    use crate::server::accept_forever;
    use crate::server::tests::{CAPTURED_LOGIN_FRAME, LOGIN_ACK_FRAME};

    type Browser = WebSocketStream<TcpStream>;

    async fn serve_on_loopback() -> SocketAddr {
        let session = SessionConfig {
            line: LineKind::Pad,
            ..SessionConfig::default()
        };
        let settings = ConnectionSettings::new(session, World::stock(), None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(
            listener,
            Arc::new(settings),
            serve_websocket,
        ));
        address
    }

    async fn open(address: SocketAddr) -> Browser {
        let stream = TcpStream::connect(address).await.unwrap();
        let (socket, _response) = client_async(format!("ws://{address}/"), stream)
            .await
            .unwrap();
        socket
    }

    /// Collects binary messages until exactly `expected` has arrived, however it was split.
    async fn expect_bytes(socket: &mut Browser, expected: &[u8]) {
        let mut received = Vec::new();
        while received.len() < expected.len() {
            let next = tokio::time::timeout(Duration::from_secs(5), socket.next());
            match next
                .await
                .expect("server answered in time")
                .unwrap()
                .unwrap()
            {
                Message::Binary(bytes) => received.extend_from_slice(&bytes),
                other => panic!("unexpected message {other:?}"),
            }
        }
        assert_eq!(
            String::from_utf8_lossy(&received),
            String::from_utf8_lossy(expected)
        );
    }

    async fn send(socket: &mut Browser, bytes: &[u8]) {
        socket.send(Message::binary(bytes.to_vec())).await.unwrap();
    }

    #[tokio::test]
    async fn a_websocket_line_logs_on_like_a_tcp_line() {
        let mut socket = open(serve_on_loopback().await).await;
        send(&mut socket, b"@D\r\r").await;
        expect_bytes(&mut socket, b"\r\nTERMINAL=\r\n@").await;
        send(&mut socket, b"c SIERRA\r").await;
        expect_bytes(&mut socket, b"\r\nSIERRA CONNECTED\r\n").await;
        for byte in CAPTURED_LOGIN_FRAME {
            send(&mut socket, &[byte]).await;
        }
        let link_ack = [0x81, 0x49, 0x62, 0x90, 0x82];
        expect_bytes(&mut socket, &[&link_ack[..], &LOGIN_ACK_FRAME].concat()).await;
    }

    #[tokio::test]
    async fn text_messages_are_ignored_and_the_line_carries_on() {
        let mut socket = open(serve_on_loopback().await).await;
        socket.send(Message::text("@D\r\r")).await.unwrap();
        send(&mut socket, b"@D\r\r").await;
        expect_bytes(&mut socket, b"\r\nTERMINAL=\r\n@").await;
    }

    #[tokio::test]
    async fn a_plain_http_request_is_refused_without_a_session() {
        let address = serve_on_loopback().await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        let mut reply = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut reply));
        read.await.expect("server closed in time").unwrap();
        assert!(!reply.starts_with(b"HTTP/1.1 101"), "{reply:?}");
    }
}
