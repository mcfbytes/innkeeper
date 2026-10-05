use std::fmt;
use std::time::Instant;

use crate::hayes_fever::{HayesConfig, HayesModem, ModemEvent, ModemOutput};
use crate::pad::{Pad, PadOutput};
use crate::{PadEvent, Reachable};

/// What sits between the client's UART and us.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineKind {
    /// Decide from the first bytes: `AT` or `+` means a raw line, anything else a modem emulator.
    #[default]
    Auto,
    /// Raw serial or null-modem: we play the Hayes modem as well as the PAD.
    Hayes,
    /// The client side already emulates the modem (DOSBox): the PAD dialogue starts at once.
    Pad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineEvent {
    /// Auto-detection settled on [`LineKind::Hayes`] or [`LineKind::Pad`].
    Detected(LineKind),
    Modem(ModemEvent),
    Pad(PadEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineOutput {
    Reply(Vec<u8>),
    /// Bytes for the host link once a call is up.
    Data(Vec<u8>),
    Event(LineEvent),
}

#[derive(Debug)]
enum Front {
    Detecting { held: Vec<u8> },
    Hayes(HayesModem),
    Bare,
}

/// Everything SprintNet did for a caller: the optional modem, then the PAD.
#[derive(Debug)]
pub struct Line {
    hayes: HayesConfig,
    front: Front,
    pad: Pad,
}

impl Front {
    fn for_kind(kind: LineKind, hayes: HayesConfig) -> Self {
        match kind {
            LineKind::Auto => Front::Detecting { held: Vec::new() },
            LineKind::Hayes => Front::Hayes(HayesModem::new(hayes)),
            LineKind::Pad => Front::Bare,
        }
    }
}

impl Line {
    pub fn new(kind: LineKind, hayes: HayesConfig) -> Self {
        Line {
            hayes,
            front: Front::for_kind(kind, hayes),
            pad: Pad::new(Reachable::Any),
        }
    }

    /// The same line with calls connecting only where a host answers.
    #[must_use]
    pub fn with_reachable(mut self, reachable: Reachable) -> Self {
        self.pad = Pad::new(reachable);
        self
    }

    #[must_use]
    pub fn handle_input(&mut self, bytes: &[u8], now: Instant) -> Vec<LineOutput> {
        match &mut self.front {
            Front::Detecting { held } => {
                let Some(&first) = bytes.iter().find(|&&byte| !is_line_noise(byte)) else {
                    held.extend(bytes);
                    return Vec::new();
                };
                let mut replay = std::mem::take(held);
                replay.extend(bytes);
                let kind = if starts_hayes_command(first) {
                    LineKind::Hayes
                } else {
                    LineKind::Pad
                };
                self.front = Front::for_kind(kind, self.hayes);
                let mut outputs = vec![LineOutput::Event(LineEvent::Detected(kind))];
                outputs.extend(self.handle_input(&replay, now));
                outputs
            }
            Front::Hayes(modem) => {
                let modem_outputs = modem.handle_input(bytes, now);
                self.absorb_modem(modem_outputs)
            }
            Front::Bare => self.feed_pad(bytes),
        }
    }

    /// The link saw the client leave framing; `rest` already passed the modem and goes to the PAD.
    #[must_use]
    pub fn escape_to_pad(&mut self, rest: &[u8]) -> Vec<LineOutput> {
        let mut outputs = convert_pad(self.pad.escape());
        outputs.extend(self.feed_pad(rest));
        outputs
    }

    #[must_use]
    pub fn handle_timeout(&mut self, now: Instant) -> Vec<LineOutput> {
        match &mut self.front {
            Front::Hayes(modem) => {
                let modem_outputs = modem.handle_timeout(now);
                self.absorb_modem(modem_outputs)
            }
            Front::Detecting { .. } | Front::Bare => Vec::new(),
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        match &self.front {
            Front::Hayes(modem) => modem.next_deadline(),
            Front::Detecting { .. } | Front::Bare => None,
        }
    }

    fn absorb_modem(&mut self, modem_outputs: Vec<ModemOutput>) -> Vec<LineOutput> {
        let mut outputs = Vec::new();
        for output in modem_outputs {
            match output {
                ModemOutput::Reply(bytes) => outputs.push(LineOutput::Reply(bytes)),
                ModemOutput::Data(bytes) => outputs.extend(self.feed_pad(&bytes)),
                ModemOutput::Event(event) => {
                    let pad_reaction = match event {
                        ModemEvent::Dialled { .. } | ModemEvent::HungUp => {
                            self.pad.restart();
                            Vec::new()
                        }
                        ModemEvent::Break => self.pad.escape(),
                        ModemEvent::Command(_)
                        | ModemEvent::Escaped
                        | ModemEvent::ReturnedOnline => Vec::new(),
                    };
                    outputs.push(LineOutput::Event(LineEvent::Modem(event)));
                    outputs.extend(convert_pad(pad_reaction));
                }
            }
        }
        outputs
    }

    fn feed_pad(&mut self, bytes: &[u8]) -> Vec<LineOutput> {
        convert_pad(self.pad.handle_input(bytes))
    }
}

fn is_line_noise(byte: u8) -> bool {
    matches!(byte, b'\r' | b'\n' | b' ')
}

fn starts_hayes_command(byte: u8) -> bool {
    matches!(byte, b'A' | b'a' | b'+')
}

fn convert_pad(outputs: Vec<PadOutput>) -> Vec<LineOutput> {
    let convert = |output| match output {
        PadOutput::Reply(bytes) => LineOutput::Reply(bytes),
        PadOutput::Data(bytes) => LineOutput::Data(bytes),
        PadOutput::Event(event) => LineOutput::Event(LineEvent::Pad(event)),
    };
    outputs.into_iter().map(convert).collect()
}

impl fmt::Display for LineEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineEvent::Detected(kind) => write!(f, "line detected as {kind:?}"),
            LineEvent::Modem(event) => write!(f, "modem: {event}"),
            LineEvent::Pad(event) => write!(f, "PAD: {event}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replies(outputs: &[LineOutput]) -> String {
        let text = outputs.iter().filter_map(|o| match o {
            LineOutput::Reply(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        });
        text.collect()
    }

    #[test]
    fn leading_return_then_at_selects_the_modem() {
        let mut line = Line::new(LineKind::Auto, HayesConfig::default());
        let now = Instant::now();
        assert!(line.handle_input(b"\r", now).is_empty());
        let outputs = line.handle_input(b"AT\r", now);
        assert_eq!(
            outputs[0],
            LineOutput::Event(LineEvent::Detected(LineKind::Hayes))
        );
        assert_eq!(replies(&outputs), "\rAT\r\r\nOK\r\n");
    }

    #[test]
    fn pad_wake_up_selects_the_bare_pad() {
        let mut line = Line::new(LineKind::Auto, HayesConfig::default());
        let outputs = line.handle_input(b"@D\r", Instant::now());
        assert_eq!(
            outputs[0],
            LineOutput::Event(LineEvent::Detected(LineKind::Pad))
        );
        assert_eq!(replies(&outputs), "\r\nTERMINAL=");
    }

    #[test]
    fn modem_break_prompts_at_the_pad() {
        let mut line = Line::new(LineKind::Hayes, HayesConfig::default());
        let now = Instant::now();
        let _ = line.handle_input(b"ATE0\rATDT1\r@D\r\rc SIERRA\r", now);
        let guard = HayesConfig::default().guard_time;
        let _ = line.handle_input(b"+++", now + guard);
        let _ = line.handle_timeout(now + guard * 2);
        let outputs = line.handle_input(b"AT\\B\r", now + guard * 3);
        assert_eq!(replies(&outputs), "\r\nOK\r\n\r\n@");
    }
}
