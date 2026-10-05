use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// `AckMsg` for any request but a Login: which command and sub-command it answers, then the
/// reply's own bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ack {
    pub to: Sid,
    pub which_cmd: u8,
    pub which_sub: u8,
    pub tail: Vec<u8>,
}

impl Ack {
    pub(crate) fn parse_after(
        reader: &mut WireReader,
        to: Sid,
        which_cmd: u8,
    ) -> Result<Self, MessageError> {
        Ok(Ack {
            to,
            which_cmd,
            which_sub: reader.byte("whichSub")?,
            tail: reader.rest(),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Ack.byte()).byte(0).word(self.to.0);
        writer.byte(self.which_cmd).byte(self.which_sub);
        writer.array(&self.tail);
    }
}
