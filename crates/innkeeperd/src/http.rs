//! Plain HTTP on the WebSocket listener: the upgrade, `/healthz` and the sign-in routes
//! (docs/server/innkeeperd.md section 8).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, CACHE_CONTROL, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tracing::{debug, warn};

use crate::connection::ConnectionSettings;
use crate::{forwarded, websocket};

/// A browser that opens the socket but never finishes its request head is dropped after this.
pub(crate) const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// The sign-in JSON bodies are a few dozen bytes.
const MAX_BODY: usize = 16 * 1024;

pub(crate) type Reply = Response<Full<Bytes>>;

/// Serves one accepted socket: HTTP/1.1 requests until the client goes, or one WebSocket.
pub(crate) async fn serve(stream: TcpStream, addr: SocketAddr, settings: Arc<ConnectionSettings>) {
    if let Err(error) = stream.set_nodelay(true) {
        warn!(%error, peer = %settings.peer_log(addr), "cannot disable Nagle; replies may lag");
    }
    let service = service_fn(move |request| {
        let settings = Arc::clone(&settings);
        async move { Ok::<_, Infallible>(route(request, addr, settings).await) }
    });
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT);
    let connection = builder
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades();
    if let Err(error) = connection.await {
        debug!(%error, "http connection ended");
    }
}

async fn route(
    request: Request<Incoming>,
    addr: SocketAddr,
    settings: Arc<ConnectionSettings>,
) -> Reply {
    if websocket::is_upgrade(&request) {
        // The sealed record and the log token name the client, never the proxy's loopback address.
        return match forwarded::client_addr(request.headers(), addr, settings.forwarding) {
            Ok(client) => websocket::accept(request, client, settings),
            Err(why) => {
                warn!(%why, "websocket refused: no client address");
                crate::metrics::counters().ws_refused();
                error(
                    StatusCode::BAD_REQUEST,
                    "no_client_address",
                    "This server is reached through its proxy only.",
                )
            }
        };
    }
    let path = request.uri().path();
    if path == "/healthz" {
        return text(StatusCode::OK, "ok");
    }
    if path.starts_with("/auth/") {
        if let Some(sign_in) = settings.sign_in.clone() {
            return sign_in.serve(request).await;
        }
    }
    error(StatusCode::NOT_FOUND, "not_found", "No such endpoint.")
}

/// The request body as JSON, or `None` when it is too large or not JSON.
pub(crate) async fn read_json(request: Request<Incoming>) -> Option<Value> {
    let collected = Limited::new(request.into_body(), MAX_BODY)
        .collect()
        .await
        .ok()?;
    let bytes = collected.to_bytes();
    if bytes.is_empty() {
        return Some(json!({}));
    }
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn reply(status: StatusCode, content_type: Option<&'static str>, body: Bytes) -> Reply {
    let mut response = Response::new(Full::new(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(kind) = content_type {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    }
    response
}

pub(crate) fn empty(status: StatusCode) -> Reply {
    reply(status, None, Bytes::new())
}

pub(crate) fn text(status: StatusCode, body: &'static str) -> Reply {
    reply(
        status,
        Some("text/plain; charset=utf-8"),
        Bytes::from_static(body.as_bytes()),
    )
}

pub(crate) fn json_reply(status: StatusCode, value: &Value) -> Reply {
    reply(
        status,
        Some("application/json"),
        Bytes::from(value.to_string()),
    )
}

/// The contract's error shape: `{"error": "<code>", "message": "<words>"}`.
pub(crate) fn error(status: StatusCode, code: &str, message: &str) -> Reply {
    json_reply(status, &json!({"error": code, "message": message}))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use innkeeper_session::SessionConfig;
    use innkeeper_world::World;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use crate::server::accept_forever;

    async fn serve_on_loopback() -> SocketAddr {
        let settings = ConnectionSettings::new(SessionConfig::default(), World::stock(), None);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(listener, Arc::new(settings), serve));
        address
    }

    pub(crate) async fn request(address: SocketAddr, head: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut reply));
        read.await.expect("server closed in time").unwrap();
        String::from_utf8_lossy(&reply).into_owned()
    }

    /// The status line and headers of one answer, for a server that keeps the connection open.
    pub(crate) async fn request_head(address: SocketAddr, head: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        let mut chunk = [0u8; 1024];
        while !reply.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk));
            let n = read.await.expect("server answered in time").unwrap();
            assert!(n > 0, "server closed early");
            reply.extend_from_slice(&chunk[..n]);
        }
        String::from_utf8_lossy(&reply).into_owned()
    }

    #[tokio::test]
    async fn healthz_answers_ok_without_credentials() {
        let address = serve_on_loopback().await;
        let reply = request(
            address,
            "GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
        assert!(reply.contains("cache-control: no-store"), "{reply}");
        assert!(reply.ends_with("\r\n\r\nok"), "{reply}");
    }

    #[tokio::test]
    async fn sign_in_routes_are_absent_until_sign_in_is_configured() {
        let address = serve_on_loopback().await;
        let reply = request(
            address,
            "GET /auth/providers HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 404 Not Found\r\n"), "{reply}");
        assert!(reply.contains(r#""error":"not_found""#), "{reply}");
    }
}
