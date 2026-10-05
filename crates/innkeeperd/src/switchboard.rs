use std::collections::HashMap;
use std::iter;
use std::sync::{Mutex, MutexGuard, PoisonError};

use innkeeper_world::{ConnectionId, Delivery, HostMessage};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tracing::debug;

type Inboxes = HashMap<ConnectionId, UnboundedSender<HostMessage>>;

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
}

impl Switchboard {
    pub(crate) fn register(&self, connection: ConnectionId) -> Inbox<'_> {
        let (sender, messages) = mpsc::unbounded_channel();
        self.lock().insert(connection, sender);
        Inbox {
            switchboard: self,
            connection,
            messages,
        }
    }

    /// Queues the message for its connection; one that has hung up meanwhile misses it.
    pub(crate) fn deliver(&self, delivery: Delivery) {
        let Delivery { to, message } = delivery;
        let inboxes = self.lock();
        let sent = inboxes.get(&to).map(|inbox| inbox.send(message));
        if !matches!(sent, Some(Ok(()))) {
            debug!(connection = to.0, "delivery to a closed connection dropped");
        }
    }

    /// Queues a copy of the message for every connection on the board; a closed inbox is skipped.
    pub(crate) fn broadcast(&self, message: &HostMessage) -> usize {
        let inboxes = self.lock();
        let reached = inboxes
            .values()
            .filter(|inbox| inbox.send(message.clone()).is_ok())
            .count();
        debug!(reached, "broadcast");
        reached
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
    use innkeeper_world::{Notice, Sid};

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

    #[tokio::test]
    async fn a_broadcast_reaches_every_open_inbox_and_skips_a_closed_one() {
        let switchboard = Switchboard::default();
        let notice = HostMessage::Notice(Notice {
            text: "hello".into(),
        });
        let mut first = switchboard.register(ConnectionId(1));
        let mut second = switchboard.register(ConnectionId(2));
        let mut closed = switchboard.register(ConnectionId(3));
        closed.messages.close();
        assert_eq!(switchboard.broadcast(&notice), 2);
        assert_eq!(first.next().await, Some(notice.clone()));
        assert_eq!(second.next().await, Some(notice));
        assert!(closed.drain().is_empty());
    }
}
