# The router

`innkeeper_world::router` (`crates/innkeeper-world/src/router.rs`) carries game traffic between players:
the `Send` (command 2) and the multicast (command 28) that every game, chat line and move travels in
(`docs/protocol/messages.md` sections 3.2 and 8). It reads only the target SIDs; the payload is relayed
unread, so the host holds no game logic. The object store it asks is described in [objects.md](objects.md).

## 1. Functions

| Function | Input | Result |
|---|---|---|
| `router::send(objects, sender, &SendMessage)` | `to`, `from`, payload | one `HostMessage::Send` per recipient connection |
| `router::multicast(objects, sender, &Multicast)` | `from`, a list of SIDs, body | the `Send` of the body to each listed SID, in list order |

`PlayerSession::handle` calls them for a logged-in connection; a connection that has not logged in is
ignored like for every other command. Both are pure over a shared `&ObjectStore`: they change nothing
and return `Delivery` values that `innkeeperd` hands to the switchboard.

## 2. Who receives what

`ObjectStore::recipients(sid)` names the connections:

| Target | Recipients |
|---|---|
| an object | every connection that holds it |
| a group | every connection that holds a member of it, once however many members it holds |
| a SID nobody holds (never given out, freed, a group without members) | nobody |

Every copy has `to` the SID the sender named (the group SID for a group), `from` as sent, and the payload
byte for byte, so the receiver sees what the sender built. A connection gets its copies in request order,
and the copies of one SID go in ascending `ConnectionId`. A multicast is one such `Send` per listed SID,
so a SID listed twice is sent twice and a group in the list is not expanded to its members' SIDs.

An unknown target is logged as a warning and dropped with no Nak: no census handler reads a Nak for a
`Send`, and GOLF aborts only when its own send call fails, not when nothing arrives.

The host never originates a game message. Red Baron's fatal "group sent init!" needs a `Send` from the
group SID itself, and nothing here sets `from` but the sender.

## 3. Assumptions

| Constant | Value | Why | Source |
|---|---|---|---|
| `GROUP_SEND_ECHOES_SENDER_ASSUMED` | true | the GOLF `CC` and Red Baron `0xC9` handlers set a flag when they receive their own copy, and GOLF stops sending position updates without it | `messages.md` 11, `int14h-census/golf.md` 7.1 |

The constant covers the sender's own connection for any target it holds. With `false` the router would
leave it out of every set.

## 4. Open

- GOLF and Red Baron address their multicast to `{group, own player SID}`. With the echo on, the sender
  receives two copies, one to the group SID and one to its own SID, both with its own SID in `from`. Both
  handlers test only `from`, so one is probably enough, but the original host's behaviour is unknown; a
  capture of a two-player round would settle it (and the echo question with it).
- `GrpGetProp`, `getProp`, locks and property replication are not routed here; they belong to the object
  store's lock and mirror tables.
