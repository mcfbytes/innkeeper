//! Host behaviour the original client does not pin down; each names its section in
//! docs/protocol/messages.md.

use crate::{ClientVersion, LandFlags, LoginNakReason, MailboxNumber, Money, ObjectKind, Sid};

/// The version range of a land directory row (section 6); a range this wide admits every client.
pub(crate) const ANY_VERSION_MIN_ASSUMED: ClientVersion = ClientVersion::new(0, 0, 0);
pub(crate) const ANY_VERSION_MAX_ASSUMED: ClientVersion = ClientVersion::new(255, 255, 255);
/// Two bytes after the land number that no client script reads (section 6).
pub(crate) const DIRECTORY_UNREAD_BYTES_ASSUMED: [u8; 2] = [0, 0];
/// Land flags: the client drops a row whose flags are 0, and 1 is the default of its land object;
/// bits 0x0C select the CasinoLand disclaimer (section 6).
pub(crate) const OPEN_LAND_FLAGS_ASSUMED: LandFlags = LandFlags(0x01);
/// The word at byte 4 of a member list: the DOS clients read the list from byte 6 (section 3.2.1).
pub(crate) const MEMBER_LIST_UNREAD_WORD_ASSUMED: u16 = 0;
/// The word at byte 4 of a located-object reply: only the SID at byte 6 is read (section 3.2.1).
pub(crate) const LOCATED_UNREAD_WORD_ASSUMED: u16 = 0;
/// A conference name check is keyed by the raw tail `GotoConference::init` sends: the land number's
/// low byte, 15 and 0 (`hub/script.120`). A 41/0 whose tail was a title string would settle it.
pub(crate) const CONFERENCE_KEY_IS_THE_RAW_TAIL_ASSUMED: bool = true;
/// Byte 1 of a notice: both DOS handlers show the text from byte 2 and never read it (section 3.2.1).
pub(crate) const NOTICE_UNREAD_BYTE_ASSUMED: u8 = 0;
/// Host time in HostInfoRequest reply (messages.md 3.3): sent as UTC, matching SystemTime::now().
/// The original host's time zone is unknown and has no observable effect on the DOS clients.
/// This constant exists to document the assumption; its value is true and not used in code.
#[allow(dead_code)]
pub(crate) const HOST_TIME_ZONE_UTC_ASSUMED: bool = true;
/// A Login for a land type the catalog lacks gets this Nak reason: only 9 is special to the client,
/// every other value shows error 100 + reason (section 4.2).
pub(crate) const UNLISTED_LAND_NAK_ASSUMED: LoginNakReason = LoginNakReason::UnlistedLand;
/// A host with a data directory creates an unknown account from the password the Login presents, so
/// a dev machine needs no sign-up. The original host's policy is unknown.
pub(crate) const STORED_BOOK_ENROLS_ASSUMED: bool = true;
/// The first account number a signup issues when the store has none higher. Six-digit numbers are
/// what the stock client keeps in `LSCI.CFG`. The original host's allocator is unknown.
pub(crate) const SIGNUP_FIRST_ACCOUNT_ASSUMED: u32 = 100_001;
/// Placeholder password stored at signup. The form has no password (`hub/script.012`); the first
/// Login replaces these bytes (`SIGNUP_ADOPTS_PRESENTED_PASSWORD_ASSUMED`). A form field that
/// carried the encoded password would settle it.
pub(crate) const SIGNUP_PASSWORD_ASSUMED: [u8; 10] = *b"SIGNUP\0\0\0\0";
/// The first Login after signup stores the password that Login presented. The signup form does not
/// carry one (`hub/script.012`, and `hub/script.004` global91 is a Prodigy id file). A form field
/// that did would settle it.
pub(crate) const SIGNUP_ADOPTS_PRESENTED_PASSWORD_ASSUMED: bool = true;
/// Signup keeps an account and nothing from the form. `AccountRecord` has no address or card
/// columns (`store.md`). A column for those fields would settle it the other way.
pub(crate) const SIGNUP_KEEPS_ONLY_THE_ACCOUNT_ASSUMED: bool = true;
/// An explicit signup creates an account even when a Login will not enrol an unknown number.
/// Enrolment is the silent-login policy. The original host's split between the two is unknown.
pub(crate) const SIGNUP_CREATES_WHEN_ENROLMENT_IS_CLOSED_ASSUMED: bool = true;
/// joinNet kinds that take members on every land; the codec leaves this to the host (section 3.2.1).
pub(crate) const GROUP_KINDS_ASSUMED: [ObjectKind; 3] = [
    ObjectKind::Group,
    ObjectKind::PrivateGroup,
    ObjectKind::LandGroup,
];
/// A group parameter that is negative as a word (-3 for a personal party) asks for a group of the
/// requester's own; other groups are shared by kind, land type and parameter (section 11).
pub(crate) const PRIVATE_PARAMETER_MIN_ASSUMED: u16 = 0x8000;
/// A GrpJoin reaches the other members as well as the joiner: the waiting room's members fetch the
/// newcomer's name when it arrives (section 11).
pub(crate) const GROUP_JOIN_TELLS_MEMBERS_ASSUMED: bool = true;
/// A joinNet of this kind with a cookie the connection already holds replaces that object: the
/// Clubhouse entry re-joins the game object with no leaveNet (section 5.4, section 11).
pub(crate) const REJOIN_REPLACES_KIND_ASSUMED: ObjectKind = ObjectKind::GameObject;
/// A `Send` also reaches the sender's own connection when it holds the target: the GOLF `CC` and
/// Red Baron `0xC9` handlers count their own copy (section 11).
pub(crate) const GROUP_SEND_ECHOES_SENDER_ASSUMED: bool = true;
/// A 40/4 persona name gets no reply: the stock client sends it at logon and in the Clubhouse and
/// carries on without one (section 3.3). The name lives for the session; the store is not written.
#[allow(dead_code)]
pub(crate) const PERSONA_SET_UNANSWERED_ASSUMED: bool = true;
/// The bankroll a 40/3 read reports for an account the store has no balance for: 1,000,000, the value
/// `LL/script.120`'s waiting room seeds a CasinoLand visitor with. The client keeps its own copy when
/// no reply arrives, and its seed is that same value for a player who may enter the land, 0 otherwise.
pub(crate) const MONEY_STARTING_BALANCE_ASSUMED: Money = Money(1_000_000);
/// Byte 6 of the Ack for a delivered letter: the client reads the box words from byte 7 (section 3.3).
pub(crate) const ACK_UNREAD_BYTE_ASSUMED: u8 = 0;
/// Byte 0 of a listing row: only bit 0x80 (a system letter) changes what the client does (section 3.3).
pub(crate) const LISTED_LETTER_STATUS_ASSUMED: u8 = 0;
/// The two words after the header of the empty system list; the client reads from byte 8 (section 3.3).
pub(crate) const SYSTEM_LIST_UNREAD_WORDS_ASSUMED: [u16; 2] = [0, 0];
/// The box words of a refused mailbox assignment; the client reads them only with status 1 (section 3.3).
pub(crate) const REFUSED_MAILBOX_ASSUMED: MailboxNumber = MailboxNumber(0);
/// Status of a refused assignment when the account is not in the store, as on the open book (section 3.3).
pub(crate) const NO_ACCOUNT_MAILBOX_STATUS_ASSUMED: u8 = 2;
/// Status of a refused assignment when the store fails: "busy creating a mail box", retried at the next logon.
pub(crate) const STORE_FAULT_MAILBOX_STATUS_ASSUMED: u8 = 4;
/// Mail requests name a box, and only the requester's own box may be checked, listed, read or emptied.
pub(crate) const OTHER_BOXES_ARE_PRIVATE_ASSUMED: bool = true;
/// The word at byte 4 of a `SetMsg`: the client reads the property records from byte 6 (section 3.2).
pub(crate) const PROPERTIES_UNREAD_WORD_ASSUMED: u16 = 0;
/// An unlock gets no reply: the invitation scripts send it and move on without waiting (section 3.2).
pub(crate) const UNLOCK_IS_ACKNOWLEDGED_ASSUMED: bool = false;
/// A property update or remote call reaches every connection that shares a group with the object, not
/// only its holders: a waiting room builds a replica of each member it is told about (section 3.2).
pub(crate) const REPLICAS_INCLUDE_GROUP_FELLOWS_ASSUMED: bool = true;
/// A member-properties cell the mirror lacks: the client's table has a value for every member and
/// column, so the host sends 0 or empty text (section 3.2).
pub(crate) const UNSET_INT_PROPERTY_ASSUMED: u16 = 0;
/// The 8 bytes that end each 17-byte board record; the client reads the first 9 (yserbius.md 4.4).
pub(crate) const BOARD_UNREAD_BYTES_ASSUMED: [u8; 8] = [0; 8];
/// The 4 bytes a privileged reader skips before a message's text; what they held is unknown (yserbius.md 4.4).
pub(crate) const PRIVILEGED_UNREAD_BYTES_ASSUMED: [u8; 4] = [0; 4];
/// The SID a 41/2 lookup of the RPG land type returns: no client reads it back, and SIDs below the
/// first allocated one are free for well-known objects (yserbius.md 4).
pub(crate) const RPG_SERVICE_SID_ASSUMED: Sid = Sid(0x008B);
/// A map group (kind 2, land type 0x8B) carries only the map number, so it is shared within the
/// requester's land place: Yserbius and Twinion maps, and two places of one land, stay apart.
pub(crate) const MAP_GROUPS_PER_PLACE_ASSUMED: bool = true;
/// The land GrpJoin (10/3) of an RPG title is refused with code 6 unless it carries that title's
/// own client version (1.1.7, 1.0.22); the original host's range is unknown.
pub(crate) const RPG_LAND_ADMITS_ONLY_ITS_VERSION_ASSUMED: bool = true;
/// A `Send` to an RPG object or group skips the sender: DARKSTRT prints its own chat line when it
/// sends it, so an echo would show it twice (yserbius.md 5).
pub(crate) const RPG_SENDS_ECHO_SENDER_ASSUMED: bool = false;
/// An account whose stored rating is 0 logs in with the rights to every land this host runs: the
/// hub refuses a land whose bit (`landType - 1`) is clear, and reads 0 as 1, the Clubhouse alone.
pub(crate) const UNSET_RATING_GRANTS_EVERY_LAND_ASSUMED: bool = true;
/// The SID a 41/2 lookup of Red Baron's land type (101) returns, and the object its 17/0 reads and
/// the setInt answers name: the client stores it and compares it, nothing else reads it. Below the
/// first allocated SID, like the RPG service (redbaron.md sections 5.2 and 10).
pub(crate) const RED_BARON_SERVICE_SID_ASSUMED: Sid = Sid(0x0065);
/// Word 2 of a Shoppers Advantage reply: an index into Shop's 4096-slot object table. Slot 5 is
/// empty (`shopadv.md` section 7.2). A capture of a live host's word 2, or a registration into
/// another slot, would settle it.
pub(crate) const MALL_HEADER_SLOT_ASSUMED: u16 = 5;
/// Kind byte of the `54/19` that starts Shop. LSCI ignores it; Shop acts only on `0x1A` and `0x1B`
/// (`shopadv.md` section 7.2). A capture of that byte on a mall open would settle it.
pub(crate) const MALL_OPEN_KIND_ASSUMED: u8 = 0;
/// The one line Shop appends. The original mall was a store, so no capture has this sentence; a
/// host that wanted a different notice would replace it (`shopadv.md` section 10).
pub(crate) const MALL_CLOSED_TEXT_ASSUMED: &str = "The Shoppers Advantage mall is closed.\n";
/// Bytes 13 and 14 of a `54/20`. The text the client shows starts at byte 15 and nothing reads the
/// two bytes before it (`shopadv.md` section 7.2). A capture of those bytes would settle them.
pub(crate) const MALL_NAK_UNREAD_BYTES_ASSUMED: [u8; 2] = [0, 0];
/// The mall gate (`54/27`) starts Shop. Shop shows a `54/16` kind 1 with no catalog (`shopadv.md`
/// sections 9 and 11), so a closed mall can be that notice. A client that ignored an unsolicited
/// `54/16` would settle it the other way: answer the gate with `54/20` and never start Shop.
pub(crate) const MALL_LAUNCHES_SHOP_ASSUMED: bool = true;
/// The closed notice follows `54/17` without waiting for a typed line. Shop's idle state displays
/// a `54/16` whenever the host has one (`shopadv.md` section 9 step 3). A capture whose first
/// screen arrived only after a `54/24` would settle it the other way.
pub(crate) const MALL_NOTICE_FOLLOWS_JOIN_ASSUMED: bool = true;
/// Property 0xF of Red Baron's service: how long a game lasts, in minutes. The client accepts 5 to 30
/// and scales it to timer ticks (redbaron.md section 6); 10 is a host choice.
pub(crate) const RED_BARON_GAME_MINUTES_ASSUMED: u16 = 10;
