use std::fmt;

use crate::message::wire::{WireReader, WireWriter};
use crate::message::Command;
use crate::{AccountId, ClientVersion, Cookie, LandType, MessageError, Sid, Stamp};

/// A message the client transmits, decoded; layouts are in docs/protocol/messages.md.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    Login(Login),
    JoinNet(JoinNet),
    LeaveNet(Sid),
    GroupJoin(GroupJoin),
    HostInfo(HostInfoRequest),
    LandOccupancyRequest,
}

/// Command 53, or 59 with a Prodigy ID: the first message after every Connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub land_type: LandType,
    pub version: ClientVersion,
    pub account: AccountId,
    pub password_source: PasswordSource,
    pub password: EncodedPassword,
    pub name: String,
    pub prodigy_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PasswordSource {
    Typed = 0,
    StoredFile = 1,
}

impl TryFrom<u8> for PasswordSource {
    type Error = MessageError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        match byte {
            0 => Ok(PasswordSource::Typed),
            1 => Ok(PasswordSource::StoredFile),
            value => Err(MessageError::UnknownValue {
                field: "fromFile",
                value: u16::from(value),
            }),
        }
    }
}

/// The password as `script.095` encodes it; the client never sends the plain text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EncodedPassword(pub [u8; 10]);

impl fmt::Debug for EncodedPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncodedPassword(..)")
    }
}

/// Command 7: the client asks for a SID for one of its objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinNet {
    pub cookie: Cookie,
    pub kind: ObjectKind,
    pub land_type: LandType,
    /// The land number for a waiting-room group, otherwise 0.
    pub parameter: u16,
    /// The property count for objects, the member limit for groups.
    pub size: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectKind {
    Object = 1,
    GameGroup = 2,
    RedBaron = 4,
    WaitingRoom = 5,
    GameObject = 129,
}

const OBJECT_KINDS: [ObjectKind; 5] = [
    ObjectKind::Object,
    ObjectKind::GameGroup,
    ObjectKind::RedBaron,
    ObjectKind::WaitingRoom,
    ObjectKind::GameObject,
];

impl ObjectKind {
    pub fn is_group(self) -> bool {
        match self {
            ObjectKind::GameGroup | ObjectKind::WaitingRoom => true,
            ObjectKind::Object | ObjectKind::RedBaron | ObjectKind::GameObject => false,
        }
    }

    const fn byte(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for ObjectKind {
    type Error = MessageError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        OBJECT_KINDS
            .into_iter()
            .find(|kind| kind.byte() == byte)
            .ok_or(MessageError::UnknownValue {
                field: "kind",
                value: u16::from(byte),
            })
    }
}

/// Command 10: the client adds one of its objects to a group; waiting rooms add the version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupJoin {
    pub group: Sid,
    pub member: Sid,
    pub version: Option<ClientVersion>,
}

/// Command 36 from the client: one of the host's service tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostInfoRequest {
    HostAddressFile { stamp: Stamp },
    HostTime,
    HostNumber,
    LandDirectory { stamp: Stamp },
}

const HOST_ADDRESS_FILE: u8 = 1;
const HOST_TIME: u8 = 2;
const HOST_NUMBER: u8 = 5;
const LAND_DIRECTORY: u8 = 6;
const LAND_OCCUPANCY: u8 = 1;

impl ClientMessage {
    pub fn parse(bytes: &[u8]) -> Result<Self, MessageError> {
        let mut reader = WireReader::new(bytes);
        let command = Command::try_from(reader.byte("command")?)?;
        let message = match command {
            Command::Login | Command::LoginWithProdigyId => {
                ClientMessage::Login(Login::parse(&mut reader, command)?)
            }
            Command::JoinNet => ClientMessage::JoinNet(JoinNet::parse(&mut reader)?),
            Command::LeaveNet => {
                reader.byte("flags")?;
                let sid = Sid(reader.word("sid")?);
                reader.word("zero")?;
                ClientMessage::LeaveNet(sid)
            }
            Command::GroupJoin => ClientMessage::GroupJoin(GroupJoin::parse(&mut reader)?),
            Command::HostInfo => ClientMessage::HostInfo(HostInfoRequest::parse(&mut reader)?),
            Command::WaitGroup => {
                expect_sub(&mut reader, command, LAND_OCCUPANCY)?;
                reader.word("toSID")?;
                reader.word("fromSID")?;
                ClientMessage::LandOccupancyRequest
            }
            Command::Ack | Command::Nak | Command::ObjId => {
                return Err(MessageError::UnsupportedCommand(command.byte()))
            }
        };
        reader.finish()?;
        Ok(message)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = WireWriter::default();
        match self {
            ClientMessage::Login(login) => login.write(&mut writer),
            ClientMessage::JoinNet(join) => join.write(&mut writer),
            ClientMessage::LeaveNet(sid) => {
                writer
                    .byte(Command::LeaveNet.byte())
                    .byte(0)
                    .word(sid.0)
                    .word(0);
            }
            ClientMessage::GroupJoin(join) => join.write(&mut writer),
            ClientMessage::HostInfo(request) => request.write(&mut writer),
            ClientMessage::LandOccupancyRequest => {
                writer.byte(Command::WaitGroup.byte()).byte(LAND_OCCUPANCY);
                writer.word(0).word(0);
            }
        }
        writer.into_bytes()
    }
}

fn expect_sub(reader: &mut WireReader, command: Command, sub: u8) -> Result<(), MessageError> {
    match reader.byte("sub-command")? {
        found if found == sub => Ok(()),
        found => Err(MessageError::UnsupportedSub {
            command: command.byte(),
            sub: found,
        }),
    }
}

impl Login {
    fn parse(reader: &mut WireReader, command: Command) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        reader.word("toSID")?;
        let land_type = LandType(reader.byte("landType")?);
        let [major, minor, revision] = reader.array("version")?;
        let low = reader.word("idLow")?;
        let high = reader.word("idHigh")?;
        let password_source = PasswordSource::try_from(reader.byte("fromFile")?)?;
        let password = reader.array("password")?;
        reader.byte("password padding")?;
        let name = reader.text("name")?;
        let prodigy_id = match command {
            Command::LoginWithProdigyId => Some(reader.text("prodigyId")?),
            _ => None,
        };
        Ok(Login {
            land_type,
            version: ClientVersion::new(major, minor, revision),
            account: AccountId::from_words(low, high),
            password_source,
            password: EncodedPassword(password),
            name,
            prodigy_id,
        })
    }

    fn write(&self, writer: &mut WireWriter) {
        let command = match self.prodigy_id {
            Some(_) => Command::LoginWithProdigyId,
            None => Command::Login,
        };
        let version = self.version;
        writer.byte(command.byte()).byte(0).word(Sid::GAME_OBJECT.0);
        writer.byte(self.land_type.0);
        writer.array(&[version.major, version.minor, version.revision]);
        let (low, high) = self.account.words();
        writer.word(low).word(high);
        writer
            .byte(self.password_source as u8)
            .array(&self.password.0);
        writer.byte(0); // the 11th password byte is heap garbage the host ignores
        writer.text(&self.name);
        if let Some(prodigy_id) = &self.prodigy_id {
            writer.text(prodigy_id);
        }
    }
}

impl JoinNet {
    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        reader.word("toSID")?;
        let cookie = Cookie(reader.word("cookie")?);
        let kind = ObjectKind::try_from(reader.byte("kind")?)?;
        Ok(JoinNet {
            cookie,
            kind,
            land_type: LandType(reader.byte("landType")?),
            parameter: reader.word("param")?,
            size: reader.word("size")?,
        })
    }

    fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::JoinNet.byte()).byte(0).word(0);
        writer.word(self.cookie.0).byte(self.kind.byte());
        writer.byte(self.land_type.0);
        writer.word(self.parameter).word(self.size);
    }
}

impl GroupJoin {
    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let group = Sid(reader.word("groupSID")?);
        let member = Sid(reader.word("memberSID")?);
        let version = match reader.is_empty() {
            true => None,
            false => {
                let [major, minor, revision] = reader.array("version")?;
                Some(ClientVersion::new(major, minor, revision))
            }
        };
        Ok(GroupJoin {
            group,
            member,
            version,
        })
    }

    fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::GroupJoin.byte()).byte(0);
        writer.word(self.group.0).word(self.member.0);
        if let Some(version) = self.version {
            writer.array(&[version.major, version.minor, version.revision]);
        }
    }
}

impl HostInfoRequest {
    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let request = match reader.byte("sub-command")? {
            HOST_ADDRESS_FILE => HostInfoRequest::HostAddressFile {
                stamp: read_stamp(reader)?,
            },
            HOST_TIME => HostInfoRequest::HostTime,
            HOST_NUMBER => HostInfoRequest::HostNumber,
            LAND_DIRECTORY => HostInfoRequest::LandDirectory {
                stamp: read_stamp(reader)?,
            },
            sub => {
                let command = Command::HostInfo.byte();
                return Err(MessageError::UnsupportedSub { command, sub });
            }
        };
        Ok(request)
    }

    fn write(&self, writer: &mut WireWriter) {
        writer.byte(Command::HostInfo.byte());
        match *self {
            HostInfoRequest::HostAddressFile { stamp } => {
                write_stamp(writer.byte(HOST_ADDRESS_FILE), stamp);
            }
            HostInfoRequest::HostTime => {
                writer.byte(HOST_TIME);
            }
            HostInfoRequest::HostNumber => {
                writer.byte(HOST_NUMBER);
            }
            HostInfoRequest::LandDirectory { stamp } => {
                write_stamp(writer.byte(LAND_DIRECTORY), stamp);
            }
        }
    }
}

pub(crate) fn read_stamp(reader: &mut WireReader) -> Result<Stamp, MessageError> {
    let low = reader.word("stampLow")?;
    let high = reader.word("stampHigh")?;
    Ok(Stamp::from_words(low, high))
}

pub(crate) fn write_stamp(writer: &mut WireWriter, stamp: Stamp) {
    let (low, high) = stamp.words();
    writer.word(low).word(high);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_object_kind_and_trailing_bytes_are_rejected() {
        let mut join = vec![7, 0, 0, 0, 0x34, 0x12, 3, 1, 0, 0, 0, 0];
        assert_eq!(
            ClientMessage::parse(&join),
            Err(MessageError::UnknownValue {
                field: "kind",
                value: 3
            })
        );
        join[6] = 129;
        join.push(0);
        assert_eq!(
            ClientMessage::parse(&join),
            Err(MessageError::TrailingBytes(1))
        );
    }

    #[test]
    fn host_info_sub_commands_round_trip() {
        let requests = [
            HostInfoRequest::HostAddressFile { stamp: Stamp(7) },
            HostInfoRequest::HostTime,
            HostInfoRequest::HostNumber,
            HostInfoRequest::LandDirectory {
                stamp: Stamp(0x0102_0304),
            },
        ];
        for request in requests {
            let message = ClientMessage::HostInfo(request);
            assert_eq!(ClientMessage::parse(&message.encode()), Ok(message));
        }
        assert_eq!(
            ClientMessage::parse(&[36, 9]),
            Err(MessageError::UnsupportedSub {
                command: 36,
                sub: 9
            })
        );
    }

    #[test]
    fn messages_the_capture_lacks_round_trip() {
        let login = Login {
            land_type: LandType(2),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(0x0001_86A1),
            password_source: PasswordSource::Typed,
            password: EncodedPassword([9; 10]),
            name: "guybrush".into(),
            prodigy_id: Some("ABCD12A".into()),
        };
        let messages = [
            ClientMessage::Login(login),
            ClientMessage::LeaveNet(Sid(0x0103)),
            ClientMessage::GroupJoin(GroupJoin {
                group: Sid(0x0103),
                member: Sid(0x0102),
                version: None,
            }),
            ClientMessage::LandOccupancyRequest,
        ];
        for message in messages {
            assert_eq!(ClientMessage::parse(&message.encode()), Ok(message));
        }
    }

    #[test]
    fn the_password_stays_out_of_debug_output() {
        assert_eq!(
            format!("{:?}", EncodedPassword([0x5A; 10])),
            "EncodedPassword(..)"
        );
    }
}
