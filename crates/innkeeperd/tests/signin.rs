//! Sign-in end to end: a public innkeeperd, a mock OpenID Connect provider in this test, the page
//! contract over HTTP and a WebSocket Login with the issued credential.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

mod common;

use common::*;

use std::sync::Arc;

use futures_util::SinkExt;
use serde_json::{json, Value};
use tokio::net::TcpListener;

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
    let server = tokio::task::block_in_place(|| start(&dir, port, &[]));

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

    // 8b. A save put on the logged-on line is what the page fetches over HTTP before it dials.
    let chars = "INN\\YSERBIUS\\MYCHARS.DAT";
    let put =
        json!({"op": "put", "path": chars, "base": 0, "mtime": 5, "id": "p1", "data": "aGVybw=="});
    line.socket
        .send(tokio_tungstenite::tungstenite::Message::text(
            put.to_string(),
        ))
        .await
        .unwrap();
    loop {
        if let tokio_tungstenite::tungstenite::Message::Text(text) = line.next().await {
            let reply: Value = serde_json::from_str(text.as_str()).unwrap();
            if reply["id"] == "p1" {
                assert_eq!(reply["op"], "stored", "{reply}");
                break;
            }
        }
    }
    let saves = format!("/auth/accounts/{id}/saves");
    let (status, listed) = call(&base, reqwest::Method::GET, &saves, &token, None).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["files"][0]["path"], chars);
    assert_eq!(listed["files"][0]["data"], "aGVybw==");
    let (status, _) = call(&base, reqwest::Method::GET, &saves, &bob, None).await;
    assert_eq!(status, 404, "another identity's account has no saves route");
    let (status, _) = call(&base, reqwest::Method::GET, &saves, "bad-token", None).await;
    assert_eq!(status, 401);

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
