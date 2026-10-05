//! Relays a `Send` or a multicast to the connections that hold its target, and property updates and
//! remote calls to the other replicas; the host never reads a payload. Behaviour: docs/server/router.md.

use tracing::warn;

use crate::assumptions::GROUP_SEND_ECHOES_SENDER_ASSUMED;
use crate::{
    ConnectionId, Delivery, HostMessage, InvokeMethod, Multicast, ObjectStore, SendMessage, SetInt,
    SetStr, Sid,
};

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

/// A `SetInt`: mirrored, then passed on to every other connection with a replica of the target.
#[must_use]
pub fn set_int(objects: &mut ObjectStore, sender: ConnectionId, set: &SetInt) -> Vec<Delivery> {
    objects.mirror_ints(set);
    replicate(
        objects,
        sender,
        set.target,
        &HostMessage::SetInt(set.clone()),
    )
}

/// A `SetStr`: mirrored, then passed on like a `SetInt`.
#[must_use]
pub fn set_str(objects: &mut ObjectStore, sender: ConnectionId, set: &SetStr) -> Vec<Delivery> {
    objects.mirror_text(set);
    replicate(
        objects,
        sender,
        set.target,
        &HostMessage::SetStr(set.clone()),
    )
}

/// An invokeMethod: the sender has run it on its own copy, so only the other replicas get it.
#[must_use]
pub fn invoke_method(
    objects: &ObjectStore,
    sender: ConnectionId,
    call: &InvokeMethod,
) -> Vec<Delivery> {
    replicate(
        objects,
        sender,
        call.target,
        &HostMessage::InvokeMethod(call.clone()),
    )
}

fn replicate(
    objects: &ObjectStore,
    sender: ConnectionId,
    target: Sid,
    message: &HostMessage,
) -> Vec<Delivery> {
    let mut audience = objects.replicas(target);
    audience.remove(&sender);
    let audience = audience.into_iter();
    audience
        .map(|replica| Delivery::new(replica, message.clone()))
        .collect()
}
