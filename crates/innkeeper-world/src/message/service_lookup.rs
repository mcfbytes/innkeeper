use crate::assumptions::LOCATED_UNREAD_WORD_ASSUMED;
use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{LandType, MessageError, Sid};

/// The sub-command of the DOS libraries' service lookup; LSCI's name check sends 0.
const SERVICE_LOOKUP_SUB: u8 = 2;

/// Command 41 from the client, in the two forms seen (docs/protocol/messages.md section 3.2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjExists {
    /// LSCI: is there an object with this name; the name is kept as the client wrote it.
    Name { to: Sid, from: Sid, name: Vec<u8> },
    /// DOS games: where is the service object of this land type.
    Service { user: Sid, land_type: LandType },
}

impl ObjExists {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        match reader.byte("sub-command")? {
            0 => Ok(ObjExists::Name {
                to: Sid(reader.word("toSID")?),
                from: Sid(reader.word("fromSID")?),
                name: reader.rest(),
            }),
            SERVICE_LOOKUP_SUB => {
                reader.word("toSID")?;
                let user = Sid(reader.word("userSID")?);
                let land_type = LandType(reader.byte("landType")?);
                reader.word("unused")?;
                Ok(ObjExists::Service { user, land_type })
            }
            sub => Err(MessageError::UnsupportedSub {
                command: Command::ObjExists.byte(),
                sub,
            }),
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::ObjExists.byte());
        match self {
            ObjExists::Name { to, from, name } => {
                writer.byte(0).word(to.0).word(from.0).array(name);
            }
            ObjExists::Service { user, land_type } => {
                writer.byte(SERVICE_LOOKUP_SUB).word(0).word(user.0);
                writer.byte(land_type.0).word(0);
            }
        }
    }
}

/// Command 41 from the host: the SID the lookup found, at byte 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectLocated {
    pub to: Sid,
    pub sid: Sid,
}

impl ObjectLocated {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let to = Sid(reader.word("toSID")?);
        reader.word("unused")?;
        Ok(ObjectLocated {
            to,
            sid: Sid(reader.word("sid")?),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::ObjExists.byte()).byte(0);
        writer.word(self.to.0).word(LOCATED_UNREAD_WORD_ASSUMED);
        writer.word(self.sid.0);
    }
}
