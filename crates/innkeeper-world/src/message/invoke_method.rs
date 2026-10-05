use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Command 25 in both directions, the same bytes: call `selector` with word `args` on every replica of
/// `target`. The sender has already called it on its own copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvokeMethod {
    pub target: Sid,
    pub selector: u16,
    pub args: Vec<u16>,
}

impl InvokeMethod {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let target = Sid(reader.word("sid")?);
        let selector = reader.word("selector")?;
        let mut args = Vec::new();
        while !reader.is_empty() {
            args.push(reader.word("argument")?);
        }
        Ok(InvokeMethod {
            target,
            selector,
            args,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::InvokeMethod.byte()).byte(0);
        writer.word(self.target.0).word(self.selector);
        for arg in &self.args {
            writer.word(*arg);
        }
    }
}
