use crate::Int14hError;

/// Printable ASCII of at most `MAX` bytes, the C strings the API passes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BoundedText<const MAX: usize>(String);

impl<const MAX: usize> BoundedText<MAX> {
    const FITS_STR8: () = assert!(
        MAX <= u8::MAX as usize,
        "text travels with a one-byte length"
    );

    pub fn try_new(text: impl Into<String>) -> Result<Self, Int14hError> {
        let () = Self::FITS_STR8;
        let text = text.into();
        if text.len() > MAX {
            return Err(Int14hError::TooLong {
                field: "text",
                len: text.len(),
                max: MAX,
            });
        }
        if !text
            .bytes()
            .all(|byte| byte == b' ' || byte.is_ascii_graphic())
        {
            return Err(Int14hError::NotAscii { field: "text" });
        }
        Ok(BoundedText(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque bytes of at most `MAX` bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BoundedBytes<const MAX: usize>(Vec<u8>);

impl<const MAX: usize> BoundedBytes<MAX> {
    const FITS_BYTES16: () = assert!(
        MAX <= u16::MAX as usize,
        "bytes travel with a 16-bit length"
    );

    pub fn try_new(bytes: impl Into<Vec<u8>>) -> Result<Self, Int14hError> {
        let () = Self::FITS_BYTES16;
        let bytes = bytes.into();
        if bytes.len() > MAX {
            return Err(Int14hError::TooLong {
                field: "bytes",
                len: bytes.len(),
                max: MAX,
            });
        }
        Ok(BoundedBytes(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Connect's argument; driver function 3 copies at most 127 bytes (`MODEM.DRV:0A6B`).
pub type DialString = BoundedText<127>;
/// SwitchHost's argument, parsed by the driver like the dial string's `t` field.
pub type SwitchAddress = BoundedText<127>;
/// A `TSN.PRG` block name.
pub type ProgramName = BoundedText<255>;
/// Free-form identification in HELLO and WELCOME.
pub type PeerName = BoundedText<255>;
/// SetSharedData keeps at most 256 bytes (`031D:013A`).
pub type SharedData = BoundedBytes<256>;
/// One message body; the link-layer length prefix tops out at 16 bits.
pub type MessageBody = BoundedBytes<65535>;

/// Client clock ticks, 60 Hz in LSCITV and 18.2 Hz in the DOS games.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticks(pub u16);

/// Driver function 9's state word, initialised to 1200 and updated from `CONNECT <rate>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineRate(pub u16);

/// GetStatus: `AL` is driver function 0 (3 for MODEM.DRV), `AH` the connection byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecStatus {
    pub driver_version: u8,
    pub connected: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_enforced() {
        assert!(DialString::try_new("x".repeat(127)).is_ok());
        assert_eq!(
            DialString::try_new("x".repeat(128)),
            Err(Int14hError::TooLong {
                field: "text",
                len: 128,
                max: 127
            })
        );
        assert!(ProgramName::try_new("SLand\n").is_err());
        assert!(SharedData::try_new(vec![0; 257]).is_err());
    }
}
