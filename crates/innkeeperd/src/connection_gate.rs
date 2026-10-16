//! Whether an accepted WebSocket may be upgraded. Sign-in checks the session token here
//! (docs/server/hardening.md).

use std::io;
use std::net::SocketAddr;

use tokio::net::TcpStream;

/// Logged once at startup while the sign-in check still admits every socket.
pub(crate) const SIGN_IN_NOT_READY: &str = "\
public mode: sign-in is not implemented yet, so WebSocket connections are not authenticated; \
do not expose this server publicly until it is";

/// TODO: require and validate the session token before upgrading.
pub(crate) fn admit(_stream: &TcpStream, _peer: SocketAddr) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SIGN_IN_NOT_READY;

    #[test]
    fn the_startup_warning_is_the_operator_sentence() {
        assert_eq!(
            SIGN_IN_NOT_READY,
            "public mode: sign-in is not implemented yet, so WebSocket connections are not \
             authenticated; do not expose this server publicly until it is"
        );
    }
}
