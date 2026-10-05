use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// One integer property: the byte offset of the property in the object and its new value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntProperty {
    pub offset: u16,
    pub value: u16,
}

/// Command 13: set integer properties on the replicas of `target`; the host relays it the same way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetInt {
    pub target: Sid,
    pub from: Sid,
    pub properties: Vec<IntProperty>,
}

impl SetInt {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let target = Sid(reader.word("targetSID")?);
        let from = Sid(reader.word("fromSID")?);
        let mut properties = Vec::new();
        while !reader.is_empty() {
            properties.push(IntProperty {
                offset: reader.word("propOffset")?,
                value: reader.word("value")?,
            });
        }
        Ok(SetInt {
            target,
            from,
            properties,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::SetInt.byte()).byte(0);
        writer.word(self.target.0).word(self.from.0);
        for property in &self.properties {
            writer.word(property.offset).word(property.value);
        }
    }
}

/// Command 14: set a string or array property; the value is kept verbatim to the end of the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetStr {
    pub target: Sid,
    pub from: Sid,
    pub offset: u16,
    pub value: Vec<u8>,
}

impl SetStr {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        Ok(SetStr {
            target: Sid(reader.word("targetSID")?),
            from: Sid(reader.word("fromSID")?),
            offset: reader.word("propOffset")?,
            value: reader.rest(),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::SetStr.byte()).byte(0);
        writer.word(self.target.0).word(self.from.0);
        writer.word(self.offset).array(&self.value);
    }
}
