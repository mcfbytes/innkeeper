//! Process-wide counters and gauges: a few numbers, no personal data.
//!
//! The Azure pusher (`--azure-metrics`) and the portal's Status panel read them. Every value is a
//! plain count or a size; there are no labels, so nothing here can name a player (docs/server/privacy.md).

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

pub(crate) mod insights;
mod payload;
mod sampler;

/// What the process counts since it started. Read with [`counters`].
#[derive(Debug, Default)]
pub(crate) struct Counters {
    players: AtomicI64,
    logins: AtomicU64,
    signins: AtomicU64,
    accounts_created: AtomicU64,
    ws_refused: AtomicU64,
    store_errors: AtomicU64,
    messages_in: AtomicU64,
}

/// A copy of the counters at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub(crate) players: u64,
    pub(crate) logins: u64,
    pub(crate) signins: u64,
    pub(crate) accounts_created: u64,
    pub(crate) ws_refused: u64,
    pub(crate) store_errors: u64,
    pub(crate) messages_in: u64,
}

static COUNTERS: Counters = Counters {
    players: AtomicI64::new(0),
    logins: AtomicU64::new(0),
    signins: AtomicU64::new(0),
    accounts_created: AtomicU64::new(0),
    ws_refused: AtomicU64::new(0),
    store_errors: AtomicU64::new(0),
    messages_in: AtomicU64::new(0),
};

pub(crate) fn counters() -> &'static Counters {
    &COUNTERS
}

impl Counters {
    /// A line logged in as an account (players goes up with it until the line hangs up).
    pub(crate) fn login(&self, new_player: bool) {
        self.logins.fetch_add(1, Ordering::Relaxed);
        if new_player {
            self.players.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A logged-in line ended.
    pub(crate) fn player_left(&self) {
        self.players.fetch_sub(1, Ordering::Relaxed);
    }

    /// A web sign-in finished and opened a session.
    pub(crate) fn signin(&self) {
        self.signins.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn account_created(&self) {
        self.accounts_created.fetch_add(1, Ordering::Relaxed);
    }

    /// A WebSocket was refused with a 401.
    pub(crate) fn ws_refused(&self) {
        self.ws_refused.fetch_add(1, Ordering::Relaxed);
    }

    /// The database failed a request (not a missing row: the backend itself).
    pub(crate) fn store_error(&self) {
        self.store_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// A client message arrived on any line.
    pub(crate) fn message_in(&self) {
        self.messages_in.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn totals(&self) -> Totals {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        Totals {
            players: u64::try_from(self.players.load(Ordering::Relaxed)).unwrap_or(0),
            logins: load(&self.logins),
            signins: load(&self.signins),
            accounts_created: load(&self.accounts_created),
            ws_refused: load(&self.ws_refused),
            store_errors: load(&self.store_errors),
            messages_in: load(&self.messages_in),
        }
    }
}

/// Resident memory in bytes, from `/proc/self/status`; `None` where there is no such file.
pub(crate) fn rss_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_vm_rss(&text)
}

fn parse_vm_rss(status: &str) -> Option<u64> {
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    let kib: u64 = line.split_whitespace().next()?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_count_and_players_follow_logins_and_hang_ups() {
        let counters = Counters::default();
        counters.login(true);
        counters.login(false);
        counters.signin();
        counters.account_created();
        counters.ws_refused();
        counters.store_error();
        counters.message_in();
        counters.player_left();
        let totals = counters.totals();
        assert_eq!(
            totals,
            Totals {
                players: 0,
                logins: 2,
                signins: 1,
                accounts_created: 1,
                ws_refused: 1,
                store_errors: 1,
                messages_in: 1
            }
        );
    }

    #[test]
    fn vm_rss_is_read_in_bytes() {
        assert_eq!(
            parse_vm_rss("Name:\tx\nVmRSS:\t  2048 kB\nThreads:\t3\n"),
            Some(2048 * 1024)
        );
        assert_eq!(parse_vm_rss("Name:\tx\n"), None);
        #[cfg(target_os = "linux")]
        assert!(rss_bytes().is_some_and(|bytes| bytes > 0));
    }
}
