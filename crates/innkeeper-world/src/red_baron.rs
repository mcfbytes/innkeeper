//! Red Baron's service object: what `RB.EXE` asks for at start-up before it joins the game group.
//! The wire layouts and what each wait does are in docs/protocol/int14h-census/redbaron.md sections 5
//! and 6; the host's side is docs/server/games.md section 6.

use tracing::info;

use crate::assumptions::{RED_BARON_GAME_MINUTES_ASSUMED, RED_BARON_SERVICE_SID_ASSUMED};
use crate::{
    HostMessage, IntProperty, LandType, ObjExists, ObjectLocated, ServicePropertyRequest, SetInt,
};

/// The land type of everything Red Baron creates: the service lookup, the in-game object, the queue's group.
pub(crate) const RED_BARON_LAND_TYPE: LandType = LandType(101);

/// Property 9 of the service object: the version of the game that the host serves. The client
/// refuses to fly unless it reads `0x0201` (high byte 2, low byte 1).
const VERSION_PROPERTY: u16 = 9;
const VERSION: u16 = 0x0201;
/// Property 0xF: the length of a game, in minutes; the client clamps it to 5..=30.
const LENGTH_PROPERTY: u16 = 0xF;

/// The answer to a 41/2 lookup of Red Baron's service; other lookups are not ours.
pub(crate) fn locate_service(lookup: &ObjExists) -> Option<HostMessage> {
    match *lookup {
        ObjExists::Service { user, land_type } if land_type == RED_BARON_LAND_TYPE => {
            info!(user = user.0, "Red Baron service located");
            Some(HostMessage::ObjectLocated(ObjectLocated {
                to: user,
                sid: RED_BARON_SERVICE_SID_ASSUMED,
            }))
        }
        ObjExists::Service { .. } | ObjExists::Name { .. } => None,
    }
}

/// The answers to a 17/0 read of the service object: one setInt per requested property whose first
/// pair is that property, since the client reads only the first pair and waits for each reply.
pub(crate) fn service_properties(request: &ServicePropertyRequest) -> Vec<HostMessage> {
    if request.service != RED_BARON_SERVICE_SID_ASSUMED || request.sub != 0 {
        return Vec::new();
    }
    let answers = request.properties.iter();
    answers
        .filter_map(|&offset| {
            let value = match offset {
                VERSION_PROPERTY => VERSION,
                LENGTH_PROPERTY => RED_BARON_GAME_MINUTES_ASSUMED,
                other => {
                    info!(
                        property = other,
                        "Red Baron service property unknown: no answer"
                    );
                    return None;
                }
            };
            Some(HostMessage::SetInt(SetInt {
                target: request.service,
                from: request.service,
                properties: vec![IntProperty { offset, value }],
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sid;

    fn read(sub: u8, service: Sid, properties: &[u16]) -> Vec<HostMessage> {
        service_properties(&ServicePropertyRequest {
            sub,
            service,
            from: Sid(0x0100),
            properties: properties.to_vec(),
        })
    }

    #[test]
    fn the_version_and_the_length_are_answered_one_property_each() {
        let service = RED_BARON_SERVICE_SID_ASSUMED;
        let answers = read(0, service, &[9, 0xF]);
        let pairs: Vec<(u16, u16)> = answers
            .iter()
            .map(|answer| match answer {
                HostMessage::SetInt(set) => (set.properties[0].offset, set.properties[0].value),
                other => panic!("expected a setInt, got {other:?}"),
            })
            .collect();
        assert_eq!(
            pairs,
            vec![(9, 0x0201), (0xF, RED_BARON_GAME_MINUTES_ASSUMED)]
        );
        assert!((5..=30).contains(&RED_BARON_GAME_MINUTES_ASSUMED));
    }

    #[test]
    fn other_services_and_subs_get_no_answer() {
        assert!(read(0, Sid(0x008B), &[9]).is_empty());
        assert!(read(2, RED_BARON_SERVICE_SID_ASSUMED, &[9]).is_empty());
        assert!(read(0, RED_BARON_SERVICE_SID_ASSUMED, &[1]).is_empty());
    }

    #[test]
    fn only_the_red_baron_land_type_is_located() {
        let lookup = |land_type| ObjExists::Service {
            user: Sid(0x0100),
            land_type,
        };
        assert!(locate_service(&lookup(RED_BARON_LAND_TYPE)).is_some());
        assert!(locate_service(&lookup(LandType(0x8B))).is_none());
    }
}
