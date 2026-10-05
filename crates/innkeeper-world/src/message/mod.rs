//! Typed application messages in both directions, parsed and encoded at the link boundary.

mod ack;
mod client;
mod command;
mod group;
mod host;
mod mail;
mod multicast;
mod notice;
mod object_kind;
mod properties;
mod send;
mod service_lookup;
mod user_info;
mod wire;

pub use ack::Ack;
pub use client::{
    ChangePassword, ClientMessage, EncodedPassword, GroupJoin, HostInfoRequest, JoinNet, Login,
    PasswordSource,
};
pub(crate) use command::Command;
pub use group::{GroupLeave, GroupMembers, GroupMembersRequest};
pub use host::{
    HostMessage, LandDirectory, LandEntry, LoginAck, LoginNakReason, LoginStatus, Nak, Occupancy,
};
pub use mail::{
    Envelope, LetterRef, ListedLetter, MailFault, MailReply, MailRequest, MailboxAnswer,
    MailboxRequest, OutgoingLetter,
};
pub(crate) use mail::{StoredLetter, FIELD_LEN};
pub use multicast::Multicast;
pub use notice::Notice;
pub use object_kind::ObjectKind;
pub use properties::{IntProperty, SetInt, SetStr};
pub use send::SendMessage;
pub use service_lookup::{ObjExists, ObjectLocated};
pub use user_info::SetPersona;
