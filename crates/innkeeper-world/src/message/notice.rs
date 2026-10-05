use crate::assumptions::NOTICE_UNREAD_BYTE_ASSUMED;
use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::MessageError;

/// Command 48 (`Unsolicited`): operator text the client shows, with no request behind it. The
/// NUL after the text is INFERRED from the other text fields (docs/protocol/messages.md section 3.2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
}

impl Notice {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("unused")?;
        Ok(Notice {
            text: reader.text("text")?,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Notice.byte());
        writer.byte(NOTICE_UNREAD_BYTE_ASSUMED).text(&self.text);
    }
}
