use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Command 50 from the DOS games: `b 50, b state, w 0, w user`. Red Baron sends state 0 when its
/// game starts and 1 when it ends or fails, GOLF the same shape (`32 b w w`); no client reads a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayStatus {
    pub playing: bool,
    pub user: Sid,
}

impl PlayStatus {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let state = reader.byte("state")?;
        reader.word("unused")?;
        let user = Sid(reader.word("userSID")?);
        match state {
            0 | 1 => Ok(PlayStatus {
                playing: state == 0,
                user,
            }),
            value => Err(MessageError::UnknownValue {
                field: "state",
                value: u16::from(value),
            }),
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer
            .byte(Command::PlayStatus.byte())
            .byte(u8::from(!self.playing));
        writer.word(0).word(self.user.0);
    }
}
