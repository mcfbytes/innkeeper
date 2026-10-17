use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Command 17 from the DOS games: integer properties of a service object. Twinion and Cawdor send
/// sub 2 with props 8 to 11 and wait for nothing; Red Baron sends sub 0 (docs/protocol/messages.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicePropertyRequest {
    pub sub: u8,
    pub service: Sid,
    pub from: Sid,
    pub properties: Vec<u16>,
}

impl ServicePropertyRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let sub = reader.byte("sub-command")?;
        let service = Sid(reader.word("serviceSID")?);
        let from = Sid(reader.word("fromSID")?);
        let mut properties = Vec::new();
        while !reader.is_empty() {
            properties.push(reader.word("property")?);
        }
        Ok(ServicePropertyRequest {
            sub,
            service,
            from,
            properties,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer
            .byte(Command::ServiceProperties.byte())
            .byte(self.sub);
        writer.word(self.service.0).word(self.from.0);
        self.properties.iter().for_each(|&property| {
            writer.word(property);
        });
    }
}
