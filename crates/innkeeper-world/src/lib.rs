//! Host behaviour for INN clients, sans-IO: typed messages, accounts, logon and land tables.
//! The messages are specified in docs/protocol/messages.md; the crate in docs/server/innkeeperd.md.
#![forbid(unsafe_code)]

mod account;
mod assumptions;
mod error;
mod host_time;
mod ids;
mod land;
mod logon;
mod message;
mod objects;
mod player;
pub mod router;
mod store;
mod world;

pub use account::{Account, AccountBook, Enrolment, Refusal};
pub use error::MessageError;
pub use host_time::{Clock, FixedClock, HostTime, SystemClock};
pub use ids::{
    AccountId, ClientVersion, Cookie, HostNumber, LandFlags, LandNumber, LandType, Sid, Stamp,
};
pub use land::LandCatalog;
pub use message::{
    Ack, ChangePassword, ClientMessage, EncodedPassword, GroupJoin, GroupLeave, GroupMembers,
    GroupMembersRequest, HostInfoRequest, HostMessage, IntProperty, JoinNet, LandDirectory,
    LandEntry, Login, LoginAck, LoginNakReason, LoginStatus, Multicast, Nak, Notice, ObjExists,
    ObjectKind, ObjectLocated, Occupancy, PasswordSource, SendMessage, SetInt, SetStr,
};
pub use objects::{ConnectionId, Delivery, GroupKey, ObjectStore};
pub use player::PlayerSession;
#[cfg(feature = "test-support")]
pub use store::conformance;
pub use store::{
    AccountRecord, BoardId, Letter, LetterId, MailboxNumber, MemoryStore, Post, PostId, Store,
    StoreError,
};
pub use world::World;
