//! What the host knows about a game being formed or played: the game groups (own groups on any land
//! type but the RPG titles') and the invitations that lock players for them. The clients run the
//! invitation and the game; this record lets the host apply its rules and say what is going on
//! (docs/server/games.md).

use std::collections::{BTreeMap, BTreeSet};

use tracing::info;

use super::{ConnectionId, ObjectStore};
use crate::{JoinNet, LockId, Sid};

/// The land-type byte of the joinNet that made a game group: the game, as its client names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameType(pub u8);

impl GameType {
    /// Red Baron's land type: the 41/2 lookup, the player objects and the queue's group all carry it.
    pub const RED_BARON: GameType = GameType(101);

    /// The name of a game type that was seen, if any.
    pub fn name(self) -> Option<&'static str> {
        (self == GameType::RED_BARON).then_some("Red Baron")
    }
}

/// How far a game group has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamePhase {
    /// Fewer than two connections have a member in the group.
    Forming,
    /// Two or more connections have a member in the group: the game can run.
    Gathered,
}

/// An invitation that holds locks: the host sees it only as locks taken by one connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    pub inviter: ConnectionId,
    /// The object the lock request named (the inviter's invitation object, or 0 in the Clubhouse).
    pub from: Sid,
    /// The lock words not yet released: the player SIDs of the inviter and of every guest.
    pub locks: BTreeSet<LockId>,
}

/// One game group and who is behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSession {
    pub group: Sid,
    pub game: GameType,
    /// The connection whose joinNet made the group.
    pub leader: ConnectionId,
    /// The size word of the joinNet: how many players the group takes.
    pub capacity: u16,
    pub phase: GamePhase,
    /// Connections that took a reference with command 30 instead of a joinNet.
    pub registered: BTreeSet<ConnectionId>,
    /// The leader's invitation, if it was still open when the group was made.
    pub invitation: Option<Invitation>,
}

#[derive(Debug, Default)]
pub(super) struct GameBook {
    sessions: BTreeMap<Sid, GameSession>,
    invitations: BTreeMap<ConnectionId, Invitation>,
}

impl GameBook {
    fn lock_granted(&mut self, connection: ConnectionId, from: Sid, locks: &[LockId]) {
        let invitation = self.invitations.entry(connection).or_insert_with(|| {
            info!(
                connection = connection.0,
                from = from.0,
                "invitation opened"
            );
            Invitation {
                inviter: connection,
                from,
                locks: BTreeSet::new(),
            }
        });
        invitation.locks.extend(locks.iter().copied());
    }

    fn unlocked(&mut self, connection: ConnectionId, locks: &[LockId]) {
        let Some(invitation) = self.invitations.get_mut(&connection) else {
            return;
        };
        for lock in locks {
            invitation.locks.remove(lock);
        }
        if invitation.locks.is_empty() {
            self.invitations.remove(&connection);
            info!(connection = connection.0, "invitation closed");
        }
    }

    pub(super) fn connection_gone(&mut self, connection: ConnectionId) {
        if self.invitations.remove(&connection).is_some() {
            info!(
                connection = connection.0,
                "invitation dropped with its inviter"
            );
        }
    }

    pub(super) fn open(&mut self, group: Sid, request: &JoinNet, leader: ConnectionId) {
        let session = GameSession {
            group,
            game: GameType(request.land_type.0),
            leader,
            capacity: request.size,
            phase: GamePhase::Forming,
            registered: BTreeSet::new(),
            invitation: self.invitations.get(&leader).cloned(),
        };
        info!(
            group = group.0,
            game = session.game.0,
            name = session.game.name(),
            leader = leader.0,
            invited = session.invitation.is_some(),
            "game group formed"
        );
        self.sessions.insert(group, session);
    }

    pub(super) fn close(&mut self, group: Sid) {
        if self.sessions.remove(&group).is_some() {
            info!(group = group.0, "game group closed");
        }
    }

    pub(super) fn contains(&self, group: Sid) -> bool {
        self.sessions.contains_key(&group)
    }

    pub(super) fn registered(&mut self, group: Sid, connection: ConnectionId) {
        if let Some(session) = self.sessions.get_mut(&group) {
            session.registered.insert(connection);
        }
    }

    pub(super) fn gathered(&mut self, group: Sid, players: usize) {
        let Some(session) = self.sessions.get_mut(&group) else {
            return;
        };
        let phase = if players >= 2 {
            GamePhase::Gathered
        } else {
            GamePhase::Forming
        };
        if session.phase != phase {
            info!(group = group.0, players, ?phase, "game group changed phase");
            session.phase = phase;
        }
    }
}

impl ObjectStore {
    /// The game groups alive now, by SID.
    pub fn game_sessions(&self) -> impl Iterator<Item = &GameSession> {
        self.games.sessions.values()
    }

    /// The game group with this SID, if it is one.
    pub fn game_session(&self, group: Sid) -> Option<&GameSession> {
        self.games.sessions.get(&group)
    }

    /// The invitation a connection has open: the locks it holds and has not released.
    pub fn invitation_of(&self, connection: ConnectionId) -> Option<&Invitation> {
        self.games.invitations.get(&connection)
    }

    pub(super) fn note_locks_granted(
        &mut self,
        connection: ConnectionId,
        from: Sid,
        locks: &[LockId],
    ) {
        self.games.lock_granted(connection, from, locks);
    }

    pub(super) fn note_locks_released(&mut self, connection: ConnectionId, locks: &[LockId]) {
        self.games.unlocked(connection, locks);
    }

    /// Command 30 on a live game group: the connection takes a reference, as a joinNet would, so the
    /// group lasts while any registered player is in it (docs/server/games.md section 4, rule 3).
    pub(crate) fn register(&mut self, connection: ConnectionId, sids: &[Sid]) {
        for &sid in sids {
            if !self.games.contains(sid) {
                info!(
                    sid = sid.0,
                    "register for something that is not a game group: ignored"
                );
                continue;
            }
            if let Some(object) = self.objects.get_mut(&sid) {
                object.holders.entry(connection).or_insert(1);
            }
            self.games.registered(sid, connection);
            info!(
                sid = sid.0,
                connection = connection.0,
                "game group registered"
            );
        }
    }

    /// Keeps the phase of the game group `group` in step with its members.
    pub(super) fn note_members(&mut self, group: Sid) {
        if self.games.contains(group) {
            let players = self.connections_of(self.members_of(group)).len();
            self.games.gathered(group, players);
        }
    }

    pub(super) fn note_group_closed(&mut self, group: Sid) {
        self.games.close(group);
    }

    pub(super) fn note_connection_gone(&mut self, connection: ConnectionId) {
        self.games.connection_gone(connection);
    }
}
