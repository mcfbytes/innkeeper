use crate::wire::{WireReader, WireWriter};
use crate::{Call, Export, Int14hError, PeerName, Reply};

/// Transport revision announced in HELLO and WELCOME.
pub const PROTOCOL_VERSION: u8 = 1;
const HELLO_MAGIC: [u8; 4] = *b"I14H";
const LENGTH_PREFIX_LEN: usize = 4;
/// Largest envelope after the length prefix: a maximal message with room for its headers.
const MAX_ENVELOPE_LEN: usize = 0x1_0000 + 0x100;

const KIND_HELLO: u8 = 0x01;
const KIND_WELCOME: u8 = 0x02;
const KIND_CALL: u8 = 0x10;
const KIND_REPLY: u8 = 0x11;

/// Chosen by the client for each CALL and echoed by the matching REPLY.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tag(pub u16);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub version: u8,
    pub client: PeerName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub version: u8,
    pub server: PeerName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Hello(Hello),
    Welcome(Welcome),
    Call(Call),
    Reply(Reply),
}

/// One transport unit: a tag and a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub tag: Tag,
    pub body: Body,
}

/// Encodes an envelope with its 32-bit length prefix, ready for a byte stream.
pub fn encode_envelope(envelope: &Envelope) -> Vec<u8> {
    let mut writer = WireWriter::default();
    match &envelope.body {
        Body::Hello(hello) => {
            writer.u8(KIND_HELLO);
            writer.u16(envelope.tag.0);
            HELLO_MAGIC.iter().for_each(|&byte| writer.u8(byte));
            writer.u8(hello.version);
            writer.text(&hello.client);
        }
        Body::Welcome(welcome) => {
            writer.u8(KIND_WELCOME);
            writer.u16(envelope.tag.0);
            writer.u8(welcome.version);
            writer.text(&welcome.server);
        }
        Body::Call(call) => {
            writer.u8(KIND_CALL);
            writer.u16(envelope.tag.0);
            writer.u8(call.export().index());
            call.encode_args(&mut writer);
        }
        Body::Reply(reply) => {
            writer.u8(KIND_REPLY);
            writer.u16(envelope.tag.0);
            writer.u8(reply.export().index());
            reply.encode_result(&mut writer);
        }
    }
    let unit = writer.finish();
    let mut framed = Vec::with_capacity(LENGTH_PREFIX_LEN + unit.len());
    framed.extend((unit.len() as u32).to_le_bytes());
    framed.extend(unit);
    framed
}

/// Parses one envelope without its length prefix, as a WebSocket message would carry it.
pub fn parse_envelope(unit: &[u8]) -> Result<Envelope, Int14hError> {
    let mut reader = WireReader::new(unit);
    let kind = reader.u8()?;
    let tag = Tag(reader.u16()?);
    let body = match kind {
        KIND_HELLO => {
            let magic = [reader.u8()?, reader.u8()?, reader.u8()?, reader.u8()?];
            if magic != HELLO_MAGIC {
                return Err(Int14hError::BadMagic);
            }
            Body::Hello(Hello {
                version: reader.u8()?,
                client: reader.text()?,
            })
        }
        KIND_WELCOME => Body::Welcome(Welcome {
            version: reader.u8()?,
            server: reader.text()?,
        }),
        KIND_CALL => {
            let export = Export::try_from(reader.u8()?)?;
            Body::Call(Call::parse_args(export, &mut reader)?)
        }
        KIND_REPLY => {
            let export = Export::try_from(reader.u8()?)?;
            Body::Reply(Reply::parse_result(export, &mut reader)?)
        }
        other => return Err(Int14hError::UnknownKind(other)),
    };
    reader.finish()?;
    Ok(Envelope { tag, body })
}

/// Splits a byte stream into envelopes; an error leaves the stream unusable.
#[derive(Debug, Default)]
pub struct EnvelopeParser {
    buffer: Vec<u8>,
}

impl EnvelopeParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend(bytes);
    }

    pub fn next_envelope(&mut self) -> Option<Result<Envelope, Int14hError>> {
        let prefix: [u8; LENGTH_PREFIX_LEN] =
            self.buffer.get(..LENGTH_PREFIX_LEN)?.try_into().ok()?;
        let len = u32::from_le_bytes(prefix) as usize;
        if len > MAX_ENVELOPE_LEN {
            return Some(Err(Int14hError::EnvelopeTooLarge(len)));
        }
        let unit = self
            .buffer
            .get(LENGTH_PREFIX_LEN..LENGTH_PREFIX_LEN + len)?;
        let parsed = parse_envelope(unit);
        self.buffer.drain(..LENGTH_PREFIX_LEN + len);
        Some(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ConnectResult, DialString, ExecStatus, LineRate, LinkStatus, MessageBody, ProgramName,
        SharedData, SwitchAddress, SwitchHostResult, Ticks,
    };

    fn every_call() -> Vec<Call> {
        vec![
            Call::GetStatus,
            Call::GetSharedData,
            Call::SetSharedData(SharedData::try_new(vec![1, 2, 3]).unwrap()),
            Call::Connect(DialString::try_new("+++AT!~tSIERRAATDT5551234!~").unwrap()),
            Call::Send(MessageBody::try_new(vec![0x22, 0x04, 0x10]).unwrap()),
            Call::Receive,
            Call::SetAckTimeout(Ticks(300)),
            Call::Disconnect,
            Call::Poll,
            Call::SetNextProgram(Some(ProgramName::try_new("SLand").unwrap())),
            Call::SetNextProgram(None),
            Call::Service,
            Call::GetPreviousProgram,
            Call::IsTransmitIdle,
            Call::SwitchHost(SwitchAddress::try_new("311083420207").unwrap()),
            Call::Flush,
            Call::SetCallbacks { installed: true },
            Call::GetLineRate,
        ]
    }

    fn every_reply() -> Vec<Reply> {
        vec![
            Reply::GetStatus(ExecStatus {
                driver_version: 3,
                connected: true,
            }),
            Reply::GetSharedData(SharedData::try_new(vec![9; 256]).unwrap()),
            Reply::SetSharedData,
            Reply::Connect(ConnectResult::WrongPrompt),
            Reply::Send { queued: true },
            Reply::Receive(Some(MessageBody::try_new(vec![0x41; 300]).unwrap())),
            Reply::Receive(None),
            Reply::SetAckTimeout,
            Reply::Disconnect,
            Reply::Poll(LinkStatus::CarrierLost),
            Reply::SetNextProgram,
            Reply::Service(LinkStatus::Ok),
            Reply::GetPreviousProgram(Some(ProgramName::try_new("DEFAULT").unwrap())),
            Reply::IsTransmitIdle { idle: false },
            Reply::SwitchHost(SwitchHostResult::Remade),
            Reply::Flush,
            Reply::SetCallbacks,
            Reply::GetLineRate(LineRate(2400)),
        ]
    }

    #[test]
    fn every_export_round_trips_through_a_stream() {
        let bodies = every_call().into_iter().map(Body::Call);
        let bodies: Vec<Body> = bodies
            .chain(every_reply().into_iter().map(Body::Reply))
            .collect();
        let envelopes: Vec<Envelope> = bodies
            .into_iter()
            .enumerate()
            .map(|(n, body)| Envelope {
                tag: Tag(n as u16),
                body,
            })
            .collect();
        let stream: Vec<u8> = envelopes.iter().flat_map(encode_envelope).collect();
        let mut parser = EnvelopeParser::new();
        let mut parsed = Vec::new();
        for chunk in stream.chunks(5) {
            parser.push(chunk);
            while let Some(envelope) = parser.next_envelope() {
                parsed.push(envelope.unwrap());
            }
        }
        assert_eq!(parsed, envelopes);
        let exports: std::collections::HashSet<Export> =
            every_call().iter().map(Call::export).collect();
        assert_eq!(exports.len(), crate::EXPORT_TABLE.len());
    }

    #[test]
    fn malformed_units_are_rejected() {
        assert_eq!(
            parse_envelope(&[0x10, 0, 0, 17]),
            Err(Int14hError::UnknownExport(17))
        );
        assert_eq!(
            parse_envelope(&[0x10, 0, 0, 8, 0]),
            Err(Int14hError::TrailingBytes(1))
        );
        assert_eq!(
            parse_envelope(&[0x11, 0, 0, 8]),
            Err(Int14hError::Truncated)
        );
        assert_eq!(
            parse_envelope(&[0x7F, 0, 0]),
            Err(Int14hError::UnknownKind(0x7F))
        );
        assert_eq!(
            parse_envelope(&[0x01, 0, 0, b'X', b'1', b'4', b'H']),
            Err(Int14hError::BadMagic)
        );
        let mut parser = EnvelopeParser::new();
        parser.push(&[0xFF, 0xFF, 0xFF, 0x00]);
        assert_eq!(
            parser.next_envelope(),
            Some(Err(Int14hError::EnvelopeTooLarge(0xFF_FFFF)))
        );
    }
}
