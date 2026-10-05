# innkeeper fan-out plan (from the INT 14h census)

Independent work items for implementing agents, derived from `docs/protocol/int14h-census/README.md`
and the per-program censuses. Order follows `docs/PLAN.md`: common host services first (Phase 3), then
Yserbius (Phase 5), Twinion (Phase 6), then the deferred titles (golf, Red Baron, Shoppers Advantage).
Crate placement follows `docs/CONVENTIONS.md` section 3.1; nothing here changes that layering.

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

## 1. Target module map

| Crate | New or extended modules | Owns |
|---|---|---|
| `int14h` | (exists) `call`, `reply`, `envelope` | export types and transport codec; no change expected |
| `innkeeper-session` | `session` (exists), `outbox`, `program_switch` | per-connection state: outbound queue, drain/idle, link-loss events, quiet window at program switch |
| `innkeeper-world` | `message` (exists; extend), `objects`, `router`, `logon`, `presence`, `host_time`, `chat`, `mail`, `bbs`, `store` (trait), `game::rpg`, `game::yserbius`, `game::twinion`, `game::golf`, `game::red_baron`, `game::shopadv` | domain behaviour: object store, routing, host services, per-game service objects |
| `innkeeperd` | `server`, `connection` (exist), `int14h_listener`, `sqlite_store` | sockets, the INT 14h transport listener, persistence, wiring |
| `tools/` | `dosbox/launch_block.py`, a tiny DOS launcher (see C11) | test harness only |

## 2. Work items

### 2.1 Common host services (Phase 3)

| Id | Item and scope | Crate / module | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|
| C1 | **Shared-object message codec**: typed parse/encode for commands 0, 1, 2, 7, 8, 9, 10, 11, 12, 13, 14, 28, 41, 48 in both header shapes (`w cmd` from LSCI, `b cmd, b sub` from the DOS libraries; same bytes). Includes every DOS-game variant from the census (joinNet kinds 1–5, 10/3 with version bytes, GrpMem reply, multicast recipient list) | `innkeeper-world::message` | none | golden bytes from each census message table; round trip; unknown command → error value | M | Sonnet high |
| C2 | **Object store and groups**: SID allocator, joinNet → ObjID (cookie echoed at @2, SID at @6, replies in request order), GrpJoin echo to joiner and members, GrpDel notify, GrpMem reply (`@6..`), ObjFree of a shared group = release one reference, capacities, scoping key `(kind, landType, param)` | `innkeeper-world::objects` | C1 | transcript tests per census rule (ordering, echo, Nak 10 codes 1–6); property test that SIDs are unique per host | M | Opus high |
| C3 | **Router**: `Send` (2) to an object SID or fan-out to a group SID with the real `fromSid` (never the group), body untouched; multicast (28) split per recipient, group SIDs expanded, sender gets a copy when its own SID is listed; echo-to-sender policy as one assumption | `innkeeper-world::router` | C2 | transcripts: GOLF CC and RB 0xC9 loopback, Yserbius/Twinion relay of 200–219 unchanged, RB "group sent init!" never triggered | M | Sonnet high |
| C4 | **Session outbox and lifecycle**: per-connection ordered queue, IsTransmitIdle semantics (idle once all sent data is acknowledged), link loss → Poll status 1 and synthesized GrpDel/ObjFree to peers, quiet window across a program switch (TSNEXEC drops queued messages at child exit) | `innkeeper-session::outbox`, `program_switch` | C2 | transcripts: drop mid-game notifies peers; nothing is sent between LSCI's `TSN(9)` drain and the child's first message | M | Opus high |
| C5 | **Logon**: Login 53 / 59 → AckMsg (`whichCmd` 22, `userFlags`) or Nak within the 70 s window; fresh Login after SwitchHost to a different host; reuse existing `Login`, `LoginAck`, `AccountBook` types | `innkeeper-world::logon`, wiring in `innkeeperd::connection` | C1 | golden reply bytes; DOSBox: stock client passes "LoginTimeout" (integration ladder step 1) | M | Sonnet high |
| C6 | **Presence and land directory**: HostInfo 36/1 (HOSTADDR), 36/5 host number, 36/6 + WaitGrpRequest 47 msgType 1/2, waiting-room groups (joinNet kind 5 with version bytes), UserFindMsg 49, UserInfoMsg 40/4 set name | `innkeeper-world::presence` | C2, C5 | golden replies; DOSBox: hub map lists lands and occupancy | M | Sonnet high |
| C7 | **Host time**: 36/2 reply `b year-1900, b month0, b mday, b hour, b minute, b second` from an injected `Clock` | `innkeeper-world::host_time` | C1 | golden bytes at a fixed clock; RB and GOLF parse it (census 8.2 / 6.2) | S | Haiku medium |
| C8 | **Chat and notices**: Send msgType 1 room chat, multicast 28 conferences (msgType 50), ObjExists 41/0 conference name check, operator push Unsolicited 48 | `innkeeper-world::chat` | C3, C6 | transcripts; DOSBox ladder step 2 (two clients chat) | M | Sonnet medium |
| C9 | **Persistence boundary**: `Store` trait (accounts, mailboxes, boards) with an in-memory test implementation and an SQLite implementation | `innkeeper-world::store`, `innkeeperd::sqlite_store` | none | trait conformance tests run against both implementations | M | Sonnet high |
| C10 | **Mail**: EMMsg 37 (new-mail check, send, read, delete, forward, services, system list), NewBoxHandler 45 mailbox assignment | `innkeeper-world::mail` | C5, C9 | golden replies from `messages.md` 3.3; DOSBox: send and read a letter between two accounts | L | Sonnet high |
| C11 | **Program-launch harness**: a test `TSN.PRG` block plus a tiny DOS launcher that writes a chosen shared block (SetSharedData), names a game with SetNextProgram and exits, so any DOS game can start without driving the LSCI land | `tools/dosbox/` (+ a `.COM` source under `tools/dosbox/launcher/`) | none | launcher starts GOLF with a hand-made block and GOLF reaches its joinNet in a capture | M | Opus medium |
| C12 | **INT 14h transport listener**: serve `int14h-transport.md` envelopes in `innkeeperd`, mapping Connect/SwitchHost/Login onto the same session and world as the legacy link; GetStatus `AL`/`AH` and GetLineRate per lsci.md section 10 | `innkeeperd::int14h_listener` | C4, C5 | `int14h` golden envelopes driven through a loopback session; parity transcript legacy vs transport | M | Sonnet high |
| C13 | **Locks and property replication** for LSCI games: lock 4 / unlock 6 (Nak with holder SID at w@5), setInt 13, setStr 14, invokeMethod 25, getProp 32 / SetMsg 33, GrpGetProp 31 | `innkeeper-world::objects` (locks), `router` (replication) | C2, C3 | transcripts from `messages.md` 3.2; DOSBox ladder step 3 (a card game) | M | Sonnet high |
| C14 | **Land switch**: SwitchHost accepted whatever the address, leaveNet of the inherited SID (also a game group SID returned by GOLF/RB, after the players' own GrpDel), re-login, join of the new land | `innkeeper-session::program_switch`, `innkeeper-world::logon` | C4, C5, C6 | DOSBox: walk hub → SierraLand → hub with no orphan objects in the store | M | Opus high |

### 2.2 Yserbius and Twinion (Phases 5 and 6)

Both use `hostcomm.cpp` + `rpgcomms.cpp`; a shared `game::rpg` module carries the common behaviour and a
profile per title carries the differences (twinion.md section 7).

| Id | Item and scope | Crate / module | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|
| R1 | **RPG service object**: 41/2 (land type `0x8B`) → cmd 41 with a service SID at w@6 within 60 s; 17/2 props 8–11 accepted (Twinion), optional setInt 13 whose third pair value (w@0x10) is the step delay 12..48; 40/4 set name; 48 operator text | `innkeeper-world::game::rpg` | C2, C3 | golden replies per census 4.1/4.2; transcript: the 60 s resend loop ends after one reply | S | Sonnet medium |
| R2 | **RPG group scoping**: kinds 1 player, 5 land group (landType, land number, cap 100), 2 map group (`0x8B`, map number, cap 104 or 80), 4 personal party (cap 4); ObjID matched by request order; 10/3 version gate (1.1.7 / 1.0.22); Nak whichCmd 10 codes 1–6 only, never 0 or >6 | `innkeeper-world::game::rpg` | C2 | transcripts: two players in one land and map get the same group SIDs; full map gives code 2; wrong version gives code 6 | M | Opus high |
| R3 | **RPG bulletin board**: 27/13 directory (17-byte records + names), 27/14 index, read subs 1–5, post 27/0 (`name|text`, 309 bytes), Nak 27 = offline, privileged text offset @0x16 by `userFlags` | `innkeeper-world::bbs` (+ `game::rpg` binding) | C9 | golden bytes for each reply layout; DOSBox: post in DARKSTRT, read it in FATES' board | M | Sonnet high |
| R4 | **Yserbius profile and end-to-end**: TSN.PRG blocks `Yserbius`/`Yserba`, shared block from the hub (`+0`, `+2`, `+6`, `+8`), version 1.1.7, map cap 104; two DOSBox clients see each other on a map, form a party (204/212/211), chat (207), leave cleanly (GrpDel) | `innkeeper-world::game::yserbius`; DOSBox scripts | R1, R2, R3, C4, C14 | DOSBox two-client run; captures diffed against census message tables | M | Opus high |
| T1 | **Twinion profile**: version 1.0.22, land type fixed 8 (block `+8` ignored), map cap 80, 17/2 live, game header `w map @6, b @8, b @9` passed through, msgType 219, `TWINA` case-insensitive; no unsolicited traffic to TWGENN | `innkeeper-world::game::twinion` | R1, R2, R3 | transcripts for each delta row of twinion.md 7 | S | Sonnet medium |
| T2 | **Twinion end-to-end**: gallery → FATES → party → back to gallery and hub | DOSBox scripts | T1, R4 | DOSBox two-client run | S | Sonnet high |

### 2.3 Deferred titles (golf, Red Baron, Shoppers Advantage)

| Id | Item and scope | Crate / module | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|
| D1 | **Game-launch hand-off from LSCI**: how a land forms the game group (GameGroup joinNet kind 2, RedBaronQueue / `global119`), writes `+4` and the `+0x80` parameters, and frees the group when players return. Research first (LSCI scripts, not the DOS binaries), then implement the host side | `innkeeper-world::objects` (group lifetime), `game::golf`/`game::red_baron` launch rules | C2, C13, C14 | DOSBox: two clients launch the game from the land; group survives the program switch | M | Opus high |
| D2 | **Golf**: joinNet kind 1 type `0x66`, GrpJoin/GrpMem within 120 s, GrpDel broadcast on leave or drop, multicast CC loopback, Unsolicited 48; nothing persisted | `innkeeper-world::game::golf` | C3, C4, D1 (or C11 for standalone tests) | transcripts of golf.md 7.2 handshake; DOSBox two-player round via C11 | S | Sonnet medium |
| D3 | **Red Baron service object**: 41/2 type 101 → service SID; 17/0 prop 9 → setInt `0x0201`, prop `0xF` → value 5..30; joinNet kind 3; absorb 50 (0x32) status; 36/2 from C7; GrpJoin echo, GrpMem, multicast 0xC9 loopback | `innkeeper-world::game::red_baron` | C3, C7, D1 (or C11) | golden replies; DOSBox two-plane session via C11 | S | Sonnet medium |
| D4 | **Shoppers Advantage, closed**: answer LSCI's 54/27 with 54/20 and a NUL-terminated "closed" text at byte 15 | `innkeeper-world::game::shopadv` | C1, C5 | golden reply; DOSBox: mall entry shows the text | S | Haiku medium |
| D5 | **Shoppers Advantage, joke edition**: 54/27 → 54/19; JOIN 54/25 → 54/17 token; TEXT 54/24 answered within 636 ticks with 54/16 kind 1 text (CR LF and LF terminators, w@2 < 0x1000, full-length bodies); `/TO MAIN`, `/TO STORE`, `/EXIT`; EXIT 54/26 → 54/19 kind `0x1A`; replies are plain Amazon search URLs (no scraping, no API) | `innkeeper-world::game::shopadv` | D4, C4 | golden 54/x bytes; DOSBox: search, then Exit returns to the land | M | Sonnet medium |

### 2.4 Tooling (any time)

| Id | Item and scope | Where | Depends on | Test strategy | Size | Tier |
|---|---|---|---|---|---|---|
| X1 | Fix `tools/int14h_census.py`: drop C++ virtual calls (`mov bx,es:[bx]` before `lcall [bx+N]`), scan to `+44`, fix the far-call scan after a `0x9A`-low-byte segment; coordinate with its owner | `tools/int14h_census.py` | none | reproduces every count in the census README matrix | S | Sonnet medium |

## 3. Dependency order and parallel waves

| Wave | Items (parallel within a wave) | Milestone |
|---|---|---|
| 1 | C1, C7, C9, C11, X1 | codecs, clock, store and harness exist |
| 2 | C2, C5 | ladder step 1: a DOSBox client logs in |
| 3 | C3, C4, C6, C10 | ladder step 2: two clients chat; mail works |
| 4 | C8, C12, C13, C14 | ladder steps 3 and 4: a card game; ScummVM over `int14h` |
| 5 | R1, R2, R3 | RPG services ready |
| 6 | R4, then T1, T2 | Phase 5 and 6 milestones |
| 7 (deferred) | D1, then D2, D3; D4 any time after C5, D5 after D4 | DOS games through the land |

Critical path: C1 → C2 → C3/C4 → C14 → R4 → T2. The first ten items to hand out, in order:
C1, C2, C5, C3, C4, C7, C9, C6, C11, C10.

## 4. Open questions that block or steer items

| Question | Blocks | Source |
|---|---|---|
| Is a group `Send` echoed to its sender? | C3 (assumption), R4, D2, D3 | every census; `messages.md` 11 |
| What 41/2 looks up and what SID it must return | R1, D3 (any stable SID works for the clients) | yserbius.md, twinion.md, redbaron.md open questions |
| How map groups are scoped (land number is not in the kind-2 request) | R2 | yserbius.md 8 |
| How the land forms game groups and who sets the player count | D1 | redbaron.md 12, golf.md 11 |
| Whether the host stays quiet across a program switch | C4, C14 | `int14h-api.md` 9.3, 13 |
