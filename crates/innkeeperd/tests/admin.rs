//! The operator portal end to end: a public, jailed innkeeperd with the portal on its own loopback
//! listener, the mock provider, a staff allowlist file, and a real player on a WebSocket line.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

mod common;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures_util::StreamExt;
use serde_json::json;
use tokio::net::TcpListener;

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text
        .find(start)
        .unwrap_or_else(|| panic!("no {start:?} in\n{text}"))
        + start.len();
    let to = text[from..].find(end).unwrap() + from;
    &text[from..to]
}

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

struct Portal {
    admin: String,
    issuer: String,
}

/// The mock provider's consent and the portal's callback, as one browser would do them.
async fn admin_sign_in(portal: &Portal, subject: &str) -> reqwest::Response {
    let client = http();
    let start = format!("{}/admin/auth/login?provider=dev", portal.admin);
    let first = client.get(&start).send().await.unwrap();
    assert_eq!(first.status(), 302, "login");
    let there = location(&first);
    assert!(there.starts_with(&portal.issuer), "{there}");
    let allow = format!("{there}&decision=allow&subject={subject}");
    let back = client.get(&allow).send().await.unwrap();
    assert_eq!(back.status(), 302, "the mock redirects to the callback");
    let callback = location(&back);
    assert!(
        callback.starts_with(&format!("{}/admin/auth/callback?", portal.admin)),
        "the portal has a callback of its own: {callback}"
    );
    client.get(&callback).send().await.unwrap()
}

fn cookie_of(response: &reqwest::Response) -> String {
    let set = response.headers()["set-cookie"].to_str().unwrap();
    assert!(
        set.contains("HttpOnly") && set.contains("SameSite=Strict"),
        "{set}"
    );
    set.split(';').next().unwrap().to_owned()
}

struct Session<'a> {
    portal: &'a Portal,
    cookie: String,
}

impl Session<'_> {
    async fn get(&self, path: &str) -> (u16, String) {
        let response = http()
            .get(format!("{}{path}", self.portal.admin))
            .header("cookie", &self.cookie)
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }

    async fn post(&self, path: &str, csrf: &str, fields: &[(&str, &str)]) -> (u16, String) {
        let mut form = vec![("csrf", csrf)];
        form.extend_from_slice(fields);
        let response = http()
            .post(format!("{}{path}", self.portal.admin))
            .header("cookie", &self.cookie)
            .header("origin", &self.portal.admin)
            .form(&form)
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }
}

/// A second connection to the server's database, foreign keys on like every connection the
/// server makes.
fn sqlite(dir: &Path) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(dir.join("data/innkeeper.db")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db
}

fn cli(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_innkeeperd"))
        .args(args)
        .args(["--data-dir", &dir.join("data").display().to_string()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn staff_sign_in_through_the_allowlist_and_run_the_office() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let key = rsa::RsaPrivateKey::new(&mut aes_gcm::aead::OsRng, 2048).unwrap();
    let provider = Arc::new(dev_oidc::MockProvider::new(issuer.clone(), key));
    tokio::spawn(Arc::clone(&provider).serve(listener));

    let port = free_port();
    let admin_port = free_port();
    let dir = std::env::temp_dir().join(format!("inn-admin-e2e-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::create_dir_all(dir.join("staff")).unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let admin = format!("http://127.0.0.1:{admin_port}");
    std::fs::write(
        dir.join("auth.toml"),
        format!(
            "public_url = \"{base}\"\nadmin_public_url = \"{admin}\"\n\n[[provider]]\nid = \"dev\"\n\
             name = \"Dev Phone Co.\"\nissuer = \"{issuer}\"\nclient_id = \"inn-test\"\n\
             client_secret_name = \"dev-secret\"\n\n[dev_secrets]\ndev-secret = \"not-a-real-secret\"\n"
        ),
    )
    .unwrap();
    let list = dir.join("staff/staff.txt");
    std::fs::write(&list, "# nobody yet\n").unwrap();
    let extra = [
        "--admin-bind".to_owned(),
        format!("127.0.0.1:{admin_port}"),
        "--admin-allowlist".to_owned(),
        list.display().to_string(),
    ];
    let server = tokio::task::block_in_place(|| start(&dir, port, &extra));
    let portal = Portal {
        admin: admin.clone(),
        issuer,
    };

    // A player is online, and some data exists to look at.
    let token = sign_in(&base, "elaine").await;
    let persona = Some(json!({"persona": "Elaine"}));
    let (_, created) = call(
        &base,
        reqwest::Method::POST,
        "/auth/accounts",
        &token,
        persona,
    )
    .await;
    let id = created["id"].as_u64().unwrap() as u32;
    let path = format!("/auth/accounts/{id}/credential");
    let (_, credential) = call(&base, reqwest::Method::POST, &path, &token, None).await;
    let password = credential["password"].as_str().unwrap().to_owned();
    let mut line = Line::open(port, Some(&token)).await.expect("upgraded");
    line.call_host().await;
    let reply = line.login(id, &password).await;
    assert_eq!(reply[0], ACK, "{}", server.log());
    {
        let db = sqlite(&dir);
        db.execute(
            "INSERT INTO accounts
             (id, password, persona, user_flags, rating, status, password_expired, created_at)
             VALUES (100777, x'01010101010101010101', 'Bruno', 512, 0, 0, 0, 1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO mailboxes (account, created_at) VALUES (100777, 1)",
            [],
        )
        .unwrap();
        let mailbox: u32 = db
            .query_row(
                "SELECT number FROM mailboxes WHERE account = 100777",
                [],
                |row| row.get(0),
            )
            .unwrap();
        db.execute(
            "INSERT INTO letters (to_mailbox, from_account, body, created_at) VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![
                mailbox,
                id,
                letter_body(7, "Elaine", "Bruno", "pier", "meet me at the pier")
            ],
        )
        .unwrap();
        let post = |board: u32, about: Option<u32>, text: &str| {
            db.execute(
                "INSERT INTO posts (board, account, about_account, body, created_at)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                rusqlite::params![board, id, about, text.as_bytes()],
            )
            .unwrap();
        };
        post(16_794_113, None, "Elaine|buy my stuff");
        post(
            25_347,
            Some(100_777),
            "Elaine|Complaint: From: Elaine in: Tavern; about: Bruno, who said \"x\" Comments: rude",
        );
    }

    // 1. A stranger is turned away politely, and shown only their own key.
    let refused = admin_sign_in(&portal, "staffer").await;
    assert_eq!(refused.status(), 403);
    assert!(
        refused.headers().get("set-cookie").is_none(),
        "no session for a stranger"
    );
    let page = refused.text().await.unwrap();
    let hex = between(&page, "<code id=\"mykey\">", "</code>").to_owned();
    assert_eq!(hex.len(), 64, "{hex}");
    assert!(hex.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(
        page.contains("data-copy=\"mykey\"") && page.contains("admin-identities"),
        "{page}"
    );
    assert!(
        !page.contains("staffer"),
        "the provider's subject is never shown"
    );
    let direct = http().get(format!("{admin}/admin/")).send().await.unwrap();
    assert_eq!(direct.status(), 303, "no cookie, no office");

    // 2. Put the key in the file; the next sign-in is let through, with no restart.
    std::fs::write(&list, format!("# the operator\n{hex}\n")).unwrap();
    let entered = admin_sign_in(&portal, "staffer").await;
    assert_eq!(entered.status(), 200);
    let cookie = cookie_of(&entered);
    assert!(
        cookie.starts_with("inn-admin="),
        "plain http on loopback: {cookie}"
    );
    let session = Session {
        portal: &portal,
        cookie: cookie.clone(),
    };
    let (status, dash) = session.get("/admin/").await;
    assert_eq!(status, 200);
    assert!(
        dash.contains("Who is in (1)") && dash.contains("guybrush"),
        "{dash}"
    );
    assert!(!dash.contains("DEV: open door"));
    let csrf = between(&dash, "name=\"csrf\" value=\"", "\"").to_owned();
    let connection = between(&dash, "name=\"connection\" value=\"", "\"").to_owned();

    // 3. Every page opens.
    for path in [
        "/admin/accounts",
        "/admin/accounts?q=Elaine",
        &format!("/admin/accounts/{id}"),
        "/admin/accounts/100777",
        "/admin/complaints",
        "/admin/posts",
        "/admin/letters",
        "/admin/letters/1",
        "/admin/holds",
        "/admin/saves",
        "/admin/audit",
        "/admin/help",
    ] {
        let (status, _) = session.get(path).await;
        assert_eq!(status, 200, "{path}\n{}", server.log());
    }

    // 4. Kick the online player: their line closes.
    let (status, done) = session
        .post(
            "/admin/kick",
            &csrf,
            &[("connection", &connection), ("confirm", "yes")],
        )
        .await;
    assert_eq!(status, 200, "{done}");
    assert!(done.contains("was ended"), "{done}");
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match line.socket.next().await {
                None
                | Some(Err(_))
                | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "the kicked line was closed");

    // 5. Suspend: a new line cannot log in. Unsuspend: it can.
    let (status, body) = session
        .post(
            &format!("/admin/accounts/{id}/suspend"),
            &csrf,
            &[("confirm", "yes"), ("reason", "test")],
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut second = Line::open(port, Some(&token)).await.expect("upgraded");
    second.call_host().await;
    assert_eq!(
        second.login(id, &password).await[0],
        NAK,
        "suspended accounts are refused"
    );
    session
        .post(&format!("/admin/accounts/{id}/unsuspend"), &csrf, &[])
        .await;
    let mut third = Line::open(port, Some(&token)).await.expect("upgraded");
    third.call_host().await;
    assert_eq!(
        third.login(id, &password).await[0],
        ACK,
        "and welcomed back"
    );
    drop((second, third));

    // 6. Remove a post and mark a complaint handled.
    let post_id: u32 = sqlite(&dir)
        .query_row("SELECT id FROM posts WHERE board = 16794113", [], |row| {
            row.get(0)
        })
        .unwrap();
    let (status, body) = session
        .post(
            &format!("/admin/posts/16794113/{post_id}/remove"),
            &csrf,
            &[("confirm", "yes")],
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let removed: Option<i64> = sqlite(&dir)
        .query_row(
            "SELECT deleted_at FROM posts WHERE id = ?1",
            [post_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(removed.is_some());
    let (_, complaints) = session.get("/admin/complaints").await;
    assert!(
        complaints.contains("Elaine") && complaints.contains("waiting"),
        "{complaints}"
    );

    // 7. Read a letter: the reason is required and the audit row exists.
    let (status, _) = session
        .post("/admin/letters/1/1/read", &csrf, &[("reason", "")])
        .await;
    assert_eq!(status, 400);
    let (status, letter) = session
        .post(
            "/admin/letters/1/1/read",
            &csrf,
            &[("reason", "Member Services case 4411")],
        )
        .await;
    assert_eq!(status, 200);
    assert!(letter.contains("meet me at the pier"), "{letter}");
    let (reason, action, key): (String, String, Vec<u8>) = sqlite(&dir)
        .query_row(
            "SELECT reason, action, identity_key FROM admin_audit WHERE action = 'letter.read'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (reason.as_str(), action.as_str()),
        ("Member Services case 4411", "letter.read")
    );
    let key_hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(key_hex, hex, "the operator's identity key");

    // 8. Revoke by emptying the file: the very next request is refused.
    std::fs::write(&list, "# the operator has left the building\n").unwrap();
    let (status, _) = session.get("/admin/").await;
    assert_eq!(status, 303, "the session ended with the permission");
    // A revocation in the staff table beats the file, and an allow lifts it. Both work while the
    // server runs.
    std::fs::write(&list, format!("{hex}\n")).unwrap();
    let again = admin_sign_in(&portal, "staffer").await;
    assert_eq!(again.status(), 200);
    let session = Session {
        portal: &portal,
        cookie: cookie_of(&again),
    };
    assert_eq!(session.get("/admin/").await.0, 200);
    let said = cli(&dir, &["admin", "revoke", &hex]);
    assert!(said.contains("shut out"), "{said}");
    assert_eq!(
        session.get("/admin/").await.0,
        303,
        "the file lists the key; the revocation wins"
    );
    assert_eq!(admin_sign_in(&portal, "staffer").await.status(), 403);
    cli(&dir, &["admin", "allow", &hex, "--note", "the operator"]);
    let listed = cli(&dir, &["admin", "list"]);
    assert!(
        listed.contains(&hex) && listed.contains("allowed") && listed.contains("the operator"),
        "{listed}"
    );
    assert_eq!(admin_sign_in(&portal, "staffer").await.status(), 200);

    // 9. The log holds no token, key, letter, post or reason.
    let log = server.log();
    for secret in [
        token.as_str(),
        cookie.as_str(),
        hex.as_str(),
        "meet me at the pier",
        "buy my stuff",
        "Member Services case 4411",
        "staffer",
        "not-a-real-secret",
    ] {
        assert!(!log.contains(secret), "the log holds {secret}\n{log}");
    }
    assert!(
        log.contains("portal sign-in refused: not on the staff list"),
        "{log}"
    );
    let mut server = server;
    assert!(
        server.child.try_wait().unwrap().is_none(),
        "still running\n{log}"
    );
}

#[test]
fn a_public_server_refuses_to_start_with_an_open_portal() {
    let dir = std::env::temp_dir().join(format!("inn-admin-open-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let port = free_port();
    let admin_port = free_port();
    std::fs::write(
        dir.join("auth.toml"),
        "[[provider]]\nid = \"dev\"\nname = \"Dev\"\nissuer = \"http://127.0.0.1:1\"\nclient_id = \"c\"\n\
         client_secret_name = \"dev-secret\"\n\n[dev_secrets]\ndev-secret = \"x\"\n",
    )
    .unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_innkeeperd"))
            .args([
                "--public",
                "--insecure-dev-keys",
                "--no-sandbox",
                "--ws-bind",
                &format!("127.0.0.1:{port}"),
                "--data-dir",
                &dir.join("data").display().to_string(),
                "--auth-config",
                &dir.join("auth.toml").display().to_string(),
                "--page-origin",
                "http://127.0.0.1:2901",
                "--admin-bind",
                &format!("127.0.0.1:{admin_port}"),
            ])
            .args(extra)
            .output()
            .unwrap()
    };
    let open = run(&[]);
    assert!(!open.status.success());
    let err = String::from_utf8_lossy(&open.stderr);
    assert!(err.contains("refuses an open admin portal"), "{err}");
    let forced = run(&["--admin-dev-open"]);
    assert!(!forced.status.success());
    let err = String::from_utf8_lossy(&forced.stderr);
    assert!(
        err.contains("--admin-dev-open") && err.contains("--public"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
