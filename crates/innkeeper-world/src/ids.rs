use std::fmt;

/// The host's 16-bit identifier for a networked object; 0 addresses the client's game object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sid(pub u16);

impl Sid {
    pub const GAME_OBJECT: Sid = Sid(0);
}

/// The client's own handle for an object that asked for a SID; echoed back in `ObjID`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cookie(pub u16);

/// The account number, the `id` key of `LSCI.CFG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(pub u32);

impl AccountId {
    pub fn from_words(low: u16, high: u16) -> Self {
        AccountId(join_words(low, high))
    }

    pub fn words(self) -> (u16, u16) {
        split_words(self.0)
    }
}

/// A host's number in `HOSTADDR` and in the land tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostNumber(pub u8);

/// The program a land runs, the third column of `LAND.CFG` (1 is the Clubhouse).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LandType(pub u8);

/// One running instance of a land type on a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LandNumber(pub u8);

/// A change marker the client stores in `landaddr.tim` or `hostaddr.tim` and sends back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Stamp(pub u32);

impl Stamp {
    pub fn from_words(low: u16, high: u16) -> Self {
        Stamp(join_words(low, high))
    }

    pub fn words(self) -> (u16, u16) {
        split_words(self.0)
    }
}

/// The client keeps 32-bit values as a low and a high word.
fn join_words(low: u16, high: u16) -> u32 {
    u32::from(low) | (u32::from(high) << 16)
}

fn split_words(value: u32) -> (u16, u16) {
    ((value & 0xFFFF) as u16, (value >> 16) as u16)
}

/// The flag byte of a land directory row; the client drops a row whose flags are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LandFlags(pub u8);

/// An interpreter version, major.minor.revision (the title bar's "v2.3" is 2.3.18).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientVersion {
    pub major: u8,
    pub minor: u8,
    pub revision: u8,
}

impl ClientVersion {
    pub const fn new(major: u8, minor: u8, revision: u8) -> Self {
        ClientVersion {
            major,
            minor,
            revision,
        }
    }
}

impl fmt::Display for ClientVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_splits_into_low_and_high_words() {
        let stamp = Stamp::from_words(0x5678, 0x1234);
        assert_eq!(stamp, Stamp(0x1234_5678));
        assert_eq!(stamp.words(), (0x5678, 0x1234));
    }

    #[test]
    fn account_number_splits_like_the_stamp() {
        let account = AccountId::from_words(0x86A1, 1);
        assert_eq!(account, AccountId(100_001));
        assert_eq!(account.words(), (0x86A1, 1));
    }
}
