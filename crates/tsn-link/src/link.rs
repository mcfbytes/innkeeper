use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use crate::assumptions::PAD_ESCAPE_BYTE_ASSUMED;
use crate::frame::encode_control_frame;
use crate::message::MessageParser;
use crate::packer::FramePacker;
use crate::{encode_frame, Control, Frame, FrameParser, Message, Received, Seq};

/// Host-side timing choices; the client's own values are in link-layer.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkConfig {
    pub resend_after: Duration,
    pub max_resends: u8,
    /// The client discards everything for about 1 s after ` CONNECTED`.
    pub quiet_after_connect: Duration,
}

impl Default for LinkConfig {
    fn default() -> Self {
        LinkConfig {
            resend_after: Duration::from_secs(3),
            max_resends: 10,
            quiet_after_connect: Duration::from_millis(1500),
        }
    }
}

/// Something worth logging that happened on the link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkEvent {
    FrameReceived {
        control: Control,
        payload_len: usize,
    },
    DuplicateData {
        seq: Seq,
    },
    BadCrc {
        ctrl: u8,
    },
    Aborted {
        ctrl: u8,
    },
    Oversized {
        ctrl: u8,
    },
    UnknownControl {
        ctrl: u8,
    },
    Resent {
        seq: Seq,
        attempt: u8,
    },
    GaveUp {
        seq: Seq,
    },
    /// The client left framing for the PAD; see [`Link::handle_input`].
    LineEscape,
}

/// Effects of feeding the link, in the order they happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkOutput {
    ToClient(Vec<u8>),
    Delivered(Message),
    Event(LinkEvent),
}

#[derive(Debug)]
enum Sender {
    Idle,
    AwaitingAck {
        seq: Seq,
        frame: Vec<u8>,
        sent_at: Instant,
        resends: u8,
    },
}

/// The host end of one framed connection: stop-and-wait in both directions, window 1.
#[derive(Debug)]
pub struct Link {
    config: LinkConfig,
    parser: FrameParser,
    messages: MessageParser,
    expected: Seq,
    next_seq: Seq,
    packer: FramePacker,
    sender: Sender,
    quiet_until: Instant,
    outputs: VecDeque<LinkOutput>,
}

impl Link {
    /// Starts a link right after the host connection came up; both sequences begin at 0.
    pub fn new(config: LinkConfig, now: Instant) -> Self {
        Link {
            config,
            parser: FrameParser::new(),
            messages: MessageParser::new(),
            expected: Seq::ZERO,
            next_seq: Seq::ZERO,
            packer: FramePacker::default(),
            sender: Sender::Idle,
            quiet_until: now + config.quiet_after_connect,
            outputs: VecDeque::new(),
        }
    }

    /// Feeds bytes from the client. Returns the bytes after a line escape, which belong to the PAD;
    /// `None` means every byte was link traffic.
    #[must_use]
    pub fn handle_input<'a>(&mut self, bytes: &'a [u8], now: Instant) -> Option<&'a [u8]> {
        for (position, &byte) in bytes.iter().enumerate() {
            if byte == PAD_ESCAPE_BYTE_ASSUMED && self.parser.is_hunting() {
                self.outputs
                    .push_back(LinkOutput::Event(LinkEvent::LineEscape));
                return Some(bytes.get(position + 1..).unwrap_or_default());
            }
            if let Some(received) = self.parser.push(byte) {
                self.handle_received(received, now);
            }
        }
        None
    }

    /// Queues a message for the client; it goes out once [`Link::flush`] or a full frame closes it.
    pub fn send_message(&mut self, message: &Message, now: Instant) {
        self.packer.push_message(message);
        self.send_next(now);
    }

    pub fn flush(&mut self, now: Instant) {
        self.packer.flush();
        self.send_next(now);
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        match &self.sender {
            Sender::AwaitingAck { sent_at, .. } => Some(*sent_at + self.config.resend_after),
            Sender::Idle if self.packer.has_closed_frame() => Some(self.quiet_until),
            Sender::Idle => None,
        }
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        if let Sender::AwaitingAck { sent_at, .. } = &self.sender {
            if now >= *sent_at + self.config.resend_after {
                self.resend_or_give_up(now);
            }
        }
        self.send_next(now);
    }

    pub fn poll_output(&mut self) -> Option<LinkOutput> {
        self.outputs.pop_front()
    }

    fn handle_received(&mut self, received: Received, now: Instant) {
        match received {
            Received::Frame(frame) => self.handle_frame(frame, now),
            Received::BadCrc { ctrl } => {
                self.emit(LinkEvent::BadCrc { ctrl });
                self.nak(ctrl);
            }
            Received::Aborted { ctrl } => {
                self.emit(LinkEvent::Aborted { ctrl });
                self.nak(ctrl);
            }
            Received::Oversized { ctrl } => self.emit(LinkEvent::Oversized { ctrl }),
            Received::UnknownControl { ctrl } => self.emit(LinkEvent::UnknownControl { ctrl }),
        }
    }

    fn handle_frame(&mut self, frame: Frame, now: Instant) {
        let payload_len = frame.payload.len();
        self.emit(LinkEvent::FrameReceived {
            control: frame.control,
            payload_len,
        });
        match frame.control {
            Control::Data(seq) => self.accept_data(seq, &frame.payload, now),
            Control::Ack(seq) => self.handle_ack(seq, now),
            Control::Nak(seq) => self.handle_nak(seq, now),
        }
    }

    fn accept_data(&mut self, seq: Seq, payload: &[u8], now: Instant) {
        self.quiet_until = now;
        self.outputs
            .push_back(LinkOutput::ToClient(encode_control_frame(Control::Ack(
                seq,
            ))));
        if seq != self.expected {
            self.emit(LinkEvent::DuplicateData { seq });
            return;
        }
        self.expected = seq.next();
        for message in self.messages.push(payload) {
            self.outputs.push_back(LinkOutput::Delivered(message));
        }
        self.send_next(now);
    }

    fn handle_ack(&mut self, seq: Seq, now: Instant) {
        if self.outstanding() == Some(seq) {
            self.sender = Sender::Idle;
            self.send_next(now);
        }
    }

    fn handle_nak(&mut self, seq: Seq, now: Instant) {
        if self.outstanding() == Some(seq) {
            self.resend_or_give_up(now);
        }
    }

    fn outstanding(&self) -> Option<Seq> {
        match &self.sender {
            Sender::AwaitingAck { seq, .. } => Some(*seq),
            Sender::Idle => None,
        }
    }

    fn resend_or_give_up(&mut self, now: Instant) {
        let Sender::AwaitingAck {
            seq,
            frame,
            resends,
            ..
        } = std::mem::replace(&mut self.sender, Sender::Idle)
        else {
            return;
        };
        if resends >= self.config.max_resends {
            self.emit(LinkEvent::GaveUp { seq });
            self.send_next(now);
            return;
        }
        let attempt = resends + 1;
        self.emit(LinkEvent::Resent { seq, attempt });
        self.outputs.push_back(LinkOutput::ToClient(frame.clone()));
        self.sender = Sender::AwaitingAck {
            seq,
            frame,
            sent_at: now,
            resends: attempt,
        };
    }

    fn send_next(&mut self, now: Instant) {
        if !matches!(self.sender, Sender::Idle) || now < self.quiet_until {
            return;
        }
        let Some(payload) = self.packer.pop_closed() else {
            return;
        };
        let seq = self.next_seq;
        match encode_frame(Control::Data(seq), &payload) {
            Ok(frame) => {
                self.next_seq = seq.next();
                self.outputs.push_back(LinkOutput::ToClient(frame.clone()));
                self.sender = Sender::AwaitingAck {
                    seq,
                    frame,
                    sent_at: now,
                    resends: 0,
                };
            }
            Err(_) => unreachable!("the packer keeps every payload within the frame limit"),
        }
    }

    fn nak(&mut self, ctrl: u8) {
        if let Some(nak) = Control::nak_for(ctrl) {
            self.outputs
                .push_back(LinkOutput::ToClient(encode_control_frame(nak)));
        }
    }

    fn emit(&mut self, event: LinkEvent) {
        self.outputs.push_back(LinkOutput::Event(event));
    }
}

impl fmt::Display for LinkEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkEvent::FrameReceived {
                control,
                payload_len,
            } => {
                write!(f, "frame {control}, {payload_len} payload bytes")
            }
            LinkEvent::DuplicateData { seq } => write!(f, "duplicate DATA {seq} re-acknowledged"),
            LinkEvent::BadCrc { ctrl } => write!(f, "bad CRC (ctrl {ctrl:#04x})"),
            LinkEvent::Aborted { ctrl } => write!(f, "frame aborted by SOF (ctrl {ctrl:#04x})"),
            LinkEvent::Oversized { ctrl } => write!(f, "oversized frame (ctrl {ctrl:#04x})"),
            LinkEvent::UnknownControl { ctrl } => write!(f, "unknown frame type {ctrl:#04x}"),
            LinkEvent::Resent { seq, attempt } => write!(f, "resent DATA {seq}, attempt {attempt}"),
            LinkEvent::GaveUp { seq } => write!(f, "client never acknowledged DATA {seq}"),
            LinkEvent::LineEscape => write!(f, "CR between frames: escape to the PAD"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(value: u8) -> Seq {
        Seq::try_from(value).unwrap()
    }

    fn data(seq_value: u8, payload: &[u8]) -> Vec<u8> {
        encode_frame(Control::Data(seq(seq_value)), payload).unwrap()
    }

    fn drain(link: &mut Link) -> Vec<LinkOutput> {
        std::iter::from_fn(|| link.poll_output()).collect()
    }

    fn sent(outputs: &[LinkOutput]) -> Vec<Vec<u8>> {
        let to_client = outputs.iter().filter_map(|o| match o {
            LinkOutput::ToClient(bytes) => Some(bytes.clone()),
            _ => None,
        });
        to_client.collect()
    }

    fn delivered(outputs: &[LinkOutput]) -> Vec<Vec<u8>> {
        let messages = outputs.iter().filter_map(|o| match o {
            LinkOutput::Delivered(message) => Some(message.body().to_vec()),
            _ => None,
        });
        messages.collect()
    }

    fn ack(seq_value: u8) -> Vec<u8> {
        encode_control_frame(Control::Ack(seq(seq_value)))
    }

    #[test]
    fn data_is_acknowledged_and_delivered_once() {
        let t0 = Instant::now();
        let mut link = Link::new(LinkConfig::default(), t0);
        let frame = data(0, &[3, 0x22, 0x04, 0x10]);
        assert_eq!(link.handle_input(&frame, t0), None);
        let outputs = drain(&mut link);
        assert_eq!(sent(&outputs), vec![ack(0)]);
        assert_eq!(delivered(&outputs), vec![vec![0x22, 0x04, 0x10]]);

        assert_eq!(link.handle_input(&frame, t0), None);
        let outputs = drain(&mut link);
        assert_eq!(sent(&outputs), vec![ack(0)]);
        assert!(delivered(&outputs).is_empty());
        assert!(outputs.contains(&LinkOutput::Event(LinkEvent::DuplicateData { seq: seq(0) })));
    }

    #[test]
    fn bad_crc_is_negatively_acknowledged() {
        let t0 = Instant::now();
        let mut link = Link::new(LinkConfig::default(), t0);
        let mut frame = data(2, b"xyz");
        frame[1] ^= 0xFF;
        let _ = link.handle_input(&frame, t0);
        assert_eq!(
            sent(&drain(&mut link)),
            vec![encode_control_frame(Control::Nak(seq(2)))]
        );
    }

    #[test]
    fn host_data_waits_for_quiet_period_then_ack() {
        let t0 = Instant::now();
        let config = LinkConfig::default();
        let mut link = Link::new(config, t0);
        link.send_message(&Message::try_new(vec![0x07, 0x01]).unwrap(), t0);
        link.send_message(&Message::try_new(vec![0x08]).unwrap(), t0);
        link.flush(t0);
        assert!(sent(&drain(&mut link)).is_empty());
        assert_eq!(link.next_deadline(), Some(t0 + config.quiet_after_connect));

        let t1 = t0 + config.quiet_after_connect;
        link.handle_timeout(t1);
        assert_eq!(
            sent(&drain(&mut link)),
            vec![data(0, &[2, 0x07, 0x01, 1, 0x08])]
        );

        let _ = link.handle_input(&ack(0), t1);
        assert!(sent(&drain(&mut link)).is_empty());
        assert_eq!(link.next_deadline(), None);
    }

    #[test]
    fn only_one_data_frame_is_outstanding_until_its_ack() {
        let t0 = Instant::now();
        let mut link = Link::new(LinkConfig::default(), t0);
        let after_quiet = t0 + LinkConfig::default().quiet_after_connect;
        for body in [0x07, 0x08] {
            link.send_message(&Message::try_new(vec![body]).unwrap(), after_quiet);
            link.flush(after_quiet);
        }
        assert_eq!(sent(&drain(&mut link)), vec![data(0, &[1, 0x07])]);

        let _ = link.handle_input(&ack(3), after_quiet);
        assert!(sent(&drain(&mut link)).is_empty());
        let _ = link.handle_input(&ack(0), after_quiet);
        assert_eq!(sent(&drain(&mut link)), vec![data(1, &[1, 0x08])]);
    }

    #[test]
    fn unacknowledged_frame_is_resent_then_abandoned() {
        let t0 = Instant::now();
        let config = LinkConfig {
            max_resends: 2,
            ..LinkConfig::default()
        };
        let mut link = Link::new(config, t0);
        let _ = link.handle_input(&data(0, &[]), t0);
        link.send_message(&Message::try_new(vec![0x07]).unwrap(), t0);
        link.flush(t0);
        let first = sent(&drain(&mut link));
        assert_eq!(first, vec![ack(0), data(0, &[1, 0x07])]);

        let _ = link.handle_input(&encode_control_frame(Control::Nak(seq(0))), t0);
        assert_eq!(sent(&drain(&mut link)), vec![data(0, &[1, 0x07])]);
        let t1 = t0 + config.resend_after;
        link.handle_timeout(t1);
        assert_eq!(sent(&drain(&mut link)), vec![data(0, &[1, 0x07])]);
        link.handle_timeout(t1 + config.resend_after);
        let outputs = drain(&mut link);
        assert!(sent(&outputs).is_empty());
        assert!(outputs.contains(&LinkOutput::Event(LinkEvent::GaveUp { seq: seq(0) })));
    }

    #[test]
    fn carriage_return_between_frames_hands_the_rest_to_the_pad() {
        let t0 = Instant::now();
        let mut link = Link::new(LinkConfig::default(), t0);
        let mut input = data(0, &[]);
        input.extend(b"\rSET?");
        assert_eq!(link.handle_input(&input, t0), Some(&b"SET?"[..]));
        assert!(drain(&mut link).contains(&LinkOutput::Event(LinkEvent::LineEscape)));
    }

    #[test]
    fn carriage_return_inside_a_frame_is_payload() {
        let t0 = Instant::now();
        let mut link = Link::new(LinkConfig::default(), t0);
        assert_eq!(link.handle_input(&data(0, &[1, b'\r']), t0), None);
        assert_eq!(delivered(&drain(&mut link)), vec![vec![b'\r']]);
    }
}
