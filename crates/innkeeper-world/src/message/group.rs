use crate::assumptions::MEMBER_LIST_UNREAD_WORD_ASSUMED;
use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{MessageError, Sid};

/// Command 11: `member` leaves `group`. The host sends the same fields to the group as `GroupLeft`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupLeave {
    pub group: Sid,
    pub member: Sid,
}

impl GroupLeave {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        Ok(GroupLeave {
            group: Sid(reader.word("groupSID")?),
            member: Sid(reader.word("memberSID")?),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::GroupLeave.byte()).byte(0);
        writer.word(self.group.0).word(self.member.0);
    }
}

/// Command 12 from the client: `user` asks for the member list of `group`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupMembersRequest {
    pub group: Sid,
    pub user: Sid,
}

impl GroupMembersRequest {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        Ok(GroupMembersRequest {
            group: Sid(reader.word("groupSID")?),
            user: Sid(reader.word("userSID")?),
        })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::GroupMembers.byte()).byte(0);
        writer.word(self.group.0).word(self.user.0);
    }
}

/// Command 12 from the host: every member SID of `group`, from byte 6 to the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMembers {
    pub group: Sid,
    pub members: Vec<Sid>,
}

impl GroupMembers {
    pub(crate) fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let group = Sid(reader.word("toSID")?);
        reader.word("unused")?;
        let mut members = Vec::new();
        while !reader.is_empty() {
            members.push(Sid(reader.word("member")?));
        }
        Ok(GroupMembers { group, members })
    }

    pub(crate) fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::GroupMembers.byte()).byte(0);
        writer
            .word(self.group.0)
            .word(MEMBER_LIST_UNREAD_WORD_ASSUMED);
        for member in &self.members {
            writer.word(member.0);
        }
    }
}
