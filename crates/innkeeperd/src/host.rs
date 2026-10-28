use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use innkeeper_session::SessionError;
use innkeeper_world::{
    AccountId, ClientMessage, ConnectionId, Delivery, HostMessage, LogPolicy, LoginNakReason,
    MessageError, Nak, ObjectStore, PlayerSession, World,
};
use int14h::Int14hError;
use thiserror::Error;
use tokio::sync::watch;
use tracing::warn;
use tsn_link::LinkError;

use crate::connection::ConnectionSettings;
use crate::connection_gate::Binding;
use crate::limits::Limits;
use crate::metrics::trace::{self, Call, CallResult, LineTrace, Stopwatch};
use crate::signin::{LoginGate, SignIn};
use crate::switchboard::{Inbox, Switchboard};

/// The reply `logon` gives a wrong password, so a refused sign-in looks like one to the client.
const WRONG_PASSWORD_TRIES: u8 = 3;
const WRONG_PASSWORD_TEXT: &str = "That password is not right.";

/// The application end of one connection, whatever its transport: decodes client messages, returns
/// the world's replies and what other connections caused, and releases the connection's objects.
#[derive(Debug)]
pub(crate) struct Host<'s> {
    world: &'s World,
    objects: &'s Mutex<ObjectStore>,
    switchboard: &'s Switchboard,
    limits: &'s Limits,
    connection: ConnectionId,
    inbox: Inbox<'s>,
    player: PlayerSession,
    logon: Option<LogonSignal>,
    audit: Option<crate::audit::AuditTicket>,
    sign_in: Option<&'s SignIn>,
    signup_open: bool,
    binding: Option<Binding>,
    noted_account: Option<AccountId>,
    /// The account this line has been counted as a player for (the metrics pusher and the portal).
    counted_account: Option<AccountId>,
    trace: Option<Arc<LineTrace>>,
}

/// Tells the connection's other tasks which account its line session is signed on as.
pub(crate) type LogonSignal = watch::Sender<Option<AccountId>>;

/// One step of a message exchange, for the log and the capture file.
#[derive(Debug)]
pub(crate) enum Exchange {
    Received(ClientMessage),
    Undecodable(MessageError),
    Replied(HostMessage),
    NotSent(ReplyError),
}

/// Why a host message did not reach the client, on either transport.
#[derive(Debug, Error)]
pub(crate) enum ReplyError {
    #[error(transparent)]
    Link(#[from] LinkError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Transport(#[from] Int14hError),
}

/// What one client message caused: how it was read, and the host messages this connection is owed.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) exchange: Vec<Exchange>,
    pub(crate) outgoing: Vec<HostMessage>,
    /// On a traced line: the call to report once the replies are sent ([`Host::finish_call`]).
    pub(crate) call: Option<Call>,
}

fn answer_of(exchange: Exchange, outgoing: Vec<HostMessage>) -> Answer {
    Answer {
        exchange: vec![exchange],
        outgoing,
        call: None,
    }
}

impl<'s> Host<'s> {
    pub(crate) fn new(settings: &'s ConnectionSettings, connection: ConnectionId) -> Self {
        Host {
            world: &settings.world,
            objects: &settings.objects,
            switchboard: &settings.switchboard,
            limits: &settings.limits,
            connection,
            inbox: settings.switchboard.register(connection),
            player: PlayerSession::with_log(settings.world.log),
            logon: None,
            audit: None,
            sign_in: settings.sign_in.as_deref(),
            signup_open: settings.signup_open,
            binding: None,
            noted_account: None,
            counted_account: None,
            trace: None,
        }
    }

    /// Reports every client message of this line to its diagnostics trace from now on.
    pub(crate) fn trace(&mut self, trace: Option<Arc<LineTrace>>) {
        self.trace = trace;
    }

    /// Limits this line's Logins to the accounts the signed-in identity owns.
    pub(crate) fn bind(&mut self, binding: Option<Binding>) {
        self.binding = binding;
    }

    /// Keeps the sealed connection record for this line, when a recipient is configured.
    pub(crate) fn watch_audit(&mut self, ticket: Option<crate::audit::AuditTicket>) {
        self.audit = ticket;
    }

    pub(crate) fn log_policy(&self) -> LogPolicy {
        self.world.log
    }

    /// Publishes the signed-on account (or `None` after a hang-up) on `signal` from now on.
    pub(crate) fn announce_logon(&mut self, signal: LogonSignal) {
        self.logon = Some(signal);
    }

    fn publish_logon(&self) {
        if let Some(signal) = &self.logon {
            let account = self.player.account();
            signal.send_if_modified(|shown| {
                let changed = *shown != account;
                *shown = account;
                changed
            });
        }
    }

    /// Answers one client message; on a traced line the answer also carries the call to report.
    pub(crate) fn answer(&mut self, body: &[u8]) -> Answer {
        let clock = self.trace.is_some().then(Stopwatch::start);
        let (mut answer, name, result) = self.answer_message(body);
        answer.call = clock.map(|clock| Call::land(name, self.land_target(), clock, result));
        answer
    }

    fn answer_message(&mut self, body: &[u8]) -> (Answer, &'static str, CallResult) {
        crate::metrics::counters().message_in();
        let received = match ClientMessage::parse(body) {
            Ok(received) => received,
            Err(error) => {
                let answer = answer_of(Exchange::Undecodable(error), Vec::new());
                return (answer, trace::UNDECODABLE, CallResult::Undecodable);
            }
        };
        let name = received.kind();
        if let (Some(trace), ClientMessage::Login(login)) = (&self.trace, &received) {
            trace.note_client_version(login.version);
        }
        if let Some(outgoing) = self.refusal(&received) {
            let answer = answer_of(Exchange::Received(received), outgoing);
            return (answer, name, CallResult::Refused);
        }
        let mut objects = lock(self.objects);
        // Under the store lock the inbox holds everything caused before this message, in order.
        let mut outgoing = self.inbox.drain();
        let caused = self
            .player
            .handle(self.world, &mut objects, self.connection, &received);
        let own = self.dispatch(caused);
        let result = match own.iter().any(|m| matches!(m, HostMessage::Nak(_))) {
            true => CallResult::Nak,
            false => CallResult::Ok,
        };
        outgoing.extend(own);
        drop(objects);
        self.publish_logon();
        self.note_audit();
        self.note_sign_in();
        self.count_login();
        (
            answer_of(Exchange::Received(received), outgoing),
            name,
            result,
        )
    }

    /// Reports a traced call once its replies went out: one that could not be sent is `not-sent`.
    pub(crate) fn finish_call(&self, call: Option<Call>, sent: &[Exchange]) {
        let (Some(trace), Some(mut call)) = (&self.trace, call) else {
            return;
        };
        let unsent = sent.iter().any(|step| matches!(step, Exchange::NotSent(_)));
        if unsent && call.outcome.success {
            call.outcome = CallResult::NotSent.outcome();
        }
        trace.record(call);
    }

    /// The game this connection plays, when its name is known, else its land.
    fn land_target(&self) -> String {
        let objects = lock(self.objects);
        let connection = self.connection;
        let mut sessions = objects.game_sessions().filter(|session| {
            session.leader == connection
                || session.registered.contains(&connection)
                || objects.recipients(session.group).contains(&connection)
        });
        let game = sessions.find_map(|session| session.game.name());
        trace::land_target(game, self.player.place(), &self.world.lands)
    }

    /// Waits for a message that another connection caused for this one.
    pub(crate) async fn next_notice(&mut self) -> Option<HostMessage> {
        self.inbox.next().await
    }

    /// Notified when the operator's portal kicks this connection, or its inbox fills up.
    pub(crate) fn kick_signal(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.inbox.kick_signal()
    }

    /// Whether the kick came from a full inbox rather than the operator.
    pub(crate) fn inbox_overflowed(&self) -> bool {
        self.inbox.overflowed()
    }

    pub(crate) fn limits(&self) -> &'s Limits {
        self.limits
    }

    /// Names the transport on the switchboard, for the portal's list.
    pub(crate) fn label(&self, transport: &'static str) {
        self.switchboard.label(self.connection, transport);
    }

    /// The connection ended: everything it held is released and the peers are told.
    pub(crate) fn hang_up(&mut self) {
        let mut objects = lock(self.objects);
        self.world.leave_trivia(self.connection);
        let released = objects.disconnect(self.connection);
        released
            .into_iter()
            .for_each(|delivery| self.switchboard.deliver(delivery));
        if let Some(signal) = &self.logon {
            signal.send_replace(None);
        }
        if let Some(ticket) = &mut self.audit {
            ticket.close_now();
        }
        if self.counted_account.take().is_some() {
            crate::metrics::counters().player_left();
        }
    }

    /// Counts a Login that opened an account, and keeps the players gauge for as long as the line lasts.
    fn count_login(&mut self) {
        let account = self.player.account();
        if account.is_some() && account != self.counted_account {
            let first = self.counted_account.is_none();
            crate::metrics::counters().login(first);
            self.counted_account = account;
        }
    }

    /// A Login a bound line may not make, or a signup enrolment does not allow, is answered here.
    fn refusal(&self, received: &ClientMessage) -> Option<Vec<HostMessage>> {
        match received {
            ClientMessage::Login(login) => {
                let (binding, sign_in) = (self.binding.as_ref()?, self.sign_in?);
                match sign_in.may_log_in(binding, login.account) {
                    LoginGate::Open => None,
                    LoginGate::NotOwned => {
                        warn!(
                            account = login.account.0,
                            "login refused: not an account of this sign-in"
                        );
                        Some(vec![wrong_password()])
                    }
                    LoginGate::NeedsAttestation => {
                        warn!(
                            account = login.account.0,
                            "login refused: the age and terms answer is missing or out of date"
                        );
                        Some(vec![needs_attestation()])
                    }
                }
            }
            ClientMessage::Signup(_) if !self.signup_open => {
                warn!("signup refused: new accounts are made through sign-in");
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn note_sign_in(&mut self) {
        let (Some(binding), Some(sign_in)) = (self.binding.as_ref(), self.sign_in) else {
            return;
        };
        let account = self.player.account();
        if account.is_some() && account != self.noted_account {
            if let Some(id) = account {
                sign_in.note_login(binding, id);
            }
            self.noted_account = account;
        }
    }

    fn note_audit(&mut self) {
        let Some(account) = self.player.account() else {
            return;
        };
        if let Some(ticket) = &mut self.audit {
            ticket.note_account(account.0);
        }
    }

    /// Hands other connections' deliveries to the switchboard and returns this connection's own.
    fn dispatch(&self, deliveries: Vec<Delivery>) -> Vec<HostMessage> {
        let mut own = Vec::new();
        for delivery in deliveries {
            if delivery.to == self.connection {
                own.push(delivery.message);
            } else {
                self.switchboard.deliver(delivery);
            }
        }
        own
    }
}

/// The client shows the text and hangs up; the page asks the age and terms question again.
fn needs_attestation() -> HostMessage {
    HostMessage::Nak(Nak::login(
        LoginNakReason::UnknownAccount,
        0,
        "Accept the current terms on the sign-in page first, then dial again.",
    ))
}

fn wrong_password() -> HostMessage {
    let reason = LoginNakReason::RetryPassword;
    HostMessage::Nak(Nak::login(
        reason,
        WRONG_PASSWORD_TRIES,
        WRONG_PASSWORD_TEXT,
    ))
}

fn lock(objects: &Mutex<ObjectStore>) -> MutexGuard<'_, ObjectStore> {
    objects.lock().unwrap_or_else(PoisonError::into_inner)
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exchange::Received(message) => write!(f, "client {message:?}"),
            Exchange::Undecodable(error) => write!(f, "client message not decoded: {error}"),
            Exchange::Replied(message) => write!(f, "host {message:?}"),
            Exchange::NotSent(error) => write!(f, "reply not sent: {error}"),
        }
    }
}

/// Exchange text for a log line: full Debug when verbose, kind only when public.
#[derive(Debug)]
pub(crate) struct LoggedExchange<'a> {
    pub(crate) step: &'a Exchange,
    pub(crate) policy: LogPolicy,
}

impl fmt::Display for LoggedExchange<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.policy {
            LogPolicy::Verbose => write!(f, "{}", self.step),
            LogPolicy::Public => match self.step {
                Exchange::Received(message) => write!(f, "client {}", message.kind()),
                Exchange::Replied(message) => write!(f, "host {}", message.kind()),
                Exchange::Undecodable(_) => write!(f, "client message not decoded"),
                Exchange::NotSent(_) => write!(f, "reply not sent"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use innkeeper_world::{
        AccountId, ClientVersion, EncodedPassword, LandType, Login, PasswordSource,
    };

    fn login() -> ClientMessage {
        ClientMessage::Login(Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(100_001),
            password_source: PasswordSource::StoredFile,
            password: EncodedPassword([0x12, 0xa1, 0x86, 0x01, 0x00, 0x01, 0x1d, 0x7a, 0x01, 0x66]),
            name: "guybrush".into(),
            prodigy_id: None,
        })
    }

    #[test]
    fn public_mode_logs_a_login_by_kind_only() {
        let step = Exchange::Received(login());
        let public = LoggedExchange {
            step: &step,
            policy: LogPolicy::Public,
        }
        .to_string();
        assert_eq!(public, "client Login");
        assert!(!public.contains("guybrush"));
        assert!(!public.contains("EncodedPassword"));
        assert!(!public.contains("12"));
        assert!(!public.contains("a1"));
        let verbose = step.to_string();
        assert!(verbose.starts_with("client Login"));
        assert!(verbose.contains("guybrush"));
    }

    #[test]
    fn public_mode_logs_a_host_reply_by_kind_only() {
        let step = Exchange::Replied(HostMessage::LoginAccepted(Default::default()));
        let public = LoggedExchange {
            step: &step,
            policy: LogPolicy::Public,
        }
        .to_string();
        assert_eq!(public, "host LoginAccepted");
        let verbose = step.to_string();
        assert!(verbose.starts_with("host LoginAccepted"));
        assert!(verbose.contains('{'));
    }

    fn traced_host_settings() -> (
        crate::connection::ConnectionSettings,
        tokio::sync::mpsc::Receiver<trace::Item>,
    ) {
        let world = World {
            accounts: innkeeper_world::AccountBook::listed([(
                AccountId(100_001),
                EncodedPassword([0x12, 0xa1, 0x86, 0x01, 0x00, 0x01, 0x1d, 0x7a, 0x01, 0x66]),
            )]),
            lands: innkeeper_world::LandCatalog::stock(),
            ..World::stock()
        };
        let session = innkeeper_session::SessionConfig::default();
        let mut settings = crate::connection::ConnectionSettings::new(session, world, None);
        let (tracer, items) = trace::channel();
        settings.tracer = Some(tracer);
        (settings, items)
    }

    fn outcome_of(call: &Option<Call>) -> (&'static str, &'static str, &str) {
        let call = call.as_ref().unwrap();
        (call.name, call.outcome.code, call.target.as_str())
    }

    #[test]
    fn a_traced_host_classifies_each_call_and_names_its_land() {
        let (settings, mut items) = traced_host_settings();
        let tracer = settings.tracer.clone().unwrap();
        let query = "traceparent=00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let mut host = Host::new(&settings, ConnectionId(1));
        host.trace(tracer.begin_line(Some(query)));
        let garbage = host.answer(&[0xff]);
        assert_eq!(
            outcome_of(&garbage.call),
            ("Undecodable", "undecodable", "Hub")
        );
        let mut wrong = login();
        if let ClientMessage::Login(login) = &mut wrong {
            login.password = EncodedPassword([9; 10]);
        }
        let nak = host.answer(&wrong.encode());
        assert_eq!(outcome_of(&nak.call), ("Login", "nak", "Hub"));
        let accepted = host.answer(&login().encode());
        assert_eq!(outcome_of(&accepted.call), ("Login", "ok", "Hub"));
        let land = ClientMessage::JoinNet(innkeeper_world::JoinNet {
            cookie: innkeeper_world::Cookie(2),
            kind: innkeeper_world::ObjectKind::LandGroup,
            land_type: LandType(3),
            parameter: 1,
            size: 64,
        });
        let joined = host.answer(&land.encode());
        assert_eq!(outcome_of(&joined.call), ("JoinNet", "ok", "CasinoLand"));
        let unsent = [Exchange::NotSent(ReplyError::Link(LinkError::InvalidSeq(
            9,
        )))];
        host.finish_call(joined.call, &unsent);
        host.finish_call(garbage.call, &[]);
        let sent: Vec<_> = std::iter::from_fn(|| items.try_recv().ok()).collect();
        let codes: Vec<_> = sent
            .iter()
            .map(|item| match &item.detail {
                trace::Detail::Dependency { result_code, .. } => (*result_code, item.success),
                trace::Detail::Request { .. } => ("request", item.success),
            })
            .collect();
        assert_eq!(codes, [("not-sent", false), ("undecodable", false)]);
    }

    #[test]
    fn an_untraced_host_reports_no_call() {
        let (settings, mut items) = traced_host_settings();
        let mut host = Host::new(&settings, ConnectionId(1));
        let answer = host.answer(&login().encode());
        assert!(answer.call.is_none());
        host.finish_call(answer.call, &[]);
        assert!(items.try_recv().is_err());
    }

    #[test]
    fn a_refused_sign_in_login_is_the_world_s_wrong_password_reply() {
        let world = World {
            accounts: innkeeper_world::AccountBook::listed([(
                AccountId(100_001),
                EncodedPassword([1; 10]),
            )]),
            ..World::stock()
        };
        let mut objects = ObjectStore::default();
        let mut player = PlayerSession::new();
        let replies = player.handle(&world, &mut objects, ConnectionId(1), &login());
        let messages: Vec<HostMessage> = replies.into_iter().map(|d| d.message).collect();
        assert_eq!(messages, vec![wrong_password()]);
    }
}
