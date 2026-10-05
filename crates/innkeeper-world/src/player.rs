use std::collections::{BTreeSet, HashMap};

use tracing::{info, warn};

use crate::{
    Account, ClientMessage, GroupJoin, HostInfoRequest, HostMessage, JoinNet, Login, LoginAck,
    LoginNakReason, Nak, ObjectKind, Refusal, Sid, World,
};

const PASSWORD_TRIES: u8 = 3;
/// Server choice: SIDs below this stay free for well-known objects.
const FIRST_SID: u16 = 0x0100;

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
    /// Every object that holds a SID, as the client described it when it joined.
    objects: HashMap<Sid, JoinNet>,
    groups: HashMap<Sid, BTreeSet<Sid>>,
    next_sid: u16,
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

    /// The replies to one client message, in the order they go out.
    #[must_use]
    pub fn handle(&mut self, world: &World, message: &ClientMessage) -> Vec<HostMessage> {
        match *message {
            ClientMessage::Login(ref login) => vec![self.log_in(world, login)],
            ClientMessage::JoinNet(join) => self.with_player(|player| player.join(join)),
            ClientMessage::LeaveNet(sid) => self.with_player(|player| {
                player.leave(sid);
                Vec::new()
            }),
            ClientMessage::GroupJoin(join) => self.with_player(|player| player.join_group(join)),
            ClientMessage::HostInfo(request) => {
                self.with_player(|_| answer_host_info(world, request))
            }
            ClientMessage::LandOccupancyRequest => self.with_player(|_| {
                vec![HostMessage::LandOccupancy(
                    world.lands.occupancy(world.host),
                )]
            }),
            ClientMessage::Send(_)
            | ClientMessage::GroupLeave(_)
            | ClientMessage::GroupMembers(_)
            | ClientMessage::SetInt(_)
            | ClientMessage::SetStr(_)
            | ClientMessage::Multicast(_)
            | ClientMessage::ObjExists(_) => {
                info!(?message, "decoded, no handler yet: ignored");
                Vec::new()
            }
        }
    }

    fn with_player(
        &mut self,
        answer: impl FnOnce(&mut Player) -> Vec<HostMessage>,
    ) -> Vec<HostMessage> {
        match &mut self.state {
            PlayerState::LoggedIn(player) => answer(player),
            PlayerState::AwaitingLogin => {
                warn!("message before Login ignored");
                Vec::new()
            }
        }
    }

    fn log_in(&mut self, world: &World, login: &Login) -> HostMessage {
        match world.accounts.admit(login) {
            Ok(account) => {
                info!(account = account.id.0, persona = %account.persona, "logged in");
                self.state = PlayerState::LoggedIn(Player {
                    account,
                    objects: HashMap::new(),
                    groups: HashMap::new(),
                    next_sid: FIRST_SID,
                });
                HostMessage::LoginAccepted(LoginAck::default())
            }
            Err(refusal) => {
                warn!(account = login.account.0, ?refusal, "login refused");
                HostMessage::Nak(match refusal {
                    Refusal::WrongPassword => Nak::login(
                        LoginNakReason::RetryPassword,
                        PASSWORD_TRIES,
                        "That password is not right.",
                    ),
                    Refusal::UnknownAccount => Nak::login(
                        LoginNakReason::UnknownAccount,
                        0,
                        "This account is not known here.",
                    ),
                })
            }
        }
    }
}

impl Player {
    fn join(&mut self, join: JoinNet) -> Vec<HostMessage> {
        let Some(sid) = self.allocate_sid() else {
            warn!(?join, "every SID is taken");
            return Vec::new();
        };
        info!(sid = sid.0, ?join, persona = %self.account.persona, "object joined");
        self.objects.insert(sid, join);
        vec![HostMessage::ObjId {
            cookie: join.cookie,
            sid,
        }]
    }

    /// The next SID in order that no live object holds, wrapping past the top.
    fn allocate_sid(&mut self) -> Option<Sid> {
        let candidates = (self.next_sid..=u16::MAX).chain(FIRST_SID..self.next_sid);
        let sid = candidates
            .map(Sid)
            .find(|sid| !self.objects.contains_key(sid))?;
        self.next_sid = sid.0.checked_add(1).unwrap_or(FIRST_SID);
        Some(sid)
    }

    fn join_group(&mut self, join: GroupJoin) -> Vec<HostMessage> {
        let is_group = self
            .objects
            .get(&join.group)
            .is_some_and(|group| holds_members(group.kind));
        if !is_group {
            warn!(?join, "add to an object that is not a group");
            return Vec::new();
        }
        info!(?join, "group member added");
        self.groups
            .entry(join.group)
            .or_default()
            .insert(join.member);
        vec![HostMessage::GroupJoined {
            group: join.group,
            member: join.member,
        }]
    }

    fn leave(&mut self, sid: Sid) {
        self.groups.remove(&sid);
        self.groups.values_mut().for_each(|members| {
            members.remove(&sid);
        });
        match self.objects.remove(&sid) {
            Some(object) => info!(sid = sid.0, ?object, "object left"),
            None => warn!(sid = sid.0, "leaveNet for an unknown SID"),
        }
    }
}

/// Which kinds take members until the shared object store decides it per land.
fn holds_members(kind: ObjectKind) -> bool {
    matches!(kind, ObjectKind::Group | ObjectKind::LandGroup)
}

fn answer_host_info(world: &World, request: HostInfoRequest) -> Vec<HostMessage> {
    match request {
        HostInfoRequest::HostNumber => vec![HostMessage::HostNumber(world.host)],
        HostInfoRequest::LandDirectory { .. } => {
            vec![HostMessage::LandDirectory(
                world.lands.directory(world.host),
            )]
        }
        HostInfoRequest::HostAddressFile { .. } | HostInfoRequest::HostTime => {
            info!(?request, "no reply: the client keeps what it has");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountBook, AccountId, ClientVersion, Cookie, EncodedPassword, LandCatalog, LandType,
        ObjectKind, PasswordSource, SendMessage,
    };

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

    fn join(cookie: u16, kind: ObjectKind) -> ClientMessage {
        ClientMessage::JoinNet(JoinNet {
            cookie: Cookie(cookie),
            kind,
            land_type: LandType(1),
            parameter: 0,
            size: 12,
        })
    }

    fn world_with_one_account() -> World {
        World {
            accounts: AccountBook::listed([(AccountId(100_001), EncodedPassword([7; 10]))]),
            lands: LandCatalog::stock(),
            ..World::stock()
        }
    }

    #[test]
    fn a_wrong_password_asks_the_client_to_try_again() {
        let world = world_with_one_account();
        let mut player = PlayerSession::new();
        let replies = player.handle(&world, &login(8));
        let [HostMessage::Nak(nak)] = replies.as_slice() else {
            panic!("expected one Nak, got {replies:?}");
        };
        assert_eq!(nak.which_sub, LoginNakReason::RetryPassword as u8);
        assert!(player
            .handle(&world, &join(1, ObjectKind::GameObject))
            .is_empty());
        assert!(matches!(
            player.handle(&world, &login(7)).as_slice(),
            [HostMessage::LoginAccepted(_)]
        ));
    }

    #[test]
    fn requests_before_login_get_no_reply() {
        let world = World::stock();
        let mut player = PlayerSession::new();
        let request = ClientMessage::HostInfo(HostInfoRequest::HostNumber);
        assert!(player.handle(&world, &request).is_empty());
    }

    #[test]
    fn only_groups_take_members_and_leaving_ends_membership() {
        let world = World::stock();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &login(7));
        let _ = player.handle(&world, &join(1, ObjectKind::Object));
        let _ = player.handle(&world, &join(2, ObjectKind::LandGroup));
        let add = |group| {
            ClientMessage::GroupJoin(GroupJoin {
                group: Sid(group),
                member: Sid(FIRST_SID),
                version: None,
            })
        };
        assert!(player.handle(&world, &add(FIRST_SID)).is_empty());
        assert_eq!(player.handle(&world, &add(FIRST_SID + 1)).len(), 1);
        let _ = player.handle(&world, &ClientMessage::LeaveNet(Sid(FIRST_SID + 1)));
        assert!(player.handle(&world, &add(FIRST_SID + 1)).is_empty());
    }

    #[test]
    fn decoded_shared_object_commands_are_ignored_until_a_handler_exists() {
        let world = World::stock();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &login(7));
        let relay = ClientMessage::Send(SendMessage {
            to: Sid(FIRST_SID),
            from: Sid(0),
            payload: vec![1, 0],
        });
        assert!(player.handle(&world, &relay).is_empty());
    }

    #[test]
    fn sids_wrap_past_the_top_and_skip_live_objects() {
        let world = World::stock();
        let mut player = PlayerSession::new();
        let _ = player.handle(&world, &login(7));
        let first = player.handle(&world, &join(1, ObjectKind::Object));
        assert_eq!(first, [obj_id(1, FIRST_SID)]);
        let PlayerState::LoggedIn(logged_in) = &mut player.state else {
            panic!("logged in above");
        };
        logged_in.next_sid = u16::MAX;
        assert_eq!(logged_in.join(join_net(2)), [obj_id(2, u16::MAX)]);
        assert_eq!(logged_in.join(join_net(3)), [obj_id(3, FIRST_SID + 1)]);
    }

    fn join_net(cookie: u16) -> JoinNet {
        let ClientMessage::JoinNet(join) = join(cookie, ObjectKind::Object) else {
            panic!("join builds a JoinNet");
        };
        join
    }

    fn obj_id(cookie: u16, sid: u16) -> HostMessage {
        HostMessage::ObjId {
            cookie: Cookie(cookie),
            sid: Sid(sid),
        }
    }
}
