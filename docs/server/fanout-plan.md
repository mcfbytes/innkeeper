# innkeeper fan-out plan (from the INT 14h census)

Independent work items for implementing agents, derived from `docs/protocol/int14h-census/README.md`
and the per-program censuses. Order follows `docs/PLAN.md`: common host services first (Phase 3), then
Yserbius (Phase 5), Twinion (Phase 6), then the deferred titles (golf, Red Baron, Shoppers Advantage).
Crate placement follows `docs/CONVENTIONS.md` section 3.1; nothing here changes that layering.

Reconciled with the built code on 2026-10-05, after the logon wave (commits `65103b9` to `9b526fe`): the
stock client logs on to `innkeeperd` and enters the Clubhouse Waiting Room (`docs/protocol/captures.md`
sections 9 to 11). Statuses below say what exists, and every open scope names the types to extend.

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
| `tsn-link` | framing, CRC, ACK/NAK, resend, one outstanding frame, message queue with `flush` | `Link`, `Message` | no idle query (C4) |
| `pad_thai` | Hayes modem, PAD dialogue, line detection | `Line`, `LineEvent` | none |
| `innkeeper-session` | line then link; send path (`send_message`, `flush`) from `65103b9` | `Session`, `SessionEvent`, `SessionOutput` | hang-up is silent (`HostLink::Offline`, no event); no `is_transmit_idle`; no quiet window (C4) |
| `int14h` | the 17 exports and the transport codec | `call`, `reply`, `envelope` | no listener in `innkeeperd` (C12) |
| `innkeeper-world` `message` | `ClientMessage` (Login 53/59, JoinNet 7, LeaveNet 9, GroupJoin 10 with optional version, `HostInfoRequest` 36/1, 2, 5, 6, `LandOccupancyRequest` 47/1) and `HostMessage` (login Ack, `Nak`, `ObjId`, `GroupJoined`, `HostNumber`, `LandDirectory`, `LandOccupancy`) on `WireReader`/`WireWriter`; `Command` table of 10 bytes | `ObjectKind` (1, 2, 4, 5, 129 only), `JoinNet`, `GroupJoin`, `Nak::login` | commands 2, 11, 12, 13, 14, 28, 41, 48 and the generic Ack; kind 3 is rejected (C1) |
| `innkeeper-world` `account`, `logon` path | `AccountBook::anyone()` and `listed(..)`, `Account`, `Refusal`; the policy lives in `logon::admit` | `AccountBook::admit` | none (C5 done): `logon`, `AccountBook::stored`, command 44 |
| `innkeeper-world` `land` | static `LandCatalog::stock()` (Clubhouse, SierraLand, CasinoLand on host 7): directory, occupancy with `current` 0 | `LandCatalog`, `LandEntry`, `Occupancy` | occupancy is never counted (C6) |
| `innkeeper-world` `player` | `PlayerSession` (`AwaitingLogin` / `LoggedIn`) turning client messages into `Delivery` values | `PlayerSession`, `Delivery` | no routing of `Send` (C3), no locks or property replication (C13) |
| `innkeeper-world` `objects` | `ObjectStore`: host-wide SIDs, reference-counted shared groups, capacity, Nak 10, `disconnect` | `ObjectStore`, `ConnectionId`, `GroupKey` | locks (C13), occupancy counts (C6) |
| `innkeeper-world` `world` | `World` (host number, accounts, lands, clock, store), immutable and shared | `World::stock()`, `World::stored(store)` | none |
| `innkeeperd` | TCP listener, `Host` carrying `ClientMessage` to `PlayerSession` and `HostMessage` back, hex captures, `Exchange` log | `Host`, `ConnectionSettings`, `Switchboard` | no INT 14h listener (C12); no link-loss event, so a drop is seen only when TCP closes (C4) |
| `tools/dosbox` | headless stock-client harness with key scripts `create-persona`, `play`, `clubhouse` | `run_client.py`, `key_scripts.py` | cannot start a DOS game (C11) |

Tests today: 134 across the workspace. The golden logon rows are `crates/innkeeper-world/tests/golden/logon.txt`.

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
| C1 | partial: Login 53/59, JoinNet 7, ObjID 8, LeaveNet 9, GrpJoin 10 (both versions), the Nak, 36 requests and the 47 replies are typed and golden-tested | **Rest of the shared-object codec**: client side `Send` 2 (relay body kept as raw bytes from byte 6, since LSCI has a word msgType and the DOS libraries a byte), `GrpDel` 11, `GrpMem` request 12, `SetInt` 13, `SetStr` 14, multicast 28 (`n` counts word SIDs: `5 + 2n + body`, redbaron.md 5.1), `ObjExists` 41 (raw tail, typed DOS land-type form); host side `Send`, `GroupLeft`, `GroupMembers` (@6..), `ObjectFreed`, `SetInt`/`SetStr` delivery, `ObjectLocated` (cmd 41, SID at w@6), `Notice` 48, a generic `Ack`. Widen `ObjectKind` to the DOS kinds 1..5 and 129 (kind 3 is the Red Baron service, and kind 4 means different things per land type, so group-ness leaves the codec). New files per family under `innkeeper-world::message`, exports in `lib.rs`. 36/x replies stay with C6 and C7 | `innkeeper-world::message` | none | golden bytes per family from the census tables, round trip in both header shapes (same bytes), unknown command → error value | M | Sonnet high |
| C2 | done | **Shared object store**: lift `Player.objects`, `groups` and `next_sid` out of `player.rs` into `objects::ObjectStore`, one per host, with a host-wide SID allocator (reuse `allocate_sid` and its wrap test); `ConnectionId` and `Delivery { to, message }` so a reply can go to another connection; `innkeeperd::switchboard` and `host.rs` carry deliveries through per-connection inboxes. joinNet → `ObjId` in request order (cookie echoed); shared group key `(kind, landType, parameter)` returns one SID with a reference count and the request's capacity; GrpJoin → echo to joiner and members, Nak 10 codes 1 to 3; GrpDel notify; GrpMem reply; ObjFree of a group = release one reference; a game-object re-join with the same cookie replaces the old object (Clubhouse entry, `captures.md` section 11); `disconnect(connection)` and a repeated Login release everything the connection held | `innkeeper-world::objects` (+ `innkeeperd::switchboard`, `host`, `connection`) | C1 | transcripts with two `PlayerSession`s on one store (ordering, echo, Nak codes, last-release), SID uniqueness property, two TCP clients in the `innkeeperd` server test, DOSBox `clubhouse` run unchanged | L | Opus high |
| C3 | open | **Router**: `Send` (2) to an object SID or fan-out to a group SID with the real `fromSid`, body untouched; multicast (28) split per recipient, group SIDs expanded; echo-to-sender policy as one assumption | `innkeeper-world::router` | C2 | transcripts: GOLF CC and RB 0xC9 loopback, Yserbius/Twinion relay of 200–219 unchanged, RB "group sent init!" never triggered | M | Sonnet high |
| C4 | open; `tsn-link` already queues frames, sends one at a time and flushes | **Session lifecycle**: `Session::is_transmit_idle` (nothing queued or unacknowledged), a `SessionEvent` for link loss (PAD disconnect, modem hang-up, TCP close) replacing the silent `HostLink::Offline`, a quiet window across a program switch (nothing sent between LSCI's `TSN(9)` drain and the child's first message). The drop notification itself is C2's `disconnect` called from `innkeeperd::host` | `innkeeper-session::session` (+ `program_switch`) | none for the session crate; C2 for the integration test | transcripts: hang-up emits the event once; no frame leaves in the quiet window; idle only after the last ACK | M | Opus high |
| C5 | done | **Logon policy and accounts**: move `log_in` into `logon::admit`; `Account` carries `user_flags`, `rating` and `LoginStatus` from its record so `LoginAck` is no longer default; `AccountBook::stored` over the C9 `Store` (and an enrolling dev mode); Login land-type check against `LandCatalog` with a named Nak reason; command 44 password change (`ClientMessage::ChangePassword`, Ack 44/1, record updated); `innkeeperd --data-dir` | `innkeeper-world::logon`, `account`, `world` (+ `innkeeperd::config`, `server`) | C9 | golden Ack with non-zero fields; transcripts for wrong then right password, 44, unknown land type; DOSBox `clubhouse` run with a data dir | M | Sonnet medium |
| C6 | partial: 36/5, 36/6 and 47/1 answered from `LandCatalog::stock()` | **Presence**: occupancy `current` counted from the C2 store; waiting-room group (joinNet kind 5, cap from `size`, which is 128 live while the catalog says 64) with the 10/3 version check; 36/1 HOSTADDR reply; UserFindMsg 49; 40/4 sets the persona name (`Account.persona`) | `innkeeper-world::presence`, `land` | C2, C5 | golden replies; DOSBox: hub map lists lands and occupancy | M | Sonnet high |
| C7 | open: `HostInfoRequest::HostTime` is parsed and answered with nothing | **Host time**: `HostMessage::HostTime` (36/2 reply `b 36, b 2, b year-1900, b month0, b mday, b hour, b minute, b second`), `host_time::Clock` with a system and a fixed implementation, a `clock` in `World`, the `answer_host_info` arm | `innkeeper-world::host_time` | none | golden bytes at a fixed clock; RB and GOLF parse it (census 8.2 / 6.2) | S | Haiku medium |
| C8 | open | **Chat and notices**: Send msgType 1 room chat, multicast 28 conferences (msgType 50), ObjExists 41/0 conference name check, operator push Unsolicited 48 | `innkeeper-world::chat` | C3, C6 | transcripts; DOSBox ladder step 2 (two clients chat) | M | Sonnet medium |
| C9 | open | **Persistence boundary**: `Store` trait (accounts, mailboxes, boards; letters and posts are the client's own bytes), `MemoryStore`, `SqliteStore` on `rusqlite` (0.40 is in the cargo cache, `bundled`) | `innkeeper-world::store`, `innkeeperd::sqlite_store` | none | one conformance suite run against both implementations | M | Sonnet high |
| C10 | open | **Mail**: EMMsg 37 (new-mail check, send, read, delete, forward, services, system list), NewBoxHandler 45 mailbox assignment | `innkeeper-world::mail` | C5, C9 | golden replies from `messages.md` 3.3; DOSBox: send and read a letter between two accounts | L | Sonnet high |
| C11 | open | **Program-launch harness**: a test `TSN.PRG` block plus a tiny DOS launcher that connects, logs in, writes a chosen shared block (SetSharedData), names a game with SetNextProgram and exits, so any DOS game can start without driving the LSCI land | `tools/dosbox/` (+ `launcher/`) | none | launcher starts GOLF with a hand-made block and GOLF reaches its joinNet in a capture | M | Opus medium |
| C12 | open | **INT 14h transport listener**: serve `int14h-transport.md` envelopes in `innkeeperd`, mapping Connect/SwitchHost/Login onto the same session and world as the legacy link; GetStatus `AL`/`AH` and GetLineRate per lsci.md section 10 | `innkeeperd::int14h_listener` | C4, C5 | `int14h` golden envelopes driven through a loopback session; parity transcript legacy vs transport | M | Sonnet high |
| C13 | open | **Locks and property replication** for LSCI games: lock 4 / unlock 6 (Nak with holder SID at w@5), setInt 13, setStr 14, invokeMethod 25, getProp 32 / SetMsg 33, GrpGetProp 31; the live Clubhouse already sends 13, 14 and 26 unanswered | `innkeeper-world::objects` (locks), `router` (replication) | C2, C3 | transcripts from `messages.md` 3.2; DOSBox ladder step 3 (a card game) | M | Sonnet high |
| C14 | open; a repeated Login already starts a fresh `Player`, C2 makes it release | **Land switch**: SwitchHost accepted whatever the address, leaveNet of the inherited SID (also a game group SID returned by GOLF/RB, after the players' own GrpDel), re-login, join of the new land | `innkeeper-session::program_switch`, `innkeeper-world::logon` | C4, C5, C6 | DOSBox: walk hub → SierraLand → hub with no orphan objects in the store | M | Opus high |

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
| X1 | Fix `tools/int14h_census.py` (open): drop C++ virtual calls (`mov bx,es:[bx]` before `lcall [bx+N]`), scan to `+44`, fix the far-call scan after a `0x9A`-low-byte segment; coordinate with its owner | `tools/int14h_census.py` | none | reproduces every count in the census README matrix | S | Sonnet medium |

## 3. Dependency order and parallel waves

| Wave | Items (parallel within a wave) | Milestone |
|---|---|---|
| built | logon core of C1, C5 and C6 | ladder step 1 reached by hand: the stock client logs in and enters the Clubhouse Waiting Room |
| 1 (hand out now, wave A) | C1 rest, C7, C9, C11, X1 | full codec, clock, store and a way to start any DOS game |
| 2 (hand out now, wave B) | C2, C5 rest | two connections share objects; accounts persist; step 1 closes with a data dir |
| 3 | C3, C4, C6 rest, C10 | ladder step 2 begins: two clients meet; mail works |
| 4 | C8, C12, C13, C14 | ladder steps 2 to 4: chat, a card game, ScummVM over `int14h` |
| 5 | R1, R2, R3 | RPG services ready |
| 6 | R4, then T1, T2 | Phase 5 and 6 milestones |
| 7 (deferred) | D1, then D2, D3; D4 any time after C5, D5 after D4 | DOS games through the land |

Wave changes since the first plan: C7 and C1's remainder no longer wait on anything; C5 now waits for C9
because its remaining work is the stored account book; C4's session part has no dependency (only its
drop-notification test waits for C2) and may start beside wave B; C6 moved behind C2 for occupancy.

Critical path: C1 → C2 → C3/C4 → C14 → R4 → T2. The next items to hand out, in order:
C1, C9, C7, C11, X1, then C2, C5, then C3, C4, C6, C10.

### 3.1 Wave A: closed scopes

| Id | Owns (files) | Pass criteria | Docs it owns |
|---|---|---|---|
| C1 rest | `crates/innkeeper-world/src/message/` (new family files; `client.rs` and `host.rs` only gain enum variants and match arms), `crates/innkeeper-world/tests/golden/`, `tests/golden.rs` loader; one arm in `PlayerSession::handle` that logs and ignores the new variants | every listed command round-trips byte for byte from census-built golden files; kind 3 parses; kind 6 and an unknown command are errors | `docs/protocol/messages.md` section 3.2 (type names, per-land meaning of joinNet kinds), `innkeeperd.md` section 5 `message` row |
| C7 | `crates/innkeeper-world/src/host_time.rs`, one `HostMessage` variant, `world.rs` clock field, `answer_host_info` arm in `player.rs` | golden 36/2 bytes at a fixed clock; the existing logon transcript is unchanged | `innkeeperd.md` section 5 (`host_time` row, the 36/2 line of the reply table) |
| C9 | `crates/innkeeper-world/src/store.rs` (+ `MemoryStore`, conformance suite behind a feature), `crates/innkeeperd/src/sqlite_store.rs`, workspace and `innkeeperd` `Cargo.toml`, `Cargo.lock` | the conformance suite passes on both stores, including reopen of a file database; gates clean with `--locked` | new `docs/server/store.md`; one row in `innkeeperd.md` section 5 |
| C11 | `tools/dosbox/launcher/`, `tools/dosbox/install_client.py` (game install), `tools/dosbox/run_client.py` (`--launch`), `tools/dosbox/key_scripts.py` | a GOLF capture shows Login, joinNet (kind 1, type `0x66`) and no LSCI run | `docs/dosbox.md` new section |
| X1 | `tools/int14h_census.py` | every count of the census README matrix reproduced, nothing else changes | the tool-caveat bullet of `int14h-census/README.md` |

### 3.2 Wave B: closed scopes

| Id | Owns (files) | Pass criteria | Docs it owns |
|---|---|---|---|
| C2 | `crates/innkeeper-world/src/objects.rs`, the object parts of `player.rs`, `crates/innkeeperd/src/{switchboard,host,connection}.rs`, `innkeeper-world/tests/transcripts.rs` | the transcript and TCP tests in section 2.1, and the DOSBox `clubhouse` script still reaches the Waiting Room with the same replies | new `docs/server/objects.md`, `innkeeperd.md` sections 1 and 5 (player, objects), the group questions in `messages.md` section 11 |
| C5 rest | `crates/innkeeper-world/src/{logon,account,world}.rs`, the `log_in` part of `player.rs`, command 44 in `message`, `innkeeperd` `config.rs` and `server.rs` | the transcripts and goldens in section 2.1; DOSBox `clubhouse` with `--data-dir` | `innkeeperd.md` section 5 (account, logon, `--data-dir`), `messages.md` 3.3 row 44 and 4.2 |

### 3.3 Shared files and merge order

| File | Items | Rule |
|---|---|---|
| `innkeeper-world/src/lib.rs` | C1, C7, C9 (A); C2, C5 (B) | each adds its own `mod` and `pub use` lines |
| `innkeeper-world/src/message/host.rs` | C1, C7 | C1 lands first; C7 adds only the `HostTime` variant, its encode and parse arms |
| `innkeeper-world/src/player.rs` | C1, C7 (A); C2, C5 (B) | C1 one match arm, C7 one `answer_host_info` arm; in B, C5 owns `log_in` and C2 the rest, C5 lands first |
| `innkeeper-world/src/world.rs` | C7 (A); C5 (B) | C7 adds `clock`, C5 adds the store handle |
| `docs/server/innkeeperd.md` | all | each edits only the rows named in its docs column |

## 4. Open questions that block or steer items

| Question | Blocks | Source |
|---|---|---|
| Is a group `Send` echoed to its sender? | C3 (assumption), R4, D2, D3 | every census; `messages.md` 11 |
| What 41/2 looks up and what SID it must return | R1, D3 (any stable SID works for the clients) | yserbius.md, twinion.md, redbaron.md open questions |
| How map groups are scoped (land number is not in the kind-2 request) | R2 | yserbius.md 8 |
| How the land forms game groups and who sets the player count | D1 | redbaron.md 12, golf.md 11 |
| Whether the host stays quiet across a program switch | C4, C14 | `int14h-api.md` 9.3, 13 |
| Does a second joinNet with the same cookie replace the first object (the live Clubhouse entry rejoins the game object)? Assumed yes for kind 129 (`REJOIN_REPLACES_KIND_ASSUMED`) | C14 | `captures.md` section 11 |
| Does a DOS game that inherits the session need a Login first, and may the host refuse one that has none? | C11 (the launcher logs in), D2, D3 | `messages.md` section 9 |
| Is a second Login for an account that is already online refused? | none yet (policy, after C2's registry exists) | `messages.md` 4.2 |
