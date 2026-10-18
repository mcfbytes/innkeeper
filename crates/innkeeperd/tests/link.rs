//! Linking sign-in methods end to end: a public innkeeperd and the mock provider, two subjects that
//! become one person, and the refusals for a forged, replayed or foreign link.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

mod common;

use common::*;

use std::sync::Arc;

use reqwest::Method;
use serde_json::{json, Value};
use tokio::net::TcpListener;

async fn ticket(base: &str, token: &str) -> String {
    let (status, answer) = call(base, Method::POST, "/auth/identities/ticket", token, None).await;
    assert_eq!(status, 200, "{answer}");
    answer["ticket"].as_str().unwrap().to_owned()
}

/// A link login for `subject` on `ticket`: the page address the callback answered with.
async fn link_login(base: &str, subject: &str, ticket: &str) -> String {
    redirect_login(base, subject, &format!("&link={ticket}"))
        .await
        .1
}

/// The first hop of a login whose ticket is refused before the provider: the page address.
async fn refused_login(base: &str, ticket: &str) -> String {
    let login = format!(
        "{base}/auth/login?provider=dev&mode=redirect&nonce={NONCE}&return={}&link={ticket}",
        url::form_urlencoded::byte_serialize(RETURN.as_bytes()).collect::<String>()
    );
    let answer = http().get(&login).send().await.unwrap();
    assert_eq!(answer.status(), 302);
    location(&answer)
}

async fn redeem(base: &str, token: &str, proof: &str) -> (u16, Value) {
    let body = Some(json!({ "proof": proof }));
    call(base, Method::POST, "/auth/identities", token, body).await
}

#[tokio::test(flavor = "multi_thread")]
async fn two_providers_reach_the_same_accounts_and_nothing_else_links() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let key = rsa::RsaPrivateKey::new(&mut aes_gcm::aead::OsRng, 2048).unwrap();
    let provider = Arc::new(dev_oidc::MockProvider::new(issuer.clone(), key));
    tokio::spawn(Arc::clone(&provider).serve(listener));
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("inn-link-e2e-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let base = format!("http://127.0.0.1:{port}");
    std::fs::write(
        dir.join("auth.toml"),
        format!(
            "public_url = \"{base}\"\n\n[[provider]]\nid = \"dev\"\nname = \"Dev\"\n\
             issuer = \"{issuer}\"\nclient_id = \"inn-test\"\nclient_secret_name = \"dev-secret\"\n\n\
             [dev_secrets]\ndev-secret = \"not-a-real-secret\"\n"
        ),
    )
    .unwrap();
    let server = tokio::task::block_in_place(|| start(&dir, port, &[]));

    // 1. Alice signs in with her first subject and opens an account.
    let alice = sign_in(&base, "alice-first").await;
    let (status, made) = call(
        &base,
        Method::POST,
        "/auth/accounts",
        &alice,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 201, "{made}");
    let account = made["id"].as_u64().unwrap();

    // 2. She links a second subject: the login answers with a proof, not a session.
    let first = ticket(&base, &alice).await;
    let page = link_login(&base, "alice-second", &first).await;
    assert_eq!(fragment_value(&page, "inn-nonce").as_deref(), Some(NONCE));
    assert!(fragment_value(&page, "inn-session").is_none(), "{page}");
    let proof = fragment_value(&page, "inn-link").unwrap_or_else(|| panic!("{page}"));
    let (status, linked) = redeem(&base, &alice, &proof).await;
    assert_eq!(status, 200, "{linked}");
    assert_eq!(linked["identities"].as_array().unwrap().len(), 2);
    let (status, again) = redeem(&base, &alice, &proof).await;
    assert_eq!(
        (status, again["error"].as_str()),
        (400, Some("bad_proof")),
        "replayed"
    );

    // 3. The second subject, signed in on its own, sees the same account.
    let second = sign_in(&base, "alice-second").await;
    let (_, session) = call(&base, Method::GET, "/auth/session", &second, None).await;
    assert_eq!(session["accounts"][0]["id"], account, "{session}");
    let methods = session["identities"].as_array().unwrap();
    assert_eq!(methods.len(), 2);
    let mine = methods.iter().find(|m| m["current"] == true).unwrap();
    let theirs = methods.iter().find(|m| m["current"] == false).unwrap();
    assert_eq!(mine["provider"], "dev");
    assert!(mine["linkedOn"].as_str().unwrap().len() == 10, "{mine}");
    assert!(mine.get("subject").is_none() && !session.to_string().contains("alice-second"));

    // 4. A forged ticket, a used one and one for another session's proof are refused.
    let forged = refused_login(&base, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").await;
    assert_eq!(
        fragment_value(&forged, "inn-error").as_deref(),
        Some("link_expired")
    );
    let used = refused_login(&base, &first).await;
    assert_eq!(
        fragment_value(&used, "inn-error").as_deref(),
        Some("link_expired")
    );
    let bob = sign_in(&base, "bob").await;
    let page = link_login(&base, "carol", &ticket(&base, &alice).await).await;
    let foreign = fragment_value(&page, "inn-link").unwrap();
    let (status, answer) = redeem(&base, &bob, &foreign).await;
    assert_eq!((status, answer["error"].as_str()), (400, Some("bad_proof")));

    // 5. An identity that already owns accounts is never merged in.
    let (status, _) = call(&base, Method::POST, "/auth/accounts", &bob, Some(json!({}))).await;
    assert_eq!(status, 201);
    let page = link_login(&base, "bob", &ticket(&base, &alice).await).await;
    assert_eq!(
        fragment_value(&page, "inn-error").as_deref(),
        Some("identity_in_use")
    );
    assert_eq!(fragment_value(&page, "inn-nonce").as_deref(), Some(NONCE));

    // 6. The last method and the one in use cannot be removed; another one can.
    let unlink = |token: String, id: Value| {
        let base = base.clone();
        async move {
            let body = Some(json!({ "id": id }));
            call(&base, Method::POST, "/auth/identities/unlink", &token, body).await
        }
    };
    let (_, bobs) = call(&base, Method::GET, "/auth/session", &bob, None).await;
    let (status, answer) = unlink(bob.clone(), bobs["identities"][0]["id"].clone()).await;
    assert_eq!(
        (status, answer["error"].as_str()),
        (409, Some("last_identity"))
    );
    let (status, answer) = unlink(second.clone(), mine["id"].clone()).await;
    assert_eq!(
        (status, answer["error"].as_str()),
        (409, Some("current_identity"))
    );
    let (status, answer) = unlink(second.clone(), theirs["id"].clone()).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["identities"].as_array().unwrap().len(), 1);
    let (status, _) = call(&base, Method::GET, "/auth/session", &alice, None).await;
    assert_eq!(status, 401, "the removed method's session ended");
    let back = sign_in(&base, "alice-first").await;
    let (_, alone) = call(&base, Method::GET, "/auth/session", &back, None).await;
    assert_eq!(alone["accounts"], json!([]), "{alone}");

    let log = server.log();
    for secret in [
        "alice-first",
        "alice-second",
        "carol",
        proof.as_str(),
        alice.as_str(),
    ] {
        assert!(!log.contains(secret), "the log holds {secret}\n{log}");
    }
    assert!(log.contains("a sign-in method was linked"), "{log}");
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
