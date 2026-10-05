//! Relays a `Send` or a multicast to the connections that hold its target; the host never reads
//! the payload. Behaviour and policies: docs/server/router.md.

use tracing::warn;

use crate::assumptions::GROUP_SEND_ECHOES_SENDER_ASSUMED;
use crate::{ConnectionId, Delivery, HostMessage, Multicast, ObjectStore, SendMessage, Sid};

/// A `Send`: one copy per connection that holds the object, or a member of the group, `to` names.
#[must_use]
pub fn send(objects: &ObjectStore, sender: ConnectionId, message: &SendMessage) -> Vec<Delivery> {
    relay(objects, sender, message.to, message.from, &message.payload)
}

/// A multicast: the `Send` of its body to each recipient SID in list order.
#[must_use]
pub fn multicast(
    objects: &ObjectStore,
    sender: ConnectionId,
    message: &Multicast,
) -> Vec<Delivery> {
    let recipients = message.recipients.iter();
    recipients
        .flat_map(|&to| relay(objects, sender, to, message.from, &message.body))
        .collect()
}

/// Copies of one payload for the connections of `to`, in ascending connection order.
fn relay(
    objects: &ObjectStore,
    sender: ConnectionId,
    to: Sid,
    from: Sid,
    payload: &[u8],
) -> Vec<Delivery> {
    let audience = objects.recipients(to);
    if audience.is_empty() {
        warn!(
            to = to.0,
            from = from.0,
            "Send to a SID nobody holds dropped"
        );
    }
    let audience = audience.into_iter();
    let audience = audience.filter(|&holder| GROUP_SEND_ECHOES_SENDER_ASSUMED || holder != sender);
    audience
        .map(|holder| {
            let copy = SendMessage {
                to,
                from,
                payload: payload.to_vec(),
            };
            Delivery::new(holder, HostMessage::Send(copy))
        })
        .collect()
}
