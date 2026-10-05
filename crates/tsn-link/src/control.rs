use std::fmt;

use crate::LinkError;

const DATA_TYPE: u8 = 0x00;
const NAK_TYPE: u8 = 0x80;
const ACK_TYPE: u8 = 0x90;
const TYPE_MASK: u8 = 0xF0;
const SEQ_MASK: u8 = 0x0F;

/// Link sequence number: counts 0..=7 and wraps, reset to 0 by every Connect and SwitchHost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Seq(u8);

impl Seq {
    pub const ZERO: Seq = Seq(0);
    const MODULUS: u8 = 8;

    #[must_use]
    pub const fn next(self) -> Seq {
        Seq((self.0 + 1) % Self::MODULUS)
    }

    pub const fn value(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Seq {
    type Error = LinkError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value < Self::MODULUS {
            Ok(Seq(value))
        } else {
            Err(LinkError::InvalidSeq(value))
        }
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The `ctrl` byte of a frame: type in the high nibble, sequence in the low nibble.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Data(Seq),
    Nak(Seq),
    Ack(Seq),
}

impl Control {
    pub const fn to_byte(self) -> u8 {
        match self {
            Control::Data(seq) => DATA_TYPE | seq.0,
            Control::Nak(seq) => NAK_TYPE | seq.0,
            Control::Ack(seq) => ACK_TYPE | seq.0,
        }
    }

    pub const fn seq(self) -> Seq {
        match self {
            Control::Data(seq) | Control::Nak(seq) | Control::Ack(seq) => seq,
        }
    }

    /// The sequence a NAK would name for a damaged frame whose `ctrl` byte was `byte`.
    pub(crate) fn nak_for(byte: u8) -> Option<Control> {
        Seq::try_from(byte & SEQ_MASK).ok().map(Control::Nak)
    }
}

impl TryFrom<u8> for Control {
    type Error = LinkError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        let seq = Seq::try_from(byte & SEQ_MASK).map_err(|_| LinkError::UnknownControl(byte))?;
        match byte & TYPE_MASK {
            DATA_TYPE => Ok(Control::Data(seq)),
            NAK_TYPE => Ok(Control::Nak(seq)),
            ACK_TYPE => Ok(Control::Ack(seq)),
            _ => Err(LinkError::UnknownControl(byte)),
        }
    }
}

impl fmt::Display for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Control::Data(seq) => write!(f, "DATA {seq}"),
            Control::Nak(seq) => write!(f, "NAK {seq}"),
            Control::Ack(seq) => write!(f, "ACK {seq}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_wraps_after_seven() {
        let seven = Seq::try_from(7).unwrap();
        assert_eq!(seven.next(), Seq::ZERO);
        assert_eq!(Seq::try_from(8), Err(LinkError::InvalidSeq(8)));
    }

    #[test]
    fn every_valid_control_byte_round_trips() {
        for byte in 0..=u8::MAX {
            if let Ok(control) = Control::try_from(byte) {
                assert_eq!(control.to_byte(), byte);
            }
        }
        assert_eq!(Control::try_from(0x91), Ok(Control::Ack(Seq(1))));
        assert_eq!(
            Control::try_from(0xA0),
            Err(LinkError::UnknownControl(0xA0))
        );
        assert_eq!(
            Control::try_from(0x08),
            Err(LinkError::UnknownControl(0x08))
        );
    }
}
