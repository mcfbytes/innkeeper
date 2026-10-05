//! Host behaviour the original client does not pin down; each names its section in
//! docs/protocol/messages.md.

use crate::{ClientVersion, LandFlags, LoginNakReason, MailboxNumber, ObjectKind};

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
/// a dev machine needs no sign-up (command 55 is not answered). The original host's policy is unknown.
pub(crate) const STORED_BOOK_ENROLS_ASSUMED: bool = true;
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
