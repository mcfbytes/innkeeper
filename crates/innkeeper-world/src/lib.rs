//! Host behaviour for INN clients, sans-IO: typed messages, accounts, logon and land tables.
//! The messages are specified in docs/protocol/messages.md; the crate in docs/server/innkeeperd.md.
#![forbid(unsafe_code)]

mod account;
mod assumptions;
mod bbs;
mod error;
mod host_time;
mod ids;
mod land;
mod logon;
mod mail;
mod message;
mod money;
mod objects;
mod player;
mod presence;
mod red_baron;
pub mod router;
mod rpg;
pub mod saves;
mod store;
mod world;

pub use account::{Account, AccountBook, Enrolment, Refusal};
pub use error::MessageError;
pub use host_time::{Clock, FixedClock, HostTime, SystemClock};
pub use ids::{
    AccountId, ClientVersion, Cookie, HostNumber, LandFlags, LandNumber, LandType, Money, Sid,
    Stamp,
};
pub use land::LandCatalog;
pub use message::{
    Ack, BbsReply, BbsRequest, BoardListing, BoardRef, ChangePassword, ClientMessage,
    EncodedPassword, Envelope, GroupJoin, GroupLeave, GroupMembers, GroupMembersRequest,
    HostInfoRequest, HostMessage, IntProperty, JoinNet, LandDirectory, LandEntry, LetterRef,
    ListedLetter, Login, LoginAck, LoginNakReason, LoginStatus, MailFault, MailReply, MailRequest,
    MailboxAnswer, MailboxRequest, MessagePosition, MoneyReply, Multicast, Nak, Notice, ObjExists,
    ObjectKind, ObjectLocated, Occupancy, OutgoingLetter, PasswordSource, PlayStatus, ReadMoney,
    ReadStep, SaveMoney, SendMessage, SetInt, SetPersona, SetStr,
};
pub use message::{
    InvokeMethod, LockId, LockRefused, LockRequest, MemberProperties, MemberRow, Property,
    PropertyColumn, PropertyKind, PropertyRequest, PropertyValue, PropertyValues,
    ServicePropertyRequest, Unexplained, UnexplainedCommand,
};
pub use objects::{
    ConnectionId, Delivery, GamePhase, GameSession, GameType, GroupKey, Invitation, ObjectStore,
    Place,
};
pub use player::PlayerSession;
#[cfg(feature = "test-support")]
pub use store::conformance;
pub use store::{
    AccountRecord, BoardId, Letter, LetterId, MailboxNumber, MemoryStore, Post, PostId, Store,
    StoreError,
};
pub use world::World;
