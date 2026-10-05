use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Command 2: a message the host relays unread; the payload from byte 6 stays raw because LSCI
/// has a word `msgType` and the DOS libraries a byte (docs/protocol/messages.md section 3.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendMessage {
    pub to: Sid,
    pub from: Sid,
    pub payload: Vec<u8>,
}

impl SendMessage {
    /// The LSCI `msgType`: the first word of the payload.
    pub fn msg_type_word(&self) -> Option<u16> {
        leading_word(&self.payload)
    }

    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        Ok(SendMessage {
            to: Sid(reader.word("toSID")?),
            from: Sid(reader.word("fromSID")?),
            payload: reader.rest(),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Send.byte()).byte(0);
        writer.word(self.to.0).word(self.from.0);
        writer.array(&self.payload);
    }
}

pub(crate) fn leading_word(bytes: &[u8]) -> Option<u16> {
    let (word, _) = bytes.split_first_chunk::<2>()?;
    Some(u16::from_le_bytes(*word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_type_is_the_first_payload_word() {
        let mut send = SendMessage {
            to: Sid(0x0103),
            from: Sid(0x0102),
            payload: vec![0x32, 0x00, 0xFF],
        };
        assert_eq!(send.msg_type_word(), Some(50));
        send.payload.truncate(1);
        assert_eq!(send.msg_type_word(), None);
    }
}
