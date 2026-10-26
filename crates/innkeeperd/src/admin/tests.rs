//! The portal over a real socket with an in-memory store: the cookie, origin and CSRF checks, the
//! headers, escaping, the audit log, and each action. The sign-in dance itself is in
//! `tests/admin.rs`, against the mock provider.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_session::SessionConfig;
use innkeeper_world::{
    AccountId, AccountRecord, BoardId, ClientMessage, ClientVersion, ConnectionId, HostMessage,
    LandType, Login, PasswordSource, Refusal, Store, World,
};
use tokio::net::TcpListener;
use url::Url;
use zeroize::Zeroizing;

use super::allowlist::{hex, Allowlist};
use super::{csrf_for, serve, store, Admin, AdminSetup};
use crate::connection::ConnectionSettings;
use crate::host::Host;
use crate::server::{accept_forever, account_book};
use crate::signin::config::{AuthFile, PageOrigins};
use crate::signin::keys::{self, HmacKey, IdentityKey, KeyRing};
use crate::signin::rp::{Provider, Rp};
use crate::signin::{EnrolmentMode, SignIn, SignInSetup};
use crate::sqlite_store::SqliteStore;

const ORIGIN: &str = "http://127.0.0.1:2981";
const PROVIDERS: &str = r#"
[[provider]]
id = "dev"
name = "Dev Phone Co."
issuer = "http://127.0.0.1:1"
client_id = "client"
client_secret_name = "dev-secret"
"#;
const HOSTILE: &str = "<script>alert('x')</script>\"&";

fn staff_key() -> IdentityKey {
    [0x42; 32]
}

struct Fx {
    admin: Arc<Admin>,
    store: Arc<SqliteStore>,
    settings: Arc<ConnectionSettings>,
    address: SocketAddr,
    http: reqwest::Client,
    token: String,
    csrf: String,
}

enum Door {
    /// The staff key is on a vault-style list.
    Listed,
    /// No source at all: the development fallback.
    Open,
}

fn sign_in_for(store: &Arc<SqliteStore>) -> SignIn {
    let entry = AuthFile::parse(PROVIDERS).unwrap().providers.remove(0);
    let provider = Provider::from_entry(&entry, Zeroizing::new("s".into())).unwrap();
    let key = HmacKey::from_base64("v1", &BASE64.encode([5; 32])).unwrap();
    SignIn::new(SignInSetup {
        store: Arc::clone(store),
        invites: Default::default(),
        keys: KeyRing::new(vec![key]).unwrap(),
        rp: Rp::new(vec![provider], Url::parse(ORIGIN).unwrap()).unwrap(),
        origins: PageOrigins::parse(&["http://127.0.0.1:2901".to_owned()]).unwrap(),
        enrolment: EnrolmentMode::Open,
        session_lifetime: Duration::from_secs(3600),
        accounts_per_day: 3,
        minimum_age: 18,
        terms_version: 1,
        recipient: None,
        required: false,
    })
}

async fn fixture(door: Door) -> Fx {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    let sign_in = Arc::new(sign_in_for(&store));
    let mut world = World::stored(Arc::clone(&store) as Arc<dyn Store>);
    world.accounts = account_book(&store, EnrolmentMode::Open);
    let mut settings = ConnectionSettings::new(SessionConfig::default(), world, None);
    settings.sign_in = Some(Arc::clone(&sign_in));
    let settings = Arc::new(settings);
    let list = match door {
        Door::Listed => Some(HashSet::from([staff_key()])),
        Door::Open => None,
    };
    let admin = Arc::new(Admin::new(AdminSetup {
        settings: Arc::clone(&settings),
        sign_in,
        store: Arc::clone(&store),
        allowlist: Allowlist::new(list, None, false).unwrap(),
        origin: Url::parse(ORIGIN).unwrap(),
        retention: innkeeper_world::RetentionPolicy::default(),
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(accept_forever(listener, Arc::clone(&admin), serve));
    let token = keys::new_token().to_string();
    let now = crate::dates::now_ms();
    store::create_session(
        &store,
        &keys::token_hash(&token),
        &staff_key(),
        now,
        now + crate::dates::HOUR,
    )
    .unwrap();
    Fx {
        csrf: csrf_for(&token),
        admin,
        store,
        settings,
        address,
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        token,
    }
}

struct Answer {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: String,
}

impl Fx {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Answer {
        let response = request.send().await.unwrap();
        Answer {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            body: response.text().await.unwrap_or_default(),
        }
    }

    async fn get(&self, path: &str) -> Answer {
        let request = self
            .http
            .get(self.url(path))
            .header("cookie", format!("inn-admin={}", self.token));
        self.send(request).await
    }

    async fn post(&self, path: &str, fields: &[(&str, &str)]) -> Answer {
        let mut form: Vec<(&str, &str)> = vec![("csrf", self.csrf.as_str())];
        form.extend_from_slice(fields);
        let request = self
            .http
            .post(self.url(path))
            .header("cookie", format!("inn-admin={}", self.token))
            .header("origin", ORIGIN)
            .form(&form);
        self.send(request).await
    }

    fn account(&self, id: u32, persona: &str) {
        let record = AccountRecord::new(keys::encode_password("SWORDFISH").unwrap(), persona);
        self.store.create_account(AccountId(id), record).unwrap();
    }

    fn audit_actions(&self) -> Vec<String> {
        store::recent_audit(&self.store, 100, None)
            .unwrap()
            .into_iter()
            .map(|entry| entry.action)
            .collect()
    }
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

/// A Letter body as the host stores it (six time bytes, then the envelope).
fn letter_body(from: u32, sender: &str, addressee: &str, subject: &str, text: &str) -> Vec<u8> {
    let padded = |text: &str| {
        let mut field = [0u8; 10];
        let n = text.len().min(10);
        field[..n].copy_from_slice(&text.as_bytes()[..n]);
        field
    };
    let mut body = vec![96, 0, 1, 12, 0, 0];
    body.extend([0u8; 10]);
    body.extend((from as u16).to_le_bytes());
    body.extend(((from >> 16) as u16).to_le_bytes());
    body.extend(padded(subject));
    body.extend(0u16.to_le_bytes());
    body.extend(padded(addressee));
    body.extend(padded(sender));
    body.extend(text.as_bytes());
    body.push(0);
    body
}

#[tokio::test]
async fn without_a_cookie_every_page_sends_you_to_the_entrance() {
    let fx = fixture(Door::Listed).await;
    for path in [
        "/admin/",
        "/admin/accounts",
        "/admin/audit",
        "/admin/letters",
    ] {
        let answer = fx.send(fx.http.get(fx.url(path))).await;
        assert_eq!(answer.status, 303, "{path}");
        assert_eq!(answer.headers["location"], "/admin/login");
    }
    let post = fx
        .send(
            fx.http
                .post(fx.url("/admin/kick"))
                .header("origin", ORIGIN)
                .form(&[("connection", "1")]),
        )
        .await;
    assert_eq!(
        post.status, 303,
        "a POST without a session is not served either"
    );
    let bogus = fx
        .send(
            fx.http
                .get(fx.url("/admin/"))
                .header("cookie", "inn-admin=not-a-token"),
        )
        .await;
    assert_eq!(bogus.status, 303);
    let elsewhere = fx.send(fx.http.get(fx.url("/other"))).await;
    assert_eq!(
        elsewhere.status, 404,
        "nothing outside /admin/ is served here"
    );
}

#[tokio::test]
async fn every_answer_carries_the_security_headers() {
    let fx = fixture(Door::Listed).await;
    for path in [
        "/admin/",
        "/admin/login",
        "/admin/static/admin.css",
        "/admin/healthz",
    ] {
        let answer = fx.get(path).await;
        assert_eq!(
            answer.headers["x-content-type-options"], "nosniff",
            "{path}"
        );
        assert_eq!(answer.headers["referrer-policy"], "no-referrer", "{path}");
        assert_eq!(answer.headers["x-frame-options"], "DENY", "{path}");
        let csp = answer.headers["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("default-src 'self'"), "{csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
        assert!(
            csp.contains("script-src 'self'") && !csp.contains("unsafe-inline"),
            "{csp}"
        );
    }
    assert!(fx.get("/admin/").await.headers["cache-control"]
        .to_str()
        .unwrap()
        .contains("no-store"));
    // No inline script or style anywhere on a page: the policy would block them.
    let page = fx.get("/admin/").await.body;
    assert!(
        !page.contains("<script>") && !page.contains(" style="),
        "{page}"
    );
}

#[tokio::test]
async fn a_post_needs_this_origin_and_the_session_s_csrf_token() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let path = "/admin/accounts/100001/suspend";
    let cookie = format!("inn-admin={}", fx.token);
    let post = |origin: Option<&str>, csrf: &str| {
        let mut request = fx
            .http
            .post(fx.url(path))
            .header("cookie", &cookie)
            .form(&[("csrf", csrf), ("confirm", "yes")]);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        request
    };
    let none = fx.send(post(None, &fx.csrf)).await;
    assert_eq!(none.status, 403, "no Origin and no Referer");
    let wrong = fx.send(post(Some("http://evil.example"), &fx.csrf)).await;
    assert_eq!(wrong.status, 403, "{}", wrong.body);
    let null = fx.send(post(Some("null"), &fx.csrf)).await;
    assert_eq!(null.status, 403, "a sandboxed frame says null");
    let cross = fx
        .send(post(Some("null"), &fx.csrf).header("sec-fetch-site", "cross-site"))
        .await;
    assert_eq!(cross.status, 403);

    let bad_token = fx.send(post(Some(ORIGIN), "AAAA")).await;
    assert_eq!(bad_token.status, 403);
    let missing = fx
        .send(
            fx.http
                .post(fx.url(path))
                .header("cookie", &cookie)
                .header("origin", ORIGIN)
                .form(&[("confirm", "yes")]),
        )
        .await;
    assert_eq!(missing.status, 403);
    assert!(
        !fx.store.suspended(AccountId(100_001)).unwrap(),
        "nothing changed"
    );
    assert!(fx.audit_actions().is_empty());
    // The Referer stands in for a missing Origin; the right pair works.
    let referer = fx
        .send(
            fx.http
                .post(fx.url(path))
                .header("cookie", &cookie)
                .header("referer", format!("{ORIGIN}/admin/accounts/100001"))
                .form(&[("csrf", fx.csrf.as_str()), ("confirm", "yes")]),
        )
        .await;
    assert_eq!(referer.status, 200, "{}", referer.body);
    assert!(fx.store.suspended(AccountId(100_001)).unwrap());
    // With Referrer-Policy: no-referrer a browser sends Origin: null for its own same-origin forms;
    // the Fetch Metadata header then vouches for it.
    let same_origin_form = fx
        .send(post(Some("null"), &fx.csrf).header("sec-fetch-site", "same-origin"))
        .await;
    assert_eq!(same_origin_form.status, 200, "{}", same_origin_form.body);
    // The token is bound to the session: another session's token does not work.
    assert_ne!(csrf_for("some-other-cookie"), fx.csrf);
}

#[tokio::test]
async fn a_get_never_changes_anything() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    for path in [
        "/admin/accounts/100001/suspend",
        "/admin/accounts/100001/delete",
        "/admin/kick?connection=1",
        "/admin/logout",
        "/admin/holds/place?account=100001&until=2099-01-01&reason=x",
        "/admin/letters/1/1/read?reason=because+I+can",
    ] {
        let answer = fx.get(path).await;
        assert!(
            matches!(answer.status, 404 | 200),
            "{path}: {}",
            answer.status
        );
    }
    assert!(!fx.store.suspended(AccountId(100_001)).unwrap());
    assert!(fx.store.account(AccountId(100_001)).unwrap().is_some());
    assert!(store::holds(&fx.store).unwrap().is_empty());
    assert!(fx.audit_actions().is_empty());
    assert_eq!(
        fx.get("/admin/").await.status,
        200,
        "the session survived the GET /admin/logout"
    );
}

#[test]
fn the_cookie_is_host_prefixed_and_secure_over_https_and_plain_on_loopback() {
    let make = |origin: &str| {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let sign_in = Arc::new(sign_in_for(&store));
        let settings = Arc::new(ConnectionSettings::new(
            SessionConfig::default(),
            World::stock(),
            None,
        ));
        Admin::new(AdminSetup {
            settings,
            sign_in,
            store,
            allowlist: Allowlist::new(Some(HashSet::new()), None, false).unwrap(),
            origin: Url::parse(origin).unwrap(),
            retention: innkeeper_world::RetentionPolicy::default(),
        })
    };
    let https = make("https://inn.example.org");
    assert_eq!(https.cookie_name(), "__Host-inn-admin");
    let text = https.cookie("TOKEN", 28_800);
    let text = text.to_str().unwrap();
    assert!(text.starts_with("__Host-inn-admin=TOKEN;"), "{text}");
    for part in [
        "Path=/;",
        "Secure",
        "HttpOnly",
        "SameSite=Strict",
        "Max-Age=28800",
    ] {
        assert!(text.contains(part), "{text}");
    }
    assert!(
        !text.contains("Domain"),
        "a __Host- cookie has no Domain: {text}"
    );
    let plain = make("http://127.0.0.1:2981");
    let text = plain.cookie("TOKEN", 28_800);
    let text = text.to_str().unwrap();
    assert!(text.starts_with("inn-admin=TOKEN;"), "{text}");
    assert!(!text.contains("Secure"), "{text}");
    assert!(
        text.contains("HttpOnly") && text.contains("SameSite=Strict"),
        "{text}"
    );
    assert!(text.contains("Path=/admin/"), "{text}");
}

#[tokio::test]
async fn revoking_the_key_ends_the_session_on_the_next_request() {
    let fx = fixture(Door::Listed).await;
    assert_eq!(fx.get("/admin/").await.status, 200);
    store::staff_revoke(&fx.store, &staff_key(), crate::dates::now_ms()).unwrap();
    let after = fx.get("/admin/").await;
    assert_eq!(
        after.status, 303,
        "the vault still lists the key; the revocation wins"
    );
    store::staff_allow(&fx.store, &staff_key(), None, crate::dates::now_ms()).unwrap();
    assert_eq!(
        fx.get("/admin/").await.status,
        303,
        "the session row was dropped with the permission, so signing in again is needed"
    );
}

#[tokio::test]
async fn an_idle_or_expired_session_is_not_served() {
    let fx = fixture(Door::Listed).await;
    let now = crate::dates::now_ms();
    fx.store
        .connection()
        .execute(
            "UPDATE admin_sessions SET last_seen_at = ?1",
            [now - 31 * crate::dates::MINUTE],
        )
        .unwrap();
    assert_eq!(fx.get("/admin/").await.status, 303, "idle for 31 minutes");
    let fresh = fixture(Door::Listed).await;
    fresh
        .store
        .connection()
        .execute("UPDATE admin_sessions SET expires_at = ?1", [now - 1])
        .unwrap();
    assert_eq!(
        fresh.get("/admin/").await.status,
        303,
        "past its eight hours"
    );
}

#[tokio::test]
async fn logging_out_ends_the_session_and_clears_the_cookie() {
    let fx = fixture(Door::Listed).await;
    let answer = fx.post("/admin/logout", &[]).await;
    assert_eq!(answer.status, 303);
    let cookie = answer.headers["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("Max-Age=0"), "{cookie}");
    assert_eq!(fx.get("/admin/").await.status, 303);
}

#[tokio::test]
async fn the_open_door_shows_its_banner_and_a_listed_door_does_not() {
    let open = fixture(Door::Open).await;
    let page = open.get("/admin/").await.body;
    assert!(page.contains("DEV: open door"), "{page}");
    let login = open
        .send(open.http.get(open.url("/admin/login")))
        .await
        .body;
    assert!(login.contains("DEV: open door"), "even the entrance warns");
    let listed = fixture(Door::Listed).await;
    assert!(!listed.get("/admin/").await.body.contains("DEV: open door"));
    assert!(open.admin.dev_open() && !listed.admin.dev_open());
}

#[tokio::test]
async fn hostile_strings_are_escaped_on_every_page_that_shows_them() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, HOSTILE);
    fx.account(100_002, "Bruno");
    let mailbox = fx.store.assign_mailbox(AccountId(100_001)).unwrap();
    let other = fx.store.assign_mailbox(AccountId(100_002)).unwrap();
    fx.store
        .append_letter(
            mailbox,
            AccountId(100_002),
            &letter_body(
                other.0,
                HOSTILE,
                HOSTILE,
                "<b>subj</b>",
                "<img src=x onerror=alert(1)>",
            ),
        )
        .unwrap();
    let board = BoardId(1 << 24 | 66 << 8 | 1);
    fx.store
        .post(
            board,
            AccountId(100_001),
            None,
            format!("{HOSTILE}|<iframe src=//evil>").as_bytes(),
        )
        .unwrap();
    let complaint = BoardId(99 << 8 | 3);
    fx.store
        .post(
            complaint,
            AccountId(100_001),
            Some(AccountId(100_002)),
            format!(
                "{HOSTILE}|Complaint: From: x; about: Bruno, who said \"<svg onload=alert(2)>\""
            )
            .as_bytes(),
        )
        .unwrap();
    fx.store
        .save_write(AccountId(100_001), "INN\\PLAYER.DIR", b"bytes", 1, true)
        .unwrap();
    store::record(
        &fx.store,
        &store::Who {
            handle: HOSTILE.into(),
            key: [1; 32],
        },
        "x<y",
        HOSTILE,
        Some(HOSTILE),
        None,
        1,
    )
    .unwrap();
    let hole = fx
        .post(
            "/admin/letters/1/1/read",
            &[("reason", "<script>alert('reason')</script> long enough")],
        )
        .await;
    assert_eq!(hole.status, 200, "{}", hole.body);
    let pages = [
        "/admin/accounts?q=script",
        "/admin/accounts/100001",
        "/admin/complaints",
        "/admin/complaints/2",
        "/admin/posts",
        "/admin/posts/16794113",
        "/admin/letters",
        "/admin/letters/1",
        "/admin/letters/1/1",
        "/admin/saves",
        "/admin/saves/100001",
        "/admin/audit",
        "/admin/holds",
    ];
    let mut shown = 0;
    for path in pages {
        let answer = fx.get(path).await;
        assert!(matches!(answer.status, 200), "{path}: {}", answer.status);
        for raw in [
            "<script>alert",
            "<iframe",
            "<img src=x",
            "<svg onload",
            "<b>subj",
        ] {
            assert!(!answer.body.contains(raw), "{path} leaks {raw}");
        }
        if answer.body.contains("&lt;script&gt;alert") {
            shown += 1;
        }
    }
    assert!(
        shown >= 8,
        "the hostile text was displayed, escaped, on {shown} pages"
    );
    for raw in ["<script>alert", "<img src=x", "<b>subj"] {
        assert!(!hole.body.contains(raw), "the letter page leaks {raw}");
    }
    assert!(
        hole.body.contains("&lt;img src=x onerror=alert(1)&gt;"),
        "{}",
        hole.body
    );
}

#[tokio::test]
async fn a_letter_cannot_be_read_without_a_typed_reason_and_every_read_is_recorded() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    fx.account(100_002, "Bruno");
    let a = fx.store.assign_mailbox(AccountId(100_001)).unwrap();
    let b = fx.store.assign_mailbox(AccountId(100_002)).unwrap();
    fx.store
        .append_letter(
            a,
            AccountId(100_002),
            &letter_body(b.0, "Bruno", "Elaine", "hello", "meet me at the pier"),
        )
        .unwrap();
    // The list shows metadata only, and says reading is recorded.
    let list = fx.get("/admin/letters/1").await;
    assert_eq!(list.status, 200);
    assert!(list.body.contains("Bruno") && list.body.contains("Elaine"));
    assert!(
        !list.body.contains("meet me at the pier"),
        "no body in a list"
    );
    let gate = fx.get("/admin/letters/1/1").await;
    assert_eq!(gate.status, 200);
    assert!(
        gate.body.contains("recorded in the audit log")
            || gate.body.contains("saved in the audit log"),
        "the page says so first"
    );
    assert!(
        !gate.body.contains("meet me at the pier"),
        "a GET never shows the words"
    );
    for reason in ["", "   ", "short"] {
        let refused = fx
            .post("/admin/letters/1/1/read", &[("reason", reason)])
            .await;
        assert_eq!(refused.status, 400, "{reason:?}");
        assert!(!refused.body.contains("meet me at the pier"));
    }
    assert!(
        fx.audit_actions().is_empty(),
        "a refused read records nothing and shows nothing"
    );
    let shown = fx
        .post(
            "/admin/letters/1/1/read",
            &[("reason", "Member Services case 4411, complaint review")],
        )
        .await;
    assert_eq!(shown.status, 200);
    assert!(shown.body.contains("meet me at the pier"));
    let log = store::recent_audit(&fx.store, 10, None).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action, "letter.read");
    assert_eq!(
        log[0].reason.as_deref(),
        Some("Member Services case 4411, complaint review")
    );
    assert_eq!(log[0].handle, keys::handle(&staff_key()));
    assert_eq!(log[0].key_hex, hex(&staff_key()));
    assert_eq!(log[0].target, "letter 1 in mailbox 1");
    assert!(
        !log[0].target.contains("pier") && !log[0].target.contains("hello"),
        "no subject or body in the log"
    );
    let wrong_box = fx
        .post(
            "/admin/letters/2/1/read",
            &[("reason", "a perfectly good reason")],
        )
        .await;
    assert_eq!(wrong_box.status, 404, "the letter is not in that mailbox");
    assert_eq!(fx.audit_actions().len(), 1);
}

#[tokio::test]
async fn suspending_refuses_the_next_login_kicks_the_line_and_is_audited() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let mut host = Host::new(&fx.settings, ConnectionId(7));
    let kicked = host.kick_signal();
    assert!(accepted(
        &host.answer(&login(100_001, "SWORDFISH")).outgoing
    ));
    let online = fx.get("/admin/").await.body;
    assert!(
        online.contains("guybrush") && online.contains("100001"),
        "the session persona and the account"
    );
    let ask = fx.post("/admin/accounts/100001/suspend", &[]).await;
    assert_eq!(ask.status, 409, "a suspension needs the yes");
    assert!(!fx.store.suspended(AccountId(100_001)).unwrap());
    let done = fx
        .post(
            "/admin/accounts/100001/suspend",
            &[("confirm", "yes"), ("reason", "spam")],
        )
        .await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert!(done.body.contains("is suspended"), "{}", done.body);
    tokio::time::timeout(Duration::from_secs(2), kicked.notified())
        .await
        .expect("the live line was kicked");
    let mut second = Host::new(&fx.settings, ConnectionId(8));
    let refused = second.answer(&login(100_001, "SWORDFISH")).outgoing;
    assert!(
        matches!(refused.as_slice(), [HostMessage::Nak(nak)] if nak.text.contains("suspended")),
        "{refused:?}"
    );
    assert_eq!(
        account_book(&fx.store, EnrolmentMode::Open)
            .admit(
                &match ClientMessage::parse(&login(100_001, "SWORDFISH")).unwrap() {
                    ClientMessage::Login(login) => login,
                    other => panic!("{other:?}"),
                }
            )
            .unwrap_err(),
        Refusal::Suspended
    );
    let undone = fx.post("/admin/accounts/100001/unsuspend", &[]).await;
    assert_eq!(undone.status, 200);
    let mut third = Host::new(&fx.settings, ConnectionId(9));
    assert!(accepted(
        &third.answer(&login(100_001, "SWORDFISH")).outgoing
    ));
    assert_eq!(fx.audit_actions(), ["account.unsuspend", "account.suspend"]);
    let entry = &store::recent_audit(&fx.store, 10, None).unwrap()[1];
    assert_eq!(entry.reason.as_deref(), Some("spam"));
    assert_eq!(entry.target, "account 100001");
}

#[tokio::test]
async fn the_kick_button_ends_a_live_line_and_says_so() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let mut host = Host::new(&fx.settings, ConnectionId(7));
    host.label("test");
    let signal = host.kick_signal();
    assert!(accepted(
        &host.answer(&login(100_001, "SWORDFISH")).outgoing
    ));
    let plain = fx.post("/admin/kick", &[("connection", "7")]).await;
    assert!(plain.body.contains("needs a yes"), "{}", plain.body);
    let done = fx
        .post("/admin/kick", &[("connection", "7"), ("confirm", "yes")])
        .await;
    assert_eq!(done.status, 200);
    assert!(done.body.contains("was ended"), "{}", done.body);
    tokio::time::timeout(Duration::from_secs(2), signal.notified())
        .await
        .unwrap();
    assert_eq!(fx.audit_actions(), ["connection.kick"]);
    let gone = fx
        .post("/admin/kick", &[("connection", "99"), ("confirm", "yes")])
        .await;
    assert!(gone.body.contains("no longer online"), "{}", gone.body);
}

#[tokio::test]
async fn an_announcement_reaches_every_logged_in_line_and_no_other() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let mut online = Host::new(&fx.settings, ConnectionId(7));
    assert!(accepted(
        &online.answer(&login(100_001, "SWORDFISH")).outgoing
    ));
    let mut dialing = Host::new(&fx.settings, ConnectionId(8));
    let plain = fx.post("/admin/announce", &[("text", "Back soon")]).await;
    assert!(plain.body.contains("needs a yes"), "{}", plain.body);
    let empty = fx
        .post(
            "/admin/announce",
            &[("text", " \u{e9} "), ("confirm", "yes")],
        )
        .await;
    assert!(empty.body.contains("nothing to announce"), "{}", empty.body);
    let done = fx
        .post(
            "/admin/announce",
            &[("text", "Back in five\r\nminutes"), ("confirm", "yes")],
        )
        .await;
    assert_eq!(done.status, 200);
    assert!(
        done.body.contains("went to 1 logged-in player."),
        "{}",
        done.body
    );
    let told = tokio::time::timeout(Duration::from_secs(2), online.next_notice())
        .await
        .expect("the logged-in line was told");
    assert_eq!(
        told,
        Some(HostMessage::Notice(innkeeper_world::Notice {
            text: "Back in five minutes".into()
        }))
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), dialing.next_notice())
            .await
            .is_err(),
        "a line that has not logged in is not told"
    );
    assert_eq!(fx.audit_actions(), ["notice.send"]);
    let entry = &store::recent_audit(&fx.store, 1, None).unwrap()[0];
    assert_eq!(entry.reason.as_deref(), Some("Back in five minutes"));
    assert_eq!(entry.target, "1 players");
}

#[tokio::test]
async fn flags_money_and_password_are_refused_while_the_player_is_online() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let mut host = Host::new(&fx.settings, ConnectionId(7));
    assert!(accepted(
        &host.answer(&login(100_001, "SWORDFISH")).outgoing
    ));
    let before = fx.store.account(AccountId(100_001)).unwrap().unwrap();
    let flags = fx
        .post("/admin/accounts/100001/flags", &[("flag_4", "1")])
        .await;
    assert_eq!(flags.status, 409);
    assert!(flags.body.contains("is online now"), "{}", flags.body);
    let money = fx
        .post("/admin/accounts/100001/money", &[("amount", "9000")])
        .await;
    assert_eq!(money.status, 409);
    let password = fx
        .post("/admin/accounts/100001/password", &[("confirm", "yes")])
        .await;
    assert_eq!(password.status, 409);
    assert!(
        fx.store.money(AccountId(100_001)).unwrap().is_none()
            || fx.store.money(AccountId(100_001)).unwrap().unwrap().0 != 9000
    );
    assert_eq!(
        fx.store.account(AccountId(100_001)).unwrap().unwrap(),
        before
    );
    assert!(fx.audit_actions().is_empty());
    host.hang_up();
    // Once the line has hung up, the same edits go through.
    let flags = fx
        .post(
            "/admin/accounts/100001/flags",
            &[
                ("flag_512", "1"),
                ("flag_4", "1"),
                ("land_1", "1"),
                ("land_3", "1"),
            ],
        )
        .await;
    assert_eq!(flags.status, 200, "{}", flags.body);
    let after = fx.store.account(AccountId(100_001)).unwrap().unwrap();
    assert_eq!(after.user_flags, 0x0204);
    assert_eq!(after.rating, 0b101);
    let money = fx
        .post("/admin/accounts/100001/money", &[("amount", "9000")])
        .await;
    assert_eq!(money.status, 200);
    assert_eq!(fx.store.money(AccountId(100_001)).unwrap().unwrap().0, 9000);
    let bad = fx
        .post("/admin/accounts/100001/money", &[("amount", "-5")])
        .await;
    assert_eq!(bad.status, 409);
    assert_eq!(fx.audit_actions(), ["account.money", "account.flags"]);
}

#[tokio::test]
async fn a_reset_password_is_shown_once_and_works() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let ask = fx.post("/admin/accounts/100001/password", &[]).await;
    assert!(ask.body.contains("needs a yes"));
    let done = fx
        .post("/admin/accounts/100001/password", &[("confirm", "yes")])
        .await;
    assert_eq!(done.status, 200);
    let marker = "id=\"newpw\">";
    let start = done.body.find(marker).expect("the password is shown") + marker.len();
    let password = &done.body[start..start + 8];
    assert!(
        password
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()),
        "{password}"
    );
    assert_eq!(
        fx.store
            .account(AccountId(100_001))
            .unwrap()
            .unwrap()
            .password,
        keys::encode_password(password).unwrap()
    );
    let again = fx.get("/admin/accounts/100001").await.body;
    assert!(!again.contains(password), "never shown a second time");
    let log = store::recent_audit(&fx.store, 5, None).unwrap();
    assert!(log.iter().all(|entry| !entry.target.contains(password)
        && entry
            .reason
            .as_deref()
            .is_none_or(|r| !r.contains(password))));
    assert_eq!(done.headers["cache-control"], "no-store");
}

#[tokio::test]
async fn deleting_and_restoring_use_the_retention_paths() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let mailbox = fx.store.assign_mailbox(AccountId(100_001)).unwrap();
    fx.store
        .append_letter(
            mailbox,
            AccountId(100_001),
            &letter_body(mailbox.0, "Elaine", "Elaine", "n", "t"),
        )
        .unwrap();
    assert!(fx
        .post("/admin/accounts/100001/delete", &[])
        .await
        .body
        .contains("needs a yes"));
    let done = fx
        .post("/admin/accounts/100001/delete", &[("confirm", "yes")])
        .await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert!(
        done.body.contains("Forgotten (for 30 days)"),
        "{}",
        done.body
    );
    assert!(
        fx.store.account(AccountId(100_001)).unwrap().is_none(),
        "the player cannot log on"
    );
    assert!(
        fx.store.letters(mailbox).unwrap().is_empty(),
        "what it owned is hidden too"
    );
    let page = fx.get("/admin/accounts/100001").await.body;
    assert!(page.contains("Restore the account"), "{page}");
    let again = fx
        .post("/admin/accounts/100001/delete", &[("confirm", "yes")])
        .await;
    assert_eq!(again.status, 409);
    let restored = fx.post("/admin/accounts/100001/restore", &[]).await;
    assert_eq!(restored.status, 200);
    assert!(fx.store.account(AccountId(100_001)).unwrap().is_some());
    assert_eq!(fx.store.letters(mailbox).unwrap().len(), 1);
    assert_eq!(fx.audit_actions(), ["account.restore", "account.delete"]);
}

#[tokio::test]
async fn removing_a_post_hides_it_for_its_window_and_needs_a_yes() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let board = BoardId(1 << 24 | 66 << 8 | 1);
    let post = fx
        .store
        .post(board, AccountId(100_001), None, b"Elaine|spam spam spam")
        .unwrap();
    let path = format!("/admin/posts/{}/{}/remove", board.0, post.0);
    let ask = fx.post(&path, &[]).await;
    assert!(ask.body.contains("needs a yes"));
    assert_eq!(fx.store.posts(board).unwrap().len(), 1);
    let done = fx.post(&path, &[("confirm", "yes")]).await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert!(done.body.contains("Forgotten (for 90 days)"));
    assert!(fx.store.posts(board).unwrap().is_empty());
    let deleted: Option<i64> = fx
        .store
        .connection()
        .query_row(
            "SELECT deleted_at FROM posts WHERE id = ?1",
            [post.0],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        deleted.is_some(),
        "a soft delete: the row waits for the purge"
    );
    let entries = store::recent_audit(&fx.store, 5, None).unwrap();
    assert_eq!(entries[0].action, "post.remove");
    assert!(
        !entries[0].target.contains("spam"),
        "no post text in the log"
    );
    let wrong = fx
        .post(
            &format!("/admin/posts/999/{}/remove", post.0),
            &[("confirm", "yes")],
        )
        .await;
    assert_eq!(wrong.status, 404);
}

#[tokio::test]
async fn complaints_are_marked_beside_the_original_and_link_to_the_people_named() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    fx.account(100_002, "Bruno");
    fx.store.assign_mailbox(AccountId(100_002)).unwrap();
    let board = BoardId(99 << 8 | 3);
    let body = "Elaine|Complaint: From: Elaine in: Tavern; about: Bruno, who said \"rude\" Comments: very rude";
    let post = fx
        .store
        .post(
            board,
            AccountId(100_001),
            Some(AccountId(100_002)),
            body.as_bytes(),
        )
        .unwrap();
    let list = fx.get("/admin/complaints").await.body;
    assert!(
        list.contains("Elaine") && list.contains("waiting"),
        "{list}"
    );
    let detail = fx.get(&format!("/admin/complaints/{}", post.0)).await.body;
    assert!(
        detail.contains("/admin/accounts/100001") && detail.contains("/admin/accounts/100002"),
        "{detail}"
    );
    assert!(
        detail.contains("/admin/letters/1"),
        "the offender's mailbox is linked"
    );
    let counts = store::counts(&fx.store, 0).unwrap();
    assert_eq!(counts.complaints_waiting, 1);
    let done = fx
        .post(
            &format!("/admin/complaints/{}/handled", post.0),
            &[("handled", "1")],
        )
        .await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(store::counts(&fx.store, 0).unwrap().complaints_waiting, 0);
    let stored = fx.store.posts(board).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].body,
        body.as_bytes(),
        "the original is never edited"
    );
    let empty = fx.get("/admin/complaints").await.body;
    assert!(
        empty.contains("The in-tray is empty. The clerk is suspicious."),
        "{empty}"
    );
    let all = fx.get("/admin/complaints?all=1").await.body;
    assert!(all.contains("handled"));
    fx.post(
        &format!("/admin/complaints/{}/handled", post.0),
        &[("handled", "0")],
    )
    .await;
    assert_eq!(store::counts(&fx.store, 0).unwrap().complaints_waiting, 1);
    assert_eq!(
        fx.audit_actions(),
        ["complaint.reopened", "complaint.handled"]
    );
}

#[tokio::test]
async fn holds_are_placed_and_released_through_the_retention_code() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let missing = fx
        .post(
            "/admin/holds/place",
            &[("account", "999"), ("until", "2099-01-01"), ("reason", "x")],
        )
        .await;
    assert!(missing.body.contains("no account 999"));
    let past = fx
        .post(
            "/admin/holds/place",
            &[
                ("account", "100001"),
                ("until", "2001-01-01"),
                ("reason", "x"),
            ],
        )
        .await;
    assert!(past.body.contains("already passed"));
    let no_reason = fx
        .post(
            "/admin/holds/place",
            &[
                ("account", "100001"),
                ("until", "2099-01-01"),
                ("reason", " "),
            ],
        )
        .await;
    assert!(no_reason.body.contains("needs a reason"));
    assert!(store::holds(&fx.store).unwrap().is_empty());
    let done = fx
        .post(
            "/admin/holds/place",
            &[
                ("account", "100001"),
                ("until", "2099-01-01"),
                ("reason", "court order 17"),
            ],
        )
        .await;
    assert_eq!(done.status, 200);
    assert!(done.body.contains("through 2099-01-01"), "{}", done.body);
    assert!(done.body.contains("overrides the purge"));
    assert!(crate::retention::account_held(&fx.store, 100_001, crate::dates::now_ms()).unwrap());
    let ask = fx
        .post("/admin/holds/release", &[("account", "100001")])
        .await;
    assert!(ask.body.contains("needs a yes"));
    assert!(!store::holds(&fx.store).unwrap().is_empty());
    fx.post(
        "/admin/holds/release",
        &[("account", "100001"), ("confirm", "yes")],
    )
    .await;
    assert!(store::holds(&fx.store).unwrap().is_empty());
    assert_eq!(fx.audit_actions(), ["hold.release", "hold.place"]);
}

#[tokio::test]
async fn saves_are_listed_and_downloaded_not_edited() {
    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    fx.store
        .save_write(AccountId(100_001), "INN\\PLAYER.DIR", b"first", 1000, true)
        .unwrap();
    fx.store
        .save_write(AccountId(100_001), "INN\\PLAYER.DIR", b"second", 2000, true)
        .unwrap();
    let page = fx.get("/admin/saves/100001").await.body;
    assert!(
        page.contains("v2 (current)") && page.contains("v1"),
        "{page}"
    );
    let download = fx
        .post(
            "/admin/saves/100001/download",
            &[("path", "INN\\PLAYER.DIR"), ("version", "1")],
        )
        .await;
    assert_eq!(download.status, 200);
    assert_eq!(download.body, "first");
    assert!(download.headers["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("attachment; filename=\"account100001-v1-PLAYER.DIR\""));
    assert_eq!(download.headers["content-type"], "application/octet-stream");
    assert_eq!(fx.audit_actions(), ["save.download"]);
    for path in ["/admin/saves/100001/upload", "/admin/saves/100001/edit"] {
        assert_eq!(fx.post(path, &[]).await.status, 404, "{path}: no editing");
    }
    assert_eq!(
        fx.store
            .save_versions(AccountId(100_001), "INN\\PLAYER.DIR")
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn the_dashboard_counts_and_the_empty_states_speak_in_the_clerk_s_voice() {
    let fx = fixture(Door::Listed).await;
    let page = fx.get("/admin/").await.body;
    assert!(page.contains("Nobody is logged in"), "{page}");
    assert!(page.contains("The in-tray is empty. The clerk is suspicious."));
    assert!(page.contains("/healthz") && page.contains("/admin/healthz"));
    fx.account(100_001, "Elaine");
    fx.store
        .post(
            BoardId(1 << 24 | 66 << 8 | 1),
            AccountId(100_001),
            None,
            b"Elaine|hi",
        )
        .unwrap();
    let counts = store::counts(&fx.store, crate::dates::now_ms()).unwrap();
    assert_eq!((counts.accounts, counts.posts, counts.letters), (1, 1, 0));
    assert_eq!(fx.get("/admin/healthz").await.body, "ok");
    assert_eq!(fx.get("/admin/nowhere").await.status, 404);
    assert_eq!(fx.get("/admin/accounts/abc").await.status, 404);
    let search = fx.get("/admin/accounts?q=nobody").await.body;
    assert!(search.contains("No account matches that"));
    assert!(fx
        .get("/admin/accounts?q=lain")
        .await
        .body
        .contains("Elaine"));
    assert!(fx
        .get("/admin/accounts?q=100001")
        .await
        .body
        .contains("Elaine"));
    assert!(
        fx.get("/admin/accounts?q=%25")
            .await
            .body
            .contains("No account matches"),
        "a % is not a wildcard"
    );
}

#[tokio::test]
async fn the_help_and_audit_pages_render_and_the_menu_offers_day_and_dusk() {
    let fx = fixture(Door::Listed).await;
    let help = fx.get("/admin/help").await;
    assert_eq!(help.status, 200);
    assert!(
        help.body.contains("data-theme-set=\"light\"")
            && help.body.contains("data-theme-set=\"dark\"")
    );
    let audit = fx.get("/admin/audit").await.body;
    assert!(audit.contains("The audit log has nothing to show"));
    assert_eq!(fx.get("/admin/static/admin.js").await.status, 200);
    assert!(fx
        .get("/admin/static/vignette.js")
        .await
        .body
        .contains("AdminVignette"));
}

/// A value an editor form may send for a field that differs from what is stored.
fn another_value(field: &crate::ybarra::schema::Field, bytes: &[u8]) -> String {
    use crate::ybarra::schema::{Kind, Value};
    match (field.kind, field.read(bytes, 0)) {
        (Kind::Str, Value::Text(text)) => if text == "Bob" { "Al".into() } else { "Bob".into() },
        (_, Value::Raw(mut raw)) => {
            raw[0] ^= 1;
            Value::Raw(raw).form_text()
        }
        (_, Value::Number(number)) => {
            let (low, high) = field.bounds();
            if number == low { high.to_string() } else { low.to_string() }
        }
        (_, other) => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_character_editor_imports_tables_edits_a_slot_and_refuses_stale_or_online_edits() {
    use crate::ybarra::cipher::tests::sample;
    use crate::ybarra::extract::tests::fake_exe;
    use crate::ybarra::schema::schema;
    use crate::ybarra::{CharFile, Game, CHARS_FILE_BYTES};
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let fx = fixture(Door::Listed).await;
    fx.account(100_001, "Elaine");
    let path = Game::Yserbius.chars_path();
    let cipher = sample();
    let mut plain = CharFile::open(&cipher, &vec![0u8; CHARS_FILE_BYTES]).unwrap();
    plain.slots[0][8..14].copy_from_slice(b"Thorg\0");
    fx.store
        .save_write(AccountId(100_001), path, &plain.seal(&cipher), 1000, true)
        .unwrap();

    // Without the game's tables the editor says how to get them.
    let page = fx.get("/admin/saves/100001/ybarra/yserbius").await.body;
    assert!(page.contains("import the game"), "{page}");
    let saves = fx.get("/admin/saves/100001").await.body;
    assert!(saves.contains("/admin/saves/100001/ybarra/yserbius"), "{saves}");

    let refused = fx
        .post("/admin/ybarra/import", &[("program", &URL_SAFE_NO_PAD.encode(b"MZ junk"))])
        .await;
    assert!(refused.body.contains("Nothing was imported"), "{}", refused.body);
    let program = URL_SAFE_NO_PAD.encode(fake_exe(Game::Yserbius, &cipher));
    let imported = fx.post("/admin/ybarra/import", &[("program", &program)]).await;
    assert_eq!(imported.status, 200);
    assert!(imported.body.contains("saves can now be opened"), "{}", imported.body);
    let tables = fx.get("/admin/ybarra").await.body;
    assert!(tables.contains("ready"), "{tables}");

    let list = fx.get("/admin/saves/100001/ybarra/yserbius").await;
    assert_eq!(list.status, 200);
    assert!(list.body.contains("/admin/saves/100001/ybarra/yserbius/1"), "{}", list.body);
    let form = fx.get("/admin/saves/100001/ybarra/yserbius/1").await;
    assert_eq!(form.status, 200);
    assert!(form.body.contains("name=\"version\" value=\"1\""), "{}", form.body);

    // Change whichever field holds the name's first byte.
    let fields = &schema(Game::Yserbius).record;
    let index = fields
        .iter()
        .position(|field| field.mirror_of.is_none() && field.span().any(|at| at == 8))
        .unwrap();
    let name = format!("f{index}_0");
    let value = another_value(&fields[index], &plain.slots[0]);
    let mut expected = plain.slots[0];
    fields[index].write(&mut expected, 0, &value).unwrap();
    for mirror in fields.iter().filter(|f| f.mirror_of.as_ref() == Some(&fields[index].name)) {
        for (from, to) in fields[index].span().zip(mirror.span()) {
            expected[to] = expected[from];
        }
    }

    let stale = fx
        .post("/admin/saves/100001/ybarra/yserbius/1", &[("version", "0"), (&name, &value)])
        .await;
    assert_eq!(stale.status, 409, "{}", stale.body);
    assert!(stale.body.contains("changed since this form was opened"), "{}", stale.body);

    let mut host = Host::new(&fx.settings, ConnectionId(7));
    assert!(accepted(&host.answer(&login(100_001, "SWORDFISH")).outgoing));
    let online = fx
        .post("/admin/saves/100001/ybarra/yserbius/1", &[("version", "1"), (&name, &value)])
        .await;
    assert_eq!(online.status, 409);
    assert!(online.body.contains("is online now"), "{}", online.body);
    host.hang_up();
    assert_eq!(fx.store.save_versions(AccountId(100_001), path).unwrap().len(), 1);

    let saved = fx
        .post(
            "/admin/saves/100001/ybarra/yserbius/1",
            &[("version", "1"), (&name, &value), ("reason", "ticket 12")],
        )
        .await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert!(saved.body.contains("Saved as v2"), "{}", saved.body);
    let (meta, data) = fx.store.save_read(AccountId(100_001), path, None).unwrap().unwrap();
    assert_eq!(meta.version, 2);
    let after = CharFile::open(&cipher, &data).unwrap();
    assert_eq!(after.slots[0], expected);
    assert_eq!(after.slots[1..], plain.slots[1..]);
    assert_eq!(after.header, plain.header);

    // A value outside its field is refused and nothing is stored.
    let bad = fx
        .post("/admin/saves/100001/ybarra/yserbius/header", &[("version", "2"), ("f0_0", "zz")])
        .await;
    assert_eq!(bad.status, 422, "{}", bad.body);
    assert_eq!(fx.store.save_versions(AccountId(100_001), path).unwrap().len(), 2);
    assert_eq!(fx.audit_actions(), ["save.edit", "ybarra.import"]);
    let entry = &store::recent_audit(&fx.store, 1, None).unwrap()[0];
    assert!(entry.target.contains("v2 character slot 1"), "{}", entry.target);
    assert_eq!(entry.reason.as_deref(), Some("ticket 12"));
}

#[tokio::test]
async fn sign_ins_without_an_account_are_listed_with_why_and_never_their_subject() {
    use crate::signin::store as signin_store;
    let fx = fixture(Door::Listed).await;
    let ring = KeyRing::new(vec![HmacKey::from_base64("v1", &BASE64.encode([5; 32])).unwrap()])
        .unwrap();
    let now = crate::dates::now_ms();
    let sealed = || Some(b"SUBJECT-SEALED-BYTES".to_vec());
    let lone = signin_store::sign_in(&fx.store, &ring, "iss", "lone-subject", "dev", sealed, now)
        .unwrap()
        .key;
    let owner = signin_store::sign_in(&fx.store, &ring, "iss", "owner-subject", "dev", sealed, now)
        .unwrap()
        .key;
    let record = AccountRecord::new(keys::encode_password("SWORDFISH").unwrap(), "Elaine");
    signin_store::create_owned_account(&fx.store, &owner, &record, now).unwrap();

    let page = fx.get("/admin/signins").await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains(&hex(&lone)), "{}", page.body);
    assert!(!page.body.contains(&hex(&owner)), "{}", page.body);
    assert!(page.body.contains(&keys::handle(&lone)), "{}", page.body);
    assert!(page.body.contains("has not answered the age and terms question"), "{}", page.body);
    for secret in ["lone-subject", "SUBJECT-SEALED", "iss"] {
        assert!(!page.body.contains(secret), "{secret} leaked");
    }
    let dashboard = fx.get("/admin/").await.body;
    assert!(dashboard.contains("<b>1</b><span>sign-ins without an account"), "{dashboard}");

    let attested = signin_store::Attested { age: 21, terms: 1, at: now };
    signin_store::record_attestation(&fx.store, &lone, &attested).unwrap();
    let page = fx.get("/admin/signins").await.body;
    assert!(
        page.contains("21+, terms v1") && page.contains("Yes: nothing stops it"),
        "{page}"
    );
}
