//! TSNEXEC's INT 14h exports as typed calls and replies (docs/protocol/int14h-api.md), and the
//! envelope codec that carries them over TCP (docs/protocol/int14h-transport.md).
#![forbid(unsafe_code)]

mod call;
mod codes;
mod envelope;
mod error;
mod export;
mod field;
mod reply;
mod wire;

pub use call::Call;
pub use codes::{ConnectResult, LanErrorCode, LinkStatus, SwitchHostResult};
pub use envelope::{
    encode_envelope, parse_envelope, Body, Envelope, EnvelopeParser, Hello, Tag, Welcome,
    PROTOCOL_VERSION,
};
pub use error::Int14hError;
pub use export::{Export, ExportRow, LockUse, EXPORT_TABLE};
pub use field::{
    BoundedBytes, BoundedText, DialString, ExecStatus, LineRate, MessageBody, PeerName,
    ProgramName, SharedData, SwitchAddress, Ticks,
};
pub use reply::Reply;
