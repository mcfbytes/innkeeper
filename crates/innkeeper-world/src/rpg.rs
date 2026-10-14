//! The RPG titles (Yserbius, Twinion): their lands, the service lookup and map groups per place.
//! What the clients need and how it was established: docs/protocol/yserbius.md.

use tracing::info;

use crate::assumptions::{
    MAP_GROUPS_PER_PLACE_ASSUMED, RPG_LAND_ADMITS_ONLY_ITS_VERSION_ASSUMED, RPG_SERVICE_SID_ASSUMED,
};
use crate::{
    ClientVersion, ConnectionId, Delivery, HostMessage, JoinNet, LandType, ObjExists, ObjectKind,
    ObjectLocated, ObjectStore, Place,
};

/// The land type of every player object, map group and party both titles create.
pub(crate) const RPG_OBJECT_LAND_TYPE: LandType = LandType(0x8B);

/// One title: the land `LAND.CFG` gives it and what its game program sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpgTitle {
    pub(crate) land_type: LandType,
    /// One word, `_` for a space, at most 16 characters: the hub's place list shows it.
    pub(crate) description: &'static str,
    /// The version bytes of the land GrpJoin (10/3).
    pub(crate) client_version: ClientVersion,
    /// The size word of the land joinNet (kind 5).
    pub(crate) land_capacity: u8,
}

pub(crate) const RPG_TITLES: [RpgTitle; 2] = [
    RpgTitle {
        land_type: LandType(4),
        description: "Yserbius",
        client_version: ClientVersion::new(1, 1, 7),
        land_capacity: 100,
    },
    RpgTitle {
        land_type: LandType(8),
        description: "Twinion",
        client_version: ClientVersion::new(1, 0, 22),
        land_capacity: 100,
    },
];

/// Whether a land GrpJoin's version bytes fit the title of that land; other lands are not gated here.
pub(crate) fn admits(land_type: LandType, version: ClientVersion) -> bool {
    let mut titles = RPG_TITLES
        .iter()
        .filter(|title| title.land_type == land_type);
    titles.all(|title| !RPG_LAND_ADMITS_ONLY_ITS_VERSION_ASSUMED || title.client_version == version)
}

/// The answer to a 41/2 service lookup of the RPG land type; other lookups have no answer yet.
pub(crate) fn locate_service(lookup: &ObjExists) -> Option<HostMessage> {
    match *lookup {
        ObjExists::Service { user, land_type } if land_type == RPG_OBJECT_LAND_TYPE => {
            info!(user = user.0, "RPG service located");
            Some(HostMessage::ObjectLocated(ObjectLocated {
                to: user,
                sid: RPG_SERVICE_SID_ASSUMED,
            }))
        }
        ObjExists::Service { .. } | ObjExists::Name { .. } => None,
    }
}

/// The place a land joinNet (kind 5) enters; the RPG clients scope their map groups by it.
pub(crate) fn place_of(join: &JoinNet) -> Option<Place> {
    (join.kind == ObjectKind::LandGroup).then_some(Place {
        land_type: join.land_type,
        land_number: join.parameter,
    })
}

/// joinNet: an RPG map group is shared within the requester's place, everything else as usual.
pub(crate) fn join(
    objects: &mut ObjectStore,
    connection: ConnectionId,
    join: JoinNet,
    place: Option<Place>,
) -> Vec<Delivery> {
    let map_group = join.kind == ObjectKind::Group && join.land_type == RPG_OBJECT_LAND_TYPE;
    let within = place.filter(|_| map_group && MAP_GROUPS_PER_PLACE_ASSUMED);
    objects.join_within(connection, join, within)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cookie, Sid};

    #[test]
    fn only_the_rpg_service_lookup_is_answered() {
        let lookup = |land_type| ObjExists::Service {
            user: Sid(0x0100),
            land_type,
        };
        assert_eq!(
            locate_service(&lookup(RPG_OBJECT_LAND_TYPE)),
            Some(HostMessage::ObjectLocated(ObjectLocated {
                to: Sid(0x0100),
                sid: RPG_SERVICE_SID_ASSUMED,
            }))
        );
        assert_eq!(locate_service(&lookup(LandType(101))), None);
    }

    #[test]
    fn a_land_join_names_its_place_and_other_joins_do_not() {
        let join = |kind, land_type, parameter| JoinNet {
            cookie: Cookie(0),
            kind,
            land_type,
            parameter,
            size: 100,
        };
        assert_eq!(
            place_of(&join(ObjectKind::LandGroup, LandType(4), 1)),
            Some(Place {
                land_type: LandType(4),
                land_number: 1
            })
        );
        assert_eq!(
            place_of(&join(ObjectKind::Group, RPG_OBJECT_LAND_TYPE, 3)),
            None
        );
    }

    #[test]
    fn a_title_land_admits_its_own_client_version_only() {
        assert!(admits(LandType(4), ClientVersion::new(1, 1, 7)));
        assert!(!admits(LandType(4), ClientVersion::new(1, 0, 22)));
        assert!(admits(LandType(8), ClientVersion::new(1, 0, 22)));
        assert!(admits(LandType(1), ClientVersion::new(2, 3, 18)));
    }

    #[test]
    fn titles_fit_the_hubs_place_list() {
        for title in RPG_TITLES {
            assert!(title.description.len() <= 16 && !title.description.contains(' '));
        }
    }
}
