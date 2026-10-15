use tracing::{info, info_span, warn};

use crate::assumptions::UNSET_RATING_GRANTS_EVERY_LAND_ASSUMED;
use crate::{
    bbs, logon, mail, money, presence, red_baron, router, rpg, Account, ClientMessage,
    ConnectionId, Delivery, HostInfoRequest, HostMessage, JoinNet, Login, ObjectStore, Place,
    Unexplained, UnexplainedCommand, World,
};

/// The host side of one logged-in client, from Login to hang-up.
#[derive(Debug)]
pub struct PlayerSession {
    state: PlayerState,
}

#[derive(Debug)]
enum PlayerState {
    AwaitingLogin,
    LoggedIn(Player),
}

#[derive(Debug)]
struct Player {
    account: Account,
    /// The land place of the player's latest land joinNet; the RPG clients' map groups and boards
    /// belong to it.
    place: Option<Place>,
}

impl Default for PlayerSession {
    fn default() -> Self {
        PlayerSession::new()
    }
}

impl PlayerSession {
    pub fn new() -> Self {
        PlayerSession {
            state: PlayerState::AwaitingLogin,
        }
    }

    /// Everything one client message causes: the sender's replies in request order, and the
    /// notices for other connections where they happened.
    #[must_use]
    pub fn handle(
        &mut self,
        world: &World,
        objects: &mut ObjectStore,
        connection: ConnectionId,
        message: &ClientMessage,
    ) -> Vec<Delivery> {
        let reply = |messages: Vec<HostMessage>| replies_to(connection, messages);
        match *message {
            ClientMessage::Login(ref login) => {
                let mut deliveries = objects.disconnect(connection);
                deliveries.extend(reply(vec![self.log_in(world, login)]));
                deliveries
            }
            ClientMessage::JoinNet(join) => self.join_net(objects, connection, join),
            ClientMessage::LeaveNet(sid) => self.when_logged_in(|_| objects.leave(connection, sid)),
            ClientMessage::GroupJoin(join) => self
                .when_logged_in(|_| presence::join_group(&world.lands, objects, connection, join)),
            ClientMessage::GroupLeave(leave) => {
                self.when_logged_in(|_| objects.leave_group(connection, leave))
            }
            ClientMessage::GroupMembers(request) => {
                self.when_logged_in(|_| objects.list_members(connection, request))
            }
            ClientMessage::ChangePassword(change) => self.when_logged_in(|player| {
                reply(vec![logon::change_password(world, &player.account, change)])
            }),
            ClientMessage::HostInfo(request) => {
                self.when_logged_in(|_| reply(answer_host_info(world, request)))
            }
            ClientMessage::LandOccupancyRequest => self.when_logged_in(|_| {
                let occupancy = world.lands.occupancy(world.host, objects);
                reply(vec![HostMessage::LandOccupancy(occupancy)])
            }),
            ClientMessage::Send(ref relayed) => {
                self.when_logged_in(|_| router::send(objects, connection, relayed))
            }
            ClientMessage::Multicast(ref relayed) => {
                self.when_logged_in(|_| router::multicast(objects, connection, relayed))
            }
            ClientMessage::SetPersona(ref set) => {
                self.set_persona(&set.name);
                Vec::new()
            }
            ClientMessage::ReadMoney(request) => {
                self.when_logged_in(|player| reply(money::read(world, &player.account, request)))
            }
            ClientMessage::SaveMoney(request) => {
                self.when_logged_in(|player| reply(money::save(world, &player.account, request)))
            }
            ClientMessage::NewMailbox(request) => self.when_logged_in(|player| {
                reply(vec![mail::assign_mailbox(world, &player.account, request)])
            }),
            ClientMessage::Mail(ref request) => {
                self.when_logged_in(|player| reply(mail::answer(world, &player.account, request)))
            }
            ClientMessage::Bbs(ref request) => self.when_logged_in(|player| {
                reply(bbs::answer(world, &player.account, player.place, request))
            }),
            ClientMessage::SetInt(ref set) => {
                self.when_logged_in(|_| router::set_int(objects, connection, set))
            }
            ClientMessage::SetStr(ref set) => {
                self.when_logged_in(|_| router::set_str(objects, connection, set))
            }
            ClientMessage::InvokeMethod(ref call) => {
                self.when_logged_in(|_| router::invoke_method(objects, connection, call))
            }
            ClientMessage::Lock(ref request) => {
                self.when_logged_in(|_| objects.lock(connection, request))
            }
            ClientMessage::Unlock(ref request) => {
                self.when_logged_in(|_| objects.unlock(connection, request))
            }
            ClientMessage::GetProperties(ref request) => {
                self.when_logged_in(|_| objects.properties(connection, request))
            }
            ClientMessage::GetMemberProperties(ref request) => {
                self.when_logged_in(|_| objects.member_properties(connection, request))
            }
            ClientMessage::ServiceProperties(ref request) => self.when_logged_in(|_| {
                let answers = red_baron::service_properties(request);
                if answers.is_empty() {
                    info!(?message, "no reply awaited: absorbed");
                }
                reply(answers)
            }),
            ClientMessage::PlayStatus(status) => {
                info!(?status, "play status noted");
                Vec::new()
            }
            ClientMessage::Unexplained(Unexplained {
                command: UnexplainedCommand::Register,
                ref sids,
            }) => self.when_logged_in(|_| {
                objects.register(connection, sids);
                Vec::new()
            }),
            ClientMessage::Unexplained(_) => {
                info!(?message, "purpose unknown, no reply expected: absorbed");
                Vec::new()
            }
            ClientMessage::ObjExists(ref lookup) => self.when_logged_in(|_| {
                let located =
                    rpg::locate_service(lookup).or_else(|| red_baron::locate_service(lookup));
                if located.is_none() {
                    info!(?message, "decoded, no handler yet: ignored");
                }
                reply(located.into_iter().collect())
            }),
        }
    }

    /// The persona the player plays now; none before the Login is accepted.
    pub fn persona(&self) -> Option<&str> {
        match &self.state {
            PlayerState::LoggedIn(player) => Some(&player.account.persona),
            PlayerState::AwaitingLogin => None,
        }
    }

    fn set_persona(&mut self, name: &str) {
        match &mut self.state {
            PlayerState::LoggedIn(player) => {
                info!(from = %player.account.persona, to = name, "persona renamed");
                name.clone_into(&mut player.account.persona);
            }
            PlayerState::AwaitingLogin => warn!("message before Login ignored"),
        }
    }

    /// joinNet: a land group also becomes the player's place, which scopes the map groups after it.
    fn join_net(
        &mut self,
        objects: &mut ObjectStore,
        connection: ConnectionId,
        join: JoinNet,
    ) -> Vec<Delivery> {
        let PlayerState::LoggedIn(player) = &mut self.state else {
            warn!("message before Login ignored");
            return Vec::new();
        };
        if let Some(place) = rpg::place_of(&join) {
            player.place = Some(place);
        }
        rpg::join(objects, connection, join, player.place)
    }

    fn when_logged_in(&self, answer: impl FnOnce(&Player) -> Vec<Delivery>) -> Vec<Delivery> {
        match &self.state {
            PlayerState::LoggedIn(player) => {
                let _player = info_span!("player", persona = %player.account.persona).entered();
                answer(player)
            }
            PlayerState::AwaitingLogin => {
                warn!("message before Login ignored");
                Vec::new()
            }
        }
    }

    fn log_in(&mut self, world: &World, login: &Login) -> HostMessage {
        match logon::admit(world, login) {
            Ok(account) => {
                info!(account = account.id.0, persona = %account.persona, "logged in");
                let mut ack = account.login_ack();
                if ack.rating == 0 && UNSET_RATING_GRANTS_EVERY_LAND_ASSUMED {
                    ack.rating = world.lands.rights();
                }
                self.state = PlayerState::LoggedIn(Player {
                    account,
                    place: None,
                });
                HostMessage::LoginAccepted(ack)
            }
            Err(nak) => HostMessage::Nak(nak),
        }
    }
}

fn replies_to(connection: ConnectionId, messages: Vec<HostMessage>) -> Vec<Delivery> {
    let replies = messages.into_iter();
    replies
        .map(|message| Delivery::new(connection, message))
        .collect()
}

fn answer_host_info(world: &World, request: HostInfoRequest) -> Vec<HostMessage> {
    match request {
        HostInfoRequest::HostNumber => vec![HostMessage::HostNumber(world.host)],
        HostInfoRequest::HostTime => vec![HostMessage::HostTime(world.clock.now())],
        HostInfoRequest::LandDirectory { .. } => {
            vec![HostMessage::LandDirectory(
                world.lands.directory(world.host),
            )]
        }
        HostInfoRequest::HostAddressFile { .. } => {
            info!(?request, "no reply: the client keeps what it has");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountBook, AccountId, ClientVersion, Cookie, EncodedPassword, IntProperty, JoinNet,
        LandCatalog, LandType, LoginNakReason, ObjectKind, PasswordSource, SendMessage, SetInt,
        Sid,
    };

    const ME: ConnectionId = ConnectionId(1);

    fn login(password: u8) -> ClientMessage {
        ClientMessage::Login(Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(100_001),
            password_source: PasswordSource::StoredFile,
            password: EncodedPassword([password; 10]),
            name: "guybrush".into(),
            prodigy_id: None,
        })
    }

    fn join_game_object() -> ClientMessage {
        ClientMessage::JoinNet(JoinNet {
            cookie: Cookie(1),
            kind: ObjectKind::GameObject,
            land_type: LandType(1),
            parameter: 0,
            size: 12,
        })
    }

    fn world_with_one_account() -> World {
        World {
            host: World::stock().host,
            accounts: AccountBook::listed([(AccountId(100_001), EncodedPassword([7; 10]))]),
            lands: LandCatalog::stock(),
            ..World::stock()
        }
    }

    fn messages(deliveries: Vec<Delivery>) -> Vec<HostMessage> {
        assert!(deliveries.iter().all(|delivery| delivery.to == ME));
        deliveries
            .into_iter()
            .map(|delivery| delivery.message)
            .collect()
    }

    #[test]
    fn a_wrong_password_asks_the_client_to_try_again() {
        let world = world_with_one_account();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let replies = messages(player.handle(&world, &mut objects, ME, &login(8)));
        let [HostMessage::Nak(nak)] = replies.as_slice() else {
            panic!("expected one Nak, got {replies:?}");
        };
        assert_eq!(nak.which_sub, LoginNakReason::RetryPassword as u8);
        assert!(player
            .handle(&world, &mut objects, ME, &join_game_object())
            .is_empty());
        assert!(matches!(
            messages(player.handle(&world, &mut objects, ME, &login(7))).as_slice(),
            [HostMessage::LoginAccepted(_)]
        ));
    }

    #[test]
    fn a_repeat_login_on_a_logged_in_session_admits_again() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &mut objects, ME, &login(7));
        let _ = player.handle(&world, &mut objects, ME, &join_game_object());
        let replies = messages(player.handle(&world, &mut objects, ME, &login(7)));
        assert!(matches!(
            replies.as_slice(),
            [HostMessage::LoginAccepted(_)]
        ));
    }

    #[test]
    fn requests_before_login_get_no_reply() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let request = ClientMessage::HostInfo(HostInfoRequest::HostNumber);
        assert!(player.handle(&world, &mut objects, ME, &request).is_empty());
    }

    #[test]
    fn a_second_login_releases_what_the_first_one_held() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &mut objects, ME, &login(7));
        let _ = player.handle(&world, &mut objects, ME, &join_game_object());
        let _ = player.handle(&world, &mut objects, ME, &login(7));
        let leave = ClientMessage::LeaveNet(Sid(0x0100));
        assert!(player.handle(&world, &mut objects, ME, &leave).is_empty());
        assert!(objects.is_empty());
    }

    #[test]
    fn a_property_update_nobody_else_replicates_goes_nowhere() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &mut objects, ME, &login(7));
        let _ = player.handle(&world, &mut objects, ME, &join_game_object());
        let set = ClientMessage::SetInt(SetInt {
            target: Sid(0x0100),
            from: Sid(0x0100),
            properties: vec![IntProperty {
                offset: 1,
                value: 2,
            }],
        });
        assert!(player.handle(&world, &mut objects, ME, &set).is_empty());
    }

    #[test]
    fn a_send_to_a_held_object_comes_back_to_its_holder() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &mut objects, ME, &login(7));
        let _ = player.handle(&world, &mut objects, ME, &join_game_object());
        let relay = SendMessage {
            to: Sid(0x0100),
            from: Sid(0),
            payload: vec![1, 0],
        };
        let sent = ClientMessage::Send(relay.clone());
        let replies = messages(player.handle(&world, &mut objects, ME, &sent));
        assert_eq!(replies, vec![HostMessage::Send(relay)]);
    }
}
