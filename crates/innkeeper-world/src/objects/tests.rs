use super::*;
use crate::Cookie;

const A: ConnectionId = ConnectionId(1);
const B: ConnectionId = ConnectionId(2);

fn request(cookie: u16, kind: ObjectKind, parameter: u16) -> JoinNet {
    JoinNet {
        cookie: Cookie(cookie),
        kind,
        land_type: LandType(1),
        parameter,
        size: 2,
    }
}

fn granted(deliveries: &[Delivery]) -> Sid {
    match deliveries.last() {
        Some(Delivery {
            message: HostMessage::ObjId { sid, .. },
            ..
        }) => *sid,
        other => panic!("expected an ObjID last, got {other:?}"),
    }
}

#[test]
fn sids_wrap_past_the_top_and_skip_live_objects() {
    let mut store = ObjectStore::new();
    let first = granted(&store.join(A, request(1, ObjectKind::Object, 0xFFFF)));
    assert_eq!(first, Sid(FIRST_SID));
    store.next_sid = u16::MAX;
    let top = granted(&store.join(A, request(2, ObjectKind::Object, 0xFFFF)));
    assert_eq!(top, Sid(u16::MAX));
    let wrapped = granted(&store.join(B, request(3, ObjectKind::Object, 0xFFFF)));
    assert_eq!(wrapped, Sid(FIRST_SID + 1));
}

#[test]
fn scoping_follows_kind_and_parameter() {
    let mut store = ObjectStore::new();
    let mut sid =
        |connection, kind, parameter| granted(&store.join(connection, request(9, kind, parameter)));
    let land = ObjectKind::LandGroup;
    assert_eq!(sid(A, land, 1), sid(B, land, 1));
    assert_ne!(sid(A, land, 1), sid(B, land, 2));
    assert_ne!(sid(A, ObjectKind::Group, 1), sid(B, land, 1));
    let party = ObjectKind::PrivateGroup;
    assert_ne!(sid(A, party, 0xFFFD), sid(B, party, 0xFFFD));
    assert_ne!(sid(A, ObjectKind::Object, 1), sid(B, ObjectKind::Object, 1));
}

#[test]
fn a_shared_group_is_found_by_its_sid_and_nothing_else_is() {
    let mut store = ObjectStore::new();
    let room = granted(&store.join(A, request(1, ObjectKind::LandGroup, 1)));
    let party = granted(&store.join(A, request(2, ObjectKind::PrivateGroup, 0xFFFD)));
    let player = granted(&store.join(A, request(3, ObjectKind::Object, 0xFFFF)));
    let key = GroupKey {
        kind: ObjectKind::LandGroup,
        land_type: LandType(1),
        parameter: 1,
    };
    assert_eq!(store.group_key(room), Some(key));
    assert_eq!(store.group_key(party), None);
    assert_eq!(store.group_key(player), None);
    assert_eq!(store.group_key(Sid(0x7777)), None);
}

/// A tiny xorshift generator, so the property run is the same every time.
struct Seeded(u64);

impl Seeded {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        match items.len() {
            0 => None,
            len => items.get(self.below(len)).copied(),
        }
    }
}

const KINDS: [(ObjectKind, u16); 6] = [
    (ObjectKind::Object, 0xFFFF),
    (ObjectKind::SoloObject, 0xFFFF),
    (ObjectKind::GameObject, 0),
    (ObjectKind::Group, 3),
    (ObjectKind::PrivateGroup, 0xFFFD),
    (ObjectKind::LandGroup, 1),
];

#[test]
fn a_sid_is_never_handed_out_twice_while_it_lives() {
    let mut store = ObjectStore::new();
    store.next_sid = u16::MAX - 40;
    let mut random = Seeded(0x9E37_79B9_7F4A_7C15);
    let connections = [A, B, ConnectionId(3)];
    for _ in 0..20_000 {
        let Some(connection) = random.pick(&connections) else {
            unreachable!("three connections");
        };
        let held: Vec<Sid> = (store.objects.iter())
            .filter(|(_, object)| object.holders.contains_key(&connection))
            .map(|(&sid, _)| sid)
            .collect();
        match random.below(10) {
            0..=3 => {
                let Some((kind, parameter)) = random.pick(&KINDS) else {
                    unreachable!("six kinds");
                };
                let cookie = random.below(3) as u16;
                join_and_check(&mut store, connection, request(cookie, kind, parameter));
            }
            4..=6 => {
                let groups: Vec<Sid> = store.objects.keys().copied().collect();
                if let (Some(group), Some(member)) = (random.pick(&groups), random.pick(&held)) {
                    let version = None;
                    let join = GroupJoin {
                        group,
                        member,
                        version,
                    };
                    let _ = store.join_group(connection, join);
                }
            }
            7..=8 => {
                if let Some(sid) = random.pick(&held) {
                    let _ = store.leave(connection, sid);
                }
            }
            _ => {
                let _ = store.disconnect(connection);
            }
        }
        assert_consistent(&store);
    }
    for connection in connections {
        let _ = store.disconnect(connection);
    }
    assert!(store.is_empty() && store.shared_groups.is_empty());
}

fn join_and_check(store: &mut ObjectStore, connection: ConnectionId, join: JoinNet) {
    let live_before: Vec<Sid> = store.objects.keys().copied().collect();
    let shared_before = store.shared_groups.clone();
    let sid = granted(&store.join(connection, join));
    assert!(sid.0 >= FIRST_SID);
    match Scope::of(&join) {
        Scope::SharedGroup(key) if shared_before.contains_key(&key) => {
            assert_eq!(shared_before.get(&key), Some(&sid));
        }
        Scope::Object | Scope::OwnGroup | Scope::SharedGroup(_) => {
            assert!(!live_before.contains(&sid), "{sid:?} was live");
        }
    }
}

fn assert_consistent(store: &ObjectStore) {
    for (key, sid) in &store.shared_groups {
        let group = store
            .objects
            .get(sid)
            .map(|group| Scope::of(&group.request));
        assert!(matches!(group, Some(Scope::SharedGroup(live)) if live == *key));
    }
    for (sid, object) in &store.objects {
        assert!(!object.holders.is_empty() && object.holders.values().all(|&count| count > 0));
        let members = object.members();
        assert!(members
            .iter()
            .all(|member| store.objects.contains_key(member)));
        assert!(!members.contains(sid));
        let unique: BTreeSet<_> = members.iter().collect();
        assert_eq!(unique.len(), members.len());
        if let Role::Group { capacity, .. } = object.role {
            assert!(members.len() <= usize::from(capacity));
        }
    }
}

/// A waiting room with a player object of each connection in it.
fn waiting_room() -> (ObjectStore, Sid, Sid, Sid) {
    let mut store = ObjectStore::new();
    let room = request(1, ObjectKind::LandGroup, 1);
    let group = granted(&store.join(A, room));
    assert_eq!(granted(&store.join(B, room)), group);
    let a_player = granted(&store.join(A, request(2, ObjectKind::Object, 0xFFFF)));
    let b_player = granted(&store.join(B, request(2, ObjectKind::Object, 0xFFFF)));
    for (connection, member) in [(A, a_player), (B, b_player)] {
        let join = GroupJoin {
            group,
            member,
            version: None,
        };
        let _ = store.join_group(connection, join);
    }
    (store, group, a_player, b_player)
}

#[test]
fn an_object_goes_to_its_holders_and_a_group_to_the_holders_of_its_members() {
    let (store, group, a_player, b_player) = waiting_room();
    assert_eq!(store.recipients(a_player), BTreeSet::from([A]));
    assert_eq!(store.recipients(b_player), BTreeSet::from([B]));
    assert_eq!(store.recipients(group), BTreeSet::from([A, B]));
    assert!(store.recipients(Sid(0x7777)).is_empty());
}

#[test]
fn a_member_that_left_the_group_no_longer_brings_its_connection() {
    let (mut store, group, a_player, _) = waiting_room();
    let leave = GroupLeave {
        group,
        member: a_player,
    };
    let _ = store.leave_group(A, leave);
    assert_eq!(store.recipients(group), BTreeSet::from([B]));
    assert_eq!(store.recipients(a_player), BTreeSet::from([A]));
}

#[test]
fn a_hang_up_removes_the_connection_from_every_recipient_set() {
    let (mut store, group, a_player, b_player) = waiting_room();
    let _ = store.disconnect(B);
    assert_eq!(store.recipients(group), BTreeSet::from([A]));
    assert!(store.recipients(b_player).is_empty());
    let _ = store.disconnect(A);
    assert!(store.recipients(group).is_empty() && store.recipients(a_player).is_empty());
}
