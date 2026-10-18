use std::collections::HashMap;
use std::iter;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use innkeeper_world::{ConnectionId, Delivery, HostMessage};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;
use tracing::debug;

use crate::dates;

/// What the board knows of one live connection.
#[derive(Debug)]
struct Line {
    sender: UnboundedSender<HostMessage>,
    /// Woken by the operator's portal to end the connection.
    kick: Arc<Notify>,
    since: i64,
    transport: &'static str,
}

type Inboxes = HashMap<ConnectionId, Line>;

/// A live connection as the operator's portal lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Connected {
    pub(crate) connection: ConnectionId,
    /// Unix milliseconds.
    pub(crate) since: i64,
    pub(crate) transport: &'static str,
}

/// Every live connection's inbox, so a delivery for another connection reaches its pump loop.
#[derive(Debug, Default)]
pub(crate) struct Switchboard {
    inboxes: Mutex<Inboxes>,
}

/// One connection's end of the switchboard; dropping it takes the connection off the board.
#[derive(Debug)]
pub(crate) struct Inbox<'s> {
    switchboard: &'s Switchboard,
    connection: ConnectionId,
    messages: UnboundedReceiver<HostMessage>,
    kick: Arc<Notify>,
}

impl Switchboard {
    pub(crate) fn register(&self, connection: ConnectionId) -> Inbox<'_> {
        let (sender, messages) = mpsc::unbounded_channel();
        let kick = Arc::new(Notify::new());
        let line = Line {
            sender,
            kick: Arc::clone(&kick),
            since: dates::now_ms(),
            transport: "",
        };
        self.lock().insert(connection, line);
        Inbox {
            switchboard: self,
            connection,
            messages,
            kick,
        }
    }

    /// Names the transport a registered connection arrived on.
    pub(crate) fn label(&self, connection: ConnectionId, transport: &'static str) {
        if let Some(line) = self.lock().get_mut(&connection) {
            line.transport = transport;
        }
    }

    /// Ends the connection at its next turn. False when it is not on the board.
    pub(crate) fn kick(&self, connection: ConnectionId) -> bool {
        let inboxes = self.lock();
        let Some(line) = inboxes.get(&connection) else {
            return false;
        };
        line.kick.notify_one();
        true
    }

    /// How many connections are live.
    pub(crate) fn count(&self) -> usize {
        self.lock().len()
    }

    /// Every live connection, oldest first.
    pub(crate) fn connections(&self) -> Vec<Connected> {
        let mut all: Vec<Connected> = self
            .lock()
            .iter()
            .map(|(&connection, line)| Connected {
                connection,
                since: line.since,
                transport: line.transport,
            })
            .collect();
        all.sort_by_key(|entry| entry.connection.0);
        all
    }

    /// Queues the message for its connection; one that has hung up meanwhile misses it.
    pub(crate) fn deliver(&self, delivery: Delivery) {
        let Delivery { to, message } = delivery;
        let inboxes = self.lock();
        let dropped = match inboxes.get(&to) {
            Some(line) => line.sender.send(message).is_err(),
            None => true,
        };
        if dropped {
            debug!(connection = to.0, "delivery to a closed connection dropped");
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inboxes> {
        self.inboxes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Inbox<'_> {
    /// The next message another connection caused; never ends while the inbox is registered.
    pub(crate) async fn next(&mut self) -> Option<HostMessage> {
        self.messages.recv().await
    }

    /// Notified when the operator kicks this connection; hold it across a select loop.
    pub(crate) fn kick_signal(&self) -> Arc<Notify> {
        Arc::clone(&self.kick)
    }

    /// Every message already waiting, oldest first.
    pub(crate) fn drain(&mut self) -> Vec<HostMessage> {
        iter::from_fn(|| self.messages.try_recv().ok()).collect()
    }
}

impl Drop for Inbox<'_> {
    fn drop(&mut self) {
        self.switchboard.lock().remove(&self.connection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use innkeeper_world::Sid;

    #[tokio::test]
    async fn a_delivery_reaches_the_registered_inbox_only_while_it_lives() {
        let switchboard = Switchboard::default();
        let freed = HostMessage::ObjectFreed(Sid(0x0100));
        let mut inbox = switchboard.register(ConnectionId(1));
        switchboard.deliver(Delivery::new(ConnectionId(1), freed.clone()));
        assert_eq!(inbox.next().await, Some(freed.clone()));
        drop(inbox);
        switchboard.deliver(Delivery::new(ConnectionId(1), freed));
        assert!(switchboard.lock().is_empty());
    }
}
