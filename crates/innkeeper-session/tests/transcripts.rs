//! Golden dialogues between the stock client and the session; the syntax is in each file header.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use std::time::{Duration, Instant};

use innkeeper_session::{Session, SessionConfig, SessionEvent, SessionOutput};

fn unescape(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            bytes.push(c as u8);
            continue;
        }
        match chars.next() {
            Some('r') => bytes.push(b'\r'),
            Some('n') => bytes.push(b'\n'),
            Some('\\') => bytes.push(b'\\'),
            Some('x') => {
                let pair: String = chars.by_ref().take(2).collect();
                bytes.push(u8::from_str_radix(&pair, 16).unwrap());
            }
            other => panic!("bad escape \\{other:?} in {text:?}"),
        }
    }
    bytes
}

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

#[derive(Default)]
struct Pending {
    to_client: Vec<u8>,
    messages: Vec<Vec<u8>>,
}

impl Pending {
    fn collect(&mut self, session: &mut Session) {
        while let Some(output) = session.poll_output() {
            match output {
                SessionOutput::ToClient(bytes) => self.to_client.extend(bytes),
                SessionOutput::Event(SessionEvent::Message(message)) => {
                    self.messages.push(message.body().to_vec());
                }
                SessionOutput::Event(_) => {}
            }
        }
    }

    fn expect_to_client(&mut self, expected: &[u8], line: usize) {
        let actual = self.to_client.get(..expected.len());
        assert_eq!(
            actual.map(String::from_utf8_lossy),
            Some(String::from_utf8_lossy(expected)),
            "line {line}: pending output {:?}",
            String::from_utf8_lossy(&self.to_client)
        );
        self.to_client.drain(..expected.len());
    }

    fn assert_consumed(&self, line: usize) {
        let unexpected = String::from_utf8_lossy(&self.to_client);
        assert!(
            self.to_client.is_empty(),
            "line {line}: unexpected output {unexpected:?}"
        );
        assert!(
            self.messages.is_empty(),
            "line {line}: unexpected messages {:?}",
            self.messages
        );
    }
}

fn run_transcript(transcript: &str) {
    let mut now = Instant::now();
    let mut session = Session::new(SessionConfig::default());
    let mut pending = Pending::default();
    for (index, text) in transcript.lines().enumerate() {
        let line = index + 1;
        let (directive, argument) = text.split_once(' ').unwrap_or((text, ""));
        match directive {
            "" | "#" => {}
            ">" | ">x" => {
                pending.assert_consumed(line);
                let bytes = if directive == ">" {
                    unescape(argument)
                } else {
                    hex(argument)
                };
                session.handle_input(&bytes, now);
            }
            "<" => pending.expect_to_client(&unescape(argument), line),
            "<x" => pending.expect_to_client(&hex(argument), line),
            "m" => assert_eq!(pending.messages.remove(0), hex(argument), "line {line}"),
            advance if advance.starts_with('+') => {
                pending.assert_consumed(line);
                now += Duration::from_millis(advance[1..].parse().unwrap());
                session.handle_timeout(now);
            }
            other => panic!("line {line}: unknown directive {other:?}"),
        }
        pending.collect(&mut session);
    }
    pending.assert_consumed(transcript.lines().count());
}

#[test]
fn raw_serial_dial_escape_and_hangup() {
    run_transcript(include_str!("golden/raw_serial.txt"));
}

#[test]
fn modem_emulator_logon_and_land_switch() {
    run_transcript(include_str!("golden/modem_emulator.txt"));
}
