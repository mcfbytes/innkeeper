use crate::wire::{WireReader, WireWriter};
use crate::{
    ConnectResult, ExecStatus, Export, Int14hError, LineRate, LinkStatus, MessageBody, ProgramName,
    SharedData, SwitchHostResult,
};

/// GetStatus reports `0x80` in `AH` while connected and 0 otherwise.
const CONNECTED_BYTE: u8 = 0x80;

/// One export's result, the same 17 cases as [`crate::Call`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    GetStatus(ExecStatus),
    GetSharedData(SharedData),
    SetSharedData,
    Connect(ConnectResult),
    Send {
        queued: bool,
    },
    /// The oldest complete message, or `None` when the queue is empty.
    Receive(Option<MessageBody>),
    SetAckTimeout,
    Disconnect,
    Poll(LinkStatus),
    SetNextProgram,
    Service(LinkStatus),
    GetPreviousProgram(Option<ProgramName>),
    IsTransmitIdle {
        idle: bool,
    },
    SwitchHost(SwitchHostResult),
    Flush,
    SetCallbacks,
    GetLineRate(LineRate),
}

impl Reply {
    pub fn export(&self) -> Export {
        match self {
            Reply::GetStatus(_) => Export::GetStatus,
            Reply::GetSharedData(_) => Export::GetSharedData,
            Reply::SetSharedData => Export::SetSharedData,
            Reply::Connect(_) => Export::Connect,
            Reply::Send { .. } => Export::Send,
            Reply::Receive(_) => Export::Receive,
            Reply::SetAckTimeout => Export::SetAckTimeout,
            Reply::Disconnect => Export::Disconnect,
            Reply::Poll(_) => Export::Poll,
            Reply::SetNextProgram => Export::SetNextProgram,
            Reply::Service(_) => Export::Service,
            Reply::GetPreviousProgram(_) => Export::GetPreviousProgram,
            Reply::IsTransmitIdle { .. } => Export::IsTransmitIdle,
            Reply::SwitchHost(_) => Export::SwitchHost,
            Reply::Flush => Export::Flush,
            Reply::SetCallbacks => Export::SetCallbacks,
            Reply::GetLineRate(_) => Export::GetLineRate,
        }
    }

    pub(crate) fn encode_result(&self, writer: &mut WireWriter) {
        match self {
            Reply::GetStatus(status) => {
                writer.u8(status.driver_version);
                writer.u8(if status.connected { CONNECTED_BYTE } else { 0 });
            }
            Reply::GetSharedData(data) => writer.bytes(data),
            Reply::Connect(result) => writer.u8(result.code()),
            Reply::Send { queued } => writer.bool(*queued),
            Reply::Receive(message) => writer.option(message.as_ref(), WireWriter::bytes),
            Reply::Poll(status) | Reply::Service(status) => writer.u8(status.code()),
            Reply::GetPreviousProgram(name) => writer.option(name.as_ref(), WireWriter::text),
            Reply::IsTransmitIdle { idle } => writer.bool(*idle),
            Reply::SwitchHost(result) => writer.u8(result.code()),
            Reply::GetLineRate(LineRate(rate)) => writer.u16(*rate),
            Reply::SetSharedData
            | Reply::SetAckTimeout
            | Reply::Disconnect
            | Reply::SetNextProgram
            | Reply::Flush
            | Reply::SetCallbacks => {}
        }
    }

    pub(crate) fn parse_result(
        export: Export,
        reader: &mut WireReader,
    ) -> Result<Reply, Int14hError> {
        Ok(match export {
            Export::GetStatus => Reply::GetStatus(parse_exec_status(reader)?),
            Export::GetSharedData => Reply::GetSharedData(reader.bytes()?),
            Export::SetSharedData => Reply::SetSharedData,
            Export::Connect => Reply::Connect(ConnectResult::try_from(reader.u8()?)?),
            Export::Send => Reply::Send {
                queued: reader.bool()?,
            },
            Export::Receive => Reply::Receive(reader.option(WireReader::bytes)?),
            Export::SetAckTimeout => Reply::SetAckTimeout,
            Export::Disconnect => Reply::Disconnect,
            Export::Poll => Reply::Poll(LinkStatus::try_from(reader.u8()?)?),
            Export::SetNextProgram => Reply::SetNextProgram,
            Export::Service => Reply::Service(LinkStatus::try_from(reader.u8()?)?),
            Export::GetPreviousProgram => {
                Reply::GetPreviousProgram(reader.option(WireReader::text)?)
            }
            Export::IsTransmitIdle => Reply::IsTransmitIdle {
                idle: reader.bool()?,
            },
            Export::SwitchHost => Reply::SwitchHost(SwitchHostResult::try_from(reader.u8()?)?),
            Export::Flush => Reply::Flush,
            Export::SetCallbacks => Reply::SetCallbacks,
            Export::GetLineRate => Reply::GetLineRate(LineRate(reader.u16()?)),
        })
    }
}

fn parse_exec_status(reader: &mut WireReader) -> Result<ExecStatus, Int14hError> {
    let driver_version = reader.u8()?;
    let connected = match reader.u8()? {
        CONNECTED_BYTE => true,
        0 => false,
        value => {
            return Err(Int14hError::UnknownCode {
                field: "connection byte",
                value,
            })
        }
    };
    Ok(ExecStatus {
        driver_version,
        connected,
    })
}
