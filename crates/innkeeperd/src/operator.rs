//! The operator console: lines on standard input become host actions; the syntax is in
//! docs/server/innkeeperd.md section 2.

use std::io::{self, BufRead};
use std::sync::Arc;
use std::thread;

use innkeeper_world::{HostMessage, Notice};
use tracing::{debug, info, warn};

use crate::connection::ConnectionSettings;
use crate::switchboard::Switchboard;

const SAY: &str = "say";

/// Reads standard input on a thread of its own: a blocked read there must not hold up shutdown.
pub(crate) fn spawn_console(settings: Arc<ConnectionSettings>) -> io::Result<()> {
    thread::Builder::new()
        .name("operator".into())
        .spawn(move || run(io::stdin().lock(), &settings.switchboard))
        .map(drop)
}

/// Carries out each line of `input` until it ends.
fn run(input: impl BufRead, switchboard: &Switchboard) {
    for line in input.lines() {
        match line {
            Ok(line) => carry_out(&line, switchboard),
            Err(error) => {
                warn!(%error, "operator console stopped");
                return;
            }
        }
    }
    debug!("operator console closed");
}

fn carry_out(line: &str, switchboard: &Switchboard) {
    let (verb, rest) = line
        .trim()
        .split_once(char::is_whitespace)
        .unwrap_or((line.trim(), ""));
    let text = rest.trim();
    match (verb, text) {
        ("", _) => {}
        (SAY, "") => warn!("say needs a text"),
        (SAY, text) => {
            let notice = HostMessage::Notice(Notice { text: text.into() });
            let reached = switchboard.broadcast(&notice);
            info!(reached, text, "operator notice sent");
        }
        (verb, _) => warn!(verb, "unknown operator command; try: say TEXT"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use innkeeper_world::ConnectionId;

    fn notice(text: &str) -> HostMessage {
        HostMessage::Notice(Notice { text: text.into() })
    }

    #[tokio::test]
    async fn a_say_line_is_a_notice_for_every_connection() {
        let switchboard = Switchboard::default();
        let mut first = switchboard.register(ConnectionId(1));
        let mut second = switchboard.register(ConnectionId(2));
        run("say Back in five minutes\n".as_bytes(), &switchboard);
        assert_eq!(first.next().await, Some(notice("Back in five minutes")));
        assert_eq!(second.next().await, Some(notice("Back in five minutes")));
    }

    #[test]
    fn other_lines_send_nothing() {
        let switchboard = Switchboard::default();
        let mut inbox = switchboard.register(ConnectionId(1));
        run(
            "\nsay\nsay   \nsaying hi\nshout hi\n".as_bytes(),
            &switchboard,
        );
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn the_text_keeps_its_inner_spacing_and_loses_the_outer() {
        let switchboard = Switchboard::default();
        let mut inbox = switchboard.register(ConnectionId(1));
        run("  say \t two  words  \n".as_bytes(), &switchboard);
        assert_eq!(inbox.drain(), [notice("two  words")]);
    }
}
