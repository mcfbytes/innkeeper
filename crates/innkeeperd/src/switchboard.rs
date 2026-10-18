use std::collections::HashMap;
use std::iter;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use innkeeper_world::{ConnectionId, Delivery, HostMessage};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;
use tracing::{debug, warn};

use crate::dates;
use crate::limits::LINE_QUEUE_BYTES_DEFAULT;

/// The bytes before a `Send` payload: command, flags, `to` and `from`.
const SEND_HEADER_LEN: usize = 6;

/// What the board knows of one live connection.
#[derive(Debug)]
struct Line {
    sender: UnboundedSender<HostMessage>,
    /// Woken by the operator's portal, or by a full inbox, to end the connection.
    kick: Arc<Notify>,
    backlog: Arc<Backlog>,
    since: i64,
    transport: &'static str,
}

/// The encoded bytes in one inbox, and whether it overflowed; shared by the board and the inbox.
#[derive(Debug, Default)]
struct Backlog {
    bytes: AtomicUsize,
    overflowed: AtomicBool,
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
/// An inbox past `queue_limit` bytes is closed and its connection ended.
#[derive(Debug)]
pub(crate) struct Switchboard {
    inboxes: Mutex<Inboxes>,
    queue_limit: usize,
}

/// One connection's end of the switchboard; dropping it takes the connection off the board.
#[derive(Debug)]
pub(crate) struct Inbox<'s> {
    switchboard: &'s Switchboard,
    connection: ConnectionId,
    messages: UnboundedReceiver<HostMessage>,
    kick: Arc<Notify>,
    backlog: Arc<Backlog>,
}

impl Default for Switchboard {
    fn default() -> Self {
        Switchboard::with_queue_limit(LINE_QUEUE_BYTES_DEFAULT)
    }
}

impl Switchboard {
    pub(crate) fn with_queue_limit(queue_limit: usize) -> Self {
        Switchboard {
            inboxes: Mutex::default(),
            queue_limit,
        }
    }

    pub(crate) fn register(&self, connection: ConnectionId) -> Inbox<'_> {
        let (sender, messages) = mpsc::unbounded_channel();
        let kick = Arc::new(Notify::new());
        let backlog = Arc::new(Backlog::default());
        let line = Line {
            sender,
            kick: Arc::clone(&kick),
            backlog: Arc::clone(&backlog),
            since: dates::now_ms(),
            transport: "",
        };
        self.lock().insert(connection, line);
        Inbox {
            switchboard: self,
            connection,
            messages,
            kick,
            backlog,
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

    /// Queues the message for its connection; one that has hung up meanwhile misses it, and one
    /// whose inbox would pass the limit is ended instead.
    pub(crate) fn deliver(&self, delivery: Delivery) {
        let Delivery { to, message } = delivery;
        let inboxes = self.lock();
        let Some(line) = inboxes.get(&to) else {
            debug!(connection = to.0, "delivery to a closed connection dropped");
            return;
        };
        if line.backlog.overflowed.load(Ordering::Relaxed) {
            return;
        }
        let size = queued_size(&message);
        let queued = line.backlog.bytes.fetch_add(size, Ordering::Relaxed) + size;
        if queued > self.queue_limit {
            line.backlog.overflowed.store(true, Ordering::Relaxed);
            line.kick.notify_one();
            warn!(
                connection = to.0,
                queued, "inbox full: the connection is ended"
            );
            return;
        }
        if line.sender.send(message).is_err() {
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
        let message = self.messages.recv().await?;
        self.taken(&message);
        Some(message)
    }

    /// Whether the inbox filled up, which is why its kick signal fired.
    pub(crate) fn overflowed(&self) -> bool {
        self.backlog.overflowed.load(Ordering::Relaxed)
    }

    /// Notified when the operator kicks this connection; hold it across a select loop.
    pub(crate) fn kick_signal(&self) -> Arc<Notify> {
        Arc::clone(&self.kick)
    }

    /// Every message already waiting, oldest first.
    pub(crate) fn drain(&mut self) -> Vec<HostMessage> {
        let drained: Vec<HostMessage> = iter::from_fn(|| self.messages.try_recv().ok()).collect();
        drained.iter().for_each(|message| self.taken(message));
        drained
    }

    fn taken(&self, message: &HostMessage) {
        let size = queued_size(message);
        let bytes = &self.backlog.bytes;
        let _ = bytes.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |queued| {
            Some(queued.saturating_sub(size))
        });
    }
}

/// What a queued message will take once encoded; a `Send` is sized without copying its payload.
fn queued_size(message: &HostMessage) -> usize {
    match message {
        HostMessage::Send(send) => SEND_HEADER_LEN + send.payload.len(),
        other => other.encode().len(),
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

    fn send_of(len: usize) -> HostMessage {
        HostMessage::Send(innkeeper_world::SendMessage {
            to: Sid(0x0100),
            from: Sid(0x0101),
            payload: vec![0x41; len].into(),
        })
    }

    #[tokio::test]
    async fn a_full_inbox_ends_its_connection_and_takes_no_more() {
        let switchboard = Switchboard::with_queue_limit(1000);
        let mut inbox = switchboard.register(ConnectionId(1));
        let kicked = inbox.kick_signal();
        let to = |message| Delivery::new(ConnectionId(1), message);
        switchboard.deliver(to(send_of(400)));
        assert_eq!(inbox.drain().len(), 1);
        switchboard.deliver(to(send_of(400)));
        switchboard.deliver(to(send_of(400)));
        assert!(!inbox.overflowed(), "drained bytes no longer count");
        switchboard.deliver(to(send_of(400)));
        assert!(inbox.overflowed());
        tokio::time::timeout(std::time::Duration::from_secs(1), kicked.notified())
            .await
            .expect("the connection is kicked");
        switchboard.deliver(to(send_of(1)));
        assert_eq!(inbox.drain().len(), 2);
    }
}
