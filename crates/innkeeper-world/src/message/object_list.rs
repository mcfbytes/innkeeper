//! PaintBall object lists, commands 51 and 52.
//!
//! `placeMen` sends 51/0 and waits until every inventory object has a SID (`SL/script.780`).
//! The reply's word at byte 4 is PaintBall's own handle. After `class_67::init` copies the
//! body to the front, `size()` is still the original length, so `size() - 8` is exactly the
//! SID words. `PaintBall::dispose` sends 52/0 and does not wait. A host copy of 52 is not sent:
//! peers hear `ObjFree` (9), the same notice leaveNet uses.

use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{LandType, MessageError, ObjectKind, Sid};

/// Command 51/0 from `placeMen`: one plain object per inventory item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjNewList {
    /// Word at byte 2. The script sends 0.
    pub preface: u16,
    /// PaintBall's object handle, which the reply echoes at word 4.
    pub cookie: u16,
    pub kind: ObjectKind,
    pub land_type: LandType,
    /// The script sends -2.
    pub parameter: u16,
    /// `NetInventoryObject`'s property count, not the number of objects.
    pub size: u16,
    /// How many objects to create.
    pub count: u16,
}

/// Command 51 from the host: `cookie` at word 4, then one SID word per object from byte 8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjNewListReply {
    pub preface: u16,
    pub cookie: u16,
    /// Word at byte 6. `class_67` stores it and PaintBall does not read it.
    pub unread: u16,
    pub sids: Vec<Sid>,
}

/// Command 52/0 from `PaintBall::dispose`: the SIDs to release. The sender does not wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjFreeList {
    /// Word at byte 2. The script sends 0. A received 52 would route by this handle.
    pub handle: u16,
    pub sids: Vec<Sid>,
}

impl ObjNewList {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        expect_sub(reader, Command::ObjNewList)?;
        let preface = reader.word("preface")?;
        let cookie = reader.word("cookie")?;
        let kind = ObjectKind::try_from(reader.byte("kind")?)?;
        let land_type = LandType(reader.byte("landType")?);
        Ok(ObjNewList {
            preface,
            cookie,
            kind,
            land_type,
            parameter: reader.word("parameter")?,
            size: reader.word("size")?,
            count: reader.word("count")?,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::ObjNewList.byte()).byte(0);
        writer.word(self.preface).word(self.cookie);
        writer.byte(self.kind.byte()).byte(self.land_type.0);
        writer.word(self.parameter).word(self.size).word(self.count);
    }
}

impl ObjNewListReply {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        expect_sub(reader, Command::ObjNewList)?;
        let preface = reader.word("preface")?;
        let cookie = reader.word("cookie")?;
        let unread = reader.word("unread")?;
        let mut sids = Vec::new();
        while !reader.is_empty() {
            sids.push(Sid(reader.word("sid")?));
        }
        Ok(ObjNewListReply {
            preface,
            cookie,
            unread,
            sids,
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::ObjNewList.byte()).byte(0);
        writer
            .word(self.preface)
            .word(self.cookie)
            .word(self.unread);
        for sid in &self.sids {
            writer.word(sid.0);
        }
    }
}

impl ObjFreeList {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        expect_sub(reader, Command::ObjFreeList)?;
        let handle = reader.word("handle")?;
        let count = reader.word("count")?;
        let mut sids = Vec::new();
        for _ in 0..count {
            sids.push(Sid(reader.word("sid")?));
        }
        Ok(ObjFreeList { handle, sids })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        let count = u16::try_from(self.sids.len()).unwrap_or(u16::MAX);
        writer.byte(Command::ObjFreeList.byte()).byte(0);
        writer.word(self.handle).word(count);
        for sid in self.sids.iter().take(usize::from(count)) {
            writer.word(sid.0);
        }
    }
}

fn expect_sub(reader: &mut WireReader, command: Command) -> Result<(), MessageError> {
    match reader.byte("sub-command")? {
        0 => Ok(()),
        sub => Err(MessageError::UnsupportedSub {
            command: command.byte(),
            sub,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, HostMessage};

    #[test]
    fn an_odd_tail_on_a_list_reply_is_truncated() {
        let bytes = [51, 0, 0, 0, 0x34, 0x12, 0, 0, 0x00];
        assert_eq!(
            HostMessage::parse(&bytes),
            Err(MessageError::Truncated { field: "sid" })
        );
    }

    #[test]
    fn an_unknown_list_kind_is_rejected() {
        let bytes = [51, 0, 0, 0, 0, 0, 6, 1, 0, 0, 1, 0, 1, 0];
        assert_eq!(
            ClientMessage::parse(&bytes),
            Err(MessageError::UnknownValue {
                field: "kind",
                value: 6
            })
        );
    }

    #[test]
    fn a_host_does_not_parse_a_free_list() {
        let bytes = [52, 0, 0, 0, 0, 0];
        assert_eq!(
            HostMessage::parse(&bytes),
            Err(MessageError::UnsupportedCommand(52))
        );
    }
}
