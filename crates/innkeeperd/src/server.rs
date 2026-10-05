use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::config::Config;
use crate::connection::{serve_connection, ConnectionSettings};

const STARTUP_LINE: &str = "INT 14h hooked. Please wait while ImagiNation loads...";
/// Pause after a failed accept, so a full descriptor table does not spin the loop.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub(crate) enum ServerError {
    #[error("cannot listen on {bind}: {source}")]
    Bind { bind: SocketAddr, source: io::Error },
}

pub(crate) async fn serve(config: Config) -> Result<(), ServerError> {
    let bind = config.bind;
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|source| ServerError::Bind { bind, source })?;
    let settings = ConnectionSettings {
        session: config.session_config(),
        capture_dir: config.capture_dir(),
    };
    info!(%bind, captures = ?settings.capture_dir, "{STARTUP_LINE}");
    tokio::select! {
        () = accept_forever(listener, Arc::new(settings)) => {}
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
    }
    Ok(())
}

async fn accept_forever(listener: TcpListener, settings: Arc<ConnectionSettings>) {
    let mut next_id = 1u64;
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tokio::spawn(serve_connection(
                    stream,
                    peer,
                    next_id,
                    Arc::clone(&settings),
                ));
                next_id += 1;
            }
            Err(error) => {
                warn!(%error, "accept failed");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use innkeeper_session::SessionConfig;
    use pad_thai::LineKind;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    async fn expect_reply(client: &mut TcpStream, expected: &[u8]) {
        let mut received = vec![0; expected.len()];
        let read = tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut received));
        read.await.expect("server answered in time").unwrap();
        assert_eq!(
            String::from_utf8_lossy(&received),
            String::from_utf8_lossy(expected)
        );
    }

    #[tokio::test]
    async fn stock_client_logon_over_tcp_is_answered_and_captured() {
        let capture_dir =
            std::env::temp_dir().join(format!("innkeeperd-test-{}", std::process::id()));
        let session = SessionConfig {
            line: LineKind::Pad,
            ..SessionConfig::default()
        };
        let settings = ConnectionSettings {
            session,
            capture_dir: Some(capture_dir.clone()),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(listener, Arc::new(settings)));

        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"@D\r\r").await.unwrap();
        expect_reply(&mut client, b"\r\nTERMINAL=\r\n@").await;
        client.write_all(b"c SIERRA\r").await.unwrap();
        expect_reply(&mut client, b"\r\nSIERRA CONNECTED\r\n").await;
        client
            .write_all(&[0x81, 0x83, 0xBC, 0x00, 0x03, 0x22, 0x04, 0x10, 0x82])
            .await
            .unwrap();
        expect_reply(&mut client, &[0x81, 0x49, 0x62, 0x90, 0x82]).await;
        drop(client);

        let mut captured = String::new();
        for _ in 0..50 {
            let files: Vec<_> = std::fs::read_dir(&capture_dir).unwrap().flatten().collect();
            captured = files
                .iter()
                .map(|f| std::fs::read_to_string(f.path()).unwrap())
                .collect();
            if captured.contains("ev message len=3 22 04 10") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        std::fs::remove_dir_all(&capture_dir).unwrap();
        assert!(captured.contains("ev message len=3 22 04 10"), "{captured}");
        assert!(captured.contains(" rx 81 49 62 90 82 "), "{captured}");
    }
}
