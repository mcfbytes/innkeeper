use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use innkeeper_session::{Session, SessionError};
use innkeeper_world::{
    ClientMessage, ConnectionId, Delivery, HostMessage, MessageError, ObjectStore, PlayerSession,
    World,
};
use thiserror::Error;
use tsn_link::{LinkError, Message};

use crate::connection::ConnectionSettings;
use crate::switchboard::{Inbox, Switchboard};

/// The application end of one connection: decodes client messages, sends the world's replies and
/// what other connections caused, and releases the connection's objects when it ends.
#[derive(Debug)]
pub(crate) struct Host<'s> {
    world: &'s World,
    objects: &'s Mutex<ObjectStore>,
    switchboard: &'s Switchboard,
    connection: ConnectionId,
    inbox: Inbox<'s>,
    player: PlayerSession,
}

/// One step of a message exchange, for the log and the capture file.
#[derive(Debug)]
pub(crate) enum Exchange {
    Received(ClientMessage),
    Undecodable(MessageError),
    Replied(HostMessage),
    NotSent(ReplyError),
}

#[derive(Debug, Error)]
pub(crate) enum ReplyError {
    #[error(transparent)]
    Link(#[from] LinkError),
    #[error(transparent)]
    Session(#[from] SessionError),
}

impl<'s> Host<'s> {
    pub(crate) fn new(settings: &'s ConnectionSettings, connection: ConnectionId) -> Self {
        Host {
            world: &settings.world,
            objects: &settings.objects,
            switchboard: &settings.switchboard,
            connection,
            inbox: settings.switchboard.register(connection),
            player: PlayerSession::new(),
        }
    }

    pub(crate) fn answer(
        &mut self,
        session: &mut Session,
        message: &Message,
        now: Instant,
    ) -> Vec<Exchange> {
        let received = match ClientMessage::parse(message.body()) {
            Ok(received) => received,
            Err(error) => return vec![Exchange::Undecodable(error)],
        };
        let mut objects = lock(self.objects);
        // Under the store lock the inbox holds everything caused before this message, in order.
        let mut outgoing = self.inbox.drain();
        let caused = self
            .player
            .handle(self.world, &mut objects, self.connection, &received);
        outgoing.extend(self.dispatch(caused));
        drop(objects);
        let mut exchange = vec![Exchange::Received(received)];
        exchange.extend(send_all(session, outgoing, now));
        exchange
    }

    /// Waits for a message that another connection caused for this one.
    pub(crate) async fn next_notice(&mut self) -> Option<HostMessage> {
        self.inbox.next().await
    }

    pub(crate) fn pass_on(
        &mut self,
        session: &mut Session,
        notice: HostMessage,
        now: Instant,
    ) -> Vec<Exchange> {
        send_all(session, vec![notice], now)
    }

    /// The connection ended: everything it held is released and the peers are told.
    pub(crate) fn hang_up(&mut self) {
        let mut objects = lock(self.objects);
        let released = objects.disconnect(self.connection);
        released
            .into_iter()
            .for_each(|delivery| self.switchboard.deliver(delivery));
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

fn lock(objects: &Mutex<ObjectStore>) -> MutexGuard<'_, ObjectStore> {
    objects.lock().unwrap_or_else(PoisonError::into_inner)
}

fn send_all(session: &mut Session, messages: Vec<HostMessage>, now: Instant) -> Vec<Exchange> {
    let mut exchange = Vec::new();
    for message in messages {
        exchange.push(match send(session, &message, now) {
            Ok(()) => Exchange::Replied(message),
            Err(error) => Exchange::NotSent(error),
        });
    }
    if let Err(error) = session.flush(now) {
        exchange.push(Exchange::NotSent(error.into()));
    }
    exchange
}

fn send(session: &mut Session, reply: &HostMessage, now: Instant) -> Result<(), ReplyError> {
    let message = Message::try_new(reply.encode())?;
    session.send_message(&message, now)?;
    Ok(())
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
