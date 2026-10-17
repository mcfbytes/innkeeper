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

`ObjectStore::recipients(sid)` names the connections that hold the target; the router then leaves the
sender out where the table says so:

| Target | Recipients |
|---|---|
| an object | every connection that holds it, the sender's included |
| a group | every connection that holds a member of it, once however many members it holds, except the sender's |
| a SID nobody holds (never given out, freed, a group without members) | nobody |

A sender that wants a copy of its own update names its own SID, as GOLF, Red Baron and PaintBall do; RPG
land types leave the sender out for any target (`rpg::echoes_sender`).

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
| `GROUP_SEND_SKIPS_SENDER_ASSUMED` | true | MiniGolf replays and sends again every shot of its own that comes back; GOLF, Red Baron and PaintBall address their own SID when they need their copy, which would be redundant if a group sent it | [games.md](games.md) section 5.1 |

With `false` a group `Send` would reach the sender's connection too, as it did before 2026-10-16.

## 4. Open

- Whether the original host left out the sender's connection or only the member named in `from`; the
  two differ only for a connection with more than one member in the group ([games.md](games.md) section 9).
- `GrpGetProp`, `getProp`, locks and property replication are not routed here; they belong to the object
  store's lock and mirror tables.
