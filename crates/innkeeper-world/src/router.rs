//! Relays a `Send` or a multicast to the connections that hold its target, and property updates and
//! remote calls to the other replicas; the host never reads a payload. Behaviour: docs/server/router.md.

use std::collections::BTreeSet;
use std::sync::Arc;

use tracing::warn;

use crate::assumptions::{GROUP_SEND_SKIPS_SENDER_ASSUMED, MULTICAST_RECIPIENTS_MAX_ASSUMED};
use crate::rpg;
use crate::{
    ConnectionId, Delivery, HostMessage, InvokeMethod, Multicast, ObjectStore, SendMessage, SetInt,
    SetStr, Sid,
};

/// A `Send`: one copy per connection that holds the object, or a member of the group, `to` names.
#[must_use]
pub fn send(objects: &ObjectStore, sender: ConnectionId, message: &SendMessage) -> Vec<Delivery> {
    relay(objects, sender, message.to, message.from, &message.payload)
}

/// A multicast: the `Send` of its body to each distinct recipient SID, in order of first listing.
/// A list of more distinct SIDs than any client sends is dropped whole.
#[must_use]
pub fn multicast(
    objects: &ObjectStore,
    sender: ConnectionId,
    message: &Multicast,
) -> Vec<Delivery> {
    let recipients = distinct(&message.recipients);
    if recipients.len() > MULTICAST_RECIPIENTS_MAX_ASSUMED {
        warn!(
            listed = message.recipients.len(),
            distinct = recipients.len(),
            "multicast to more SIDs than any client lists dropped"
        );
        return Vec::new();
    }
    let body: Arc<[u8]> = message.body.as_slice().into();
    let recipients = recipients.into_iter();
    recipients
        .flat_map(|to| relay(objects, sender, to, message.from, &body))
        .collect()
}

fn distinct(sids: &[Sid]) -> Vec<Sid> {
    let mut seen = BTreeSet::new();
    let first_listings = sids.iter().filter(|&&sid| seen.insert(sid));
    first_listings.copied().collect()
}

/// Copies of one payload for the connections of `to`, in ascending connection order.
fn relay(
    objects: &ObjectStore,
    sender: ConnectionId,
    to: Sid,
    from: Sid,
    payload: &Arc<[u8]>,
) -> Vec<Delivery> {
    let audience = objects.recipients(to);
    if audience.is_empty() {
        warn!(
            to = to.0,
            from = from.0,
            "Send to a SID nobody holds dropped"
        );
    }
    let skips_sender = GROUP_SEND_SKIPS_SENDER_ASSUMED && objects.is_group(to);
    let echo = rpg::echoes_sender(objects.land_type_of(to)) && !skips_sender;
    let audience = audience.into_iter();
    let audience = audience.filter(|&holder| echo || holder != sender);
    audience
        .map(|holder| {
            let copy = SendMessage {
                to,
                from,
                payload: Arc::clone(payload),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cookie, JoinNet, LandType, ObjectKind};

    const SENDER: ConnectionId = ConnectionId(1);
    const PEER: ConnectionId = ConnectionId(2);

    fn peer_object(objects: &mut ObjectStore) -> Sid {
        let join = JoinNet {
            cookie: Cookie(1),
            kind: ObjectKind::Object,
            land_type: LandType(0x66),
            parameter: 0xFFFF,
            size: 1,
        };
        let granted = objects.join(PEER, join);
        let Some(HostMessage::ObjId { sid, .. }) = granted.last().map(|d| &d.message) else {
            panic!("joinNet answered with an ObjID");
        };
        *sid
    }

    fn multicast_to(recipients: Vec<Sid>) -> Multicast {
        Multicast {
            from: Sid(0x0200),
            recipients,
            body: vec![0xC9; 35],
        }
    }

    #[test]
    fn a_sid_listed_many_times_gets_one_copy() {
        let mut objects = ObjectStore::new();
        let peer = peer_object(&mut objects);
        let message = multicast_to(vec![peer; 10_000]);
        let copies = multicast(&objects, SENDER, &message);
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].to, PEER);
    }

    #[test]
    fn a_list_longer_than_any_client_sends_is_dropped_whole() {
        let mut objects = ObjectStore::new();
        let peer = peer_object(&mut objects);
        let others = (0..MULTICAST_RECIPIENTS_MAX_ASSUMED).map(|n| Sid(0x7000 + n as u16));
        let mut most: Vec<Sid> = others.take(MULTICAST_RECIPIENTS_MAX_ASSUMED - 1).collect();
        most.push(peer);
        assert_eq!(
            multicast(&objects, SENDER, &multicast_to(most.clone())).len(),
            1
        );
        most.insert(0, Sid(0x6FFF));
        assert!(multicast(&objects, SENDER, &multicast_to(most)).is_empty());
    }
}
