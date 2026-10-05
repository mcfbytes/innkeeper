use crate::wire::{WireReader, WireWriter};
use crate::{
    DialString, Export, Int14hError, MessageBody, ProgramName, SharedData, SwitchAddress, Ticks,
};

/// One export invocation with its arguments; handles and far pointers stay on the client side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    GetStatus,
    GetSharedData,
    SetSharedData(SharedData),
    Connect(DialString),
    Send(MessageBody),
    Receive,
    SetAckTimeout(Ticks),
    Disconnect,
    Poll,
    /// `None` cancels a pending program switch.
    SetNextProgram(Option<ProgramName>),
    Service,
    GetPreviousProgram,
    IsTransmitIdle,
    SwitchHost(SwitchAddress),
    Flush,
    /// The three callbacks travel as a flag: installed, or all NULL.
    SetCallbacks {
        installed: bool,
    },
    GetLineRate,
}

impl Call {
    pub fn export(&self) -> Export {
        match self {
            Call::GetStatus => Export::GetStatus,
            Call::GetSharedData => Export::GetSharedData,
            Call::SetSharedData(_) => Export::SetSharedData,
            Call::Connect(_) => Export::Connect,
            Call::Send(_) => Export::Send,
            Call::Receive => Export::Receive,
            Call::SetAckTimeout(_) => Export::SetAckTimeout,
            Call::Disconnect => Export::Disconnect,
            Call::Poll => Export::Poll,
            Call::SetNextProgram(_) => Export::SetNextProgram,
            Call::Service => Export::Service,
            Call::GetPreviousProgram => Export::GetPreviousProgram,
            Call::IsTransmitIdle => Export::IsTransmitIdle,
            Call::SwitchHost(_) => Export::SwitchHost,
            Call::Flush => Export::Flush,
            Call::SetCallbacks { .. } => Export::SetCallbacks,
            Call::GetLineRate => Export::GetLineRate,
        }
    }

    pub(crate) fn encode_args(&self, writer: &mut WireWriter) {
        match self {
            Call::SetSharedData(data) => writer.bytes(data),
            Call::Connect(dial) => writer.text(dial),
            Call::Send(message) => writer.bytes(message),
            Call::SetAckTimeout(Ticks(ticks)) => writer.u16(*ticks),
            Call::SetNextProgram(name) => writer.option(name.as_ref(), WireWriter::text),
            Call::SwitchHost(address) => writer.text(address),
            Call::SetCallbacks { installed } => writer.bool(*installed),
            Call::GetStatus
            | Call::GetSharedData
            | Call::Receive
            | Call::Disconnect
            | Call::Poll
            | Call::Service
            | Call::GetPreviousProgram
            | Call::IsTransmitIdle
            | Call::Flush
            | Call::GetLineRate => {}
        }
    }

    pub(crate) fn parse_args(export: Export, reader: &mut WireReader) -> Result<Call, Int14hError> {
        Ok(match export {
            Export::GetStatus => Call::GetStatus,
            Export::GetSharedData => Call::GetSharedData,
            Export::SetSharedData => Call::SetSharedData(reader.bytes()?),
            Export::Connect => Call::Connect(reader.text()?),
            Export::Send => Call::Send(reader.bytes()?),
            Export::Receive => Call::Receive,
            Export::SetAckTimeout => Call::SetAckTimeout(Ticks(reader.u16()?)),
            Export::Disconnect => Call::Disconnect,
            Export::Poll => Call::Poll,
            Export::SetNextProgram => Call::SetNextProgram(reader.option(WireReader::text)?),
            Export::Service => Call::Service,
            Export::GetPreviousProgram => Call::GetPreviousProgram,
            Export::IsTransmitIdle => Call::IsTransmitIdle,
            Export::SwitchHost => Call::SwitchHost(reader.text()?),
            Export::Flush => Call::Flush,
            Export::SetCallbacks => Call::SetCallbacks {
                installed: reader.bool()?,
            },
            Export::GetLineRate => Call::GetLineRate,
        })
    }
}
