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
