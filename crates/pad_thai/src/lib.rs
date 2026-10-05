//! The phone-line side of an INN session: Hayes modem, SprintNet X.25 PAD dialogue, both sans-IO.
//! The client's half of the dialogue is in docs/protocol/link-layer.md.
#![forbid(unsafe_code)]

mod assumptions;
mod error;
mod hayes_fever;
mod host;
mod line;
mod pad;
mod typed_line;

pub use error::PadError;
pub use hayes_fever::{HayesConfig, ModemEvent};
pub use host::HostAddress;
pub use line::{Line, LineEvent, LineKind, LineOutput};
pub use pad::PadEvent;
