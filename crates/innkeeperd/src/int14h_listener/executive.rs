use std::collections::VecDeque;
use std::mem;

use innkeeper_world::{CallAddress, HostMessage};
use int14h::{
    Call, ConnectResult, ExecStatus, LineRate, LinkStatus, MessageBody, Reply, SharedData,
    SwitchAddress, SwitchHostResult,
};
use tracing::{debug, info, warn};

use crate::host::{Exchange, Host};

/// Driver function 0 of MODEM.DRV, which LSCITV reads as "dial a modem string" (lsci.md section 10).
const MODEM_DRIVER_VERSION: u8 = 3;
/// Host messages a client may leave unreceived before the server ends its session.
pub(crate) const RECEIVE_QUEUE_LIMIT: usize = 1024;

/// The call state as the client sees it through GetStatus, Poll and Receive.
#[derive(Debug)]
enum Line {
    Idle,
    /// From Connect to Disconnect, holding what Receive hands out oldest first.
    Connected(VecDeque<MessageBody>),
    /// The server ended the session; the next Poll or Service reports the carrier lost.
    Dropped,
}

/// TSNEXEC as `innkeeperd` plays it for one transport client: the 17 exports answered against `Host`.
#[derive(Debug)]
pub(crate) struct Executive<'s> {
    host: Host<'s>,
    line: Line,
    line_rate: LineRate,
    shared: SharedData,
}

/// One call's answer and what it caused for the log.
#[derive(Debug)]
pub(crate) struct Handled {
    pub(crate) reply: Reply,
    pub(crate) exchange: Vec<Exchange>,
}

impl<'s> Executive<'s> {
    pub(crate) fn new(host: Host<'s>, line_rate: LineRate) -> Self {
        let Ok(shared) = SharedData::try_new(Vec::new()) else {
            unreachable!("an empty shared block is within its limit");
        };
        Executive {
            host,
            line: Line::Idle,
            line_rate,
            shared,
        }
    }

    pub(crate) fn handle(&mut self, call: Call) -> Handled {
        let mut exchange = Vec::new();
        let reply = match call {
            Call::GetStatus => Reply::GetStatus(ExecStatus {
                driver_version: MODEM_DRIVER_VERSION,
                connected: self.is_connected(),
            }),
            Call::GetSharedData => Reply::GetSharedData(self.shared.clone()),
            Call::SetSharedData(shared) => {
                self.shared = shared;
                Reply::SetSharedData
            }
            Call::Connect(dial) => {
                debug!(dial = dial.as_str(), "connect");
                if !self.is_connected() {
                    self.line = Line::Connected(VecDeque::new());
                }
                Reply::Connect(ConnectResult::Connected)
            }
            Call::Send(body) => Reply::Send {
                queued: self.send(&body, &mut exchange),
            },
            Call::Receive => Reply::Receive(self.receive()),
            Call::Disconnect => {
                self.disconnect();
                Reply::Disconnect
            }
            Call::Poll => Reply::Poll(self.link_status()),
            Call::Service => Reply::Service(self.link_status()),
            Call::IsTransmitIdle => Reply::IsTransmitIdle { idle: true },
            Call::SwitchHost(address) => Reply::SwitchHost(self.switch_host(&address)),
            Call::Flush => Reply::Flush,
            Call::GetLineRate => Reply::GetLineRate(self.line_rate),
            Call::SetAckTimeout(ticks) => {
                debug!(ticks = ticks.0, "ack timeout, kept by the client's clock");
                Reply::SetAckTimeout
            }
            Call::SetNextProgram(name) => {
                debug!(?name, "next program, run by the client");
                Reply::SetNextProgram
            }
            Call::GetPreviousProgram => Reply::GetPreviousProgram(None),
            Call::SetCallbacks { installed } => {
                debug!(installed, "callbacks, held by the client");
                Reply::SetCallbacks
            }
        };
        Handled { reply, exchange }
    }

    /// Queues what another connection caused for this one, while it is connected.
    pub(crate) fn pass_on(&mut self, notice: HostMessage) -> Vec<Exchange> {
        let mut exchange = Vec::new();
        if self.is_connected() {
            self.queue(vec![notice], &mut exchange);
        } else {
            debug!(?notice, "notice for a client without a call dropped");
        }
        exchange
    }

    /// Waits for a message that another connection caused for this one.
    pub(crate) async fn next_notice(&mut self) -> Option<HostMessage> {
        self.host.next_notice().await
    }

    /// The transport closed: whatever the call held is released.
    pub(crate) fn hang_up(&mut self) {
        self.host.end_call();
    }

    fn is_connected(&self) -> bool {
        matches!(self.line, Line::Connected(_))
    }

    fn send(&mut self, body: &MessageBody, exchange: &mut Vec<Exchange>) -> bool {
        if !self.is_connected() || body.as_bytes().is_empty() {
            return false;
        }
        let answer = self.host.answer(body.as_bytes());
        exchange.extend(answer.exchange);
        self.queue(answer.outgoing, exchange);
        true
    }

    fn queue(&mut self, messages: Vec<HostMessage>, exchange: &mut Vec<Exchange>) {
        let Line::Connected(received) = &mut self.line else {
            return;
        };
        for message in messages {
            match MessageBody::try_new(message.encode()) {
                Ok(body) => {
                    received.push_back(body);
                    exchange.push(Exchange::Replied(message));
                }
                Err(error) => exchange.push(Exchange::NotSent(error.into())),
            }
        }
        if received.len() > RECEIVE_QUEUE_LIMIT {
            warn!(
                waiting = received.len(),
                "client stopped receiving, call ended"
            );
            self.host.end_call();
            self.line = Line::Dropped;
        }
    }

    fn receive(&mut self) -> Option<MessageBody> {
        match &mut self.line {
            Line::Connected(received) => received.pop_front(),
            Line::Idle | Line::Dropped => None,
        }
    }

    fn disconnect(&mut self) {
        if self.is_connected() {
            self.host.end_call();
        }
        self.line = Line::Idle;
    }

    fn link_status(&mut self) -> LinkStatus {
        match mem::replace(&mut self.line, Line::Idle) {
            Line::Dropped => LinkStatus::CarrierLost,
            line => {
                self.line = line;
                LinkStatus::Ok
            }
        }
    }

    /// MODEM.DRV's reconnect: the old call is cleared, then the new address is called, or the
    /// driver's default host when the argument names none (link-layer.md 5.1).
    fn switch_host(&mut self, argument: &SwitchAddress) -> SwitchHostResult {
        if !self.is_connected() {
            return SwitchHostResult::NotConnected;
        }
        self.host.end_call();
        let address = CallAddress::from_driver_argument(argument.as_str());
        let new_call = || Line::Connected(VecDeque::new());
        let (result, line) = match address {
            Some(address) if self.host.reaches(&address) => {
                (SwitchHostResult::Switched, new_call())
            }
            Some(address) => {
                warn!(%address, "switch to a host this server does not run");
                (SwitchHostResult::HostUnreachable, Line::Idle)
            }
            None => (SwitchHostResult::Remade, new_call()),
        };
        info!(argument = argument.as_str(), ?result, "switch host");
        self.line = line;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use innkeeper_session::SessionConfig;
    use innkeeper_world::{Sid, World};
    use int14h::{DialString, SwitchAddress};

    use crate::connection::ConnectionSettings;
    use crate::server::tests::{hex, JOIN_PLAYER, LOGIN, LOGIN_ACK};

    fn settings() -> ConnectionSettings {
        ConnectionSettings::new(SessionConfig::default(), World::stock(), None)
    }

    fn idle(settings: &ConnectionSettings) -> Executive<'_> {
        let host = Host::new(settings, settings.new_connection());
        Executive::new(host, LineRate(2400))
    }

    fn connected(settings: &ConnectionSettings) -> Executive<'_> {
        let mut executive = idle(settings);
        let dial = DialString::try_new("ATDT0").unwrap();
        let connected = executive.handle(Call::Connect(dial)).reply;
        assert_eq!(connected, Reply::Connect(ConnectResult::Connected));
        executive
    }

    fn send(executive: &mut Executive<'_>, body: &str) -> Reply {
        let body = MessageBody::try_new(hex(body)).unwrap();
        executive.handle(Call::Send(body)).reply
    }

    fn receive(executive: &mut Executive<'_>) -> Option<Vec<u8>> {
        let Reply::Receive(message) = executive.handle(Call::Receive).reply else {
            panic!("Receive answered something else");
        };
        message.map(|body| body.as_bytes().to_vec())
    }

    fn status(executive: &mut Executive<'_>) -> Reply {
        executive.handle(Call::GetStatus).reply
    }

    fn reports_connected(connected: bool) -> Reply {
        Reply::GetStatus(ExecStatus {
            driver_version: MODEM_DRIVER_VERSION,
            connected,
        })
    }

    #[test]
    fn a_call_runs_from_connect_to_disconnect() {
        let settings = settings();
        let mut executive = connected(&settings);
        assert_eq!(status(&mut executive), reports_connected(true));
        assert_eq!(send(&mut executive, LOGIN), Reply::Send { queued: true });
        assert_eq!(
            send(&mut executive, JOIN_PLAYER),
            Reply::Send { queued: true }
        );
        assert_eq!(receive(&mut executive), Some(hex(LOGIN_ACK)));
        assert!(!settings.objects.lock().unwrap().is_empty());

        assert_eq!(executive.handle(Call::Disconnect).reply, Reply::Disconnect);
        assert!(settings.objects.lock().unwrap().is_empty());
        assert_eq!(status(&mut executive), reports_connected(false));
        assert_eq!(receive(&mut executive), None);
        assert_eq!(send(&mut executive, LOGIN), Reply::Send { queued: false });
    }

    #[test]
    fn an_empty_send_is_refused() {
        let settings = settings();
        let mut executive = connected(&settings);
        let empty = MessageBody::try_new(Vec::new()).unwrap();
        let refused = executive.handle(Call::Send(empty)).reply;
        assert_eq!(refused, Reply::Send { queued: false });
    }

    fn switch_host(executive: &mut Executive<'_>, argument: &str) -> SwitchHostResult {
        let argument = SwitchAddress::try_new(argument).unwrap();
        let Reply::SwitchHost(result) = executive.handle(Call::SwitchHost(argument)).reply else {
            panic!("SwitchHost answered something else");
        };
        result
    }

    /// A logged-in client holding its player object, with the Ack still waiting to be received.
    fn playing(settings: &ConnectionSettings) -> Executive<'_> {
        let mut executive = connected(settings);
        send(&mut executive, LOGIN);
        send(&mut executive, JOIN_PLAYER);
        assert!(!settings.objects.lock().unwrap().is_empty());
        executive
    }

    #[test]
    fn switching_to_this_host_starts_a_new_call_that_logs_in_again() {
        let settings = settings();
        let mut executive = playing(&settings);
        let switched = switch_host(&mut executive, "311083420207");
        assert_eq!(switched, SwitchHostResult::Switched);
        assert!(settings.objects.lock().unwrap().is_empty());
        assert_eq!(receive(&mut executive), None);
        assert_eq!(status(&mut executive), reports_connected(true));

        assert_eq!(
            send(&mut executive, JOIN_PLAYER),
            Reply::Send { queued: true }
        );
        assert_eq!(receive(&mut executive), None, "nothing before the Login");
        send(&mut executive, LOGIN);
        assert_eq!(receive(&mut executive), Some(hex(LOGIN_ACK)));
    }

    #[test]
    fn a_host_this_server_does_not_run_is_unreachable_and_ends_the_call() {
        let settings = settings();
        let mut executive = playing(&settings);
        let refused = switch_host(&mut executive, "311083420208");
        assert_eq!(refused, SwitchHostResult::HostUnreachable);
        assert!(settings.objects.lock().unwrap().is_empty());
        assert_eq!(status(&mut executive), reports_connected(false));
        assert_eq!(
            switch_host(&mut executive, "SIERRA"),
            SwitchHostResult::NotConnected
        );
    }

    #[test]
    fn an_argument_naming_no_address_remakes_the_call_to_the_default_host() {
        let settings = settings();
        let mut executive = playing(&settings);
        assert_eq!(switch_host(&mut executive, ""), SwitchHostResult::Remade);
        assert!(settings.objects.lock().unwrap().is_empty());
        assert_eq!(status(&mut executive), reports_connected(true));
    }

    #[test]
    fn a_client_that_stops_receiving_loses_the_call_once() {
        let settings = settings();
        let mut executive = connected(&settings);
        for _ in 0..=RECEIVE_QUEUE_LIMIT {
            executive.pass_on(HostMessage::ObjectFreed(Sid(0x0100)));
        }
        let lost = executive.handle(Call::Poll).reply;
        assert_eq!(lost, Reply::Poll(LinkStatus::CarrierLost));
        let after = executive.handle(Call::Service).reply;
        assert_eq!(after, Reply::Service(LinkStatus::Ok));
        assert_eq!(status(&mut executive), reports_connected(false));
        assert_eq!(receive(&mut executive), None);
    }

    #[test]
    fn notices_wait_for_a_call() {
        let settings = settings();
        let mut executive = idle(&settings);
        executive.pass_on(HostMessage::ObjectFreed(Sid(0x0100)));
        assert_eq!(receive(&mut executive), None);
    }

    #[test]
    fn executive_local_exports_keep_their_state() {
        let settings = settings();
        let mut executive = connected(&settings);
        let block = SharedData::try_new(vec![7; 256]).unwrap();
        executive.handle(Call::SetSharedData(block.clone()));
        let shared = executive.handle(Call::GetSharedData).reply;
        assert_eq!(shared, Reply::GetSharedData(block));
        let rate = executive.handle(Call::GetLineRate).reply;
        assert_eq!(rate, Reply::GetLineRate(LineRate(2400)));
        let idle = executive.handle(Call::IsTransmitIdle).reply;
        assert_eq!(idle, Reply::IsTransmitIdle { idle: true });
        let previous = executive.handle(Call::GetPreviousProgram).reply;
        assert_eq!(previous, Reply::GetPreviousProgram(None));
    }
}
