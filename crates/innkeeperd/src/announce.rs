//! Operator notices (command 48, `Unsolicited`): what the portal announces, and the countdown a stop
//! gives the players before the server goes (docs/server/innkeeperd.md section 11).
//!
//! The LSCI hub (`RoomZeroHandler`) and every DOS game show the text; Red Baron only once a game runs
//! (docs/server/coverage.md, row 48). Only logged-in lines are told: a line still dialing has no
//! handler for it yet.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use innkeeper_world::{Delivery, HostMessage, Notice};
use tokio::signal::unix::{signal, Signal, SignalKind};
use tokio::time::Instant;
use tracing::{info, warn};

use crate::connection::ConnectionSettings;

/// Longest notice sent. The client's own width is not known; this fits two lines of the hub's text.
pub(crate) const MAX_NOTICE: usize = 120;
/// How long the final notice is given to reach the clients before the process ends.
const LAST_WORD: Duration = Duration::from_secs(2);
/// How often the countdown looks whether everyone has left, so it can stop early.
const EMPTY_CHECK: Duration = Duration::from_secs(5);

/// The text as a notice: printable ASCII only, runs of space and control characters made one space,
/// cut to [`MAX_NOTICE`]. None when nothing is left.
pub(crate) fn clean(text: &str) -> Option<String> {
    let mut out = String::new();
    for word in text
        .split(|ch: char| ch.is_whitespace() || ch.is_control())
        .filter(|word| !word.is_empty())
    {
        let word: String = word.chars().filter(|ch| ch.is_ascii_graphic()).collect();
        if word.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
    }
    let out: String = out.chars().take(MAX_NOTICE).collect();
    let out = out.trim_end().to_owned();
    (!out.is_empty()).then_some(out)
}

/// Sends the notice to every logged-in line; returns how many were told.
pub(crate) fn announce(settings: &ConnectionSettings, text: &str) -> usize {
    let online: Vec<_> = settings
        .objects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .online()
        .into_iter()
        .map(|entry| entry.connection)
        .collect();
    for &to in &online {
        settings.switchboard.deliver(Delivery {
            to,
            message: HostMessage::Notice(Notice {
                text: text.to_owned(),
            }),
        });
    }
    online.len()
}

fn online_count(settings: &ConnectionSettings) -> usize {
    settings
        .objects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .online()
        .len()
}

fn span(left: Duration) -> String {
    let secs = left.as_secs();
    match secs.div_ceil(60) {
        _ if secs < 60 => format!("{secs} seconds"),
        1 => "1 minute".to_owned(),
        minutes => format!("{minutes} minutes"),
    }
}

/// The countdown's notices: (time after the stop was asked for, text). The first goes at once; then
/// one a minute before and ten seconds before, where the grace leaves room for them.
pub(crate) fn countdown(grace: Duration) -> Vec<(Duration, String)> {
    let mut out = vec![(
        Duration::ZERO,
        format!(
            "INN will restart for maintenance in {}. Please finish up and log off; you can reconnect in a few minutes.",
            span(grace)
        ),
    )];
    for before in [Duration::from_secs(60), Duration::from_secs(10)] {
        // Only a reminder that comes well after the first notice says anything new.
        if grace >= before * 2 {
            out.push((
                grace - before,
                format!("INN restarts in {}. Please log off now.", span(before)),
            ));
        }
    }
    out
}

const RESTARTING_NOW: &str = "INN is restarting now. Please reconnect in a minute or two.";

/// SIGTERM and SIGINT, registered now so that neither ends the process before [`stop_requested`]
/// is waiting for it.
pub(crate) struct StopSignals {
    term: Signal,
    interrupt: Signal,
}

impl StopSignals {
    pub(crate) fn register() -> std::io::Result<Self> {
        Ok(StopSignals {
            term: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }
}

/// Returns when the server should stop. The first SIGTERM (systemd's stop, a deploy's restart, a
/// reboot) warns the logged-in players and waits `grace`, with reminders, or until nobody is left; a
/// second SIGTERM or any SIGINT ends the wait at once. With nobody online, or a zero grace, it
/// returns straight after the signal. The players are told once more as it returns.
pub(crate) async fn stop_requested(
    mut signals: StopSignals,
    settings: Arc<ConnectionSettings>,
    grace: Duration,
) {
    let interrupted = tokio::select! {
        _ = signals.term.recv() => false,
        _ = signals.interrupt.recv() => true,
    };
    crate::notify::stopping();
    let online = online_count(&settings);
    if interrupted || grace.is_zero() || online == 0 {
        info!(online, interrupted, "stopping now");
    } else {
        info!(online, grace = %humantime::format_duration(grace), "stop asked for: warning the players first");
        let start = Instant::now();
        let deadline = start + grace;
        let mut notices = countdown(grace).into_iter().peekable();
        loop {
            let next_notice = notices.peek().map_or(deadline, |(after, _)| start + *after);
            let wake = next_notice.min(deadline).min(Instant::now() + EMPTY_CHECK);
            tokio::select! {
                () = tokio::time::sleep_until(wake) => {}
                _ = signals.term.recv() => { warn!("a second stop: not waiting any longer"); break; }
                _ = signals.interrupt.recv() => { warn!("interrupted: not waiting any longer"); break; }
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            if notices
                .peek()
                .is_some_and(|(after, _)| now >= start + *after)
            {
                if let Some((_, text)) = notices.next() {
                    let told = announce(&settings, &text);
                    info!(told, "stop countdown notice sent");
                }
            }
            if online_count(&settings) == 0 {
                info!("everyone has left: stopping before the countdown ends");
                break;
            }
        }
    }
    if announce(&settings, RESTARTING_NOW) > 0 {
        tokio::time::sleep(LAST_WORD).await;
    }
    info!("shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_is_printable_ascii_on_one_line_and_cut_to_size() {
        assert_eq!(
            clean("  Back\tin  five\r\nminutes  ").as_deref(),
            Some("Back in five minutes")
        );
        assert_eq!(
            clean("caf\u{e9} \u{1b}[31m ok").as_deref(),
            Some("caf [31m ok")
        );
        assert_eq!(clean(" \u{e9}\u{e9} \n"), None);
        let long = "x".repeat(500);
        assert_eq!(clean(&long).map(|text| text.len()), Some(MAX_NOTICE));
    }

    #[test]
    fn five_minutes_warns_at_once_then_a_minute_and_ten_seconds_before() {
        let notices = countdown(Duration::from_secs(300));
        let at: Vec<u64> = notices.iter().map(|(after, _)| after.as_secs()).collect();
        assert_eq!(at, [0, 240, 290]);
        assert!(notices[0].1.contains("in 5 minutes"), "{}", notices[0].1);
        assert!(notices[1].1.contains("in 1 minute."), "{}", notices[1].1);
        assert!(notices[2].1.contains("in 10 seconds"), "{}", notices[2].1);
        for (_, text) in &notices {
            assert!(text.len() <= MAX_NOTICE, "{text}");
            assert_eq!(clean(text).as_deref(), Some(text.as_str()));
        }
        assert!(RESTARTING_NOW.len() <= MAX_NOTICE);
    }

    #[test]
    fn a_short_grace_skips_the_reminders_it_has_no_room_for() {
        let at = |secs| -> Vec<u64> {
            countdown(Duration::from_secs(secs))
                .iter()
                .map(|(after, _)| after.as_secs())
                .collect()
        };
        assert_eq!(at(90), [0, 80]);
        assert_eq!(at(15), [0]);
        assert!(countdown(Duration::from_secs(90))[0]
            .1
            .contains("in 2 minutes"));
        assert!(countdown(Duration::from_secs(15))[0]
            .1
            .contains("in 15 seconds"));
    }
}
