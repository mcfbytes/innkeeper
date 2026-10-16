//! Sign-in over real sockets: CORS, enrolment, the WebSocket gate and the bound Login.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_session::SessionConfig;
use innkeeper_world::{
    AccountId, AccountRecord, ClientMessage, ClientVersion, ConnectionId, HostMessage, LandType,
    Login, PasswordSource, World,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use url::Url;
use zeroize::Zeroizing;

use super::config::{AuthFile, PageOrigins};
use super::keys::{self, HmacKey, IdentityKey, KeyRing};
use super::rp::{Provider, Rp};
use super::store::{self, Created};
use super::{EnrolmentMode, SignIn, SignInSetup};
use crate::connection::ConnectionSettings;
use crate::connection_gate::{self, Refusal};
use crate::host::Host;
use crate::server::accept_forever;
use crate::sqlite_store::SqliteStore;

const PAGE: &str = "http://127.0.0.1:2901";
const PROVIDERS: &str = r#"
[[provider]]
id = "dev"
name = "Dev Phone Co."
issuer = "http://127.0.0.1:1"
client_id = "client"
client_secret_name = "dev-secret"
"#;

struct Fixture {
    store: Arc<SqliteStore>,
    sign_in: Arc<SignIn>,
    settings: Arc<ConnectionSettings>,
    address: SocketAddr,
    http: reqwest::Client,
}

fn sign_in_for(store: &Arc<SqliteStore>, enrolment: EnrolmentMode, per_day: u32) -> SignIn {
    let entry = AuthFile::parse(PROVIDERS).unwrap().providers.remove(0);
    let provider = Provider::from_entry(&entry, Zeroizing::new("s".into())).unwrap();
    let key = HmacKey::from_base64("v1", &BASE64.encode([5; 32])).unwrap();
    SignIn::new(SignInSetup {
        store: Arc::clone(store),
        keys: KeyRing::new(vec![key]).unwrap(),
        rp: Rp::new(vec![provider], Url::parse("http://127.0.0.1:2900").unwrap()).unwrap(),
        origins: PageOrigins::parse(&[PAGE.to_owned()]).unwrap(),
        enrolment,
        session_lifetime: Duration::from_secs(3600),
        accounts_per_day: per_day,
        minimum_age: 18,
        terms_version: 1,
        recipient: None,
    })
}

async fn fixture(enrolment: EnrolmentMode, per_day: u32, require_session: bool) -> Fixture {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    let sign_in = Arc::new(sign_in_for(&store, enrolment, per_day));
    let mut world = World::stored(Arc::clone(&store) as Arc<dyn innkeeper_world::Store>);
    world.accounts = crate::server::account_book(&store, enrolment);
    let mut settings = ConnectionSettings::new(SessionConfig::default(), world, None);
    settings.sign_in = Some(Arc::clone(&sign_in));
    settings.require_session = require_session;
    let settings = Arc::new(settings);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(accept_forever(
        listener,
        Arc::clone(&settings),
        crate::websocket::serve_websocket,
    ));
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    Fixture {
        store,
        sign_in,
        settings,
        address,
        http,
    }
}

impl Fixture {
    /// A signed-in identity and a live token for it, without a provider round trip.
    fn token(&self, subject: &str) -> (String, IdentityKey) {
        let (token, key) = self.token_unattested(subject);
        let on = "2026-10-16".to_owned();
        let said = store::Attested {
            age: 18,
            terms: 1,
            on,
        };
        store::record_attestation(&self.store, &key, &said).unwrap();
        (token, key)
    }

    /// Like `token`, but the identity has not said its age or accepted the terms.
    fn token_unattested(&self, subject: &str) -> (String, IdentityKey) {
        let keys = KeyRing::new(vec![
            HmacKey::from_base64("v1", &BASE64.encode([5; 32])).unwrap()
        ])
        .unwrap();
        let now = crate::dates::unix_now();
        let signed =
            store::sign_in(&self.store, &keys, "iss", subject, "dev", || None, now).unwrap();
        let token = keys::new_token();
        store::create_session(
            &self.store,
            &signed.key,
            &keys::token_hash(&token),
            now,
            now + 3600,
        )
        .unwrap();
        (token.to_string(), signed.key)
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut request = self.http.request(method, self.url(path)).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn cors_names_only_the_configured_page_origin() {
    let fx = fixture(EnrolmentMode::Open, 3, false).await;
    let preflight = fx
        .http
        .request(reqwest::Method::OPTIONS, fx.url("/auth/session"))
        .header("origin", PAGE)
        .header("access-control-request-method", "GET")
        .header("access-control-request-headers", "authorization")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 204);
    let headers = preflight.headers();
    assert_eq!(headers["access-control-allow-origin"], PAGE);
    assert_eq!(
        headers["access-control-allow-methods"],
        "GET, POST, OPTIONS"
    );
    assert_eq!(
        headers["access-control-allow-headers"],
        "Authorization, Content-Type"
    );
    assert_eq!(headers["vary"], "Origin");
    assert_eq!(headers["cache-control"], "no-store");
    assert!(headers.get("access-control-allow-credentials").is_none());
    let other = fx
        .http
        .get(fx.url("/auth/providers"))
        .header("origin", "https://evil.test")
        .send()
        .await
        .unwrap();
    assert!(other.headers().get("access-control-allow-origin").is_none());
    let list: Value = other.json().await.unwrap();
    assert_eq!(
        list,
        json!([{"id": "dev", "name": "Dev Phone Co.", "url": "/auth/login?provider=dev"}])
    );
}

#[tokio::test]
async fn the_login_checks_the_provider_and_the_return_origin() {
    let fx = fixture(EnrolmentMode::Open, 3, false).await;
    for (query, code) in [
        (
            "provider=nope&return=http%3A%2F%2F127.0.0.1%3A2901%2Finn.html",
            "bad_provider",
        ),
        (
            "provider=dev&return=https%3A%2F%2Fevil.test%2Finn.html",
            "bad_return",
        ),
        (
            "provider=dev&return=http%3A%2F%2F127.0.0.1%3A29010%2F",
            "bad_return",
        ),
    ] {
        let response = fx
            .http
            .get(fx.url(&format!("/auth/login?{query}")))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{query}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"], code, "{query}");
    }
    let stale = fx
        .http
        .get(fx.url("/auth/callback?state=unknown&code=x"))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 400);
}

#[tokio::test]
async fn the_session_answer_follows_the_contract() {
    let fx = fixture(EnrolmentMode::Open, 3, false).await;
    let (missing, _) = fx
        .call(reqwest::Method::GET, "/auth/session", "nope", None)
        .await;
    assert_eq!(missing, 401);
    let (token, key) = fx.token_unattested("alice");
    let (status, session) = fx
        .call(reqwest::Method::GET, "/auth/session", &token, None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(session["identity"]["handle"], keys::handle(&key));
    assert_eq!(session["identity"]["provider"], "dev");
    assert_eq!(session["accounts"], json!([]));
    assert_eq!(session["canCreate"], true);
    assert_eq!(session["attested"], Value::Null);
    let (bad, body) = fx
        .call(
            reqwest::Method::POST,
            "/auth/attest",
            &token,
            Some(json!({"attest": "18"})),
        )
        .await;
    assert_eq!((bad, body["error"].as_str()), (400, Some("bad_attest")));
    let (ok, _) = fx
        .call(
            reqwest::Method::POST,
            "/auth/attest",
            &token,
            Some(json!({"attest": "18+v2"})),
        )
        .await;
    assert_eq!(ok, 204);
    let (_, session) = fx
        .call(reqwest::Method::GET, "/auth/session", &token, None)
        .await;
    assert_eq!(session["attested"], json!({"age": 18, "terms": 2}));
}

#[tokio::test]
async fn accounts_follow_enrolment_the_rate_limit_and_ownership() {
    let closed = fixture(EnrolmentMode::Closed, 3, false).await;
    let (token, _) = closed.token("alice");
    let (status, body) = closed
        .call(
            reqwest::Method::POST,
            "/auth/accounts",
            &token,
            Some(json!({"persona": "Elaine"})),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("enrolment_closed"))
    );
    let (_, session) = closed
        .call(reqwest::Method::GET, "/auth/session", &token, None)
        .await;
    assert_eq!(session["canCreate"], false);

    let fx = fixture(EnrolmentMode::WebOnly, 1, false).await;
    let (token, _) = fx.token("alice");
    let post = |persona: &str| json!({"persona": persona});
    let (status, body) = fx
        .call(
            reqwest::Method::POST,
            "/auth/accounts",
            &token,
            Some(post("a b")),
        )
        .await;
    assert_eq!((status, body["error"].as_str()), (400, Some("bad_persona")));
    let (status, created) = fx
        .call(
            reqwest::Method::POST,
            "/auth/accounts",
            &token,
            Some(post("Elaine")),
        )
        .await;
    assert_eq!(status, 201);
    assert_eq!(created, json!({"id": 100_001, "persona": "Elaine"}));
    let (status, body) = fx
        .call(
            reqwest::Method::POST,
            "/auth/accounts",
            &token,
            Some(post("Carla")),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (429, Some("rate_limited"))
    );

    let (other, _) = fx.token("bob");
    let (status, body) = fx
        .call(
            reqwest::Method::POST,
            "/auth/accounts",
            &other,
            Some(post("ELAINE")),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (409, Some("persona_taken"))
    );
    let path = "/auth/accounts/100001/credential";
    let (status, _) = fx.call(reqwest::Method::POST, path, &other, None).await;
    assert_eq!(status, 404, "another identity's account");
    let (status, first) = fx.call(reqwest::Method::POST, path, &token, None).await;
    assert_eq!(status, 200);
    let (_, second) = fx.call(reqwest::Method::POST, path, &token, None).await;
    assert_eq!(first["account"], 100_001);
    assert_ne!(first["password"], second["password"], "every call rotates");
    let stored = innkeeper_world::Store::account(&*fx.store, AccountId(100_001))
        .unwrap()
        .unwrap();
    let password = second["password"].as_str().unwrap();
    assert_eq!(Some(stored.password), keys::encode_password(password));
    assert_eq!(
        stored.user_flags,
        AccountRecord::new(stored.password, "x").user_flags
    );
}

#[tokio::test]
async fn an_account_or_password_needs_the_age_and_terms_answer() {
    let fx = fixture(EnrolmentMode::WebOnly, 3, false).await;
    let (token, _) = fx.token_unattested("alice");
    let create = Some(json!({"persona": "Elaine"}));
    let (status, body) = fx
        .call(
            reqwest::Method::POST,
            "/auth/accounts",
            &token,
            create.clone(),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("attest_required"))
    );
    assert!(
        body["message"].as_str().unwrap().contains("18 or older"),
        "{body}"
    );
    let attest = |text: &str| Some(json!({"attest": text}));
    let post = reqwest::Method::POST;
    fx.call(post.clone(), "/auth/attest", &token, attest("17+v1"))
        .await;
    let (status, body) = fx
        .call(post.clone(), "/auth/accounts", &token, create.clone())
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("attest_required")),
        "too young"
    );
    fx.call(post.clone(), "/auth/attest", &token, attest("18+v0"))
        .await;
    let (status, _) = fx
        .call(post.clone(), "/auth/accounts", &token, create.clone())
        .await;
    assert_eq!(status, 403, "old terms");
    fx.call(post.clone(), "/auth/attest", &token, attest("18+v1"))
        .await;
    let (status, _) = fx
        .call(post.clone(), "/auth/accounts", &token, create)
        .await;
    assert_eq!(status, 201);
    // A credential is gated the same way, even for an owner whose answer was withdrawn.
    store::record_attestation(
        &fx.store,
        &fx.token_unattested("alice").1,
        &store::Attested {
            age: 13,
            terms: 1,
            on: "2026-10-16".into(),
        },
    )
    .unwrap();
    let path = "/auth/accounts/100001/credential";
    let (status, body) = fx.call(post, path, &token, None).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("attest_required"))
    );
}

fn upgrade_uri(query: &str) -> hyper::Uri {
    format!("/?{query}").parse().unwrap()
}

#[tokio::test]
async fn the_gate_needs_a_live_token_in_public_mode_only() {
    let public = fixture(EnrolmentMode::WebOnly, 3, true).await;
    let open = fixture(EnrolmentMode::Open, 3, false).await;
    for fx in [&public, &open] {
        let (token, key) = fx.token("alice");
        let bound = connection_gate::admit(&upgrade_uri(&format!("session={token}")), &fx.settings)
            .unwrap();
        assert_eq!(bound.map(|binding| binding.identity), Some(key));
        let dead = connection_gate::admit(&upgrade_uri("session=dead"), &fx.settings);
        assert_eq!(dead, Err(Refusal::DeadSession));
    }
    assert_eq!(
        connection_gate::admit(&upgrade_uri("x=1"), &public.settings),
        Err(Refusal::MissingSession)
    );
    assert_eq!(
        connection_gate::admit(&upgrade_uri("x=1"), &open.settings),
        Ok(None)
    );

    let mut default = ConnectionSettings::new(SessionConfig::default(), World::stock(), None);
    assert_eq!(
        connection_gate::admit(&upgrade_uri("session=any"), &default),
        Ok(None)
    );
    default.require_session = true;
    let refused = connection_gate::admit(&upgrade_uri("session=any"), &default);
    assert_eq!(refused, Err(Refusal::MissingSession));
}

#[tokio::test]
async fn a_public_upgrade_without_a_token_is_an_http_401() {
    let fx = fixture(EnrolmentMode::WebOnly, 3, true).await;
    let head = "GET / HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
                Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
    let reply = crate::http::tests::request_head(fx.address, head).await;
    assert!(reply.starts_with("HTTP/1.1 401"), "{reply}");
}

fn login(account: u32, password: &str) -> Vec<u8> {
    ClientMessage::Login(Login {
        land_type: LandType(1),
        version: ClientVersion::new(2, 3, 18),
        account: AccountId(account),
        password_source: PasswordSource::StoredFile,
        password: keys::encode_password(password).unwrap(),
        name: "guybrush".into(),
        prodigy_id: None,
    })
    .encode()
}

fn accepted(outgoing: &[HostMessage]) -> bool {
    matches!(outgoing.first(), Some(HostMessage::LoginAccepted(_)))
}

#[tokio::test]
async fn a_bound_line_logs_in_only_to_the_identity_s_accounts() {
    let fx = fixture(EnrolmentMode::WebOnly, 5, true).await;
    let (token, key) = fx.token("alice");
    let (other, other_key) = fx.token("bob");
    let record =
        |persona: &str| AccountRecord::new(keys::encode_password("SWORDFISH").unwrap(), persona);
    let Created::Account(mine) =
        store::create_owned_account(&fx.store, &key, &record("Elaine"), 0).unwrap()
    else {
        panic!("created");
    };
    let Created::Account(theirs) =
        store::create_owned_account(&fx.store, &other_key, &record("Carla"), 0).unwrap()
    else {
        panic!("created");
    };
    let binding = fx.sign_in.bind(&token).unwrap().unwrap();
    let mut host = Host::new(&fx.settings, ConnectionId(1));
    host.bind(Some(binding));
    assert!(accepted(&host.answer(&login(mine.0, "SWORDFISH")).outgoing));
    let refused = host.answer(&login(theirs.0, "SWORDFISH")).outgoing;
    assert!(
        matches!(refused.as_slice(), [HostMessage::Nak(nak)] if nak.text == "That password is not right.")
    );
    let unknown = host.answer(&login(123_456, "SWORDFISH")).outgoing;
    assert!(
        !accepted(&unknown),
        "a bound line cannot open an unowned number"
    );
    assert!(fx.sign_in.bind(&other).unwrap().is_some());

    let logout = fx
        .http
        .post(fx.url("/auth/logout"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);
    assert!(fx.sign_in.bind(&token).unwrap().is_none());
    assert!(
        !accepted(&host.answer(&login(mine.0, "SWORDFISH")).outgoing),
        "the token is dead"
    );
}

#[test]
fn a_signup_creates_nothing_unless_enrolment_is_open() {
    let signup = |settings: &ConnectionSettings| {
        let body = innkeeper_world::Signup {
            cookie: 1,
            form: vec![0; 4],
        };
        let mut host = Host::new(settings, ConnectionId(1));
        host.answer(&ClientMessage::Signup(body).encode()).outgoing
    };
    let mut settings = ConnectionSettings::new(SessionConfig::default(), World::stock(), None);
    settings.signup_open = false;
    assert!(signup(&settings).is_empty());
    settings.signup_open = true;
    assert_eq!(signup(&settings).len(), 1);
}
