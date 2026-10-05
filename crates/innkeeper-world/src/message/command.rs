use crate::MessageError;

/// The command bytes this host reads or writes; the catalog is docs/protocol/messages.md section 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Command {
    Ack = 0,
    Nak = 1,
    Send = 2,
    Lock = 4,
    Unlock = 6,
    JoinNet = 7,
    ObjId = 8,
    LeaveNet = 9,
    GroupJoin = 10,
    GroupLeave = 11,
    GroupMembers = 12,
    SetInt = 13,
    SetStr = 14,
    InvokeMethod = 25,
    PlayerObjectNotice = 26,
    Multicast = 28,
    Register = 30,
    MemberProperties = 31,
    GetProperties = 32,
    Properties = 33,
    HostInfo = 36,
    Mail = 37,
    UserInfo = 40,
    ObjExists = 41,
    ChangePassword = 44,
    NewBox = 45,
    WaitGroup = 47,
    Notice = 48,
    Login = 53,
    LoginWithProdigyId = 59,
}

pub(crate) const COMMANDS: [Command; 30] = [
    Command::Ack,
    Command::Nak,
    Command::Send,
    Command::Lock,
    Command::Unlock,
    Command::JoinNet,
    Command::ObjId,
    Command::LeaveNet,
    Command::GroupJoin,
    Command::GroupLeave,
    Command::GroupMembers,
    Command::SetInt,
    Command::SetStr,
    Command::InvokeMethod,
    Command::PlayerObjectNotice,
    Command::Multicast,
    Command::Register,
    Command::MemberProperties,
    Command::GetProperties,
    Command::Properties,
    Command::HostInfo,
    Command::Mail,
    Command::UserInfo,
    Command::ObjExists,
    Command::ChangePassword,
    Command::NewBox,
    Command::WaitGroup,
    Command::Notice,
    Command::Login,
    Command::LoginWithProdigyId,
];

impl Command {
    pub(crate) const fn byte(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for Command {
    type Error = MessageError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        COMMANDS
            .into_iter()
            .find(|command| command.byte() == byte)
            .ok_or(MessageError::UnsupportedCommand(byte))
    }
}

/// The `whichCmd` that acknowledges a Login: the number of the login it replaced.
pub(crate) const LOGIN_REPLY_COMMAND: u8 = 22;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_one_row_and_round_trips() {
        for command in COMMANDS {
            let rows = COMMANDS.iter().filter(|row| **row == command).count();
            assert_eq!(rows, 1, "{command:?}");
            assert_eq!(Command::try_from(command.byte()), Ok(command));
        }
        assert_eq!(
            Command::try_from(3),
            Err(MessageError::UnsupportedCommand(3))
        );
    }
}
