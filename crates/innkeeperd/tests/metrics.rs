//! A jailed innkeeperd (seccomp kill mode) pushes metrics to a mock ingestion endpoint and talks
//! the systemd notify protocol. Skips when the kernel has no Landlock or seccomp.
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    for _ in 0..32 {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        if !matches!(port, 2314 | 2316 | 2317 | 2318 | 8080 | 8081) {
            return port;
        }
    }
    panic!("no usable port");
}

/// Answers the managed-identity token request and the track POST; keeps the track bodies.
fn mock(listener: TcpListener, tracks: Arc<Mutex<Vec<String>>>, stop: Arc<AtomicBool>) {
    listener.set_nonblocking(true).unwrap();
    while !stop.load(Ordering::Relaxed) {
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let tracks = Arc::clone(&tracks);
        std::thread::spawn(move || answer(&mut stream, &tracks));
    }
}

fn answer(stream: &mut TcpStream, tracks: &Mutex<Vec<String>>) {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let body = loop {
        let Ok(n) = stream.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            return;
        }
        data.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&data).into_owned();
        let Some(split) = text.find("\r\n\r\n") else {
            continue;
        };
        let length = text[..split]
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if data.len() >= split + 4 + length {
            break (text[..split].to_owned(), text[split + 4..].to_owned());
        }
    };
    let reply = if body.0.starts_with("GET /metadata/identity/oauth2/token") {
        r#"{"access_token":"t","expires_in":"3599"}"#
    } else {
        tracks.lock().unwrap().push(body.1);
        r#"{"itemsReceived":1,"itemsAccepted":1,"errors":[]}"#
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
        reply.len()
    );
}

#[test]
fn a_jailed_daemon_pushes_metrics_and_notifies_systemd() {
    let dir = std::env::temp_dir().join(format!("inn-metrics-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let notify_path: PathBuf = dir.join("notify");
    let notify = UnixDatagram::bind(&notify_path).unwrap();
    notify
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mock_url = format!("http://{}", listener.local_addr().unwrap());
    let tracks = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let mock_thread = {
        let (tracks, stop) = (Arc::clone(&tracks), Arc::clone(&stop));
        std::thread::spawn(move || mock(listener, tracks, stop))
    };

    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_innkeeperd"))
        .args([
            "--sandbox",
            "--no-int14h",
            "--no-capture",
            "--data-dir",
            &dir.join("data").display().to_string(),
            "--bind",
            &format!("127.0.0.1:{port}"),
            "--app-insights-connection-string",
            "InstrumentationKey=0123abcd-0123-4567-89ab-0123456789ab;IngestionEndpoint=https://x.in.applicationinsights.azure.com/",
            "--test-ingestion-endpoint",
            &mock_url,
            "--test-imds-endpoint",
            &mock_url,
            "--test-metrics-interval",
            "600ms",
        ])
        .env("NOTIFY_SOCKET", &notify_path)
        .env("WATCHDOG_USEC", "400000")
        .env_remove("WATCHDOG_PID")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let log = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&log);
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });

    let mut states = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        if let Ok(n) = notify.recv(&mut buf) {
            states.push(String::from_utf8_lossy(&buf[..n]).into_owned());
        }
        let watchdog = states.iter().filter(|s| *s == "WATCHDOG=1").count();
        if watchdog >= 2 && !tracks.lock().unwrap().is_empty() {
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
    }
    let alive = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    stop.store(true, Ordering::Relaxed);
    mock_thread.join().unwrap();
    let text = log.lock().unwrap().clone();
    let _ = std::fs::remove_dir_all(&dir);

    if text.contains("this kernel has no Landlock") || text.contains("seccomp:") {
        eprintln!("skipping: no sandbox here\n{text}");
        return;
    }
    assert!(text.contains("seccomp=kill"), "{text}");
    assert!(alive, "the daemon died under the sandbox\n{text}");
    assert_eq!(
        states.first().map(String::as_str),
        Some("READY=1"),
        "{states:?}\n{text}"
    );
    assert!(
        states.iter().filter(|s| *s == "WATCHDOG=1").count() >= 2,
        "{states:?}\n{text}"
    );
    let tracks = tracks.lock().unwrap();
    assert!(!tracks.is_empty(), "no push reached the mock\n{text}");
    assert!(tracks[0].contains("\"heartbeat\""), "{}", tracks[0]);
    assert!(!text.contains("push failed"), "{text}");
}
