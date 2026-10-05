use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Commands 26 and 30: the layouts are known, the purpose is not, and no client waits for a reply.
/// 26 is `w 26, w sid, w sid` for a new player object; 30 (`register`) is `b 30, w sid…`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unexplained {
    pub command: UnexplainedCommand,
    pub sids: Vec<Sid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnexplainedCommand {
    PlayerObjectNotice,
    Register,
}

impl Unexplained {
    pub(crate) fn parse(reader: &mut WireReader, command: Command) -> Result<Self, MessageError> {
        let command = match command {
            Command::Register => UnexplainedCommand::Register,
            _ => {
                reader.byte("flags")?;
                UnexplainedCommand::PlayerObjectNotice
            }
        };
        let mut sids = Vec::new();
        while !reader.is_empty() {
            sids.push(Sid(reader.word("sid")?));
        }
        Ok(Unexplained { command, sids })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        match self.command {
            UnexplainedCommand::PlayerObjectNotice => {
                writer.byte(Command::PlayerObjectNotice.byte()).byte(0);
            }
            UnexplainedCommand::Register => {
                writer.byte(Command::Register.byte());
            }
        }
        for sid in &self.sids {
            writer.word(sid.0);
        }
    }
}
