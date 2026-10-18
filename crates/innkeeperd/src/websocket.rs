//! WebSocket ingress for the browser client: binary messages carry the raw serial bytes of one
//! legacy line session (docs/server/innkeeperd.md section 2) and text messages are the save
//! channel's JSON control messages (docs/server/saves.md).
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use hyper::body::Incoming;
use hyper::header::{HeaderMap, HeaderValue, CONNECTION, SEC_WEBSOCKET_ACCEPT, UPGRADE};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info, warn};

use innkeeper_world::AccountId;

use crate::connection::{serve_line, ConnectionSettings, READ_CHUNK};
use crate::connection_gate::{self, Binding};
use crate::http::{self, Reply};
use crate::save_channel::SaveChannel;

/// Serial bytes arrive a few at a time; a binary message larger than this is not a modem line.
const MAX_SERIAL_MESSAGE: usize = 64 * 1024;
/// A save file is at most 1 MiB, 4/3 of that as base64 in a JSON text message.
const MAX_MESSAGE_SIZE: usize = 2 * 1024 * 1024;
const LINE_BUFFER: usize = 4 * READ_CHUNK;

#[derive(Debug, Error)]
enum RelayError {
    #[error("websocket: {0}")]
    Socket(#[from] tungstenite::Error),
    #[error("a binary message of {0} bytes is not serial data")]
    SerialTooLarge(usize),
    #[error("line: {0}")]
    Line(#[from] io::Error),
}

/// Serves one accepted socket of the WebSocket listener, which also answers plain HTTP.
pub(crate) async fn serve_websocket(
    stream: TcpStream,
    addr: SocketAddr,
    settings: Arc<ConnectionSettings>,
) {
    http::serve(stream, addr, settings).await;
}

/// True for a request that asks to become a WebSocket, whatever its path.
pub(crate) fn is_upgrade<B>(request: &Request<B>) -> bool {
    has_token(request.headers(), UPGRADE, "websocket")
}

/// Checks the handshake and the session token, then answers 101 and runs the line once the
/// connection is handed over. A refusal is an HTTP status, so no WebSocket is opened.
pub(crate) fn accept(
    request: Request<Incoming>,
    addr: SocketAddr,
    settings: Arc<ConnectionSettings>,
) -> Reply {
    let peer = settings.peer_log(addr);
    let Some(key) = handshake_key(&request) else {
        warn!(%peer, "websocket refused: not a version 13 upgrade");
        return http::error(
            StatusCode::BAD_REQUEST,
            "bad_upgrade",
            "Not a WebSocket upgrade.",
        );
    };
    let binding = match connection_gate::admit(request.uri(), &settings) {
        Ok(binding) => binding,
        Err(refusal) => {
            warn!(%peer, %refusal, "websocket refused");
            crate::metrics::counters().ws_refused();
            return http::error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "The session is missing or has expired.",
            );
        }
    };
    let accept_key = derive_accept_key(key.as_bytes());
    tokio::spawn(async move {
        match hyper::upgrade::on(request).await {
            Ok(upgraded) => run(TokioIo::new(upgraded), addr, settings, binding).await,
            Err(error) => warn!(%error, %peer, "websocket refused"),
        }
    });
    let mut response = http::empty(StatusCode::SWITCHING_PROTOCOLS);
    let headers = response.headers_mut();
    headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
    if let Ok(value) = HeaderValue::from_str(&accept_key) {
        headers.insert(SEC_WEBSOCKET_ACCEPT, value);
    }
    response
}

fn handshake_key<B>(request: &Request<B>) -> Option<String> {
    let headers = request.headers();
    let version = headers.get("sec-websocket-version")?.to_str().ok()?;
    if request.method() != hyper::Method::GET
        || version.trim() != "13"
        || !has_token(headers, CONNECTION, "upgrade")
    {
        return None;
    }
    let key = headers.get("sec-websocket-key")?.to_str().ok()?.trim();
    (!key.is_empty()).then(|| key.to_owned())
}

fn has_token(headers: &HeaderMap, name: hyper::header::HeaderName, token: &str) -> bool {
    headers.get_all(name).iter().any(|value| {
        let text = value.to_str().unwrap_or("");
        text.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

/// Runs a line session on an upgraded connection's bytes, logging instead of failing.
async fn run<S>(
    io: S,
    addr: SocketAddr,
    settings: Arc<ConnectionSettings>,
    binding: Option<Binding>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let peer = settings.peer_log(addr);
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_MESSAGE_SIZE));
    let socket = WebSocketStream::from_raw_socket(io, Role::Server, Some(config)).await;
    info!(%peer, signed_in = binding.is_some(), "websocket upgraded");
    let (line, session_end) = tokio::io::duplex(LINE_BUFFER);
    let (logon, signed_on) = watch::channel(None);
    let saves = SaveChannel::new(Arc::clone(&settings.world.store));
    let (relayed, ()) = tokio::join!(
        relay(socket, line, saves, signed_on),
        serve_line(
            session_end,
            addr,
            settings,
            "websocket",
            Some(logon),
            binding
        )
    );
    if let Err(error) = relayed {
        warn!(%error, %peer, "websocket relay stopped");
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
async fn relay<S>(
    socket: WebSocketStream<S>,
    line: DuplexStream,
    mut saves: SaveChannel,
    mut signed_on: watch::Receiver<Option<AccountId>>,
) -> Result<(), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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
    use std::time::Duration;

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

    /// A server behind the proxy, sealing every line; the store is where its rows land.
    async fn serve_behind_proxy() -> (SocketAddr, Arc<crate::sqlite_store::SqliteStore>) {
        let store = Arc::new(crate::sqlite_store::SqliteStore::open_in_memory().unwrap());
        let mut settings = ConnectionSettings::new(SessionConfig::default(), World::stock(), None);
        settings.audit = Some(crate::audit::tests::seal_for(Arc::clone(&store)));
        settings.forwarding = crate::forwarded::Forwarding::Proxy {
            peer_when_absent: false,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(
            listener,
            Arc::new(settings),
            serve_websocket,
        ));
        (address, store)
    }

    #[tokio::test]
    async fn behind_the_proxy_the_sealed_record_names_the_forwarded_client() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let (address, store) = serve_behind_proxy().await;
        let mut request = format!("ws://{address}/").into_client_request().unwrap();
        request.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("10.9.9.9, 203.0.113.77"),
        );
        let stream = TcpStream::connect(address).await.unwrap();
        let (_socket, _) = client_async(request, stream).await.unwrap();
        let mut sealed = None;
        for _ in 0..100 {
            let id: Option<i64> = store
                .connection()
                .query_row("SELECT max(id) FROM audit_sessions", [], |row| row.get(0))
                .unwrap();
            if let Some(id) = id {
                sealed = Some(crate::audit::tests::plain_of(&store, id));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let plain = sealed.expect("a sealed row");
        assert_eq!(plain["peer_ip"], "203.0.113.77");
        assert!(plain["peer_port"].is_null(), "{plain}");
        assert_eq!(plain["transport"], "websocket");
    }

    #[tokio::test]
    async fn behind_the_proxy_an_upgrade_without_the_header_is_refused_unsealed() {
        let (address, store) = serve_behind_proxy().await;
        let head = "GET / HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
                    Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        let reply = crate::http::tests::request_head(address, head).await;
        assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
        let rows: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM audit_sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn a_plain_http_request_is_not_upgraded() {
        let address = serve_on_loopback().await;
        let head = "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
        let reply = crate::http::tests::request(address, head).await;
        assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");
    }
}
