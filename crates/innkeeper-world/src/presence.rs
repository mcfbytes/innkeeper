//! Who is in a land's waiting room, and who may enter it (docs/server/objects.md section 7).

use tracing::info;

use crate::{
    ConnectionId, Delivery, GroupJoin, GroupKey, LandCatalog, LandNumber, LandType, ObjectKind,
    ObjectStore,
};

/// The shared group a land's waiting room is: joinNet kind 5 with the land number as its parameter.
pub(crate) fn waiting_room(land_type: LandType, land_number: LandNumber) -> GroupKey {
    GroupKey {
        kind: ObjectKind::LandGroup,
        land_type,
        parameter: u16::from(land_number.0),
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
        ) if !lands.admits(land_type, version) => {
            info!(%version, land_type = land_type.0, "client version outside the land's range");
            objects.refuse_wrong_version(connection, join)
        }
        _ => objects.join_group(connection, join),
    }
}
