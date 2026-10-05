//! One client connection, sans-IO: the phone line (`pad_thai`) feeding the host link (`tsn-link`).
//! The stages follow the server checklist in docs/protocol/link-layer.md.
#![forbid(unsafe_code)]

mod error;
mod session;

pub use error::SessionError;
pub use session::{Session, SessionConfig, SessionEvent, SessionOutput};
