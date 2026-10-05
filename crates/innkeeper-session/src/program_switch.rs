//! The host's quiet window while the client chains programs (docs/protocol/int14h-api.md 9.3).
//! The driver arms it when the world says a program ended (`PlayerSession::ends_program`).

use std::time::Instant;

use crate::assumptions::PROGRAM_SWITCH_QUIET_MAX_ASSUMED;
use crate::{Session, SessionError};

impl Session {
    /// Holds host frames until the next program's first DATA frame, so none arrives while the
    /// client drops its queues; at the deadline they leave in order.
    pub fn begin_program_switch(&mut self, now: Instant) -> Result<(), SessionError> {
        let link = self.online_link()?;
        link.hold_until_heard(now, PROGRAM_SWITCH_QUIET_MAX_ASSUMED);
        Ok(())
    }
}
