//! What one server lets its lines take: how many at once and per signed-in identity, how long
//! silent, how long a write may stall and how many bytes may wait (docs/server/hardening.md 10).

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::signin::keys::IdentityKey;

/// Lines at once: each may hold its queue, a 2 MiB save message and its buffers, about 6 MiB, so
/// 64 of them stay within 400 MiB of a 1 GiB host.
pub(crate) const MAX_LINES_DEFAULT: usize = 64;
/// Lines one signed-in identity may hold: its game, and a reconnect that beats the old line's close.
pub(crate) const LINES_PER_IDENTITY_DEFAULT: usize = 2;
/// The client has no keepalive and a DOS game may stay silent while the player waits.
pub(crate) const IDLE_TIMEOUT_DEFAULT: Duration = Duration::from_secs(30 * 60);
/// A write the peer has not taken for this long means it stopped reading.
pub(crate) const WRITE_TIMEOUT_DEFAULT: Duration = Duration::from_secs(60);
/// Bytes waiting for one client, in its inbox or its link: some ten minutes of a 2400 bps line.
pub(crate) const LINE_QUEUE_BYTES_DEFAULT: usize = 2 * 1024 * 1024;

type IdentityLines = Arc<Mutex<HashMap<IdentityKey, usize>>>;

/// The limits every line of one server runs under, and the lines each identity holds now.
#[derive(Debug)]
pub(crate) struct Limits {
    pub(crate) write_timeout: Duration,
    pub(crate) line_queue_bytes: usize,
    /// A line that has sent no byte for this long is ended; `None` keeps silent lines.
    pub(crate) idle_timeout: Option<Duration>,
    lines: Arc<Semaphore>,
    lines_per_identity: usize,
    identities: IdentityLines,
}

/// Why a new line was turned away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    ServerFull,
    IdentityFull,
}

/// A line's place among the live ones; dropping it frees the place.
#[derive(Debug)]
pub(crate) struct LinePermit {
    _line: OwnedSemaphorePermit,
    identity: Option<(IdentityKey, IdentityLines)>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits::with_lines(MAX_LINES_DEFAULT, LINES_PER_IDENTITY_DEFAULT)
    }
}

impl Limits {
    pub(crate) fn with_lines(max_lines: usize, lines_per_identity: usize) -> Self {
        Limits {
            write_timeout: WRITE_TIMEOUT_DEFAULT,
            line_queue_bytes: LINE_QUEUE_BYTES_DEFAULT,
            idle_timeout: Some(IDLE_TIMEOUT_DEFAULT),
            lines: Arc::new(Semaphore::new(max_lines)),
            lines_per_identity,
            identities: Arc::default(),
        }
    }

    /// A place for one more line, counted against `identity` too when the line is signed in.
    pub(crate) fn admit(&self, identity: Option<&IdentityKey>) -> Result<LinePermit, Refusal> {
        let line = Arc::clone(&self.lines).try_acquire_owned();
        let line = line.map_err(|_| Refusal::ServerFull)?;
        let Some(&identity) = identity else {
            return Ok(LinePermit {
                _line: line,
                identity: None,
            });
        };
        let mut identities = lock(&self.identities);
        let held = identities.entry(identity).or_default();
        if *held >= self.lines_per_identity {
            return Err(Refusal::IdentityFull);
        }
        *held += 1;
        drop(identities);
        Ok(LinePermit {
            _line: line,
            identity: Some((identity, Arc::clone(&self.identities))),
        })
    }

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

impl Drop for LinePermit {
    fn drop(&mut self) {
        let Some((identity, identities)) = &self.identity else {
            return;
        };
        let mut identities = lock(identities);
        if let Some(held) = identities.get_mut(identity) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                identities.remove(identity);
            }
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::ServerFull => "every line is taken",
            Refusal::IdentityFull => "this sign-in already has its lines",
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
