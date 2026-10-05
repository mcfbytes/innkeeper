use crate::crc::Crc16;
use crate::frame::{EOF, ESC, MAX_ESCAPED_PAYLOAD, SOF};
use crate::{Control, Frame};

/// What the receiver makes of a complete or abandoned frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Received {
    Frame(Frame),
    /// Good CRC but a type the client never sends; the client ignores these silently.
    UnknownControl {
        ctrl: u8,
    },
    BadCrc {
        ctrl: u8,
    },
    /// An unescaped SOF arrived inside the frame; it starts the next frame.
    Aborted {
        ctrl: u8,
    },
    /// More payload than any client frame can carry; the parser resynchronises.
    Oversized {
        ctrl: u8,
    },
}

#[derive(Debug)]
struct OpenFrame {
    expected: Crc16,
    running: Crc16,
    ctrl: u8,
    payload: Vec<u8>,
    escaped_len: usize,
}

#[derive(Debug, Default)]
enum State {
    #[default]
    Hunt,
    CrcLow,
    CrcHigh {
        low: u8,
    },
    Control {
        expected: Crc16,
    },
    Body(OpenFrame),
    Escape(OpenFrame),
}

/// Receiver state machine of TSNEXEC (`0000:1E7B`): byte in, frame verdict out.
#[derive(Debug, Default)]
pub struct FrameParser {
    state: State,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// True between frames, where the client never sends anything but SOF.
    pub fn is_hunting(&self) -> bool {
        matches!(self.state, State::Hunt)
    }

    pub fn push(&mut self, byte: u8) -> Option<Received> {
        let (next, verdict) = match std::mem::take(&mut self.state) {
            State::Hunt if byte == SOF => (State::CrcLow, None),
            State::Hunt => (State::Hunt, None),
            State::CrcLow => (State::CrcHigh { low: byte }, None),
            State::CrcHigh { low } => {
                let expected = Crc16::from_le_bytes([low, byte]);
                (State::Control { expected }, None)
            }
            State::Control { expected } => (State::Body(OpenFrame::new(expected, byte)), None),
            State::Body(open) => open.push_body(byte),
            State::Escape(open) => open.push_payload(byte),
        };
        self.state = next;
        verdict
    }
}

impl OpenFrame {
    fn new(expected: Crc16, ctrl: u8) -> Self {
        OpenFrame {
            expected,
            running: Crc16::new().update(ctrl),
            ctrl,
            payload: Vec::new(),
            escaped_len: 0,
        }
    }

    fn push_body(mut self, byte: u8) -> (State, Option<Received>) {
        match byte {
            SOF => (State::CrcLow, Some(Received::Aborted { ctrl: self.ctrl })),
            EOF => (State::Hunt, Some(self.finish())),
            ESC => {
                self.count(byte);
                (State::Escape(self), None)
            }
            _ => self.push_payload(byte),
        }
    }

    fn push_payload(mut self, byte: u8) -> (State, Option<Received>) {
        self.count(byte);
        if self.escaped_len > MAX_ESCAPED_PAYLOAD {
            return (State::Hunt, Some(Received::Oversized { ctrl: self.ctrl }));
        }
        self.payload.push(byte);
        (State::Body(self), None)
    }

    fn count(&mut self, byte: u8) {
        self.running = self.running.update(byte);
        self.escaped_len += 1;
    }

    fn finish(self) -> Received {
        if self.running != self.expected {
            return Received::BadCrc { ctrl: self.ctrl };
        }
        match Control::try_from(self.ctrl) {
            Ok(control) => Received::Frame(Frame {
                control,
                payload: self.payload,
            }),
            Err(_) => Received::UnknownControl { ctrl: self.ctrl },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encode_frame, Seq};

    fn parse_all(parser: &mut FrameParser, bytes: &[u8]) -> Vec<Received> {
        bytes.iter().filter_map(|&byte| parser.push(byte)).collect()
    }

    #[test]
    fn decodes_what_the_encoder_writes() {
        let payload = [0x05, SOF, EOF, ESC, 0x00, 0xFF];
        let wire = encode_frame(Control::Data(Seq::ZERO), &payload).unwrap();
        let mut parser = FrameParser::new();
        let frame = Frame {
            control: Control::Data(Seq::ZERO),
            payload: payload.to_vec(),
        };
        assert_eq!(parse_all(&mut parser, &wire), vec![Received::Frame(frame)]);
        assert!(parser.is_hunting());
    }

    #[test]
    fn noise_between_frames_is_discarded() {
        let mut parser = FrameParser::new();
        assert!(parse_all(&mut parser, b"\x00garbage\x82").is_empty());
        assert!(parser.is_hunting());
    }

    #[test]
    fn corrupted_payload_is_reported_with_its_control_byte() {
        let mut wire = encode_frame(Control::Data(Seq::ZERO), b"abc").unwrap();
        wire[5] ^= 0x01;
        let mut parser = FrameParser::new();
        assert_eq!(
            parse_all(&mut parser, &wire),
            vec![Received::BadCrc { ctrl: 0x00 }]
        );
    }

    #[test]
    fn sof_inside_a_frame_restarts_with_the_next_frame() {
        let good = encode_frame(Control::Data(Seq::ZERO), b"ok").unwrap();
        let mut wire = vec![SOF, 0x12, 0x34, 0x03, 0x41];
        wire.extend(&good);
        let mut parser = FrameParser::new();
        let verdicts = parse_all(&mut parser, &wire);
        assert_eq!(verdicts[0], Received::Aborted { ctrl: 0x03 });
        assert!(matches!(verdicts[1], Received::Frame(_)));
    }

    #[test]
    fn positional_bytes_may_look_like_framing() {
        let nak_one = [SOF, 0x59, 0x60, 0x81, EOF];
        let mut parser = FrameParser::new();
        let frame = Frame {
            control: Control::Nak(Seq::try_from(1).unwrap()),
            payload: vec![],
        };
        assert_eq!(
            parse_all(&mut parser, &nak_one),
            vec![Received::Frame(frame)]
        );
    }

    #[test]
    fn overlong_frame_is_abandoned() {
        let mut wire = vec![SOF, 0x00, 0x00, 0x00];
        wire.extend([0x41; MAX_ESCAPED_PAYLOAD + 1]);
        let mut parser = FrameParser::new();
        assert_eq!(
            parse_all(&mut parser, &wire),
            vec![Received::Oversized { ctrl: 0x00 }]
        );
        assert!(parser.is_hunting());
    }
}
