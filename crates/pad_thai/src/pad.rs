use std::fmt;

use crate::assumptions::{
    BARE_RETURNS_BEFORE_PROMPT_ASSUMED, FRAMING_BIT_ASSUMED, PAD_LINE_BREAK_ASSUMED,
    PAD_PROMPT_ASSUMED, TERMINAL_PROMPT_ASSUMED, UNREACHABLE_CALL_WORD_ASSUMED,
};
use crate::typed_line::{TypedLine, CR, SEVEN_BITS};
use crate::{HostAddress, Reachable};

const MAX_COMMAND_LEN: usize = 127;
/// The driver waits for exactly these words, the leading space included (`MODEM.DRV:0E94`, `0FC0`).
const CONNECTED_WORD: &str = " CONNECTED";
const DISCONNECTED_WORD: &str = " DISCONNECTED";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PadEvent {
    Command(String),
    HostConnected(HostAddress),
    /// No host answers at the address; the line stays at the prompt.
    HostUnreachable(HostAddress),
    HostDisconnected(HostAddress),
    /// A BREAK or link-level escape returned the line to the PAD prompt.
    Escaped,
    UnknownCommand(String),
    InvalidAddress(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PadOutput {
    Reply(Vec<u8>),
    /// Bytes for the host once a call is up.
    Data(Vec<u8>),
    Event(PadEvent),
}

#[derive(Debug)]
enum PadState {
    Wakeup {
        terminal_requested: bool,
        bare_returns: u8,
    },
    TerminalPrompt,
    Command {
        line: TypedLine,
        call: Option<HostAddress>,
    },
    DataTransfer {
        host: HostAddress,
    },
}

impl PadState {
    /// The call that stays up while the line sits at the prompt.
    fn call(&self) -> Option<&HostAddress> {
        match self {
            PadState::DataTransfer { host } => Some(host),
            PadState::Command { call, .. } => call.as_ref(),
            PadState::Wakeup { .. } | PadState::TerminalPrompt => None,
        }
    }

    fn prompt(call: Option<HostAddress>) -> Self {
        PadState::Command {
            line: TypedLine::new(MAX_COMMAND_LEN),
            call,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PadCommand<'a> {
    Call(&'a str),
    Disconnect,
    SetParameters,
    Unknown,
}

/// A SprintNet asynchronous PAD as far as the INN com driver tests it, sans-IO.
#[derive(Debug)]
pub(crate) struct Pad {
    state: PadState,
    reachable: Reachable,
}

impl Pad {
    pub(crate) fn new(reachable: Reachable) -> Self {
        Pad {
            state: PadState::Wakeup {
                terminal_requested: false,
                bare_returns: 0,
            },
            reachable,
        }
    }

    /// A new modem call starts the dialogue again; the hosts the network reaches stay.
    pub(crate) fn restart(&mut self) {
        *self = Pad::new(std::mem::take(&mut self.reachable));
    }

    pub(crate) fn is_transferring(&self) -> bool {
        matches!(self.state, PadState::DataTransfer { .. })
    }

    #[must_use]
    pub(crate) fn handle_input(&mut self, bytes: &[u8]) -> Vec<PadOutput> {
        let mut outputs = Vec::new();
        for (position, &byte) in bytes.iter().enumerate() {
            self.detect_direct_framing(byte, &mut outputs);
            if self.is_transferring() {
                let data = bytes.get(position..).unwrap_or_default();
                outputs.push(PadOutput::Data(data.to_vec()));
                break;
            }
            self.dialogue_byte(byte & SEVEN_BITS, &mut outputs);
        }
        outputs
    }

    /// A BREAK from the line: leave data transfer for the prompt; the call stays up until `D`.
    #[must_use]
    pub(crate) fn escape(&mut self) -> Vec<PadOutput> {
        self.state = PadState::prompt(self.state.call().cloned());
        vec![PadOutput::Event(PadEvent::Escaped), prompt()]
    }

    fn detect_direct_framing(&mut self, byte: u8, outputs: &mut Vec<PadOutput>) {
        let waking = matches!(self.state, PadState::Wakeup { .. });
        if waking && byte & FRAMING_BIT_ASSUMED != 0 {
            let host = HostAddress::direct();
            outputs.push(PadOutput::Event(PadEvent::HostConnected(host.clone())));
            self.state = PadState::DataTransfer { host };
        }
    }

    fn dialogue_byte(&mut self, byte: u8, outputs: &mut Vec<PadOutput>) {
        match &mut self.state {
            PadState::Wakeup {
                terminal_requested,
                bare_returns,
            } => match byte {
                CR if *terminal_requested => {
                    outputs.push(reply(TERMINAL_PROMPT_ASSUMED));
                    self.state = PadState::TerminalPrompt;
                }
                CR if *bare_returns + 1 >= BARE_RETURNS_BEFORE_PROMPT_ASSUMED => {
                    self.enter_command(None, outputs);
                }
                CR => *bare_returns += 1,
                b'D' | b'd' => *terminal_requested = true,
                _ => {}
            },
            PadState::TerminalPrompt if byte == CR => self.enter_command(None, outputs),
            PadState::TerminalPrompt => {}
            PadState::Command { line, call } => {
                if let Some(text) = line.push(byte) {
                    let call = call.take();
                    self.execute(text.trim(), call, outputs);
                }
            }
            PadState::DataTransfer { .. } => {}
        }
    }

    fn execute(&mut self, text: &str, call: Option<HostAddress>, outputs: &mut Vec<PadOutput>) {
        if text.is_empty() {
            self.enter_command(call, outputs);
            return;
        }
        outputs.push(PadOutput::Event(PadEvent::Command(text.to_owned())));
        match parse_pad_command(text) {
            PadCommand::Call(address) => match HostAddress::try_new(address) {
                Ok(host) if self.reachable.admits(&host) => self.connect(host, outputs),
                Ok(host) => {
                    let line =
                        format!("{PAD_LINE_BREAK_ASSUMED}{host}{UNREACHABLE_CALL_WORD_ASSUMED}");
                    outputs.push(reply(&line));
                    outputs.push(PadOutput::Event(PadEvent::HostUnreachable(host)));
                    self.enter_command(call, outputs);
                }
                Err(_) => {
                    outputs.push(PadOutput::Event(PadEvent::InvalidAddress(
                        address.to_owned(),
                    )));
                    self.enter_command(call, outputs);
                }
            },
            PadCommand::Disconnect => match call {
                Some(host) => {
                    let line = format!("{PAD_LINE_BREAK_ASSUMED}{host}{DISCONNECTED_WORD}");
                    outputs.push(reply(&line));
                    outputs.push(PadOutput::Event(PadEvent::HostDisconnected(host)));
                    self.enter_command(None, outputs);
                }
                None => self.enter_command(None, outputs),
            },
            PadCommand::SetParameters => self.enter_command(call, outputs),
            PadCommand::Unknown => {
                outputs.push(PadOutput::Event(PadEvent::UnknownCommand(text.to_owned())));
                self.enter_command(call, outputs);
            }
        }
    }

    fn connect(&mut self, host: HostAddress, outputs: &mut Vec<PadOutput>) {
        let line =
            format!("{PAD_LINE_BREAK_ASSUMED}{host}{CONNECTED_WORD}{PAD_LINE_BREAK_ASSUMED}");
        outputs.push(reply(&line));
        outputs.push(PadOutput::Event(PadEvent::HostConnected(host.clone())));
        self.state = PadState::DataTransfer { host };
    }

    fn enter_command(&mut self, call: Option<HostAddress>, outputs: &mut Vec<PadOutput>) {
        outputs.push(prompt());
        self.state = PadState::prompt(call);
    }
}

fn parse_pad_command(text: &str) -> PadCommand<'_> {
    let (word, argument) = text.split_once(' ').unwrap_or((text, ""));
    let word = word.to_ascii_uppercase();
    match word.as_str() {
        "C" if !argument.trim().is_empty() => PadCommand::Call(argument.trim()),
        "D" if argument.trim().is_empty() => PadCommand::Disconnect,
        _ if word.starts_with("SET") || word.starts_with("PAR") => PadCommand::SetParameters,
        _ => PadCommand::Unknown,
    }
}

fn reply(text: &str) -> PadOutput {
    PadOutput::Reply(text.as_bytes().to_vec())
}

fn prompt() -> PadOutput {
    reply(PAD_PROMPT_ASSUMED)
}

impl fmt::Display for PadEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PadEvent::Command(text) => write!(f, "PAD command {text:?}"),
            PadEvent::HostConnected(host) => write!(f, "call to {host} connected"),
            PadEvent::HostUnreachable(host) => write!(f, "call to {host} cleared: no host there"),
            PadEvent::HostDisconnected(host) => write!(f, "call to {host} cleared"),
            PadEvent::Escaped => write!(f, "escaped to the PAD prompt"),
            PadEvent::UnknownCommand(text) => write!(f, "unknown PAD command {text:?}"),
            PadEvent::InvalidAddress(text) => write!(f, "invalid call address {text:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replies(outputs: &[PadOutput]) -> String {
        let text = outputs.iter().filter_map(|o| match o {
            PadOutput::Reply(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        });
        text.collect()
    }

    fn sierra() -> HostAddress {
        HostAddress::try_new("SIERRA").unwrap()
    }

    #[test]
    fn wake_up_above_1200_bps() {
        let mut pad = Pad::new(Reachable::Any);
        assert_eq!(replies(&pad.handle_input(b"@D")), "");
        assert_eq!(replies(&pad.handle_input(b"\r")), "\r\nTERMINAL=");
        assert_eq!(replies(&pad.handle_input(b"\r")), "\r\n@");
        assert_eq!(replies(&pad.handle_input(b"\r")), "\r\n@");
    }

    #[test]
    fn wake_up_at_1200_bps_starts_with_a_return() {
        let mut pad = Pad::new(Reachable::Any);
        assert_eq!(replies(&pad.handle_input(b"\rD\r\r")), "\r\nTERMINAL=\r\n@");
    }

    #[test]
    fn call_connects_and_later_bytes_are_data() {
        let mut pad = Pad::new(Reachable::Any);
        let _ = pad.handle_input(b"@D\r\r");
        let outputs = pad.handle_input(b"c SIERRA\r\x81");
        assert_eq!(replies(&outputs), "\r\nSIERRA CONNECTED\r\n");
        assert!(outputs.contains(&PadOutput::Event(PadEvent::HostConnected(sierra()))));
        assert_eq!(outputs.last(), Some(&PadOutput::Data(vec![0x81])));
    }

    #[test]
    fn land_switch_dialogue() {
        let mut pad = Pad::new(Reachable::Any);
        let _ = pad.handle_input(b"@D\r\rc SIERRA\r");
        assert_eq!(replies(&pad.escape()), "\r\n@");
        assert_eq!(replies(&pad.handle_input(b"SET? 0:0,32:0\r")), "\r\n@");
        let outputs = pad.handle_input(b"D\r");
        assert_eq!(replies(&outputs), "\r\nSIERRA DISCONNECTED\r\n@");
        assert!(outputs.contains(&PadOutput::Event(PadEvent::HostDisconnected(sierra()))));
        assert_eq!(
            replies(&pad.handle_input(b"c 83420207\r")),
            "\r\n83420207 CONNECTED\r\n"
        );
        assert!(pad.is_transferring());
    }

    #[test]
    fn a_call_no_host_takes_is_cleared_and_the_default_call_still_connects() {
        let mut pad = Pad::new(Reachable::Only(vec![sierra()]));
        let _ = pad.handle_input(b"@D\r\r");
        let unreachable = HostAddress::try_new("83420208").unwrap();
        let outputs = pad.handle_input(b"c 83420208\r");
        assert_eq!(replies(&outputs), "\r\n83420208 DISCONNECTED\r\n@");
        assert!(outputs.contains(&PadOutput::Event(PadEvent::HostUnreachable(unreachable))));
        assert!(!pad.is_transferring());
        let outputs = pad.handle_input(b"c SIERRA\r");
        assert_eq!(replies(&outputs), "\r\nSIERRA CONNECTED\r\n");
    }

    #[test]
    fn framing_during_wake_up_means_direct() {
        let mut pad = Pad::new(Reachable::Any);
        let outputs = pad.handle_input(&[0x81, 0x49]);
        let direct = PadEvent::HostConnected(HostAddress::direct());
        assert_eq!(
            outputs,
            vec![PadOutput::Event(direct), PadOutput::Data(vec![0x81, 0x49])]
        );
    }

    #[test]
    fn commands_are_classified() {
        assert_eq!(parse_pad_command("c SIERRA"), PadCommand::Call("SIERRA"));
        assert_eq!(parse_pad_command("D"), PadCommand::Disconnect);
        assert_eq!(
            parse_pad_command("SET? 0:0,32:0"),
            PadCommand::SetParameters
        );
        assert_eq!(parse_pad_command("c"), PadCommand::Unknown);
        assert_eq!(parse_pad_command("HELLO"), PadCommand::Unknown);
    }
}
