use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use innkeeper_session::SessionError;
use innkeeper_world::{
    ClientMessage, ConnectionId, Delivery, HostMessage, MessageError, ObjectStore, PlayerSession,
    World,
};
use int14h::Int14hError;
use thiserror::Error;
use tsn_link::LinkError;

use crate::connection::ConnectionSettings;
use crate::switchboard::{Inbox, Switchboard};

/// The application end of one connection, whatever its transport: decodes client messages, returns
/// the world's replies and what other connections caused, and releases the connection's objects.
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

    pub(crate) fn answer(&mut self, body: &[u8]) -> Answer {
        let received = match ClientMessage::parse(body) {
            Ok(received) => received,
            Err(error) => {
                return Answer {
                    exchange: vec![Exchange::Undecodable(error)],
                    outgoing: Vec::new(),
                }
            }
        };
        let mut objects = lock(self.objects);
        // Under the store lock the inbox holds everything caused before this message, in order.
        let mut outgoing = self.inbox.drain();
        let caused = self
            .player
            .handle(self.world, &mut objects, self.connection, &received);
        outgoing.extend(self.dispatch(caused));
        drop(objects);
        Answer {
            exchange: vec![Exchange::Received(received)],
            outgoing,
        }
    }

    /// Waits for a message that another connection caused for this one.
    pub(crate) async fn next_notice(&mut self) -> Option<HostMessage> {
        self.inbox.next().await
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
