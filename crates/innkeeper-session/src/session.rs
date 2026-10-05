use std::collections::VecDeque;
use std::fmt;
use std::time::Instant;

use pad_thai::{
    HayesConfig, Line, LineEvent, LineKind, LineOutput, ModemEvent, PadEvent, Reachable,
};
use tsn_link::{Link, LinkConfig, LinkEvent, LinkOutput, Message};

use crate::SessionError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionConfig {
    pub line: LineKind,
    pub hayes: HayesConfig,
    pub link: LinkConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Line(LineEvent),
    Link(LinkEvent),
    /// A complete application message from the client.
    Message(Message),
    /// The host call ended while the TCP connection stays open; reported once per call.
    LinkLost(LinkLoss),
}

/// What ended a host call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkLoss {
    /// The client cleared the call at the PAD prompt with `D`.
    PadDisconnected,
    /// The emulated modem hung up (`ATH` or `ATZ`) with the call still up.
    ModemHungUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionOutput {
    ToClient(Vec<u8>),
    Event(SessionEvent),
}

#[derive(Debug)]
enum HostLink {
    Offline,
    /// A PAD call is up; a new call replaces the link, which resets both sequence numbers.
    Online(Box<Link>),
}

/// Everything one TCP connection carries, from the first `AT` to framed messages.
#[derive(Debug)]
pub struct Session {
    config: SessionConfig,
    line: Line,
    host: HostLink,
    outputs: VecDeque<SessionOutput>,
}

impl Session {
    pub fn new(config: SessionConfig) -> Self {
        Session {
            config,
            line: Line::new(config.line, config.hayes),
            host: HostLink::Offline,
            outputs: VecDeque::new(),
        }
    }

    /// The same session with calls connecting only where a host answers.
    #[must_use]
    pub fn with_reachable(mut self, reachable: Reachable) -> Self {
        self.line = self.line.with_reachable(reachable);
        self
    }

    pub fn handle_input(&mut self, bytes: &[u8], now: Instant) {
        let line_outputs = self.line.handle_input(bytes, now);
        self.absorb_line(line_outputs, now);
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        let line_outputs = self.line.handle_timeout(now);
        self.absorb_line(line_outputs, now);
        if let HostLink::Online(link) = &mut self.host {
            link.handle_timeout(now);
            self.drain_link();
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        let link_deadline = match &self.host {
            HostLink::Online(link) => link.next_deadline(),
            HostLink::Offline => None,
        };
        [self.line.next_deadline(), link_deadline]
            .into_iter()
            .flatten()
            .min()
    }

    /// Queues a message for the client; it leaves with [`Session::flush`] or once a frame is full.
    pub fn send_message(&mut self, message: &Message, now: Instant) -> Result<(), SessionError> {
        self.online_link()?.send_message(message, now);
        self.drain_link();
        Ok(())
    }

    /// Sends what [`Session::send_message`] queued without waiting for a full frame.
    pub fn flush(&mut self, now: Instant) -> Result<(), SessionError> {
        self.online_link()?.flush(now);
        self.drain_link();
        Ok(())
    }

    /// Whether every message for the client has been acknowledged; true while no call is up.
    pub fn is_transmit_idle(&self) -> bool {
        match &self.host {
            HostLink::Online(link) => link.is_transmit_idle(),
            HostLink::Offline => true,
        }
    }

    pub fn poll_output(&mut self) -> Option<SessionOutput> {
        self.outputs.pop_front()
    }

    pub(crate) fn online_link(&mut self) -> Result<&mut Link, SessionError> {
        match &mut self.host {
            HostLink::Online(link) => Ok(link),
            HostLink::Offline => Err(SessionError::NoHostCall),
        }
    }

    fn absorb_line(&mut self, line_outputs: Vec<LineOutput>, now: Instant) {
        for output in line_outputs {
            match output {
                LineOutput::Reply(bytes) => self.outputs.push_back(SessionOutput::ToClient(bytes)),
                LineOutput::Data(bytes) => self.feed_link(&bytes, now),
                LineOutput::Event(event) => {
                    self.emit(SessionEvent::Line(event.clone()));
                    self.follow_call_state(&event, now);
                }
            }
        }
    }

    fn follow_call_state(&mut self, event: &LineEvent, now: Instant) {
        match event {
            LineEvent::Pad(PadEvent::HostConnected(_)) => {
                self.host = HostLink::Online(Box::new(Link::new(self.config.link, now)));
            }
            LineEvent::Pad(PadEvent::HostDisconnected(_)) => {
                self.lose_link(LinkLoss::PadDisconnected);
            }
            LineEvent::Modem(ModemEvent::HungUp) => self.lose_link(LinkLoss::ModemHungUp),
            LineEvent::Detected(_) | LineEvent::Modem(_) | LineEvent::Pad(_) => {}
        }
    }

    /// Only a call that was up can be lost, so a second hang-up reports nothing.
    fn lose_link(&mut self, loss: LinkLoss) {
        if let HostLink::Online(_) = std::mem::replace(&mut self.host, HostLink::Offline) {
            self.emit(SessionEvent::LinkLost(loss));
        }
    }

    fn feed_link(&mut self, bytes: &[u8], now: Instant) {
        let HostLink::Online(link) = &mut self.host else {
            return;
        };
        let escaped_rest = link.handle_input(bytes, now);
        self.drain_link();
        if let Some(rest) = escaped_rest {
            let line_outputs = self.line.escape_to_pad(rest);
            self.absorb_line(line_outputs, now);
        }
    }

    fn drain_link(&mut self) {
        let HostLink::Online(link) = &mut self.host else {
            return;
        };
        while let Some(output) = link.poll_output() {
            self.outputs.push_back(match output {
                LinkOutput::ToClient(bytes) => SessionOutput::ToClient(bytes),
                LinkOutput::Delivered(message) => {
                    SessionOutput::Event(SessionEvent::Message(message))
                }
                LinkOutput::Event(event) => SessionOutput::Event(SessionEvent::Link(event)),
            });
        }
    }

    fn emit(&mut self, event: SessionEvent) {
        self.outputs.push_back(SessionOutput::Event(event));
    }
}

impl fmt::Display for SessionEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionEvent::Line(event) => event.fmt(f),
            SessionEvent::Link(event) => write!(f, "link: {event}"),
            SessionEvent::Message(message) => write!(f, "message {message}"),
            SessionEvent::LinkLost(loss) => write!(f, "link lost: {loss}"),
        }
    }
}

impl fmt::Display for LinkLoss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LinkLoss::PadDisconnected => "the PAD cleared the call",
            LinkLoss::ModemHungUp => "the modem hung up",
        })
    }
}
