//! Command 41/0: is this conference name taken?
//!
//! `GotoConference::handleMsg` reads the word at byte 6. Zero means the name is free and the
//! script moves on; any other SID means the name is taken (`hub/script.120`). The check does not
//! create the conference.

use crate::assumptions::CONFERENCE_KEY_IS_THE_RAW_TAIL_ASSUMED;
use crate::{HostMessage, ObjExists, ObjectLocated, ObjectStore};

pub(crate) fn locate(objects: &ObjectStore, lookup: &ObjExists) -> Option<HostMessage> {
    let ObjExists::Name { to, name, .. } = lookup else {
        return None;
    };
    let _raw_tail = CONFERENCE_KEY_IS_THE_RAW_TAIL_ASSUMED;
    Some(HostMessage::ObjectLocated(ObjectLocated {
        to: *to,
        sid: objects.conference_sid(name),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountId, ClientMessage, ClientVersion, ConnectionId, EncodedPassword, Login,
        ObjectLocated, ObjectStore, PasswordSource, PlayerSession, Sid, World,
    };

    const ME: ConnectionId = ConnectionId(1);
    const TAIL: &[u8] = &[1, 15, 0];

    fn logged_in() -> (World, ObjectStore, PlayerSession) {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let login = ClientMessage::Login(Login {
            land_type: crate::LandType(1),
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

    fn name_check(to: Sid) -> ClientMessage {
        ClientMessage::ObjExists(ObjExists::Name {
            to,
            from: to,
            name: TAIL.to_vec(),
        })
    }

    fn located(
        player: &mut PlayerSession,
        world: &World,
        objects: &mut ObjectStore,
    ) -> ObjectLocated {
        let replies = player.handle(world, objects, ME, &name_check(Sid(0x0102)));
        let Some(crate::Delivery {
            message: HostMessage::ObjectLocated(located),
            ..
        }) = replies.first()
        else {
            panic!("a name check answers with the SID at word 6, got {replies:?}");
        };
        *located
    }

    #[test]
    fn a_free_name_is_sid_zero_and_a_bound_name_is_its_object() {
        let (world, mut objects, mut player) = logged_in();
        let free = located(&mut player, &world, &mut objects);
        assert_eq!(
            free,
            ObjectLocated {
                to: Sid(0x0102),
                sid: Sid(0)
            }
        );
        assert!(objects.is_empty());

        let bound = objects.bind_conference(ME, TAIL).unwrap();
        let taken = located(&mut player, &world, &mut objects);
        assert_eq!(taken.sid, bound);
        assert_ne!(taken.sid, Sid(0));

        let _ = objects.leave(ME, bound);
        let freed = located(&mut player, &world, &mut objects);
        assert_eq!(freed.sid, Sid(0));
    }
}
