//! A sandboxed innkeeperd accepts a scripted Login. Skips when the kernel has no Landlock or seccomp.
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers may abort

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOGIN_FRAME: [u8; 39] = [
    0x81, 0x58, 0xd8, 0x00, 0x21, 0x35, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x12, 0xa1, 0x86, 0x01,
    0x00, 0x01, 0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00, 0x00, 0x67, 0x75, 0x79,
    0x62, 0x72, 0x75, 0x73, 0x68, 0x00, 0x82,
];
const LINK_ACK: [u8; 5] = [0x81, 0x49, 0x62, 0x90, 0x82];

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_innkeeperd")
}

fn free_port() -> u16 {
    for _ in 0..32 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a probe port");
        let port = listener.local_addr().expect("probe port").port();
        drop(listener);
        if !matches!(port, 2314 | 2316 | 8080 | 8081) {
            return port;
        }
    }
    panic!("no usable high port");
}

fn temp_dir(label: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("inn-sb-{label}-{}-{n}", std::process::id()))
}

struct Spawned {
    child: Child,
    log: Arc<Mutex<String>>,
}

impl Drop for Spawned {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(args: &[String]) -> Spawned {
    let mut child = Command::new(bin())
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn innkeeperd");
    let stderr = child.stderr.take().expect("stderr");
    let log = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&log);
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut buf = [0u8; 1024];
        loop {
            match stderr.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink
                    .lock()
                    .expect("log")
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
    Spawned { child, log }
}

fn log_text(spawned: &Spawned) -> String {
    spawned.log.lock().expect("log").clone()
}

fn skip(reason: &str) {
    eprintln!("skipping sandbox integration test: {reason}");
}

/// Landlock missing is a warning and the server still runs. Seccomp missing exits.
fn unavailable(text: &str, exited: bool) -> Option<&'static str> {
    if text.contains("this kernel has no Landlock") || text.contains("fs=open") {
        return Some("landlock is not available");
    }
    // The summary line says `seccomp=kill`. The install failure says `seccomp:`.
    if exited && text.contains("seccomp:") {
        return Some("seccomp is not available");
    }
    None
}

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn a_sandboxed_server_answers_a_scripted_login() {
    let port = free_port();
    let data = Temp(temp_dir("login"));
    std::fs::create_dir_all(&data.0).expect("data dir");
    let args = [
        "--sandbox",
        "--line",
        "pad",
        "--no-int14h",
        "--no-capture",
        "--data-dir",
        &data.0.display().to_string(),
        "--bind",
        &format!("127.0.0.1:{port}"),
    ]
    .map(str::to_string);
    let mut spawned = spawn(&args);
    let ready = wait_for_sandbox(&mut spawned);
    let text = ready.unwrap_or_else(|text| text);
    if let Some(reason) = unavailable(&text, spawned.child.try_wait().ok().flatten().is_some()) {
        skip(reason);
        return;
    }
    assert!(
        spawned.child.try_wait().expect("status").is_none(),
        "innkeeperd exited before login\n{text}"
    );
    let reply = pad_login(port);
    assert!(
        reply
            .windows(LINK_ACK.len())
            .any(|window| window == LINK_ACK),
        "login reply {reply:?}\n{text}"
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        spawned.child.try_wait().expect("status").is_none(),
        "innkeeperd died after answering\n{text}"
    );
}

#[test]
fn a_sandboxed_process_cannot_open_a_file_outside_the_data_dir() {
    let port = free_port();
    let data = Temp(temp_dir("probe-data"));
    let outside = Temp(temp_dir("probe-file"));
    std::fs::create_dir_all(&data.0).expect("data dir");
    std::fs::write(&outside.0, b"secret").expect("outside file");
    let mut child = Command::new(bin())
        .args([
            "--sandbox",
            "--sandbox-probe",
            outside.0.to_str().expect("utf8"),
            "--data-dir",
            data.0.to_str().expect("utf8"),
            "--bind",
            &format!("127.0.0.1:{port}"),
            "--no-int14h",
            "--no-capture",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn probe");
    let status = child.wait().expect("probe exit");
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    if let Some(reason) = unavailable(&stderr, !status.success()) {
        skip(reason);
        return;
    }
    assert!(status.success(), "probe status {status:?}\n{stderr}");
    assert!(
        stdout.contains("sandbox-probe: denied"),
        "{stdout}\n{stderr}"
    );
}

fn wait_for_sandbox(spawned: &mut Spawned) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let text = log_text(spawned);
        let exited = spawned.child.try_wait().expect("status").is_some();
        if text.contains("sandbox:") || text.contains("innkeeperd stopped") || exited {
            return if exited { Err(text) } else { Ok(text) };
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err(log_text(spawned))
}

fn pad_login(port: u16) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    stream.write_all(b"@D\r\r").expect("wake");
    let _ = read_until(&mut stream, b"TERMINAL=");
    stream.write_all(b"c SIERRA\r").expect("call");
    let _ = read_until(&mut stream, b"CONNECTED");
    stream.write_all(&LOGIN_FRAME).expect("login");
    read_until(&mut stream, &LINK_ACK)
}

fn read_until(stream: &mut TcpStream, needle: &[u8]) -> Vec<u8> {
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if got.windows(needle.len()).any(|window| window == needle) {
                    return got;
                }
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => panic!("read failed: {error}"),
        }
    }
    panic!("timed out waiting for {needle:?}, got {got:?}");
}
