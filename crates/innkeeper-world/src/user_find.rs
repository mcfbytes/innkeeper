//! Who is online. Command 49's reply, from the sessions signed on in the object store.
//!
//! `FindPerson::handleMsg` (`hub/script.126`) prints each record, or "not found" at count 0.

use tracing::info;

use crate::assumptions::{
    FIND_ALSO_MATCHES_LOGIN_NAME_ASSUMED, FIND_RESULTS_IN_CONNECTION_ORDER_ASSUMED,
};
use crate::objects::OnlinePlayer;
use crate::presence::waiting_room;
use crate::{
    ConnectionId, FoundUser, HostMessage, LandNumber, LandType, ObjectStore, UserFind,
    UserFindReply, World,
};

/// Land type 0, land number 0: no directory row matches, so the client prints "the Map".
const ON_THE_MAP: (LandType, LandNumber) = (LandType(0), LandNumber(0));

/// The reply the requester is waiting on. A miss is count 0, not silence.
pub(crate) fn answer(world: &World, objects: &ObjectStore, request: &UserFind) -> Vec<HostMessage> {
    let found = find(world, objects, request);
    info!(hits = found.len(), "phone book answered");
    let reply = match request {
        UserFind::ByAccount(account) => UserFindReply::ByAccount {
            account: *account,
            found,
        },
        UserFind::ByName(_) => UserFindReply::ByName { found },
    };
    vec![HostMessage::UserFind(reply)]
}

fn find(world: &World, objects: &ObjectStore, request: &UserFind) -> Vec<FoundUser> {
    let mut hits: Vec<_> = objects
        .online_players()
        .filter(|(_, player)| selected(player, request))
        .map(|(connection, player)| (connection, located(world, objects, connection, player)))
        .collect();
    let hits = match FIND_RESULTS_IN_CONNECTION_ORDER_ASSUMED {
        true => {
            hits.sort_by_key(|(connection, _)| *connection);
            hits
        }
        false => hits,
    };
    hits.into_iter().map(|(_, user)| user).collect()
}

fn selected(player: &OnlinePlayer, request: &UserFind) -> bool {
    match request {
        UserFind::ByAccount(account) => player.account == *account,
        UserFind::ByName(name) => {
            let persona = same_name(&player.persona, name);
            let login_name = match FIND_ALSO_MATCHES_LOGIN_NAME_ASSUMED {
                true => same_name(&player.login_name, name),
                false => false,
            };
            persona || login_name
        }
    }
}

fn same_name(stored: &str, query: &str) -> bool {
    stored.trim().eq_ignore_ascii_case(query.trim())
}

fn located(
    world: &World,
    objects: &ObjectStore,
    connection: ConnectionId,
    player: &OnlinePlayer,
) -> FoundUser {
    let (land_type, land_number) = land_of(objects, connection, player);
    FoundUser {
        host: world.host,
        land_type,
        land_number,
        name: player.persona.clone(),
    }
}

/// The place from the latest land joinNet, and only while this connection still holds a member of
/// that land's waiting room. Leaving the group does not clear the place the map groups use.
fn land_of(
    objects: &ObjectStore,
    connection: ConnectionId,
    player: &OnlinePlayer,
) -> (LandType, LandNumber) {
    let Some(place) = player.place else {
        return ON_THE_MAP;
    };
    let Ok(number) = u8::try_from(place.land_number) else {
        return ON_THE_MAP;
    };
    let land_number = LandNumber(number);
    let room = waiting_room(place.land_type, land_number);
    match objects.holds_member_of(connection, room) {
        true => (place.land_type, land_number),
        false => ON_THE_MAP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountId, AccountRecord, ClientMessage, ClientVersion, ConnectionId, Cookie, Delivery,
        EncodedPassword, GroupJoin, GroupKey, GroupLeave, HostMessage, JoinNet, LandType, Login,
        MemoryStore, ObjectKind, ObjectStore, PasswordSource, Place, PlayerSession, SetPersona,
        Sid, Store, UserFindReply, World,
    };
    use std::sync::Arc;

    const A: ConnectionId = ConnectionId(1);
    const B: ConnectionId = ConnectionId(2);

    fn login(account: u32, name: &str) -> ClientMessage {
        ClientMessage::Login(Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(account),
            password_source: PasswordSource::StoredFile,
            password: EncodedPassword([7; 10]),
            name: name.into(),
            prodigy_id: None,
        })
    }

    fn find(
        player: &mut PlayerSession,
        world: &World,
        objects: &mut ObjectStore,
        connection: ConnectionId,
        request: UserFind,
    ) -> UserFindReply {
        let deliveries = player.handle(
            world,
            objects,
            connection,
            &ClientMessage::UserFind(request),
        );
        assert!(deliveries.iter().all(|delivery| delivery.to == connection));
        let [delivery] = deliveries.as_slice() else {
            panic!("expected one reply, got {deliveries:?}");
        };
        let HostMessage::UserFind(reply) = &delivery.message else {
            panic!("expected a find reply, got {delivery:?}");
        };
        reply.clone()
    }

    fn sid_of(deliveries: Vec<Delivery>) -> Sid {
        let mut ids = deliveries
            .into_iter()
            .filter_map(|delivery| match delivery.message {
                HostMessage::ObjId { sid, .. } => Some(sid),
                _ => None,
            });
        let sid = ids.next().expect("joinNet answers with ObjId");
        assert!(ids.next().is_none());
        sid
    }

    fn join(cookie: u16, kind: ObjectKind, land_type: u8, parameter: u16, size: u16) -> JoinNet {
        JoinNet {
            cookie: Cookie(cookie),
            kind,
            land_type: LandType(land_type),
            parameter,
            size,
        }
    }

    #[test]
    fn logged_in_players_are_found_by_name_and_by_account_and_a_miss_is_empty() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut searcher = PlayerSession::new();
        assert!(searcher
            .handle(
                &world,
                &mut objects,
                B,
                &ClientMessage::UserFind(UserFind::ByName("Guybrush".into()))
            )
            .is_empty());

        let mut guybrush = PlayerSession::new();
        let _ = guybrush.handle(&world, &mut objects, A, &login(100_001, "Guybrush"));
        let _ = searcher.handle(&world, &mut objects, B, &login(100_002, "Elaine"));
        let player = sid_of(guybrush.handle(
            &world,
            &mut objects,
            A,
            &ClientMessage::JoinNet(join(1, ObjectKind::Object, 1, 0, 30)),
        ));
        let room = sid_of(guybrush.handle(
            &world,
            &mut objects,
            A,
            &ClientMessage::JoinNet(join(2, ObjectKind::LandGroup, 1, 1, 128)),
        ));
        // The land joinNet sets the place, and the player is not a member until the GrpJoin.
        let before = find(
            &mut searcher,
            &world,
            &mut objects,
            B,
            UserFind::ByName("  gUyBrUsH ".into()),
        );
        assert_eq!(
            before,
            UserFindReply::ByName {
                found: vec![on_map(&world, "Guybrush")]
            }
        );

        let _ = guybrush.handle(
            &world,
            &mut objects,
            A,
            &ClientMessage::GroupJoin(GroupJoin {
                group: room,
                member: player,
                version: None,
            }),
        );
        let directory_host = world.lands.directory(world.host).lands[0].host;
        assert_eq!(directory_host, world.host);
        let by_name = find(
            &mut searcher,
            &world,
            &mut objects,
            B,
            UserFind::ByName("guybrush".into()),
        );
        assert_eq!(
            by_name,
            UserFindReply::ByName {
                found: vec![FoundUser {
                    host: directory_host,
                    land_type: LandType(1),
                    land_number: LandNumber(1),
                    name: "Guybrush".into(),
                }]
            }
        );
        let self_find = find(
            &mut searcher,
            &world,
            &mut objects,
            B,
            UserFind::ByAccount(AccountId(100_002)),
        );
        assert_eq!(
            self_find,
            UserFindReply::ByAccount {
                account: AccountId(100_002),
                found: vec![on_map(&world, "Elaine")],
            }
        );
        assert_eq!(
            find(
                &mut searcher,
                &world,
                &mut objects,
                B,
                UserFind::ByName("nobody".into())
            ),
            UserFindReply::ByName { found: Vec::new() }
        );
        assert_eq!(
            find(
                &mut searcher,
                &world,
                &mut objects,
                B,
                UserFind::ByAccount(AccountId(42))
            ),
            UserFindReply::ByAccount {
                account: AccountId(42),
                found: Vec::new(),
            }
        );

        let _ = guybrush.handle(
            &world,
            &mut objects,
            A,
            &ClientMessage::GroupLeave(GroupLeave {
                group: room,
                member: player,
            }),
        );
        let left = find(
            &mut searcher,
            &world,
            &mut objects,
            B,
            UserFind::ByAccount(AccountId(100_001)),
        );
        assert_eq!(
            left,
            UserFindReply::ByAccount {
                account: AccountId(100_001),
                found: vec![on_map(&world, "Guybrush")],
            }
        );
        // The place the RPG map groups use is still the land, after the group leave.
        let map = sid_of(guybrush.handle(
            &world,
            &mut objects,
            A,
            &ClientMessage::JoinNet(join(3, ObjectKind::Group, 0x8B, 5, 104)),
        ));
        assert_eq!(
            objects.group_key(map),
            Some(GroupKey {
                kind: ObjectKind::Group,
                land_type: LandType(0x8B),
                parameter: 5,
                within: Some(Place {
                    land_type: LandType(1),
                    land_number: 1,
                }),
            })
        );

        let _ = objects.disconnect(A);
        assert_eq!(
            find(
                &mut searcher,
                &world,
                &mut objects,
                B,
                UserFind::ByName("Guybrush".into())
            ),
            UserFindReply::ByName { found: Vec::new() }
        );
        assert_eq!(
            find(
                &mut searcher,
                &world,
                &mut objects,
                B,
                UserFind::ByName("Elaine".into())
            ),
            UserFindReply::ByName {
                found: vec![on_map(&world, "Elaine")]
            }
        );
    }

    #[test]
    fn a_renamed_player_matches_the_login_name_and_the_persona() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut guybrush = PlayerSession::new();
        let mut searcher = PlayerSession::new();
        let _ = guybrush.handle(&world, &mut objects, A, &login(100_001, "Guybrush"));
        let _ = searcher.handle(&world, &mut objects, B, &login(100_002, "Elaine"));
        let _ = guybrush.handle(
            &world,
            &mut objects,
            A,
            &ClientMessage::SetPersona(SetPersona {
                name: "Threepwood".into(),
            }),
        );
        let by_login = find(
            &mut searcher,
            &world,
            &mut objects,
            B,
            UserFind::ByName("GUYBRUSH".into()),
        );
        let by_persona = find(
            &mut searcher,
            &world,
            &mut objects,
            B,
            UserFind::ByName("  threepwood ".into()),
        );
        let printed = UserFindReply::ByName {
            found: vec![on_map(&world, "Threepwood")],
        };
        assert_eq!(by_login, printed);
        assert_eq!(by_persona, printed);
    }

    #[test]
    fn an_account_that_is_only_in_the_store_is_not_found() {
        let store = Arc::new(MemoryStore::new());
        store
            .create_account(
                AccountId(100_003),
                AccountRecord::new(EncodedPassword([7; 10]), "offline"),
            )
            .unwrap();
        let world = World::stored(store);
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &mut objects, A, &login(100_001, "Guybrush"));
        assert_eq!(
            find(
                &mut player,
                &world,
                &mut objects,
                A,
                UserFind::ByName("offline".into())
            ),
            UserFindReply::ByName { found: Vec::new() }
        );
        assert_eq!(
            find(
                &mut player,
                &world,
                &mut objects,
                A,
                UserFind::ByAccount(AccountId(100_003))
            ),
            UserFindReply::ByAccount {
                account: AccountId(100_003),
                found: Vec::new(),
            }
        );
    }

    fn on_map(world: &World, name: &str) -> FoundUser {
        FoundUser {
            host: world.host,
            land_type: LandType(0),
            land_number: LandNumber(0),
            name: name.into(),
        }
    }
}
