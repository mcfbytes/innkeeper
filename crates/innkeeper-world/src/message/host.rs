use crate::assumptions::DIRECTORY_UNREAD_BYTES_ASSUMED;
use crate::host_time::HostTime;
use crate::message::client::{read_stamp, write_stamp};
use crate::message::command::LOGIN_REPLY_COMMAND;
use crate::message::wire::{fitting_count, row_count, WireReader, WireWriter};
use crate::message::{
    Ack, Command, GroupLeave, GroupMembers, Notice, ObjectLocated, SendMessage, SetInt, SetStr,
};
use crate::{
    ClientVersion, Cookie, HostNumber, LandFlags, LandNumber, LandType, MessageError, Sid, Stamp,
};

/// A message the client receives, typed; layouts are in docs/protocol/messages.md.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostMessage {
    LoginAccepted(LoginAck),
    /// An `AckMsg` for any request but a Login.
    Ack(Ack),
    Nak(Nak),
    Send(SendMessage),
    ObjId {
        cookie: Cookie,
        sid: Sid,
    },
    /// `GrpJoin`, delivered to the group object: `member` is now in it.
    GroupJoined {
        group: Sid,
        member: Sid,
    },
    /// `GrpDel`, delivered to the group object: `member` left it.
    GroupLeft(GroupLeave),
    GroupMembers(GroupMembers),
    /// `ObjFree`: the object `sid` is gone and disposes itself.
    ObjectFreed(Sid),
    SetInt(SetInt),
    SetStr(SetStr),
    /// The answer to an `ObjExists` lookup.
    ObjectLocated(ObjectLocated),
    Notice(Notice),
    HostNumber(HostNumber),
    HostTime(HostTime),
    LandDirectory(LandDirectory),
    LandOccupancy(Vec<Occupancy>),
}

/// `AckMsg` for a Login; the client keeps all three fields for the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoginAck {
    pub user_flags: u16,
    pub status: LoginStatus,
    pub rating: u16,
}

/// Byte 7 of the login Ack; only 11 has a known meaning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoginStatus(pub u8);

impl LoginStatus {
    pub const PASSWORD_OUT_OF_DATE: LoginStatus = LoginStatus(11);
}

/// `NakMsg`: a refused request, named by its command and sub-command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nak {
    pub to: Sid,
    pub which_cmd: u8,
    pub which_sub: u8,
    pub num_tries: u8,
    pub text: String,
}

/// Why a Login is refused; the client reads the reason from byte 5 of the Nak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LoginNakReason {
    /// Any reason but a retry shows the text as error 100 + reason and hangs up.
    UnknownAccount = 1,
    /// The Login names a land type this host does not run; the number is the host's choice.
    UnlistedLand = 2,
    /// The client counts a retry and asks for the password again, up to `numTries` times.
    RetryPassword = 9,
}

impl Nak {
    pub fn login(reason: LoginNakReason, num_tries: u8, text: &str) -> Self {
        Nak {
            to: Sid::GAME_OBJECT,
            which_cmd: LOGIN_REPLY_COMMAND,
            which_sub: reason as u8,
            num_tries,
            text: text.to_owned(),
        }
    }
}

/// `WaitGrpRequest` msgType 2: every land the client may enter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandDirectory {
    pub stamp: Stamp,
    pub lands: Vec<LandEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandEntry {
    pub host: HostNumber,
    pub land_type: LandType,
    pub land_number: LandNumber,
    pub min_version: ClientVersion,
    pub max_version: ClientVersion,
    pub flags: LandFlags,
    /// One word: the client splits its land file at spaces and shows `_` as a space.
    pub description: String,
}

/// One row of `WaitGrpRequest` msgType 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occupancy {
    pub host: HostNumber,
    pub land_type: LandType,
    pub land_number: LandNumber,
    pub maximum: u8,
    pub current: u8,
}

const HOST_NUMBER_TYPE: u8 = 5;
const HOST_TIME_TYPE: u8 = 2;
const OCCUPANCY_TYPE: u8 = 1;
const DIRECTORY_TYPE: u8 = 2;

impl HostMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = WireWriter::default();
        match self {
            HostMessage::LoginAccepted(ack) => {
                writer
                    .byte(Command::Ack.byte())
                    .byte(0)
                    .word(Sid::GAME_OBJECT.0);
                writer.byte(LOGIN_REPLY_COMMAND).word(ack.user_flags);
                writer.byte(ack.status.0).word(ack.rating);
            }
            HostMessage::Ack(ack) => ack.write(&mut writer),
            HostMessage::Send(send) => send.write(&mut writer),
            HostMessage::Nak(nak) => {
                writer.byte(Command::Nak.byte()).byte(0).word(nak.to.0);
                writer.byte(nak.which_cmd).byte(nak.which_sub).byte(0);
                writer.byte(nak.num_tries).text(&nak.text);
            }
            HostMessage::ObjId { cookie, sid } => {
                writer.byte(Command::ObjId.byte()).byte(0).word(cookie.0);
                writer.word(0).word(sid.0);
            }
            HostMessage::GroupJoined { group, member } => {
                writer.byte(Command::GroupJoin.byte()).byte(0);
                writer.word(group.0).word(member.0);
            }
            HostMessage::GroupLeft(leave) => leave.write(&mut writer),
            HostMessage::GroupMembers(members) => members.write(&mut writer),
            HostMessage::ObjectFreed(sid) => {
                writer.byte(Command::LeaveNet.byte()).byte(0).word(sid.0);
            }
            HostMessage::SetInt(set) => set.write(&mut writer),
            HostMessage::SetStr(set) => set.write(&mut writer),
            HostMessage::ObjectLocated(located) => located.write(&mut writer),
            HostMessage::Notice(notice) => notice.write(&mut writer),
            HostMessage::HostNumber(host) => {
                writer.byte(Command::HostInfo.byte()).byte(HOST_NUMBER_TYPE);
                writer.word(u16::from(host.0));
            }
            HostMessage::HostTime(time) => {
                writer.byte(Command::HostInfo.byte()).byte(HOST_TIME_TYPE);
                writer
                    .byte(time.year_1900)
                    .byte(time.month0)
                    .byte(time.mday);
                writer.byte(time.hour).byte(time.minute).byte(time.second);
            }
            HostMessage::LandDirectory(directory) => {
                let lands = fitting_count(&directory.lands);
                write_wait_group_header(&mut writer, DIRECTORY_TYPE);
                write_stamp(&mut writer, directory.stamp);
                writer.word(row_count(lands));
                lands.iter().for_each(|land| land.write(&mut writer));
            }
            HostMessage::LandOccupancy(rows) => {
                let rows = fitting_count(rows);
                write_wait_group_header(&mut writer, OCCUPANCY_TYPE);
                writer.word(0).word(0).word(row_count(rows));
                rows.iter().for_each(|row| row.write(&mut writer));
            }
        }
        writer.into_bytes()
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, MessageError> {
        let mut reader = WireReader::new(bytes);
        let command = Command::try_from(reader.byte("command")?)?;
        let message = match command {
            Command::Ack => {
                reader.byte("flags")?;
                let to = Sid(reader.word("toSID")?);
                match reader.byte("whichCmd")? {
                    LOGIN_REPLY_COMMAND => HostMessage::LoginAccepted(LoginAck {
                        user_flags: reader.word("userFlags")?,
                        status: LoginStatus(reader.byte("status")?),
                        rating: reader.word("rating")?,
                    }),
                    which_cmd => HostMessage::Ack(Ack::parse_after(&mut reader, to, which_cmd)?),
                }
            }
            Command::Nak => HostMessage::Nak(Nak::parse(&mut reader)?),
            Command::Send => HostMessage::Send(SendMessage::parse(&mut reader)?),
            Command::ObjId => {
                reader.byte("flags")?;
                let cookie = Cookie(reader.word("cookie")?);
                reader.word("unused")?;
                let sid = Sid(reader.word("sid")?);
                HostMessage::ObjId { cookie, sid }
            }
            Command::GroupJoin => {
                reader.byte("flags")?;
                let group = Sid(reader.word("toSID")?);
                let member = Sid(reader.word("who")?);
                HostMessage::GroupJoined { group, member }
            }
            Command::GroupLeave => HostMessage::GroupLeft(GroupLeave::parse(&mut reader)?),
            Command::GroupMembers => HostMessage::GroupMembers(GroupMembers::parse(&mut reader)?),
            Command::LeaveNet => {
                reader.byte("flags")?;
                HostMessage::ObjectFreed(Sid(reader.word("toSID")?))
            }
            Command::SetInt => HostMessage::SetInt(SetInt::parse(&mut reader)?),
            Command::SetStr => HostMessage::SetStr(SetStr::parse(&mut reader)?),
            Command::ObjExists => HostMessage::ObjectLocated(ObjectLocated::parse(&mut reader)?),
            Command::Notice => HostMessage::Notice(Notice::parse(&mut reader)?),
            Command::HostInfo => {
                let host_info_type = reader.byte("type")?;
                match host_info_type {
                    HOST_NUMBER_TYPE => {
                        let host = reader.word("host")?;
                        let host = u8::try_from(host).map_err(|_| MessageError::UnknownValue {
                            field: "host",
                            value: host,
                        })?;
                        HostMessage::HostNumber(HostNumber(host))
                    }
                    HOST_TIME_TYPE => {
                        let year_1900 = reader.byte("year_1900")?;
                        let month0 = reader.byte("month0")?;
                        let mday = reader.byte("mday")?;
                        let hour = reader.byte("hour")?;
                        let minute = reader.byte("minute")?;
                        let second = reader.byte("second")?;
                        HostMessage::HostTime(HostTime {
                            year_1900,
                            month0,
                            mday,
                            hour,
                            minute,
                            second,
                        })
                    }
                    sub => Err(MessageError::UnsupportedSub {
                        command: Command::HostInfo.byte(),
                        sub,
                    })?,
                }
            }
            Command::WaitGroup => parse_wait_group(&mut reader)?,
            Command::JoinNet
            | Command::Multicast
            | Command::ChangePassword
            | Command::Login
            | Command::LoginWithProdigyId => {
                return Err(MessageError::UnsupportedCommand(command.byte()))
            }
        };
        reader.finish()?;
        Ok(message)
    }
}

fn write_wait_group_header(writer: &mut WireWriter, msg_type: u8) {
    writer.byte(Command::WaitGroup.byte()).byte(msg_type);
    writer.word(Sid::GAME_OBJECT.0).word(0);
}

fn parse_wait_group(reader: &mut WireReader) -> Result<HostMessage, MessageError> {
    let msg_type = reader.byte("msgType")?;
    reader.word("toSID")?;
    reader.word("fromSID")?;
    match msg_type {
        OCCUPANCY_TYPE => {
            reader.word("unused")?;
            reader.word("unused")?;
            let count = reader.word("count")?;
            let rows = (0..count).map(|_| Occupancy::parse(reader));
            Ok(HostMessage::LandOccupancy(rows.collect::<Result<_, _>>()?))
        }
        DIRECTORY_TYPE => {
            let stamp = read_stamp(reader)?;
            let count = reader.word("count")?;
            let lands = (0..count).map(|_| LandEntry::parse(reader));
            let lands = lands.collect::<Result<_, _>>()?;
            Ok(HostMessage::LandDirectory(LandDirectory { stamp, lands }))
        }
        sub => Err(MessageError::UnsupportedSub {
            command: Command::WaitGroup.byte(),
            sub,
        }),
    }
}

impl Nak {
    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        reader.byte("flags")?;
        let to = Sid(reader.word("toSID")?);
        let which_cmd = reader.byte("whichCmd")?;
        let which_sub = reader.byte("whichSub")?;
        reader.byte("unused")?;
        let num_tries = reader.byte("numTries")?;
        let text = match reader.is_empty() {
            true => String::new(),
            false => reader.text("text")?,
        };
        Ok(Nak {
            to,
            which_cmd,
            which_sub,
            num_tries,
            text,
        })
    }
}

impl LandEntry {
    fn write(&self, writer: &mut WireWriter) {
        writer
            .byte(self.host.0)
            .byte(self.land_type.0)
            .byte(self.land_number.0);
        writer.array(&DIRECTORY_UNREAD_BYTES_ASSUMED);
        for version in [self.min_version, self.max_version] {
            writer.array(&[version.major, version.minor, version.revision]);
        }
        writer.byte(self.flags.0).text(&self.description);
    }

    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let [host, land_type, land_number] = reader.array("land")?;
        let _unread: [u8; 2] = reader.array("unread")?;
        let [major, minor, revision] = reader.array("minVer")?;
        let min_version = ClientVersion::new(major, minor, revision);
        let [major, minor, revision] = reader.array("maxVer")?;
        Ok(LandEntry {
            host: HostNumber(host),
            land_type: LandType(land_type),
            land_number: LandNumber(land_number),
            min_version,
            max_version: ClientVersion::new(major, minor, revision),
            flags: LandFlags(reader.byte("flags")?),
            description: reader.text("description")?,
        })
    }
}

impl Occupancy {
    fn write(&self, writer: &mut WireWriter) {
        writer
            .byte(self.host.0)
            .byte(self.land_type.0)
            .byte(self.land_number.0);
        writer.byte(self.maximum).byte(self.current);
    }

    fn parse(reader: &mut WireReader) -> Result<Self, MessageError> {
        let [host, land_type, land_number, maximum, current] = reader.array("occupancy")?;
        Ok(Occupancy {
            host: HostNumber(host),
            land_type: LandType(land_type),
            land_number: LandNumber(land_number),
            maximum,
            current,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_the_capture_lacks_round_trip() {
        let nak = Nak::login(
            LoginNakReason::RetryPassword,
            3,
            "That password is not right.",
        );
        let messages = [
            HostMessage::Nak(nak),
            HostMessage::LoginAccepted(LoginAck {
                user_flags: 0x0086,
                status: LoginStatus::PASSWORD_OUT_OF_DATE,
                rating: 4,
            }),
        ];
        for message in messages {
            assert_eq!(HostMessage::parse(&message.encode()), Ok(message));
        }
    }

    #[test]
    fn host_time_golden_bytes_match_census_layout() {
        let time = HostTime {
            year_1900: 94, // 1994
            month0: 1,     // February (0-based)
            mday: 15,
            hour: 10,
            minute: 30,
            second: 45,
        };
        let message = HostMessage::HostTime(time);
        let bytes = message.encode();
        // Layout: b 36 (HostInfo), b 2 (type=HostTime), then 6 bytes
        assert_eq!(bytes[0], 36);
        assert_eq!(bytes[1], 2);
        assert_eq!(bytes[2], 94); // year_1900
        assert_eq!(bytes[3], 1); // month0
        assert_eq!(bytes[4], 15); // mday
        assert_eq!(bytes[5], 10); // hour
        assert_eq!(bytes[6], 30); // minute
        assert_eq!(bytes[7], 45); // second
    }

    #[test]
    fn host_time_round_trip_through_encode_and_parse() {
        let time = HostTime {
            year_1900: 94,
            month0: 1,
            mday: 15,
            hour: 10,
            minute: 30,
            second: 45,
        };
        let message = HostMessage::HostTime(time);
        assert_eq!(HostMessage::parse(&message.encode()), Ok(message));
    }
}
