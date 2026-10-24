//! A stop warns the logged-in players: the first SIGTERM sends a notice (command 48) and waits for
//! `--stop-grace`, a second one ends the wait, and a server with nobody online stops at once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

mod common;

use common::*;

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tsn_link::LinkOutput;

const NOTICE: u8 = 48;

async fn provider() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let key = rsa::RsaPrivateKey::new(&mut aes_gcm::aead::OsRng, 2048).unwrap();
    let provider = Arc::new(dev_oidc::MockProvider::new(issuer.clone(), key));
    tokio::spawn(Arc::clone(&provider).serve(listener));
    issuer
}

fn server(name: &str, issuer: &str, grace: &str) -> (Server, u16, String) {
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("inn-stop-{name}-{}", std::process::id()));
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
    let extra = ["--stop-grace".to_owned(), grace.to_owned()];
    let server = tokio::task::block_in_place(|| start(&dir, port, &extra));
    (server, port, base)
}

async fn logged_in_line(base: &str, port: u16) -> Line {
    let token = sign_in(base, "alice").await;
    let persona = Some(json!({"persona": "Elaine"}));
    let (status, created) = call(
        base,
        reqwest::Method::POST,
        "/auth/accounts",
        &token,
        persona,
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_u64().unwrap() as u32;
    let path = format!("/auth/accounts/{id}/credential");
    let (_, credential) = call(base, reqwest::Method::POST, &path, &token, None).await;
    let mut line = Line::open(port, Some(&token)).await.expect("upgraded");
    line.call_host().await;
    let reply = line
        .login(id, credential["password"].as_str().unwrap())
        .await;
    assert_eq!(reply[0], ACK);
    line
}

/// The next notice's text, acknowledging frames as the client would.
async fn next_notice(line: &mut Line, within: Duration) -> Option<String> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        while let Some(output) = line.link.poll_output() {
            match output {
                LinkOutput::ToClient(bytes) => {
                    use futures_util::SinkExt;
                    line.socket.send(Message::binary(bytes)).await.unwrap();
                }
                LinkOutput::Delivered(message) if message.body().first() == Some(&NOTICE) => {
                    let body = message.body();
                    let text = body.get(2..).unwrap_or_default();
                    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
                    return Some(String::from_utf8_lossy(&text[..end]).into_owned());
                }
                _ => {}
            }
        }
        use futures_util::StreamExt;
        let next = tokio::time::timeout_at(deadline, line.socket.next()).await;
        match next {
            Ok(Some(Ok(Message::Binary(bytes)))) => {
                let _escaped = line
                    .link
                    .handle_input(&bytes, tokio::time::Instant::now().into_std());
            }
            Ok(Some(Ok(_))) => {}
            _ => return None,
        }
    }
}

fn term(server: &Server) {
    let status = Command::new("kill")
        .args(["-TERM", &server.child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

fn exited_within(server: &mut Server, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if server.child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_warns_the_players_and_a_second_stop_ends_the_wait() {
    let issuer = provider().await;
    let (mut server, port, base) = server("warn", &issuer, "30s");
    let mut line = logged_in_line(&base, port).await;

    term(&server);
    let warning = next_notice(&mut line, Duration::from_secs(5))
        .await
        .expect("a warning");
    assert!(
        warning.contains("restart for maintenance in 30 seconds"),
        "{warning}"
    );
    assert!(
        !tokio::task::block_in_place(|| exited_within(&mut server, Duration::from_secs(1))),
        "the server waits while a player is online"
    );

    term(&server);
    let last = next_notice(&mut line, Duration::from_secs(5))
        .await
        .expect("the last word");
    assert!(last.contains("restarting now"), "{last}");
    assert!(
        tokio::task::block_in_place(|| exited_within(&mut server, Duration::from_secs(10))),
        "a second stop ends the wait\n{}",
        server.log()
    );
    let log = server.log();
    assert!(log.contains("warning the players first"), "{log}");
    assert!(log.contains("a second stop"), "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn with_nobody_online_a_stop_is_immediate() {
    let issuer = provider().await;
    let (mut server, _, _) = server("empty", &issuer, "5m");
    term(&server);
    assert!(
        tokio::task::block_in_place(|| exited_within(&mut server, Duration::from_secs(5))),
        "{}",
        server.log()
    );
    assert!(server.log().contains("stopping now"), "{}", server.log());
}
