# The shared object store

`innkeeper_world::ObjectStore` (`crates/innkeeper-world/src/objects.rs`) holds every networked object of one
host: the SIDs it hands out, which connection keeps each object alive, and the members of each group. One
store serves every connection; `innkeeperd` keeps it behind one `Mutex` in `ConnectionSettings` and carries
what it says to other connections through the switchboard (section 5). Message layouts are in
`docs/protocol/messages.md` section 3.2.

The policies below are host decisions the client does not pin down. Each INFERRED one is a named constant
in `crates/innkeeper-world/src/assumptions.rs`, and section 6 lists them.

## 1. Vocabulary

| Term | Meaning | Type |
|---|---|---|
| connection | one client link to this host, numbered by `innkeeperd` | `ConnectionId` |
| delivery | a host message and the connection it goes to | `Delivery { to, message }` |
| holder | a connection that keeps an object alive; it holds one reference per joinNet that returned the SID | `NetObject.holders` |
| group | an object that takes members (`Role::Group`, with a capacity and a member list in joining order) | `Role` |
| shared group | a group found by its key: every joiner with the same key gets the same SID | `GroupKey` |
| peer | another connection that sees an object: it has an object in a group with it, or (for a group) in it | `ObjectStore::peers` |

`PlayerSession::handle(world, objects, connection, message)` returns the deliveries one client message
causes. The sender's own replies are in request order; notices for other connections sit where they happened.
`ObjectStore::disconnect(connection)` returns the notices a hang-up causes.

## 2. SIDs

SIDs are host-wide and start at `0x0100`; lower values stay free for well-known objects (a server choice).
The allocator counts up, wraps from `0xFFFF` back to `0x0100`, and skips SIDs that a live object still holds,
so a SID is never handed out twice while it lives (unit test `sids_wrap_past_the_top_and_skip_live_objects`
and the seeded property run `a_sid_is_never_handed_out_twice_while_it_lives`). Every joinNet gets exactly
one `ObjID`, in request order, with the request's cookie echoed: the DOS games match replies by order only
(`int14h-census/yserbius.md` section 4.2), LSCI by cookie.

## 3. Scoping a joinNet

| Request | Scope | SID |
|---|---|---|
| kind 1, 3 or 129 (`Object`, `SoloObject`, `GameObject`) | an object of the requester's own | fresh |
| kind 2, 4 or 5 with a parameter of `0x8000` or more (the personal party sends `0xFFFD`) | a group of the requester's own | fresh |
| kind 2, 4 or 5 with any other parameter | the group shared by `(kind, landType, parameter)` | the key's SID while anyone holds it, else fresh |

A shared group's capacity is the `size` of the joinNet that created it (128 for the stock Clubhouse waiting
room, `captures.md` section 11); later joiners add a reference and do not change it. `member_count(key)`
reports how many members the shared group of a key has, for land occupancy, and `group_key(sid)` gives
the key of a shared group back from its SID. `recipients(sid)` names the connections a `Send` reaches
([router.md](router.md)).

**Re-join.** A joinNet of kind 129 with a cookie the same connection already holds replaces that object: the
old one is freed (section 4) and the new one gets the next SID. The stock client does this when it enters the
Clubhouse (`messages.md` section 5.4), and it is why the waiting-room capture shows `0x0101` for the second
game-object join.

## 4. Group changes and releases

| Client sends | The store does | Who hears what |
|---|---|---|
| GrpJoin (10) | checks, then adds the member (adding it again changes nothing) | the joiner first, then every other connection with a member in the group: `GrpJoin` (10) with group and member |
| GrpJoin refused | nothing | the joiner only: Nak with `whichCmd` 10, `toSID` the group, code at byte 5 (section 4.1) |
| GrpDel (11) | removes the member | every other connection with a member left in the group: `GrpDel` (11) |
| GrpMem (12) | nothing | the requester only: `GrpMem` (12) with every member from byte 6, in joining order |
| leaveNet (9) | releases one of the sender's references (section 4.2) | as section 4.2 |
| Login (53, 59) again | releases everything the connection held, as a hang-up | as section 4.2 |

Nobody is told about their own action except the joiner, whose client waits for the `GrpJoin` echo
(`hub/script.120 PlayerLogin`, `int14h-census/yserbius.md` section 4.3).

### 4.1 GrpJoin refusals

| Code | Meaning (DOS client text) | When |
|---|---|---|
| 1 | "Invalid Group" | the group SID is not a live group |
| 2 | "Group Full" | the group has `capacity` members and the member is not one of them |
| 3 | "Invalid Object" | the member is not a live object of the requesting connection, or is the group itself |

Codes 4 and 5 (locked, no rights) are not sent here. Code 6 (version) is decided outside the store:
`presence::join_group` checks a waiting-room join against the land's range and calls `refuse_wrong_version`.

### 4.2 Release and destruction

A connection's reference count on an object goes down by one per leaveNet. When it reaches 0 the connection
lets go: its own objects leave the group with `GrpDel` to the remaining members, and when no holder is left
the object is destroyed:

1. it leaves every group it is in, with `GrpDel` to each group's remaining members;
2. every peer (computed before step 1) gets `ObjFree` (9) for it; for a destroyed group these are the
   connections whose members were still in it.

A hang-up or a repeated Login lets go of everything the connection held, plain objects before groups, so the
peers still see the groups when the player object goes: in the waiting room the others get `GrpDel` for the
player and then `ObjFree`.

## 5. Delivery in `innkeeperd`

`innkeeperd::switchboard::Switchboard` maps each `ConnectionId` to an unbounded tokio channel, the
connection's `Inbox`. `Host::answer` takes the store lock, hands deliveries for other connections to the
switchboard while it still holds the lock, and returns its own; the transport's driver sends them (the
legacy link with `Session::send_message` and `Session::flush`) or queues them for Receive (the INT 14h
transport). A connection's pump loop waits on its inbox beside the socket and the timers, and treats what
arrives the same way. Because every inbox is filled under the store lock, and `Host::answer` first
takes what is already in its own inbox, each client receives deliveries in the order the store made them.
On TCP close, a lost call (`SessionEvent::LinkLost`) or a transport Disconnect, `Host::hang_up` calls `ObjectStore::disconnect`; dropping the inbox takes the connection off the
switchboard, and a delivery for a connection that is gone is dropped.

## 6. Assumptions

| Constant | Value | Why | Source |
|---|---|---|---|
| `GROUP_KINDS_ASSUMED` | kinds 2, 4, 5 | the kinds whose census callers add members; the codec leaves group-ness to the host | `messages.md` 3.2.1 |
| `PRIVATE_PARAMETER_MIN_ASSUMED` | `0x8000` | the personal party sends -3 and the player -1; land and map numbers are small | `int14h-census/yserbius.md` 4.1, `SL/script.120` |
| `GROUP_JOIN_TELLS_MEMBERS_ASSUMED` | true | waiting-room members fetch the newcomer's name from `addMember` (`hub/script.003 WaitingRoomGroup::addMember`) | `messages.md` 11 |
| `REJOIN_REPLACES_KIND_ASSUMED` | kind 129 | the Clubhouse entry re-joins the game object with no leaveNet | `captures.md` 11 |
| `UNLOCK_IS_ACKNOWLEDGED_ASSUMED` | false | no script waits for a reply to an unlock | `messages.md` 3.2 |
| `REPLICAS_INCLUDE_GROUP_FELLOWS_ASSUMED` | true | `WaitingRoomGroup::addMember` builds a replica of each newcomer and reads its properties, so a later change must reach it | `messages.md` 3.2 |
| `UNSET_INT_PROPERTY_ASSUMED` | 0 | a `GrpGetProp` row needs a value in every column; text columns get empty text | `messages.md` 3.2 |
| `PROPERTIES_UNREAD_WORD_ASSUMED` | 0 | the word at byte 4 of `SetMsg`, which the client skips | `messages.md` 3.2 |

Also INFERRED but not constants, because no alternative is plausible: the shared key is
`(kind, landType, parameter)`; a release by a connection takes its own members out of the group; a destroyed
object's peers get `ObjFree`; GrpDel is accepted from any connection for a member that is in the group.

## 7. Open

- Map groups (kind 2 from Yserbius and Twinion) carry only the map number, so two lands with the same map
  number would share a map group (`int14h-census/yserbius.md` section 8); R2 decides the RPG scoping.
- LSCI's `GameGroup` sends kind 2 with a parameter from its caller; whether two tables of one game share that
  parameter is not known (D1).
- No capture of the original host exists, so the echo policy, the re-join rule, the lock refusal word and
  the replica set rest on the client's handlers and the live run.
- What a `SetStr` array with a zero byte should look like in a `SetMsg`: the client reads text records only
  to the first NUL, so the mirror keeps the bytes before it (the live looks array ends in its only zero).

## 8. Locks

`lock` (4) and `unlock` (6) name one or more 16-bit lock words (`LockId`); the invitation scripts lock the
player SIDs of the inviter and every guest, so two invitations cannot claim the same player
(`docs/protocol/messages.md` section 3.2). The store keeps a `LockTable` from word to connection:

| Client sends | The store does | Reply |
|---|---|---|
| lock, every word free or already the connection's | takes them all | Ack, `whichCmd` 4, to the request's `fromSID` |
| lock, a word held by another connection | takes none | Nak, `whichCmd` 4, the first such word at byte 5 (`LockRefused`) |
| unlock | frees the named words the connection holds; another connection's stay | none |
| hang-up or Login again | frees every word the connection holds (`disconnect`) | none |

A connection, not a SID, holds a lock: the inviting script has no SID and sends `fromSID` 0, so the Ack
reaches the game object and through it the room's script.

## 9. The property mirror and replicas

`SetInt` (13) and `SetStr` (14) change the properties of an object's replicas. The store keeps the last value
of every property of every live object in a `PropertyMirror`, by byte offset, and forgets an object when it is
freed. Text is kept up to its first NUL, since that is all a `SetMsg` can carry.

`ObjectStore::replicas(sid)` names the connections with a copy of an object: its holders, the holders of its
members when it is a group, and the connections with a member in a group it belongs to. `router::set_int`,
`router::set_str` and `router::invoke_method` send the message unchanged to every replica but the sender's
connection; the sender has applied it already. An update for a SID nobody holds is neither mirrored nor sent.

| Client sends | Reply, to the requester only |
|---|---|
| getProp (32) for a live object | `SetMsg` (33) addressed to the object, one record per requested property the mirror has, in request order |
| getProp for a SID nobody holds | none |
| `GrpGetProp` (31) for a group | `GrpGetProp` with one column per requested offset and one row per member in joining order; a column is text when any member has text there, and a missing cell is 0 or empty text |
| `GrpGetProp` for something that is not a group | none |

So a player who arrives late reads what the others published before it came: the waiting room asks each
newcomer's name, game and room with getProp, and `GrpGetProp` fetches every member at once.
