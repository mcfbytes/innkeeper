use std::fmt;

use crate::PadError;

/// The driver keeps at most 14 characters of the `t` field (`MODEM.DRV:0B0F`).
const MAX_ADDRESS_LEN: usize = 14;
/// `hostID = DIRECT` makes the driver skip the PAD dialogue (`MODEM.DRV:0DA9`).
const DIRECT_HOST: &str = "DIRECT";

/// A PAD call address as the client sends it: an X.25 number without its DNIC, or a mnemonic.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostAddress(String);

impl HostAddress {
    pub fn try_new(text: &str) -> Result<Self, PadError> {
        let valid_char = |c: char| c.is_ascii_graphic();
        if text.is_empty() || text.len() > MAX_ADDRESS_LEN || !text.chars().all(valid_char) {
            return Err(PadError::InvalidHostAddress(text.to_owned()));
        }
        Ok(HostAddress(text.to_owned()))
    }

    /// The pseudo-host of a client that dialled straight into framing.
    pub fn direct() -> Self {
        HostAddress(DIRECT_HOST.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_mnemonics_and_numbers_only() {
        assert_eq!(HostAddress::try_new("SIERRA").unwrap().as_str(), "SIERRA");
        assert!(HostAddress::try_new("83420207").is_ok());
        assert!(HostAddress::try_new("").is_err());
        assert!(HostAddress::try_new("TWO WORDS").is_err());
        assert!(HostAddress::try_new("ABCDEFGHIJKLMNO").is_err());
    }
}
