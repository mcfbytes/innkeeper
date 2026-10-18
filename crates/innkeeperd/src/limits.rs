//! What one server lets its lines take: how long a write may stall and how many bytes may wait
//! for one client (docs/server/hardening.md section 10).

use std::future::Future;
use std::io;
use std::time::Duration;

/// A write the peer has not taken for this long means it stopped reading.
pub(crate) const WRITE_TIMEOUT_DEFAULT: Duration = Duration::from_secs(60);
/// Bytes waiting for one client, in its inbox or its link: some ten minutes of a 2400 bps line.
pub(crate) const LINE_QUEUE_BYTES_DEFAULT: usize = 2 * 1024 * 1024;

/// The limits every line of one server runs under.
#[derive(Debug)]
pub(crate) struct Limits {
    pub(crate) write_timeout: Duration,
    pub(crate) line_queue_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            write_timeout: WRITE_TIMEOUT_DEFAULT,
            line_queue_bytes: LINE_QUEUE_BYTES_DEFAULT,
        }
    }
}

impl Limits {
    /// Runs one write and returns its result, or `TimedOut` when the peer has not taken it in time.
    pub(crate) async fn write_within<F: Future>(&self, write: F) -> io::Result<F::Output> {
        let written = tokio::time::timeout(self.write_timeout, write).await;
        written.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the client stopped reading"))
    }

    /// Fails once more than the line's share of bytes waits for its client.
    pub(crate) fn check_backlog(&self, queued: usize) -> io::Result<()> {
        if queued <= self.line_queue_bytes {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            format!("{queued} bytes wait for a client that stopped acknowledging"),
        ))
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
