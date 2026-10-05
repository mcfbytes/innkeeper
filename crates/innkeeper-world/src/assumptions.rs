//! Host behaviour the original client does not pin down; each names its section in
//! docs/protocol/messages.md.

use crate::{ClientVersion, LandFlags, LoginNakReason};

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
