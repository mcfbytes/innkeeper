//! Who is in a land's waiting room, and who may enter it (docs/server/objects.md section 7).

use tracing::info;

use crate::rpg;
use crate::{
    ConnectionId, Delivery, GroupJoin, GroupKey, HostMessage, LandCatalog, LandNumber, LandType,
    ObjectKind, ObjectStore,
};

/// The shared group a land's waiting room is: joinNet kind 5 with the land number as its parameter.
pub(crate) fn waiting_room(land_type: LandType, land_number: LandNumber) -> GroupKey {
    GroupKey {
        kind: ObjectKind::LandGroup,
        land_type,
        parameter: u16::from(land_number.0),
        within: None,
    }
}

/// GrpJoin through the version gate: a waiting room turns away a client outside its land's range.
pub(crate) fn join_group(
    lands: &LandCatalog,
    objects: &mut ObjectStore,
    connection: ConnectionId,
    join: GroupJoin,
) -> Vec<Delivery> {
    let room = objects.group_key(join.group);
    match (join.version, room) {
        (
            Some(version),
            Some(GroupKey {
                kind: ObjectKind::LandGroup,
                land_type,
                ..
            }),
        ) if !lands.admits(land_type, version) || !rpg::admits(land_type, version) => {
            info!(%version, land_type = land_type.0, "client version outside the land's range");
            objects.refuse_wrong_version(connection, join)
        }
        (
            Some(_),
            Some(GroupKey {
                kind: ObjectKind::LandGroup,
                land_type,
                ..
            }),
        ) if !rpg::is_title(land_type) => {
            let mut deliveries = objects.join_group(connection, join);
            deliveries.extend(tell_who_is_here(objects, connection, join));
            deliveries
        }
        _ => objects.join_group(connection, join),
    }
}

/// A newcomer to an LSCI waiting room (its GrpJoin carries the client's version) learns who is already there from one `GrpJoin` per member after its
/// own echo: its client sends no member request, and the room's `GrpJoin` handler
/// (`class_83::handleMsg`) builds a replica of the member and fetches its properties. A single `GrpMem`
/// with the whole list shows the names too, but its replicas left the game groups after it
/// short of members (docs/server/games.md section 2).
fn tell_who_is_here(
    objects: &ObjectStore,
    connection: ConnectionId,
    join: GroupJoin,
) -> Vec<Delivery> {
    if !objects.group_has_member(join.group, join.member) {
        return Vec::new();
    }
    let others = objects.members(join.group).iter().copied();
    others
        .filter(|&member| member != join.member)
        .map(|member| {
            Delivery::new(
                connection,
                HostMessage::GroupJoined {
                    group: join.group,
                    member,
                },
            )
        })
        .collect()
}
