//! TSNEXEC's link layer as a sans-IO state machine: framing, CRC, ACK/NAK and message packing.
//! The wire format is specified in docs/protocol/link-layer.md.
#![forbid(unsafe_code)]

mod assumptions;
mod control;
mod crc;
mod error;
mod frame;
mod link;
mod message;
mod packer;
mod parser;

pub use control::{Control, Seq};
pub use error::LinkError;
pub use frame::{encode_frame, Frame};
pub use link::{Link, LinkConfig, LinkEvent, LinkOutput};
pub use message::Message;
pub use parser::{FrameParser, Received};
