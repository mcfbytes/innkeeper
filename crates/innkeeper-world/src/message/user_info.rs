use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::MessageError;

const SET_NAME: u8 = 4;

/// Command 40 sub 4 (`UserInfoMsg`): the persona name the player just chose, after logon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetPersona {
    pub name: String,
}

impl SetPersona {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        match reader.byte("sub-command")? {
            SET_NAME => Ok(SetPersona {
                name: reader.text("name")?,
            }),
            sub => Err(MessageError::UnsupportedSub {
                command: Command::UserInfo.byte(),
                sub,
            }),
        }
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::UserInfo.byte()).byte(SET_NAME);
        writer.text(&self.name);
    }
}

#[cfg(test)]
mod tests {
    use crate::{ClientMessage, MessageError};

    #[test]
    fn only_the_name_sub_command_decodes() {
        assert_eq!(
            ClientMessage::parse(&[40, 3, 0, 0]),
            Err(MessageError::UnsupportedSub {
                command: 40,
                sub: 3
            })
        );
        assert_eq!(
            ClientMessage::parse(&[40, 4, b'x']),
            Err(MessageError::Truncated { field: "name" })
        );
    }
}
