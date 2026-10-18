//! Helpers shared by the sign-in and portal end-to-end tests: a jailed innkeeperd as a child
//! process, the mock provider, and a WebSocket line speaking the link layer.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

#[allow(dead_code)]
#[path = "../../src/dev_oidc.rs"]
pub mod dev_oidc;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use innkeeper_world::{
    AccountId, ClientMessage, ClientVersion, EncodedPassword, LandType, Login, PasswordSource,
};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};
use tsn_link::{Link, LinkConfig, LinkOutput};

pub const PAGE: &str = "http://127.0.0.1:2901";
pub const RETURN: &str = "http://127.0.0.1:2901/inn.html?auth=x";

pub struct Server {
    pub child: Child,
    log: Arc<Mutex<String>>,
    pub dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Server {
    pub fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }
}

pub fn free_port() -> u16 {
    loop {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        if !matches!(port, 2314 | 2316 | 8080 | 8081) {
            return port;
        }
    }
}

fn spawn(dir: &Path, port: u16, sandbox: bool, extra: &[String]) -> Server {
    let mut args: Vec<String> = [
        "--public",
        "--insecure-dev-keys",
        "--ws-bind",
        &format!("127.0.0.1:{port}"),
        "--data-dir",
        &dir.join("data").display().to_string(),
        "--auth-config",
        &dir.join("auth.toml").display().to_string(),
        "--page-origin",
        PAGE,
    ]
    .map(str::to_owned)
    .to_vec();
    if !sandbox {
        args.push("--no-sandbox".into());
    }
    args.extend(extra.iter().cloned());
    let mut child = Command::new(env!("CARGO_BIN_EXE_innkeeperd"))
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let log = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&log);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    Server {
        child,
        log,
        dir: dir.to_path_buf(),
    }
}

/// Starts innkeeperd jailed; a kernel that cannot install seccomp gets an unjailed run.
pub fn start(dir: &Path, port: u16, extra: &[String]) -> Server {
    for sandbox in [true, false] {
        let mut server = spawn(dir, port, sandbox, extra);
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let text = server.log();
            let portal = !extra.iter().any(|arg| arg == "--admin-bind")
                || text.contains("serving the operator portal");
            if text.contains("serving WebSocket lines") && portal {
                return server;
            }
            if server.child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let text = server.log();
        if !(sandbox && text.contains("seccomp:")) {
            panic!("innkeeperd did not start\n{text}");
        }
        eprintln!("seccomp is not available here; running the sign-in test without the sandbox");
        server.dir = PathBuf::new();
    }
    unreachable!()
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

pub fn location(response: &reqwest::Response) -> String {
    response.headers()["location"].to_str().unwrap().to_owned()
}

/// The redirect-mode login: our server, the mock's consent page (Allow), the callback, the page.
pub async fn sign_in(base: &str, subject: &str) -> String {
    let (_, page) = redirect_login(base, subject, "").await;
    fragment_value(&page, "inn-session").unwrap_or_else(|| panic!("{page}"))
}

/// The page nonce a login carries unless `extra` names its own: a public server needs one.
pub const NONCE: &str = "testNonce_0123456789-ab";

/// One value of the page address's fragment, after `RETURN#`.
pub fn fragment_value(page: &str, name: &str) -> Option<String> {
    let fragment = page.strip_prefix(&format!("{RETURN}#"))?;
    fragment
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .map(str::to_owned)
}

/// One redirect-mode login with `extra` appended to its query: the callback URL the browser was
/// sent to, and the page address (with its fragment) the callback answered with.
pub async fn redirect_login(base: &str, subject: &str, extra: &str) -> (String, String) {
    let client = http();
    let nonce = match extra.contains("nonce=") {
        true => String::new(),
        false => format!("&nonce={NONCE}"),
    };
    let login = format!(
        "{base}/auth/login?provider=dev&mode=redirect&attest=18%2Bv1&return={}{nonce}{extra}",
        url::form_urlencoded::byte_serialize(RETURN.as_bytes()).collect::<String>()
    );
    let first = client.get(&login).send().await.unwrap();
    assert_eq!(first.status(), 302, "login");
    let consent = client.get(location(&first)).send().await.unwrap();
    assert_eq!(consent.status(), 200, "the mock shows its consent page");
    let allow = format!("{}&decision=allow&subject={subject}", location(&first));
    let back = client.get(&allow).send().await.unwrap();
    assert_eq!(back.status(), 302, "the mock redirects to the callback");
    let callback = location(&back);
    assert!(
        callback.starts_with(&format!("{base}/auth/callback?")),
        "{callback}"
    );
    let finished = client.get(&callback).send().await.unwrap();
    assert_eq!(finished.status(), 302, "the callback redirects to the page");
    (callback, location(&finished))
}

pub async fn call(
    base: &str,
    method: reqwest::Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut request = http()
        .request(method, format!("{base}{path}"))
        .header("origin", PAGE)
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    assert_eq!(response.headers()["access-control-allow-origin"], PAGE);
    (status, response.json().await.unwrap_or(Value::Null))
}

/// One WebSocket line speaking the link layer through a `Link` of its own.
pub struct Line {
    pub socket: WebSocketStream<TcpStream>,
    pub link: Link,
}

impl Line {
    pub async fn open(port: u16, token: Option<&str>) -> Result<Self, u16> {
        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let query = token.map(|t| format!("?session={t}")).unwrap_or_default();
        match client_async(format!("ws://127.0.0.1:{port}/{query}"), stream).await {
            Ok((socket, _)) => {
                let config = LinkConfig {
                    quiet_after_connect: Duration::ZERO,
                    ..LinkConfig::default()
                };
                Ok(Line {
                    socket,
                    link: Link::new(config, Instant::now()),
                })
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                Err(response.status().as_u16())
            }
            Err(other) => panic!("{other}"),
        }
    }

    pub async fn raw(&mut self, bytes: &[u8], until: &[u8]) {
        self.socket
            .send(Message::binary(bytes.to_vec()))
            .await
            .unwrap();
        let mut got = Vec::new();
        while !got.windows(until.len()).any(|w| w == until) {
            if let Message::Binary(bytes) = self.next().await {
                got.extend_from_slice(&bytes);
            }
        }
    }

    pub async fn next(&mut self) -> Message {
        let next = tokio::time::timeout(Duration::from_secs(5), self.socket.next());
        next.await
            .expect("server answered in time")
            .unwrap()
            .unwrap()
    }

    pub async fn call_host(&mut self) {
        self.raw(b"@D\r\r", b"TERMINAL=").await;
        self.raw(b"c SIERRA\r", b"CONNECTED").await;
    }

    /// Sends a Login and returns the host's first reply body.
    pub async fn login(&mut self, account: u32, password: &str) -> Vec<u8> {
        let body = ClientMessage::Login(Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(account),
            password_source: PasswordSource::StoredFile,
            password: encode(password),
            name: "guybrush".into(),
            prodigy_id: None,
        })
        .encode();
        let message = tsn_link::Message::try_new(body).unwrap();
        self.link.send_message(&message, Instant::now());
        self.link.flush(Instant::now());
        loop {
            if let Some(reply) = self.pump().await {
                return reply;
            }
            if let Message::Binary(bytes) = self.next().await {
                let escaped = self.link.handle_input(&bytes, Instant::now());
                assert_eq!(escaped, None);
            }
        }
    }

    pub async fn pump(&mut self) -> Option<Vec<u8>> {
        let mut delivered = None;
        while let Some(output) = self.link.poll_output() {
            match output {
                LinkOutput::ToClient(bytes) => {
                    self.socket.send(Message::binary(bytes)).await.unwrap()
                }
                LinkOutput::Delivered(message) => {
                    delivered = delivered.or(Some(message.body().to_vec()))
                }
                LinkOutput::Event(_) => {}
            }
        }
        delivered
    }
}

/// The client's stored form of a password (docs/protocol/messages.md section 4.1).
pub fn encode(password: &str) -> EncodedPassword {
    const MASK: &[u8; 10] = b"M34546788S";
    let bytes = password.to_ascii_uppercase().into_bytes();
    let mut out = [0u8; 10];
    for (i, byte) in bytes.iter().enumerate() {
        out[i] = byte ^ MASK[i];
    }
    let n = bytes.len();
    for i in 0..n {
        out[(i + 1) % n] ^= out[i];
    }
    EncodedPassword(out)
}

pub const ACK: u8 = 0;
pub const NAK: u8 = 1;
