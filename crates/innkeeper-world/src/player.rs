use tracing::{info, info_span, warn};

use crate::{
    logon, Account, ClientMessage, ConnectionId, Delivery, HostInfoRequest, HostMessage, Login,
    ObjectStore, World,
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
            ClientMessage::JoinNet(join) => self.when_logged_in(|_| objects.join(connection, join)),
            ClientMessage::LeaveNet(sid) => self.when_logged_in(|_| objects.leave(connection, sid)),
            ClientMessage::GroupJoin(join) => {
                self.when_logged_in(|_| objects.join_group(connection, join))
            }
            ClientMessage::GroupLeave(leave) => {
                self.when_logged_in(|_| objects.leave_group(connection, leave))
            }
            ClientMessage::GroupMembers(request) => {
                self.when_logged_in(|_| objects.list_members(connection, request))
            }
            ClientMessage::ChangePassword(change) => self.when_logged_in(|account| {
                reply(vec![logon::change_password(world, account, change)])
            }),
            ClientMessage::HostInfo(request) => {
                self.when_logged_in(|_| reply(answer_host_info(world, request)))
            }
            ClientMessage::LandOccupancyRequest => self.when_logged_in(|_| {
                reply(vec![HostMessage::LandOccupancy(
                    world.lands.occupancy(world.host),
                )])
            }),
            ClientMessage::Send(_)
            | ClientMessage::SetInt(_)
            | ClientMessage::SetStr(_)
            | ClientMessage::Multicast(_)
            | ClientMessage::ObjExists(_) => {
                info!(?message, "decoded, no handler yet: ignored");
                Vec::new()
            }
        }
    }

    fn when_logged_in(&self, answer: impl FnOnce(&Account) -> Vec<Delivery>) -> Vec<Delivery> {
        match &self.state {
            PlayerState::LoggedIn(player) => {
                let _player = info_span!("player", persona = %player.account.persona).entered();
                answer(&player.account)
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
                let ack = account.login_ack();
                self.state = PlayerState::LoggedIn(Player { account });
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
        AccountBook, AccountId, ClientVersion, Cookie, EncodedPassword, JoinNet, LandCatalog,
        LandType, LoginNakReason, ObjectKind, PasswordSource, SendMessage, Sid,
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
    fn decoded_shared_object_commands_are_ignored_until_a_handler_exists() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &mut objects, ME, &login(7));
        let relay = ClientMessage::Send(SendMessage {
            to: Sid(0x0100),
            from: Sid(0),
            payload: vec![1, 0],
        });
        assert!(player.handle(&world, &mut objects, ME, &relay).is_empty());
    }
}
