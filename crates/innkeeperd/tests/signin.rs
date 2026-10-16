//! Sign-in end to end: a public innkeeperd, a mock OpenID Connect provider in this test, the page
//! contract over HTTP and a WebSocket Login with the issued credential.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

#[allow(dead_code)]
#[path = "../src/dev_oidc.rs"]
mod dev_oidc;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use innkeeper_world::{
    AccountId, ClientMessage, ClientVersion, EncodedPassword, LandType, Login, PasswordSource,
};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};
use tsn_link::{Link, LinkConfig, LinkOutput};

const PAGE: &str = "http://127.0.0.1:2901";
const RETURN: &str = "http://127.0.0.1:2901/inn.html?auth=x";

struct Server {
    child: Child,
    log: Arc<Mutex<String>>,
    dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Server {
    fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }
}

fn free_port() -> u16 {
    loop {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        if !matches!(port, 2314 | 2316 | 8080 | 8081) {
            return port;
        }
    }
}

fn spawn(dir: &Path, port: u16, sandbox: bool) -> Server {
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
fn start(dir: &Path, port: u16) -> Server {
    for sandbox in [true, false] {
        let mut server = spawn(dir, port, sandbox);
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let text = server.log();
            if text.contains("serving WebSocket lines") {
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

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn location(response: &reqwest::Response) -> String {
    response.headers()["location"].to_str().unwrap().to_owned()
}

/// The redirect-mode login: our server, the mock's consent page (Allow), the callback, the page.
async fn sign_in(base: &str, subject: &str) -> String {
    let client = http();
    let login = format!(
        "{base}/auth/login?provider=dev&mode=redirect&attest=18%2Bv1&return={}",
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
    let page = location(&finished);
    let token = page
        .strip_prefix(&format!("{RETURN}#inn-session="))
        .unwrap_or_else(|| panic!("{page}"));
    token.to_owned()
}

async fn call(
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
struct Line {
    socket: WebSocketStream<TcpStream>,
    link: Link,
}

impl Line {
    async fn open(port: u16, token: Option<&str>) -> Result<Self, u16> {
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

    async fn raw(&mut self, bytes: &[u8], until: &[u8]) {
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

    async fn next(&mut self) -> Message {
        let next = tokio::time::timeout(Duration::from_secs(5), self.socket.next());
        next.await
            .expect("server answered in time")
            .unwrap()
            .unwrap()
    }

    async fn call_host(&mut self) {
        self.raw(b"@D\r\r", b"TERMINAL=").await;
        self.raw(b"c SIERRA\r", b"CONNECTED").await;
    }

    /// Sends a Login and returns the host's first reply body.
    async fn login(&mut self, account: u32, password: &str) -> Vec<u8> {
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

    async fn pump(&mut self) -> Option<Vec<u8>> {
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
fn encode(password: &str) -> EncodedPassword {
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

const ACK: u8 = 0;
const NAK: u8 = 1;

#[tokio::test(flavor = "multi_thread")]
async fn a_web_sign_in_opens_its_own_account_and_nothing_else() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let key = rsa::RsaPrivateKey::new(&mut aes_gcm::aead::OsRng, 2048).unwrap();
    let provider = Arc::new(dev_oidc::MockProvider::new(issuer.clone(), key));
    tokio::spawn(Arc::clone(&provider).serve(listener));

    let port = free_port();
    let dir = std::env::temp_dir().join(format!("inn-signin-e2e-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let base = format!("http://127.0.0.1:{port}");
    std::fs::write(
        dir.join("auth.toml"),
        format!(
            "public_url = \"{base}\"\n\n[[provider]]\nid = \"dev\"\nname = \"Dev Phone Co.\"\n\
             issuer = \"{issuer}\"\nclient_id = \"inn-test\"\nclient_secret_name = \"dev-secret\"\n\n\
             [dev_secrets]\ndev-secret = \"not-a-real-secret\"\n"
        ),
    )
    .unwrap();
    let server = tokio::task::block_in_place(|| start(&dir, port));

    // 1. Providers.
    let listed: Value = http()
        .get(format!("{base}/auth/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        listed,
        json!([{"id": "dev", "name": "Dev Phone Co.", "url": "/auth/login?provider=dev"}])
    );

    // 2 and 3. A redirect-mode login, then the session it opened.
    let token = sign_in(&base, "alice").await;
    let (status, session) = call(&base, reqwest::Method::GET, "/auth/session", &token, None).await;
    assert_eq!(status, 200);
    assert_eq!(session["identity"]["provider"], "dev");
    assert!(session["identity"]["handle"]
        .as_str()
        .unwrap()
        .starts_with("Caller "));
    assert_eq!(session["attested"], json!({"age": 18, "terms": 1}));
    assert_eq!(session["canCreate"], true);

    // 4. A newer terms version.
    let attest = Some(json!({"attest": "18+v2"}));
    let (status, _) = call(&base, reqwest::Method::POST, "/auth/attest", &token, attest).await;
    assert_eq!(status, 204);

    // 5 and 6. An account and its credential.
    let persona = Some(json!({"persona": "Elaine"}));
    let (status, created) = call(
        &base,
        reqwest::Method::POST,
        "/auth/accounts",
        &token,
        persona,
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_u64().unwrap() as u32;
    let path = format!("/auth/accounts/{id}/credential");
    let (status, credential) = call(&base, reqwest::Method::POST, &path, &token, None).await;
    assert_eq!(status, 200);
    let password = credential["password"].as_str().unwrap().to_owned();
    assert_eq!(credential["account"], id);

    // Someone else's account, for step 8.
    let bob = sign_in(&base, "bob").await;
    let carla = Some(json!({"persona": "Carla"}));
    let (_, theirs) = call(&base, reqwest::Method::POST, "/auth/accounts", &bob, carla).await;
    let their_id = theirs["id"].as_u64().unwrap() as u32;
    let their_path = format!("/auth/accounts/{their_id}/credential");
    let (_, their_credential) = call(&base, reqwest::Method::POST, &their_path, &bob, None).await;
    let their_password = their_credential["password"].as_str().unwrap().to_owned();

    // 7. The WebSocket with the token logs in with the credential.
    let mut line = Line::open(port, Some(&token)).await.expect("upgraded");
    line.call_host().await;
    let reply = line.login(id, &password).await;
    assert_eq!(reply[0], ACK, "Login accepted: {reply:?}\n{}", server.log());

    // 8. The same token cannot open another identity's account, even with its password.
    let refused = line.login(their_id, &their_password).await;
    assert_eq!(refused[0], NAK, "{refused:?}");

    // 9. Public mode refuses a socket without a token before the upgrade.
    assert_eq!(Line::open(port, None).await.err(), Some(401));

    // 10. After logout the token opens nothing.
    let (status, _) = call(&base, reqwest::Method::POST, "/auth/logout", &token, None).await;
    assert_eq!(status, 204);
    assert_eq!(Line::open(port, Some(&token)).await.err(), Some(401));
    let (status, _) = call(&base, reqwest::Method::GET, "/auth/session", &token, None).await;
    assert_eq!(status, 401);

    let log = server.log();
    assert!(log.contains("logged in"), "{log}");
    assert!(
        log.contains("login refused: not an account of this sign-in"),
        "{log}"
    );
    for secret in [
        token.as_str(),
        password.as_str(),
        "not-a-real-secret",
        "alice",
    ] {
        assert!(!log.contains(secret), "the log holds {secret}\n{log}");
    }
    let mut server = server;
    assert!(
        server.child.try_wait().unwrap().is_none(),
        "innkeeperd is still running\n{log}"
    );
}
