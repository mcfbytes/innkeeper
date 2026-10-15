//! WebSocket ingress for the browser client: binary messages carry the raw serial bytes of one
//! legacy line session (docs/server/innkeeperd.md section 2) and text messages are the save
//! channel's JSON control messages (docs/server/saves.md).
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info, warn};

use innkeeper_world::AccountId;

use crate::connection::{serve_line, ConnectionSettings, READ_CHUNK};
use crate::save_channel::SaveChannel;

/// A browser that opens the socket but never finishes the upgrade is dropped after this.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Serial bytes arrive a few at a time; a binary message larger than this is not a modem line.
const MAX_SERIAL_MESSAGE: usize = 64 * 1024;
/// A save file is at most 1 MiB, 4/3 of that as base64 in a JSON text message.
const MAX_MESSAGE_SIZE: usize = 2 * 1024 * 1024;
const LINE_BUFFER: usize = 4 * READ_CHUNK;

#[derive(Debug, Error)]
enum RelayError {
    #[error("websocket handshake did not finish in {HANDSHAKE_TIMEOUT:?}")]
    HandshakeTimedOut,
    #[error("websocket: {0}")]
    Socket(#[from] tungstenite::Error),
    #[error("a binary message of {0} bytes is not serial data")]
    SerialTooLarge(usize),
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
    let (logon, signed_on) = watch::channel(None);
    let saves = SaveChannel::new(Arc::clone(&settings.world.store));
    let (relayed, ()) = tokio::join!(
        relay(socket, line, saves, signed_on),
        serve_line(session_end, peer, settings, Some(logon))
    );
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

fn now_millis() -> u64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH);
    since_epoch.map_or(0, |elapsed| {
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    })
}

/// Copies bytes both ways until either side ends; dropping `line` then ends the other side too.
/// Text messages go to the save channel, which learns the account from `signed_on`.
async fn relay(
    socket: WebSocketStream<TcpStream>,
    line: DuplexStream,
    mut saves: SaveChannel,
    mut signed_on: watch::Receiver<Option<AccountId>>,
) -> Result<(), RelayError> {
    let mut watching = true;
    let (mut to_browser, mut from_browser) = socket.split();
    let (mut from_session, mut to_session) = tokio::io::split(line);
    let mut chunk = vec![0; READ_CHUNK];
    loop {
        tokio::select! {
            message = from_browser.next() => {
                let Some(message) = message else { return Ok(()) };
                match message? {
                    Message::Binary(bytes) if bytes.len() > MAX_SERIAL_MESSAGE => {
                        return Err(RelayError::SerialTooLarge(bytes.len()));
                    }
                    Message::Binary(bytes) => to_session.write_all(&bytes).await?,
                    Message::Close(_) => return Ok(()),
                    Message::Text(text) => {
                        debug!(len = text.len(), "save channel request");
                        for reply in saves.handle(&text, now_millis()) {
                            to_browser.send(Message::text(reply)).await?;
                        }
                    }
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            changed = signed_on.changed(), if watching => {
                if changed.is_err() {
                    watching = false;
                    continue;
                }
                let account = *signed_on.borrow_and_update();
                for reply in saves.set_account(account) {
                    to_browser.send(Message::text(reply)).await?;
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

    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use innkeeper_session::SessionConfig;
    use innkeeper_world::World;
    use pad_thai::LineKind;
    use serde_json::json;
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
                // The save channel's manifest follows a logon; it is checked in its own tests.
                Message::Text(_) => {}
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

    async fn log_on(socket: &mut Browser) {
        send(socket, b"@D\r\r").await;
        expect_bytes(socket, b"\r\nTERMINAL=\r\n@").await;
        send(socket, b"c SIERRA\r").await;
        expect_bytes(socket, b"\r\nSIERRA CONNECTED\r\n").await;
        send(socket, &CAPTURED_LOGIN_FRAME).await;
    }

    /// The next text message as JSON; binary line bytes in between are skipped.
    async fn next_json(socket: &mut Browser) -> serde_json::Value {
        loop {
            let next = tokio::time::timeout(Duration::from_secs(5), socket.next());
            match next
                .await
                .expect("server answered in time")
                .unwrap()
                .unwrap()
            {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Binary(_) => {}
                other => panic!("unexpected message {other:?}"),
            }
        }
    }

    async fn ask(socket: &mut Browser, request: serde_json::Value) -> serde_json::Value {
        socket
            .send(Message::text(request.to_string()))
            .await
            .unwrap();
        next_json(socket).await
    }

    #[tokio::test]
    async fn the_save_channel_refuses_before_logon_and_the_line_carries_on() {
        let mut socket = open(serve_on_loopback().await).await;
        let reply = ask(&mut socket, json!({"op": "manifest", "id": 7})).await;
        assert_eq!(
            (reply["op"].as_str(), reply["code"].as_str()),
            (Some("error"), Some("not_logged_on"))
        );
        assert_eq!(reply["id"], 7);
        send(&mut socket, b"@D\r\r").await;
        expect_bytes(&mut socket, b"\r\nTERMINAL=\r\n@").await;
    }

    #[tokio::test]
    async fn the_manifest_follows_logon_and_saves_round_trip() {
        let address = serve_on_loopback().await;
        let mut socket = open(address).await;
        log_on(&mut socket).await;
        let manifest = next_json(&mut socket).await;
        assert_eq!(manifest["op"], "manifest");
        assert_eq!(manifest["files"], json!([]));
        let account = manifest["account"].clone();

        let path = "inn/yserbius/mychars.dat";
        let big: Vec<u8> = (0..168_000u32).map(|n| (n % 251) as u8).collect();
        let put = json!({"op": "put", "path": path, "base": 0, "mtime": 1000, "data": BASE64.encode(&big)});
        let stored = ask(&mut socket, put).await;
        assert_eq!(
            (stored["op"].as_str(), stored["version"].as_u64()),
            (Some("stored"), Some(1))
        );
        assert_eq!(stored["path"], "INN\\YSERBIUS\\MYCHARS.DAT");
        assert_eq!(stored["hash"], innkeeper_world::saves::hash_hex(&big));

        let file = ask(&mut socket, json!({"op": "get", "path": path})).await;
        assert_eq!(BASE64.decode(file["data"].as_str().unwrap()).unwrap(), big);
        let listed = ask(&mut socket, json!({"op": "manifest"})).await;
        assert_eq!(listed["files"][0]["size"], 168_000);

        // Another line that signs on as the same account finds the file in its manifest.
        let mut second = open(address).await;
        log_on(&mut second).await;
        let manifest = next_json(&mut second).await;
        assert_eq!(manifest["account"], account);
        assert_eq!(manifest["files"][0]["version"], 1);
    }

    #[tokio::test]
    async fn an_oversized_binary_message_closes_the_line() {
        let mut socket = open(serve_on_loopback().await).await;
        send(&mut socket, &vec![0; MAX_SERIAL_MESSAGE + 1]).await;
        let next = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap();
        assert!(
            matches!(next, None | Some(Ok(Message::Close(_)) | Err(_))),
            "{next:?}"
        );
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
