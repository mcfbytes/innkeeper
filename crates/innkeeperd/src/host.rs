use std::fmt;
use std::time::Instant;

use innkeeper_session::{Session, SessionError};
use innkeeper_world::{ClientMessage, HostMessage, MessageError, PlayerSession, World};
use thiserror::Error;
use tsn_link::{LinkError, Message};

/// The application end of one connection: decodes client messages and sends the world's replies.
#[derive(Debug)]
pub(crate) struct Host<'w> {
    world: &'w World,
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

impl<'w> Host<'w> {
    pub(crate) fn new(world: &'w World) -> Self {
        Host {
            world,
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
        let replies = self.player.handle(self.world, &received);
        let mut exchange = vec![Exchange::Received(received)];
        for reply in replies {
            exchange.push(match send(session, &reply, now) {
                Ok(()) => Exchange::Replied(reply),
                Err(error) => Exchange::NotSent(error),
            });
        }
        if let Err(error) = session.flush(now) {
            exchange.push(Exchange::NotSent(error.into()));
        }
        exchange
    }
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
