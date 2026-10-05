//! A Hayes modem for clients on raw serial or null-modem links, where nothing else answers `AT`.
//! What the com driver sends and expects is in docs/protocol/link-layer.md.

mod command;

use std::fmt;
use std::time::{Duration, Instant};

use command::{parse_command_line, AtCommand};

use crate::typed_line::{TypedLine, CR, SEVEN_BITS};

const ESCAPE_CHAR: u8 = b'+';
const ESCAPE_LEN: u8 = 3;
const MAX_COMMAND_LEN: usize = 255;

/// How the impersonated modem presents itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HayesConfig {
    /// Rate in the `CONNECT` line; the driver ignores values of 32768 and above.
    pub connect_rate: u32,
    /// Silence required around `+++`, the Hayes default of S12 = 50.
    pub guard_time: Duration,
}

impl Default for HayesConfig {
    fn default() -> Self {
        HayesConfig {
            connect_rate: 2400,
            guard_time: Duration::from_secs(1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModemEvent {
    Command(String),
    Dialled {
        number: String,
    },
    /// `+++` with guard times: back to command mode with the call still up.
    Escaped,
    ReturnedOnline,
    /// The client asked the modem to send a BREAK, which escapes to the PAD.
    Break,
    HungUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModemOutput {
    Reply(Vec<u8>),
    /// Bytes that passed through the modem in data mode.
    Data(Vec<u8>),
    Event(ModemEvent),
}

#[derive(Clone, Copy, Debug)]
enum EscapeWatch {
    Idle,
    Counting { pluses: u8 },
    Pending { since: Instant },
}

#[derive(Debug)]
struct CommandMode {
    line: TypedLine,
    call_up: bool,
}

#[derive(Debug)]
struct OnlineMode {
    escape: EscapeWatch,
    last_input_at: Instant,
}

#[derive(Debug)]
enum Mode {
    Command(CommandMode),
    Online(OnlineMode),
}

/// The modem's side of the serial line, sans-IO: client bytes and a clock in, replies out.
#[derive(Debug)]
pub(crate) struct HayesModem {
    config: HayesConfig,
    echo: bool,
    mode: Mode,
}

impl HayesModem {
    pub(crate) fn new(config: HayesConfig) -> Self {
        HayesModem {
            config,
            echo: true,
            mode: Mode::Command(CommandMode::new(false)),
        }
    }

    #[cfg(test)]
    fn is_online(&self) -> bool {
        matches!(self.mode, Mode::Online(_))
    }

    #[must_use]
    pub(crate) fn handle_input(&mut self, bytes: &[u8], now: Instant) -> Vec<ModemOutput> {
        let mut outputs = self.handle_timeout(now);
        let mut rest = bytes;
        while !rest.is_empty() {
            rest = match &mut self.mode {
                Mode::Command(_) => self.command_input(rest, now, &mut outputs),
                Mode::Online(online) => {
                    let guard = self.config.guard_time;
                    rest.iter().for_each(|&byte| online.watch(byte, now, guard));
                    outputs.push(ModemOutput::Data(rest.to_vec()));
                    &[]
                }
            };
        }
        outputs
    }

    /// Completes a `+++` escape once the trailing guard time has passed in silence.
    #[must_use]
    pub(crate) fn handle_timeout(&mut self, now: Instant) -> Vec<ModemOutput> {
        match self.next_deadline() {
            Some(deadline) if now >= deadline => {
                self.mode = Mode::Command(CommandMode::new(true));
                vec![self.result("OK"), ModemOutput::Event(ModemEvent::Escaped)]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        match &self.mode {
            Mode::Online(OnlineMode {
                escape: EscapeWatch::Pending { since },
                ..
            }) => Some(*since + self.config.guard_time),
            _ => None,
        }
    }

    fn command_input<'a>(
        &mut self,
        bytes: &'a [u8],
        now: Instant,
        outputs: &mut Vec<ModemOutput>,
    ) -> &'a [u8] {
        let line_end = bytes.iter().position(|&byte| byte & SEVEN_BITS == CR);
        let (typed, rest) = bytes.split_at(line_end.map_or(bytes.len(), |at| at + 1));
        if self.echo {
            outputs.push(ModemOutput::Reply(typed.to_vec()));
        }
        let Mode::Command(command) = &mut self.mode else {
            return rest;
        };
        let call_up = command.call_up;
        if let Some(line) = command.type_bytes(typed) {
            self.execute(&line, call_up, now, outputs);
        }
        rest
    }

    fn execute(&mut self, line: &str, call_up: bool, now: Instant, out: &mut Vec<ModemOutput>) {
        let Some(commands) = parse_command_line(line) else {
            return;
        };
        out.push(ModemOutput::Event(ModemEvent::Command(
            line.trim().to_owned(),
        )));
        let mut call_up = call_up;
        let mut after_ok = Vec::new();
        for command in commands {
            match command {
                AtCommand::Dial(number) => {
                    out.push(ModemOutput::Event(ModemEvent::Dialled { number }));
                    self.go_online(now, out);
                    return;
                }
                AtCommand::Online if call_up => {
                    out.push(ModemOutput::Event(ModemEvent::ReturnedOnline));
                    self.go_online(now, out);
                    return;
                }
                AtCommand::Online => {
                    out.push(self.result("NO CARRIER"));
                    return;
                }
                AtCommand::HangUp | AtCommand::Reset if call_up => {
                    call_up = false;
                    after_ok.push(ModemOutput::Event(ModemEvent::HungUp));
                }
                AtCommand::SendBreak if call_up => {
                    after_ok.push(ModemOutput::Event(ModemEvent::Break));
                }
                AtCommand::Echo(on) => self.echo = on,
                AtCommand::HangUp | AtCommand::Reset | AtCommand::SendBreak => {}
                AtCommand::Other(_) => {}
            }
        }
        self.mode = Mode::Command(CommandMode::new(call_up));
        out.push(self.result("OK"));
        out.extend(after_ok);
    }

    fn go_online(&mut self, now: Instant, out: &mut Vec<ModemOutput>) {
        self.mode = Mode::Online(OnlineMode {
            escape: EscapeWatch::Idle,
            last_input_at: now,
        });
        out.push(self.result(&format!("CONNECT {}", self.config.connect_rate)));
    }

    fn result(&self, text: &str) -> ModemOutput {
        ModemOutput::Reply(format!("\r\n{text}\r\n").into_bytes())
    }
}

impl CommandMode {
    fn new(call_up: bool) -> Self {
        CommandMode {
            line: TypedLine::new(MAX_COMMAND_LEN),
            call_up,
        }
    }

    /// Applies typed bytes to the line buffer; returns the line when a CR completes it.
    fn type_bytes(&mut self, typed: &[u8]) -> Option<String> {
        typed.iter().find_map(|&byte| self.line.push(byte))
    }
}

impl OnlineMode {
    fn watch(&mut self, byte: u8, now: Instant, guard: Duration) {
        let silent_before = now.saturating_duration_since(self.last_input_at) >= guard;
        self.escape = match (self.escape, byte) {
            (_, ESCAPE_CHAR) if silent_before => EscapeWatch::Counting { pluses: 1 },
            (EscapeWatch::Counting { pluses }, ESCAPE_CHAR) if pluses + 1 == ESCAPE_LEN => {
                EscapeWatch::Pending { since: now }
            }
            (EscapeWatch::Counting { pluses }, ESCAPE_CHAR) => {
                EscapeWatch::Counting { pluses: pluses + 1 }
            }
            _ => EscapeWatch::Idle,
        };
        self.last_input_at = now;
    }
}

impl fmt::Display for ModemEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModemEvent::Command(line) => write!(f, "AT command {line:?}"),
            ModemEvent::Dialled { number } => write!(f, "dialled {number:?}"),
            ModemEvent::Escaped => write!(f, "+++ escape to command mode"),
            ModemEvent::ReturnedOnline => write!(f, "back online"),
            ModemEvent::Break => write!(f, "BREAK requested"),
            ModemEvent::HungUp => write!(f, "hung up"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replies(outputs: &[ModemOutput]) -> String {
        let text = outputs.iter().filter_map(|o| match o {
            ModemOutput::Reply(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        });
        text.collect()
    }

    fn events(outputs: &[ModemOutput]) -> Vec<ModemEvent> {
        let found = outputs.iter().filter_map(|o| match o {
            ModemOutput::Event(event) => Some(event.clone()),
            _ => None,
        });
        found.collect()
    }

    fn online_modem(t0: Instant) -> HayesModem {
        let mut modem = HayesModem::new(HayesConfig::default());
        let _ = modem.handle_input(b"ATE0\rATDT5551234\r", t0);
        assert!(modem.is_online());
        modem
    }

    #[test]
    fn dial_answers_connect_and_passes_later_bytes_through() {
        let t0 = Instant::now();
        let mut modem = HayesModem::new(HayesConfig::default());
        let outputs = modem.handle_input(b"\rAT\rATDT5551234\r@D", t0);
        assert_eq!(
            replies(&outputs),
            "\rAT\r\r\nOK\r\nATDT5551234\r\r\nCONNECT 2400\r\n"
        );
        assert!(events(&outputs).contains(&ModemEvent::Dialled {
            number: "5551234".into()
        }));
        assert_eq!(outputs.last(), Some(&ModemOutput::Data(b"@D".to_vec())));
    }

    #[test]
    fn guarded_plus_escape_reaches_command_mode() {
        let t0 = Instant::now();
        let guard = HayesConfig::default().guard_time;
        let mut modem = online_modem(t0);
        let _ = modem.handle_input(b"+++", t0 + guard);
        assert_eq!(modem.next_deadline(), Some(t0 + guard * 2));
        let outputs = modem.handle_timeout(t0 + guard * 2);
        assert_eq!(replies(&outputs), "\r\nOK\r\n");
        assert!(!modem.is_online());

        let outputs = modem.handle_input(b"AT\\B\rATO\r", t0 + guard * 3);
        assert_eq!(replies(&outputs), "\r\nOK\r\n\r\nCONNECT 2400\r\n");
        assert_eq!(
            events(&outputs)[1..],
            [
                ModemEvent::Break,
                ModemEvent::Command("ATO".into()),
                ModemEvent::ReturnedOnline
            ]
        );
    }

    #[test]
    fn pluses_inside_data_do_not_escape() {
        let t0 = Instant::now();
        let mut modem = online_modem(t0);
        let _ = modem.handle_input(b"x+++", t0);
        assert_eq!(modem.next_deadline(), None);
        let guard = HayesConfig::default().guard_time;
        let _ = modem.handle_input(b"+++", t0 + guard);
        let _ = modem.handle_input(b"y", t0 + guard + Duration::from_millis(10));
        assert_eq!(modem.next_deadline(), None);
        assert!(modem.is_online());
    }

    #[test]
    fn hangup_after_escape_ends_the_call() {
        let t0 = Instant::now();
        let guard = HayesConfig::default().guard_time;
        let mut modem = online_modem(t0);
        let _ = modem.handle_input(b"+++", t0 + guard);
        let _ = modem.handle_timeout(t0 + guard * 2);
        let outputs = modem.handle_input(b"AT H0\r", t0 + guard * 3);
        assert_eq!(
            events(&outputs),
            [ModemEvent::Command("AT H0".into()), ModemEvent::HungUp]
        );
        let outputs = modem.handle_input(b"ATO\r", t0 + guard * 3);
        assert_eq!(replies(&outputs), "\r\nNO CARRIER\r\n");
    }
}
