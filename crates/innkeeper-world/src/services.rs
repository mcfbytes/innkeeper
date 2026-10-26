//! The host's service objects and the commands addressed to them: the 41/2 lookup, the 17 property
//! read and the 50 play status. The contract is docs/server/game-services.md section 2.

use tracing::info;

use crate::assumptions::{RED_BARON_SERVICE_SID_ASSUMED, RPG_SERVICE_SID_ASSUMED};
use crate::red_baron::RED_BARON_LAND_TYPE;
use crate::rpg::RPG_OBJECT_LAND_TYPE;
use crate::{
    conference, red_baron, rpg, HostMessage, LandType, LogPolicy, ObjExists, ObjectStore,
    PlayStatus, ServicePropertyRequest, Sid,
};

/// A well-known object a DOS program finds with 41/2 by its land type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceObject {
    /// Yserbius, Cawdor and Twinion: located, never read.
    Rpg,
    /// Red Baron: located, then properties 9 and 0xF read with command 17.
    RedBaron,
}

impl ServiceObject {
    pub const ALL: [ServiceObject; 2] = [ServiceObject::Rpg, ServiceObject::RedBaron];

    /// The land type a 41/2 names to find it.
    pub fn land_type(self) -> LandType {
        match self {
            ServiceObject::Rpg => RPG_OBJECT_LAND_TYPE,
            ServiceObject::RedBaron => RED_BARON_LAND_TYPE,
        }
    }

    /// The SID the lookup returns and later requests address.
    pub fn sid(self) -> Sid {
        match self {
            ServiceObject::Rpg => RPG_SERVICE_SID_ASSUMED,
            ServiceObject::RedBaron => RED_BARON_SERVICE_SID_ASSUMED,
        }
    }

    pub fn by_land_type(land_type: LandType) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|service| service.land_type() == land_type)
    }

    pub fn by_sid(sid: Sid) -> Option<Self> {
        Self::ALL.into_iter().find(|service| service.sid() == sid)
    }
}

/// Command 41: a service lookup goes to the service object of its land type, a name check to the
/// conferences. None when nothing answers.
pub(crate) fn locate(objects: &ObjectStore, lookup: &ObjExists) -> Option<HostMessage> {
    match *lookup {
        ObjExists::Service { land_type, .. } => match ServiceObject::by_land_type(land_type)? {
            ServiceObject::Rpg => rpg::locate_service(lookup),
            ServiceObject::RedBaron => red_baron::locate_service(lookup),
        },
        ObjExists::Name { .. } => conference::locate(objects, lookup),
    }
}

/// Command 17: the answers of the service object it addresses; empty for one that is not read.
pub(crate) fn properties(request: &ServicePropertyRequest) -> Vec<HostMessage> {
    match ServiceObject::by_sid(request.service) {
        Some(ServiceObject::RedBaron) => red_baron::service_properties(request),
        Some(ServiceObject::Rpg) | None => Vec::new(),
    }
}

/// Command 50: noted, never answered.
pub(crate) fn play_status(log: LogPolicy, status: PlayStatus) {
    info!(status = %log.debug(&status), "play status noted");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectLocated, SetInt};

    const USER: Sid = Sid(0x0100);

    fn lookup(land_type: LandType) -> ObjExists {
        ObjExists::Service {
            user: USER,
            land_type,
        }
    }

    fn read(service: Sid, sub: u8, properties: &[u16]) -> ServicePropertyRequest {
        ServicePropertyRequest {
            sub,
            service,
            from: USER,
            properties: properties.to_vec(),
        }
    }

    #[test]
    fn each_service_object_is_found_by_its_land_type_and_its_sid() {
        for service in ServiceObject::ALL {
            assert_eq!(
                ServiceObject::by_land_type(service.land_type()),
                Some(service)
            );
            assert_eq!(ServiceObject::by_sid(service.sid()), Some(service));
            let located = locate(&ObjectStore::new(), &lookup(service.land_type()));
            assert_eq!(
                located,
                Some(HostMessage::ObjectLocated(ObjectLocated {
                    to: USER,
                    sid: service.sid(),
                }))
            );
        }
        assert_eq!(ServiceObject::RedBaron.sid(), Sid(0x0065));
        assert_eq!(ServiceObject::Rpg.land_type(), LandType(0x8B));
    }

    #[test]
    fn a_lookup_of_any_other_land_type_is_not_answered() {
        let objects = ObjectStore::new();
        for land_type in [LandType(102), LandType(2), LandType(0)] {
            assert_eq!(locate(&objects, &lookup(land_type)), None);
        }
    }

    #[test]
    fn a_name_check_goes_to_the_conferences() {
        let check = ObjExists::Name {
            to: USER,
            from: USER,
            name: vec![1, 15, 0],
        };
        let free = Some(HostMessage::ObjectLocated(ObjectLocated {
            to: USER,
            sid: Sid(0),
        }));
        assert_eq!(locate(&ObjectStore::new(), &check), free);
    }

    #[test]
    fn only_the_red_baron_service_answers_a_property_read() {
        let answers = properties(&read(ServiceObject::RedBaron.sid(), 0, &[9]));
        assert!(matches!(
            answers.as_slice(),
            [HostMessage::SetInt(SetInt { properties, .. })] if properties[0].value == 0x0201
        ));
        assert!(properties(&read(ServiceObject::Rpg.sid(), 2, &[8, 9, 10, 11])).is_empty());
        assert!(properties(&read(Sid(0x0300), 0, &[9])).is_empty());
    }
}
