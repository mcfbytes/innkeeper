use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::iter;

use tracing::{info, warn};

use crate::assumptions::{
    GROUP_JOIN_TELLS_MEMBERS_ASSUMED, GROUP_KINDS_ASSUMED, PRIVATE_PARAMETER_MIN_ASSUMED,
    REJOIN_REPLACES_KIND_ASSUMED,
};
use crate::message::Command;
use crate::{
    GroupJoin, GroupLeave, GroupMembers, GroupMembersRequest, HostMessage, JoinNet, LandType, Nak,
    ObjectKind, Sid,
};

/// Server choice: SIDs below this stay free for well-known objects.
const FIRST_SID: u16 = 0x0100;

/// One client connection to this host, numbered by the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(pub u64);

/// A host message and the connection it goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub to: ConnectionId,
    pub message: HostMessage,
}

impl Delivery {
    pub fn new(to: ConnectionId, message: HostMessage) -> Self {
        Delivery { to, message }
    }
}

/// What a shared group is found by: every joiner with the same key gets the same SID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GroupKey {
    pub kind: ObjectKind,
    pub land_type: LandType,
    pub parameter: u16,
}

/// Why a GrpJoin is refused; the DOS clients read the code from word 5 of the Nak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum GroupJoinRefusal {
    InvalidGroup = 1,
    Full = 2,
    InvalidObject = 3,
    WrongVersion = 6,
}

/// Every networked object on the host, who holds it, and the members of each group.
/// Behaviour and the policies behind it: docs/server/objects.md.
#[derive(Debug)]
pub struct ObjectStore {
    objects: BTreeMap<Sid, NetObject>,
    shared_groups: HashMap<GroupKey, Sid>,
    next_sid: u16,
}

#[derive(Debug)]
struct NetObject {
    request: JoinNet,
    /// How many joinNet requests of each connection keep the object alive; never 0.
    holders: BTreeMap<ConnectionId, u32>,
    role: Role,
}

#[derive(Debug)]
enum Role {
    Object,
    Group { capacity: u16, members: Vec<Sid> },
}

/// How a joinNet is scoped: an object or group of the requester's own, or a group shared by key.
enum Scope {
    Object,
    OwnGroup,
    SharedGroup(GroupKey),
}

impl Scope {
    fn of(join: &JoinNet) -> Self {
        if !GROUP_KINDS_ASSUMED.contains(&join.kind) {
            Scope::Object
        } else if join.parameter >= PRIVATE_PARAMETER_MIN_ASSUMED {
            Scope::OwnGroup
        } else {
            Scope::SharedGroup(GroupKey {
                kind: join.kind,
                land_type: join.land_type,
                parameter: join.parameter,
            })
        }
    }
}

impl Default for ObjectStore {
    fn default() -> Self {
        ObjectStore::new()
    }
}

impl ObjectStore {
    pub fn new() -> Self {
        ObjectStore {
            objects: BTreeMap::new(),
            shared_groups: HashMap::new(),
            next_sid: FIRST_SID,
        }
    }

    /// The members of the shared group with this key; 0 while nobody holds it.
    pub fn member_count(&self, key: GroupKey) -> usize {
        let group = self.shared_groups.get(&key);
        group.map_or(0, |&group| self.members_of(group).len())
    }

    /// The connections a `Send` to `sid` reaches: the holders of an object, the connections that hold
    /// a member of a group; none for a SID nobody holds.
    pub fn recipients(&self, sid: Sid) -> BTreeSet<ConnectionId> {
        match self.objects.get(&sid) {
            None => BTreeSet::new(),
            Some(object) => match object.role {
                Role::Object => object.holders.keys().copied().collect(),
                Role::Group { .. } => self.connections_of(object.members()),
            },
        }
    }

    /// The key of the shared group with this SID; none for an object, an own group or a free SID.
    pub fn group_key(&self, sid: Sid) -> Option<GroupKey> {
        match Scope::of(&self.objects.get(&sid)?.request) {
            Scope::SharedGroup(key) => Some(key),
            Scope::Object | Scope::OwnGroup => None,
        }
    }

    /// What `sid` is, when this connection holds it.
    pub fn kind_held(&self, connection: ConnectionId, sid: Sid) -> Option<ObjectKind> {
        let object = self.objects.get(&sid)?;
        object
            .holders
            .contains_key(&connection)
            .then_some(object.request.kind)
    }

    /// True when no connection holds anything.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// joinNet: the requester's `ObjID`, after the notices about an object it replaces.
    pub(crate) fn join(&mut self, connection: ConnectionId, join: JoinNet) -> Vec<Delivery> {
        let mut deliveries = self.replace_rejoined(connection, join);
        let sid = match Scope::of(&join) {
            Scope::Object => self.create(connection, join, Role::Object),
            Scope::OwnGroup => self.create(connection, join, Role::group(join.size)),
            Scope::SharedGroup(key) => self.share(connection, key, join),
        };
        match sid {
            Some(sid) => deliveries.push(Delivery::new(
                connection,
                HostMessage::ObjId {
                    cookie: join.cookie,
                    sid,
                },
            )),
            None => warn!(?join, "every SID is taken"),
        }
        deliveries
    }

    /// leaveNet: releases one of the connection's references; the last one lets go of the object.
    pub(crate) fn leave(&mut self, connection: ConnectionId, sid: Sid) -> Vec<Delivery> {
        let object = self.objects.get_mut(&sid);
        let Some(references) = object.and_then(|object| object.holders.get_mut(&connection)) else {
            warn!(
                sid = sid.0,
                "leaveNet for a SID this connection does not hold"
            );
            return Vec::new();
        };
        *references = references.saturating_sub(1);
        if *references > 0 {
            info!(sid = sid.0, references, "shared group released");
            return Vec::new();
        }
        self.let_go(connection, sid)
    }

    /// GrpJoin: the echo to the joiner and the members, or a Nak naming what is wrong.
    pub(crate) fn join_group(
        &mut self,
        connection: ConnectionId,
        join: GroupJoin,
    ) -> Vec<Delivery> {
        if let Err(refusal) = self.admit(connection, join) {
            warn!(?join, ?refusal, "group join refused");
            return vec![Delivery::new(connection, refusal.nak(join.group))];
        }
        info!(?join, "group member added");
        let mut audience = if GROUP_JOIN_TELLS_MEMBERS_ASSUMED {
            self.connections_of(self.members_of(join.group))
        } else {
            BTreeSet::new()
        };
        audience.remove(&connection);
        let joined = HostMessage::GroupJoined {
            group: join.group,
            member: join.member,
        };
        let echo = Delivery::new(connection, joined.clone());
        iter::once(echo).chain(tell(audience, joined)).collect()
    }

    /// The Nak that turns a GrpJoin away because the client's version is outside the land's range.
    pub(crate) fn refuse_wrong_version(
        &self,
        connection: ConnectionId,
        join: GroupJoin,
    ) -> Vec<Delivery> {
        let refusal = GroupJoinRefusal::WrongVersion;
        warn!(?join, ?refusal, "group join refused");
        vec![Delivery::new(connection, refusal.nak(join.group))]
    }

    /// GrpDel: the member leaves, and the remaining members hear of it.
    pub(crate) fn leave_group(
        &mut self,
        connection: ConnectionId,
        leave: GroupLeave,
    ) -> Vec<Delivery> {
        if !self.members_of(leave.group).contains(&leave.member) {
            warn!(?leave, "GrpDel for a member that is not in the group");
            return Vec::new();
        }
        self.remove_member(connection, leave.group, leave.member)
    }

    /// GrpMem: the member list, to the requester only.
    pub(crate) fn list_members(
        &self,
        connection: ConnectionId,
        request: GroupMembersRequest,
    ) -> Vec<Delivery> {
        match self.objects.get(&request.group).map(|object| &object.role) {
            Some(Role::Group { members, .. }) => vec![Delivery::new(
                connection,
                HostMessage::GroupMembers(GroupMembers {
                    group: request.group,
                    members: members.clone(),
                }),
            )],
            Some(Role::Object) | None => {
                warn!(?request, "member list of something that is not a group");
                Vec::new()
            }
        }
    }

    /// Releases everything the connection holds, as when it hangs up or logs in again.
    pub fn disconnect(&mut self, connection: ConnectionId) -> Vec<Delivery> {
        let mut held: Vec<(Sid, bool)> = self
            .objects
            .iter()
            .filter(|(_, object)| object.holders.contains_key(&connection))
            .map(|(&sid, object)| (sid, matches!(object.role, Role::Group { .. })))
            .collect();
        // Plain objects go first, while the groups they are in still name their peers.
        held.sort_by_key(|&(_, is_group)| is_group);
        let deliveries = held.into_iter();
        deliveries
            .flat_map(|(sid, _)| self.let_go(connection, sid))
            .collect()
    }

    fn replace_rejoined(&mut self, connection: ConnectionId, join: JoinNet) -> Vec<Delivery> {
        if join.kind != REJOIN_REPLACES_KIND_ASSUMED {
            return Vec::new();
        }
        let replaced = self.objects.iter().find(|(_, object)| {
            object.request.kind == join.kind
                && object.request.cookie == join.cookie
                && object.holders.contains_key(&connection)
        });
        let Some((&sid, _)) = replaced else {
            return Vec::new();
        };
        info!(sid = sid.0, "re-joined object replaces its old SID");
        self.let_go(connection, sid)
    }

    fn create(&mut self, connection: ConnectionId, request: JoinNet, role: Role) -> Option<Sid> {
        let sid = self.allocate_sid()?;
        info!(sid = sid.0, ?request, "object joined");
        let holders = BTreeMap::from([(connection, 1)]);
        let object = NetObject {
            request,
            holders,
            role,
        };
        self.objects.insert(sid, object);
        Some(sid)
    }

    fn share(&mut self, connection: ConnectionId, key: GroupKey, join: JoinNet) -> Option<Sid> {
        let existing = self.shared_groups.get(&key).copied();
        let Some(sid) = existing else {
            let sid = self.create(connection, join, Role::group(join.size))?;
            self.shared_groups.insert(key, sid);
            return Some(sid);
        };
        if let Some(group) = self.objects.get_mut(&sid) {
            let references = group.holders.entry(connection).or_default();
            *references = references.saturating_add(1);
        }
        info!(sid = sid.0, ?key, "shared group joined");
        Some(sid)
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

    fn admit(&mut self, connection: ConnectionId, join: GroupJoin) -> Result<(), GroupJoinRefusal> {
        let member_is_own = join.member != join.group && self.holds(connection, join.member);
        let role = self
            .objects
            .get_mut(&join.group)
            .map(|group| &mut group.role);
        let Some(Role::Group { capacity, members }) = role else {
            return Err(GroupJoinRefusal::InvalidGroup);
        };
        if !member_is_own {
            return Err(GroupJoinRefusal::InvalidObject);
        }
        if members.contains(&join.member) {
            return Ok(());
        }
        if members.len() >= usize::from(*capacity) {
            return Err(GroupJoinRefusal::Full);
        }
        members.push(join.member);
        Ok(())
    }

    /// The connection holds `sid` no more: its own members leave, and the last holder destroys it.
    fn let_go(&mut self, connection: ConnectionId, sid: Sid) -> Vec<Delivery> {
        let own_members: Vec<Sid> = (self.members_of(sid).iter().copied())
            .filter(|&member| self.holds(connection, member))
            .collect();
        let mut deliveries: Vec<Delivery> = own_members
            .into_iter()
            .flat_map(|member| self.remove_member(connection, sid, member))
            .collect();
        let Some(object) = self.objects.get_mut(&sid) else {
            return deliveries;
        };
        object.holders.remove(&connection);
        if object.holders.is_empty() {
            deliveries.extend(self.destroy(connection, sid));
        }
        deliveries
    }

    /// Frees `sid`: it leaves its groups, and every other connection that sees it gets `ObjFree`.
    fn destroy(&mut self, actor: ConnectionId, sid: Sid) -> Vec<Delivery> {
        let peers = self.peers(actor, sid);
        let groups = self.groups_containing(sid);
        let mut deliveries: Vec<Delivery> = groups
            .into_iter()
            .flat_map(|group| self.remove_member(actor, group, sid))
            .collect();
        if let Some(object) = self.objects.remove(&sid) {
            info!(sid = sid.0, request = ?object.request, "object freed");
            if let Scope::SharedGroup(key) = Scope::of(&object.request) {
                self.shared_groups.remove(&key);
            }
        }
        deliveries.extend(tell(peers, HostMessage::ObjectFreed(sid)));
        deliveries
    }

    /// `member` leaves `group`; the connections of the remaining members get `GrpDel`.
    fn remove_member(&mut self, actor: ConnectionId, group: Sid, member: Sid) -> Vec<Delivery> {
        let role = self.objects.get_mut(&group).map(|object| &mut object.role);
        let Some(Role::Group { members, .. }) = role else {
            return Vec::new();
        };
        members.retain(|&kept| kept != member);
        info!(group = group.0, member = member.0, "group member left");
        let mut audience = self.connections_of(self.members_of(group));
        audience.remove(&actor);
        tell(
            audience,
            HostMessage::GroupLeft(GroupLeave { group, member }),
        )
    }

    /// The other connections that know `sid`: they share a group with it, or are in it.
    fn peers(&self, actor: ConnectionId, sid: Sid) -> BTreeSet<ConnectionId> {
        let groups = self.groups_containing(sid);
        let fellows = groups.iter().flat_map(|&group| self.members_of(group));
        let insiders = self.members_of(sid);
        let others: Vec<Sid> = (fellows.chain(insiders).copied())
            .filter(|&other| other != sid)
            .collect();
        let mut peers = self.connections_of(&others);
        peers.remove(&actor);
        peers
    }

    fn groups_containing(&self, sid: Sid) -> Vec<Sid> {
        let groups = self.objects.iter();
        groups
            .filter(|(_, group)| group.members().contains(&sid))
            .map(|(&group, _)| group)
            .collect()
    }

    fn connections_of(&self, objects: &[Sid]) -> BTreeSet<ConnectionId> {
        let objects = objects.iter().filter_map(|sid| self.objects.get(sid));
        objects
            .flat_map(|object| object.holders.keys().copied())
            .collect()
    }

    fn members_of(&self, sid: Sid) -> &[Sid] {
        self.objects.get(&sid).map_or(&[], NetObject::members)
    }

    fn holds(&self, connection: ConnectionId, sid: Sid) -> bool {
        let object = self.objects.get(&sid);
        object.is_some_and(|object| object.holders.contains_key(&connection))
    }
}

impl NetObject {
    fn members(&self) -> &[Sid] {
        match &self.role {
            Role::Object => &[],
            Role::Group { members, .. } => members,
        }
    }
}

impl Role {
    fn group(capacity: u16) -> Self {
        Role::Group {
            capacity,
            members: Vec::new(),
        }
    }
}

impl GroupJoinRefusal {
    fn nak(self, group: Sid) -> HostMessage {
        HostMessage::Nak(Nak {
            to: group,
            which_cmd: Command::GroupJoin.byte(),
            which_sub: self as u8,
            num_tries: 0,
            text: String::new(),
        })
    }
}

fn tell(audience: BTreeSet<ConnectionId>, message: HostMessage) -> Vec<Delivery> {
    let deliveries = audience.into_iter();
    deliveries
        .map(|to| Delivery::new(to, message.clone()))
        .collect()
}

#[cfg(test)]
mod tests;
