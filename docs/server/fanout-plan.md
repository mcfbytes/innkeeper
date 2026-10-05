# innkeeper fan-out plan (from the INT 14h census)

Independent work items for implementing agents, derived from `docs/protocol/int14h-census/README.md`
and the per-program censuses. Order follows `docs/PLAN.md`: common host services first (Phase 3), then
Yserbius (Phase 5), Twinion (Phase 6), then the deferred titles (golf, Red Baron, Shoppers Advantage).
Crate placement follows `docs/CONVENTIONS.md` section 3.1; nothing here changes that layering.

Reconciled with the built code on 2026-10-05, after the logon wave (commits `65103b9` to `9b526fe`) and
the shared-object and logon-policy wave (`feafb86` to `e4f5daf`): the stock client logs on to `innkeeperd`,
enters the Clubhouse Waiting Room and meets other connections there (`docs/protocol/captures.md` sections 9
to 11, `docs/server/objects.md`). Statuses below say what exists, and every open scope names the types to
extend. Waves C and D are scoped in section 3.

## 0. Rules for every item

- **Scope is closed.** An item touches only the modules it names; a needed change elsewhere becomes a
  note to the owner of that item, not an edit.
- **Facts from docs, guesses in `assumptions.rs`.** Every INFERRED behaviour (group scoping, echo to sender,
  41/2 meaning) is a named constant with a doc reference in the crate's `assumptions.rs`.
- **Golden tests** (`crates/<crate>/tests/golden/`): byte strings hand-built from the census layouts, one
  file per message, round-tripped through parse and encode. Cite the census section in the file name or
  a one-line header.
- **Transcript tests** (`crates/innkeeper-session/tests/transcripts.rs` style): scripted message
  sequences against the sans-IO world, asserting the replies byte for byte and in order.
- **DOSBox end-to-end** (`docs/dosbox.md`, `tools/dosbox/run_client.py`): the stock client against
  `innkeeperd`, captures under `work/captures/`, pass criteria named per item.
- Sizes: **S** under ~300 lines, **M** 300–1000, **L** over 1000 (code plus tests).
- Tiers: **Haiku** for mechanical, fully specified work; **Sonnet** for normal implementation; **Opus**
  where semantics are INFERRED or the item fixes ordering, concurrency or a shared contract.
- **Reuse what is built** (section 1.1): extend the named type, do not write a second one. A new module
  starts only where the item says so.
- **Status belongs to the coordinator.** An item does not edit this file; it reports what it finished and
  the coordinator updates the status column. An item does edit the docs it owns.
- **Files shared by parallel items** (section 3.3) are edited in disjoint regions, and the merge order
  there decides who rebases.

## 1. What is built and the target module map

### 1.1 Built (logon wave)

| Crate | Built | Notable types | Gap that the items below close |
|---|---|---|---|
| `tsn-link` | framing, CRC, ACK/NAK, resend, one outstanding frame, message queue with `flush`, a `quiet_until` hold that a client DATA frame ends | `Link`, `Message` | none (C4 done): `is_transmit_idle`, `hold_until_heard` |
| `pad_thai` | Hayes modem, PAD dialogue, line detection | `Line`, `LineEvent` | none |
| `innkeeper-session` | line then link; send path (`send_message`, `flush`) | `Session`, `SessionEvent`, `SessionOutput` | none (C4 done): `SessionEvent::LinkLost`, `is_transmit_idle`, `begin_program_switch` (its trigger is C14's) |
| `int14h` | the 17 exports and the transport codec with golden envelopes | `Call`, `Reply`, `Envelope`, `EnvelopeParser` | none (C12 done): `innkeeperd::int14h_listener` serves it |
| `innkeeper-world` `message` | `ClientMessage` and `HostMessage` for commands 0, 1, 2, 7 to 14, 28, 36, 41, 44, 47, 48, 53 and 59 on `WireReader`/`WireWriter`, one file per family, `Command` table of 22 rows (37, 40 and 45 added by C6 and C10), `ObjectKind` 1 to 5 and 129 (C1 done) | `ClientMessage`, `HostMessage`, `SendMessage`, `Multicast`, `SetInt`, `SetStr`, `Notice` | commands 34 and 49 are not decoded (no item yet); 4, 6, 25, 31 to 33 are not decoded (C13) |
| `innkeeper-world` `account`, `logon` path | `AccountBook::anyone()` and `listed(..)`, `Account`, `Refusal`; the policy lives in `logon::admit` | `AccountBook::admit` | none (C5 done): `logon`, `AccountBook::stored`, command 44 |
| `innkeeper-world` `land` | static `LandCatalog::stock()` (Clubhouse, SierraLand, CasinoLand on host 7): directory with version ranges, occupancy counted from the waiting room, `admits`, `runs(land_type)`; `presence` gates waiting-room joins | `LandCatalog`, `LandEntry`, `Occupancy` | none (C6 done) |
| `innkeeper-world` `player` | `PlayerSession` (`AwaitingLogin` / `LoggedIn`) turning client messages into `Delivery` values; `Send` and `Multicast` go through `router`, 40/4 renames the persona, 37 and 45 go to `mail`; `SetInt`, `SetStr` and `ObjExists` are decoded and ignored | `PlayerSession`, `Delivery` | no locks or property replication (C13) |
| `innkeeper-world` `objects` | `ObjectStore`: host-wide SIDs, reference-counted shared groups, capacity, Nak 10 codes 1 to 3 and 6, `disconnect`, `member_count`, `recipients`, `group_key` | `ObjectStore`, `ConnectionId`, `GroupKey` | no locks or property mirror (C13) |
| `innkeeper-world` `world`, `host_time`, `store` | `World` (host number, accounts, lands, clock, store), immutable and shared; `Clock` with a system and a fixed implementation; the `Store` trait with `MemoryStore` | `World::stock()`, `World::stored(store)`, `Store` | none (C7, C9 done) |
| `innkeeperd` | TCP listener, `Host` carrying `ClientMessage` to `PlayerSession` and `HostMessage` back, `Switchboard` between connections, `SqliteStore` behind `--data-dir`, hex captures, `Exchange` log | `Host`, `ConnectionSettings`, `Switchboard`, `SqliteStore` | none for wave C: `Host::answer` is transport-neutral, the INT 14h listener runs on `--int14h-bind` (C12), a lost call releases its objects (C4) |
| `tools/dosbox` | headless stock-client harness with key scripts `create-persona`, `play`, `clubhouse`, and the launcher that starts a DOS game without the LSCI land (C11 done) | `run_client.py`, `key_scripts.py`, `launcher/` | no key script for chat (`mailbox` added by C10) |
| ScummVM fork | `TsnExecutive` (the 17 exports), `OfflineTsnExecutive`, `TsnSession`, `kTsn`; the Clubhouse boots to the Dialing screen (`docs/lsci/scummvm-integration.md` section 10) | `TsnExecutive` | no executive that talks to `innkeeperd` (S1) |

Tests today: 185 across the workspace. The golden logon rows are `crates/innkeeper-world/tests/golden/logon.txt`; the
scripted multi-player transcripts are `crates/innkeeper-world/tests/transcripts.rs`.

### 1.2 Target module map

| Crate | Existing modules | New or extended modules | Owns |
|---|---|---|---|
| `int14h` | `call`, `reply`, `envelope` | none expected | export types and transport codec |
| `innkeeper-session` | `session` | `outbox` only if `tsn-link`'s queue is not enough, `program_switch` | per-connection state: drain/idle, link-loss events, quiet window at program switch |
| `innkeeper-world` | `message`, `account`, `land`, `player`, `world`, `assumptions` | `message` (more files, one per family), `objects`, `router`, `logon`, `host_time`, `presence`, `chat`, `mail`, `bbs`, `store` (trait), `game::rpg`, `game::yserbius`, `game::twinion`, `game::golf`, `game::red_baron`, `game::shopadv` | domain behaviour: object store, routing, host services, per-game service objects |
| `innkeeperd` | `server`, `connection`, `host`, `config`, `capture` | `switchboard` (C2), `int14h_listener` (C12), `sqlite_store` (C9) | sockets, delivery between connections, the INT 14h listener, persistence, wiring |
| `tools/` | `dosbox/*`, `int14h_census.py` | `dosbox/launcher/` (C11) | test harness only |

## 2. Work items

Status key: **done**, **partial** (what exists is named), **open**.

### 2.1 Common host services (Phase 3)

| Id | Status | Remaining scope | Crate / module | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|---|
| C1 | done: every family of `messages.md` 3.2.1 is typed and golden-tested; the scope text is what was built | **Rest of the shared-object codec**: client side `Send` 2 (relay body kept as raw bytes from byte 6, since LSCI has a word msgType and the DOS libraries a byte), `GrpDel` 11, `GrpMem` request 12, `SetInt` 13, `SetStr` 14, multicast 28 (`n` counts word SIDs: `5 + 2n + body`, redbaron.md 5.1), `ObjExists` 41 (raw tail, typed DOS land-type form); host side `Send`, `GroupLeft`, `GroupMembers` (@6..), `ObjectFreed`, `SetInt`/`SetStr` delivery, `ObjectLocated` (cmd 41, SID at w@6), `Notice` 48, a generic `Ack`. Widen `ObjectKind` to the DOS kinds 1..5 and 129 (kind 3 is the Red Baron service, and kind 4 means different things per land type, so group-ness leaves the codec). New files per family under `innkeeper-world::message`, exports in `lib.rs`. 36/x replies stay with C6 and C7 | `innkeeper-world::message` | none | golden bytes per family from the census tables, round trip in both header shapes (same bytes), unknown command → error value | M | Sonnet high |
| C2 | done | **Shared object store**: lift `Player.objects`, `groups` and `next_sid` out of `player.rs` into `objects::ObjectStore`, one per host, with a host-wide SID allocator (reuse `allocate_sid` and its wrap test); `ConnectionId` and `Delivery { to, message }` so a reply can go to another connection; `innkeeperd::switchboard` and `host.rs` carry deliveries through per-connection inboxes. joinNet → `ObjId` in request order (cookie echoed); shared group key `(kind, landType, parameter)` returns one SID with a reference count and the request's capacity; GrpJoin → echo to joiner and members, Nak 10 codes 1 to 3; GrpDel notify; GrpMem reply; ObjFree of a group = release one reference; a game-object re-join with the same cookie replaces the old object (Clubhouse entry, `captures.md` section 11); `disconnect(connection)` and a repeated Login release everything the connection held | `innkeeper-world::objects` (+ `innkeeperd::switchboard`, `host`, `connection`) | C1 | transcripts with two `PlayerSession`s on one store (ordering, echo, Nak codes, last-release), SID uniqueness property, two TCP clients in the `innkeeperd` server test, DOSBox `clubhouse` run unchanged | L | Opus high |
| C3 | done: `router::send` and `router::multicast` over `ObjectStore::recipients` ([router.md](router.md)); a multicast sends one `Send` per listed SID and a group SID is not expanded to member SIDs, so with the echo on GOLF and Red Baron get two copies (router.md section 4, to check in D2 and D3); an unknown SID is dropped without a Nak | **Router**: `Send` (2) to an object SID or fan-out to a group SID with the real `fromSid`, body untouched; multicast (28) split per recipient, group SIDs expanded; echo-to-sender policy as one assumption | `innkeeper-world::router` | C2 | transcripts: GOLF CC and RB 0xC9 loopback, Yserbius/Twinion relay of 200–219 unchanged, RB "group sent init!" never triggered | M | Sonnet high |
| C4 | done: `Session::is_transmit_idle`, `SessionEvent::LinkLost` (PAD `D`, modem hang-up) answered by `Host::hang_up`, `Session::begin_program_switch` (mechanism built, trigger open, C14) | **Session lifecycle**: `Session::is_transmit_idle` (nothing queued or unacknowledged), `SessionEvent::LinkLost` for a PAD disconnect or modem hang-up replacing the silent `HostLink::Offline` (a TCP close stays the driver's), a quiet window across a program switch built on `Link`'s `quiet_until` (the trigger is C14's). `innkeeperd::connection` calls C2's `disconnect` on the event | `innkeeper-session::session` (+ `program_switch`) | none for the session crate; C2 for the integration test | transcripts: hang-up emits the event once; no frame leaves in the quiet window; idle only after the last ACK | M | Opus high |
| C5 | done | **Logon policy and accounts**: move `log_in` into `logon::admit`; `Account` carries `user_flags`, `rating` and `LoginStatus` from its record so `LoginAck` is no longer default; `AccountBook::stored` over the C9 `Store` (and an enrolling dev mode); Login land-type check against `LandCatalog` with a named Nak reason; command 44 password change (`ClientMessage::ChangePassword`, Ack 44/1, record updated); `innkeeperd --data-dir` | `innkeeper-world::logon`, `account`, `world` (+ `innkeeperd::config`, `server`) | C9 | golden Ack with non-zero fields; transcripts for wrong then right password, 44, unknown land type; DOSBox `clubhouse` run with a data dir | M | Sonnet medium |
| C6 | done: 47/1 `current` counts the land's waiting room (DOSBox `clubhouse` after the wave C merge: 0 on the map, 1 once the player joined the Waiting Room), Nak 10 code 6 from `presence::join_group`, 40/4 renames the session's persona; 36/1 and 49 stay unanswered | **Presence**: occupancy `current` counted from the C2 store; waiting-room group (joinNet kind 5, cap from `size`, which is 128 live while the catalog says 64) with the 10/3 version check (Nak code 6); 40/4 sets the persona name (`Account.persona`). 36/1 HOSTADDR stays unanswered and UserFindMsg 49 waits for its reply layout (section 4) | `innkeeper-world::presence`, `land` | C2, C5 | golden replies; DOSBox: hub map lists lands and occupancy | M | Sonnet high |
| C7 | done: 36/2 is answered from `World.clock` | **Host time**: `HostMessage::HostTime` (36/2 reply `b 36, b 2, b year-1900, b month0, b mday, b hour, b minute, b second`), `host_time::Clock` with a system and a fixed implementation, a `clock` in `World`, the `answer_host_info` arm | `innkeeper-world::host_time` | none | golden bytes at a fixed clock; RB and GOLF parse it (census 8.2 / 6.2) | S | Haiku medium |
| C8 | open (wave D) | **Chat and notices**: Send msgType 1 room chat, multicast 28 conferences (msgType 50), ObjExists 41/0 conference name check (research first: how a conference is named), operator push Unsolicited 48 from a `say` line on standard input | `innkeeper-world::chat`, `innkeeperd::operator` | C3, C6 | transcripts; DOSBox ladder step 2 (two clients chat) | M | Sonnet high |
| C9 | done: `Store`, `MemoryStore`, `SqliteStore`, one conformance suite | **Persistence boundary**: `Store` trait (accounts, mailboxes, boards; letters and posts are the client's own bytes), `MemoryStore`, `SqliteStore` on `rusqlite` (0.40 is in the cargo cache, `bundled`) | `innkeeper-world::store`, `innkeeperd::sqlite_store` | none | one conformance suite run against both implementations | M | Sonnet high |
| C10 | done: 45/1 and 37/17 to 37/32 over the store ([mail.md](mail.md)); DOSBox reached "Mailbox 1" and the empty box, but send and read between two accounts are covered by transcripts only; enrolled accounts lack the stamps flag `0x200`, so they cannot send | **Mail**: EMMsg 37 (new-mail check, send, read, delete, forward, services, system list), NewBoxHandler 45 mailbox assignment | `innkeeper-world::mail` | C5, C9 | golden replies from `messages.md` 3.3; DOSBox: send and read a letter between two accounts | L | Sonnet high |
| C11 | done: `tools/dosbox/launcher/`, `run_client.py --launch` (`docs/dosbox.md`) | **Program-launch harness**: a test `TSN.PRG` block plus a tiny DOS launcher that connects, logs in, writes a chosen shared block (SetSharedData), names a game with SetNextProgram and exits, so any DOS game can start without driving the LSCI land | `tools/dosbox/` (+ `launcher/`) | none | launcher starts GOLF with a hand-made block and GOLF reaches its joinNet in a capture | M | Opus medium |
| C12 | done: `innkeeperd::int14h_listener` on `--int14h-bind` (default 127.0.0.1:2315), parity and mixed-room tests; deviation: ack timeout, next program and the callbacks flag are acknowledged and logged, not stored, because no export reads them back | **INT 14h transport listener**: serve `int14h-transport.md` envelopes in `innkeeperd`, mapping Connect/SwitchHost/Login onto the same session and world as the legacy link; GetStatus `AL`/`AH` and GetLineRate per lsci.md section 10; `Host` stops depending on `Session` | `innkeeperd::int14h_listener` (+ `host`, `connection`, `server`, `config`) | C2, C5 (done); C4 only for the legacy link | `int14h` golden envelopes driven through a loopback session; parity transcript legacy vs transport, one legacy and one transport client in the same room | M | Opus medium |
| C13 | open (wave D) | **Locks and property replication** for LSCI games: lock 4 / unlock 6 (Nak with holder SID at w@5), setInt 13 and setStr 14 relayed to the replicas and mirrored, invokeMethod 25, getProp 32 / SetMsg 33, GrpGetProp 31 as far as `messages.md` pins them; the live Clubhouse already sends 13, 14 and 26 unanswered | `innkeeper-world::objects` (locks, mirror), `message::lock`, `router` (replication) | C2, C3 | transcripts from `messages.md` 3.2; DOSBox ladder step 3 (a card game) | M | Opus medium |
| C14 | open (wave D); a repeated Login already starts a fresh `Player`, C2 makes it release | **Land switch**: SwitchHost accepted whatever the address, leaveNet of the inherited SID (also a game group SID returned by GOLF/RB, after the players' own GrpDel), re-login, join of the new land | `innkeeper-session::program_switch`, `innkeeper-world::logon` | C4, C5, C6 | DOSBox: walk hub → SierraLand → hub with no orphan objects in the store | M | Opus high |

### 2.2 Yserbius and Twinion (Phases 5 and 6)

Both use `hostcomm.cpp` + `rpgcomms.cpp`; a shared `game::rpg` module carries the common behaviour and a
profile per title carries the differences (twinion.md section 7). All open.

| Id | Item and scope | Crate / module | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|
| R1 | **RPG service object**: 41/2 (land type `0x8B`) → cmd 41 with a service SID at w@6 within 60 s (C1 `ObjectLocated`); 17/2 props 8–11 accepted (Twinion), optional setInt 13 whose third pair value (w@0x10) is the step delay 12..48; 40/4 set name; 48 operator text | `innkeeper-world::game::rpg` | C2, C3 | golden replies per census 4.1/4.2; transcript: the 60 s resend loop ends after one reply | S | Sonnet medium |
| R2 | **RPG group scoping**: kinds 1 player, 5 land group (landType, land number, cap 100), 2 map group (`0x8B`, map number, cap 104 or 80), 4 personal party (cap 4) as `GroupKey` rows over C2's store; ObjID matched by request order; 10/3 version gate (1.1.7 / 1.0.22); Nak whichCmd 10 codes 1–6 only, never 0 or >6 | `innkeeper-world::game::rpg` | C2 | transcripts: two players in one land and map get the same group SIDs; full map gives code 2; wrong version gives code 6 | M | Opus high |
| R3 | **RPG bulletin board**: 27/13 directory (17-byte records + names), 27/14 index, read subs 1–5, post 27/0 (`name|text`, 309 bytes), Nak 27 = offline, privileged text offset @0x16 by `userFlags` | `innkeeper-world::bbs` (+ `game::rpg` binding) | C9 | golden bytes for each reply layout; DOSBox: post in DARKSTRT, read it in FATES' board | M | Sonnet high |
| R4 | **Yserbius profile and end-to-end**: TSN.PRG blocks `Yserbius`/`Yserba`, shared block from the hub (`+0`, `+2`, `+6`, `+8`), version 1.1.7, map cap 104; two DOSBox clients see each other on a map, form a party (204/212/211), chat (207), leave cleanly (GrpDel) | `innkeeper-world::game::yserbius`; DOSBox scripts | R1, R2, R3, C4, C14 | DOSBox two-client run; captures diffed against census message tables | M | Opus high |
| T1 | **Twinion profile**: version 1.0.22, land type fixed 8 (block `+8` ignored), map cap 80, 17/2 live, game header `w map @6, b @8, b @9` passed through, msgType 219, `TWINA` case-insensitive; no unsolicited traffic to TWGENN | `innkeeper-world::game::twinion` | R1, R2, R3 | transcripts for each delta row of twinion.md 7 | S | Sonnet medium |
| T2 | **Twinion end-to-end**: gallery → FATES → party → back to gallery and hub | DOSBox scripts | T1, R4 | DOSBox two-client run | S | Sonnet high |

### 2.3 Deferred titles (golf, Red Baron, Shoppers Advantage)

All open.

| Id | Item and scope | Crate / module | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|
| D1 | **Game-launch hand-off from LSCI**: how a land forms the game group (GameGroup joinNet kind 2, RedBaronQueue / `global119`), writes `+4` and the `+0x80` parameters, and frees the group when players return. Research first (LSCI scripts, not the DOS binaries), then implement the host side | `innkeeper-world::objects` (group lifetime), `game::golf`/`game::red_baron` launch rules | C2, C13, C14 | DOSBox: two clients launch the game from the land; group survives the program switch | M | Opus high |
| D2 | **Golf**: joinNet kind 1 type `0x66`, GrpJoin/GrpMem within 120 s, GrpDel broadcast on leave or drop, multicast CC loopback, Unsolicited 48; nothing persisted | `innkeeper-world::game::golf` | C3, C4, D1 (or C11 for standalone tests) | transcripts of golf.md 7.2 handshake; DOSBox two-player round via C11 | S | Sonnet medium |
| D3 | **Red Baron service object**: 41/2 type 101 → service SID; 17/0 prop 9 → setInt `0x0201`, prop `0xF` → value 5..30; joinNet kind 3; absorb 50 (0x32) status; 36/2 from C7; GrpJoin echo, GrpMem, multicast 0xC9 loopback | `innkeeper-world::game::red_baron` | C3, C7, D1 (or C11) | golden replies; DOSBox two-plane session via C11 | S | Sonnet medium |
| D4 | **Shoppers Advantage, closed**: answer LSCI's 54/27 with 54/20 and a NUL-terminated "closed" text at byte 15 (needs a `PMSMsg` 54 codec, which C1 does not cover) | `innkeeper-world::game::shopadv` | C1, C5 | golden reply; DOSBox: mall entry shows the text | S | Haiku medium |
| D5 | **Shoppers Advantage, joke edition**: 54/27 → 54/19; JOIN 54/25 → 54/17 token; TEXT 54/24 answered within 636 ticks with 54/16 kind 1 text (CR LF and LF terminators, w@2 < 0x1000, full-length bodies); `/TO MAIN`, `/TO STORE`, `/EXIT`; EXIT 54/26 → 54/19 kind `0x1A`; replies are plain Amazon search URLs (no scraping, no API) | `innkeeper-world::game::shopadv` | D4, C4 | golden 54/x bytes; DOSBox: search, then Exit returns to the land | M | Sonnet medium |

### 2.4 Tooling (any time)

| Id | Item and scope | Where | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|
| X1 | Fix `tools/int14h_census.py` (done, `fcdf26c`): drop C++ virtual calls (`mov bx,es:[bx]` before `lcall [bx+N]`), scan to `+44`, fix the far-call scan after a `0x9A`-low-byte segment; coordinate with its owner | `tools/int14h_census.py` | none | reproduces every count in the census README matrix | S | Sonnet medium |
| S1 | **ScummVM online executive**: `OnlineTsnExecutive` in the fork speaking the `int14h-transport.md` TCP framing through `Networking::Socket::connect`, chosen by a `lsci_host` option; the offline executive stays the default | `scummvm/engines/sci/engine/tsn_online_executive.*`, `SciEngine`, `test/engines/sci/` | C12 | unit test on a fake socket with the golden envelopes; headless Clubhouse boot logs in against `innkeeperd` | M | Sonnet high |

## 3. Dependency order and parallel waves

| Wave | Items (parallel within a wave) | Milestone |
|---|---|---|
| A, done | C1, C7, C9, C11, X1 | full codec, clock, store, a way to start any DOS game, a working census tool |
| B, done | C2, C5 | two connections share objects; accounts persist; step 1 closes with a data dir |
| C, done | C3, C4, C6, C10, C12 | ladder step 2 can begin: game traffic is relayed, occupancy is counted, mail works, and `innkeeperd` serves the INT 14h transport |
| D (hand out now) | C8, C13, C14, S1 | ladder steps 2 to 4: chat and notices, a card game, land switch, ScummVM logs on |
| E | R1, R2, R3, then R4, T1, T2 | RPG services, then the Phase 5 and 6 milestones |
| F (deferred) | D1, then D2, D3; D4 any time after C5, D5 after D4 | DOS games through the land |

Changes since the last plan: waves A and B are built, so their closed scopes are gone from this file (the
commits `feafb86`, `138b9e0`, `b369971`, `fcdf26c`, `188ce2a`, `43694d1` hold them). C12 moved from the fourth
wave to wave C: the transport has no link layer, so its link loss is a TCP close and its idle query is
constant, and the only thing it needs is a `Host` that does not know which transport it serves, which C12
makes itself. C4 keeps the legacy link's side (idle, hang-up, quiet window). C6 lost the HOSTADDR reply and
`UserFindMsg` (section 4). C8 gained a research step, and S1 (the ScummVM online executive) is new.

Critical path: C3 → C13; C4 + C12 → C14 → R4 → T2; C12 → S1. Wave C items touch disjoint modules except the
files in section 3.3, whose merge order is fixed there.

Every item also follows section 0, the commit rules of `docs/CONVENTIONS.md` (subject `crate: summary`, never an
item id) and its gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked`, and `tools/check_comments.py` on each touched path. The 185 tests of today stay green.

### 3.1 Wave C: closed scopes

#### C3 Router (Sonnet high, M)

- **Owns.** New `innkeeper-world/src/router.rs`: `router::send` and `router::multicast`, each taking
  `(&ObjectStore, ConnectionId, &message)` and returning `Vec<Delivery>` of `HostMessage::Send`. In `objects.rs` one
  read-only `ObjectStore::recipients(sid) -> BTreeSet<ConnectionId>` (the holders of an object, the connections that
  hold a member of a group, empty for an unknown SID). In `player.rs` the `Send` and `Multicast` arms of `handle`;
  `SetInt`, `SetStr` and `ObjExists` stay ignored. One constant in `assumptions.rs`, the `mod` and `pub use` lines in `lib.rs`.
  Its first commit moves the `Table` harness of `tests/transcripts.rs` to `tests/common/mod.rs`, so later items add
  test files of their own.
- **Behaviour.** A `Send` to an object SID goes to every holder of it. A `Send` to a group SID goes once to each
  connection holding a member of the group, with `to` the group SID and `from` and the payload unchanged; whether the
  sender's own connection is among them is `GROUP_SEND_ECHOES_SENDER_ASSUMED` (true: the GOLF CC and Red Baron `0xC9`
  handlers count their own copy). A multicast becomes one `Send` per recipient SID in list order, the multicast body being the
  payload. An unknown SID is dropped with a warning and no Nak. The host never originates a game message (Red Baron's
  "group sent init!" is never triggered). One connection receives in request order; several in ascending `ConnectionId`.
- **Tests.** `tests/routing.rs` transcripts with bytes hand-built from `golf.md` 7, `redbaron.md` 5.1 and
  `yserbius.md` 4.2: a group `Send` reaches both players of a waiting room (echo per the constant); Yserbius and Twinion
  msgType 200 to 219 bodies arrive byte for byte; a multicast to two object SIDs on two connections plus one group SID
  gives the expected `Send`s in order; a `Send` to a freed SID gives nothing; an unauthenticated connection is ignored.
  Unit tests for `recipients` after `leave_group` and `disconnect`.
- **Docs.** New `docs/server/router.md`; `messages.md` section 11 (the echo question becomes the named assumption) and
  rows 2 and 28 of 3.2.1; `innkeeperd.md` section 5 (`router` row, rows for 2 and 28 in the reply table) and section 7 (first bullet).

#### C4 Session lifecycle (Opus high, M)

- **Owns.** `tsn-link/src/link.rs` (and `packer.rs` if the idle test needs it): `Link::is_transmit_idle()` (nothing
  packed, no closed frame waiting, no frame awaiting its ACK) and `Link::hold_until_heard(now, longest)`, which reuses the
  existing `quiet_until` (a DATA frame from the client already ends it). `innkeeper-session/src/session.rs`:
  `Session::is_transmit_idle()`, `SessionEvent::LinkLost(LinkLoss)` with `LinkLoss::{PadDisconnected, ModemHungUp}`
  emitted once when the link goes from online to offline (never for a session that was not online, never twice), and
  `Session::begin_program_switch(now)` in the new `program_switch.rs`: host frames are held until the client's next
  DATA frame or `PROGRAM_SWITCH_QUIET_MAX_ASSUMED`, then leave in order. The new `innkeeper-session/src/assumptions.rs`
  names that constant. In `innkeeperd/src/connection.rs` one arm: `LinkLost` calls `host.hang_up()` and is logged and
  captured. Arming the quiet window is C14's.
- **Tests.** `innkeeper-session/tests/` transcripts gain directives for idle and for expected events: hang-up by `ATH`
  and by the PAD's `D` each emit the event once; idle only after the last ACK; no frame leaves
  in a quiet window and the queued ones leave in order at its end or its deadline; `tsn-link` unit tests for the two
  `Link` methods. `innkeeperd` test: a modem hang-up with the TCP socket still open releases the connection's
  objects and tells its peers. Pass: all of these and the golden transcripts unchanged.
- **Docs.** `innkeeperd.md` section 1 (lifecycle paragraph) and section 4 (link and hang-up rows);
  `link-layer.md` checklist 12 gets the quiet-window sentence; the quiet-window question in section 4 below is
  restated as "mechanism built, trigger open (C14)".

#### C6 Presence (Sonnet high, M)

- **Owns.** New `innkeeper-world/src/presence.rs`; `land.rs`: `LandCatalog::occupancy(host, &ObjectStore)` counts
  `current` as `ObjectStore::member_count` of the waiting-room key `(LandGroup, land_type, land_number)` while `maximum`
  stays the catalog's 64 (the group's own capacity is the joinNet `size`, 128 live), and `LandCatalog::admits(land_type,
  ClientVersion)` from the directory row's version range. New `message/user_info.rs` for command 40 sub 4
  (`ClientMessage::SetPersona`, one row in `command.rs`, exports in `message/mod.rs` and `lib.rs`; byte layout
  `28 04 "guybrush\0"` from `captures.md` 6.185). `objects.rs`: `ObjectStore::group_key(sid)` and the refusal code 6
  (wrong version) on the existing `GroupJoinRefusal`. `player.rs`: the `GroupJoin` arm passes through the version gate
  (a `GroupJoin` without version bytes is not gated), the `LandOccupancyRequest` arm uses the new counts, a `SetPersona` arm
  updates the logged-in account's persona, which a new `PlayerSession::persona()` reports. The reply to 40/4 is none (`PERSONA_SET_UNANSWERED_ASSUMED`); the persona is
  not written back to the store.
- **Not in scope.** The 36/1 HOSTADDR reply stays unanswered (the client keeps its file) and `UserFindMsg` 49 stays
  undecoded, because its reply layout is unknown (section 4).
- **Tests.** `tests/presence.rs` transcripts: 47/1 shows `current` 1 after A enters the Clubhouse waiting room, 2 after B,
  1 after A hangs up, `maximum` 64 throughout, other lands 0; a 129th joiner of the 128-member room gets Nak 10 code 2;
  a `GroupJoin` with a version outside the row's range gets Nak 10 code 6 and is not added; 40/4 changes the persona
  that `PlayerSession` reports. Golden `tests/golden/user_info.txt` round trip. DOSBox: the `clubhouse` key script still reaches
  the Waiting Room and the capture's 47/1 reply shows `current` 1 for the Clubhouse.
- **Docs.** `innkeeperd.md` section 5 (`land`, `player` rows; the 47/1 and 40/4 reply rows); `messages.md` row 40 of 3.3
  and the occupancy note in section 6.

#### C10 Mail (Sonnet high, L)

- **Owns.** New `innkeeper-world/src/mail.rs` and `message/mail.rs` (EMMsg 37 and NewBoxHandler 45, both directions;
  rows 37 and 45 in `command.rs`; variants in `client.rs` and `host.rs`; exports in `message/mod.rs` and `lib.rs`);
  the arms in `player.rs`. Only the existing `Store` calls are used (`assign_mailbox`, `append_letter`, `letters`,
  `delete_letter`); a change the trait needs is a note to the owner of `store.rs`. Sub-commands: 45/1 assign (status 1
  with the number as low and high words, status 2 to 6 for errors), 37/17 and 37/18 new-mail check, 37/20, 21 and 24
  send, 25 read, 26 delete, 27 forward, 30 form to a service box (accepted and dropped), 31 services and 32 system list
  (empty). Letters are the client's own bytes; the host reads only the fields it needs (the recipient mailbox, the letter
  id). Layouts come from `messages.md` 3.3 and the handlers in `hub/script.*` (`docs/lsci/`); a reply field the client never
  reads is a named zero in `assumptions.rs`. On the open book (`World::stock()`, no stored accounts) 45/1 answers an
  error status and 37 requests are refused, as a named assumption.
- **Tests.** Golden `tests/golden/mail.txt`, one line per request and reply from the census layouts; `tests/mail.rs`
  transcripts on a stored world: A sends B a letter, B's new-mail check says 1, read returns the bytes unchanged,
  delete brings it to 0, the mailbox number is stable across a second 45/1. DOSBox with `--data-dir`: a `mail` key script in
  `tools/dosbox/key_scripts.py` sends and reads a letter between two accounts; if the mail dialog cannot be driven by
  keys, the transcripts are the criterion and the doc says why.
- **Docs.** New `docs/server/mail.md`; `messages.md` rows 37 and 45 of 3.3 (host column, INFERRED labels);
  `store.md` section 5; `innkeeperd.md` section 5 and 7.

#### C12 INT 14h transport listener (Opus medium, M)

- **Owns.** New `innkeeperd/src/int14h_listener/mod.rs` (accept loop, per-connection pump on `int14h::EnvelopeParser`
  and `encode_envelope`) and `executive.rs` (the sans-IO state of one transport session: connected flag, 256-byte shared
  block, next and previous program, ack timeout, callbacks flag, and the queue Receive pops). `host.rs`: `Host::answer`
  returns the world's outgoing `HostMessage`s plus the `Exchange` log instead of writing to a `Session`; the inbox drain
  under the store lock stays inside it. `connection.rs`: the legacy driver encodes and sends what `answer` returns
  (`send_all` moves there) and the connection ids come from one counter in `ConnectionSettings` that both listeners
  use. `server.rs` spawns the second listener; `config.rs` adds `--int14h-bind ADDR` (default `127.0.0.1:2315`) and
  `--no-int14h`. The `int14h` crate is not changed; a defect there is a note to its owner.
- **Behaviour.** HELLO gets WELCOME (version 1, "innkeeperd"); any other first envelope, a version mismatch or a malformed
  envelope closes the connection. Per call, with `int14h-transport.md` section 5 and `lsci.md` section 10: GetStatus
  driver 3 and `AH` `0x80` from Connect until Disconnect; GetLineRate is `--connect-rate`; Connect accepts any dial string
  (result 0, also when already connected); Send queues true after the body went through `Host::answer` and false when not
  connected or empty; Receive pops the oldest queued host message (replies and the switchboard's notices, kept in order);
  Poll and Service answer status 0, and 1 once when the server ends the session; IsTransmitIdle is true (calls are handled in
  order, so nothing the client sent is unprocessed); SwitchHost is accepted whatever the address and clears the
  Receive queue (C14 refines it); Flush is a no-op; the executive-local exports keep per-session state. A client that
  stops calling Receive is cut off at `RECEIVE_QUEUE_LIMIT`. Closing the socket or Disconnect calls `Host::hang_up`.
- **Tests.** The golden envelopes of `int14h-transport.md` section 6 are replayed against a loopback socket (HELLO to
  WELCOME byte for byte, Connect, GetStatus). A transport client sends the captured Login and joinNet bytes of
  `captures.md` and reads exactly the legacy replies (parity with `Host` run through a `Session`). One legacy TCP client and one
  transport client enter the same waiting room and each receives the `GrpJoin` of the other; closing the transport socket
  gives the peer `GrpDel` and `ObjFree`. SwitchHost, Disconnect and an oversize queue each have a case.
- **Docs.** `int14h-transport.md` (status line, section 7: served, and the server choices for IsTransmitIdle,
  SwitchHost and Poll); `innkeeperd.md` sections 1, 2 and 7.

### 3.2 Wave D: closed scopes

#### C8 Chat and notices (Sonnet high, M)

- **Owns.** New `innkeeper-world/src/chat.rs` and the `ObjExists` arm of `player.rs`; in `innkeeperd` an `operator.rs`
  reading `say TEXT` lines from standard input into `Switchboard::broadcast` (new, one method) as `HostMessage::Notice`.
- **Research first.** How a conference gets its name and what the 41/0 name check expects back when the name is free
  (`GotoConference::init`, `Conference::init`, `docs/lsci/`, the LSCI scripts); the result is a table in `messages.md` 3.2.1
  labelled CONFIRMED or INFERRED, and every INFERRED choice a named constant.
- **Behaviour.** Room chat (msgType 1) and conference chat (multicast, msgType 50) need no code beyond the router;
  C8 proves them in transcripts. Conference names are a registry on the host (`chat::Conferences`) fed by what the
  research finds; 41/0 answers with `ObjectLocated`.
- **Tests.** Transcripts with two players chatting in a room and in a conference; the name check for a taken and a
  free name; `Notice` reaches every connection and a closed one is skipped. DOSBox ladder step 2: two clients chat.
- **Docs.** `messages.md` 3.2.1 and 3.3 row 41; `innkeeperd.md` section 2 (`say`) and 5.

#### C13 Locks and property replication (Opus medium, M)

- **Owns.** New `message/lock.rs` (lock 4, unlock 6, Ack and Nak with the holder SID at word 5) and the codec of
  invokeMethod 25 and getProp 32 / GrpGetProp 31 with their `SetMsg` 33 replies as far as `messages.md` pins them;
  `objects.rs`: a lock table (`lockId` to holder, released with the connection, so `disconnect` frees them) and
  a property mirror per object that `SetInt` and `SetStr` update; `router.rs`: `SetInt`, `SetStr` and invokeMethod relayed to
  the recipients of the target; `player.rs` arms; commands 26 and 30 recorded as known and unanswered.
- **Tests.** Transcripts from `messages.md` 3.2: lock taken, a second taker gets the Nak with the holder, unlock, the
  hang-up of the holder releases it; `SetInt` and `SetStr` reach the other replicas and not the sender, and a late
  joiner's getProp returns the mirrored values; the live Clubhouse's 13, 14 and 26 are absorbed with the replies the
  census shows. DOSBox ladder step 3: a card game table.
- **Docs.** `messages.md` 3.2 rows 4, 6, 13, 14, 25, 26, 30 to 33; `objects.md` (locks and the mirror).

#### C14 Land switch (Opus high, M)

- **Owns.** `innkeeper-session/src/program_switch.rs` (the trigger for C4's `begin_program_switch`, chosen and recorded as
  an assumption), `innkeeper-world/src/logon.rs` (the host number a SwitchHost address names, link-layer 3.2 grammar,
  refusing hosts this server does not run), the repeated-Login and new-call path in `player.rs`, and the SwitchHost
  case in `innkeeperd/src/int14h_listener/executive.rs`.
- **Behaviour.** A new host call or a SwitchHost for a host this server runs releases the connection's objects (the
  inherited game SID and a game group SID returned by GOLF or Red Baron after the players' own `GrpDel` included),
  resets the player to awaiting Login, and keeps the quiet window around the switch. A host it does not run gets
  `HostUnreachable` on the transport and a `DISCONNECTED` from the PAD.
- **Tests.** Transcripts hub to SierraLand to hub with `ObjectStore::is_empty()` after each leg; the quiet window is armed and
  ends at the child's first message; parity of the legacy and transport paths. DOSBox: the walk hub, SierraLand, hub leaves
  no orphan objects (capture and a store dump in the test log).
- **Docs.** `messages.md` section 5 (host side) and the open question in section 4; `innkeeperd.md` section 5.

#### S1 ScummVM online executive (Sonnet high, M)

- **Owns.** In the fork, branch `lsci`: `engines/sci/engine/tsn_online_executive.{h,cpp}` implementing the existing
  `TsnExecutive` over `Networking::Socket::connect` (the precedent is `engines/scumm/he/net/net_lobby.cpp`, behind
  `USE_LIBCURL`) with the TCP framing of `int14h-transport.md`; selection in `SciEngine` by a `lsci_host` option
  (`host:port`, empty keeps `OfflineTsnExecutive`); `engines/sci/module.mk`; a unit test in `test/engines/sci/` against
  an in-memory fake socket using the golden envelopes. Executive-local exports stay on the client as the transport allows.
- **Pass.** The unit test; `make test` and a warning-free build; with `innkeeperd` running, the headless Clubhouse
  boot (`docs/lsci/scummvm-integration.md` section 12) gets past Dialing, logs in and receives the login Ack, with the
  capture in `work/scummvm-shots/`.
- **Docs.** `docs/lsci/scummvm-integration.md` section 10 (online executive) and the open-work list; `ktsn.md` is unchanged.
  Commit subjects start `SCI:` and stay within 50 characters; trailer `Assisted-by: Claude:claude-opus-5-5`, never `Co-Authored-By`.

### 3.3 Shared files and merge order

| File | Items | Rule |
|---|---|---|
| `innkeeper-world/src/lib.rs`, `assumptions.rs` | C3, C6, C10 (C); C8, C13 (D) | each adds its own `mod`, `pub use` and constants; append only |
| `innkeeper-world/src/message/{command,client,host,mod}.rs` | C6, C10 (C); C13 (D) | new families go in files of their own; C6 lands before C10 on `command.rs`, whose `COMMANDS` length changes |
| `innkeeper-world/src/player.rs` | C3, C6, C10 (C); C8, C13, C14 (D) | each owns its own `match` arms; landing order C3, C6, C10 |
| `innkeeper-world/src/objects.rs` | C3, C6 (C); C13, C14 (D) | C3 adds `recipients` first, C6 adds `group_key` and the refusal code, C13 the lock table and mirror |
| `innkeeper-world/tests/transcripts.rs` | C3 | C3 moves the harness to `tests/common/mod.rs` in its first commit; everyone else adds a test file |
| `innkeeperd/src/{host,connection,server,config}.rs` | C4, C12 (C); C8, C14 (D) | C4 lands its one arm in `connection.rs` first, C12 rebases onto it |
| `docs/server/innkeeperd.md`, `docs/protocol/messages.md` | all | each edits only the rows named in its docs bullet |

## 4. Open questions that block or steer items

| Question | Blocks | Source |
|---|---|---|
| Is a group `Send` echoed to its sender? Assumed yes (`GROUP_SEND_ECHOES_SENDER_ASSUMED`, C3); a multicast to {group, own SID} then reaches the sender twice (`router.md` section 4) | R4, D2, D3 | every census; `messages.md` 11 |
| What 41/2 looks up and what SID it must return | R1, D3 (any stable SID works for the clients) | yserbius.md, twinion.md, redbaron.md open questions |
| How map groups are scoped (land number is not in the kind-2 request) | R2 | yserbius.md 8 |
| How the land forms game groups and who sets the player count | D1 | redbaron.md 12, golf.md 11 |
| Whether the host stays quiet across a program switch: mechanism built (`Session::begin_program_switch`), trigger open | C14 | `int14h-api.md` 9.3, 13 |
| Does a second joinNet with the same cookie replace the first object (the live Clubhouse entry rejoins the game object)? Assumed yes for kind 129 (`REJOIN_REPLACES_KIND_ASSUMED`) | C14 | `captures.md` section 11 |
| Does a DOS game that inherits the session need a Login first, and may the host refuse one that has none? | C11 (the launcher logs in), D2, D3 | `messages.md` section 9 |
| Is a second Login for an account that is already online refused? | none yet (policy, after C2's registry exists) | `messages.md` 4.2 |
| The reply layout of `UserFindMsg` 49 (`body @4`), so the command stays undecoded | a later presence item | `messages.md` 3.3 |
| Whether the persona set by 40/4 is kept: the `Store` has no update for it, so it lives for the session | a later account item | `messages.md` 3.3 row 40 |
| How a conference is named and what 41/0 returns for a free name | C8 (research first) | `messages.md` 3.3 row 41 |
| What an unreachable host looks like to the legacy PAD and to the transport | C14 | `link-layer.md` 3.2, 5.1; `int14h-transport.md` 5 |
