//! systemd's notify protocol in a few lines, with no library: `READY=1` once the daemon serves,
//! `WATCHDOG=1` while it is healthy, `STOPPING=1` on the way out (docs/server/hardening.md).
//!
//! Everything is a no-op without `NOTIFY_SOCKET` (and the watchdog without `WATCHDOG_USEC`), so a
//! daemon run by hand behaves as before.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use innkeeper_world::{AccountId, Store};
use tracing::{info, warn};

use crate::sqlite_store::SqliteStore;

/// Where `NOTIFY_SOCKET` points.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Path(std::path::PathBuf),
    /// A Linux abstract socket: `@name` in the variable.
    Abstract(Vec<u8>),
}

fn target_from(value: Option<OsString>) -> Option<Target> {
    use std::os::unix::ffi::OsStrExt;
    let value = value?;
    let bytes = value.as_bytes();
    match bytes.first()? {
        b'@' => Some(Target::Abstract(bytes.get(1..)?.to_vec())),
        b'/' => Some(Target::Path(value.into())),
        _ => None,
    }
}

#[cfg(unix)]
fn send_to(target: &Target, state: &str) -> std::io::Result<()> {
    use std::os::unix::net::{SocketAddr, UnixDatagram};
    let address = match target {
        Target::Path(path) => SocketAddr::from_pathname(path)?,
        #[cfg(target_os = "linux")]
        Target::Abstract(name) => {
            use std::os::linux::net::SocketAddrExt;
            SocketAddr::from_abstract_name(name)?
        }
        #[cfg(not(target_os = "linux"))]
        Target::Abstract(_) => return Ok(()),
    };
    let socket = UnixDatagram::unbound()?;
    socket.send_to_addr(state.as_bytes(), &address)?;
    Ok(())
}

#[cfg(not(unix))]
fn send_to(_target: &Target, _state: &str) -> std::io::Result<()> {
    Ok(())
}

/// Sends one state line. `Ok(false)` when there is no notify socket.
fn send(state: &str) -> std::io::Result<bool> {
    let Some(target) = target_from(std::env::var_os("NOTIFY_SOCKET")) else {
        return Ok(false);
    };
    send_to(&target, state)?;
    Ok(true)
}

/// Tells systemd the daemon is up: sockets bound and the sandbox applied.
pub(crate) fn ready() {
    match send("READY=1") {
        Ok(true) => info!("told systemd the daemon is ready"),
        Ok(false) => {}
        Err(error) => warn!(%error, "cannot tell systemd the daemon is ready"),
    }
}

/// Tells systemd the daemon is going down on purpose, so the watchdog stands down.
pub(crate) fn stopping() {
    if let Err(error) = send("STOPPING=1") {
        warn!(%error, "cannot tell systemd the daemon is stopping");
    }
}

/// The watchdog period systemd asked for, from `WATCHDOG_USEC`, unless the variables are for another
/// process (`WATCHDOG_PID` names it).
fn watchdog_period(usec: Option<&str>, pid: Option<&str>, own_pid: u32) -> Option<Duration> {
    if let Some(pid) = pid {
        if pid.trim().parse::<u32>().ok()? != own_pid {
            return None;
        }
    }
    let usec: u64 = usec?.trim().parse().ok()?;
    (usec > 0).then(|| Duration::from_micros(usec))
}

/// Half of `WATCHDOG_USEC`, as systemd recommends; `None` without a watchdog.
pub(crate) fn watchdog_interval() -> Option<Duration> {
    let usec = std::env::var("WATCHDOG_USEC").ok();
    let pid = std::env::var("WATCHDOG_PID").ok();
    let period = watchdog_period(usec.as_deref(), pid.as_deref(), std::process::id())?;
    // A socket to send to is part of the deal; without one there is nobody to ping.
    std::env::var_os("NOTIFY_SOCKET")?;
    Some(period / 2)
}

/// Runs a cheap read on a blocking thread; true if it succeeds within `within`.
async fn answers_within<E: std::fmt::Display + Send + 'static>(
    within: Duration,
    ping: impl FnOnce() -> Result<(), E> + Send + 'static,
) -> bool {
    match tokio::time::timeout(within, tokio::task::spawn_blocking(ping)).await {
        Ok(Ok(Ok(()))) => true,
        Ok(Ok(Err(error))) => {
            crate::metrics::counters().store_error();
            warn!(%error, "watchdog: the database did not answer");
            false
        }
        Ok(Err(_)) | Err(_) => {
            warn!("watchdog: the database did not answer in time");
            false
        }
    }
}

/// A cheap read the database must answer. A wedged database never does, and the pings stop.
async fn store_alive(store: &Arc<SqliteStore>, within: Duration) -> bool {
    let store = Arc::clone(store);
    answers_within(within, move || store.account(AccountId(0)).map(|_| ())).await
}

/// Pings systemd every `interval` while this task runs (so the runtime is turning) and the
/// database answers. Never returns.
pub(crate) async fn watchdog(interval: Duration, store: Option<Arc<SqliteStore>>) {
    info!(?interval, "systemd watchdog on");
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let alive = match &store {
            Some(store) => store_alive(store, interval).await,
            None => true,
        };
        if alive {
            if let Err(error) = send("WATCHDOG=1") {
                warn!(%error, "cannot ping the systemd watchdog");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixDatagram;

    #[test]
    fn notify_socket_names_a_path_or_an_abstract_socket() {
        assert_eq!(
            target_from(Some("/run/systemd/notify".into())),
            Some(Target::Path("/run/systemd/notify".into()))
        );
        assert_eq!(
            target_from(Some("@innkeeper".into())),
            Some(Target::Abstract(b"innkeeper".to_vec()))
        );
        assert_eq!(target_from(Some("relative".into())), None);
        assert_eq!(target_from(Some("".into())), None);
        assert_eq!(target_from(None), None);
    }

    #[test]
    fn the_watchdog_period_needs_usec_and_the_right_pid() {
        let second = Some(Duration::from_secs(1));
        assert_eq!(watchdog_period(Some("1000000"), None, 7), second);
        assert_eq!(watchdog_period(Some("1000000"), Some("7"), 7), second);
        assert_eq!(watchdog_period(Some("1000000"), Some("8"), 7), None);
        assert_eq!(watchdog_period(Some("0"), None, 7), None);
        assert_eq!(watchdog_period(Some("soon"), None, 7), None);
        assert_eq!(watchdog_period(None, None, 7), None);
    }

    #[test]
    fn a_state_line_reaches_a_datagram_socket() {
        let dir = std::env::temp_dir().join(format!("innkeeperd-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify");
        let receiver = UnixDatagram::bind(&path).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        send_to(&Target::Path(path), "READY=1").unwrap();
        let mut buffer = [0u8; 64];
        let read = receiver.recv(&mut buffer).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(&buffer[..read], b"READY=1");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_state_line_reaches_an_abstract_socket() {
        use std::os::linux::net::SocketAddrExt;
        let name = format!("innkeeperd-notify-{}", std::process::id());
        let address = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let receiver = UnixDatagram::bind_addr(&address).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        send_to(&Target::Abstract(name.into_bytes()), "WATCHDOG=1").unwrap();
        let mut buffer = [0u8; 64];
        let read = receiver.recv(&mut buffer).unwrap();
        assert_eq!(&buffer[..read], b"WATCHDOG=1");
    }

    #[test]
    fn without_the_environment_nothing_is_sent() {
        // The test runner is not under systemd. If it were, this would not be a no-op to assert.
        if std::env::var_os("NOTIFY_SOCKET").is_none() {
            assert!(!send("READY=1").unwrap());
            assert_eq!(watchdog_interval(), None);
        }
    }

    #[tokio::test]
    async fn a_live_store_answers_the_ping() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        assert!(store_alive(&store, Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn a_wedged_or_failing_ping_is_not_alive() {
        let wedged = answers_within(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(400));
            Ok::<(), String>(())
        })
        .await;
        assert!(!wedged);
        let failing = answers_within(Duration::from_secs(5), || Err::<(), _>("disk")).await;
        assert!(!failing);
    }
}
