//! Host behaviour the original client does not pin down; each names its section in
//! docs/protocol/messages.md.

use crate::{ClientVersion, LandFlags};

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
