use crate::message::wire::{WireReader, WireWriter};
use crate::message::{Ack, Command};
use crate::{MessageError, Sid};

/// One word of a host lock; the invitation scripts lock the SIDs of the players they invite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LockId(pub u16);

/// Commands 4 (lock) and 6 (unlock): every word after `from` names one lock, taken or freed together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockRequest {
    pub from: Sid,
    pub locks: Vec<LockId>,
}

/// The Nak that refuses a lock: `taken` is the first requested lock someone else holds, at word 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockRefused {
    pub to: Sid,
    pub taken: LockId,
}

impl LockRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let from = Sid(reader.word("fromSID")?);
        let mut locks = Vec::new();
        while !reader.is_empty() {
            locks.push(LockId(reader.word("lockId")?));
        }
        Ok(LockRequest { from, locks })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter, command: Command) {
        writer.byte(command.byte()).byte(0).word(self.from.0);
        for lock in &self.locks {
            writer.word(lock.0);
        }
    }

    /// The Ack for this request as `command`, addressed to the object that asked.
    pub(crate) fn acknowledged(&self, command: Command) -> Ack {
        Ack {
            to: self.from,
            which_cmd: command.byte(),
            which_sub: 0,
            tail: Vec::new(),
        }
    }

    pub(crate) fn refused(&self, taken: LockId) -> LockRefused {
        LockRefused {
            to: self.from,
            taken,
        }
    }
}

impl LockRefused {
    /// Reads on from byte 5, where a Nak for any other command has its `whichSub`.
    pub(crate) fn parse_after(reader: &mut WireReader, to: Sid) -> Result<Self, MessageError> {
        let taken = LockId(reader.word("holder")?);
        reader.byte("numTries")?;
        Ok(LockRefused { to, taken })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::Nak.byte()).byte(0).word(self.to.0);
        writer.byte(Command::Lock.byte()).word(self.taken.0).byte(0);
    }
}
