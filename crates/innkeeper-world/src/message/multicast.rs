use crate::message::send::leading_word;
use crate::message::wire::{fitting_count, row_count, WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Command 28: one body for a list of SIDs; `n` counts words, so the message is `5 + 2n + body`.
/// Each recipient presumably receives a `Send` (docs/protocol/messages.md section 3.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Multicast {
    pub from: Sid,
    pub recipients: Vec<Sid>,
    pub body: Vec<u8>,
}

impl Multicast {
    /// The LSCI `msgType`: the first word of the body.
    pub fn msg_type_word(&self) -> Option<u16> {
        leading_word(&self.body)
    }

    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let from = Sid(reader.word("fromSID")?);
        let count = reader.word("n")?;
        let recipients = (0..count).map(|_| reader.word("recipient").map(Sid));
        Ok(Multicast {
            from,
            recipients: recipients.collect::<Result<_, _>>()?,
            body: reader.rest(),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        let recipients = fitting_count(&self.recipients);
        writer.byte(Command::Multicast.byte()).word(self.from.0);
        writer.word(row_count(recipients));
        for sid in recipients {
            writer.word(sid.0);
        }
        writer.array(&self.body);
    }
}
