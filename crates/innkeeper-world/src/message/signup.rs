//! Online signup, command 55. Layout: docs/protocol/messages.md section 3.3.

use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::MessageError;

const SIGNUP_SUB: u8 = 1;
/// The byte the script pushes after the cookie (`hub/script.012`, format `bbwba`).
const MARKER: u8 = 1;

/// Command 55/1: a demographic form. The form is not a password and not a persona.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signup {
    /// `MakeConnection`'s own object handle. The Ack is addressed to SID 0, not to this.
    pub cookie: u16,
    pub form: Vec<u8>,
}

impl Signup {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        expect_marker(reader, "sub-command")?;
        let cookie = reader.word("cookie")?;
        expect_marker(reader, "marker")?;
        Ok(Signup {
            cookie,
            form: reader.rest(),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Signup.byte()).byte(SIGNUP_SUB);
        writer.word(self.cookie).byte(MARKER).array(&self.form);
    }
}

fn expect_marker(reader: &mut WireReader, field: &'static str) -> Result<(), MessageError> {
    match reader.byte(field)? {
        MARKER => Ok(()),
        value => Err(MessageError::UnknownValue {
            field,
            value: u16::from(value),
        }),
    }
}
