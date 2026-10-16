//! Commands 51/0 and 52/0: create a list of objects, then free them.
//!
//! `placeMen` polls until every object it asked for has a nonzero SID (`SL/script.780`). The
//! reply therefore carries one SID word per requested object, and no `ObjID`. Freeing matches
//! leaveNet: the sender hears nothing, and a peer that shares a group hears `ObjFree`.

use tracing::warn;

use crate::assumptions::{
    OBJ_FREE_LIST_TELLS_PEERS_ASSUMED, OBJ_NEW_LIST_MAX_ASSUMED, OBJ_NEW_LIST_PREFACE_WORD_ASSUMED,
    OBJ_NEW_LIST_UNREAD_WORD_ASSUMED,
};
use crate::{
    ConnectionId, Cookie, Delivery, HostMessage, JoinNet, ObjFreeList, ObjNewList, ObjNewListReply,
    ObjectStore,
};

pub(crate) fn created(
    objects: &mut ObjectStore,
    connection: ConnectionId,
    request: &ObjNewList,
) -> HostMessage {
    let count = if request.count > OBJ_NEW_LIST_MAX_ASSUMED {
        warn!(
            count = request.count,
            cap = OBJ_NEW_LIST_MAX_ASSUMED,
            "object list cut to the cap"
        );
        OBJ_NEW_LIST_MAX_ASSUMED
    } else {
        request.count
    };
    let join = JoinNet {
        cookie: Cookie(request.cookie),
        kind: request.kind,
        land_type: request.land_type,
        parameter: request.parameter,
        size: request.size,
    };
    HostMessage::ObjNewList(ObjNewListReply {
        preface: OBJ_NEW_LIST_PREFACE_WORD_ASSUMED,
        cookie: request.cookie,
        unread: OBJ_NEW_LIST_UNREAD_WORD_ASSUMED,
        sids: objects.new_list(connection, join, count),
    })
}

pub(crate) fn freed(
    objects: &mut ObjectStore,
    connection: ConnectionId,
    list: &ObjFreeList,
) -> Vec<Delivery> {
    let mut deliveries = Vec::new();
    for sid in &list.sids {
        let notices = objects.leave(connection, *sid);
        if OBJ_FREE_LIST_TELLS_PEERS_ASSUMED {
            deliveries.extend(notices);
        }
    }
    deliveries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assumptions::OBJ_NEW_LIST_MAX_ASSUMED;
    use crate::{
        AccountId, ClientMessage, ClientVersion, ConnectionId, Cookie, EncodedPassword, GroupJoin,
        GroupLeave, HostMessage, JoinNet, LandType, Login, ObjFreeList, ObjNewList, ObjectKind,
        ObjectStore, PasswordSource, PlayerSession, Sid, World,
    };

    const ME: ConnectionId = ConnectionId(1);
    const PEER: ConnectionId = ConnectionId(2);
    const COOKIE: u16 = 0x0A0B;

    fn logged_in() -> (World, ObjectStore, PlayerSession) {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let login = ClientMessage::Login(Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(100_001),
            password_source: PasswordSource::StoredFile,
            password: EncodedPassword([7; 10]),
            name: "guybrush".into(),
            prodigy_id: None,
        });
        let _ = player.handle(&world, &mut objects, ME, &login);
        (world, objects, player)
    }

    fn new_list(count: u16) -> ClientMessage {
        ClientMessage::ObjNewList(ObjNewList {
            preface: 0,
            cookie: COOKIE,
            kind: ObjectKind::Object,
            land_type: LandType(1),
            parameter: 0xFFFE,
            size: 0x001E,
            count,
        })
    }

    fn created_sids(replies: &[Delivery]) -> Vec<Sid> {
        let Some(Delivery {
            message: HostMessage::ObjNewList(reply),
            ..
        }) = replies.first()
        else {
            panic!("a new list answers with SIDs from byte 8, got {replies:?}");
        };
        assert_eq!(reply.cookie, COOKIE);
        assert_eq!(reply.preface, 0);
        assert_eq!(reply.unread, 0);
        assert_eq!(replies.len(), 1);
        reply.sids.clone()
    }

    #[test]
    fn a_list_creates_one_object_per_count_and_a_free_drops_them() {
        let (world, mut objects, mut player) = logged_in();
        let replies = player.handle(&world, &mut objects, ME, &new_list(2));
        let sids = created_sids(&replies);
        assert_eq!(sids, vec![Sid(0x0100), Sid(0x0101)]);
        assert!(replies.iter().all(|delivery| delivery.to == ME));
        assert_eq!(objects.land_type_of(sids[0]), Some(LandType(1)));

        let free = ClientMessage::ObjFreeList(ObjFreeList {
            handle: 0,
            sids: sids.clone(),
        });
        let gone = player.handle(&world, &mut objects, ME, &free);
        assert!(gone.is_empty());
        assert_eq!(objects.land_type_of(sids[0]), None);
        let again = player.handle(&world, &mut objects, ME, &free);
        assert!(again.is_empty());
    }

    #[test]
    fn a_list_before_login_creates_nothing() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let replies = player.handle(&world, &mut objects, ME, &new_list(1));
        assert!(replies.is_empty());
        assert!(objects.is_empty());
    }

    #[test]
    fn a_list_longer_than_the_cap_is_cut() {
        let (world, mut objects, mut player) = logged_in();
        let replies = player.handle(
            &world,
            &mut objects,
            ME,
            &new_list(OBJ_NEW_LIST_MAX_ASSUMED + 1),
        );
        let sids = created_sids(&replies);
        assert_eq!(sids.len(), usize::from(OBJ_NEW_LIST_MAX_ASSUMED));
        let first = sids[0].0;
        let consecutive = sids
            .iter()
            .enumerate()
            .all(|(index, sid)| sid.0 == first + u16::try_from(index).unwrap());
        assert!(consecutive);
        assert_eq!(
            objects.land_type_of(Sid(first + OBJ_NEW_LIST_MAX_ASSUMED)),
            None
        );
    }

    #[test]
    fn freeing_a_listed_object_tells_the_peer_and_not_the_sender() {
        let mut objects = ObjectStore::new();
        let shared = JoinNet {
            cookie: Cookie(1),
            kind: ObjectKind::LandGroup,
            land_type: LandType(1),
            parameter: 1,
            size: 2,
        };
        let group = granted(&objects.join(ME, shared));
        let _ = objects.join(PEER, shared);
        let item = objects.new_list(
            ME,
            JoinNet {
                cookie: Cookie(COOKIE),
                kind: ObjectKind::Object,
                land_type: LandType(1),
                parameter: 0xFFFE,
                size: 1,
            },
            1,
        );
        let item = item[0];
        let _ = objects.join_group(
            ME,
            GroupJoin {
                group,
                member: item,
                version: None,
            },
        );
        let player = granted(&objects.join(
            PEER,
            JoinNet {
                cookie: Cookie(2),
                kind: ObjectKind::Object,
                land_type: LandType(1),
                parameter: 0xFFFF,
                size: 1,
            },
        ));
        let _ = objects.join_group(
            PEER,
            GroupJoin {
                group,
                member: player,
                version: None,
            },
        );

        let notices = freed(
            &mut objects,
            ME,
            &ObjFreeList {
                handle: 0,
                sids: vec![item],
            },
        );
        assert!(notices.iter().all(|delivery| delivery.to == PEER));
        assert_eq!(
            notices,
            vec![
                Delivery::new(
                    PEER,
                    HostMessage::GroupLeft(GroupLeave {
                        group,
                        member: item
                    })
                ),
                Delivery::new(PEER, HostMessage::ObjectFreed(item)),
            ]
        );
        assert_eq!(objects.land_type_of(item), None);
    }

    fn granted(deliveries: &[Delivery]) -> Sid {
        match deliveries.last() {
            Some(Delivery {
                message: HostMessage::ObjId { sid, .. },
                ..
            }) => *sid,
            other => panic!("expected an ObjID last, got {other:?}"),
        }
    }
}
