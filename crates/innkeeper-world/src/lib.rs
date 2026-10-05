//! Host behaviour for INN clients, sans-IO: typed messages, accounts, logon and land tables.
//! The messages are specified in docs/protocol/messages.md; the crate in docs/server/innkeeperd.md.
#![forbid(unsafe_code)]

mod account;
mod assumptions;
mod error;
mod ids;
mod land;
mod message;
mod player;
mod world;

pub use account::{Account, AccountBook, Refusal};
pub use error::MessageError;
pub use ids::{
    AccountId, ClientVersion, Cookie, HostNumber, LandFlags, LandNumber, LandType, Sid, Stamp,
};
pub use land::LandCatalog;
pub use message::{
    ClientMessage, EncodedPassword, GroupJoin, HostInfoRequest, HostMessage, JoinNet,
    LandDirectory, LandEntry, Login, LoginAck, LoginNakReason, LoginStatus, Nak, ObjectKind,
    Occupancy, PasswordSource,
};
pub use player::PlayerSession;
pub use world::World;
