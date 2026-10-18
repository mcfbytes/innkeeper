use tracing::{info, warn};

use crate::assumptions::{
    RPG_CHARACTER_NAME_NOT_STORED_ASSUMED, UNSET_RATING_GRANTS_EVERY_LAND_ASSUMED,
};
use crate::objects::OnlinePlayer;
use crate::{
    bbs, conference, game_info, hint_board, is_reserved_name, logon, mail, mall, money,
    object_list, presence, red_baron, router, rpg, signup, town_hall, trivia, user_find, Account,
    AccountId, AccountRequest, ClientMessage, ConnectionId, Delivery, HostInfoRequest, HostMessage,
    JoinNet, LogPolicy, Login, ObjectStore, Place, TriviaRequest, Unexplained, UnexplainedCommand,
    World,
};

/// The host side of one logged-in client, from Login to hang-up.
#[derive(Debug)]
pub struct PlayerSession {
    state: PlayerState,
    log: LogPolicy,
}

#[derive(Debug)]
enum PlayerState {
    AwaitingLogin,
    LoggedIn(Player),
}

#[derive(Debug)]
struct Player {
    account: Account,
    /// The name the Login presented. A 40/4 renames the persona and leaves this.
    login_name: String,
    /// The land place of the player's latest land joinNet; the RPG clients' map groups and boards
    /// belong to it. A group leave does not clear it.
    place: Option<Place>,
    /// True once the logon's 34/4 has been seen; later ones come from the Town Hall.
    logon_rates_seen: bool,
    /// Topic 1, 2 or 3 from the last trivia group join (hub 16/17/18, SierraLand 7/8/9).
    trivia_room: Option<u8>,
}

impl Player {
    fn signed_on(&self) -> OnlinePlayer {
        OnlinePlayer {
            account: self.account.id,
            login_name: self.login_name.clone(),
            persona: self.account.persona.clone(),
            place: self.place,
        }
    }
}

impl Default for PlayerSession {
    fn default() -> Self {
        PlayerSession::new()
    }
}

impl PlayerSession {
    pub fn new() -> Self {
        Self::with_log(LogPolicy::Verbose)
    }

    pub fn with_log(log: LogPolicy) -> Self {
        PlayerSession {
            state: PlayerState::AwaitingLogin,
            log,
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
        let deliveries = match *message {
            ClientMessage::Signup(ref form) => reply(signup::answer(world, form)),
            ClientMessage::Login(ref login) => {
                world.leave_trivia(connection);
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
                self.set_persona(world, &set.name);
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
                reply(bbs::answer(
                    world,
                    objects,
                    &player.account,
                    player.place,
                    request,
                ))
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
                    info!(message = %self.log.debug(message), "no reply awaited: absorbed");
                }
                reply(answers)
            }),
            ClientMessage::PlayStatus(status) => {
                info!(status = %self.log.debug(&status), "play status noted");
                Vec::new()
            }
            ClientMessage::Mall(ref request) => {
                self.when_logged_in(|_| reply(mall::answer(request)))
            }
            ClientMessage::Trivia(ref request) => self.trivia(world, connection, request),
            ClientMessage::ObjNewList(ref request) => self.when_logged_in(|_| {
                reply(vec![object_list::created(objects, connection, request)])
            }),
            ClientMessage::ObjFreeList(ref list) => {
                self.when_logged_in(|_| object_list::freed(objects, connection, list))
            }
            ClientMessage::Unexplained(Unexplained {
                command: UnexplainedCommand::Register,
                ref sids,
            }) => self.when_logged_in(|_| {
                objects.register(connection, sids);
                Vec::new()
            }),
            ClientMessage::Unexplained(_) => {
                info!(message = %self.log.debug(message), "purpose unknown, no reply expected: absorbed");
                Vec::new()
            }
            ClientMessage::ObjExists(ref lookup) => self.when_logged_in(|_| {
                let located = rpg::locate_service(lookup)
                    .or_else(|| red_baron::locate_service(lookup))
                    .or_else(|| conference::locate(objects, lookup));
                if located.is_none() {
                    info!(message = %self.log.debug(message), "decoded, no handler yet: ignored");
                }
                reply(located.into_iter().collect())
            }),
            ClientMessage::UserFind(ref request) => {
                self.when_logged_in(|_| reply(user_find::answer(world, objects, request)))
            }
            ClientMessage::Account(request) => reply(self.account_request(&request)),
            ClientMessage::Hint(request) => {
                self.when_logged_in(|_| reply(hint_board::answer(&request)))
            }
        };
        self.publish(objects, connection);
        deliveries
    }

    /// The persona the player plays now; none before the Login is accepted.
    pub fn persona(&self) -> Option<&str> {
        match &self.state {
            PlayerState::LoggedIn(player) => Some(&player.account.persona),
            PlayerState::AwaitingLogin => None,
        }
    }

    /// The account the Login opened; none before the Login is accepted.
    pub fn account(&self) -> Option<AccountId> {
        match &self.state {
            PlayerState::LoggedIn(player) => Some(player.account.id),
            PlayerState::AwaitingLogin => None,
        }
    }

    /// Keeps the phone book's copy of this session in step with the player.
    fn publish(&self, objects: &mut ObjectStore, connection: ConnectionId) {
        match &self.state {
            PlayerState::LoggedIn(player) => objects.sign_on(connection, player.signed_on()),
            PlayerState::AwaitingLogin => objects.sign_off(connection),
        }
    }

    /// The player takes the name for the session, and the store keeps it as the account's persona
    /// when it is one a board post can carry (`storable_persona`). A session whose latest land
    /// join was an RPG title's land (`rpg::is_title`) only renames: the name is a game character
    /// (`RPG_CHARACTER_NAME_NOT_STORED_ASSUMED`). A store that refuses is logged and the session
    /// still plays under the name. A staff or service name (`is_reserved_name`) is not taken.
    fn set_persona(&mut self, world: &World, name: &str) {
        match &mut self.state {
            PlayerState::LoggedIn(_) if is_reserved_name(name) => {
                // 40/4 has no reply to refuse with: the session keeps the name it had.
                warn!("persona rename refused: a staff or service name");
            }
            PlayerState::LoggedIn(player) => {
                info!(
                    from = %self.log.persona(player.account.id.0, &player.account.persona),
                    to = %self.log.persona(player.account.id.0, name),
                    "persona renamed"
                );
                name.clone_into(&mut player.account.persona);
                let in_rpg = player
                    .place
                    .is_some_and(|place| rpg::is_title(place.land_type));
                if !(RPG_CHARACTER_NAME_NOT_STORED_ASSUMED && in_rpg) && storable_persona(name) {
                    if let Err(error) = world.store.set_persona(player.account.id, name) {
                        warn!(account = player.account.id.0, %error, "persona not stored");
                    }
                }
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
        if let Some(room) = trivia::room_for(&join) {
            player.trivia_room = Some(room);
        }
        if let Some(place) = rpg::place_of(&join) {
            player.place = Some(place);
        }
        rpg::join(objects, connection, join, player.place)
    }

    fn trivia(
        &mut self,
        world: &World,
        connection: ConnectionId,
        request: &TriviaRequest,
    ) -> Vec<Delivery> {
        let PlayerState::LoggedIn(player) = &self.state else {
            warn!("message before Login ignored");
            return Vec::new();
        };
        let _player = self
            .log
            .player_span(player.account.id.0, &player.account.persona)
            .entered();
        let host = (u16::from(world.host.0), self.log);
        let now = world.clock.millis();
        world.with_trivia(|shows| {
            shows.on_request(connection, player.trivia_room, host, request, now)
        })
    }

    /// The Town Hall billing screens. The first 34/4 of a Login is the logon's and stays unanswered.
    fn account_request(&mut self, request: &AccountRequest) -> Vec<HostMessage> {
        match &mut self.state {
            PlayerState::LoggedIn(player) => {
                let _player = self
                    .log
                    .player_span(player.account.id.0, &player.account.persona)
                    .entered();
                town_hall::account_request(&player.account, &mut player.logon_rates_seen, request)
            }
            PlayerState::AwaitingLogin => {
                warn!("message before Login ignored");
                Vec::new()
            }
        }
    }

    fn when_logged_in(&self, answer: impl FnOnce(&Player) -> Vec<Delivery>) -> Vec<Delivery> {
        match &self.state {
            PlayerState::LoggedIn(player) => {
                let _player = self
                    .log
                    .player_span(player.account.id.0, &player.account.persona)
                    .entered();
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
                info!(
                    account = account.id.0,
                    persona = %self.log.persona(account.id.0, &account.persona),
                    "logged in"
                );
                let mut ack = account.login_ack();
                if ack.rating == 0 && UNSET_RATING_GRANTS_EVERY_LAND_ASSUMED {
                    ack.rating = world.lands.rights();
                }
                self.state = PlayerState::LoggedIn(Player {
                    login_name: login.name.clone(),
                    account,
                    place: None,
                    logon_rates_seen: false,
                    trivia_room: None,
                });
                HostMessage::LoginAccepted(ack)
            }
            Err(nak) => HostMessage::Nak(nak),
        }
    }
}

/// Longest persona the store keeps; the web page's own rule is shorter.
const STORED_PERSONA_MAX: usize = 16;

/// A persona the store keeps: 1 to 16 printable ASCII characters with no `|` (the board posts'
/// `author|text` separator), no leading or trailing space.
fn storable_persona(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= STORED_PERSONA_MAX
        && name == name.trim()
        && name.bytes().all(|b| (0x20..0x7f).contains(&b) && b != b'|')
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
            info!(request = %world.log.debug(&request), "no reply: the client keeps what it has");
            Vec::new()
        }
        // Six-byte stamp echo: the client keeps LAND.CFG (CLIENT_CONFIG_FILES_KEPT_ASSUMED).
        HostInfoRequest::LandConfigFile { stamp } => {
            vec![HostMessage::LandConfigFile { stamp }]
        }
        HostInfoRequest::GameInfoFile { stamp } => vec![game_info::reply(stamp)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::{
        AccountBook, AccountId, AccountRecord, ClientVersion, Cookie, EncodedPassword, Enrolment,
        IntProperty, JoinNet, LandCatalog, LandType, LoginNakReason, MemoryStore, ObjectKind,
        PasswordSource, SendMessage, SetInt, Sid, Stamp, Store, StoreError,
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
    fn a_deleted_account_is_refused_like_an_unknown_one() {
        let store = Arc::new(MemoryStore::new());
        let id = AccountId(100_001);
        store
            .create_account(id, AccountRecord::new(EncodedPassword([7; 10]), "guybrush"))
            .unwrap();
        store.retire_account(id).unwrap();
        let world = World {
            accounts: AccountBook::stored(store.clone(), Enrolment::Open),
            store: store.clone(),
            ..World::stock()
        };
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let replies = messages(player.handle(&world, &mut objects, ME, &login(7)));
        let [HostMessage::Nak(nak)] = replies.as_slice() else {
            panic!("expected one Nak, got {replies:?}");
        };
        assert_eq!(nak.which_sub, LoginNakReason::UnknownAccount as u8);
        assert_eq!(nak.text, "This account is not known here.");
        assert!(player.account().is_none());
        assert!(store.account(id).unwrap().is_none());
        let again = AccountRecord::new(EncodedPassword([7; 10]), "guybrush");
        assert!(matches!(
            store.create_account(id, again),
            Err(StoreError::AccountExists(_))
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

    #[test]
    fn a_land_config_request_is_answered_with_the_clients_own_stamp() {
        let world = World::stock();
        let mut objects = ObjectStore::new();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &mut objects, ME, &login(7));
        let stamp = Stamp::from_words(0xA853, 0x21DF);
        let request = ClientMessage::HostInfo(HostInfoRequest::LandConfigFile { stamp });
        let replies = messages(player.handle(&world, &mut objects, ME, &request));
        assert_eq!(replies, vec![HostMessage::LandConfigFile { stamp }]);
        assert_eq!(replies[0].encode(), [0x24, 0x07, 0x53, 0xa8, 0xdf, 0x21]);
        assert_eq!(replies[0].encode(), request.encode());

        let fresh = ClientMessage::HostInfo(HostInfoRequest::GameInfoFile { stamp: Stamp(0) });
        let replies = messages(player.handle(&world, &mut objects, ME, &fresh));
        assert_eq!(replies, vec![game_info::reply(Stamp(0))]);
        let encoded = replies[0].encode();
        assert!(encoded.len() > 10);
        assert_eq!(&encoded[..6], &[0x24, 0x08, 0x01, 0x00, 0x00, 0x00]);

        let held = ClientMessage::HostInfo(HostInfoRequest::GameInfoFile { stamp: Stamp(1) });
        let replies = messages(player.handle(&world, &mut objects, ME, &held));
        assert_eq!(replies[0].encode(), [0x24, 0x08, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(replies[0].encode(), held.encode());
    }
}
